// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QByteArray>
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
class ConsoleConnection : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString state READ state NOTIFY changed)
    Q_PROPERTY(QString userCode READ userCode NOTIFY changed)
    Q_PROPERTY(QString verificationUrl READ verificationUrl NOTIFY changed)
    Q_PROPERTY(QString message READ message NOTIFY changed)
    Q_PROPERTY(bool connected READ connected NOTIFY changed)
    Q_PROPERTY(bool fromEnvironment READ fromEnvironment CONSTANT)

public:
    explicit ConsoleConnection(QObject *parent = nullptr);
    ~ConsoleConnection() override;

    // "idle", "starting", "waiting" (code shown), or "connected".
    QString state() const { return m_state; }
    QString userCode() const { return m_userCode; }
    QString verificationUrl() const { return m_verificationUrl; }
    QString message() const { return m_message; }
    bool connected() const { return !m_key.isEmpty(); }
    bool fromEnvironment() const { return m_fromEnvironment; }
    QByteArray key() const { return m_key; }

    Q_INVOKABLE void connectToConsole();
    Q_INVOKABLE void cancel();
    Q_INVOKABLE void disconnectFromConsole();
    Q_INVOKABLE void openVerificationPage() const;

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

    QString m_state = QStringLiteral("idle");
    QString m_userCode;
    QString m_verificationUrl;
    QString m_message;
    QByteArray m_key;
    bool m_fromEnvironment = false;
    RedrobDeviceFlow *m_flow = nullptr;
    QFutureWatcher<StartResult> m_startWatcher;
    QFutureWatcher<WaitResult> m_waitWatcher;
};
