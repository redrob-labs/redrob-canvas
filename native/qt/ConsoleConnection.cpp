// SPDX-License-Identifier: GPL-3.0-or-later
#include "ConsoleConnection.h"

#include "redrob_ffi.h"

#include <QDesktopServices>
#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocale>
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

// The key an earlier version of this app saved for itself, if any. Read once, for the migration.
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

// Hands a stored key to the engine and removes the file once the engine has it. The engine holds
// the key from here on; a file that could not be handed over is left for the next start.
void migrateStoredKeyToEngine()
{
    const QByteArray key = readStoredKey();
    if (key.isEmpty()) {
        // An unreadable or malformed leftover is removed too: nothing reads it any more.
        if (QFile::exists(ConsoleConnection::keyFilePath()))
            QFile::remove(ConsoleConnection::keyFilePath());
        return;
    }
    if (redrob_engine_sign_in(reinterpret_cast<const uint8_t *>(key.constData()),
                              static_cast<size_t>(key.size())) == REDROB_OK)
        QFile::remove(ConsoleConnection::keyFilePath());
}
} // namespace

ConsoleConnection::ConsoleConnection(QObject *parent)
    : QObject(parent)
{
    connect(&m_statusWatcher, &QFutureWatcher<StatusResult>::finished, this, &ConsoleConnection::finishStatus);
    connect(&m_startWatcher, &QFutureWatcher<StartResult>::finished, this, &ConsoleConnection::finishStart);
    connect(&m_waitWatcher, &QFutureWatcher<WaitResult>::finished, this, &ConsoleConnection::finishWait);
    connect(&m_signOutWatcher, &QFutureWatcher<WaitResult>::finished, this, &ConsoleConnection::finishSignOut);
    migrateStoredKey();
}

ConsoleConnection::~ConsoleConnection()
{
    // A wait still running holds the flow; cancel it and let it return before freeing.
    if (m_flow)
        redrob_device_flow_cancel(m_flow);
    m_statusWatcher.waitForFinished();
    m_startWatcher.waitForFinished();
    m_waitWatcher.waitForFinished();
    m_signOutWatcher.waitForFinished();
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

QString ConsoleConnection::installUrl()
{
    return QStringLiteral("https://github.com/redrob-labs/redrob-launcher/releases/latest");
}

QString ConsoleConnection::launcherSignedInText()
{
    if (QLocale::system().language() == QLocale::Korean)
        return QStringLiteral("Redrob Launcher로 로그인됨");
    return QStringLiteral("Signed in with Redrob Launcher");
}

void ConsoleConnection::migrateStoredKey()
{
    // The first status read also performs the one-time migration, on the same worker thread, so
    // the status it reports already includes the migrated key.
    if (m_statusWatcher.isRunning()) {
        // Asked again mid-read: read once more when this one lands, so a sign-in that moved
        // while the engine was being asked is not missed.
        m_refreshAgain = true;
        return;
    }
    const bool migrate = QFile::exists(keyFilePath());
    // While the engine is asked, the state says so; a device flow in progress keeps its own.
    if (m_state != QStringLiteral("starting") && m_state != QStringLiteral("waiting"))
        setState(QStringLiteral("checking"), QString{});
    m_statusWatcher.setFuture(QtConcurrent::run([migrate] {
        if (migrate)
            migrateStoredKeyToEngine();
        StatusResult result;
        RedrobBuffer json{};
        if (redrob_engine_account_status(&json) == REDROB_OK) {
            const QJsonObject status = QJsonDocument::fromJson(takeOwned(json)).object();
            result.available = true;
            result.connected = status.value(QStringLiteral("connected")).toBool();
            result.source = status.value(QStringLiteral("source")).toString();
        } else {
            redrob_buffer_free(json);
            result.error = lastError();
        }
        return result;
    }));
}

void ConsoleConnection::refresh()
{
    migrateStoredKey();
}

void ConsoleConnection::finishStatus()
{
    applyStatus(m_statusWatcher.result());
    if (m_refreshAgain) {
        m_refreshAgain = false;
        migrateStoredKey();
    }
}

void ConsoleConnection::applyStatus(const StatusResult &status)
{
    const bool wasConnected = m_connected;
    const bool wasAvailable = m_engineAvailable;
    const QString wasSource = m_source;
    m_engineAvailable = status.available;
    m_connected = status.available && status.connected;
    m_source = m_connected ? status.source : QString{};
    // A device flow in progress keeps its state and its message.
    const bool flowing = m_state == QStringLiteral("starting") || m_state == QStringLiteral("waiting");
    if (!flowing) {
        if (!m_engineAvailable)
            setState(QStringLiteral("unavailable"), status.error);
        else if (m_connected)
            setState(QStringLiteral("connected"), fromLauncher() ? launcherSignedInText() : QString{});
        else
            setState(QStringLiteral("idle"), QString{});
    } else {
        emit changed();
    }
    if (wasConnected != m_connected || wasAvailable != m_engineAvailable || wasSource != m_source)
        emit keyChanged();
}

void ConsoleConnection::connectToConsole()
{
    if (m_state == QStringLiteral("starting") || m_state == QStringLiteral("waiting"))
        return;
    if (!m_engineAvailable) {
        // Nothing to hand the key to. The message already says what to install.
        refresh();
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
        setState(m_connected ? QStringLiteral("connected") : QStringLiteral("idle"),
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
        // On approval the engine takes the key; nothing comes back here but the verdict.
        WaitResult result;
        result.ok = redrob_device_flow_wait(flow) == REDROB_OK;
        if (!result.ok)
            result.error = lastError();
        return result;
    }));
}

