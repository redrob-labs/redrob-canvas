// SPDX-License-Identifier: GPL-3.0-or-later
#include "McpServer.h"
#include "OwnerOnlyFile.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QRandomGenerator>
#include <QSaveFile>
#include <QStandardPaths>
#include <QTcpSocket>

namespace {

constexpr auto kProtocolVersion = "2025-03-26";

QByteArray jsonBody(const QJsonObject &object)
{
    return QJsonDocument(object).toJson(QJsonDocument::Compact);
}

QByteArray errorBody(const QString &message)
{
    return jsonBody({{QStringLiteral("error"), message}});
}

QJsonObject rpcError(const QJsonValue &id, int code, const QString &message)
{
    return {{QStringLiteral("jsonrpc"), QStringLiteral("2.0")},
            {QStringLiteral("id"), id},
            {QStringLiteral("error"),
             QJsonObject{{QStringLiteral("code"), code}, {QStringLiteral("message"), message}}}};
}

QJsonObject rpcResult(const QJsonValue &id, const QJsonObject &result)
{
    return {{QStringLiteral("jsonrpc"), QStringLiteral("2.0")},
            {QStringLiteral("id"), id},
            {QStringLiteral("result"), result}};
}

QByteArray reasonPhrase(int status)
{
    switch (status) {
    case 200: return "OK";
    case 202: return "Accepted";
    case 400: return "Bad Request";
    case 401: return "Unauthorized";
    case 403: return "Forbidden";
    case 404: return "Not Found";
    case 405: return "Method Not Allowed";
    case 413: return "Payload Too Large";
    default: return "Error";
    }
}

} // namespace

McpServer::McpServer(QObject *parent)
    : QObject(parent)
{
    connect(&m_server, &QTcpServer::newConnection, this, &McpServer::onNewConnection);
}

McpServer::~McpServer()
{
    stop();
}

bool McpServer::start(QString *error)
{
    if (m_server.isListening())
        return true;
    // 32 random bytes from the OS generator: a token a local process cannot guess.
    QByteArray raw(32, Qt::Uninitialized);
    QRandomGenerator::system()->fillRange(reinterpret_cast<quint32 *>(raw.data()),
                                          raw.size() / int(sizeof(quint32)));
    m_token = raw.toHex();
    // Loopback only, port chosen by the OS. QHostAddress::LocalHost is 127.0.0.1, never 0.0.0.0.
    if (!m_server.listen(QHostAddress::LocalHost, 0)) {
        if (error)
            *error = m_server.errorString();
        m_token.clear();
        return false;
    }
    if (!writeConnectionFile(error)) {
        m_server.close();
        m_token.clear();
        return false;
    }
    return true;
}

void McpServer::stop()
{
    if (m_server.isListening())
        m_server.close();
    for (auto it = m_buffers.cbegin(); it != m_buffers.cend(); ++it)
        it.key()->abort();
    m_buffers.clear();
    m_token.clear();
    QFile::remove(connectionFilePath());
}

bool McpServer::isListening() const { return m_server.isListening(); }
quint16 McpServer::port() const { return m_server.isListening() ? m_server.serverPort() : 0; }
QString McpServer::token() const { return QString::fromLatin1(m_token); }

QString McpServer::connectionFilePath() const
{
    return QStandardPaths::writableLocation(QStandardPaths::AppConfigLocation)
        + QStringLiteral("/mcp-connection.json");
}

bool McpServer::writeConnectionFile(QString *error) const
{
    const QString path = connectionFilePath();
    QDir().mkpath(QFileInfo(path).absolutePath());
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) {
        if (error)
            *error = file.errorString();
        return false;
    }
    // Owner read/write only, set BEFORE the token is written.
    file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner);
    file.write(jsonBody({{QStringLiteral("url"),
                          QStringLiteral("http://127.0.0.1:%1/mcp").arg(port())},
                         {QStringLiteral("token"), QString::fromLatin1(m_token)}}));
    if (!file.commit()) {
        if (error)
            *error = file.errorString();
        return false;
    }
    if (!OwnerOnlyFile::restrictToOwner(path)) {
        QFile::remove(path);
        if (error)
            *error = QStringLiteral("could not restrict the connection file to its owner");
        return false;
    }
    return true;
}

