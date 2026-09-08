// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QtGlobal>
#include <QVector>

#include <functional>
#include <optional>

namespace FrameIdAllocator {
constexpr int kMaxFrameCount = 10'000;
constexpr int kRandomAttempts = 32;

std::optional<quint32> allocate(const QVector<quint32> &existingIds,
                                const std::function<quint32()> &nextCandidate);
} // namespace FrameIdAllocator
