// SPDX-License-Identifier: GPL-3.0-or-later
#include "SharedAccount.h"

#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>

namespace {
// The launcher writes a few hundred bytes; anything far larger is not this file.
constexpr qint64 kMaxFileBytes = 64 * 1024;
// A key is rrk_{prefix}_{secret}; anything far longer is not one.
constexpr qsizetype kMaxKeyBytes = 4096;

QString nonEmptyEnv(const char *name)
{
    return QString::fromLocal8Bit(qgetenv(name)).trimmed();
}

QString sharedDirectory()
{
    const QString override = nonEmptyEnv("REDROB_SHARED_DIR");
    if (!override.isEmpty())
        return override;
#if defined(Q_OS_WIN)
    const QString appData = nonEmptyEnv("APPDATA");
    if (appData.isEmpty())
        return {};
    return QDir(appData).filePath(QStringLiteral("Redrob/Shared"));
#elif defined(Q_OS_MACOS)
    const QString home = QDir::homePath();
    if (home.isEmpty())
        return {};
    return QDir(home).filePath(QStringLiteral("Library/Application Support/ai.redrob.shared"));
#else
    QString config = nonEmptyEnv("XDG_CONFIG_HOME");
    if (config.isEmpty()) {
        const QString home = QDir::homePath();
        if (home.isEmpty())
            return {};
        config = QDir(home).filePath(QStringLiteral(".config"));
    }
    return QDir(config).filePath(QStringLiteral("redrob-shared"));
#endif
}
} // namespace

QString SharedAccount::filePath()
{
    const QString dir = sharedDirectory();
    if (dir.isEmpty())
        return {};
    return QDir(dir).filePath(QStringLiteral("account.json"));
}

SharedAccount SharedAccount::read()
{
    return readFrom(filePath());
}

SharedAccount SharedAccount::readFrom(const QString &path)
{
    SharedAccount account;
    if (path.isEmpty())
        return account;
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly) || file.size() > kMaxFileBytes)
        return account;
    const QJsonDocument document = QJsonDocument::fromJson(file.readAll());
    if (!document.isObject())
        return account;
    const QJsonObject object = document.object();
    const QJsonValue key = object.value(QStringLiteral("apiKey"));
    if (!key.isString())
        return account;
    const QByteArray bytes = key.toString().trimmed().toUtf8();
    if (bytes.isEmpty() || bytes.size() > kMaxKeyBytes || bytes.contains('\n'))
        return account;
    account.apiKey = bytes;
    account.apiKeyName = object.value(QStringLiteral("apiKeyName")).toString();
    account.accountName = object.value(QStringLiteral("accountName")).toString();
    account.apiBaseUrl = object.value(QStringLiteral("apiBaseUrl")).toString();
    return account;
}