void McpServer::setToolsJson(const QByteArray &toolsJson) { m_toolsJson = toolsJson; }
void McpServer::setCallHandler(CallHandler handler) { m_handler = std::move(handler); }

bool McpServer::tokenMatches(const QByteArray &presented, const QByteArray &expected)
{
    // Constant time in the presented length: no early exit on the first differing byte.
    if (expected.isEmpty() || presented.size() != expected.size())
        return false;
    unsigned char difference = 0;
    for (qsizetype i = 0; i < expected.size(); ++i)
        difference |= static_cast<unsigned char>(presented[i] ^ expected[i]);
    return difference == 0;
}

McpServer::Reply McpServer::handle(const QByteArray &method, const QByteArray &path,
                                   const QHash<QByteArray, QByteArray> &headers,
                                   const QByteArray &body) const
{
    if (path != "/mcp")
        return {404, errorBody(QStringLiteral("not found"))};
    // A browser always sends Origin on a cross-origin POST; an MCP client does not. Refusing it
    // keeps any web page the user visits from reaching the canvas.
    if (headers.contains("origin"))
        return {403, errorBody(QStringLiteral("browser origins are not accepted"))};
    const QByteArray host = headers.value("host");
    const QByteArray expectedPort = QByteArray::number(port());
    if (host != "127.0.0.1:" + expectedPort && host != "localhost:" + expectedPort)
        return {403, errorBody(QStringLiteral("unexpected Host header"))};
    const QByteArray authorization = headers.value("authorization");
    if (!authorization.startsWith("Bearer ")
        || !tokenMatches(authorization.mid(int(sizeof("Bearer ") - 1)), m_token))
        return {401, errorBody(QStringLiteral("missing or wrong bearer token"))};
    if (method != "POST")
        return {405, errorBody(QStringLiteral("only POST is supported"))};
    if (body.size() > kMaxBodyBytes)
        return {413, errorBody(QStringLiteral("request too large"))};

    QJsonParseError parseError;
    const QJsonDocument document = QJsonDocument::fromJson(body, &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject())
        return {400, jsonBody(rpcError(QJsonValue::Null, -32700, QStringLiteral("parse error")))};
    bool notification = false;
    const QJsonObject response = dispatch(document.object(), &notification);
    if (notification)
        return {202, {}};
    return {200, jsonBody(response)};
}

QJsonObject McpServer::dispatch(const QJsonObject &request, bool *isNotification) const
{
    const QJsonValue id = request.value(QStringLiteral("id"));
    const QString method = request.value(QStringLiteral("method")).toString();
    *isNotification = !request.contains(QStringLiteral("id"));
    if (request.value(QStringLiteral("jsonrpc")).toString() != QStringLiteral("2.0") || method.isEmpty())
        return rpcError(id, -32600, QStringLiteral("invalid request"));
    const QJsonObject params = request.value(QStringLiteral("params")).toObject();

    if (method == QStringLiteral("initialize")) {
        const QString asked = params.value(QStringLiteral("protocolVersion")).toString();
        return rpcResult(
            id, {{QStringLiteral("protocolVersion"),
                  asked.isEmpty() ? QString::fromLatin1(kProtocolVersion) : asked},
                 {QStringLiteral("capabilities"),
                  QJsonObject{{QStringLiteral("tools"), QJsonObject{}}}},
                 {QStringLiteral("serverInfo"),
                  QJsonObject{{QStringLiteral("name"), QStringLiteral("redrob-canvas")},
                              {QStringLiteral("version"), QStringLiteral("1")}}},
                 {QStringLiteral("instructions"),
                  QStringLiteral("Every editing tool queues a proposal in Redrob Canvas. Nothing "
                                 "changes until the user approves it there.")}});
    }
    if (method == QStringLiteral("ping"))
        return rpcResult(id, {});
    if (method.startsWith(QStringLiteral("notifications/")))
        return {};
    if (method == QStringLiteral("tools/list")) {
        const QJsonDocument tools = QJsonDocument::fromJson(m_toolsJson);
        return rpcResult(id, tools.isObject() ? tools.object()
                                              : QJsonObject{{QStringLiteral("tools"), QJsonArray{}}});
    }
    if (method == QStringLiteral("tools/call")) {
        const QString name = params.value(QStringLiteral("name")).toString();
        if (name.isEmpty())
            return rpcError(id, -32602, QStringLiteral("tools/call needs a tool name"));
        if (!m_handler)
            return rpcError(id, -32603, QStringLiteral("the canvas is not ready"));
        const QJsonObject outcome = m_handler(name, params.value(QStringLiteral("arguments")).toObject());
        if (outcome.contains(QStringLiteral("error"))) {
            // A tool failure is a result with isError, not a protocol error (MCP tools spec).
            return rpcResult(
                id, {{QStringLiteral("isError"), true},
                     {QStringLiteral("content"),
                      QJsonArray{QJsonObject{{QStringLiteral("type"), QStringLiteral("text")},
                                             {QStringLiteral("text"),
                                              outcome.value(QStringLiteral("error")).toString()}}}}});
        }
        return rpcResult(id, outcome);
    }
    return rpcError(id, -32601, QStringLiteral("method not found: %1").arg(method));
}

