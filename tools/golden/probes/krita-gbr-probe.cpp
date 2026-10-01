// Krita's GBR load path, with Qt replaced by equivalents, used to (a) generate valid GBR files for the
// Rust tests and (b) print what Krita's reader makes of them, so the translation is checked against
// behaviour rather than against my reading of the source.
//
// Two inversions matter here and they compose. GIMP stores a grayscale GBR with 255 meaning PAINT.
// Krita's masks run the other way -- 0 is opaque -- so `kis_gbr_brush.cpp` stores `255 - v`. This product
// emits coverage, where 1.0 is opaque, so it must invert Krita's stored value BACK, which means the GBR
// byte maps straight to coverage. Getting that wrong yields a photographic negative of every brush.

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

static void put_be32(std::vector<uint8_t> &out, uint32_t value) {
    out.push_back((value >> 24) & 0xff);
    out.push_back((value >> 16) & 0xff);
    out.push_back((value >> 8) & 0xff);
    out.push_back(value & 0xff);
}

static uint32_t get_be32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | (uint32_t)p[3];
}

static const uint32_t GIMP_V2_MAGIC = ('G' << 24) + ('I' << 16) + ('M' << 8) + ('P' << 0);
static const size_t V1_HEADER = 20;  // header_size, version, width, height, bytes
static const size_t V2_HEADER = 28;  // + magic_number, spacing

// Build a GBR of the given version and depth.
static std::vector<uint8_t> build_gbr(uint32_t version, uint32_t width, uint32_t height,
                                      uint32_t bytes, uint32_t spacing, const std::string &name,
                                      const std::vector<uint8_t> &payload) {
    std::vector<uint8_t> out;
    const size_t base = (version == 1) ? V1_HEADER : V2_HEADER;
    const uint32_t header_size = (uint32_t)(base + name.size() + 1);
    put_be32(out, header_size);
    put_be32(out, version);
    put_be32(out, width);
    put_be32(out, height);
    put_be32(out, bytes);
    if (version != 1) {
        put_be32(out, GIMP_V2_MAGIC);
        put_be32(out, spacing);
    }
    out.insert(out.end(), name.begin(), name.end());
    out.push_back(0);
    out.insert(out.end(), payload.begin(), payload.end());
    return out;
}

struct Decoded {
    bool ok = false;
    std::string reason;
    uint32_t version = 0, width = 0, height = 0, bytes = 0;
    double spacing = 0.0;
    std::string name;
    std::vector<uint8_t> krita_mask;  // Krita's convention: 0 opaque, 255 transparent
};

// Krita's KisGbrBrush::init, structurally verbatim.
static Decoded krita_init(const std::vector<uint8_t> &data) {
    Decoded r;
    if (V2_HEADER > data.size()) { r.reason = "data shorter than the v2 header"; return r; }

    const uint32_t header_size = get_be32(&data[0]);
    const uint32_t version = get_be32(&data[4]);
    const uint32_t width = get_be32(&data[8]);
    const uint32_t height = get_be32(&data[12]);
    const uint32_t bytes = get_be32(&data[16]);
    uint32_t spacing;

    if (version == 1) {
        spacing = (uint32_t)(0.10 * 100);  // Krita's DEFAULT_SPACING
    } else {
        spacing = get_be32(&data[24]);
        if (spacing > 1000) { r.reason = "spacing above 1000"; return r; }
    }

    if (header_size > data.size() || header_size == 0) {
        r.reason = "header size larger than the data, or zero";
        return r;
    }

    const size_t name_base = (version == 1) ? V1_HEADER : V2_HEADER;
    if (header_size < name_base + 1) { r.reason = "header too small for a name"; return r; }
    r.name.assign((const char *)&data[name_base], header_size - name_base - 1);

    if (width == 0 || height == 0) { r.reason = "width or height is 0"; return r; }

    size_t k = header_size;
    if (bytes == 1) {
        if (k + (size_t)width * height > data.size()) { r.reason = "grayscale payload short"; return r; }
        r.krita_mask.resize((size_t)width * height);
        for (size_t i = 0; i < (size_t)width * height; ++i) {
            // The inversion.
            r.krita_mask[i] = (uint8_t)(255 - data[k + i]);
        }
    } else if (bytes == 4) {
        if (k + (size_t)width * height * 4 > data.size()) { r.reason = "rgba payload short"; return r; }
        r.krita_mask.resize((size_t)width * height);
        for (size_t i = 0; i < (size_t)width * height; ++i) {
            // Krita builds an ARGB image here and derives the mask later; the alpha channel is what a
            // coverage reading needs, and GBR stores RGBA in that order.
            r.krita_mask[i] = (uint8_t)(255 - data[k + i * 4 + 3]);
        }
    } else {
        r.reason = "unsupported depth";
        return r;
    }

    r.ok = true;
    r.version = version; r.width = width; r.height = height; r.bytes = bytes;
    r.spacing = spacing / 100.0;
    return r;
}

