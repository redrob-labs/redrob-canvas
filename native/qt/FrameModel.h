// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QAbstractListModel>
#include <QJsonObject>
#include <QVector>

class FrameModel final : public QAbstractListModel
{
    Q_OBJECT

public:
    enum Role {
        FrameIdRole = Qt::UserRole + 1,
        IndexRole,
        DurationRole,
        CurrentRole,
        InRangeRole,
        IsFirstRole,
        IsLastRole,
    };
    Q_ENUM(Role)

    explicit FrameModel(QObject *parent = nullptr);
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    bool replaceFromSnapshot(const QJsonObject &timeline);
    quint32 currentFrameId() const;
    int currentIndex() const;
    Q_INVOKABLE quint32 frameIdAt(int index) const;
    Q_INVOKABLE int indexOf(quint32 id) const;

private:
    struct FrameRow {
        quint32 id = 0;
        int index = 0;
        int duration = 0;
        bool current = false;
        bool inRange = false;
    };
    QVector<FrameRow> m_frames;
};