void ConsoleConnection::finishWait()
{
    const WaitResult result = m_waitWatcher.result();
    releaseFlow();
    m_userCode.clear();
    m_verificationUrl.clear();
    if (!result.ok) {
        setState(m_connected ? QStringLiteral("connected") : QStringLiteral("idle"),
                 QStringLiteral("Not connected: %1").arg(result.error));
        return;
    }
    m_connected = true;
    m_source = QStringLiteral("api");
    setState(QStringLiteral("connected"), QStringLiteral("Connected to the Redrob console"));
    emit keyChanged();
    // The engine's own account of it, which is what later reads use.
    refresh();
}

void ConsoleConnection::cancel()
{
    if (m_state == QStringLiteral("waiting") && m_flow) {
        redrob_device_flow_cancel(m_flow);
        return; // finishWait reports and releases.
    }
    if (m_state == QStringLiteral("starting"))
        setState(m_connected ? QStringLiteral("connected") : QStringLiteral("idle"),
                 QStringLiteral("Connection cancelled"));
}

void ConsoleConnection::disconnectFromConsole()
{
    if (!m_connected)
        return;
    if (fromLauncher()) {
        // Not this app's key to remove: the person signs out in Redrob Launcher.
        setState(QStringLiteral("connected"), launcherSignedInText());
        return;
    }
    if (m_signOutWatcher.isRunning())
        return;
    setState(QStringLiteral("connected"), QStringLiteral("Disconnecting…"));
    m_signOutWatcher.setFuture(QtConcurrent::run([] {
        WaitResult result;
        result.ok = redrob_engine_sign_out() == REDROB_OK;
        if (!result.ok)
            result.error = lastError();
        return result;
    }));
}

void ConsoleConnection::finishSignOut()
{
    const WaitResult result = m_signOutWatcher.result();
    if (!result.ok) {
        setState(QStringLiteral("connected"),
                 QStringLiteral("Could not disconnect: %1").arg(result.error));
        return;
    }
    m_connected = false;
    m_source.clear();
    setState(QStringLiteral("idle"),
             QStringLiteral("Disconnected. The key still exists in the console until you revoke it there"));
    emit keyChanged();
    refresh();
}

void ConsoleConnection::openVerificationPage() const
{
    const QUrl url(m_verificationUrl);
    // Only ever hand the browser an https page the console named.
    if (url.isValid() && url.scheme() == QStringLiteral("https"))
        QDesktopServices::openUrl(url);
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
