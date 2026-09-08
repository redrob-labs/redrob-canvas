// SPDX-License-Identifier: GPL-3.0-or-later
#include "ProposalModel.h"

#include <QUuid>

namespace {
constexpr int kMaxPendingProposals = 128;
}

ProposalModel::ProposalModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

int ProposalModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_items.size();
}

QVariant ProposalModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() < 0 || index.row() >= m_items.size())
        return {};
    const auto &item = m_items.at(index.row());
    switch (role) {
    case IdRole: return item.id;
    case TitleRole: return item.title;
    case SummaryRole: return item.summary;
    case ActionTypeRole: return item.actionType;
    case CommandJsonRole: return item.commandJson;
    default: return {};
    }
}

QHash<int, QByteArray> ProposalModel::roleNames() const
{
    return {{IdRole, "proposalId"},
            {TitleRole, "proposalTitle"},
            {SummaryRole, "proposalSummary"},
            {ActionTypeRole, "proposalAction"},
            {CommandJsonRole, "commandJson"}};
}

QString ProposalModel::enqueue(QString title, QString summary, QString actionType,
                               QString commandJson, qulonglong baseGeneration,
                               qulonglong baseDocumentEpoch, QString sourceId)
{
    if (m_items.size() >= kMaxPendingProposals)
        return {};
    if (!sourceId.isEmpty()) {
        for (const auto &item : m_items) {
            if (item.sourceId == sourceId)
                return {};
        }
    }

    const QString id = QUuid::createUuid().toString(QUuid::WithoutBraces);
    const int row = m_items.size();
    beginInsertRows({}, row, row);
    m_items.push_back({id, std::move(title), std::move(summary), std::move(actionType),
                       std::move(commandJson), baseGeneration, baseDocumentEpoch,
                       std::move(sourceId)});
    endInsertRows();
    return id;
}

bool ProposalModel::lookup(const QString &id, QString *actionType, QString *commandJson,
                           qulonglong *baseGeneration, qulonglong *baseDocumentEpoch) const
{
    for (const auto &item : m_items) {
        if (item.id != id)
            continue;
        if (actionType)
            *actionType = item.actionType;
        if (commandJson)
            *commandJson = item.commandJson;
        if (baseGeneration)
            *baseGeneration = item.baseGeneration;
        if (baseDocumentEpoch)
            *baseDocumentEpoch = item.baseDocumentEpoch;
        return true;
    }
    return false;
}

bool ProposalModel::remove(const QString &id)
{
    for (int row = 0; row < m_items.size(); ++row) {
        if (m_items.at(row).id != id)
            continue;
        beginRemoveRows({}, row, row);
        m_items.removeAt(row);
        endRemoveRows();
        return true;
    }
    return false;
}

bool ProposalModel::reject(const QString &id)
{
    return remove(id);
}
