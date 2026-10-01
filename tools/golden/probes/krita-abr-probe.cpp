// Krita's ABR brush-collection loader, run with its own arithmetic, to produce reference values for the Rust
// translation and to demonstrate the seek defect in its version-1/2 path.
//
// Qt's QDataStream and QIODevice are replaced by a minimal big-endian reader over a byte buffer. Every line of
// the record walk, the header parsing, the PackBits decoder and the seek arithmetic is Krita's, from
// libs/brush/kis_abr_brush_collection.cpp at the commit pinned in docs/upstream-sources.toml, GPL-2.0-or-later.
//
// THE DEFECT THIS EXISTS TO SHOW. In abr_brush_load_v12 Krita computes
//
//     next_brush = abr.device()->pos() + brush_size;      // already ABSOLUTE
//
// and then, for a computed brush (brush_type == 1), seeks
//
//     abr.device()->seek(abr.device()->pos() + next_brush);   // adds pos() a SECOND time
//
// Its other two exits in the same function seek plain `next_brush`, and the v6 path does so everywhere. There
// is a TODO beside the line. The consequence is that every sampled brush after a computed one in a v1/v2 file
// is lost, because the walk lands past the start of the next record.

#include <cstdio>
#include <cstdint>
#include <cstring>
#include <cstdlib>
#include <vector>
#include <string>

// ---------------------------------------------------------------- a minimal QDataStream/QIODevice
struct Reader {
    const uint8_t *data;
    size_t size;
    size_t at = 0;
    bool bad = false;

    // QDataStream reads big-endian by default, which is what the ABR format is.
    uint8_t u8()   { if (at + 1 > size) { bad = true; return 0; } return data[at++]; }
    int16_t i16()  { if (at + 2 > size) { bad = true; return 0; }
                     int16_t v = (int16_t)((data[at] << 8) | data[at + 1]); at += 2; return v; }
    int32_t i32()  { if (at + 4 > size) { bad = true; return 0; }
                     int32_t v = (int32_t)(((uint32_t)data[at] << 24) | ((uint32_t)data[at + 1] << 16)
                                         | ((uint32_t)data[at + 2] << 8) | (uint32_t)data[at + 3]);
                     at += 4; return v; }
    // QIODevice::seek accepts any position; a read past the end then fails rather than the seek.
    void seek(long long pos) { at = (size_t)(pos < 0 ? 0 : pos); }
    size_t pos() const { return at; }
};

struct AbrInfo {
    int16_t version = 0;
    int16_t subversion = 0;
    int32_t count = 0;
};

// Krita's own predicate, unchanged.
static bool abr_supported_content(AbrInfo *abr_hdr)
{
    switch (abr_hdr->version) {
    case 1:
    case 2:
        return true;
    case 6:
        if (abr_hdr->subversion == 1 || abr_hdr->subversion == 2)
            return true;
        break;
    }
    return false;
}

// Krita's PackBits decoder. `-128` is a no-op consuming no data byte, and an overlong row is truncated at its
// row rather than spilling into the next.
static int32_t rle_decode(Reader &abr, char *buffer, int32_t height)
{
    char32_t n;
    char ptmp;
    char c;
    int32_t len = 0;
    int32_t offset = 0;

    std::vector<int16_t> cscanline_len((size_t)height);
    for (int32_t i = 0; i < height; i++) {
        cscanline_len[(size_t)i] = abr.i16();
    }

    for (int32_t i = 0; i < height; i++) {
        len = 0;
        while (len < cscanline_len[(size_t)i]) {
            ptmp = (char)abr.u8();
            n = (char32_t)(unsigned char)ptmp;
            len++;
            if ((signed char)ptmp < 0) {
                // Compressed run.
                int32_t count = 256 - (int32_t)n + 1;
                if (count == 129) {
                    // Krita's -128 case: a no-op that consumes no data byte.
                    continue;
                }
                c = (char)abr.u8();
                len++;
                for (int32_t j = 0; j < count; j++) {
                    buffer[offset++] = c;
                }
            } else {
                // Literal run.
                int32_t count = (int32_t)n + 1;
                for (int32_t j = 0; j < count; j++) {
                    buffer[offset++] = (char)abr.u8();
                }
                len += count;
            }
        }
    }
    return offset;
}

