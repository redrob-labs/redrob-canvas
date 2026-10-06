// SPDX-License-Identifier: GPL-3.0-or-later
#include "RedrobCodeRunner.h"

#include <QJsonDocument>
#include <QJsonParseError>
#include <QProcessEnvironment>
#include <QStandardPaths>
#include <QTemporaryDir>

namespace {
// The log is a window onto the run, not a transcript; keep the last lines only.
constexpr int kMaxLogLines = 200;
// A task is a sentence or a paragraph. Bounding it keeps the argv sane.
constexpr int kMaxTaskChars = 8000;
} // namespace

RedrobCodeRunner::RedrobCodeRunner(QObject *parent)
    : QObject(parent)
{
    refreshAvailability();
}

RedrobCodeRunner::~RedrobCodeRunner()
{
    stop();
}

bool RedrobCodeRunner::running() const
{
    return m_process && m_process->state() != QProcess::NotRunning;
}

bool RedrobCodeRunner::available() const { return !m_executable.isEmpty(); }

void RedrobCodeRunner::refreshAvailability()
{
    const QString found = QStandardPaths::findExecutable(QStringLiteral("redrob"));
    if (found == m_executable)
        return;
    m_executable = found;
    emit availableChanged();
}

QByteArray RedrobCodeRunner::lockedDownConfig(const QJsonObject &mcpServers)
{
    // Keys from redrob-code's documented permission set. The run may only call the canvas tools:
    // everything that could touch files, run commands, reach the network or ask a question nobody
    // is there to answer is denied. An unknown key would hard-fail redrob's config load, so only
    // documented keys appear here.
    const QJsonObject permission{
        {QStringLiteral("edit"), QStringLiteral("deny")},
        {QStringLiteral("bash"), QStringLiteral("deny")},
        {QStringLiteral("task"), QStringLiteral("deny")},
        {QStringLiteral("webfetch"), QStringLiteral("deny")},
        {QStringLiteral("websearch"), QStringLiteral("deny")},
        {QStringLiteral("external_directory"), QStringLiteral("deny")},
        {QStringLiteral("question"), QStringLiteral("deny")},
        {QStringLiteral("skill"), QStringLiteral("deny")},
    };
    const QJsonObject config{
        {QStringLiteral("$schema"), QStringLiteral("https://code.redrob.ai/config.json")},
        {QStringLiteral("mcp"), mcpServers},
        {QStringLiteral("permission"), permission},
    };
    return QJsonDocument(config).toJson(QJsonDocument::Compact);
}

QString RedrobCodeRunner::describeEvent(const QByteArray &line)
{
    QJsonParseError error{};
    const QJsonDocument doc = QJsonDocument::fromJson(line, &error);
    if (error.error != QJsonParseError::NoError || !doc.isObject())
        return {};
    const QJsonObject event = doc.object();
    const QString type = event.value(QStringLiteral("type")).toString();
    const QJsonObject part = event.value(QStringLiteral("part")).toObject();
    if (type == QStringLiteral("text")) {
        const QString text = part.value(QStringLiteral("text")).toString().trimmed();
        return text.isEmpty() ? QString() : text;
    }
    if (type == QStringLiteral("tool_use")) {
        const QString tool = part.value(QStringLiteral("tool")).toString();
        const QString state = part.value(QStringLiteral("state")).toObject().value(QStringLiteral("status")).toString();
        return QStringLiteral("tool %1 %2").arg(tool, state).trimmed();
    }
    if (type == QStringLiteral("error")) {
        const QJsonObject err = event.value(QStringLiteral("error")).toObject();
        QString message = err.value(QStringLiteral("data")).toObject().value(QStringLiteral("message")).toString();
        if (message.isEmpty())
            message = err.value(QStringLiteral("name")).toString();
        return QStringLiteral("error: %1").arg(message.isEmpty() ? QStringLiteral("unknown") : message);
    }
    return {};
}

