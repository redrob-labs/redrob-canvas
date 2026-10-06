// SPDX-License-Identifier: GPL-3.0-or-later
#include "ConsoleConnection.h"

#include "redrob_ffi.h"

#include <QDesktopServices>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QStandardPaths>
#include <QUrl>
#include <QtConcurrentRun>

namespace {
// A key is rrk_{prefix}_{secret}; anything far longer is not one.
constexpr qsizetype kMaxKeyBytes = 4096;

QByteArray takeOwned(RedrobBuffer buffer)
{
    QByteArray bytes;
    if (buffer.data && buffer.len > 0)
        bytes = QByteArray(reinterpret_cast<const char *>(buffer.data), static_cast<qsizetype>(buffer.len));
    redrob_buffer_free(buffer);
    return bytes;
}

// Read on the thread that made the failing call: the engine's last error is thread-local.
QString lastError()
{
    const size_t length = redrob_last_error_copy(nullptr, 0);
    QByteArray bytes(static_cast<qsizetype>(length + 1), '\0');
    redrob_last_error_copy(bytes.data(), static_cast<size_t>(bytes.size()));
    bytes.truncate(static_cast<qsizetype>(length));
    return QString::fromUtf8(bytes);
}

QByteArray readStoredKey()
{
    QFile file(ConsoleConnection::keyFilePath());
    if (!file.open(QIODevice::ReadOnly))
        return {};
    const QByteArray key = file.read(kMaxKeyBytes + 1).trimmed();
    if (key.isEmpty() || key.size() > kMaxKeyBytes || key.contains('\n'))
        return {};
    return key;
}
} // namespace

ConsoleConnection::ConsoleConnection(QObject *parent)
    : QObject(parent)
{
    m_fromEnvironment = !qgetenv("REDROB_API_KEY").trimmed().isEmpty();
    if (m_fromEnvironment) {
        m_key = qgetenv("REDROB_API_KEY").trimmed();
    } else {
        m_key = readStoredKey();
    }
    if (!m_key.isEmpty())
        m_state = QStringLiteral("connected");
    connect(&m_startWatcher, &QFutureWatcher<StartResult>::finished, this, &ConsoleConnection::finishStart);
    connect(&m_waitWatcher, &QFutureWatcher<WaitResult>::finished, this, &ConsoleConnection::finishWait);
}

ConsoleConnection::~ConsoleConnection()
{
    // A wait still running holds the flow; cancel it and let it return before freeing.
    if (m_flow)
        redrob_device_flow_cancel(m_flow);
    m_startWatcher.waitForFinished();
    m_waitWatcher.waitForFinished();
    if (m_startWatcher.isFinished() && m_startWatcher.future().resultCount() > 0) {
        RedrobDeviceFlow *orphan = m_startWatcher.result().flow;
        if (orphan && orphan != m_flow)
            redrob_device_flow_destroy(orphan);
    }
    releaseFlow();
}

QString ConsoleConnection::keyFilePath()
{
    return QDir(QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation))
        .filePath(QStringLiteral("console-key"));
}

void ConsoleConnection::connectToConsole()
{
    if (m_state == QStringLiteral("starting") || m_state == QStringLiteral("waiting"))
        return;
    if (m_fromEnvironment) {
        setState(QStringLiteral("connected"), QStringLiteral("Using REDROB_API_KEY from the environment"));
        return;
    }
    setState(QStringLiteral("starting"), QStringLiteral("Asking the Redrob console for a code…"));
    m_startWatcher.setFuture(QtConcurrent::run([] {
        StartResult result;
        RedrobBuffer json{};
        if (redrob_device_flow_start(&result.flow, &json) == REDROB_OK)
            result.json = takeOwned(json);
        else {
            redrob_buffer_free(json);
            result.error = lastError();
            result.flow = nullptr;
        }
        return result;
    }));
}

