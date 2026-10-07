// SPDX-License-Identifier: GPL-3.0-or-later
#include "IopaintEngine.h"

#include "EditorBridge.h"

#include <QBuffer>
#include <QDir>
#include <QFileInfo>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonDocument>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QPainter>
#include <QProcessEnvironment>
#include <QSettings>
#include <QStandardPaths>
#include <QTcpServer>

#ifdef Q_OS_LINUX
#include <csignal>
#include <sys/prctl.h>
#endif

namespace {
constexpr int kMaxLogLines = 300;
// IOPaint downloads a model on first start; a big diffusion model is gigabytes. Poll for 40 min.
constexpr int kReadyPollMs = 1500;
constexpr int kMaxReadyPolls = 1600;

const QStringList kPluginNames = {
    QStringLiteral("InteractiveSeg"), QStringLiteral("RemoveBG"), QStringLiteral("AnimeSeg"),
    QStringLiteral("RealESRGAN"), QStringLiteral("GFPGAN"), QStringLiteral("RestoreFormer"),
};

bool isWindows()
{
#ifdef Q_OS_WIN
    return true;
#else
    return false;
#endif
}

bool isMac()
{
#ifdef Q_OS_MACOS
    return true;
#else
    return false;
#endif
}

QString findUv()
{
    QString uv = QStandardPaths::findExecutable(QStringLiteral("uv"));
    if (!uv.isEmpty())
        return uv;
    const QString home = QDir::homePath();
    return QStandardPaths::findExecutable(QStringLiteral("uv"),
                                          {home + QStringLiteral("/.local/bin"),
                                           home + QStringLiteral("/.cargo/bin")});
}

// IOPaint 1.6.0 pins Pillow 9.5.0, which has no wheels past Python 3.11.
QStringList findPython311()
{
    for (const QString &name : {QStringLiteral("python3.11"), QStringLiteral("python3.10")}) {
        const QString found = QStandardPaths::findExecutable(name);
        if (!found.isEmpty())
            return {found};
    }
    if (isWindows()) {
        const QString py = QStandardPaths::findExecutable(QStringLiteral("py"));
        if (!py.isEmpty())
            return {py, QStringLiteral("-3.11")};
    }
    return {};
}
} // namespace

IopaintEngine::IopaintEngine(EditorBridge *bridge, QObject *parent)
    : QObject(parent)
    , m_bridge(bridge)
{
    QSettings settings;
    m_enabledPlugins = settings
                           .value(QStringLiteral("iopaint/plugins"),
                                  QStringList{QStringLiteral("InteractiveSeg"), QStringLiteral("RemoveBG"),
                                              QStringLiteral("RealESRGAN")})
                           .toStringList();
    m_device = settings.value(QStringLiteral("iopaint/device"), QStringLiteral("cpu")).toString();
    m_model = settings.value(QStringLiteral("iopaint/model"), QStringLiteral("lama")).toString();
    m_readyTimer.setInterval(kReadyPollMs);
    connect(&m_readyTimer, &QTimer::timeout, this, &IopaintEngine::pollReady);
    m_state = QFileInfo::exists(iopaintExecutable()) ? QStringLiteral("stopped") : QStringLiteral("missing");
    m_message = m_state == QStringLiteral("missing")
        ? QStringLiteral("The AI engine (IOPaint) is not installed")
        : QStringLiteral("The AI engine is installed and stopped");
}

IopaintEngine::~IopaintEngine()
{
    cancel();
    stopServer();
}

// ---- settings ----

QVariantMap IopaintEngine::modelInfo() const
{
    return m_modelInfos.value(m_model).toObject().toVariantMap();
}

QStringList IopaintEngine::downloadedModels() const { return m_modelInfos.keys(); }

QStringList IopaintEngine::eraseModels()
{
    // iopaint/const.py AVAILABLE_MODELS, plus anime-lama.
    return {QStringLiteral("lama"), QStringLiteral("anime-lama"), QStringLiteral("migan"),
            QStringLiteral("mat"), QStringLiteral("fcf"), QStringLiteral("zits"),
            QStringLiteral("ldm"), QStringLiteral("manga"), QStringLiteral("cv2")};
}

QStringList IopaintEngine::diffusionModels()
{
    // The `name` of each diffusion model class under iopaint/model/.
    return {QStringLiteral("Sanster/PowerPaint-V1-stable-diffusion-inpainting"),
            QStringLiteral("runwayml/stable-diffusion-inpainting"),
            QStringLiteral("stabilityai/stable-diffusion-2-inpainting"),
            QStringLiteral("Sanster/Realistic_Vision_V1.4-inpainting"),
            QStringLiteral("Sanster/anything-4.0-inpainting"),
            QStringLiteral("diffusers/stable-diffusion-xl-1.0-inpainting-0.1"),
            QStringLiteral("kandinsky-community/kandinsky-2-2-decoder-inpaint"),
            QStringLiteral("Sanster/AnyText"),
            QStringLiteral("Fantasy-Studio/Paint-by-Example"),
            QStringLiteral("timbrooks/instruct-pix2pix")};
}

