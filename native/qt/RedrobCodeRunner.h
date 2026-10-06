// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QJsonObject>
#include <QObject>
#include <QProcess>
#include <QString>
#include <QStringList>

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

public:
    explicit RedrobCodeRunner(QObject *parent = nullptr);
    ~RedrobCodeRunner() override;

    bool running() const;
    bool available() const;
    QStringList log() const { return m_log; }
    QString status() const { return m_status; }

    // `configJson` is the redrob.json content naming the canvas MCP server. Returns false (and sets
    // status) when a task is already running, the task is empty, or redrob is not installed.
    bool start(const QString &task, const QByteArray &configJson);
    Q_INVOKABLE void stop();
    Q_INVOKABLE void refreshAvailability();

    // Exposed for the guard tests: the permission block every run is started with.
    static QByteArray lockedDownConfig(const QJsonObject &mcpServers);
    // One line of `--format json` output to a short human line, or empty to skip it.
    static QString describeEvent(const QByteArray &line);

signals:
    void runningChanged();
    void availableChanged();
    void logChanged();
    void statusChanged();

private:
    void appendLog(const QString &line);
    void setStatus(const QString &status);
    void readOutput();
    void finished(int exitCode, QProcess::ExitStatus exitStatus);

    QString m_executable;
    std::unique_ptr<QProcess> m_process;
    std::unique_ptr<QTemporaryDir> m_workDir;
    QByteArray m_pending;
    QStringList m_log;
    QString m_status;
};