static bool abr_read_content(Reader &abr, AbrInfo *abr_hdr)
{
    abr_hdr->version = abr.i16();
    abr_hdr->subversion = 0;
    abr_hdr->count = 0;
    switch (abr_hdr->version) {
    case 1:
    case 2:
        // `count` is a SHORT in Krita's AbrInfo, not a long. My first version of this probe read it as
        // four bytes and its fixtures wrote four, so the two errors agreed and the probe looked correct --
        // until the Rust decoder, which reads two, found nothing in the same file. The geometry assertions
        // could not catch it because the geometry was fine; only comparing against the other
        // implementation did.
        abr_hdr->count = abr.i16();
        break;
    case 6:
        abr_hdr->subversion = abr.i16();
        // find_sample_count_v6 walks 8BIM sections; the fixtures here are v1/v2, so it is not reproduced.
        abr_hdr->count = 0;
        break;
    default:
        break;
    }
    return true;
}

struct Loaded {
    std::string name;
    int32_t width = 0;
    int32_t height = 0;
    int32_t depth = 0;
    size_t record_started_at = 0;
};

// Krita's abr_brush_load_v12, with its seek arithmetic exactly as written.
static int32_t abr_brush_load_v12(Reader &abr, AbrInfo *abr_hdr, int32_t id,
                                  std::vector<Loaded> &out, bool fix_the_seek)
{
    size_t record_started_at = abr.pos();
    int16_t brush_type;
    int32_t brush_size;
    long long next_brush;

    int32_t top, left, bottom, right;
    int16_t depth;
    char compression;

    brush_type = abr.i16();
    brush_size = abr.i32();
    next_brush = (long long)abr.pos() + brush_size;

    if (brush_type == 1) {
        // Computed brush: unsupported upstream, and THIS is the defective seek.
        if (fix_the_seek) {
            abr.seek(next_brush);
        } else {
            abr.seek((long long)abr.pos() + next_brush);
        }
        printf("    record at %zu: computed brush, size %d; next_brush=%lld, Krita seeks to %zu%s\n",
               record_started_at, brush_size, next_brush, abr.pos(),
               fix_the_seek ? " (corrected)" : "");
    } else if (brush_type == 2) {
        // Sampled brush. Discard 4 misc bytes and 2 spacing bytes.
        abr.seek((long long)abr.pos() + 6);

        std::string name;
        if (abr_hdr->version == 2) {
            // abr_read_ucs2_text: a long character count, then UCS-2BE.
            int32_t chars = abr.i32();
            for (int32_t i = 0; i < chars && !abr.bad; i++) {
                int16_t ch = abr.i16();
                if (ch > 0 && ch < 128) name.push_back((char)ch);
            }
        }
        if (name.empty()) {
            char buf[64];
            snprintf(buf, sizeof buf, "brush_%d", id);
            name = buf;
        }

        // Discard 1 byte for antialiasing and 4 shorts of short bounds.
        abr.seek((long long)abr.pos() + 9);

        top = abr.i32();
        left = abr.i32();
        bottom = abr.i32();
        right = abr.i32();
        depth = abr.i16();
        compression = (char)abr.u8();

        int32_t width = right - left;
        int32_t height = bottom - top;
        int32_t size = width * (depth >> 3) * height;

        if (height > 16384) {
            printf("    record at %zu: wide brush unsupported, skipping\n", record_started_at);
            abr.seek(next_brush);
            return -1;
        }
        if (size <= 0 || abr.bad) {
            printf("    record at %zu: unusable size %d (bad=%d)\n", record_started_at, size, (int)abr.bad);
            abr.seek(next_brush);
            return -1;
        }

        std::vector<char> buffer((size_t)size, 0);
        if (!compression) {
            for (int32_t i = 0; i < size; i++) buffer[(size_t)i] = (char)abr.u8();
        } else {
            rle_decode(abr, buffer.data(), height);
        }

        printf("    record at %zu: sampled '%s' %dx%d depth %d compression %d -> first bytes",
               record_started_at, name.c_str(), width, height, (int)depth, (int)compression);
        for (int32_t i = 0; i < (size < 6 ? size : 6); i++) {
            printf(" %d", (int)(unsigned char)buffer[(size_t)i]);
        }
        printf("\n");

        Loaded loaded;
        loaded.name = name;
        loaded.width = width;
        loaded.height = height;
        loaded.depth = depth;
        loaded.record_started_at = record_started_at;
        out.push_back(loaded);
        abr.seek(next_brush);
        return 1;
    } else {
        printf("    record at %zu: unknown brush type %d, skipping\n", record_started_at, (int)brush_type);
        abr.seek(next_brush);
    }
    return -1;
}