void ConsoleConnection::finishStart()
{
    const StartResult result = m_startWatcher.result();
    if (!result.flow) {
        setState(QStringLiteral("idle"),
                 QStringLiteral("Could not connect to the Redrob console: %1").arg(result.error));
        return;
    }
    if (m_state != QStringLiteral("starting")) {
        // Cancelled while the request was in flight.
        redrob_device_flow_destroy(result.flow);
        return;
    }
    m_flow = result.flow;
    const QJsonObject shown = QJsonDocument::fromJson(result.json).object();
    m_userCode = shown.value(QStringLiteral("userCode")).toString();
    const QString complete = shown.value(QStringLiteral("verificationUriComplete")).toString();
    m_verificationUrl = complete.isEmpty() ? shown.value(QStringLiteral("verificationUri")).toString() : complete;
    setState(QStringLiteral("waiting"),
             QStringLiteral("Approve code %1 in the console page that just opened").arg(m_userCode));
    openVerificationPage();

    RedrobDeviceFlow *flow = m_flow;
    m_waitWatcher.setFuture(QtConcurrent::run([flow] {
        WaitResult result;
        RedrobBuffer key{};
        if (redrob_device_flow_wait(flow, &key) == REDROB_OK)
            result.key = takeOwned(key);
        else {
            redrob_buffer_free(key);
            result.error = lastError();
        }
        return result;
    }));
}

void ConsoleConnection::finishWait()
{
    const WaitResult result = m_waitWatcher.result();
    releaseFlow();
    m_userCode.clear();
    m_verificationUrl.clear();
    if (result.key.isEmpty()) {
        setState(QStringLiteral("idle"), QStringLiteral("Not connected: %1").arg(result.error));
        return;
    }
    if (!storeKey(result.key)) {
        // Still usable for this session; say that it will not survive a restart.
        m_key = result.key;
        setState(QStringLiteral("connected"),
                 QStringLiteral("Connected for this session only: the key could not be saved"));
        emit keyChanged();
        return;
    }
    m_key = result.key;
    setState(QStringLiteral("connected"), QStringLiteral("Connected to the Redrob console"));
    emit keyChanged();
}

void ConsoleConnection::cancel()
{
    if (m_state == QStringLiteral("waiting") && m_flow) {
        redrob_device_flow_cancel(m_flow);
        return; // finishWait reports and releases.
    }
    if (m_state == QStringLiteral("starting"))
        setState(QStringLiteral("idle"), QStringLiteral("Connection cancelled"));
}

void ConsoleConnection::disconnectFromConsole()
{
    if (m_fromEnvironment) {
        setState(QStringLiteral("connected"),
                 QStringLiteral("REDROB_API_KEY is set in the environment; unset it to disconnect"));
        return;
    }
    QFile::remove(keyFilePath());
    m_key.clear();
    setState(QStringLiteral("idle"),
             QStringLiteral("Disconnected. The key still exists in the console until you revoke it there"));
    emit keyChanged();
}

void ConsoleConnection::openVerificationPage() const
{
    const QUrl url(m_verificationUrl);
    // Only ever hand the browser an https page the console named.
    if (url.isValid() && url.scheme() == QStringLiteral("https"))
        QDesktopServices::openUrl(url);
}

bool ConsoleConnection::storeKey(const QByteArray &key)
{
    if (key.size() > kMaxKeyBytes || key.contains('\n'))
        return false;
    const QString path = keyFilePath();
    if (!QDir().mkpath(QFileInfo(path).absolutePath()))
        return false;
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly))
        return false;
    // Owner read/write only, set before the secret is written.
    file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner);
    if (file.write(key) != key.size())
        return false;
    if (!file.commit())
        return false;
    return QFile::setPermissions(path, QFileDevice::ReadOwner | QFileDevice::WriteOwner);
}

void ConsoleConnection::setState(const QString &state, const QString &message)
{
    m_state = state;
    m_message = message;
    emit changed();
}

void ConsoleConnection::releaseFlow()
{
    if (m_flow) {
        redrob_device_flow_destroy(m_flow);
        m_flow = nullptr;
    }
}