void IopaintEngine::setEnabledPlugins(const QStringList &plugins)
{
    QStringList clean;
    for (const QString &name : plugins) {
        if (kPluginNames.contains(name) && !clean.contains(name))
            clean.append(name);
    }
    if (clean == m_enabledPlugins)
        return;
    m_enabledPlugins = clean;
    QSettings().setValue(QStringLiteral("iopaint/plugins"), clean);
    emit settingsChanged();
    // Plugins load at server start only.
    if (m_server && m_server->state() != QProcess::NotRunning) {
        stopServer();
        start();
    }
}

void IopaintEngine::setDevice(const QString &device)
{
    if (device != QStringLiteral("cpu") && device != QStringLiteral("cuda") && device != QStringLiteral("mps"))
        return;
    if (device == m_device)
        return;
    m_device = device;
    QSettings().setValue(QStringLiteral("iopaint/device"), device);
    emit settingsChanged();
}

QString IopaintEngine::installMethod() const
{
    if (!findUv().isEmpty())
        return QStringLiteral("uv");
    if (!findPython311().isEmpty())
        return QStringLiteral("python 3.11");
    return {};
}

QString IopaintEngine::envDir() const
{
    return QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation) + QStringLiteral("/iopaint/env");
}

QString IopaintEngine::modelDir() const
{
    return QStandardPaths::writableLocation(QStandardPaths::AppLocalDataLocation) + QStringLiteral("/iopaint/models");
}

QString IopaintEngine::iopaintExecutable() const
{
    return isWindows() ? envDir() + QStringLiteral("/Scripts/iopaint.exe") : envDir() + QStringLiteral("/bin/iopaint");
}

QStringList IopaintEngine::pinnedPackages()
{
    // IOPaint 1.6.0 is the last release (the project is archived). rembg is its RemoveBG plugin's
    // optional dependency; it is pinned to a release that accepts IOPaint's own pins (Pillow 9.5,
    // NumPy 1.x), since a newer rembg silently upgrades both.
    return {QStringLiteral("iopaint==1.6.0"), QStringLiteral("rembg[cpu]==2.0.57"),
            QStringLiteral("pillow==9.5.0"), QStringLiteral("numpy<2")};
}

QStringList IopaintEngine::serverArguments(int port, const QString &model, const QString &device,
                                           const QString &modelDir, const QStringList &plugins)
{
    // Loopback only: the server has no authentication. No --input, so its file manager is off.
    QStringList args{QStringLiteral("start"), QStringLiteral("--host"), QStringLiteral("127.0.0.1"),
                     QStringLiteral("--port"), QString::number(port), QStringLiteral("--model"), model,
                     QStringLiteral("--device"), device, QStringLiteral("--model-dir"), modelDir};
    if (plugins.contains(QStringLiteral("InteractiveSeg")))
        args << QStringLiteral("--enable-interactive-seg") << QStringLiteral("--interactive-seg-model")
             << QStringLiteral("mobile_sam") << QStringLiteral("--interactive-seg-device") << device;
    if (plugins.contains(QStringLiteral("RemoveBG")))
        args << QStringLiteral("--enable-remove-bg") << QStringLiteral("--remove-bg-device") << device;
    if (plugins.contains(QStringLiteral("AnimeSeg")))
        args << QStringLiteral("--enable-anime-seg");
    if (plugins.contains(QStringLiteral("RealESRGAN")))
        args << QStringLiteral("--enable-realesrgan") << QStringLiteral("--realesrgan-device") << device;
    if (plugins.contains(QStringLiteral("GFPGAN")))
        args << QStringLiteral("--enable-gfpgan") << QStringLiteral("--gfpgan-device") << device;
    if (plugins.contains(QStringLiteral("RestoreFormer")))
        args << QStringLiteral("--enable-restoreformer") << QStringLiteral("--restoreformer-device") << device;
    return args;
}

// ---- state ----

void IopaintEngine::setState(const QString &state)
{
    if (state == m_state)
        return;
    m_state = state;
    emit stateChanged();
}

void IopaintEngine::setMessage(const QString &message)
{
    if (message == m_message)
        return;
    m_message = message;
    emit messageChanged();
    if (m_bridge && !message.isEmpty())
        m_bridge->aiStatus(message);
}

