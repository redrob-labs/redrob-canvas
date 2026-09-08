// SPDX-License-Identifier: GPL-3.0-or-later
#include "RedrobKritaAdapter.h"

bool RedrobKritaAdapter::attach(KisDocument *document, KisImage *image) noexcept
{
    if (document == nullptr || image == nullptr) {
        detach();
        return false;
    }
    m_document = document;
    m_image = image;
    return true;
}

void RedrobKritaAdapter::detach() noexcept
{
    m_document = nullptr;
    m_image = nullptr;
}

bool RedrobKritaAdapter::isAttached() const noexcept
{
    return m_document != nullptr && m_image != nullptr;
}

RedrobKritaAdapter::Capabilities RedrobKritaAdapter::capabilities() const noexcept
{
    return {.scaffoldCompiled = true,
            .attached = isAttached(),
            .ready = false,
            .operationCount = 0,
            .formatCount = 0};
}

bool RedrobKritaAdapter::applyPaintStroke(std::span<const std::uint8_t> typedStrokeJson) noexcept
{
    (void)typedStrokeJson;
    // Deliberately unavailable until fixtures for the pinned Krita commit and
    // explicit KisDocument/KisImage lifetime integration are supplied.
    return false;
}