static void report(const char *label, const std::vector<uint8_t> &data) {
    Decoded d = krita_init(data);
    printf("=== %s  (%zu bytes)\n", label, data.size());
    if (!d.ok) { printf("    REFUSED: %s\n", d.reason.c_str()); return; }
    printf("    version=%u  %ux%u  depth=%u  spacing=%.2f  name=\"%s\"\n",
           d.version, d.width, d.height, d.bytes, d.spacing, d.name.c_str());
    printf("    krita mask (0=opaque):");
    for (size_t i = 0; i < d.krita_mask.size() && i < 12; ++i) printf(" %3d", d.krita_mask[i]);
    printf("\n    coverage (1=opaque) :");
    for (size_t i = 0; i < d.krita_mask.size() && i < 12; ++i)
        printf(" %.2f", 1.0 - d.krita_mask[i] / 255.0);
    printf("\n");
}

static void hexdump(const char *label, const std::vector<uint8_t> &data) {
    printf("%s = [", label);
    for (size_t i = 0; i < data.size(); ++i) printf("%s%u", i ? ", " : "", data[i]);
    printf("]\n");
}

int main() {
    // A 3x2 grayscale ramp. GIMP semantics: 255 is paint.
    std::vector<uint8_t> gray = {0, 64, 128, 192, 255, 32};
    auto v2 = build_gbr(2, 3, 2, 1, 25, "ramp", gray);
    report("v2 grayscale 3x2, spacing 25", v2);
    hexdump("V2_GRAY", v2);

    auto v1 = build_gbr(1, 3, 2, 1, 0, "old", gray);
    report("v1 grayscale 3x2 (no magic, default spacing)", v1);
    hexdump("V1_GRAY", v1);

    // RGBA: opaque red, half-alpha green, transparent blue.
    std::vector<uint8_t> rgba = {255,0,0,255,  0,255,0,128,  0,0,255,0,
                                 255,255,255,255,  0,0,0,64,  9,9,9,200};
    auto v2rgba = build_gbr(2, 3, 2, 4, 10, "colour", rgba);
    report("v2 rgba 3x2", v2rgba);
    hexdump("V2_RGBA", v2rgba);

    printf("\n--- 거부되어야 하는 것들\n");
    report("spacing 1001", build_gbr(2, 2, 2, 1, 1001, "x", {1,2,3,4}));
    report("width 0", build_gbr(2, 0, 2, 1, 10, "x", {1,2}));
    report("height 0", build_gbr(2, 2, 0, 1, 10, "x", {1,2}));
    report("depth 2", build_gbr(2, 2, 2, 2, 10, "x", {1,2,3,4}));
    report("payload one byte short", build_gbr(2, 2, 2, 1, 10, "x", {1,2,3}));
    {
        auto bad = build_gbr(2, 2, 2, 1, 10, "x", {1,2,3,4});
        bad[0] = bad[1] = bad[2] = bad[3] = 0;  // header_size = 0
        report("header_size 0", bad);
    }
    {
        auto bad = build_gbr(2, 2, 2, 1, 10, "x", {1,2,3,4});
        put_be32(bad, 0); bad[0] = 0xff;  // header_size enormous
        report("header_size enormous", bad);
    }
    report("truncated to 12 bytes", std::vector<uint8_t>(v2.begin(), v2.begin() + 12));

    printf("\n--- spacing 경계\n");
    report("spacing 0", build_gbr(2, 1, 1, 1, 0, "s", {200}));
    report("spacing 1000", build_gbr(2, 1, 1, 1, 1000, "s", {200}));
    return 0;
}