void McpServer::onNewConnection()
{
    while (QTcpSocket *socket = m_server.nextPendingConnection()) {
        // Belt and braces: the listener is loopback-only, but refuse any peer that is not.
        if (!socket->peerAddress().isLoopback()) {
            socket->abort();
            socket->deleteLater();
            continue;
        }
        m_buffers.insert(socket, {});
        connect(socket, &QTcpSocket::readyRead, this, [this, socket] { onReadyRead(socket); });
        connect(socket, &QTcpSocket::disconnected, this, [this, socket] {
            m_buffers.remove(socket);
            socket->deleteLater();
        });
    }
}

void McpServer::onReadyRead(QTcpSocket *socket)
{
    QByteArray &buffer = m_buffers[socket];
    buffer += socket->readAll();
    const qsizetype headerEnd = buffer.indexOf("\r\n\r\n");
    if (headerEnd < 0) {
        if (buffer.size() > kMaxHeaderBytes) {
            writeReply(socket, {413, errorBody(QStringLiteral("headers too large"))});
            m_buffers.remove(socket);
        }
        return;
    }
    const QList<QByteArray> lines = buffer.left(headerEnd).split('\n');
    const QList<QByteArray> requestLine = lines.value(0).trimmed().split(' ');
    QHash<QByteArray, QByteArray> headers;
    for (qsizetype i = 1; i < lines.size(); ++i) {
        const qsizetype colon = lines[i].indexOf(':');
        if (colon > 0)
            headers.insert(lines[i].left(colon).trimmed().toLower(), lines[i].mid(colon + 1).trimmed());
    }
    bool lengthOk = true;
    const qsizetype length = headers.contains("content-length")
        ? headers.value("content-length").toLongLong(&lengthOk)
        : 0;
    if (!lengthOk || length < 0 || length > kMaxBodyBytes) {
        writeReply(socket, {413, errorBody(QStringLiteral("bad or too large Content-Length"))});
        m_buffers.remove(socket);
        return;
    }
    const qsizetype bodyStart = headerEnd + 4;
    if (buffer.size() - bodyStart < length)
        return; // wait for the rest of the body
    const QByteArray body = buffer.mid(bodyStart, length);
    m_buffers.remove(socket);
    writeReply(socket, handle(requestLine.value(0), requestLine.value(1), headers, body));
}

void McpServer::writeReply(QTcpSocket *socket, const Reply &reply)
{
    QByteArray response = "HTTP/1.1 " + QByteArray::number(reply.status) + ' '
        + reasonPhrase(reply.status) + "\r\n";
    if (!reply.body.isEmpty())
        response += "Content-Type: application/json\r\n";
    response += "Content-Length: " + QByteArray::number(reply.body.size()) + "\r\n";
    response += "Connection: close\r\n\r\n";
    response += reply.body;
    socket->write(response);
    socket->disconnectFromHost();
}
