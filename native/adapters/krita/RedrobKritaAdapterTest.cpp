// SPDX-License-Identifier: GPL-3.0-or-later
#include "RedrobKritaAdapter.h"

#include <cassert>
#include <cstdint>
#include <span>

int main()
{
    RedrobKritaAdapter adapter;
    auto capabilities = adapter.capabilities();
    assert(capabilities.scaffoldCompiled);
    assert(!capabilities.attached);
    assert(!capabilities.ready);
    assert(capabilities.operationCount == 0);
    assert(capabilities.formatCount == 0);

    auto *document = reinterpret_cast<KisDocument *>(std::uintptr_t{1});
    auto *image = reinterpret_cast<KisImage *>(std::uintptr_t{2});
    assert(adapter.attach(document, image));
    assert(adapter.isAttached());
    assert(adapter.capabilities().attached);
    assert(!adapter.capabilities().ready);
    assert(!adapter.applyPaintStroke(std::span<const std::uint8_t>{}));

    assert(!adapter.attach(document, nullptr));
    assert(!adapter.isAttached());
    adapter.detach();
    assert(!adapter.isAttached());
    return 0;
}
