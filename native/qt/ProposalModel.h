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
    // A run proposes several steps against one document state: "add a layer", then "fill it".
    // Applying the first moves the document on, which would make every later step stale. Once
    // `id` has been applied, the proposals queued AFTER it against the same state are moved onto
    // the new state, so the run can be applied step by step in order. Returns how many moved.
    int rebaseAfterApply(const QString &id, qulonglong newGeneration);

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
