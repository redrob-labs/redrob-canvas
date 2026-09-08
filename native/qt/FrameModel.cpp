// SPDX-License-Identifier: GPL-3.0-or-later
#include "FrameModel.h"

#include <QJsonArray>

#include <limits>

FrameModel::FrameModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

int FrameModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_frames.size();
}

QVariant FrameModel::data(const QModelIndex &modelIndex, int role) const
{
    if (!modelIndex.isValid() || modelIndex.row() < 0 || modelIndex.row() >= m_frames.size())
        return {};
    const FrameRow &frame = m_frames.at(modelIndex.row());
    switch (role) {
    case FrameIdRole: return QVariant::fromValue(frame.id);
    case IndexRole: return frame.index;
    case DurationRole: return frame.duration;
    case CurrentRole: return frame.current;
    case InRangeRole: return frame.inRange;
    case IsFirstRole: return modelIndex.row() == 0;
    case IsLastRole: return modelIndex.row() + 1 == m_frames.size();
    default: return {};
    }
}

QHash<int, QByteArray> FrameModel::roleNames() const
{
    return {{FrameIdRole, "frameId"}, {IndexRole, "index"}, {DurationRole, "duration"},
            {CurrentRole, "current"}, {InRangeRole, "inRange"}, {IsFirstRole, "isFirst"},
            {IsLastRole, "isLast"}};
}

bool FrameModel::replaceFromSnapshot(const QJsonObject &timeline)
{
    const QJsonArray frames = timeline.value(QStringLiteral("frames")).toArray();
    if (frames.isEmpty())
        return false;
    QVector<FrameRow> next;
    next.reserve(frames.size());
    for (int row = 0; row < frames.size(); ++row) {
        const QJsonObject frame = frames.at(row).toObject();
        bool idOk = false;
        const qulonglong id = frame.value(QStringLiteral("id")).toVariant().toULongLong(&idOk);
        const int index = frame.value(QStringLiteral("index")).toInt(-1);
        const int duration = frame.value(QStringLiteral("duration_ms")).toInt();
        if (!idOk || id > std::numeric_limits<quint32>::max() || index != row || duration < 1
            || !frame.value(QStringLiteral("current")).isBool()
            || !frame.value(QStringLiteral("in_range")).isBool())
            return false;
        next.push_back({static_cast<quint32>(id), index, duration,
                        frame.value(QStringLiteral("current")).toBool(),
                        frame.value(QStringLiteral("in_range")).toBool()});
    }
    beginResetModel();
    m_frames = std::move(next);
    endResetModel();
    return true;
}

quint32 FrameModel::currentFrameId() const
{
    for (const FrameRow &frame : m_frames) {
        if (frame.current)
            return frame.id;
    }
    return 0;
}

int FrameModel::currentIndex() const
{
    for (int index = 0; index < m_frames.size(); ++index) {
        if (m_frames.at(index).current)
            return index;
    }
    return -1;
}

quint32 FrameModel::frameIdAt(int index) const
{
    return index >= 0 && index < m_frames.size() ? m_frames.at(index).id : 0;
}


int FrameModel::indexOf(quint32 id) const
{
    for (int index = 0; index < m_frames.size(); ++index) {
        if (m_frames.at(index).id == id)
            return index;
    }
    return -1;
}