static bool load(const char *label, const std::vector<uint8_t> &bytes, bool fix_the_seek,
                 std::vector<Loaded> *recovered = nullptr)
{
    printf("=== %s  (%zu bytes)%s\n", label, bytes.size(), fix_the_seek ? "  [seek corrected]" : "");
    Reader abr{bytes.data(), bytes.size()};
    AbrInfo hdr;
    abr_read_content(abr, &hdr);
    printf("    header: version %d subversion %d count %d  supported=%d\n",
           (int)hdr.version, (int)hdr.subversion, hdr.count, (int)abr_supported_content(&hdr));
    if (!abr_supported_content(&hdr)) {
        printf("    refused by abr_supported_content\n\n");
        return false;
    }
    std::vector<Loaded> loaded;
    for (int32_t i = 0; i < hdr.count; i++) {
        if (abr.pos() >= abr.size) {
            printf("    walk ran past the end of the file after %zu brush(es)\n", loaded.size());
            break;
        }
        abr_brush_load_v12(abr, &hdr, i, loaded, fix_the_seek);
    }
    printf("    declared %d brush(es), recovered %zu\n\n", hdr.count, loaded.size());
    if (recovered) *recovered = loaded;
    return true;
}

// ---------------------------------------------------------------- fixtures
//
// Built to the same layout research/abr-fixtures.py writes, so the encoder and this decoder check each other.

static void put16(std::vector<uint8_t> &v, int16_t x) { v.push_back((uint8_t)(x >> 8)); v.push_back((uint8_t)x); }
static void put32(std::vector<uint8_t> &v, int32_t x) {
    v.push_back((uint8_t)(x >> 24)); v.push_back((uint8_t)(x >> 16));
    v.push_back((uint8_t)(x >> 8)); v.push_back((uint8_t)x);
}

// One uncompressed sampled record: `width`x`height`, depth 8, bytes rising from `first`.
//
// The field order is Krita's read order and nothing may be omitted from it. My first version left out the
// version-2 UCS-2 NAME LENGTH, a long that abr_read_ucs2_text reads between the spacing bytes and the bounds
// skip. Krita then consumed four bytes of `top` as part of its 9-byte skip and reported a 524296x3 brush at
// depth 5150 -- values so absurd that they were obviously wrong, which is the only reason the omission was
// caught rather than baked into a transcript. The assertions in main() now pin the decoded geometry.
static std::vector<uint8_t> sampled_record(int width, int height, uint8_t first)
{
    std::vector<uint8_t> body;
    put16(body, 2);            // brush_type = sampled
    std::vector<uint8_t> rest;
    for (int i = 0; i < 6; i++) rest.push_back(0);   // 4 misc + 2 spacing
    put32(rest, 0);            // UCS-2 name length: zero, so the name falls back to abr_v1_brush_name
    for (int i = 0; i < 9; i++) rest.push_back(0);   // antialias + 4 short bounds
    put32(rest, 0);            // top
    put32(rest, 0);            // left
    put32(rest, height);       // bottom
    put32(rest, width);        // right
    put16(rest, 8);            // depth
    rest.push_back(0);         // compression off
    for (int i = 0; i < width * height; i++) rest.push_back((uint8_t)(first + i * 10));
    put32(body, (int32_t)rest.size());
    body.insert(body.end(), rest.begin(), rest.end());
    return body;
}

