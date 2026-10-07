// SPDX-License-Identifier: GPL-3.0-or-later
#include "RedrobCodeRunner.h"

#include <QJsonDocument>
#include <QJsonParseError>
#include <QProcessEnvironment>
#include <QRegularExpression>
#include <QStandardPaths>
#include <QTemporaryDir>
#include <QVariantMap>

namespace {
// The log is a window onto the run, not a transcript; keep the last lines only.
constexpr int kMaxLogLines = 200;
// The chat keeps the last messages only.
constexpr int kMaxMessages = 300;
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
    // The conversation keeps its private folder: redrob-code keys sessions by directory.
    if (!m_workDir)
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
    // `redrob run` reads a message from stdin when stdin is not a terminal, and waits for its
    // end. QProcess leaves stdin an open pipe, so the run sat at "init" forever. No input.
    m_process->setStandardInputFile(QProcess::nullDevice());
    connect(m_process.get(), &QProcess::readyReadStandardOutput, this, &RedrobCodeRunner::readOutput);
    // Drained so a chatty stderr cannot fill the pipe and stall the run; the last line is kept
    // for the status when the run fails.
    connect(m_process.get(), &QProcess::readyReadStandardError, this, [this]() {
        const QList<QByteArray> lines = m_process->readAllStandardError().split('\n');
        for (const QByteArray &line : lines) {
            if (!line.trimmed().isEmpty())
                m_lastError = QString::fromUtf8(line.trimmed()).left(200);
        }
    });
    connect(m_process.get(), &QProcess::finished, this, &RedrobCodeRunner::finished);
    connect(m_process.get(), &QProcess::errorOccurred, this, [this](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart) {
            setStatus(QStringLiteral("redrob-code failed to start"));
            emit runningChanged();
        }
    });

    m_log.clear();
    m_lastError.clear();
    m_pending.clear();
    m_textPartId.clear();
    emit logChanged();
    addMessage(QStringLiteral("user"), trimmed);
    // The task goes in as a single argv entry (no shell).
    m_process->start(m_executable, runArguments(m_workDir->path(), m_sessionId, trimmed));
    setStatus(QStringLiteral("redrob-code is working · edits arrive as proposals"));
    emit runningChanged();
    return true;
}

QStringList RedrobCodeRunner::runArguments(const QString &workDir, const QString &sessionId, const QString &task)
{
    QStringList args{QStringLiteral("run"), QStringLiteral("--format"), QStringLiteral("json"),
                     QStringLiteral("--dir"), workDir};
    // Session ids come from redrob's own events ("ses_..."); anything else is not passed on.
    static const QRegularExpression sessionShape(QStringLiteral("^ses_[A-Za-z0-9]{8,64}$"));
    if (sessionShape.match(sessionId).hasMatch())
        args << QStringLiteral("--session") << sessionId;
    args << task;
    return args;
}

void RedrobCodeRunner::newChat()
{
    stop();
    m_sessionId.clear();
    m_textPartId.clear();
    m_workDir.reset();
    m_messages.clear();
    m_log.clear();
    emit logChanged();
    emit messagesChanged();
    setStatus(QString());
}

void RedrobCodeRunner::addMessage(const QString &role, const QString &text)
{
    const QString trimmed = text.trimmed();
    if (trimmed.isEmpty())
        return;
    m_messages.append(QVariantMap{{QStringLiteral("role"), role}, {QStringLiteral("text"), trimmed}});
    while (m_messages.size() > kMaxMessages)
        m_messages.removeFirst();
    emit messagesChanged();
}

void RedrobCodeRunner::handleEvent(const QByteArray &line)
{
    QJsonParseError error{};
    const QJsonDocument doc = QJsonDocument::fromJson(line, &error);
    if (error.error != QJsonParseError::NoError || !doc.isObject())
        return;
    const QJsonObject event = doc.object();
    const QString session = event.value(QStringLiteral("sessionID")).toString();
    if (!session.isEmpty() && session != m_sessionId) {
        m_sessionId = session;
        emit messagesChanged();
    }
    const QString type = event.value(QStringLiteral("type")).toString();
    const QJsonObject part = event.value(QStringLiteral("part")).toObject();
    if (type == QStringLiteral("text")) {
        const QString text = part.value(QStringLiteral("text")).toString().trimmed();
        if (text.isEmpty())
            return;
        const QString partId = part.value(QStringLiteral("id")).toString();
        // A text part can be sent again as it grows; replace the bubble rather than repeat it.
        if (!partId.isEmpty() && partId == m_textPartId && !m_messages.isEmpty()) {
            QVariantMap last = m_messages.last().toMap();
            last.insert(QStringLiteral("text"), text);
            m_messages.last() = last;
            emit messagesChanged();
            return;
        }
        m_textPartId = partId;
        addMessage(QStringLiteral("assistant"), text);
        return;
    }
    m_textPartId.clear();
    const QString described = describeEvent(line);
    if (type == QStringLiteral("tool_use") && !described.isEmpty())
        addMessage(QStringLiteral("tool"), described);
    else if (type == QStringLiteral("error") && !described.isEmpty())
        addMessage(QStringLiteral("error"), described);
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
        handleEvent(line);
    }
}

void RedrobCodeRunner::finished(int exitCode, QProcess::ExitStatus exitStatus)
{
    readOutput();
    if (exitStatus != QProcess::NormalExit)
        setStatus(QStringLiteral("redrob-code stopped"));
    else if (exitCode != 0)
        setStatus(m_lastError.isEmpty()
                      ? QStringLiteral("redrob-code exited with code %1").arg(exitCode)
                      : QStringLiteral("redrob-code exited with code %1: %2").arg(exitCode).arg(m_lastError));
    else
        setStatus(QStringLiteral("redrob-code finished · review its proposals below"));
    if (exitStatus != QProcess::NormalExit || exitCode != 0)
        addMessage(QStringLiteral("error"), m_status);
    // The folder stays for the next turn of this conversation; newChat() removes it.
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