void IopaintEngine::appendLog(const QString &line)
{
    const QString trimmed = line.trimmed();
    if (trimmed.isEmpty())
        return;
    // Download progress bars redraw with \r; keep only the last frame.
    const QString last = trimmed.split(QLatin1Char('\r')).last().trimmed();
    m_log.append(last.left(300));
    while (m_log.size() > kMaxLogLines)
        m_log.removeFirst();
    emit logChanged();
}

// ---- install ----

void IopaintEngine::runSteps(QList<QStringList> steps, std::function<void(bool)> done)
{
    if (steps.isEmpty()) {
        done(true);
        return;
    }
    const QStringList step = steps.takeFirst();
    // A step chains to the next from inside its own finished signal; never delete it there.
    releaseStep();
    m_step = std::make_unique<QProcess>();
    QProcess *proc = m_step.get();
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.insert(QStringLiteral("PYTHONUTF8"), QStringLiteral("1"));
    env.insert(QStringLiteral("PIP_DISABLE_PIP_VERSION_CHECK"), QStringLiteral("1"));
    proc->setProcessEnvironment(env);
    proc->setProcessChannelMode(QProcess::MergedChannels);
    proc->setStandardInputFile(QProcess::nullDevice());
    connect(proc, &QProcess::readyReadStandardOutput, this, [this, proc]() {
        const QList<QByteArray> lines = proc->readAllStandardOutput().split('\n');
        for (const QByteArray &line : lines)
            appendLog(QString::fromUtf8(line));
    });
    connect(proc, &QProcess::finished, this,
            [this, steps, done](int code, QProcess::ExitStatus status) {
                if (status != QProcess::NormalExit || code != 0) {
                    appendLog(QStringLiteral("step failed with exit code %1").arg(code));
                    done(false);
                    return;
                }
                runSteps(steps, done);
            });
    connect(proc, &QProcess::errorOccurred, this, [this, proc, done](QProcess::ProcessError error) {
        if (error == QProcess::FailedToStart) {
            appendLog(QStringLiteral("could not start %1").arg(proc->program()));
            done(false);
        }
    });
    appendLog(QStringLiteral("$ ") + step.join(QLatin1Char(' ')));
    m_step->start(step.first(), step.mid(1));
}

void IopaintEngine::releaseStep()
{
    if (m_step)
        m_step.release()->deleteLater();
}

void IopaintEngine::install()
{
    if (m_state == QStringLiteral("installing"))
        return;
    stopServer();
    const QString env = envDir();
    QDir().mkpath(QFileInfo(env).absolutePath());
    QDir().mkpath(modelDir());
    const QString python = isWindows() ? env + QStringLiteral("/Scripts/python.exe") : env + QStringLiteral("/bin/python");
    // PyTorch's default Linux and Windows wheels carry CUDA (gigabytes); use the CPU build unless
    // the user picked CUDA. macOS wheels are CPU/Metal already.
    QStringList torchIndex;
    if (!isMac())
        torchIndex << QStringLiteral("--index-url")
                   << (m_device == QStringLiteral("cuda") ? QStringLiteral("https://download.pytorch.org/whl/cu121")
                                                          : QStringLiteral("https://download.pytorch.org/whl/cpu"));
    const QStringList torch{QStringLiteral("torch==2.5.1"), QStringLiteral("torchvision==0.20.1")};

    QList<QStringList> steps;
    const QString uv = findUv();
    if (!uv.isEmpty()) {
        steps << QStringList{uv, QStringLiteral("venv"), QStringLiteral("--python"), QStringLiteral("3.11"), env};
        steps << (QStringList{uv, QStringLiteral("pip"), QStringLiteral("install"), QStringLiteral("--python"), python}
                  + torchIndex + torch);
        steps << (QStringList{uv, QStringLiteral("pip"), QStringLiteral("install"), QStringLiteral("--python"), python}
                  + pinnedPackages());
    } else {
        const QStringList base = findPython311();
        if (base.isEmpty()) {
            setState(QStringLiteral("missing"));
            setMessage(QStringLiteral("Install uv (docs.astral.sh/uv) or Python 3.11 first, then try again"));
            return;
        }
        steps << (base + QStringList{QStringLiteral("-m"), QStringLiteral("venv"), env});
        steps << QStringList{python, QStringLiteral("-m"), QStringLiteral("pip"), QStringLiteral("install"),
                             QStringLiteral("--upgrade"), QStringLiteral("pip")};
        steps << (QStringList{python, QStringLiteral("-m"), QStringLiteral("pip"), QStringLiteral("install")}
                  + torchIndex + torch);
        steps << (QStringList{python, QStringLiteral("-m"), QStringLiteral("pip"), QStringLiteral("install")}
                  + pinnedPackages());
    }
    m_log.clear();
    emit logChanged();
    setState(QStringLiteral("installing"));
    setMessage(QStringLiteral("Installing the AI engine (about 1.5 GB)…"));
    runSteps(steps, [this](bool ok) {
        releaseStep();
        if (ok && QFileInfo::exists(iopaintExecutable())) {
            setState(QStringLiteral("stopped"));
            setMessage(QStringLiteral("AI engine installed"));
            start();
        } else {
            setState(QStringLiteral("missing"));
            setMessage(QStringLiteral("Installing the AI engine failed; see the log"));
        }
    });
}

