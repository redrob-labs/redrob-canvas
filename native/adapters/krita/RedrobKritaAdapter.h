// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <cstddef>
#include <cstdint>
#include <span>
#include <string_view>

class KisDocument;
class KisImage;

// Optional C++ seam pinned to Krita commit
// fdbf33b2146735465bb8aa59928fbc1890ceb160. Krita types remain private
// to this adapter and never cross the redrob-ffi C ABI.
class RedrobKritaAdapter final
{
public:
    struct Capabilities {
        bool scaffoldCompiled = true;
        bool attached = false;
        bool ready = false;
        std::size_t operationCount = 0;
        std::size_t formatCount = 0;
    };

    static constexpr std::string_view SourceBoundary =
        "fdbf33b2146735465bb8aa59928fbc1890ceb160";

    RedrobKritaAdapter() = default;
    RedrobKritaAdapter(const RedrobKritaAdapter &) = delete;
    RedrobKritaAdapter &operator=(const RedrobKritaAdapter &) = delete;
    RedrobKritaAdapter(RedrobKritaAdapter &&) = delete;
    RedrobKritaAdapter &operator=(RedrobKritaAdapter &&) = delete;

    bool attach(KisDocument *document, KisImage *image) noexcept;
    void detach() noexcept;
    bool isAttached() const noexcept;
    Capabilities capabilities() const noexcept;
    bool applyPaintStroke(std::span<const std::uint8_t> typedStrokeJson) noexcept;

private:
    KisDocument *m_document = nullptr;
    KisImage *m_image = nullptr;
};
