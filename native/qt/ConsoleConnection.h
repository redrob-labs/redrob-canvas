// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QByteArray>
#include <QFutureWatcher>
#include <QObject>
#include <QString>

struct RedrobDeviceFlow;

// A3: this app's view of the shared Redrob Code daemon's sign-in to the Redrob console.
//
// The daemon holds the console key and calls the console with it; this app never has one. What
// this object knows is whether the daemon is signed in (GET /provider, read through the engine)
// and with whose key: an app's (handed over through the device flow here or in another Redrob
// app), or Redrob Launcher's shared sign-in, which the engine reads itself.
//
// connectToConsole() asks the console for a code, opens the console's confirm page, and waits in
// the background; when the person approves, the engine takes the key (PUT /auth/redrob) and
// changed() fires with state "connected". The device code and the key both stay inside the engine
// (redrob_device_flow_*). disconnectFromConsole() removes the daemon's key (DELETE /auth/redrob).
//
// When no engine can be reached the state is "unavailable" and message() is what to show: the
// install sentence with the launcher link, or the start failure's reason. There is no fallback to
// the console from this app: two paths would hide whether the engine's masking applied.
//
// A key an earlier version of this app stored in its own config folder is migrated once: handed to
// the engine, then deleted.
class ConsoleConnection : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString state READ state NOTIFY changed)
    Q_PROPERTY(QString userCode READ userCode NOTIFY changed)
    Q_PROPERTY(QString verificationUrl READ verificationUrl NOTIFY changed)
    Q_PROPERTY(QString message READ message NOTIFY changed)
    Q_PROPERTY(bool connected READ connected NOTIFY changed)
    Q_PROPERTY(bool engineAvailable READ engineAvailable NOTIFY changed)
    Q_PROPERTY(bool fromLauncher READ fromLauncher NOTIFY changed)
    Q_PROPERTY(QString installUrl READ installUrl CONSTANT)
    Q_PROPERTY(QString launcherSignedInText READ launcherSignedInText CONSTANT)

public:
    explicit ConsoleConnection(QObject *parent = nullptr);
    ~ConsoleConnection() override;

    // "checking" (asking the engine), "idle" (engine up, not signed in), "starting", "waiting"
    // (code shown), "connected", or "unavailable" (no engine; message() says why).
    QString state() const { return m_state; }
    QString userCode() const { return m_userCode; }
    QString verificationUrl() const { return m_verificationUrl; }
    QString message() const { return m_message; }
    // The engine is signed in to the console, with whichever key.
    bool connected() const { return m_connected; }
    bool engineAvailable() const { return m_engineAvailable; }
    // The key in use is Redrob Launcher's shared one: this app offers no sign-out for it.
    bool fromLauncher() const { return m_connected && m_source == QStringLiteral("launcher"); }
    // Where Redrob Code comes from when it is not installed.
    static QString installUrl();

    Q_INVOKABLE void connectToConsole();
    Q_INVOKABLE void cancel();
    Q_INVOKABLE void disconnectFromConsole();
    Q_INVOKABLE void openVerificationPage() const;
    // Ask the engine again; emits changed() (and keyChanged() when the sign-in moved).
    Q_INVOKABLE void refresh();

    // "Signed in with Redrob Launcher", in the system language where a translation exists.
    static QString launcherSignedInText();

    // Where an earlier version stored its own key; read once for the migration, then removed.
    static QString keyFilePath();

signals:
    void changed();
    // The engine's sign-in moved (signed in, out, or the engine came or went).
    void keyChanged();

private:
    struct StatusResult {
        bool available = false;
        bool connected = false;
        QString source;
        QString error;
    };
    struct StartResult {
        RedrobDeviceFlow *flow = nullptr;
        QByteArray json;
        QString error;
    };
    struct WaitResult {
        bool ok = false;
        QString error;
    };

    void finishStatus();
    void finishStart();
    void finishWait();
    void finishSignOut();
    void applyStatus(const StatusResult &status);
    void migrateStoredKey();
    void setState(const QString &state, const QString &message = {});
    void releaseFlow();

    QString m_state = QStringLiteral("checking");
    QString m_userCode;
    QString m_verificationUrl;
    QString m_message;
    bool m_connected = false;
    bool m_engineAvailable = false;
    bool m_refreshAgain = false;
    QString m_source;
    RedrobDeviceFlow *m_flow = nullptr;
    QFutureWatcher<StatusResult> m_statusWatcher;
    QFutureWatcher<StartResult> m_startWatcher;
    QFutureWatcher<WaitResult> m_waitWatcher;
    QFutureWatcher<WaitResult> m_signOutWatcher;
};
