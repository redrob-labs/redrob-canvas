// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QImage>
#include <QJsonObject>
#include <QNetworkAccessManager>
#include <QObject>
#include <QPointer>
#include <QProcess>
#include <QStringList>
#include <QTimer>
#include <QUrl>
#include <QVariantList>
#include <QVariantMap>

#include <functional>
#include <memory>

class EditorBridge;
class QNetworkReply;

// AI tools: IOPaint (Sanster/IOPaint 1.6.0, Apache-2.0) as a local engine.
//
// IOPaint's models are PyTorch/diffusers, so they run in IOPaint's own Python process, not in the
// Rust engine. This class installs IOPaint into an app-owned virtual environment, starts
// `iopaint start` bound to 127.0.0.1 on a free port, and speaks its HTTP API: inpaint (erase or
// replace inside the selection, with any of its erase or diffusion models), outpaint (pad the
// canvas, then fill the new border), and the plugins (remove background, anime segmentation,
// RealESRGAN upscale, GFPGAN and RestoreFormer face restore, click-to-segment). Every result lands
// as a NEW layer or as a selection, so nothing overwrites the user's pixels and Undo removes it.
//
// The server has no authentication of its own; it listens on loopback only and is started with no
// input folder, so its file manager stays off. It stops when the window closes.
class IopaintEngine : public QObject
{
    Q_OBJECT
    // missing · installing · stopped · starting · ready · busy · error
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    Q_PROPERTY(QString message READ message NOTIFY messageChanged)
    Q_PROPERTY(QStringList log READ log NOTIFY logChanged)
    Q_PROPERTY(QString model READ model NOTIFY configChanged)
    Q_PROPERTY(QVariantMap modelInfo READ modelInfo NOTIFY configChanged)
    Q_PROPERTY(QStringList downloadedModels READ downloadedModels NOTIFY configChanged)
    Q_PROPERTY(QStringList eraseModels READ eraseModels CONSTANT)
    Q_PROPERTY(QStringList diffusionModels READ diffusionModels CONSTANT)
    Q_PROPERTY(QStringList plugins READ plugins NOTIFY configChanged)
    Q_PROPERTY(QVariantMap pluginModels READ pluginModels NOTIFY configChanged)
    Q_PROPERTY(QStringList samplers READ samplers NOTIFY configChanged)
    Q_PROPERTY(QStringList enabledPlugins READ enabledPlugins WRITE setEnabledPlugins NOTIFY settingsChanged)
    Q_PROPERTY(QString device READ device WRITE setDevice NOTIFY settingsChanged)
    Q_PROPERTY(QString installMethod READ installMethod NOTIFY settingsChanged)

public:
    explicit IopaintEngine(EditorBridge *bridge, QObject *parent = nullptr);
    ~IopaintEngine() override;

    QString state() const { return m_state; }
    QString message() const { return m_message; }
    QStringList log() const { return m_log; }
    QString model() const { return m_model; }
    QVariantMap modelInfo() const;
    QStringList downloadedModels() const;
    static QStringList eraseModels();
    static QStringList diffusionModels();
    QStringList plugins() const { return m_plugins; }
    QVariantMap pluginModels() const { return m_pluginModels; }
    QStringList samplers() const { return m_samplers; }
    QStringList enabledPlugins() const { return m_enabledPlugins; }
    void setEnabledPlugins(const QStringList &plugins);
    QString device() const { return m_device; }
    void setDevice(const QString &device);
    // How install() would provision Python: "uv", "python 3.11", or "" when neither is found.
    QString installMethod() const;

    Q_INVOKABLE void install();
    Q_INVOKABLE void start();
    Q_INVOKABLE void stopServer();
    Q_INVOKABLE void setModel(const QString &name);
    Q_INVOKABLE void switchPluginModel(const QString &plugin, const QString &model);
    // Erase / replace inside the selection with the current model. `options` are InpaintRequest
    // fields (prompt, negative_prompt, sd_steps, sd_strength, sd_guidance_scale, sd_seed,
    // powerpaint_task, enable_controlnet, ...) and pass straight through.
    Q_INVOKABLE void inpaint(const QVariantMap &options);
    // Pad the canvas by the given pixels, then fill the new border with the current model.
    Q_INVOKABLE void outpaint(int left, int top, int right, int bottom, const QVariantMap &options);
    // Paint by Example: inpaint the selection from an example image file.
    Q_INVOKABLE void paintByExample(const QUrl &exampleImage, const QVariantMap &options);
    // A gen-image plugin (RemoveBG, AnimeSeg, RealESRGAN, GFPGAN, RestoreFormer) to a new layer.
    // RealESRGAN scales the image first, by `scale`.
    Q_INVOKABLE void runPlugin(const QString &name, qreal scale = 2.0);
    // A gen-mask plugin (RemoveBG, AnimeSeg) to the selection.
    Q_INVOKABLE void pluginMask(const QString &name, const QString &mode);
    // InteractiveSeg: [[x, y, 1|0], ...] in canvas pixels (1 = include, 0 = exclude).
    Q_INVOKABLE void segmentClicks(const QVariantList &clicks, const QString &mode);
    // `iopaint run` over a folder: every image with a same-named mask in maskFolder.
    Q_INVOKABLE void runBatch(const QUrl &imageFolder, const QUrl &maskFolder, const QUrl &outputFolder);
    Q_INVOKABLE void cancel();

    // Exposed for the guard tests.
    static QStringList serverArguments(int port, const QString &model, const QString &device,
                                       const QString &modelDir, const QStringList &plugins);
    static QStringList pinnedPackages();
    QString envDir() const;
    QString modelDir() const;
    QString iopaintExecutable() const;

signals:
    void stateChanged();
    void messageChanged();
    void logChanged();
    void configChanged();
    void settingsChanged();

private:
    void setState(const QString &state);
    void setMessage(const QString &message);
    void appendLog(const QString &line);
    void runSteps(QList<QStringList> steps, std::function<void(bool)> done);
    void releaseStep();
    void pollReady();
    void readConfig(const QJsonObject &config);
    QUrl api(const QString &path) const;
    // POST JSON; `done` gets the reply body on HTTP 200, or an empty array with message set.
    void post(const QString &path, const QJsonObject &body, const QString &busyText,
              std::function<void(const QByteArray &)> done);
    bool ensureReady(const QString &action);
    static QString pngBase64(const QImage &image);
    // Transparency composited over white, as RGB888.
    static QImage opaque(const QImage &source);
    void landImage(const QByteArray &png, const QString &name, const QImage &clipMask);
    void landMask(const QByteArray &png, const QString &mode);
    void inpaintWith(const QImage &source, const QImage &mask, QJsonObject body,
                     const QString &layerName);

    QPointer<EditorBridge> m_bridge;
    QNetworkAccessManager m_network;
    std::unique_ptr<QProcess> m_server;
    std::unique_ptr<QProcess> m_step;
    QPointer<QNetworkReply> m_reply;
    QTimer m_readyTimer;
    int m_port = 0;
    int m_readyPolls = 0;
    QString m_state;
    QString m_message;
    QStringList m_log;
    QString m_model = QStringLiteral("lama");
    QJsonObject m_modelInfos; // name -> ModelInfo
    QStringList m_plugins;
    QVariantMap m_pluginModels;
    QStringList m_samplers;
    QStringList m_enabledPlugins;
    QString m_device = QStringLiteral("cpu");
    QString m_pendingModel;
};
