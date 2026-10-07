// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QByteArray>
#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QTcpServer>

#include <functional>

class QTcpSocket;

// P13. A Model Context Protocol endpoint so redrob-code (or any MCP client) can drive the canvas's
// graphics tools. Security posture, each point enforced in handle():
//   - binds 127.0.0.1 only, on a port the OS picks; never another interface;
//   - every request must carry `Authorization: Bearer <token>`, a fresh 256-bit token per start,
//     compared in constant time;
//   - a request with an Origin header is refused (a browser page cannot reach it), and the Host
//     header must name the loopback address and this port (DNS-rebinding guard);
//   - off by default and not remembered across launches: the user turns it on each session;
//   - tools/call never edits: the handler queues a proposal the user must approve in the app.
class McpServer final : public QObject
{
    Q_OBJECT

public:
    // Returns an MCP tools/call result object, or one with "error" set to a message.
    using CallHandler = std::function<QJsonObject(const QString &name, const QJsonObject &arguments)>;

    struct Reply
    {
        int status = 200;
        QByteArray body;
    };

    static constexpr qsizetype kMaxBodyBytes = 1024 * 1024;
    static constexpr qsizetype kMaxHeaderBytes = 16 * 1024;

    explicit McpServer(QObject *parent = nullptr);
    ~McpServer() override;

    bool start(QString *error);
    void stop();
    bool isListening() const;
    quint16 port() const;
    QString token() const;
    // Where the connection details are written (owner-only permissions), so a client can be
    // pointed at a file instead of a pasted token.
    QString connectionFilePath() const;

    void setToolsJson(const QByteArray &toolsJson);
    void setCallHandler(CallHandler handler);

    // Exposed for tests: the whole decision for one parsed request. Header names are lowercase.
    Reply handle(const QByteArray &method, const QByteArray &path,
                 const QHash<QByteArray, QByteArray> &headers, const QByteArray &body) const;
    static bool tokenMatches(const QByteArray &presented, const QByteArray &expected);

private:
    void onNewConnection();
    void onReadyRead(QTcpSocket *socket);
    static void writeReply(QTcpSocket *socket, const Reply &reply);
    QJsonObject dispatch(const QJsonObject &request, bool *isNotification) const;
    bool writeConnectionFile(QString *error) const;

    QTcpServer m_server;
    QByteArray m_token;
    QByteArray m_toolsJson;
    CallHandler m_handler;
    QHash<QTcpSocket *, QByteArray> m_buffers;
};