// One computed record: the type Krita cannot read and mis-seeks past.
static std::vector<uint8_t> computed_record()
{
    std::vector<uint8_t> body;
    put16(body, 1);            // brush_type = computed
    std::vector<uint8_t> rest(16, 0);
    put32(body, (int32_t)rest.size());
    body.insert(body.end(), rest.begin(), rest.end());
    return body;
}

int main()
{
    printf("Krita ABR loader, its own arithmetic, from libs/brush/kis_abr_brush_collection.cpp\n");
    printf("QDataStream/QIODevice replaced by a big-endian byte reader; the record walk is unchanged.\n\n");

    // Two sampled brushes, nothing computed: the ordinary case.
    {
        std::vector<uint8_t> f;
        put16(f, 2); put16(f, 2);      // version 2, two brushes (count is a SHORT)
        auto a = sampled_record(3, 2, 10);
        auto b = sampled_record(2, 2, 200);
        f.insert(f.end(), a.begin(), a.end());
        f.insert(f.end(), b.begin(), b.end());
        std::vector<Loaded> got;
        load("v2, two sampled brushes", f, false, &got);
        // The fixture must decode to what it was written as. Without this, an off-by-four in the record
        // layout produced a "524296x3 brush at depth 5150" and nothing objected -- it would have been
        // recorded as the upstream's own output.
        if (got.size() != 2 || got[0].width != 3 || got[0].height != 2 || got[0].depth != 8 ||
            got[1].width != 2 || got[1].height != 2) {
            printf("FIXTURE ERROR: the two sampled brushes did not decode as 3x2 and 2x2 at depth 8\n");
            return 1;
        }
    }

    // A computed brush FOLLOWED by a sampled one. This is the defect.
    {
        std::vector<uint8_t> f;
        put16(f, 2); put16(f, 2);
        auto c = computed_record();
        auto s = sampled_record(3, 2, 10);
        f.insert(f.end(), c.begin(), c.end());
        size_t sampled_begins_at = f.size();
        f.insert(f.end(), s.begin(), s.end());
        printf("The sampled record in the next two runs begins at byte %zu.\n", sampled_begins_at);
        std::vector<Loaded> as_written, corrected;
        load("v2, computed then sampled -- Krita as written", f, false, &as_written);
        load("v2, computed then sampled -- seek corrected", f, true, &corrected);
        // The defect, asserted rather than merely printed: as written the sampled brush is LOST, and the
        // only change is the seek.
        if (!as_written.empty()) {
            printf("FIXTURE ERROR: Krita as written recovered a brush it should have mis-seeked past\n");
            return 1;
        }
        if (corrected.size() != 1 || corrected[0].record_started_at != sampled_begins_at) {
            printf("FIXTURE ERROR: the corrected seek did not land on the sampled record\n");
            return 1;
        }
    }

    // A version Krita refuses by name.
    {
        std::vector<uint8_t> f;
        put16(f, 3); put16(f, 1);
        load("v3 (CinePaint), refused by abr_supported_content", f, false);
    }

    // v6 subversion 3: outside the supported set.
    {
        std::vector<uint8_t> f;
        put16(f, 6); put16(f, 3);
        load("v6 subversion 3, outside the supported set", f, false);
    }

    return 0;
}