bool RedrobCodeRunner::start(const QString &task, const QByteArray &configJson)
{
    if (running()) {
        setStatus(QStringLiteral("A redrob-code task is already running"));
        return false;
    }
    const QString trimmed = task.trimmed();
    // A task starting with a dash would be read as an option, so it is refused rather than
    // relying on `--`, whose handling depends on the CLI parser's configuration.
    if (trimmed.isEmpty() || trimmed.size() > kMaxTaskChars || trimmed.startsWith(QLatin1Char('-'))) {
        setStatus(QStringLiteral("Describe the task in 1 to %1 characters, not starting with '-'").arg(kMaxTaskChars));
        return false;
    }
    refreshAvailability();
    if (m_executable.isEmpty()) {
        setStatus(QStringLiteral("redrob-code is not installed: the redrob command was not found on PATH"));
        return false;
    }
    m_workDir = std::make_unique<QTemporaryDir>();
    if (!m_workDir->isValid()) {
        setStatus(QStringLiteral("Could not create a private folder for the run"));
        m_workDir.reset();
        return false;
    }

    m_process = std::make_unique<QProcess>();
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.insert(QStringLiteral("REDROB_CONFIG_CONTENT"), QString::fromUtf8(configJson));
    // The user's project config could re-enable tools the run must not have.
    env.insert(QStringLiteral("REDROB_DISABLE_PROJECT_CONFIG"), QStringLiteral("1"));
    m_process->setProcessEnvironment(env);
    m_process->setWorkingDirectory(m_workDir->path());
    m_process->setProcessChannelMode(QProcess::SeparateChannels);
    connect(m_process.get(), &QProcess::readyReadStandardOutput, this, &RedrobCodeRunner::readOutput);
    connect(m_process.get(), &QProcess::finished, this, &RedrobCodeRunner::finished);
    connect(m_process.get(), &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart) {
            setStatus(QStringLiteral("redrob-code failed to start"));
            emit runningChanged();
        }
    });

    m_log.clear();
    m_pending.clear();
    emit logChanged();
    // The task goes in as a single argv entry (no shell).
    m_process->start(m_executable,
                     {QStringLiteral("run"), QStringLiteral("--format"), QStringLiteral("json"),
                      QStringLiteral("--dir"), m_workDir->path(), trimmed});
    setStatus(QStringLiteral("redrob-code is working · edits arrive as proposals"));
    emit runningChanged();
    return true;
}

void RedrobCodeRunner::stop()
{
    if (!running())
        return;
    m_process->terminate();
    if (!m_process->waitForFinished(2000))
        m_process->kill();
}

void RedrobCodeRunner::readOutput()
{
    m_pending += m_process->readAllStandardOutput();
    qsizetype newline = 0;
    while ((newline = m_pending.indexOf('\n')) >= 0) {
        const QByteArray line = m_pending.left(newline).trimmed();
        m_pending.remove(0, newline + 1);
        const QString described = describeEvent(line);
        if (!described.isEmpty())
            appendLog(described);
    }
}

void RedrobCodeRunner::finished(int exitCode, QProcess::ExitStatus exitStatus)
{
    readOutput();
    if (exitStatus != QProcess::NormalExit)
        setStatus(QStringLiteral("redrob-code stopped"));
    else if (exitCode != 0)
        setStatus(QStringLiteral("redrob-code exited with code %1").arg(exitCode));
    else
        setStatus(QStringLiteral("redrob-code finished · review its proposals below"));
    m_workDir.reset();
    emit runningChanged();
}

void RedrobCodeRunner::appendLog(const QString &line)
{
    m_log.append(line);
    while (m_log.size() > kMaxLogLines)
        m_log.removeFirst();
    emit logChanged();
}

void RedrobCodeRunner::setStatus(const QString &status)
{
    if (status == m_status)
        return;
    m_status = status;
    emit statusChanged();
}
