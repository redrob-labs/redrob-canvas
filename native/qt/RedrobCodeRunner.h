// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QJsonObject>
#include <QObject>
#include <QProcess>
#include <QString>
#include <QStringList>
#include <QVariantList>

#include <memory>

class QTemporaryDir;

// A2: run one redrob-code task against this window's MCP endpoint.
//
// Starts `redrob run --format json <task>` with the canvas server injected through
// REDROB_CONFIG_CONTENT, so the bearer token never touches disk. The agent can only reach the
// canvas tools: file edits, shell, web fetches and directories outside its empty scratch folder
// are denied in that same config, project config is ignored, and the run's working directory is a
// fresh private temporary folder. Every edit still arrives as a proposal in the Agent tab; nothing
// here applies one.
class RedrobCodeRunner : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool running READ running NOTIFY runningChanged)
    Q_PROPERTY(bool available READ available NOTIFY availableChanged)
    Q_PROPERTY(QStringList log READ log NOTIFY logChanged)
    Q_PROPERTY(QString status READ status NOTIFY statusChanged)
    // The Agent chat: [{role: user|assistant|tool|error, text}], oldest first. A follow-up
    // message continues the same redrob-code session (`--session`), so it remembers the thread.
    Q_PROPERTY(QVariantList messages READ messages NOTIFY messagesChanged)
    Q_PROPERTY(bool inConversation READ inConversation NOTIFY messagesChanged)

public:
    explicit RedrobCodeRunner(QObject *parent = nullptr);
    ~RedrobCodeRunner() override;

    bool running() const;
    bool available() const;
    QStringList log() const { return m_log; }
    QString status() const { return m_status; }
    QVariantList messages() const { return m_messages; }
    bool inConversation() const { return !m_sessionId.isEmpty(); }

    // `configJson` is the redrob.json content naming the canvas MCP server. Returns false (and sets
    // status) when a task is already running, the task is empty, or redrob is not installed.
    // Each call is one chat turn: the text joins the thread as the user's message.
    bool start(const QString &task, const QByteArray &configJson);
    Q_INVOKABLE void stop();
    Q_INVOKABLE void refreshAvailability();
    // Forget the thread: the next message starts a new redrob-code session.
    Q_INVOKABLE void newChat();
    // For the hosted agent, which answers in the same thread.
    void addMessage(const QString &role, const QString &text);

    // Exposed for the guard tests: the permission block every run is started with.
    static QByteArray lockedDownConfig(const QJsonObject &mcpServers);
    // One line of `--format json` output to a short human line, or empty to skip it.
    static QString describeEvent(const QByteArray &line);
    // The run's arguments: `--session <id>` continues the thread when there is one.
    static QStringList runArguments(const QString &workDir, const QString &sessionId, const QString &task);

signals:
    void runningChanged();
    void availableChanged();
    void logChanged();
    void statusChanged();
    void messagesChanged();

private:
    void appendLog(const QString &line);
    void setStatus(const QString &status);
    void readOutput();
    void finished(int exitCode, QProcess::ExitStatus exitStatus);
    void handleEvent(const QByteArray &line);

    QString m_executable;
    std::unique_ptr<QProcess> m_process;
    // One private folder per conversation: redrob-code keys sessions by directory.
    std::unique_ptr<QTemporaryDir> m_workDir;
    QByteArray m_pending;
    // The last stderr line of the run, shown when it exits non-zero.
    QString m_lastError;
    QStringList m_log;
    QString m_status;
    QVariantList m_messages;
    QString m_sessionId;
    // The text part being streamed into the last assistant message, by part id.
    QString m_textPartId;
};
