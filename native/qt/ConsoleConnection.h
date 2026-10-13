// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include "SharedAccount.h"

#include <QByteArray>
#include <QFileSystemWatcher>
#include <QFutureWatcher>
#include <QObject>
#include <QString>

struct RedrobDeviceFlow;

// A3: connect this app to the Redrob console with the device flow, so the in-app agent gets a
// workspace key without anyone pasting one.
//
// connect() asks the console for a code, opens the console's confirm page, and waits in the
// background. When the person approves, the key is stored in the app's config folder in a file
// only this user can read, and keyChanged() fires. disconnect() deletes that file. The device code
// itself never leaves the engine (redrob_device_flow_*).
//
// REDROB_API_KEY in the environment still wins: a key set by the person launching the app is not
// replaced by a stored one, and disconnecting does not clear it.
//
// When this app has no key of its own -- nothing in the environment, nothing stored -- it falls
// back to the computer's shared sign-in from Redrob Launcher (SharedAccount). That file is read
// fresh whenever the key is needed and whenever it changes on disk, so signing in or out in the
// launcher is picked up while this app is open. It is never written here: disconnecting removes
// only this app's own key, and the UI says "Signed in with Redrob Launcher" instead of offering to
// disconnect.
class ConsoleConnection : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString state READ state NOTIFY changed)
    Q_PROPERTY(QString userCode READ userCode NOTIFY changed)
    Q_PROPERTY(QString verificationUrl READ verificationUrl NOTIFY changed)
    Q_PROPERTY(QString message READ message NOTIFY changed)
    Q_PROPERTY(bool connected READ connected NOTIFY changed)
    Q_PROPERTY(bool fromEnvironment READ fromEnvironment CONSTANT)
    Q_PROPERTY(bool fromLauncher READ fromLauncher NOTIFY changed)
    Q_PROPERTY(QString accountName READ accountName NOTIFY changed)
    Q_PROPERTY(QString keyName READ keyName NOTIFY changed)
    Q_PROPERTY(QString launcherSignedInText READ launcherSignedInText CONSTANT)

public:
    explicit ConsoleConnection(QObject *parent = nullptr);
    ~ConsoleConnection() override;

    // "idle", "starting", "waiting" (code shown), or "connected".
    QString state() const { return m_state; }
    QString userCode() const { return m_userCode; }
    QString verificationUrl() const { return m_verificationUrl; }
    QString message() const { return m_message; }
    bool connected() const { return !key().isEmpty(); }
    bool fromEnvironment() const { return m_fromEnvironment; }
    // The key in use is Redrob Launcher's shared one: this app has none of its own.
    bool fromLauncher() const { return m_key.isEmpty() && m_shared.signedIn(); }
    // Shown for the launcher's key only; this app's own key carries no names.
    QString accountName() const { return fromLauncher() ? m_shared.accountName : QString{}; }
    QString keyName() const { return fromLauncher() ? m_shared.apiKeyName : QString{}; }
    // This app's own key when it has one, else the shared one. Call refresh() first when the
    // answer must reflect the shared file as it is right now.
    QByteArray key() const { return m_key.isEmpty() ? m_shared.apiKey : m_key; }

    Q_INVOKABLE void connectToConsole();
    Q_INVOKABLE void cancel();
    Q_INVOKABLE void disconnectFromConsole();
    Q_INVOKABLE void openVerificationPage() const;
    // Re-read the shared account file; emits changed()/keyChanged() when the key in use moved.
    Q_INVOKABLE void refresh();

    // "Signed in with Redrob Launcher", in the system language where a translation exists.
    static QString launcherSignedInText();

    // Where the stored key lives; exposed for the guard tests.
    static QString keyFilePath();

signals:
    void changed();
    void keyChanged();

private:
    struct StartResult {
        RedrobDeviceFlow *flow = nullptr;
        QByteArray json;
        QString error;
    };
    struct WaitResult {
        QByteArray key;
        QString error;
    };

    void finishStart();
    void finishWait();
    void setState(const QString &state, const QString &message = {});
    bool storeKey(const QByteArray &key);
    void releaseFlow();
    void watchSharedAccount();

    QString m_state = QStringLiteral("idle");
    QString m_userCode;
    QString m_verificationUrl;
    QString m_message;
    // This app's own key: REDROB_API_KEY, or the one its console sign-in stored. Empty when the
    // app has none, in which case m_shared is what key() answers with.
    QByteArray m_key;
    bool m_fromEnvironment = false;
    SharedAccount m_shared;
    QFileSystemWatcher m_sharedWatcher;
    RedrobDeviceFlow *m_flow = nullptr;
    QFutureWatcher<StartResult> m_startWatcher;
    QFutureWatcher<WaitResult> m_waitWatcher;
};