// ---- server ----

void IopaintEngine::start()
{
    if (m_server && m_server->state() != QProcess::NotRunning)
        return;
    if (!QFileInfo::exists(iopaintExecutable())) {
        setState(QStringLiteral("missing"));
        setMessage(QStringLiteral("The AI engine (IOPaint) is not installed"));
        return;
    }
    QTcpServer probe;
    if (!probe.listen(QHostAddress::LocalHost, 0)) {
        setState(QStringLiteral("error"));
        setMessage(QStringLiteral("No free local port for the AI engine"));
        return;
    }
    m_port = probe.serverPort();
    probe.close();
    QDir().mkpath(modelDir());

    m_server = std::make_unique<QProcess>();
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    env.insert(QStringLiteral("PYTHONUTF8"), QStringLiteral("1"));
    env.insert(QStringLiteral("PYTHONUNBUFFERED"), QStringLiteral("1"));
    m_server->setProcessEnvironment(env);
    m_server->setProcessChannelMode(QProcess::MergedChannels);
    m_server->setStandardInputFile(QProcess::nullDevice());
#ifdef Q_OS_LINUX
    // If the app dies without running destructors (SIGTERM, a crash), the kernel stops the
    // server too; otherwise it was left listening with nobody to stop it.
    m_server->setChildProcessModifier([] { ::prctl(PR_SET_PDEATHSIG, SIGTERM); });
#endif
    connect(m_server.get(), &QProcess::readyReadStandardOutput, this, [this]() {
        const QList<QByteArray> lines = m_server->readAllStandardOutput().split('\n');
        for (const QByteArray &line : lines)
            appendLog(QString::fromUtf8(line));
    });
    connect(m_server.get(), &QProcess::finished, this, [this](int code, QProcess::ExitStatus) {
        m_readyTimer.stop();
        if (m_state == QStringLiteral("installing"))
            return;
        setState(QStringLiteral("stopped"));
        if (code != 0)
            setMessage(QStringLiteral("The AI engine stopped (exit code %1); see the log").arg(code));
    });
    setState(QStringLiteral("starting"));
    setMessage(QStringLiteral("Starting the AI engine with %1 (the first start downloads it)…").arg(m_model));
    m_server->start(iopaintExecutable(), serverArguments(m_port, m_model, m_device, modelDir(), m_enabledPlugins));
    m_readyPolls = 0;
    m_readyTimer.start();
}

void IopaintEngine::stopServer()
{
    m_readyTimer.stop();
    if (!m_server)
        return;
    if (m_server->state() != QProcess::NotRunning) {
        m_server->terminate();
        if (!m_server->waitForFinished(3000))
            m_server->kill();
        m_server->waitForFinished(1000);
    }
    m_server.reset();
    if (m_state != QStringLiteral("installing") && m_state != QStringLiteral("missing"))
        setState(QStringLiteral("stopped"));
}

QUrl IopaintEngine::api(const QString &path) const
{
    return QUrl(QStringLiteral("http://127.0.0.1:%1%2").arg(m_port).arg(path));
}

void IopaintEngine::pollReady()
{
    if (!m_server || m_server->state() == QProcess::NotRunning) {
        m_readyTimer.stop();
        return;
    }
    if (++m_readyPolls > kMaxReadyPolls) {
        m_readyTimer.stop();
        setState(QStringLiteral("error"));
        setMessage(QStringLiteral("The AI engine did not become ready"));
        return;
    }
    QNetworkReply *reply = m_network.get(QNetworkRequest(api(QStringLiteral("/api/v1/server-config"))));
    connect(reply, &QNetworkReply::finished, this, [this, reply]() {
        reply->deleteLater();
        if (reply->error() != QNetworkReply::NoError || m_state != QStringLiteral("starting"))
            return;
        const QJsonDocument doc = QJsonDocument::fromJson(reply->readAll());
        if (!doc.isObject())
            return;
        m_readyTimer.stop();
        readConfig(doc.object());
        setState(QStringLiteral("ready"));
        setMessage(QStringLiteral("AI engine ready · %1").arg(m_model));
    });
}

