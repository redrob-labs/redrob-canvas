// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QAbstractListModel>
#include <QString>
#include <QVector>

class ProposalModel final : public QAbstractListModel
{
    Q_OBJECT

public:
    enum Role {
        IdRole = Qt::UserRole + 1,
        TitleRole,
        SummaryRole,
        ActionTypeRole,
        CommandJsonRole
    };
    Q_ENUM(Role)

    explicit ProposalModel(QObject *parent = nullptr);
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    QString enqueue(QString title, QString summary, QString actionType, QString commandJson,
                    qulonglong baseGeneration, qulonglong baseDocumentEpoch,
                    QString sourceId = {});
    bool lookup(const QString &id, QString *actionType, QString *commandJson,
                qulonglong *baseGeneration, qulonglong *baseDocumentEpoch) const;
    bool remove(const QString &id);
    bool reject(const QString &id);

private:
    struct Proposal {
        QString id;
        QString title;
        QString summary;
        QString actionType;
        QString commandJson;
        qulonglong baseGeneration = 0;
        qulonglong baseDocumentEpoch = 0;
        QString sourceId;
    };
    QVector<Proposal> m_items;
};
