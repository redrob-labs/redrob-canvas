// SPDX-License-Identifier: GPL-3.0-or-later
#include "FrameIdAllocator.h"

#include <QSet>

namespace FrameIdAllocator {
std::optional<quint32> allocate(const QVector<quint32> &existingIds,
                                const std::function<quint32()> &nextCandidate)
{
    if (existingIds.size() >= kMaxFrameCount || !nextCandidate)
        return std::nullopt;

    const QSet<quint32> occupied(existingIds.cbegin(), existingIds.cend());
    quint32 candidate = 0;
    for (int attempt = 0; attempt < kRandomAttempts; ++attempt) {
        candidate = nextCandidate();
        if (!occupied.contains(candidate))
            return candidate;
    }

    // At most 9,999 IDs are occupied, so N + 1 probes must find a free
    // uint32 value. Unsigned addition intentionally wraps through zero.
    for (qsizetype probe = 0; probe <= occupied.size(); ++probe) {
        candidate += quint32{1};
        if (!occupied.contains(candidate))
            return candidate;
    }
    return std::nullopt;
}
} // namespace FrameIdAllocator