void IopaintEngine::readConfig(const QJsonObject &config)
{
    m_modelInfos = {};
    for (const QJsonValue &value : config.value(QStringLiteral("modelInfos")).toArray()) {
        const QJsonObject info = value.toObject();
        m_modelInfos.insert(info.value(QStringLiteral("name")).toString(), info);
    }
    m_plugins.clear();
    for (const QJsonValue &value : config.value(QStringLiteral("plugins")).toArray())
        m_plugins.append(value.toObject().value(QStringLiteral("name")).toString());
    const auto choice = [&config](const char *current, const char *all) {
        QStringList choices;
        for (const QJsonValue &value : config.value(QLatin1String(all)).toArray())
            choices.append(value.toString());
        return QVariantMap{{QStringLiteral("current"), config.value(QLatin1String(current)).toString()},
                           {QStringLiteral("choices"), choices}};
    };
    m_pluginModels = {
        {QStringLiteral("RemoveBG"), choice("removeBGModel", "removeBGModels")},
        {QStringLiteral("RealESRGAN"), choice("realesrganModel", "realesrganModels")},
        {QStringLiteral("InteractiveSeg"), choice("interactiveSegModel", "interactiveSegModels")},
    };
    m_samplers.clear();
    for (const QJsonValue &value : config.value(QStringLiteral("samplers")).toArray())
        m_samplers.append(value.toString());
    emit configChanged();
}

bool IopaintEngine::ensureReady(const QString &action)
{
    if (!m_bridge)
        return false;
    if (m_state == QStringLiteral("busy")) {
        setMessage(QStringLiteral("%1: the AI engine is still working").arg(action));
        return false;
    }
    if (m_state != QStringLiteral("ready")) {
        setMessage(QStringLiteral("%1: start the AI engine first").arg(action));
        return false;
    }
    return true;
}

void IopaintEngine::post(const QString &path, const QJsonObject &body, const QString &busyText,
                         std::function<void(const QByteArray &)> done)
{
    QNetworkRequest request(api(path));
    request.setHeader(QNetworkRequest::ContentTypeHeader, QStringLiteral("application/json"));
    // Diffusion on a CPU can take minutes; no transfer timeout.
    request.setTransferTimeout(0);
    setState(QStringLiteral("busy"));
    setMessage(busyText);
    QNetworkReply *reply = m_network.post(request, QJsonDocument(body).toJson(QJsonDocument::Compact));
    m_reply = reply;
    connect(reply, &QNetworkReply::finished, this, [this, reply, done]() {
        reply->deleteLater();
        if (m_reply == reply)
            m_reply = nullptr;
        const int status = reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt();
        const QByteArray bytes = reply->readAll();
        if (m_state == QStringLiteral("busy"))
            setState(m_server && m_server->state() != QProcess::NotRunning ? QStringLiteral("ready")
                                                                           : QStringLiteral("stopped"));
        if (reply->error() == QNetworkReply::OperationCanceledError) {
            setMessage(QStringLiteral("AI request cancelled"));
            return;
        }
        if (status != 200) {
            const QJsonObject error = QJsonDocument::fromJson(bytes).object();
            QString detail = error.value(QStringLiteral("detail")).toString();
            if (detail.isEmpty())
                detail = error.value(QStringLiteral("errors")).toString();
            if (detail.isEmpty())
                detail = reply->errorString();
            setMessage(QStringLiteral("AI engine error: %1").arg(detail.left(300)));
            return;
        }
        done(bytes);
    });
}

QImage IopaintEngine::opaque(const QImage &source)
{
    QImage flat(source.size(), QImage::Format_RGB888);
    flat.fill(Qt::white);
    QPainter painter(&flat);
    painter.drawImage(0, 0, source);
    painter.end();
    return flat;
}

QString IopaintEngine::pngBase64(const QImage &image)
{
    QByteArray bytes;
    QBuffer buffer(&bytes);
    buffer.open(QIODevice::WriteOnly);
    image.save(&buffer, "PNG");
    return QStringLiteral("data:image/png;base64,") + QString::fromLatin1(bytes.toBase64());
}

void IopaintEngine::landImage(const QByteArray &png, const QString &name, const QImage &clipMask)
{
    if (!m_bridge)
        return;
    QImage result = QImage::fromData(png, "PNG").convertToFormat(QImage::Format_RGBA8888);
    if (result.isNull()) {
        setMessage(QStringLiteral("%1: the AI engine returned no image").arg(name));
        return;
    }
    const int width = m_bridge->documentWidth();
    const int height = m_bridge->documentHeight();
    if (result.size() != QSize(width, height)) {
        // An upscaler: scale the document by the same factor first, then lay the result over it.
        const bool uniform = width > 0 && height > 0
            && qAbs(qreal(result.width()) / width - qreal(result.height()) / height) < 0.02;
        if (!uniform || result.width() < width) {
            setMessage(QStringLiteral("%1: the result is %2 × %3, the canvas %4 × %5")
                           .arg(name).arg(result.width()).arg(result.height()).arg(width).arg(height));
            return;
        }
        m_bridge->resizeCanvas(result.width(), result.height(), QStringLiteral("bilinear"));
        if (m_bridge->documentWidth() != result.width() || m_bridge->documentHeight() != result.height()) {
            setMessage(QStringLiteral("%1: could not scale the canvas to %2 × %3")
                           .arg(name).arg(result.width()).arg(result.height()));
            return;
        }
    }
    if (!clipMask.isNull() && clipMask.size() == result.size()) {
        // Only the masked area changes; the layer is transparent elsewhere, so it can be
        // hidden, masked further or deleted without touching the original.
        for (int y = 0; y < result.height(); ++y) {
            uchar *row = result.scanLine(y);
            const uchar *mask = clipMask.constScanLine(y);
            for (int x = 0; x < result.width(); ++x)
                row[x * 4 + 3] = uchar((int(row[x * 4 + 3]) * mask[x] + 127) / 255);
        }
    }
    if (m_bridge->aiAddLayer(result, name))
        setMessage(QStringLiteral("%1 added as a new layer").arg(name));
}

void IopaintEngine::landMask(const QByteArray &png, const QString &mode)
{
    if (!m_bridge)
        return;
    const QImage rgba = QImage::fromData(png, "PNG").convertToFormat(QImage::Format_RGBA8888);
    if (rgba.isNull() || rgba.size() != QSize(m_bridge->documentWidth(), m_bridge->documentHeight())) {
        setMessage(QStringLiteral("AI selection: the mask does not match the canvas"));
        return;
    }
    // IOPaint's frontend mask paints the selected pixels with a translucent colour (alpha > 0).
    QImage gray(rgba.size(), QImage::Format_Grayscale8);
    for (int y = 0; y < rgba.height(); ++y) {
        const uchar *src = rgba.constScanLine(y);
        uchar *dst = gray.scanLine(y);
        for (int x = 0; x < rgba.width(); ++x)
            dst[x] = src[x * 4 + 3] > 0 ? 255 : 0;
    }
    if (m_bridge->aiSelectMask(gray, mode))
        setMessage(QStringLiteral("Selection from the AI mask"));
}

// ---- operations ----

void IopaintEngine::setModel(const QString &name)
{
    if (name.isEmpty() || name == m_model)
        return;
    if (!eraseModels().contains(name) && !diffusionModels().contains(name) && !m_modelInfos.contains(name))
        return;
    QSettings().setValue(QStringLiteral("iopaint/model"), name);
    if (m_state == QStringLiteral("ready") && m_modelInfos.contains(name)) {
        // Downloaded already: switch in place.
        post(QStringLiteral("/api/v1/model"), {{QStringLiteral("name"), name}},
             QStringLiteral("Loading %1…").arg(name), [this, name](const QByteArray &) {
                 m_model = name;
                 emit configChanged();
                 setMessage(QStringLiteral("AI engine ready · %1").arg(name));
             });
        return;
    }
    // Not downloaded yet: `iopaint start` downloads the model it is started with.
    m_model = name;
    emit configChanged();
    if (m_server && m_server->state() != QProcess::NotRunning) {
        stopServer();
        start();
    }
}

void IopaintEngine::switchPluginModel(const QString &plugin, const QString &model)
{
    if (!ensureReady(QStringLiteral("Plugin model")))
        return;
    post(QStringLiteral("/api/v1/switch_plugin_model"),
         {{QStringLiteral("plugin_name"), plugin}, {QStringLiteral("model_name"), model}},
         QStringLiteral("Loading %1 for %2…").arg(model, plugin), [this, plugin, model](const QByteArray &) {
             QVariantMap entry = m_pluginModels.value(plugin).toMap();
             entry.insert(QStringLiteral("current"), model);
             m_pluginModels.insert(plugin, entry);
             emit configChanged();
             setMessage(QStringLiteral("%1 uses %2").arg(plugin, model));
         });
}

void IopaintEngine::inpaintWith(const QImage &source, const QImage &mask, QJsonObject body,
                                const QString &layerName)
{
    // IOPaint hands the source's alpha back unchanged, so an erased stroke on a transparent
    // canvas came back as a dark silhouette. Send opaque pixels (transparency over white); the
    // new layer's alpha is the selection instead.
    body.insert(QStringLiteral("image"), pngBase64(opaque(source)));
    body.insert(QStringLiteral("mask"), pngBase64(mask));
    const bool keepUnmasked = body.value(QStringLiteral("sd_keep_unmasked_area")).toBool(true);
    const QImage clip = keepUnmasked ? mask : QImage();
    post(QStringLiteral("/api/v1/inpaint"), body, QStringLiteral("%1 with %2…").arg(layerName, m_model),
         [this, layerName, clip](const QByteArray &png) { landImage(png, layerName, clip); });
}

void IopaintEngine::inpaint(const QVariantMap &options)
{
    if (!ensureReady(QStringLiteral("AI fill")))
        return;
    const QImage mask = m_bridge->aiSelectionMask();
    if (mask.isNull()) {
        setMessage(QStringLiteral("AI fill: select the area to erase or replace first"));
        return;
    }
    const QImage source = m_bridge->aiSourceImage();
    if (source.isNull()) {
        setMessage(QStringLiteral("AI fill: the canvas is busy; try again"));
        return;
    }
    const bool prompted = !options.value(QStringLiteral("prompt")).toString().trimmed().isEmpty();
    inpaintWith(source, mask, QJsonObject::fromVariantMap(options),
                prompted ? QStringLiteral("AI replace") : QStringLiteral("AI erase"));
}

void IopaintEngine::outpaint(int left, int top, int right, int bottom, const QVariantMap &options)
{
    if (!ensureReady(QStringLiteral("AI expand")))
        return;
    if (left < 0 || top < 0 || right < 0 || bottom < 0 || left + top + right + bottom == 0
        || left > 4096 || top > 4096 || right > 4096 || bottom > 4096) {
        setMessage(QStringLiteral("AI expand: give 1 to 4096 pixels on at least one side"));
        return;
    }
    const QImage source = m_bridge->aiSourceImage();
    if (source.isNull()) {
        setMessage(QStringLiteral("AI expand: the canvas is busy; try again"));
        return;
    }
    // The model fills the new border (mask 255) from the picture (mask 0). Opaque RGB, so the
    // border is not handed back transparent with the source's alpha.
    const QSize padded(source.width() + left + right, source.height() + top + bottom);
    QImage image(padded, QImage::Format_RGB888);
    image.fill(Qt::black);
    QImage mask(padded, QImage::Format_Grayscale8);
    mask.fill(255);
    {
        const QImage flat = opaque(source);
        for (int y = 0; y < flat.height(); ++y) {
            memcpy(image.scanLine(y + top) + left * 3, flat.constScanLine(y), size_t(flat.width()) * 3);
            memset(mask.scanLine(y + top) + left, 0, size_t(flat.width()));
        }
    }
    QJsonObject body = QJsonObject::fromVariantMap(options);
    body.insert(QStringLiteral("image"), pngBase64(image));
    body.insert(QStringLiteral("mask"), pngBase64(mask));
    post(QStringLiteral("/api/v1/inpaint"), body, QStringLiteral("AI expand with %1…").arg(m_model),
         [this, left, top, right, bottom, padded, mask](const QByteArray &png) {
             if (!m_bridge)
                 return;
             const QImage result = QImage::fromData(png, "PNG");
             if (result.size() != padded) {
                 setMessage(QStringLiteral("AI expand: the result has the wrong size"));
                 return;
             }
             m_bridge->padCanvas(left, top, right, bottom);
             if (m_bridge->documentWidth() != padded.width() || m_bridge->documentHeight() != padded.height()) {
                 setMessage(QStringLiteral("AI expand: could not grow the canvas"));
                 return;
             }
             landImage(png, QStringLiteral("AI expand"), mask);
         });
}

void IopaintEngine::paintByExample(const QUrl &exampleImage, const QVariantMap &options)
{
    if (!ensureReady(QStringLiteral("Paint by example")))
        return;
    const QImage example(exampleImage.toLocalFile());
    if (example.isNull()) {
        setMessage(QStringLiteral("Paint by example: could not read that image"));
        return;
    }
    if (!m_model.contains(QStringLiteral("Paint-by-Example"))) {
        setMessage(QStringLiteral("Paint by example: choose the Paint-by-Example model first"));
        return;
    }
    const QImage mask = m_bridge->aiSelectionMask();
    const QImage source = m_bridge->aiSourceImage();
    if (mask.isNull() || source.isNull()) {
        setMessage(QStringLiteral("Paint by example: select the area to fill first"));
        return;
    }
    QJsonObject body = QJsonObject::fromVariantMap(options);
    body.insert(QStringLiteral("paint_by_example_example_image"), pngBase64(example.convertToFormat(QImage::Format_RGB888)));
    inpaintWith(source, mask, body, QStringLiteral("Paint by example"));
}

void IopaintEngine::runPlugin(const QString &name, qreal scale)
{
    if (!ensureReady(name))
        return;
    if (!m_plugins.contains(name)) {
        setMessage(QStringLiteral("%1 is off; turn it on under AI plugins").arg(name));
        return;
    }
    const QImage source = m_bridge->aiSourceImage();
    if (source.isNull()) {
        setMessage(QStringLiteral("%1: the canvas is busy; try again").arg(name));
        return;
    }
    const qreal factor = qBound(1.0, scale, 4.0);
    if (name == QStringLiteral("RealESRGAN")
        && qint64(source.width() * factor) * qint64(source.height() * factor) > 64ll * 1024 * 1024) {
        setMessage(QStringLiteral("Upscale: the result would pass 64 Mi-pixels"));
        return;
    }
    static const QMap<QString, QString> names{
        {QStringLiteral("RemoveBG"), QStringLiteral("Remove background")},
        {QStringLiteral("AnimeSeg"), QStringLiteral("Anime cut-out")},
        {QStringLiteral("RealESRGAN"), QStringLiteral("AI upscale")},
        {QStringLiteral("GFPGAN"), QStringLiteral("Face restore (GFPGAN)")},
        {QStringLiteral("RestoreFormer"), QStringLiteral("Face restore (RestoreFormer)")},
    };
    const QString layerName = names.value(name, name);
    post(QStringLiteral("/api/v1/run_plugin_gen_image"),
         {{QStringLiteral("name"), name}, {QStringLiteral("image"), pngBase64(source)},
          {QStringLiteral("scale"), factor}},
         QStringLiteral("%1…").arg(layerName),
         [this, layerName](const QByteArray &png) { landImage(png, layerName, QImage()); });
}

void IopaintEngine::pluginMask(const QString &name, const QString &mode)
{
    if (!ensureReady(name))
        return;
    if (!m_plugins.contains(name)) {
        setMessage(QStringLiteral("%1 is off; turn it on under AI plugins").arg(name));
        return;
    }
    const QImage source = m_bridge->aiSourceImage();
    if (source.isNull())
        return;
    post(QStringLiteral("/api/v1/run_plugin_gen_mask"),
         {{QStringLiteral("name"), name}, {QStringLiteral("image"), pngBase64(source)}},
         QStringLiteral("Finding the subject…"), [this, mode](const QByteArray &png) { landMask(png, mode); });
}

void IopaintEngine::segmentClicks(const QVariantList &clicks, const QString &mode)
{
    if (!ensureReady(QStringLiteral("Click to select")))
        return;
    if (!m_plugins.contains(QStringLiteral("InteractiveSeg"))) {
        setMessage(QStringLiteral("Click to select is off; turn on InteractiveSeg under AI plugins"));
        return;
    }
    QJsonArray points;
    for (const QVariant &value : clicks) {
        const QVariantList click = value.toList();
        if (click.size() != 3)
            continue;
        points.append(QJsonArray{click[0].toInt(), click[1].toInt(), click[2].toInt() ? 1 : 0});
    }
    if (points.isEmpty()) {
        setMessage(QStringLiteral("Click to select: click on the subject first"));
        return;
    }
    const QImage source = m_bridge->aiSourceImage();
    if (source.isNull())
        return;
    post(QStringLiteral("/api/v1/run_plugin_gen_mask"),
         {{QStringLiteral("name"), QStringLiteral("InteractiveSeg")},
          {QStringLiteral("image"), pngBase64(source)},
          {QStringLiteral("clicks"), points}},
         QStringLiteral("Selecting what you clicked…"), [this, mode](const QByteArray &png) { landMask(png, mode); });
}

void IopaintEngine::runBatch(const QUrl &imageFolder, const QUrl &maskFolder, const QUrl &outputFolder)
{
    if (m_step) {
        setMessage(QStringLiteral("Batch: another AI engine job is running"));
        return;
    }
    if (!QFileInfo::exists(iopaintExecutable())) {
        setMessage(QStringLiteral("Batch: install the AI engine first"));
        return;
    }
    const QString images = imageFolder.toLocalFile();
    const QString masks = maskFolder.toLocalFile();
    const QString output = outputFolder.toLocalFile();
    if (!QFileInfo(images).isDir() || !QFileInfo(masks).exists() || output.isEmpty()) {
        setMessage(QStringLiteral("Batch: choose an image folder, a mask folder or file, and an output folder"));
        return;
    }
    QDir().mkpath(output);
    const QStringList step{iopaintExecutable(), QStringLiteral("run"), QStringLiteral("--model"), m_model,
                           QStringLiteral("--device"), m_device, QStringLiteral("--image"), images,
                           QStringLiteral("--mask"), masks, QStringLiteral("--output"), output,
                           QStringLiteral("--model-dir"), modelDir()};
    setMessage(QStringLiteral("Batch: erasing with %1…").arg(m_model));
    runSteps({step}, [this, output](bool ok) {
        releaseStep();
        setMessage(ok ? QStringLiteral("Batch finished · results in %1").arg(output)
                      : QStringLiteral("Batch failed; see the log"));
    });
}

void IopaintEngine::cancel()
{
    if (m_reply)
        m_reply->abort();
    if (m_step && m_step->state() != QProcess::NotRunning) {
        m_step->kill();
        m_step->waitForFinished(2000);
    }
}
