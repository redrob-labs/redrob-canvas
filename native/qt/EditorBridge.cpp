// SPDX-License-Identifier: GPL-3.0-or-later
#include "EditorBridge.h"

#include "FrameIdAllocator.h"

#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonParseError>
#include <QMouseEvent>
#include <QPointF>
#include <QPointingDevice>
#include <QRegularExpression>
#include <QRandomGenerator>
#include <QSaveFile>
#include <QStringList>
#include <QSet>
#include <QTabletEvent>
#include <QUuid>
#include <QtConcurrentRun>
#include <QtGlobal>

#include <cmath>
#include <limits>

namespace {
constexpr qsizetype kMaxBrushPoints = 4'096;
constexpr int kSnapshotAttempts = 3;
constexpr qint64 kMaxCanvasDimension = 32'768;
constexpr qint64 kMaxCanvasPixels = 64LL * 1024LL * 1024LL;
constexpr qsizetype kMaxNativeTextCharacters = 256 * 1024;
constexpr qreal kMaxSemanticCoordinate = 1'048'576.0;
constexpr qint64 kMaxFormatInputBytes = 64LL * 1024LL * 1024LL;
// Onion-skin ghost tints, packed 0xRRGGBBAA: earlier frames lean red, later frames lean green, which
// is the convention animators already read in this kind of tool. Alpha is full here -- the ghost's
// strength is the separate opacity the animator controls.
constexpr uint32_t kOnionTintBefore = 0xFF5050FFu;
constexpr uint32_t kOnionTintAfter = 0x50FF78FFu;

QString canonicalFormatForSuffix(const QString &suffix)
{
    const QString lower = suffix.toLower();
    if (lower == QStringLiteral("rrg") || lower == QStringLiteral("png")
        || lower == QStringLiteral("webp") || lower == QStringLiteral("ora")
        || lower == QStringLiteral("svg") || lower == QStringLiteral("psd")
        || lower == QStringLiteral("kra") || lower == QStringLiteral("xcf"))
        return lower;
    if (lower == QStringLiteral("jpg") || lower == QStringLiteral("jpeg"))
        return QStringLiteral("jpeg");
    if (lower == QStringLiteral("tif") || lower == QStringLiteral("tiff"))
        return QStringLiteral("tiff");
    return {};
}

bool suffixMatchesFormat(const QString &suffix, const QString &format)
{
    return canonicalFormatForSuffix(suffix) == format;
}

std::optional<QByteArray> readBoundedFormatFile(QFile &file, QString *error)
{
    QByteArray bytes = file.read(kMaxFormatInputBytes + 1);
    if (file.error() != QFileDevice::NoError) {
        if (error)
            *error = file.errorString();
        return std::nullopt;
    }
    if (bytes.size() > kMaxFormatInputBytes) {
        if (error)
            *error = QStringLiteral("file exceeds the 64 MiB input limit");
        return std::nullopt;
    }
    return bytes;
}

QString formatOutcomeStatus(const QString &verb, const QString &name, const QJsonObject &result)
{
    const QString format = result.value(QStringLiteral("effective_format")).toString().toUpper();
    const QJsonValue frameValue = result.value(QStringLiteral("frame"));
    QString status = QStringLiteral("%1 %2 · %3").arg(verb, name, format);
    if (frameValue.isDouble())
        status += QStringLiteral(" · frame %1").arg(frameValue.toInt());
    QStringList warnings;
    for (const QJsonValue &warning : result.value(QStringLiteral("warnings")).toArray()) {
        const QString code = warning.toObject().value(QStringLiteral("code")).toString();
        if (!code.isEmpty())
            warnings.append(code);
    }
    if (!warnings.isEmpty())
        status += QStringLiteral(" · warnings: %1").arg(warnings.join(QStringLiteral(", ")));
    return status;
}

struct ChangeInvalidation {
    qulonglong generation = 0;
    bool valid = false;
    bool selectionChanged = true;
    bool canvasChanged = true;
};

ChangeInvalidation changeInvalidation(const QByteArray &json)
{
    QJsonParseError error;
    const QJsonDocument document = QJsonDocument::fromJson(json, &error);
    if (error.error != QJsonParseError::NoError || !document.isObject())
        return {};
    const QJsonObject object = document.object();
    const QJsonValue generation = object.value(QStringLiteral("generation"));
    const QJsonValue selectionChanged = object.value(QStringLiteral("selection_changed"));
    const QJsonValue canvasChanged = object.value(QStringLiteral("canvas_changed"));
    if (!generation.isDouble() || generation.toDouble() < 0.0 || !selectionChanged.isBool()
        || !canvasChanged.isBool())
        return {};
    return {generation.toVariant().toULongLong(), true, selectionChanged.toBool(),
            canvasChanged.toBool()};
}

QByteArray copyOwnedBuffer(RedrobBuffer buffer)
{
    const QByteArray copy = buffer.data && buffer.len
        ? QByteArray(reinterpret_cast<const char *>(buffer.data), static_cast<qsizetype>(buffer.len))
        : QByteArray{};
    redrob_buffer_free(buffer);
    return copy;
}

QString currentFfiError()
{
    const size_t length = redrob_last_error_copy(nullptr, 0);
    QByteArray bytes(static_cast<qsizetype>(length + 1), '\0');
    redrob_last_error_copy(bytes.data(), static_cast<size_t>(bytes.size()));
    bytes.truncate(static_cast<qsizetype>(length));
    return QString::fromUtf8(bytes);
}

AgentResult requestAgentProposals(RedrobEditor *editor, QByteArray apiKey, QByteArray prompt,
                                   qulonglong documentEpoch)
{
    RedrobBuffer output{};
    AgentResult result;
    result.documentEpoch = documentEpoch;
    result.status = redrob_agent_propose(
        editor, reinterpret_cast<const uint8_t *>(apiKey.constData()),
        static_cast<size_t>(apiKey.size()), reinterpret_cast<const uint8_t *>(prompt.constData()),
        static_cast<size_t>(prompt.size()), &output);
    if (result.status == REDROB_OK)
        result.payload = copyOwnedBuffer(output);
    else {
        redrob_buffer_free(output);
        result.error = currentFfiError();
    }
    return result;
}

bool isFiniteValue(qreal value)
{
    return std::isfinite(static_cast<double>(value));
}
} // namespace

EditorBridge::EditorBridge(QObject *parent)
    : QObject(parent)
    , m_layers(this)
    , m_frames(this)
    , m_proposals(this)
    , m_agentWatcher(this)
    , m_filterWatcher(this)
{
    // S1: at most one live repaint per ~frame, however fast the pointer moves.
    m_liveStrokeTimer.setSingleShot(true);
    m_liveStrokeTimer.setInterval(16);
    connect(&m_liveStrokeTimer, &QTimer::timeout, this, &EditorBridge::flushLiveStroke);
    // A small default palette so the F.3 palette docker is not empty on first run.
    for (const char *hex : {"#000000", "#ffffff", "#e03131", "#f08c00", "#f5d90a",
                            "#2f9e44", "#1971c2", "#9c36b5", "#f1f3f5"}) {
        m_palette.append(QColor(QString::fromLatin1(hex)));
    }
    m_apiKey = qgetenv("REDROB_API_KEY");
    m_liveAgentConfigured = !m_apiKey.trimmed().isEmpty();
    setAgentStatus(m_liveAgentConfigured
                       ? QStringLiteral("Live Redrob · API key configured · model auto")
                       : QStringLiteral("Local deterministic proposal mode · explicitly no network"));
    connect(&m_agentWatcher, &QFutureWatcher<AgentResult>::finished, this, [this] {
        finishAgentRequest(m_agentWatcher.result());
    });
    connect(&m_filterWatcher, &QFutureWatcher<FilterRunResult>::finished, this, [this] {
        finishFilterRun(m_filterWatcher.result());
    });
    m_playbackTimer.setTimerType(Qt::PreciseTimer);
    connect(&m_playbackTimer, &QTimer::timeout, this, [this] { advancePlayback(); });
    RedrobBuffer capabilities{};
    if (redrob_ffi_capabilities_json(&capabilities) == REDROB_OK)
        m_formatCapabilities = QString::fromUtf8(takeBuffer(capabilities));
    else {
        redrob_buffer_free(capabilities);
        m_formatCapabilities = QStringLiteral("{}");
    }
    // UI-1: the filter browser's list, read once from the same capabilities document. Each entry
    // is {kind, defaults} where defaults is the engine's own parameter object for `{"kind": k}`,
    // or null when a parameter has no default.
    for (const QJsonValue &entry :
         QJsonDocument::fromJson(m_formatCapabilities.toUtf8()).object().value(QStringLiteral("filters")).toArray())
        m_filterCatalog.append(entry.toObject().toVariantMap());
    m_refreshRetryTimer.setInterval(250);
    m_refreshRetryTimer.setTimerType(Qt::CoarseTimer);
    connect(&m_refreshRetryTimer, &QTimer::timeout, this, [this] {
        const bool captureSelection = m_retryNeedsSelection;
        if (refresh(captureSelection)) {
            m_projectionStale = false;
            m_retryNeedsSelection = false;
            m_refreshRetryTimer.stop();
            setStatus(QStringLiteral("Display synchronized with the committed document"));
        }
    });
    RedrobEditor *editor = nullptr;
    if (redrob_editor_create(1280, 800, &editor) != REDROB_OK) {
        setStatus(QStringLiteral("Could not create editor: %1").arg(ffiError()));
        return;
    }
    m_editor.reset(editor, redrob_editor_destroy);
    setStatus(QStringLiteral("Ready"));
    if (!refresh())
        scheduleProjectionRefresh(true);
}

EditorBridge::~EditorBridge() = default;

int EditorBridge::documentWidth() const { return m_width; }
int EditorBridge::documentHeight() const { return m_height; }
qulonglong EditorBridge::generation() const { return m_generation; }
bool EditorBridge::canUndo() const { return m_canUndo; }
bool EditorBridge::canRedo() const { return m_canRedo; }
int EditorBridge::undoDepth() const { return m_undoDepth; }
int EditorBridge::redoDepth() const { return m_redoDepth; }
QVariantList EditorBridge::activeVectorAnchors() const { return m_activeVectorAnchors; }
QVariantList EditorBridge::activeVectorHandles() const { return m_activeVectorHandles; }
QAbstractItemModel *EditorBridge::frames() { return &m_frames; }
quint32 EditorBridge::currentFrame() const { return m_frames.currentFrameId(); }
int EditorBridge::currentFrameIndex() const { return m_frames.currentIndex(); }
int EditorBridge::frameCount() const { return m_frames.rowCount(); }
qreal EditorBridge::fps() const { return m_fps; }
quint32 EditorBridge::rangeStart() const { return m_rangeStart; }
quint32 EditorBridge::rangeEnd() const { return m_rangeEnd; }
bool EditorBridge::looping() const { return m_looping; }
bool EditorBridge::playing() const { return m_playing; }
QString EditorBridge::activeLayerId() const { return m_layers.activeLayerId(); }
QString EditorBridge::activeNodeKind() const { return m_layers.activeNodeKind(); }
bool EditorBridge::activeNodeCanEditRaster() const { return m_layers.activeNodeCanEditRaster(); }
bool EditorBridge::activeNodeCanEditText() const { return m_layers.activeNodeCanEditText(); }
bool EditorBridge::activeNodeCanEditVector() const { return m_layers.activeNodeCanEditVector(); }
bool EditorBridge::activeNodeCanRasterize() const { return m_layers.activeNodeCanRasterize(); }
bool EditorBridge::activeNodeHasMask() const { return m_layers.activeNodeHasMask(); }
QAbstractItemModel *EditorBridge::layers() { return &m_layers; }
QAbstractItemModel *EditorBridge::proposals() { return &m_proposals; }
QImage EditorBridge::renderImage() const { return m_renderImage; }
QImage EditorBridge::selectionMask() const { return m_selectionMask; }

QColor EditorBridge::sampleColor(qreal x, qreal y) const
{
    const int px = static_cast<int>(std::floor(x));
    const int py = static_cast<int>(std::floor(y));
    if (m_renderImage.isNull() || !m_renderImage.rect().contains(px, py))
        return {};
    QColor color = m_renderImage.pixelColor(px, py);
    if (color.alpha() == 0)
        return {};
    color.setAlpha(255);
    return color;
}
bool EditorBridge::selectionActive() const { return m_selectionActive; }
qreal EditorBridge::brushSize() const { return m_brushSize; }
QColor EditorBridge::brushColor() const { return m_brushColor; }
qreal EditorBridge::brushOpacity() const { return m_brushOpacity; }
QString EditorBridge::brushSmoothingKind() const { return m_brushSmoothingKind; }
int EditorBridge::brushSmoothingWindow() const { return m_brushSmoothingWindow; }
bool EditorBridge::mirrorXEnabled() const { return m_mirrorXEnabled; }
bool EditorBridge::mirrorYEnabled() const { return m_mirrorYEnabled; }
qreal EditorBridge::mirrorXAxis() const { return m_mirrorXAxis; }
qreal EditorBridge::mirrorYAxis() const { return m_mirrorYAxis; }
QString EditorBridge::statusMessage() const { return m_statusMessage; }
QString EditorBridge::currentFile() const { return m_currentFile; }
QString EditorBridge::formatCapabilities() const { return m_formatCapabilities; }
bool EditorBridge::liveAgentConfigured() const { return m_liveAgentConfigured; }
bool EditorBridge::agentBusy() const { return m_agentBusy; }
bool EditorBridge::filterBusy() const { return m_filterBusy; }

bool EditorBridge::mcpEnabled() const { return m_mcp.isListening(); }
QString EditorBridge::mcpStatus() const { return m_mcpStatus; }

void EditorBridge::setMcpEnabled(bool enabled)
{
    if (enabled == m_mcp.isListening())
        return;
    if (!enabled) {
        m_mcp.stop();
        m_mcpStatus = QStringLiteral("Off");
        emit mcpChanged();
        return;
    }
    RedrobBuffer tools{};
    if (redrob_mcp_tools_json(&tools) != REDROB_OK) {
        redrob_buffer_free(tools);
        m_mcpStatus = QStringLiteral("Could not list the tools: %1").arg(ffiError());
        emit mcpChanged();
        return;
    }
    m_mcp.setToolsJson(takeBuffer(tools));
    m_mcp.setCallHandler([this](const QString &name, const QJsonObject &arguments) {
        return handleMcpToolCall(name, arguments);
    });
    QString error;
    if (!m_mcp.start(&error)) {
        m_mcpStatus = QStringLiteral("Could not start: %1").arg(error);
    } else {
        m_mcpStatus = QStringLiteral("Listening on 127.0.0.1:%1 · edits arrive as proposals")
                          .arg(m_mcp.port());
    }
    emit mcpChanged();
}

QString EditorBridge::mcpConfigSnippet() const
{
    if (!m_mcp.isListening())
        return {};
    // redrob-code's config: a remote MCP server with a bearer header. The token changes every time
    // the endpoint is turned on, so this entry is only good for this session.
    const QJsonObject server{
        {QStringLiteral("type"), QStringLiteral("remote")},
        {QStringLiteral("url"), QStringLiteral("http://127.0.0.1:%1/mcp").arg(m_mcp.port())},
        {QStringLiteral("enabled"), true},
        {QStringLiteral("headers"),
         QJsonObject{{QStringLiteral("Authorization"),
                      QStringLiteral("Bearer %1").arg(m_mcp.token())}}}};
    const QJsonObject config{
        {QStringLiteral("mcp"), QJsonObject{{QStringLiteral("redrob-canvas"), server}}}};
    return QString::fromUtf8(QJsonDocument(config).toJson(QJsonDocument::Indented));
}

QJsonObject EditorBridge::handleMcpToolCall(const QString &name, const QJsonObject &arguments)
{
    // Runs on the GUI thread (the server lives there). A running filter holds the engine lock, so
    // asking now would freeze the window; say so instead.
    if (m_filterBusy)
        return {{QStringLiteral("error"), QStringLiteral("Redrob Canvas is applying a filter; try again shortly")}};
    if (!m_editor)
        return {{QStringLiteral("error"), QStringLiteral("no document is open")}};
    const QString callId = QStringLiteral("mcp-") + QUuid::createUuid().toString(QUuid::WithoutBraces);
    const QByteArray call = QJsonDocument(QJsonObject{{QStringLiteral("id"), callId},
                                                      {QStringLiteral("name"), name},
                                                      {QStringLiteral("arguments"), arguments}})
                                .toJson(QJsonDocument::Compact);
    RedrobBuffer output{};
    if (redrob_editor_mcp_propose(m_editor.get(), reinterpret_cast<const uint8_t *>(call.constData()),
                                  static_cast<size_t>(call.size()), &output)
        != REDROB_OK) {
        redrob_buffer_free(output);
        return {{QStringLiteral("error"), ffiError()}};
    }
    const QJsonObject result = QJsonDocument::fromJson(takeBuffer(output)).object();
    const auto text = [](const QString &body) {
        return QJsonObject{{QStringLiteral("content"),
                            QJsonArray{QJsonObject{{QStringLiteral("type"), QStringLiteral("text")},
                                                   {QStringLiteral("text"), body}}}}};
    };
    const QJsonValue inspect = result.value(QStringLiteral("inspect"));
    if (inspect.isObject())
        return text(QString::fromUtf8(QJsonDocument(inspect.toObject()).toJson(QJsonDocument::Compact)));
    const QJsonObject proposal = result.value(QStringLiteral("proposal")).toObject();
    const QJsonObject action = proposal.value(QStringLiteral("action")).toObject();
    const QString actionType = action.value(QStringLiteral("type")).toString();
    const QString commandJson = actionType == QStringLiteral("command")
        ? QString::fromUtf8(canonicalJson(action.value(QStringLiteral("command")).toObject()))
        : QString();
    // The same queue, staleness check and approval as a hosted-agent proposal: nothing applies
    // until the user presses Apply in the Agent tab.
    const QString queued = m_proposals.enqueue(
        proposal.value(QStringLiteral("title")).toString(),
        QStringLiteral("redrob-code · ") + proposal.value(QStringLiteral("summary")).toString(),
        actionType, commandJson,
        proposal.value(QStringLiteral("base_generation")).toVariant().toULongLong(),
        m_documentEpoch, callId);
    if (queued.isEmpty())
        return {{QStringLiteral("error"), QStringLiteral("the proposal could not be queued")}};
    setStatus(QStringLiteral("redrob-code proposed: %1 — review it in the Agent tab")
                  .arg(proposal.value(QStringLiteral("title")).toString()));
    return text(QStringLiteral("Queued \"%1\" for the user's approval in Redrob Canvas. It is not "
                               "applied until they approve it; the document is unchanged.")
                    .arg(proposal.value(QStringLiteral("title")).toString()));
}
QString EditorBridge::agentStatus() const { return m_agentStatus; }
QString EditorBridge::assistantText() const { return m_assistantText; }

QString EditorBridge::modelStatus() const
{
    return m_liveAgentConfigured ? QStringLiteral("Live Redrob · model auto")
                                 : QStringLiteral("Local deterministic demo · no network");
}

void EditorBridge::setBrushSize(qreal size)
{
    if (!isFiniteValue(size))
        return;
    const qreal bounded = qBound(1.0, size, 1000.0);
    if (qFuzzyCompare(m_brushSize, bounded))
        return;
    m_brushSize = bounded;
    emit brushSettingsChanged();
}

void EditorBridge::setBrushColor(const QColor &color)
{
    if (!color.isValid() || color == m_brushColor)
        return;
    m_brushColor = color;
    emit brushColorChanged();
}

QVariantList EditorBridge::brushPresets() const { return m_brushPresets; }

void EditorBridge::saveBrushPreset(const QString &name)
{
    QVariantMap preset;
    preset.insert(QStringLiteral("name"), name.trimmed().isEmpty() ? QStringLiteral("Preset") : name.trimmed());
    preset.insert(QStringLiteral("size"), brushSize());
    preset.insert(QStringLiteral("hardness"), brushHardness());
    preset.insert(QStringLiteral("opacity"), brushOpacity());
    preset.insert(QStringLiteral("pencil"), brushPencil());
    preset.insert(QStringLiteral("aspect"), brushAspect());
    m_brushPresets.append(preset);
    emit brushPresetsChanged();
}

void EditorBridge::applyBrushPreset(int index)
{
    if (index < 0 || index >= m_brushPresets.size())
        return;
    const QVariantMap preset = m_brushPresets.at(index).toMap();
    setBrushSize(preset.value(QStringLiteral("size"), brushSize()).toDouble());
    setBrushHardness(preset.value(QStringLiteral("hardness"), brushHardness()).toDouble());
    setBrushOpacity(preset.value(QStringLiteral("opacity"), brushOpacity()).toDouble());
    setBrushPencil(preset.value(QStringLiteral("pencil"), brushPencil()).toBool());
    setBrushAspect(preset.value(QStringLiteral("aspect"), brushAspect()).toDouble());
}

void EditorBridge::removeBrushPreset(int index)
{
    if (index < 0 || index >= m_brushPresets.size())
        return;
    m_brushPresets.removeAt(index);
    emit brushPresetsChanged();
}

QVariantList EditorBridge::palette() const { return m_palette; }

void EditorBridge::addPaletteColor(const QColor &color)
{
    if (!color.isValid())
        return;
    m_palette.append(color);
    emit paletteChanged();
}

void EditorBridge::removePaletteColor(int index)
{
    if (index < 0 || index >= m_palette.size())
        return;
    m_palette.removeAt(index);
    emit paletteChanged();
}

void EditorBridge::setBrushOpacity(qreal opacity)
{
    if (!isFiniteValue(opacity))
        return;
    const qreal bounded = qBound(0.0, opacity, 1.0);
    if (qFuzzyCompare(m_brushOpacity, bounded))
        return;
    m_brushOpacity = bounded;
    emit brushSettingsChanged();
}

qreal EditorBridge::brushHardness() const { return m_brushHardness; }

void EditorBridge::setBrushHardness(qreal hardness)
{
    if (!isFiniteValue(hardness))
        return;
    // DabShape::is_valid: 0.0..=1.0. qFuzzyCompare is not used because 0 is a legal value.
    const qreal bounded = qBound(0.0, hardness, 1.0);
    if (qAbs(m_brushHardness - bounded) < 1e-6)
        return;
    m_brushHardness = bounded;
    emit brushSettingsChanged();
}

bool EditorBridge::brushErase() const { return m_brushErase; }

void EditorBridge::setBrushErase(bool erase)
{
    if (m_brushErase == erase)
        return;
    m_brushErase = erase;
    emit brushSettingsChanged();
}

bool EditorBridge::brushPencil() const { return m_brushPencil; }

void EditorBridge::setBrushPencil(bool pencil)
{
    if (m_brushPencil == pencil)
        return;
    m_brushPencil = pencil;
    emit brushSettingsChanged();
}

bool EditorBridge::brushAirbrush() const { return m_brushAirbrush; }

void EditorBridge::setBrushAirbrush(bool airbrush)
{
    if (m_brushAirbrush == airbrush)
        return;
    m_brushAirbrush = airbrush;
    emit brushSettingsChanged();
}

bool EditorBridge::brushSmudge() const { return m_brushSmudge; }

void EditorBridge::setBrushSmudge(bool smudge)
{
    if (m_brushSmudge == smudge)
        return;
    m_brushSmudge = smudge;
    emit brushSettingsChanged();
}

bool EditorBridge::brushClone() const { return m_brushClone; }

void EditorBridge::setBrushClone(bool clone)
{
    if (m_brushClone == clone)
        return;
    m_brushClone = clone;
    emit brushSettingsChanged();
}

void EditorBridge::setCloneSource(qreal x, qreal y)
{
    if (!isFiniteValue(x) || !isFiniteValue(y))
        return;
    m_cloneSourceX = x;
    m_cloneSourceY = y;
    m_cloneSourceSet = true;
    setStatus(QStringLiteral("Clone source set"));
}

bool EditorBridge::brushHeal() const { return m_brushHeal; }

void EditorBridge::setBrushHeal(bool heal)
{
    if (m_brushHeal == heal)
        return;
    m_brushHeal = heal;
    emit brushSettingsChanged();
}

QString EditorBridge::brushConvolveMode() const { return m_brushConvolveMode; }

void EditorBridge::setBrushConvolveMode(const QString &mode)
{
    const QString value = (mode == QStringLiteral("blur") || mode == QStringLiteral("sharpen"))
            ? mode
            : QStringLiteral("off");
    if (m_brushConvolveMode == value)
        return;
    m_brushConvolveMode = value;
    emit brushSettingsChanged();
}

QString EditorBridge::brushDodgeBurnMode() const { return m_brushDodgeBurnMode; }

void EditorBridge::setBrushDodgeBurnMode(const QString &mode)
{
    const QString value = (mode == QStringLiteral("dodge") || mode == QStringLiteral("burn"))
            ? mode
            : QStringLiteral("off");
    if (m_brushDodgeBurnMode == value)
        return;
    m_brushDodgeBurnMode = value;
    emit brushSettingsChanged();
}

QString EditorBridge::brushDodgeRange() const { return m_brushDodgeRange; }

void EditorBridge::setBrushDodgeRange(const QString &range)
{
    const QString value = (range == QStringLiteral("shadows") || range == QStringLiteral("highlights"))
            ? range
            : QStringLiteral("midtones");
    if (m_brushDodgeRange == value)
        return;
    m_brushDodgeRange = value;
    emit brushSettingsChanged();
}

bool EditorBridge::brushInk() const { return m_brushInk; }

void EditorBridge::setBrushInk(bool ink)
{
    if (m_brushInk == ink)
        return;
    m_brushInk = ink;
    emit brushSettingsChanged();
}

bool EditorBridge::brushMyPaint() const { return m_brushMyPaint; }

void EditorBridge::setBrushMyPaint(bool mypaint)
{
    if (m_brushMyPaint == mypaint)
        return;
    m_brushMyPaint = mypaint;
    emit brushSettingsChanged();
}

QString EditorBridge::brushSizeDynamic() const { return m_brushSizeDynamic; }

void EditorBridge::setBrushSizeDynamic(const QString &sensor)
{
    const QString value = (sensor == QStringLiteral("pressure") || sensor == QStringLiteral("speed")
                           || sensor == QStringLiteral("random") || sensor == QStringLiteral("tilt"))
            ? sensor
            : QStringLiteral("off");
    if (m_brushSizeDynamic == value)
        return;
    m_brushSizeDynamic = value;
    emit brushSettingsChanged();
}

// The three channel setters share one validator: a sensor name the core does not know would be
// serialised into the settings and rejected as a whole, so an unrecognised value becomes "off".
static QString normalisedSensor(const QString &sensor)
{
    if (sensor == QStringLiteral("pressure") || sensor == QStringLiteral("speed")
        || sensor == QStringLiteral("random") || sensor == QStringLiteral("tilt"))
        return sensor;
    return QStringLiteral("off");
}

QString EditorBridge::brushOpacityDynamic() const { return m_brushOpacityDynamic; }

void EditorBridge::setBrushOpacityDynamic(const QString &sensor)
{
    const QString value = normalisedSensor(sensor);
    if (m_brushOpacityDynamic == value)
        return;
    m_brushOpacityDynamic = value;
    emit brushSettingsChanged();
}

QString EditorBridge::brushFlowDynamic() const { return m_brushFlowDynamic; }

void EditorBridge::setBrushFlowDynamic(const QString &sensor)
{
    const QString value = normalisedSensor(sensor);
    if (m_brushFlowDynamic == value)
        return;
    m_brushFlowDynamic = value;
    emit brushSettingsChanged();
}

bool EditorBridge::brushPipe() const { return m_brushPipe; }

void EditorBridge::setBrushPipe(bool pipe)
{
    if (m_brushPipe == pipe)
        return;
    m_brushPipe = pipe;
    emit brushSettingsChanged();
}

qreal EditorBridge::brushAspect() const { return m_brushAspect; }

void EditorBridge::setBrushAspect(qreal aspect)
{
    if (!isFiniteValue(aspect))
        return;
    // DabShape::is_valid allows 0.01..=100; the tool offers flattening only, 0.05..1.
    const qreal bounded = qBound(0.05, aspect, 1.0);
    if (qAbs(m_brushAspect - bounded) < 1e-6)
        return;
    m_brushAspect = bounded;
    emit brushSettingsChanged();
}

QStringList EditorBridge::brushTipNames() const
{
    QStringList names;
    for (const QJsonValue &tip : m_brushTips) {
        const QString name = tip.toObject().value(QStringLiteral("name")).toString();
        names.append(name.isEmpty() ? QStringLiteral("Tip %1").arg(names.size() + 1) : name);
    }
    return names;
}

int EditorBridge::brushTipIndex() const { return m_brushTipIndex; }

void EditorBridge::setBrushTipIndex(int index)
{
    const int bounded = index >= 0 && index < m_brushTips.size() ? index : -1;
    if (m_brushTipIndex == bounded)
        return;
    m_brushTipIndex = bounded;
    emit brushSettingsChanged();
}

int EditorBridge::loadBrushTips(const QUrl &url)
{
    if (!url.isLocalFile()) {
        setStatus(QStringLiteral("Brush import failed: choose a local .gbr or .abr file"));
        return 0;
    }
    const QFileInfo info(url.toLocalFile());
    QFile file(info.filePath());
    if (!file.open(QIODevice::ReadOnly)) {
        setStatus(QStringLiteral("Could not read %1: %2").arg(info.fileName(), file.errorString()));
        return 0;
    }
    QString readError;
    const auto bytes = readBoundedFormatFile(file, &readError);
    if (!bytes) {
        setStatus(QStringLiteral("Brush import failed: %1").arg(readError));
        return 0;
    }
    RedrobBuffer output{};
    const int status = redrob_brush_tips_decode(
        reinterpret_cast<const uint8_t *>(bytes->constData()), static_cast<size_t>(bytes->size()),
        &output);
    if (status != REDROB_OK) {
        redrob_buffer_free(output);
        setStatus(QStringLiteral("Brush import failed: %1").arg(ffiError()));
        return 0;
    }
    const QJsonDocument document = QJsonDocument::fromJson(takeBuffer(output));
    if (!document.isArray() || document.array().isEmpty()) {
        setStatus(QStringLiteral("Brush import failed: invalid tip list from core"));
        return 0;
    }
    const int first = m_brushTips.size();
    for (const QJsonValue &tip : document.array())
        m_brushTips.append(tip);
    m_brushTipIndex = first;
    emit brushSettingsChanged();
    const int added = m_brushTips.size() - first;
    setStatus(added == 1 ? QStringLiteral("Loaded brush tip from %1").arg(info.fileName())
                         : QStringLiteral("Loaded %1 brush tips from %2").arg(added).arg(info.fileName()));
    return added;
}

void EditorBridge::setBrushSmoothingKind(const QString &kind)
{
    if (kind != QStringLiteral("none") && kind != QStringLiteral("moving_average"))
        return;
    if (m_brushSmoothingKind == kind)
        return;
    m_brushSmoothingKind = kind;
    emit brushSettingsChanged();
}

void EditorBridge::setBrushSmoothingWindow(int window)
{
    const int bounded = qBound(2, window, 64);
    if (m_brushSmoothingWindow == bounded)
        return;
    m_brushSmoothingWindow = bounded;
    emit brushSettingsChanged();
}

void EditorBridge::setMirrorXEnabled(bool enabled)
{
    if (m_mirrorXEnabled == enabled)
        return;
    m_mirrorXEnabled = enabled;
    emit brushSettingsChanged();
}

void EditorBridge::setMirrorYEnabled(bool enabled)
{
    if (m_mirrorYEnabled == enabled)
        return;
    m_mirrorYEnabled = enabled;
    emit brushSettingsChanged();
}

void EditorBridge::setMirrorXAxis(qreal axis)
{
    if (!isFiniteValue(axis) || qFuzzyCompare(m_mirrorXAxis, axis))
        return;
    m_mirrorXAxis = axis;
    emit brushSettingsChanged();
}

void EditorBridge::setMirrorYAxis(qreal axis)
{
    if (!isFiniteValue(axis) || qFuzzyCompare(m_mirrorYAxis, axis))
        return;
    m_mirrorYAxis = axis;
    emit brushSettingsChanged();
}

int EditorBridge::brushSymmetryOrder() const { return m_brushSymmetryOrder; }
void EditorBridge::setBrushSymmetryOrder(int order)
{
    const int clamped = qBound(0, order, 32);
    if (m_brushSymmetryOrder == clamped)
        return;
    m_brushSymmetryOrder = clamped;
    emit brushSettingsChanged();
}
qreal EditorBridge::brushSymmetryCenterX() const { return m_brushSymmetryCenterX; }
void EditorBridge::setBrushSymmetryCenterX(qreal x)
{
    if (!isFiniteValue(x) || qFuzzyCompare(m_brushSymmetryCenterX, x))
        return;
    m_brushSymmetryCenterX = x;
    emit brushSettingsChanged();
}
qreal EditorBridge::brushSymmetryCenterY() const { return m_brushSymmetryCenterY; }
void EditorBridge::setBrushSymmetryCenterY(qreal y)
{
    if (!isFiniteValue(y) || qFuzzyCompare(m_brushSymmetryCenterY, y))
        return;
    m_brushSymmetryCenterY = y;
    emit brushSettingsChanged();
}
// Onion skin (H.2). Each setter repaints: the canvas picture itself changes, not a brush setting, so
// these emit renderImageChanged as well as the property signal.
bool EditorBridge::onionSkinEnabled() const { return m_onionSkinEnabled; }
void EditorBridge::setOnionSkinEnabled(bool enabled)
{
    if (m_onionSkinEnabled == enabled)
        return;
    m_onionSkinEnabled = enabled;
    emit onionSkinChanged();
    refresh(false);
}
int EditorBridge::onionSkinBefore() const { return m_onionSkinBefore; }
void EditorBridge::setOnionSkinBefore(int count)
{
    const int clamped = std::clamp(count, 0, 8);
    if (m_onionSkinBefore == clamped)
        return;
    m_onionSkinBefore = clamped;
    emit onionSkinChanged();
    if (m_onionSkinEnabled)
        refresh(false);
}
int EditorBridge::onionSkinAfter() const { return m_onionSkinAfter; }
void EditorBridge::setOnionSkinAfter(int count)
{
    const int clamped = std::clamp(count, 0, 8);
    if (m_onionSkinAfter == clamped)
        return;
    m_onionSkinAfter = clamped;
    emit onionSkinChanged();
    if (m_onionSkinEnabled)
        refresh(false);
}
qreal EditorBridge::onionSkinOpacity() const { return m_onionSkinOpacity; }
void EditorBridge::setOnionSkinOpacity(qreal opacity)
{
    if (!isFiniteValue(opacity))
        return;
    const qreal clamped = std::clamp(opacity, 0.0, 1.0);
    if (qFuzzyCompare(m_onionSkinOpacity, clamped))
        return;
    m_onionSkinOpacity = clamped;
    emit onionSkinChanged();
    if (m_onionSkinEnabled)
        refresh(false);
}
QString EditorBridge::brushAssistantKind() const { return m_brushAssistantKind; }
void EditorBridge::setBrushAssistantKind(const QString &kind)
{
    static const QStringList kinds{QStringLiteral("none"), QStringLiteral("vanishing"),
                                   QStringLiteral("parallel"), QStringLiteral("ellipse")};
    if (!kinds.contains(kind) || m_brushAssistantKind == kind)
        return;
    m_brushAssistantKind = kind;
    emit brushSettingsChanged();
}
void EditorBridge::setBrushAssistantParams(qreal p0, qreal p1, qreal p2, qreal p3)
{
    if (!isFiniteValue(p0) || !isFiniteValue(p1) || !isFiniteValue(p2) || !isFiniteValue(p3))
        return;
    m_brushAssistantP0 = p0;
    m_brushAssistantP1 = p1;
    m_brushAssistantP2 = p2;
    m_brushAssistantP3 = p3;
    emit brushSettingsChanged();
}
bool EditorBridge::brushDynaEnabled() const { return m_brushDynaEnabled; }
void EditorBridge::setBrushDynaEnabled(bool enabled)
{
    if (m_brushDynaEnabled == enabled)
        return;
    m_brushDynaEnabled = enabled;
    emit brushSettingsChanged();
}
qreal EditorBridge::brushDynaMass() const { return m_brushDynaMass; }
void EditorBridge::setBrushDynaMass(qreal mass)
{
    const qreal clamped = qBound(0.0, mass, 1.0);
    if (qFuzzyCompare(m_brushDynaMass, clamped))
        return;
    m_brushDynaMass = clamped;
    emit brushSettingsChanged();
}
qreal EditorBridge::brushDynaDrag() const { return m_brushDynaDrag; }
void EditorBridge::setBrushDynaDrag(qreal drag)
{
    const qreal clamped = qBound(0.0, drag, 1.0);
    if (qFuzzyCompare(m_brushDynaDrag, clamped))
        return;
    m_brushDynaDrag = clamped;
    emit brushSettingsChanged();
}

QByteArray EditorBridge::takeBuffer(RedrobBuffer buffer)
{
    return copyOwnedBuffer(buffer);
}

QString EditorBridge::ffiError() const
{
    return currentFfiError();
}

void EditorBridge::setStatus(QString status)
{
    if (m_statusMessage == status)
        return;
    m_statusMessage = std::move(status);
    emit statusMessageChanged();
}

void EditorBridge::setAgentBusy(bool busy)
{
    if (m_agentBusy == busy)
        return;
    m_agentBusy = busy;
    emit agentBusyChanged();
}

void EditorBridge::setAgentStatus(QString status)
{
    if (m_agentStatus == status)
        return;
    m_agentStatus = std::move(status);
    emit agentStatusChanged();
}

void EditorBridge::setAssistantText(QString text)
{
    if (m_assistantText == text)
        return;
    m_assistantText = std::move(text);
    emit assistantTextChanged();
}

QJsonObject EditorBridge::colorObject(const QColor &color)
{
    return {{QStringLiteral("r"), color.red()}, {QStringLiteral("g"), color.green()},
            {QStringLiteral("b"), color.blue()}, {QStringLiteral("a"), color.alpha()}};
}

bool EditorBridge::rectObject(qreal x, qreal y, qreal width, qreal height,
                              QJsonObject *outRect)
{
    if (!outRect || !isFiniteValue(x) || !isFiniteValue(y) || !isFiniteValue(width)
        || !isFiniteValue(height))
        return false;

    const double x1 = static_cast<double>(x);
    const double y1 = static_cast<double>(y);
    const double x2 = x1 + static_cast<double>(width);
    const double y2 = y1 + static_cast<double>(height);
    if (!std::isfinite(x2) || !std::isfinite(y2))
        return false;

    const double normalizedX = std::min(x1, x2);
    const double normalizedY = std::min(y1, y2);
    const double normalizedWidth = std::abs(x2 - x1);
    const double normalizedHeight = std::abs(y2 - y1);
    const double integralX = std::floor(normalizedX);
    const double integralY = std::floor(normalizedY);
    const double integralWidth = std::ceil(normalizedWidth);
    const double integralHeight = std::ceil(normalizedHeight);
    constexpr double minimumCoordinate = static_cast<double>(std::numeric_limits<int>::min());
    constexpr double maximumCoordinate = static_cast<double>(std::numeric_limits<int>::max());
    if (!std::isfinite(integralX) || !std::isfinite(integralY)
        || !std::isfinite(integralWidth) || !std::isfinite(integralHeight)
        || integralX < minimumCoordinate || integralX > maximumCoordinate
        || integralY < minimumCoordinate || integralY > maximumCoordinate
        || integralWidth < 1.0 || integralWidth > static_cast<double>(kMaxCanvasDimension)
        || integralHeight < 1.0 || integralHeight > static_cast<double>(kMaxCanvasDimension))
        return false;

    *outRect = {{QStringLiteral("x"), static_cast<int>(integralX)},
                {QStringLiteral("y"), static_cast<int>(integralY)},
                {QStringLiteral("width"), static_cast<int>(integralWidth)},
                {QStringLiteral("height"), static_cast<int>(integralHeight)}};
    return true;
}

QByteArray EditorBridge::canonicalJson(const QJsonObject &object)
{
    return QJsonDocument(object).toJson(QJsonDocument::Compact);
}

bool EditorBridge::validSelectionMode(const QString &mode)
{
    return mode == QStringLiteral("replace") || mode == QStringLiteral("add")
        || mode == QStringLiteral("subtract") || mode == QStringLiteral("intersect");
}

bool EditorBridge::validSampling(const QString &sampling)
{
    return sampling == QStringLiteral("nearest") || sampling == QStringLiteral("bilinear");
}

QJsonObject EditorBridge::brushSettingsObject() const
{
    QJsonObject smoothing{{QStringLiteral("kind"), m_brushSmoothingKind}};
    if (m_brushSmoothingKind == QStringLiteral("moving_average"))
        smoothing.insert(QStringLiteral("window"), m_brushSmoothingWindow);
    QJsonObject settings{{QStringLiteral("smoothing"), smoothing},
            {QStringLiteral("mirror_x"), m_mirrorXEnabled ? QJsonValue(m_mirrorXAxis)
                                                          : QJsonValue(QJsonValue::Null)},
            {QStringLiteral("mirror_y"), m_mirrorYEnabled ? QJsonValue(m_mirrorYAxis)
                                                          : QJsonValue(QJsonValue::Null)}};
    // Sent only when it differs from DabShape::default(), so a default stroke's command stays
    // byte-identical to what it was before the shape was exposed. Every DabShape field is required.
    if (m_brushHardness < 1.0 || m_brushAspect < 1.0 || m_brushPencil) {
        settings.insert(QStringLiteral("shape"),
                        QJsonObject{// Pencil (GIMP) forces a hard edge, so hardness is pinned to 1.
                                    {QStringLiteral("hardness"), m_brushPencil ? 1.0 : m_brushHardness},
                                    {QStringLiteral("softness"), 1.0},
                                    {QStringLiteral("ratio"), m_brushAspect},
                                    {QStringLiteral("antialias_edges"), true},
                                    {QStringLiteral("pencil"), m_brushPencil}});
    }
    // Same rule: absent unless on, so a painting stroke's command is unchanged.
    if (m_brushErase)
        settings.insert(QStringLiteral("erase"), true);
    // Airbrush: a low per-dab flow so paint builds up gradually while the pointer is held. Absent
    // unless airbrush mode is on, so a normal stroke's command is unchanged.
    if (m_brushAirbrush)
        settings.insert(QStringLiteral("flow"), m_brushFlow);
    // Smudge: drag the colour already on the layer instead of stamping the brush colour. Absent
    // unless smudge mode is on.
    if (m_brushSmudge)
        settings.insert(QStringLiteral("smudge"), m_brushSmudgeRate);
    // Clone: copy the layer from a source offset captured at stroke start. Absent unless clone mode
    // is on with a source set.
    if (m_brushClone && m_cloneSourceSet) {
        settings.insert(QStringLiteral("clone_offset"),
                        QJsonArray{m_cloneOffsetX, m_cloneOffsetY});
        // Heal: match the cloned patch to the destination's local colour. Only meaningful with a
        // clone source, so it rides inside the clone block.
        if (m_brushHeal)
            settings.insert(QStringLiteral("heal"), true);
    }
    // Convolve: blur or sharpen the pixels under the dab instead of painting. Absent when off.
    if (m_brushConvolveMode == QStringLiteral("blur"))
        settings.insert(QStringLiteral("convolve"), -0.5);
    else if (m_brushConvolveMode == QStringLiteral("sharpen"))
        settings.insert(QStringLiteral("convolve"), 0.5);
    // Dodge/Burn: lighten or darken the pixels under the dab in a tonal range. Absent when off.
    if (m_brushDodgeBurnMode == QStringLiteral("dodge")
        || m_brushDodgeBurnMode == QStringLiteral("burn")) {
        const double exposure = (m_brushDodgeBurnMode == QStringLiteral("dodge")) ? 0.3 : -0.3;
        settings.insert(QStringLiteral("dodge_burn"), exposure);
        int range = 1;
        if (m_brushDodgeRange == QStringLiteral("shadows"))
            range = 0;
        else if (m_brushDodgeRange == QStringLiteral("highlights"))
            range = 2;
        settings.insert(QStringLiteral("dodge_range"), range);
    }
    // Ink: the nib thins with speed. Absent unless ink mode is on.
    if (m_brushInk)
        settings.insert(QStringLiteral("ink"), 0.7);
    // MyPaint scatter: a grainy, textured line. Absent unless on.
    if (m_brushMyPaint) {
        settings.insert(QStringLiteral("mypaint"),
                        QJsonObject{{QStringLiteral("dabs_per_step"), 4},
                                    {QStringLiteral("radius_jitter"), 0.4},
                                    {QStringLiteral("offset_jitter"), 0.6}});
    }
    // Size dynamics (Krita sensor/preset engine): one sensor bound to size. Absent unless on. A
    // positive amount enlarges where the sensor reads high (fast speed is read inverted so a quick
    // stroke thins, matching a tablet preset).
    if (m_brushSizeDynamic != QStringLiteral("off")) {
        const double amount = (m_brushSizeDynamic == QStringLiteral("speed")) ? -0.8 : 0.8;
        settings.insert(QStringLiteral("dynamics"),
                        QJsonArray{QJsonObject{{QStringLiteral("sensor"), m_brushSizeDynamic},
                                               {QStringLiteral("amount"), amount}}});
    }
    // Opacity and flow bindings (I.1). Same inverted-speed convention as size: a fast stroke reads
    // high on the speed sensor, and a tablet preset wants a quick stroke to go thinner AND lighter.
    if (m_brushOpacityDynamic != QStringLiteral("off")) {
        const double amount = (m_brushOpacityDynamic == QStringLiteral("speed")) ? -0.8 : 0.8;
        settings.insert(QStringLiteral("opacity_dynamics"),
                        QJsonArray{QJsonObject{{QStringLiteral("sensor"), m_brushOpacityDynamic},
                                               {QStringLiteral("amount"), amount}}});
    }
    if (m_brushFlowDynamic != QStringLiteral("off")) {
        const double amount = (m_brushFlowDynamic == QStringLiteral("speed")) ? -0.8 : 0.8;
        settings.insert(QStringLiteral("flow_dynamics"),
                        QJsonArray{QJsonObject{{QStringLiteral("sensor"), m_brushFlowDynamic},
                                               {QStringLiteral("amount"), amount}}});
    }
    // Multihand radial symmetry (Krita multibrush): absent unless order >= 2.
    if (m_brushSymmetryOrder >= 2) {
        settings.insert(QStringLiteral("symmetry_center"),
                        QJsonArray{m_brushSymmetryCenterX, m_brushSymmetryCenterY});
        settings.insert(QStringLiteral("symmetry_order"), m_brushSymmetryOrder);
    }
    // Drawing assistant (Krita assistants): absent when "none".
    if (m_brushAssistantKind == QStringLiteral("vanishing")) {
        settings.insert(QStringLiteral("assistant"),
                        QJsonObject{{QStringLiteral("kind"), QStringLiteral("vanishing_point")},
                                    {QStringLiteral("x"), m_brushAssistantP0},
                                    {QStringLiteral("y"), m_brushAssistantP1}});
    } else if (m_brushAssistantKind == QStringLiteral("parallel")) {
        settings.insert(QStringLiteral("assistant"),
                        QJsonObject{{QStringLiteral("kind"), QStringLiteral("parallel_ruler")},
                                    {QStringLiteral("ax"), m_brushAssistantP0},
                                    {QStringLiteral("ay"), m_brushAssistantP1},
                                    {QStringLiteral("bx"), m_brushAssistantP2},
                                    {QStringLiteral("by"), m_brushAssistantP3}});
    } else if (m_brushAssistantKind == QStringLiteral("ellipse")) {
        settings.insert(QStringLiteral("assistant"),
                        QJsonObject{{QStringLiteral("kind"), QStringLiteral("ellipse")},
                                    {QStringLiteral("cx"), m_brushAssistantP0},
                                    {QStringLiteral("cy"), m_brushAssistantP1},
                                    {QStringLiteral("rx"), m_brushAssistantP2},
                                    {QStringLiteral("ry"), m_brushAssistantP3}});
    }
    // Dyna brush (GIMP dynamic brush): absent unless enabled.
    if (m_brushDynaEnabled) {
        settings.insert(QStringLiteral("dyna"),
                        QJsonArray{m_brushDynaMass, m_brushDynaDrag});
    }
    return settings;
}

bool EditorBridge::executeCommand(const QString &commandJson)
{
    QJsonParseError error;
    const QJsonDocument document = QJsonDocument::fromJson(commandJson.toUtf8(), &error);
    if (error.error != QJsonParseError::NoError || !document.isObject()) {
        setStatus(QStringLiteral("Edit rejected: command must be one valid JSON object (%1)")
                      .arg(error.errorString()));
        return false;
    }
    return executeCommand(document.object());
}

bool EditorBridge::executeCommand(const QJsonObject &command)
{
    if (!m_editor) {
        setStatus(QStringLiteral("Edit rejected: editor is unavailable"));
        return false;
    }
    if (m_projectionStale) {
        setStatus(QStringLiteral("Edit deferred until the committed document is visible"));
        return false;
    }
    if (refuseWhileFilterRuns(QStringLiteral("Edit")))
        return false;
    const QByteArray json = canonicalJson(command);
    // A filter can take seconds on a large image. Run it on a worker so the window keeps
    // painting and answering; every other engine call waits for it (refuseWhileFilterRuns).
    if (command.value(QStringLiteral("type")).toString() == QStringLiteral("apply_filter")) {
        // P14: recorded only once it has succeeded, in finishFilterRun.
        m_pendingFilterCommand = command;
        return startFilterRun(json);
    }
    RedrobBuffer changes{};
    const int status = redrob_editor_execute_json(
        m_editor.get(), reinterpret_cast<const uint8_t *>(json.constData()),
        static_cast<size_t>(json.size()), &changes);
    if (status != REDROB_OK) {
        redrob_buffer_free(changes);
        setStatus(QStringLiteral("Edit rejected: %1").arg(ffiError()));
        return false;
    }
    recordActionStep(command);
    m_playbackTimer.stop();
    const ChangeInvalidation invalidation = changeInvalidation(takeBuffer(changes));
    const bool captureSelection = !invalidation.valid || invalidation.selectionChanged
        || invalidation.canvasChanged;
    setStatus(QStringLiteral("Edit applied"));
    m_lastMutationProjectionRefreshed = refresh(captureSelection);
    if (!m_lastMutationProjectionRefreshed) {
        scheduleProjectionRefresh(captureSelection);
        setStatus(QStringLiteral("Edit committed once; display synchronization is retrying"));
    }
    return true;
}

bool EditorBridge::refuseWhileFilterRuns(const QString &what)
{
    if (!m_filterBusy)
        return false;
    setStatus(QStringLiteral("%1 waits: a filter is still running").arg(what));
    return true;
}

// ---- Actions (P14): record the edits that succeed, save them, play them back ----

void EditorBridge::recordActionStep(const QJsonObject &command)
{
    if (!m_actionRecording || command.isEmpty())
        return;
    // Caret moves are not edits; the engine refuses them in an action, so they are never kept.
    const QString type = command.value(QStringLiteral("type")).toString();
    if (type.endsWith(QStringLiteral("_text_caret")) || type.endsWith(QStringLiteral("_at_text_caret")))
        return;
    if (m_actionSteps.size() >= kMaxActionSteps) {
        setStatus(QStringLiteral("Action recording is full (%1 steps); stop and save it").arg(kMaxActionSteps));
        return;
    }
    m_actionSteps.append(command);
    emit actionChanged();
}

bool EditorBridge::actionRecording() const { return m_actionRecording; }
int EditorBridge::actionStepCount() const { return int(m_actionSteps.size()); }

void EditorBridge::startActionRecording()
{
    m_actionSteps = QJsonArray{};
    m_actionRecording = true;
    setStatus(QStringLiteral("Recording an action: every edit from now on is a step"));
    emit actionChanged();
}

void EditorBridge::stopActionRecording()
{
    if (!m_actionRecording)
        return;
    m_actionRecording = false;
    setStatus(QStringLiteral("Recorded %1 step(s); save the action to keep it").arg(m_actionSteps.size()));
    emit actionChanged();
}

bool EditorBridge::saveAction(const QUrl &fileUrl, const QString &name)
{
    if (m_actionRecording || m_actionSteps.isEmpty()) {
        setStatus(QStringLiteral("Stop a recording with at least one step before saving it"));
        return false;
    }
    const QString trimmed = name.trimmed().isEmpty() ? QStringLiteral("Untitled action") : name.trimmed();
    // The envelope redrob-core's Action reads (ACTION_FORMAT / ACTION_VERSION).
    const QJsonObject action{{QStringLiteral("format"), QStringLiteral("redrob-action")},
                             {QStringLiteral("version"), 1},
                             {QStringLiteral("name"), trimmed.left(256)},
                             {QStringLiteral("commands"), m_actionSteps}};
    QSaveFile file(fileUrl.isLocalFile() ? fileUrl.toLocalFile() : fileUrl.toString());
    if (!file.open(QIODevice::WriteOnly)
        || file.write(QJsonDocument(action).toJson(QJsonDocument::Indented)) < 0 || !file.commit()) {
        setStatus(QStringLiteral("Action not saved: %1").arg(file.errorString()));
        return false;
    }
    setStatus(QStringLiteral("Saved action \"%1\" (%2 steps)").arg(trimmed).arg(m_actionSteps.size()));
    return true;
}

bool EditorBridge::playActionFile(const QUrl &fileUrl)
{
    if (!m_editor || refuseWhileFilterRuns(QStringLiteral("Action")))
        return false;
    if (m_actionRecording) {
        setStatus(QStringLiteral("Stop recording before playing an action"));
        return false;
    }
    QFile file(fileUrl.isLocalFile() ? fileUrl.toLocalFile() : fileUrl.toString());
    if (!file.open(QIODevice::ReadOnly) || file.size() > kMaxActionFileBytes) {
        setStatus(QStringLiteral("Action not played: the file cannot be read or is too large"));
        return false;
    }
    const QByteArray bytes = file.readAll();
    RedrobBuffer changes{};
    // Synchronous: an action of filters can take as long as its filters, like applying them one
    // by one would. It is one undo step either way.
    const int status = redrob_editor_play_action_json(
        m_editor.get(), reinterpret_cast<const uint8_t *>(bytes.constData()),
        static_cast<size_t>(bytes.size()), &changes);
    if (status != REDROB_OK) {
        redrob_buffer_free(changes);
        setStatus(QStringLiteral("Action not played: %1").arg(ffiError()));
        return false;
    }
    m_playbackTimer.stop();
    const ChangeInvalidation invalidation = changeInvalidation(takeBuffer(changes));
    const bool captureSelection = !invalidation.valid || invalidation.selectionChanged
        || invalidation.canvasChanged;
    setStatus(QStringLiteral("Action played as one step; Undo reverts all of it"));
    m_lastMutationProjectionRefreshed = refresh(captureSelection);
    if (!m_lastMutationProjectionRefreshed)
        scheduleProjectionRefresh(captureSelection);
    return true;
}

bool EditorBridge::startFilterRun(const QByteArray &json)
{
    m_filterBusy = true;
    emit filterBusyChanged();
    setStatus(QStringLiteral("Applying filter…"));
    // The editor is shared and mutex-guarded on the Rust side, so the worker owns the call; the
    // GUI thread makes no engine call until finishFilterRun clears the flag.
    m_filterWatcher.setFuture(QtConcurrent::run([editor = m_editor, json] {
        FilterRunResult result;
        RedrobBuffer changes{};
        result.status = redrob_editor_execute_json(
            editor.get(), reinterpret_cast<const uint8_t *>(json.constData()),
            static_cast<size_t>(json.size()), &changes);
        if (result.status == REDROB_OK) {
            result.changes = copyOwnedBuffer(changes);
        } else {
            redrob_buffer_free(changes);
            result.error = currentFfiError();
        }
        return result;
    }));
    return true;
}

void EditorBridge::cancelFilter()
{
    // P8b. Lock-free on the Rust side, so it is safe to call while the worker holds the editor.
    // The worker's result then arrives as "cancelled" through finishFilterRun.
    if (!m_filterBusy || !m_editor)
        return;
    if (redrob_editor_request_cancel(m_editor.get()) == REDROB_OK)
        setStatus(QStringLiteral("Cancelling filter…"));
}

void EditorBridge::finishFilterRun(const FilterRunResult &result)
{
    m_filterBusy = false;
    emit filterBusyChanged();
    if (result.status != REDROB_OK) {
        // P8b. A cancel is the user's own request, not a rejection.
        if (result.error == QStringLiteral("cancelled")) {
            setStatus(QStringLiteral("Filter cancelled; nothing was changed"));
            return;
        }
        setStatus(QStringLiteral("Edit rejected: %1").arg(result.error));
        return;
    }
    recordActionStep(m_pendingFilterCommand);
    m_pendingFilterCommand = {};
    m_playbackTimer.stop();
    const ChangeInvalidation invalidation = changeInvalidation(result.changes);
    const bool captureSelection = !invalidation.valid || invalidation.selectionChanged
        || invalidation.canvasChanged;
    setStatus(QStringLiteral("Edit applied"));
    m_lastMutationProjectionRefreshed = refresh(captureSelection);
    if (!m_lastMutationProjectionRefreshed) {
        scheduleProjectionRefresh(captureSelection);
        setStatus(QStringLiteral("Edit committed once; display synchronization is retrying"));
    }
}

bool EditorBridge::executeHistoryAction(bool redoAction)
{
    if (refuseWhileFilterRuns(redoAction ? QStringLiteral("Redo") : QStringLiteral("Undo")))
        return false;
    if (m_projectionStale && redoAction) {
        setStatus(QStringLiteral("Redo deferred until the committed document is visible"));
        return false;
    }
    RedrobBuffer changes{};
    const int status = !m_editor
        ? REDROB_ERROR
        : (redoAction ? redrob_editor_redo(m_editor.get(), &changes)
                      : redrob_editor_undo(m_editor.get(), &changes));
    if (status != REDROB_OK) {
        redrob_buffer_free(changes);
        setStatus(QStringLiteral("%1 unavailable: %2")
                      .arg(redoAction ? QStringLiteral("Redo") : QStringLiteral("Undo"), ffiError()));
        return false;
    }
    m_playbackTimer.stop();
    const ChangeInvalidation invalidation = changeInvalidation(takeBuffer(changes));
    const bool captureSelection = !invalidation.valid || invalidation.selectionChanged
        || invalidation.canvasChanged;
    setStatus(redoAction ? QStringLiteral("Redid edit") : QStringLiteral("Undid last edit"));
    m_lastMutationProjectionRefreshed = refresh(captureSelection);
    if (!m_lastMutationProjectionRefreshed) {
        scheduleProjectionRefresh(captureSelection);
        setStatus(redoAction
                      ? QStringLiteral("Redo committed once; display synchronization is retrying")
                      : QStringLiteral("Undo committed once; display synchronization is retrying"));
    }
    return true;
}

void EditorBridge::undo() { executeHistoryAction(false); }
void EditorBridge::redo() { executeHistoryAction(true); }

QStringList EditorBridge::historyLabels() const { return m_historyLabels; }

void EditorBridge::jumpToHistory(int stepsDone)
{
    const int target = std::clamp(stepsDone, 0, m_undoDepth + m_redoDepth);
    // Each step refreshes the depths, so the loop condition reads the engine's own count. The
    // budget is the distance measured up front: if a refresh fails and leaves the depth stale, the
    // loop must not keep undoing past the row the user clicked.
    int budget = std::abs(m_undoDepth - target);
    while (m_undoDepth > target && budget-- > 0) {
        if (!executeHistoryAction(false))
            return;
    }
    while (m_undoDepth < target && budget-- > 0) {
        if (!executeHistoryAction(true))
            return;
    }
}

void EditorBridge::injectProjectionFailureForSmokeTest()
{
    if (!qEnvironmentVariableIsSet("REDROB_SMOKE_TEST"))
        return;
    m_projectionStale = true;
    m_retryNeedsSelection = true;
    setStatus(QStringLiteral("Synthetic projection failure injected for smoke recovery"));
}

std::optional<quint32> EditorBridge::allocateFrameId()
{
    if (frameCount() >= FrameIdAllocator::kMaxFrameCount) {
        setStatus(QStringLiteral("Frame creation rejected: the 10,000 frame limit was reached"));
        return std::nullopt;
    }

    QVector<quint32> existingIds;
    existingIds.reserve(frameCount());
    for (int row = 0; row < frameCount(); ++row)
        existingIds.push_back(m_frames.frameIdAt(row));
    const auto id = FrameIdAllocator::allocate(existingIds, [] {
        return QRandomGenerator::global()->generate();
    });
    if (!id)
        setStatus(QStringLiteral("Frame creation rejected: no available frame ID"));
    return id;
}

void EditorBridge::addFrame(int index)
{
    const auto id = allocateFrameId();
    if (!id)
        return;
    const int destination = index < 0 ? frameCount() : qBound(0, index, frameCount());
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_frame")},
                    {QStringLiteral("id"), static_cast<qint64>(*id)},
                    {QStringLiteral("index"), destination}});
}

void EditorBridge::duplicateFrame(quint32 sourceId, int index)
{
    const auto id = allocateFrameId();
    if (!id)
        return;
    const int destination = index < 0 ? frameCount() : qBound(0, index, frameCount());
    executeCommand({{QStringLiteral("type"), QStringLiteral("duplicate_frame")},
                    {QStringLiteral("source"), static_cast<qint64>(sourceId)},
                    {QStringLiteral("id"), static_cast<qint64>(*id)},
                    {QStringLiteral("index"), destination}});
}

void EditorBridge::removeFrame(quint32 id)
{
    if (frameCount() <= 1)
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("remove_frame")},
                    {QStringLiteral("id"), static_cast<qint64>(id)}});
}

void EditorBridge::moveFrame(quint32 id, int newIndex)
{
    if (newIndex < 0 || newIndex >= frameCount())
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("move_frame")},
                    {QStringLiteral("id"), static_cast<qint64>(id)},
                    {QStringLiteral("new_index"), newIndex}});
}

void EditorBridge::setTimelineFps(qreal value)
{
    if (!isFiniteValue(value) || value <= 0.0 || value > 240.0)
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_timeline_fps")},
                    {QStringLiteral("fps"), value}});
}

void EditorBridge::setPlaybackRange(quint32 start, quint32 end)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_playback_range")},
                    {QStringLiteral("start"), static_cast<qint64>(start)},
                    {QStringLiteral("end"), static_cast<qint64>(end)}});
}

void EditorBridge::setLooping(bool enabled)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_looping")},
                    {QStringLiteral("looping"), enabled}});
}

bool EditorBridge::executeNavigation(int kind, quint32 frameId, bool playingState)
{
    if (!m_editor || m_projectionStale || m_filterBusy)
        return false;
    RedrobBuffer changes{};
    int status = REDROB_ERROR;
    if (kind == 0)
        status = redrob_editor_set_current_frame(m_editor.get(), frameId, &changes);
    else if (kind == 1)
        status = redrob_editor_set_playing(m_editor.get(), playingState, &changes);
    else
        status = redrob_editor_advance_playback(m_editor.get(), &changes);
    if (status != REDROB_OK) {
        redrob_buffer_free(changes);
        setStatus(QStringLiteral("Timeline navigation rejected: %1").arg(ffiError()));
        return false;
    }
    const ChangeInvalidation invalidation = changeInvalidation(takeBuffer(changes));
    const bool captureSelection = !invalidation.valid || invalidation.selectionChanged;
    if (!refresh(captureSelection)) {
        scheduleProjectionRefresh(captureSelection);
        setStatus(QStringLiteral("Navigation committed; display synchronization is retrying"));
    }
    return true;
}

bool EditorBridge::setCurrentFrame(quint32 id) { return executeNavigation(0, id); }

bool EditorBridge::setPlaying(bool enabled)
{
    if (!enabled)
        m_playbackTimer.stop();
    if (!executeNavigation(1, 0, enabled))
        return false;
    if (m_playing) {
        m_playbackTimer.setInterval(qMax(1, qRound(1000.0 / m_fps)));
        m_playbackTimer.start();
    } else {
        m_playbackTimer.stop();
    }
    return true;
}

bool EditorBridge::advancePlayback()
{
    if (!executeNavigation(2)) {
        m_playbackTimer.stop();
        return false;
    }
    if (!m_playing)
        m_playbackTimer.stop();
    return true;
}

void EditorBridge::beginStroke(qreal x, qreal y, qreal pressure)
{
    if (m_projectionStale) {
        setStatus(QStringLiteral("Stroke deferred until the committed document is visible"));
        return;
    }
    m_strokePoints = {};
    m_strokeActive = true;
    m_strokeTruncated = false;
    // Aligned clone: fix the source offset at stroke start, so every dab samples a region a constant
    // vector away from the brush (GIMP's aligned clone).
    if (m_brushClone && m_cloneSourceSet) {
        m_cloneOffsetX = x - m_cloneSourceX;
        m_cloneOffsetY = y - m_cloneSourceY;
    }
    // S1. Paint while the pointer is down. The engine refuses (and the stroke commits on release,
    // as before) when the active layer has no cel, a group is open, or onion skin is showing --
    // the live render path draws the plain projection.
    m_livePending = {};
    m_liveStroke = false;
    if (m_editor && !m_filterBusy && !m_onionSkinEnabled) {
        const QByteArray json = canonicalJson(strokeCommand({}));
        m_liveStroke = redrob_editor_live_stroke_begin(
                           m_editor.get(), reinterpret_cast<const uint8_t *>(json.constData()),
                           static_cast<size_t>(json.size()))
            == REDROB_OK;
    }
    addStrokePoint(x, y, pressure);
}

QJsonObject EditorBridge::strokeCommand(const QJsonArray &points) const
{
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("brush_stroke")},
                        {QStringLiteral("points"), points},
                        {QStringLiteral("color"), colorObject(m_brushColor)},
                        {QStringLiteral("size"), m_brushSize},
                        {QStringLiteral("opacity"), m_brushOpacity},
                        {QStringLiteral("settings"), brushSettingsObject()}};
    // Command::BrushStroke::tip replaces the generated dab when present. The GIH pipe sends every
    // loaded tip as `pipe` (cycled per dab) instead of one `tip`.
    if (m_brushPipe && m_brushTips.size() >= 2) {
        command.insert(QStringLiteral("pipe"), m_brushTips);
    } else if (m_brushTipIndex >= 0 && m_brushTipIndex < m_brushTips.size()) {
        command.insert(QStringLiteral("tip"), m_brushTips.at(m_brushTipIndex));
    }
    return command;
}

void EditorBridge::flushLiveStroke()
{
    // Never on the engine while a filter holds it (cannot happen mid-stroke, but cheap to state).
    if (!m_liveStroke || m_livePending.isEmpty() || !m_editor || m_filterBusy)
        return;
    const QByteArray json = QJsonDocument(m_livePending).toJson(QJsonDocument::Compact);
    m_livePending = {};
    RedrobBuffer changes{};
    if (redrob_editor_live_stroke_extend(m_editor.get(), reinterpret_cast<const uint8_t *>(json.constData()),
                                         static_cast<size_t>(json.size()), &changes)
        != REDROB_OK) {
        redrob_buffer_free(changes);
        // Fall back to committing on release; the points are still in m_strokePoints.
        m_liveStroke = false;
        RedrobBuffer cancelled{};
        redrob_editor_live_stroke_cancel(m_editor.get(), &cancelled);
        redrob_buffer_free(cancelled);
        refreshLiveRender();
        return;
    }
    redrob_buffer_free(changes);
    refreshLiveRender();
}

bool EditorBridge::refreshLiveRender()
{
    // The cheap half of refresh(): only the composited picture changes while a stroke is drawn, so
    // the layer list, timeline and history are not re-read on every move.
    if (!m_editor || m_filterBusy)
        return false;
    RedrobRenderSnapshot render{};
    if (redrob_editor_render_rgba(m_editor.get(), &render) != REDROB_OK) {
        redrob_buffer_free(render.rgba);
        return false;
    }
    const quint64 length = quint64(render.stride) * quint64(render.height);
    const bool valid = render.width == static_cast<uint32_t>(m_width)
        && render.height == static_cast<uint32_t>(m_height) && render.stride == render.width * 4u
        && length == render.rgba.len && render.rgba.data != nullptr;
    if (valid) {
        const QImage borrowed(render.rgba.data, m_width, m_height,
                              static_cast<qsizetype>(render.stride), QImage::Format_RGBA8888);
        m_renderImage = borrowed.copy();
        m_renderImage.setDevicePixelRatio(1.0);
    }
    redrob_buffer_free(render.rgba);
    if (valid)
        emit renderImageChanged();
    return valid;
}

void EditorBridge::addStrokePoint(qreal x, qreal y, qreal pressure)
{
    if (!m_strokeActive || !isFiniteValue(x) || !isFiniteValue(y) || !isFiniteValue(pressure)
        || x < 0.0 || y < 0.0 || x >= m_width || y >= m_height)
        return;
    const qreal boundedPressure = qBound(0.0, pressure, 1.0);
    if (!m_strokePoints.isEmpty()) {
        const auto previous = m_strokePoints.last().toObject();
        if (qAbs(previous.value(QStringLiteral("x")).toDouble() - x) < 0.15
            && qAbs(previous.value(QStringLiteral("y")).toDouble() - y) < 0.15)
            return;
    }
    if (m_strokePoints.size() >= kMaxBrushPoints) {
        m_strokeTruncated = true;
        return;
    }
    QJsonObject point{{QStringLiteral("x"), x},
                      {QStringLiteral("y"), y},
                      {QStringLiteral("pressure"), boundedPressure}};
    // P7. Only a leaning pen adds the fields, so a mouse stroke's JSON is what it always was.
    if (m_penTiltX != 0.0 || m_penTiltY != 0.0) {
        point.insert(QStringLiteral("tilt_x"), qBound(-90.0, m_penTiltX, 90.0));
        point.insert(QStringLiteral("tilt_y"), qBound(-90.0, m_penTiltY, 90.0));
    }
    m_strokePoints.append(point);
    if (m_liveStroke) {
        m_livePending.append(point);
        if (!m_liveStrokeTimer.isActive())
            m_liveStrokeTimer.start();
    }
}

bool EditorBridge::eventFilter(QObject *watched, QEvent *event)
{
    switch (event->type()) {
    case QEvent::TabletPress:
    case QEvent::TabletMove:
    case QEvent::TabletRelease: {
        const auto *tablet = static_cast<QTabletEvent *>(event);
        m_penTiltX = isFiniteValue(tablet->xTilt()) ? tablet->xTilt() : 0.0;
        m_penTiltY = isFiniteValue(tablet->yTilt()) ? tablet->yTilt() : 0.0;
        break;
    }
    case QEvent::MouseButtonPress:
    case QEvent::MouseMove: {
        // Qt synthesises mouse events from an unaccepted tablet event; those keep the tilt. A real
        // mouse resets it.
        const auto *mouse = static_cast<QMouseEvent *>(event);
        const QPointingDevice *device = mouse->pointingDevice();
        if (!device || device->type() != QInputDevice::DeviceType::Stylus) {
            m_penTiltX = 0.0;
            m_penTiltY = 0.0;
        }
        break;
    }
    default:
        break;
    }
    return QObject::eventFilter(watched, event);
}

void EditorBridge::endStroke()
{
    if (!m_strokeActive)
        return;
    m_strokeActive = false;
    if (m_strokePoints.isEmpty())
        return;
    const bool truncated = m_strokeTruncated;
    const QJsonObject command = strokeCommand(m_strokePoints);
    m_strokePoints = {};
    m_strokeTruncated = false;
    if (m_liveStroke && m_editor && !m_filterBusy) {
        // S1. Paint what is still queued, then let the engine commit the stroke it has been
        // drawing: one ordinary brush stroke, one undo step, the same pixels the screen showed.
        m_liveStrokeTimer.stop();
        flushLiveStroke();
    }
    if (m_liveStroke && m_editor && !m_filterBusy) {
        m_liveStroke = false;
        RedrobBuffer changes{};
        if (redrob_editor_live_stroke_end(m_editor.get(), &changes) == REDROB_OK) {
            recordActionStep(command);
            m_playbackTimer.stop();
            const ChangeInvalidation invalidation = changeInvalidation(takeBuffer(changes));
            const bool captureSelection = !invalidation.valid || invalidation.selectionChanged
                || invalidation.canvasChanged;
            setStatus(truncated ? QStringLiteral("Stroke applied using the first 4096 points")
                                : QStringLiteral("Edit applied"));
            m_lastMutationProjectionRefreshed = refresh(captureSelection);
            if (!m_lastMutationProjectionRefreshed)
                scheduleProjectionRefresh(captureSelection);
            return;
        }
        // The engine refused the commit (the live paint is already undone by its error path):
        // report it like any rejected edit.
        redrob_buffer_free(changes);
        setStatus(QStringLiteral("Edit rejected: %1").arg(ffiError()));
        refreshLiveRender();
        return;
    }
    m_liveStroke = false;
    if (executeCommand(command) && truncated)
        setStatus(QStringLiteral("Stroke applied using the first 4096 points"));
}

void EditorBridge::cancelStroke()
{
    m_strokeActive = false;
    m_strokeTruncated = false;
    m_strokePoints = {};
    m_livePending = {};
    m_liveStrokeTimer.stop();
    if (m_liveStroke && m_editor && !m_filterBusy) {
        RedrobBuffer changes{};
        redrob_editor_live_stroke_cancel(m_editor.get(), &changes);
        redrob_buffer_free(changes);
        refreshLiveRender();
    }
    m_liveStroke = false;
}

void EditorBridge::fill(const QColor &color)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("fill")},
                    {QStringLiteral("color"), colorObject(color)}});
}

void EditorBridge::floodFill(qreal x, qreal y, const QColor &color, int tolerance)
{
    if (x < 0 || y < 0)
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("flood_fill")},
                    {QStringLiteral("x"), static_cast<int>(x)},
                    {QStringLiteral("y"), static_cast<int>(y)},
                    {QStringLiteral("color"), colorObject(color)},
                    {QStringLiteral("options"),
                     QJsonObject{{QStringLiteral("tolerance"), qBound(0, tolerance, 255)},
                                 {QStringLiteral("opacity_spread"), 100}}}});
}

void EditorBridge::clearActiveLayer()
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("clear")}});
}

void EditorBridge::addLayer(const QString &name)
{
    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("New layer") : name.trimmed();
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_layer")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("index"), m_layers.siblingCount(QString{})}});
}

void EditorBridge::addGroup(const QString &name, const QString &parentId, int siblingIndex)
{
    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("New group") : name.trimmed();
    const int count = m_layers.siblingCount(parentId);
    const int destination = siblingIndex < 0 ? count : qBound(0, siblingIndex, count);
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_group")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"), parentId.isEmpty() ? QJsonValue(QJsonValue::Null)
                                                                  : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), destination}});
}

// Paragraph text (P10). Adds `box_width`/`align` only when not the point-text default, so the
// command JSON for ordinary text is unchanged. Returns false for a value the engine would refuse.
static bool addParagraphFields(QJsonObject &content, qreal boxWidth, const QString &align)
{
    static const QStringList kAligns{QStringLiteral("left"), QStringLiteral("center"),
                                     QStringLiteral("right")};
    if (!kAligns.contains(align))
        return false;
    if (boxWidth >= 0.0) {
        if (!isFiniteValue(boxWidth) || boxWidth <= 0.0 || boxWidth > kMaxSemanticCoordinate)
            return false;
        content.insert(QStringLiteral("box_width"), boxWidth);
    }
    if (align != kAligns.first())
        content.insert(QStringLiteral("align"), align);
    return true;
}

void EditorBridge::addTextNode(const QString &name, const QString &text, qreal originX,
                               qreal originY, qreal fontSize, const QColor &color,
                               const QString &parentId, int siblingIndex, qreal boxWidth,
                               const QString &align)
{
    if (text.size() > kMaxNativeTextCharacters || !isFiniteValue(originX)
        || !isFiniteValue(originY) || !isFiniteValue(fontSize) || fontSize <= 0.0
        || fontSize > 4096.0 || qAbs(originX) > kMaxSemanticCoordinate
        || qAbs(originY) > kMaxSemanticCoordinate) {
        setStatus(QStringLiteral("Text edit rejected: native text/geometry bounds exceeded"));
        return;
    }
    for (const QChar character : text) {
        const ushort code = character.unicode();
        if (code != '\n' && (code < 0x20 || code > 0x7e)) {
            setStatus(QStringLiteral("Text edit rejected: only printable ASCII and newline are supported"));
            return;
        }
    }
    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("New text") : name.trimmed();
    const int count = m_layers.siblingCount(parentId);
    const int destination = siblingIndex < 0 ? count : qBound(0, siblingIndex, count);
    QJsonObject content{{QStringLiteral("text"), text},
                        {QStringLiteral("font_family"), QStringLiteral("font8x8 Basic Latin")},
                        {QStringLiteral("font_size"), fontSize},
                        {QStringLiteral("color"), colorObject(color)},
                        {QStringLiteral("origin_x"), originX},
                        {QStringLiteral("origin_y"), originY},
                        {QStringLiteral("font_id"), QStringLiteral("font8x8-basic-0.3.1")}};
    if (!addParagraphFields(content, boxWidth, align)) {
        setStatus(QStringLiteral("Text edit rejected: invalid paragraph width or alignment"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_text_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"), parentId.isEmpty() ? QJsonValue(QJsonValue::Null)
                                                                  : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), destination},
                    {QStringLiteral("text"), content}});
}

void EditorBridge::setTextContent(const QString &id, const QString &text, qreal originX,
                                  qreal originY, qreal fontSize, const QColor &color,
                                  const QString &fontFamily, const QString &fontId,
                                  qreal boxWidth, const QString &align)
{
    if (id.isEmpty())
        return;
    if (text.size() > kMaxNativeTextCharacters || !isFiniteValue(originX)
        || !isFiniteValue(originY) || !isFiniteValue(fontSize) || fontSize <= 0.0
        || fontSize > 4096.0 || qAbs(originX) > kMaxSemanticCoordinate
        || qAbs(originY) > kMaxSemanticCoordinate || fontFamily.trimmed().isEmpty()
        || fontId != QStringLiteral("font8x8-basic-0.3.1")) {
        setStatus(QStringLiteral("Text edit rejected: native text/geometry bounds exceeded"));
        return;
    }
    for (const QChar character : text) {
        const ushort code = character.unicode();
        if (code != '\n' && (code < 0x20 || code > 0x7e)) {
            setStatus(QStringLiteral("Text edit rejected: only printable ASCII and newline are supported"));
            return;
        }
    }
    QJsonObject content{{QStringLiteral("text"), text},
                        {QStringLiteral("font_family"), fontFamily},
                        {QStringLiteral("font_size"), fontSize},
                        {QStringLiteral("color"), colorObject(color)},
                        {QStringLiteral("origin_x"), originX},
                        {QStringLiteral("origin_y"), originY},
                        {QStringLiteral("font_id"), fontId}};
    if (!addParagraphFields(content, boxWidth, align)) {
        setStatus(QStringLiteral("Text edit rejected: invalid paragraph width or alignment"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_text_content")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("text"), content}});
}

static QJsonObject rectangleVector(qreal x, qreal y, qreal width, qreal height,
                                   const QColor &fill, const QColor &stroke, qreal strokeWidth)
{
    const QJsonArray commands{
        QJsonObject{{QStringLiteral("type"), QStringLiteral("move_to")}, {QStringLiteral("x"), x}, {QStringLiteral("y"), y}},
        QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")}, {QStringLiteral("x"), x + width}, {QStringLiteral("y"), y}},
        QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")}, {QStringLiteral("x"), x + width}, {QStringLiteral("y"), y + height}},
        QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")}, {QStringLiteral("x"), x}, {QStringLiteral("y"), y + height}},
        QJsonObject{{QStringLiteral("type"), QStringLiteral("close")}}};
    const auto rgba = [](const QColor &color) {
        return QJsonObject{{QStringLiteral("r"), color.red()}, {QStringLiteral("g"), color.green()},
                           {QStringLiteral("b"), color.blue()}, {QStringLiteral("a"), color.alpha()}};
    };
    return {{QStringLiteral("paths"), QJsonArray{QJsonObject{
                                             {QStringLiteral("commands"), commands},
                                             {QStringLiteral("fill"), rgba(fill)},
                                             {QStringLiteral("stroke"), QJsonObject{{QStringLiteral("color"), rgba(stroke)}, {QStringLiteral("width"), strokeWidth}}},
                                             {QStringLiteral("fill_rule"), QStringLiteral("non_zero")}}}}};
}

void EditorBridge::addVectorRectangle(const QString &name, qreal x, qreal y, qreal width,
                                      qreal height, const QColor &fill, const QColor &stroke,
                                      qreal strokeWidth, const QString &parentId, int siblingIndex)
{
    if (!isFiniteValue(x) || !isFiniteValue(y) || !isFiniteValue(width)
        || !isFiniteValue(height) || !isFiniteValue(strokeWidth) || width <= 0.0
        || height <= 0.0 || strokeWidth <= 0.0 || strokeWidth > 4096.0
        || qAbs(x) > kMaxSemanticCoordinate || qAbs(y) > kMaxSemanticCoordinate
        || qAbs(x + width) > kMaxSemanticCoordinate || qAbs(y + height) > kMaxSemanticCoordinate) {
        setStatus(QStringLiteral("Vector edit rejected: rectangle geometry is out of range"));
        return;
    }
    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("New vector") : name.trimmed();
    const int count = m_layers.siblingCount(parentId);
    const int destination = siblingIndex < 0 ? count : qBound(0, siblingIndex, count);
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_vector_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"), parentId.isEmpty() ? QJsonValue(QJsonValue::Null) : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), destination},
                    {QStringLiteral("vector"), rectangleVector(x, y, width, height, fill, stroke, strokeWidth)}});
}

void EditorBridge::addVectorPath(const QVariantList &points, bool closed, const QString &name)
{
    // Pen tool: build a straight-segment vector path from the clicked anchors. move_to the first,
    // line_to the rest, optionally close. Filled with the brush colour and a thin brush-colour stroke.
    if (points.size() < 4 || points.size() % 2 != 0) {
        setStatus(QStringLiteral("The pen needs at least two points"));
        return;
    }
    QJsonArray commands;
    for (int i = 0; i + 1 < points.size(); i += 2) {
        const double x = points.at(i).toDouble();
        const double y = points.at(i + 1).toDouble();
        if (!isFiniteValue(x) || !isFiniteValue(y) || qAbs(x) > kMaxSemanticCoordinate
            || qAbs(y) > kMaxSemanticCoordinate)
            return;
        commands.append(QJsonObject{
            {QStringLiteral("type"), i == 0 ? QStringLiteral("move_to") : QStringLiteral("line_to")},
            {QStringLiteral("x"), x},
            {QStringLiteral("y"), y}});
    }
    if (closed)
        commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("close")}});
    const auto rgba = [](const QColor &color) {
        return QJsonObject{{QStringLiteral("r"), color.red()}, {QStringLiteral("g"), color.green()},
                           {QStringLiteral("b"), color.blue()}, {QStringLiteral("a"), color.alpha()}};
    };
    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("Path") : name.trimmed();
    const int count = m_layers.siblingCount(QString());
    QJsonObject path{{QStringLiteral("commands"), commands},
                     {QStringLiteral("stroke"),
                      QJsonObject{{QStringLiteral("color"), rgba(m_brushColor)},
                                  {QStringLiteral("width"), 2.0}}},
                     {QStringLiteral("fill_rule"), QStringLiteral("non_zero")}};
    if (closed)
        path.insert(QStringLiteral("fill"), rgba(m_brushColor));
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_vector_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"), QJsonValue(QJsonValue::Null)},
                    {QStringLiteral("sibling_index"), count},
                    {QStringLiteral("vector"), QJsonObject{{QStringLiteral("paths"), QJsonArray{path}}}}});
}

void EditorBridge::addVectorPathBezier(const QVariantList &anchors, const QVariantList &handles,
                                       bool closed, const QString &name)
{
    // Pen tool with handles (I.2). `anchors` is flat [x,y,...]; `handles` is the same length and holds
    // each anchor's OUTGOING control point. A handle that sits exactly on its own anchor means a
    // CORNER -- which is also what a click with no drag produces, so the degenerate case and the
    // intent agree instead of needing a separate null encoding.
    //
    // The incoming control of an anchor is the MIRROR of its outgoing one through the anchor. That is
    // what makes a dragged handle produce a smooth curve through the point rather than a cusp; storing
    // the two independently would be a second feature (broken handles) and is not this one.
    if (anchors.size() < 4 || anchors.size() % 2 != 0) {
        setStatus(QStringLiteral("The pen needs at least two points"));
        return;
    }
    if (handles.size() != anchors.size()) {
        setStatus(QStringLiteral("Vector edit rejected: one handle per anchor is required"));
        return;
    }
    const int anchorCount = anchors.size() / 2;
    QVector<QPointF> point(anchorCount);
    QVector<QPointF> out(anchorCount);
    for (int i = 0; i < anchorCount; ++i) {
        const double ax = anchors.at(i * 2).toDouble();
        const double ay = anchors.at(i * 2 + 1).toDouble();
        const double hx = handles.at(i * 2).toDouble();
        const double hy = handles.at(i * 2 + 1).toDouble();
        if (!isFiniteValue(ax) || !isFiniteValue(ay) || !isFiniteValue(hx) || !isFiniteValue(hy)
            || qAbs(ax) > kMaxSemanticCoordinate || qAbs(ay) > kMaxSemanticCoordinate
            || qAbs(hx) > kMaxSemanticCoordinate || qAbs(hy) > kMaxSemanticCoordinate)
            return;
        point[i] = QPointF(ax, ay);
        out[i] = QPointF(hx, hy);
    }
    // A handle within half a pixel of its anchor is a corner: a sub-pixel drag is a click that moved,
    // and honouring it would put a control point on top of the anchor, which degenerates the cubic.
    const auto isCorner = [&](int i) {
        const QPointF d = out[i] - point[i];
        return (d.x() * d.x() + d.y() * d.y()) < 0.25;
    };

    QJsonArray commands;
    commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("move_to")},
                                {QStringLiteral("x"), point[0].x()},
                                {QStringLiteral("y"), point[0].y()}});
    // One segment per adjacent pair, plus the wrap-around pair when the path closes -- the closing
    // segment is a curve too, which a bare `close` would flatten to a straight line.
    const int segments = closed ? anchorCount : anchorCount - 1;
    for (int s = 0; s < segments; ++s) {
        const int a = s;
        const int b = (s + 1) % anchorCount;
        if (isCorner(a) && isCorner(b)) {
            commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")},
                                        {QStringLiteral("x"), point[b].x()},
                                        {QStringLiteral("y"), point[b].y()}});
            continue;
        }
        // A corner end contributes its own position as the control point, which makes the cubic leave
        // or arrive straight on that side while still curving on the other.
        const QPointF c1 = isCorner(a) ? point[a] : out[a];
        const QPointF c2 = isCorner(b) ? point[b] : (point[b] * 2.0 - out[b]);
        if (qAbs(c1.x()) > kMaxSemanticCoordinate || qAbs(c1.y()) > kMaxSemanticCoordinate
            || qAbs(c2.x()) > kMaxSemanticCoordinate || qAbs(c2.y()) > kMaxSemanticCoordinate)
            return;
        commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("cubic_to")},
                                    {QStringLiteral("control1_x"), c1.x()},
                                    {QStringLiteral("control1_y"), c1.y()},
                                    {QStringLiteral("control2_x"), c2.x()},
                                    {QStringLiteral("control2_y"), c2.y()},
                                    {QStringLiteral("x"), point[b].x()},
                                    {QStringLiteral("y"), point[b].y()}});
    }
    if (closed)
        commands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("close")}});

    const auto rgba = [](const QColor &color) {
        return QJsonObject{{QStringLiteral("r"), color.red()}, {QStringLiteral("g"), color.green()},
                           {QStringLiteral("b"), color.blue()}, {QStringLiteral("a"), color.alpha()}};
    };
    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("Path") : name.trimmed();
    const int count = m_layers.siblingCount(QString());
    QJsonObject path{{QStringLiteral("commands"), commands},
                     {QStringLiteral("stroke"),
                      QJsonObject{{QStringLiteral("color"), rgba(m_brushColor)},
                                  {QStringLiteral("width"), 2.0}}},
                     {QStringLiteral("fill_rule"), QStringLiteral("non_zero")}};
    if (closed)
        path.insert(QStringLiteral("fill"), rgba(m_brushColor));
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_vector_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"), QJsonValue(QJsonValue::Null)},
                    {QStringLiteral("sibling_index"), count},
                    {QStringLiteral("vector"), QJsonObject{{QStringLiteral("paths"), QJsonArray{path}}}}});
}

void EditorBridge::setVectorRectangle(const QString &id, qreal x, qreal y, qreal width,
                                      qreal height, const QColor &fill, const QColor &stroke,
                                      qreal strokeWidth)
{
    if (id.isEmpty() || !isFiniteValue(x) || !isFiniteValue(y) || !isFiniteValue(width)
        || !isFiniteValue(height) || !isFiniteValue(strokeWidth) || width <= 0.0
        || height <= 0.0 || strokeWidth <= 0.0 || strokeWidth > 4096.0
        || qAbs(x) > kMaxSemanticCoordinate || qAbs(y) > kMaxSemanticCoordinate
        || qAbs(x + width) > kMaxSemanticCoordinate || qAbs(y + height) > kMaxSemanticCoordinate) {
        setStatus(QStringLiteral("Vector edit rejected: rectangle geometry is out of range"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_vector_content")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("vector"), rectangleVector(x, y, width, height, fill, stroke, strokeWidth)}});
}

namespace {

// Paint for a constructed shape. `commands` is deliberately empty: `AddShapeNode` ignores it
// and the shape supplies the geometry. Sending an empty array rather than omitting the key
// keeps the payload explicit about that.
QJsonObject shapePaint(const QColor &fill, const QColor &stroke, qreal strokeWidth)
{
    const auto rgba = [](const QColor &color) {
        return QJsonObject{{QStringLiteral("r"), color.red()},
                           {QStringLiteral("g"), color.green()},
                           {QStringLiteral("b"), color.blue()},
                           {QStringLiteral("a"), color.alpha()}};
    };
    QJsonObject paint{{QStringLiteral("commands"), QJsonArray{}},
                      {QStringLiteral("fill"), rgba(fill)},
                      {QStringLiteral("fill_rule"), QStringLiteral("non_zero")}};
    // A zero stroke width means "no outline", which is a null stroke rather than a stroke of
    // width zero -- the latter renders as a hairline on some backends and as nothing on
    // others, and the document type makes the distinction available, so use it.
    if (strokeWidth > 0.0) {
        paint.insert(QStringLiteral("stroke"),
                     QJsonObject{{QStringLiteral("color"), rgba(stroke)},
                                 {QStringLiteral("width"), strokeWidth}});
    }
    return paint;
}

bool shapeStrokeInRange(qreal strokeWidth)
{
    return isFiniteValue(strokeWidth) && strokeWidth >= 0.0 && strokeWidth <= 4096.0;
}

} // namespace

void EditorBridge::addShapeFromBox(const QString &kind, const QString &name, qreal x1, qreal y1,
                                   qreal x2, qreal y2, qreal cornerRadius, const QColor &fill,
                                   const QColor &stroke, qreal strokeWidth,
                                   const QString &parentId, int siblingIndex)
{
    static const QSet<QString> kBoxKinds{QStringLiteral("rectangle"),
                                         QStringLiteral("rounded_rectangle"),
                                         QStringLiteral("ellipse"), QStringLiteral("line")};
    if (!kBoxKinds.contains(kind)) {
        setStatus(QStringLiteral("Shape rejected: unknown two-corner shape"));
        return;
    }
    if (!isFiniteValue(x1) || !isFiniteValue(y1) || !isFiniteValue(x2) || !isFiniteValue(y2)
        || !isFiniteValue(cornerRadius) || cornerRadius < 0.0 || !shapeStrokeInRange(strokeWidth)
        || qAbs(x1) > kMaxSemanticCoordinate || qAbs(y1) > kMaxSemanticCoordinate
        || qAbs(x2) > kMaxSemanticCoordinate || qAbs(y2) > kMaxSemanticCoordinate) {
        setStatus(QStringLiteral("Shape rejected: geometry is out of range"));
        return;
    }
    // A line may be drawn in any direction, but every other shape here needs a box with area:
    // Graphite's ellipse of zero height is a degenerate path, not an error the core catches,
    // so it is caught where the gesture is interpreted.
    if (kind != QStringLiteral("line")
        && (qFuzzyCompare(x1, x2) || qFuzzyCompare(y1, y2))) {
        setStatus(QStringLiteral("Shape rejected: drag further to give the shape an area"));
        return;
    }

    QJsonObject shape{{QStringLiteral("shape"), kind},
                      {QStringLiteral("x1"), x1},
                      {QStringLiteral("y1"), y1},
                      {QStringLiteral("x2"), x2},
                      {QStringLiteral("y2"), y2}};
    if (kind == QStringLiteral("rounded_rectangle")) {
        // Clamp rather than reject: a corner radius past half the shorter side is what a drag
        // that shrinks under a fixed radius setting produces every time, and the user's intent
        // there is plainly "as round as it goes".
        const qreal limit = qMin(qAbs(x2 - x1), qAbs(y2 - y1)) / 2.0;
        shape.insert(QStringLiteral("radius"), qMin(cornerRadius, limit));
    }

    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("Shape") : name.trimmed();
    const int count = m_layers.siblingCount(parentId);
    const int destination = siblingIndex < 0 ? count : qBound(0, siblingIndex, count);
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_shape_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"),
                     parentId.isEmpty() ? QJsonValue(QJsonValue::Null) : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), destination},
                    {QStringLiteral("shape"), shape},
                    {QStringLiteral("paint"), shapePaint(fill, stroke, strokeWidth)}});
}

void EditorBridge::addShapeFromRadius(const QString &kind, const QString &name, qreal centreX,
                                      qreal centreY, qreal radius, int sides, qreal innerRatio,
                                      const QColor &fill, const QColor &stroke, qreal strokeWidth,
                                      const QString &parentId, int siblingIndex)
{
    const bool isStar = kind == QStringLiteral("star");
    if (!isStar && kind != QStringLiteral("regular_polygon")) {
        setStatus(QStringLiteral("Shape rejected: unknown centre-and-radius shape"));
        return;
    }
    if (!isFiniteValue(centreX) || !isFiniteValue(centreY) || !isFiniteValue(radius)
        || radius <= 0.0 || !shapeStrokeInRange(strokeWidth)
        || qAbs(centreX) + radius > kMaxSemanticCoordinate
        || qAbs(centreY) + radius > kMaxSemanticCoordinate) {
        setStatus(QStringLiteral("Shape rejected: geometry is out of range"));
        return;
    }
    // The core's own bound, restated here so the status line can say what is wrong instead of
    // surfacing a command error after the gesture has already been made.
    if (sides < 3 || sides > 512) {
        setStatus(QStringLiteral("Shape rejected: a polygon needs 3 to 512 sides"));
        return;
    }

    QJsonObject shape{{QStringLiteral("shape"), kind},
                      {QStringLiteral("center_x"), centreX},
                      {QStringLiteral("center_y"), centreY},
                      {QStringLiteral("sides"), sides},
                      {QStringLiteral("radius"), radius}};
    if (isStar) {
        // A RATIO, not an inner radius. A slider from 0 to 1 cannot produce an inner radius
        // that exceeds the outer one, so the star stays valid however the user drags -- and
        // the shape keeps its proportions when the outer radius changes, which is what
        // dragging a star larger is expected to do.
        if (!isFiniteValue(innerRatio) || innerRatio <= 0.0 || innerRatio >= 1.0) {
            setStatus(QStringLiteral("Shape rejected: star inner ratio must be between 0 and 1"));
            return;
        }
        shape.insert(QStringLiteral("inner_radius"), radius * innerRatio);
    }

    const QString safeName = name.trimmed().isEmpty() ? QStringLiteral("Shape") : name.trimmed();
    const int count = m_layers.siblingCount(parentId);
    const int destination = siblingIndex < 0 ? count : qBound(0, siblingIndex, count);
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_shape_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), safeName},
                    {QStringLiteral("parent"),
                     parentId.isEmpty() ? QJsonValue(QJsonValue::Null) : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), destination},
                    {QStringLiteral("shape"), shape},
                    {QStringLiteral("paint"), shapePaint(fill, stroke, strokeWidth)}});
}

void EditorBridge::rasterizeSemanticNode(const QString &id)
{
    if (id.isEmpty())
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("rasterize_semantic_node")},
                    {QStringLiteral("id"), id}});
}

void EditorBridge::moveNode(const QString &id, const QString &parentId, int siblingIndex)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("move_node")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("parent"), parentId.isEmpty() ? QJsonValue(QJsonValue::Null)
                                                                  : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), qMax(0, siblingIndex)}});
}

void EditorBridge::addRasterMask(const QString &id)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_raster_mask")},
                    {QStringLiteral("id"), id}});
}

void EditorBridge::rasterMaskFromSelection(const QString &id)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("raster_mask_from_selection")},
                    {QStringLiteral("id"), id}});
}

void EditorBridge::removeRasterMask(const QString &id)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("remove_raster_mask")},
                    {QStringLiteral("id"), id}});
}

void EditorBridge::setRasterMaskEnabled(const QString &id, bool enabled)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_raster_mask_enabled")},
                    {QStringLiteral("id"), id}, {QStringLiteral("enabled"), enabled}});
}

void EditorBridge::replaceRasterMask(const QString &id, int x, int y, int width, int height,
                                     const QVariantList &pixels)
{
    constexpr qsizetype maxMaskCommandPixels = 256 * 1024;
    if (x < 0 || y < 0 || width < 1 || height < 1 || qint64(x) + width > m_width
        || qint64(y) + height > m_height || qint64(width) * height > maxMaskCommandPixels
        || pixels.size() != qint64(width) * height) {
        setStatus(QStringLiteral("Mask replacement rejected: rectangle or payload is invalid"));
        return;
    }
    QJsonArray coverage;
    for (const QVariant &value : pixels) {
        bool ok = false;
        const int sample = value.toInt(&ok);
        if (!ok || sample < 0 || sample > 255) {
            setStatus(QStringLiteral("Mask replacement rejected: coverage must be 0 through 255"));
            return;
        }
        coverage.append(sample);
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("replace_raster_mask")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("rect"), QJsonObject{{QStringLiteral("x"), x},
                                                          {QStringLiteral("y"), y},
                                                          {QStringLiteral("width"), width},
                                                          {QStringLiteral("height"), height}}},
                    {QStringLiteral("pixels"), coverage}});
}

void EditorBridge::deleteLayer(const QString &id)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("remove_layer")},
                    {QStringLiteral("id"), id}});
}

void EditorBridge::setActiveLayer(const QString &id)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_active_layer")},
                    {QStringLiteral("id"), id}});
}

void EditorBridge::renameLayer(const QString &id, const QString &name)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("rename_layer")},
                    {QStringLiteral("id"), id}, {QStringLiteral("name"), name}});
}

void EditorBridge::setLayerOpacity(const QString &id, qreal opacity)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_layer_opacity")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("opacity"), qBound(0.0, opacity, 1.0)}});
}

void EditorBridge::setLayerVisibility(const QString &id, bool visible)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_layer_visibility")},
                    {QStringLiteral("id"), id}, {QStringLiteral("visible"), visible}});
}

void EditorBridge::setLayerBlendMode(const QString &id, const QString &mode)
{
    static const QStringList modes{QStringLiteral("normal"), QStringLiteral("multiply"),
                                   QStringLiteral("screen"), QStringLiteral("overlay"),
                                   QStringLiteral("add"), QStringLiteral("darken_only"),
                                   QStringLiteral("lighten_only"), QStringLiteral("luma_darken_only"),
                                   QStringLiteral("luma_lighten_only"), QStringLiteral("dodge"),
                                   QStringLiteral("burn"), QStringLiteral("linear_burn"),
                                   QStringLiteral("linear_light"), QStringLiteral("vivid_light"),
                                   QStringLiteral("pin_light"), QStringLiteral("hard_mix"),
                                   QStringLiteral("hard_light"), QStringLiteral("soft_light"),
                                   QStringLiteral("grain_extract"), QStringLiteral("grain_merge"),
                                   QStringLiteral("difference"), QStringLiteral("exclusion"),
                                   QStringLiteral("subtract"), QStringLiteral("divide"),
                                   QStringLiteral("hsv_hue"), QStringLiteral("hsv_saturation"),
                                   QStringLiteral("hsv_value"), QStringLiteral("hsl_color"),
                                   QStringLiteral("lch_hue"), QStringLiteral("lch_chroma"),
                                   QStringLiteral("lch_color"), QStringLiteral("lch_lightness"),
                                   QStringLiteral("luminance"), QStringLiteral("dissolve"),
                                   QStringLiteral("behind"), QStringLiteral("erase"),
                                   QStringLiteral("anti_erase"), QStringLiteral("color_erase"),
                                   QStringLiteral("replace"), QStringLiteral("overwrite"),
                                   QStringLiteral("pass_through"), QStringLiteral("merge"),
                                   QStringLiteral("split")};
    if (!modes.contains(mode)) {
        setStatus(QStringLiteral("Unknown blend mode"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_layer_blend_mode")},
                    {QStringLiteral("id"), id}, {QStringLiteral("mode"), mode}});
}

void EditorBridge::reorderLayer(const QString &id, int newIndex)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("reorder_layer")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("new_index"), qBound(0, newIndex, qMax(0, m_layers.layerCount() - 1))}});
}

void EditorBridge::selectRectangle(qreal x, qreal y, qreal width, qreal height, const QString &mode)
{
    if (!validSelectionMode(mode)) {
        setStatus(QStringLiteral("Unknown selection mode"));
        return;
    }
    QJsonObject rect;
    if (!rectObject(x, y, width, height, &rect)) {
        setStatus(QStringLiteral("Selection rejected: rectangle must be finite, non-empty, and at most 32768 pixels per side"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("select_rectangle")},
                    {QStringLiteral("rect"), rect},
                    {QStringLiteral("mode"), mode}});
}

void EditorBridge::selectEllipse(qreal x, qreal y, qreal width, qreal height, const QString &mode)
{
    if (!validSelectionMode(mode)) {
        setStatus(QStringLiteral("Unknown selection mode"));
        return;
    }
    QJsonObject rect;
    if (!rectObject(x, y, width, height, &rect)) {
        setStatus(QStringLiteral("Selection rejected: ellipse bounds must be finite, non-empty, and at most 32768 pixels per side"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("select_ellipse")},
                    {QStringLiteral("rect"), rect},
                    {QStringLiteral("mode"), mode}});
}

void EditorBridge::selectPolygon(const QVariantList &points, const QString &mode)
{
    if (!validSelectionMode(mode)) {
        setStatus(QStringLiteral("Unknown selection mode"));
        return;
    }
    // points is a flat list [x0, y0, x1, y1, ...] from QML; pack into [[x,y], ...] for the command.
    if (points.size() < 6 || points.size() % 2 != 0) {
        setStatus(QStringLiteral("Lasso needs at least three points"));
        return;
    }
    QJsonArray pts;
    for (int i = 0; i + 1 < points.size(); i += 2) {
        const double x = points.at(i).toDouble();
        const double y = points.at(i + 1).toDouble();
        if (!isFiniteValue(x) || !isFiniteValue(y))
            return;
        pts.append(QJsonArray{x, y});
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("select_polygon")},
                    {QStringLiteral("points"), pts},
                    {QStringLiteral("mode"), mode}});
}

void EditorBridge::selectByColor(qreal x, qreal y, int tolerance, bool contiguous,
                                 const QString &mode)
{
    if (!validSelectionMode(mode)) {
        setStatus(QStringLiteral("Unknown selection mode"));
        return;
    }
    if (!isFiniteValue(x) || !isFiniteValue(y) || x < 0.0 || y < 0.0 || x >= m_width
        || y >= m_height)
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("select_by_color")},
                    {QStringLiteral("x"), static_cast<int>(x)},
                    {QStringLiteral("y"), static_cast<int>(y)},
                    {QStringLiteral("tolerance"), qBound(0, tolerance, 255)},
                    {QStringLiteral("contiguous"), contiguous},
                    {QStringLiteral("mode"), mode}});
}

void EditorBridge::selectScissors(const QVariantList &anchors, const QString &mode)
{
    if (!validSelectionMode(mode)) {
        setStatus(QStringLiteral("Unknown selection mode"));
        return;
    }
    if (anchors.size() < 4 || anchors.size() % 2 != 0) {
        setStatus(QStringLiteral("Scissors needs at least two anchors"));
        return;
    }
    QJsonArray pts;
    for (int i = 0; i + 1 < anchors.size(); i += 2) {
        const double x = anchors.at(i).toDouble();
        const double y = anchors.at(i + 1).toDouble();
        if (!isFiniteValue(x) || !isFiniteValue(y) || x < 0.0 || y < 0.0 || x >= m_width
            || y >= m_height)
            return;
        pts.append(QJsonArray{static_cast<int>(x), static_cast<int>(y)});
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("select_scissors")},
                    {QStringLiteral("anchors"), pts},
                    {QStringLiteral("mode"), mode}});
}

void EditorBridge::selectForeground(const QVariantList &fg, const QVariantList &bg,
                                    const QString &mode)
{
    if (!validSelectionMode(mode)) {
        setStatus(QStringLiteral("Unknown selection mode"));
        return;
    }
    const auto pack = [this](const QVariantList &marks, QJsonArray *out) -> bool {
        if (marks.size() % 2 != 0)
            return false;
        for (int i = 0; i + 1 < marks.size(); i += 2) {
            const double x = marks.at(i).toDouble();
            const double y = marks.at(i + 1).toDouble();
            if (!isFiniteValue(x) || !isFiniteValue(y) || x < 0.0 || y < 0.0 || x >= m_width
                || y >= m_height)
                return false;
            out->append(QJsonArray{static_cast<int>(x), static_cast<int>(y)});
        }
        return true;
    };
    QJsonArray fgPts;
    QJsonArray bgPts;
    if (!pack(fg, &fgPts) || !pack(bg, &bgPts) || fgPts.isEmpty()) {
        setStatus(QStringLiteral("Foreground select needs foreground marks"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("select_foreground")},
                    {QStringLiteral("fg"), fgPts},
                    {QStringLiteral("bg"), bgPts},
                    {QStringLiteral("mode"), mode}});
}

void EditorBridge::alignActiveLayer(int horizontal, int vertical, bool toCanvas)
{
    const QString id = m_layers.activeLayerId();
    if (id.isEmpty())
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("align_layers")},
                    {QStringLiteral("ids"), QJsonArray{id}},
                    {QStringLiteral("h"), qBound(0, horizontal, 3)},
                    {QStringLiteral("v"), qBound(0, vertical, 3)},
                    {QStringLiteral("to_canvas"), toCanvas}});
}

void EditorBridge::selectAll() { executeCommand({{QStringLiteral("type"), QStringLiteral("select_all")}}); }
void EditorBridge::invertSelection() { executeCommand({{QStringLiteral("type"), QStringLiteral("invert_selection")}}); }
void EditorBridge::clearSelection() { executeCommand({{QStringLiteral("type"), QStringLiteral("clear_selection")}}); }
void EditorBridge::featherSelection(int radius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("feather_selection")},
                    {QStringLiteral("radius"), qBound(0, radius, 4096)}});
}
void EditorBridge::growSelection(int radius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("grow_selection")},
                    {QStringLiteral("radius"), qBound(0, radius, 4096)}});
}
void EditorBridge::shrinkSelection(int radius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("shrink_selection")},
                    {QStringLiteral("radius"), qBound(0, radius, 4096)}});
}

void EditorBridge::linearGradient(qreal startX, qreal startY, qreal endX, qreal endY,
                                  const QColor &startColor, const QColor &endColor)
{
    QJsonObject kind{{QStringLiteral("kind"), QStringLiteral("linear")},
                     {QStringLiteral("start_x"), startX}, {QStringLiteral("start_y"), startY},
                     {QStringLiteral("end_x"), endX}, {QStringLiteral("end_y"), endY}};
    QJsonArray stops{QJsonObject{{QStringLiteral("position"), 0.0},
                                 {QStringLiteral("color"), colorObject(startColor)}},
                     QJsonObject{{QStringLiteral("position"), 1.0},
                                 {QStringLiteral("color"), colorObject(endColor)}}};
    executeCommand({{QStringLiteral("type"), QStringLiteral("gradient_fill")},
                    {QStringLiteral("kind"), kind}, {QStringLiteral("stops"), stops}});
}

void EditorBridge::radialGradient(qreal centerX, qreal centerY, qreal radius,
                                  const QColor &startColor, const QColor &endColor)
{
    QJsonObject kind{{QStringLiteral("kind"), QStringLiteral("radial")},
                     {QStringLiteral("center_x"), centerX},
                     {QStringLiteral("center_y"), centerY},
                     {QStringLiteral("radius"), qMax(0.001, radius)}};
    QJsonArray stops{QJsonObject{{QStringLiteral("position"), 0.0},
                                 {QStringLiteral("color"), colorObject(startColor)}},
                     QJsonObject{{QStringLiteral("position"), 1.0},
                                 {QStringLiteral("color"), colorObject(endColor)}}};
    executeCommand({{QStringLiteral("type"), QStringLiteral("gradient_fill")},
                    {QStringLiteral("kind"), kind}, {QStringLiteral("stops"), stops}});
}

void EditorBridge::cropCanvas(qreal x, qreal y, qreal width, qreal height)
{
    QJsonObject rect;
    if (!rectObject(x, y, width, height, &rect)) {
        setStatus(QStringLiteral("Crop rejected: rectangle must be finite, non-empty, and at most 32768 pixels per side"));
        return;
    }
    const qint64 pixels = qint64(rect.value(QStringLiteral("width")).toInt())
        * qint64(rect.value(QStringLiteral("height")).toInt());
    if (pixels > kMaxCanvasPixels) {
        setStatus(QStringLiteral("Crop rejected: target exceeds the 64 Mi-pixel canvas limit"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("crop_canvas")},
                    {QStringLiteral("rect"), rect}});
}

void EditorBridge::padCanvas(int left, int top, int right, int bottom)
{
    if (left < 0 || top < 0 || right < 0 || bottom < 0) {
        setStatus(QStringLiteral("Canvas padding rejected: sides cannot be negative"));
        return;
    }
    const qint64 targetWidth = qint64(m_width) + qint64(left) + qint64(right);
    const qint64 targetHeight = qint64(m_height) + qint64(top) + qint64(bottom);
    if (targetWidth < 1 || targetHeight < 1 || targetWidth > kMaxCanvasDimension
        || targetHeight > kMaxCanvasDimension
        || targetWidth > kMaxCanvasPixels / targetHeight) {
        setStatus(QStringLiteral("Canvas padding rejected: target must fit 32768 pixels per side and 64 Mi-pixels total"));
        return;
    }
    QJsonObject rect;
    if (!rectObject(-qreal(left), -qreal(top), qreal(targetWidth), qreal(targetHeight), &rect)) {
        setStatus(QStringLiteral("Canvas padding rejected: invalid target rectangle"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("crop_canvas")},
                    {QStringLiteral("rect"), rect}});
}

void EditorBridge::resizeCanvas(int width, int height, const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    if (width < 1 || height < 1 || width > kMaxCanvasDimension
        || height > kMaxCanvasDimension || qint64(width) > kMaxCanvasPixels / qint64(height)) {
        setStatus(QStringLiteral("Resize rejected: target must fit 32768 pixels per side and 64 Mi-pixels total"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("resize_canvas")},
                    {QStringLiteral("width"), width},
                    {QStringLiteral("height"), height},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::flipActive(bool horizontal, bool vertical)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("flip_active")},
                    {QStringLiteral("horizontal"), horizontal},
                    {QStringLiteral("vertical"), vertical}});
}

void EditorBridge::rotateActive90(bool clockwise)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("rotate_active90")},
                    {QStringLiteral("clockwise"), clockwise}});
}

void EditorBridge::transformActive(qreal m11, qreal m12, qreal m21, qreal m22,
                                   qreal tx, qreal ty, const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    const QJsonObject transform{{QStringLiteral("m11"), m11}, {QStringLiteral("m12"), m12},
                                {QStringLiteral("m21"), m21}, {QStringLiteral("m22"), m22},
                                {QStringLiteral("tx"), tx}, {QStringLiteral("ty"), ty}};
    executeCommand({{QStringLiteral("type"), QStringLiteral("transform_active")},
                    {QStringLiteral("transform"), transform},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::rotateActive(qreal degrees, const QString &sampling)
{
    if (!isFiniteValue(degrees))
        return;
    const double rad = degrees * 3.14159265358979323846 / 180.0;
    const double c = std::cos(rad);
    const double s = std::sin(rad);
    // Rotate about the canvas centre: tx/ty = centre - R*centre.
    const double cx = m_width * 0.5;
    const double cy = m_height * 0.5;
    const double tx = cx - (c * cx - s * cy);
    const double ty = cy - (s * cx + c * cy);
    transformActive(c, -s, s, c, tx, ty, sampling);
}

void EditorBridge::scaleActive(qreal sx, qreal sy, const QString &sampling)
{
    if (!isFiniteValue(sx) || !isFiniteValue(sy) || sx == 0.0 || sy == 0.0)
        return;
    const double cx = m_width * 0.5;
    const double cy = m_height * 0.5;
    transformActive(sx, 0.0, 0.0, sy, cx - sx * cx, cy - sy * cy, sampling);
}

void EditorBridge::shearActive(qreal shearX, qreal shearY, const QString &sampling)
{
    if (!isFiniteValue(shearX) || !isFiniteValue(shearY))
        return;
    const double cx = m_width * 0.5;
    const double cy = m_height * 0.5;
    // Shear about the centre: [[1, shx],[shy, 1]].
    const double tx = cx - (cx + shearX * cy);
    const double ty = cy - (shearY * cx + cy);
    transformActive(1.0, shearX, shearY, 1.0, tx, ty, sampling);
}

void EditorBridge::perspectiveActive(const QVariantList &corners, const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    if (corners.size() != 8) {
        setStatus(QStringLiteral("Perspective needs four destination corners"));
        return;
    }
    QJsonArray pts;
    for (int i = 0; i + 1 < corners.size(); i += 2) {
        const double x = corners.at(i).toDouble();
        const double y = corners.at(i + 1).toDouble();
        if (!isFiniteValue(x) || !isFiniteValue(y))
            return;
        pts.append(QJsonArray{x, y});
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("perspective_active")},
                    {QStringLiteral("corners"), pts},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::cageTransform(const QVariantList &srcCage, const QVariantList &dstCage,
                                 const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    if (srcCage.size() != dstCage.size() || srcCage.size() < 6 || (srcCage.size() % 2) != 0) {
        setStatus(QStringLiteral("Cage needs matching source and destination polygons"));
        return;
    }
    const auto pack = [](const QVariantList &flat, QJsonArray &out) -> bool {
        for (int i = 0; i + 1 < flat.size(); i += 2) {
            const double x = flat.at(i).toDouble();
            const double y = flat.at(i + 1).toDouble();
            if (!isFiniteValue(x) || !isFiniteValue(y))
                return false;
            out.append(QJsonArray{x, y});
        }
        return true;
    };
    QJsonArray src;
    QJsonArray dst;
    if (!pack(srcCage, src) || !pack(dstCage, dst))
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("cage_transform")},
                    {QStringLiteral("src_cage"), src},
                    {QStringLiteral("dst_cage"), dst},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::warpBrush(const QVariantList &points, const QString &mode, qreal radius,
                             qreal strength, const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    static const QStringList modes{QStringLiteral("move"), QStringLiteral("grow"),
                                   QStringLiteral("shrink"), QStringLiteral("swirl_cw"),
                                   QStringLiteral("swirl_ccw")};
    if (!modes.contains(mode)) {
        setStatus(QStringLiteral("Unknown warp mode"));
        return;
    }
    if (points.size() < 2 || (points.size() % 2) != 0 || radius <= 0.0)
        return;
    QJsonArray pts;
    for (int i = 0; i + 1 < points.size(); i += 2) {
        const double x = points.at(i).toDouble();
        const double y = points.at(i + 1).toDouble();
        if (!isFiniteValue(x) || !isFiniteValue(y))
            return;
        pts.append(QJsonArray{x, y});
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("warp_brush")},
                    {QStringLiteral("points"), pts},
                    {QStringLiteral("mode"), mode},
                    {QStringLiteral("radius"), radius},
                    {QStringLiteral("strength"), strength},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::nPointTransform(const QVariantList &srcPts, const QVariantList &dstPts,
                                   const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    if (srcPts.size() != dstPts.size() || srcPts.size() < 4 || (srcPts.size() % 2) != 0) {
        setStatus(QStringLiteral("N-point needs matching source and destination points"));
        return;
    }
    const auto pack = [](const QVariantList &flat, QJsonArray &out) -> bool {
        for (int i = 0; i + 1 < flat.size(); i += 2) {
            const double x = flat.at(i).toDouble();
            const double y = flat.at(i + 1).toDouble();
            if (!isFiniteValue(x) || !isFiniteValue(y))
                return false;
            out.append(QJsonArray{x, y});
        }
        return true;
    };
    QJsonArray src;
    QJsonArray dst;
    if (!pack(srcPts, src) || !pack(dstPts, dst))
        return;
    executeCommand({{QStringLiteral("type"), QStringLiteral("n_point_transform")},
                    {QStringLiteral("src_pts"), src},
                    {QStringLiteral("dst_pts"), dst},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::transform3d(qreal rotXDeg, qreal rotYDeg, qreal rotZDeg, qreal distance,
                               const QString &sampling)
{
    if (!validSampling(sampling)) {
        setStatus(QStringLiteral("Unknown sampling mode"));
        return;
    }
    if (!isFiniteValue(rotXDeg) || !isFiniteValue(rotYDeg) || !isFiniteValue(rotZDeg)
        || !isFiniteValue(distance) || distance <= 0.0)
        return;
    const double toRad = 3.14159265358979323846 / 180.0;
    executeCommand({{QStringLiteral("type"), QStringLiteral("transform3d")},
                    {QStringLiteral("rot_x"), rotXDeg * toRad},
                    {QStringLiteral("rot_y"), rotYDeg * toRad},
                    {QStringLiteral("rot_z"), rotZDeg * toRad},
                    {QStringLiteral("distance"), distance},
                    {QStringLiteral("sampling"), sampling}});
}

void EditorBridge::encloseAndFill(qreal x, qreal y, qreal w, qreal h, const QColor &color,
                                  int alphaThreshold)
{
    QJsonObject rect;
    if (!rectObject(x, y, w, h, &rect)) {
        setStatus(QStringLiteral("Enclose-and-fill rejected: rectangle must be finite and non-empty"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("enclose_and_fill")},
                    {QStringLiteral("rect"), rect},
                    {QStringLiteral("color"), colorObject(color)},
                    {QStringLiteral("alpha_threshold"), qBound(0, alphaThreshold, 255)}});
}

void EditorBridge::smartPatch(int searchRadius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("smart_patch")},
                    {QStringLiteral("search_radius"), qBound(1, searchRadius, 256)}});
}

void EditorBridge::lazybrush(const QVariantList &scribbles)
{
    if (scribbles.size() < 6 || (scribbles.size() % 6) != 0) {
        setStatus(QStringLiteral("Lazybrush needs at least one scribble (x,y,r,g,b,a)"));
        return;
    }
    QJsonArray seeds;
    for (int i = 0; i + 5 < scribbles.size(); i += 6) {
        const double x = scribbles.at(i).toDouble();
        const double y = scribbles.at(i + 1).toDouble();
        if (!isFiniteValue(x) || !isFiniteValue(y) || x < 0.0 || y < 0.0)
            return;
        const QJsonObject colour{
            {QStringLiteral("r"), qBound(0, scribbles.at(i + 2).toInt(), 255)},
            {QStringLiteral("g"), qBound(0, scribbles.at(i + 3).toInt(), 255)},
            {QStringLiteral("b"), qBound(0, scribbles.at(i + 4).toInt(), 255)},
            {QStringLiteral("a"), qBound(0, scribbles.at(i + 5).toInt(), 255)}};
        seeds.append(QJsonArray{static_cast<int>(x), static_cast<int>(y), colour});
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("lazybrush")},
                    {QStringLiteral("scribbles"), seeds}});
}

QVariantList EditorBridge::filterCatalog() const { return m_filterCatalog; }

void EditorBridge::applyFilterParams(const QString &kind, const QVariantMap &params)
{
    // The browser's path: any filter, any parameters. Nothing is clamped here -- the engine
    // validates every field and rejects an out-of-range value with a message, which is the one
    // place the ranges are defined. `kind` from the argument wins over a stray key in params.
    QJsonObject filter = QJsonObject::fromVariantMap(params);
    filter.insert(QStringLiteral("kind"), kind);
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::addAdjustmentNode(const QString &kind, const QVariantMap &params)
{
    // P11. Same filter JSON as applyFilterParams, placed as a non-destructive node above the active
    // node instead of baked into its pixels. The engine validates the filter and the precision.
    QJsonObject filter = QJsonObject::fromVariantMap(params);
    filter.insert(QStringLiteral("kind"), kind);
    const QString parentId = m_layers.activeParentId();
    const int count = m_layers.siblingCount(parentId);
    // Directly above the active node, which is where Photoshop puts a new adjustment layer.
    const int active = m_layers.activeSiblingIndex();
    executeCommand({{QStringLiteral("type"), QStringLiteral("add_adjustment_node")},
                    {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                    {QStringLiteral("name"), QStringLiteral("Adjustment: ") + kind},
                    {QStringLiteral("parent"), parentId.isEmpty() ? QJsonValue(QJsonValue::Null)
                                                                  : QJsonValue(parentId)},
                    {QStringLiteral("sibling_index"), qBound(0, active + 1, count)},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::setAdjustmentFilter(const QString &id, const QString &kind,
                                       const QVariantMap &params)
{
    if (id.isEmpty())
        return;
    QJsonObject filter = QJsonObject::fromVariantMap(params);
    filter.insert(QStringLiteral("kind"), kind);
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_adjustment_filter")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::convertColorMode(const QString &mode, const QString &palette, int maxColors,
                                    const QString &dither)
{
    QJsonObject command{{QStringLiteral("type"), QStringLiteral("convert_color_mode")},
                        {QStringLiteral("mode"), mode},
                        {QStringLiteral("dither"), dither}};
    if (mode == QStringLiteral("indexed")) {
        QJsonObject choice{{QStringLiteral("kind"), palette.isEmpty() ? QStringLiteral("generate") : palette}};
        if (choice.value(QStringLiteral("kind")).toString() == QStringLiteral("generate"))
            choice.insert(QStringLiteral("max_colors"), qBound(2, maxColors, 256));
        command.insert(QStringLiteral("palette"), choice);
    }
    executeCommand(command);
}

void EditorBridge::setDocumentPrecision(const QString &precision)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_document_precision")},
                    {QStringLiteral("precision"), precision}});
}

void EditorBridge::applyFilter(const QString &kind)
{
    if (kind != QStringLiteral("invert") && kind != QStringLiteral("grayscale")) {
        setStatus(QStringLiteral("This filter requires its typed parameter method"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{{QStringLiteral("kind"), kind}}}});
}

void EditorBridge::applyBrightnessContrast(int brightness, qreal contrast)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("brightness_contrast")},
                         {QStringLiteral("brightness"), qBound(-255, brightness, 255)},
                         {QStringLiteral("contrast"), qBound(-100.0, contrast, 100.0)}}}});
}

void EditorBridge::applyGaussianBlur(qreal sigma)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("gaussian_blur")},
                         {QStringLiteral("sigma"), qBound(0.001, sigma, 1024.0)}}}});
}

void EditorBridge::applyThreshold(int threshold)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("threshold")},
                         {QStringLiteral("threshold"), qBound(0, threshold, 255)}}}});
}

void EditorBridge::applyPosterize(int levels)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("posterize")},
                         {QStringLiteral("levels"), qBound(2, levels, 256)}}}});
}

void EditorBridge::applyCurves(int quarter, int middle, int threeQuarter)
{
    const auto point = [](qreal x, int y) {
        return QJsonObject{{QStringLiteral("x"), x}, {QStringLiteral("y"), qBound(0, y, 255) / 255.0}};
    };
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("curves")},
                         {QStringLiteral("points"), QJsonArray{point(0.0, 0), point(0.25, quarter),
                                                               point(0.5, middle), point(0.75, threeQuarter),
                                                               point(1.0, 255)}}}}});
}

void EditorBridge::applyLevels(int inputBlack, int inputWhite, qreal gamma,
                               int outputBlack, int outputWhite)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("levels")},
                         {QStringLiteral("input_black"), qBound(0, inputBlack, 255)},
                         {QStringLiteral("input_white"), qBound(0, inputWhite, 255)},
                         {QStringLiteral("gamma"), qBound(0.01, gamma, 100.0)},
                         {QStringLiteral("output_black"), qBound(0, outputBlack, 255)},
                         {QStringLiteral("output_white"), qBound(0, outputWhite, 255)}}}});
}

void EditorBridge::applyHueSaturation(qreal hueDegrees, qreal saturation, qreal lightness)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("hue_saturation")},
                         {QStringLiteral("hue_degrees"), qBound(-180.0, hueDegrees, 180.0)},
                         {QStringLiteral("saturation"), qBound(-100.0, saturation, 100.0)},
                         {QStringLiteral("lightness"), qBound(-100.0, lightness, 100.0)}}}});
}

void EditorBridge::applyBoxBlur(int radius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("box_blur")},
                         {QStringLiteral("radius"), qBound(1, radius, 4096)}}}});
}

void EditorBridge::applySharpen(qreal amount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("sharpen")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 10.0)}}}});
}

void EditorBridge::applyMotionBlur(qreal angleDegrees, int distance)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("motion_blur")},
                         {QStringLiteral("angle_degrees"), angleDegrees},
                         {QStringLiteral("distance"), qBound(1, distance, 4096)}}}});
}

void EditorBridge::applyLensBlur(int radius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("lens_blur")},
                         {QStringLiteral("radius"), qBound(1, radius, 4096)}}}});
}

void EditorBridge::applyEdgeDetect(qreal amount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("edge_detect")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 10.0)}}}});
}

void EditorBridge::applyEmboss(qreal angleDegrees)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("emboss")},
                         {QStringLiteral("angle_degrees"), angleDegrees}}}});
}

void EditorBridge::applyLaplace()
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("laplace")}}}});
}

void EditorBridge::applyPixelize(int block)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("pixelize")},
                         {QStringLiteral("block"), qBound(1, block, 4096)}}}});
}

void EditorBridge::applyWaves(qreal amplitude, qreal wavelength)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("waves")},
                         {QStringLiteral("amplitude"), amplitude},
                         {QStringLiteral("wavelength"), wavelength}}}});
}

void EditorBridge::applyRipple(qreal amplitude, qreal wavelength, bool horizontal)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("ripple")},
                         {QStringLiteral("amplitude"), amplitude},
                         {QStringLiteral("wavelength"), wavelength},
                         {QStringLiteral("horizontal"), horizontal}}}});
}

void EditorBridge::applyWhirlPinch(qreal whirlDegrees, qreal pinch)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("whirl_pinch")},
                         {QStringLiteral("whirl_degrees"), whirlDegrees},
                         {QStringLiteral("pinch"), qBound(-1.0, pinch, 1.0)}}}});
}

void EditorBridge::applyLensDistortion(qreal mainAmount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("lens_distortion")},
                         {QStringLiteral("main_amount"), qBound(-100.0, mainAmount, 100.0)}}}});
}

void EditorBridge::applyRgbNoise(qreal amount, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("rgb_noise")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 1.0)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applyHsvNoise(qreal hue, qreal saturation, qreal value, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("hsv_noise")},
                         {QStringLiteral("hue"), qBound(0.0, hue, 1.0)},
                         {QStringLiteral("saturation"), qBound(0.0, saturation, 1.0)},
                         {QStringLiteral("value"), qBound(0.0, value, 1.0)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applyHurl(qreal amount, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("hurl")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 1.0)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applyPick(qreal amount, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("pick")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 1.0)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applySpread(int amount, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("spread")},
                         {QStringLiteral("amount"), qBound(0, amount, 4096)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applyCheckerboard(int size, const QColor &a, const QColor &b)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("checkerboard")},
                         {QStringLiteral("size"), qBound(1, size, 4096)},
                         {QStringLiteral("color_a"), colorObject(a)},
                         {QStringLiteral("color_b"), colorObject(b)}}}});
}

void EditorBridge::applyGradientMap(const QColor &low, const QColor &high)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("gradient_map")},
                         {QStringLiteral("low"), colorObject(low)},
                         {QStringLiteral("high"), colorObject(high)}}}});
}

void EditorBridge::applyPlasma(qreal turbulence, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("plasma")},
                         {QStringLiteral("turbulence"), qBound(0.1, turbulence, 10.0)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applySolidNoise(int detail, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("solid_noise")},
                         {QStringLiteral("detail"), qBound(1, detail, 8)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applyCellNoise(int density, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("cell_noise")},
                         {QStringLiteral("density"), qBound(1, density, 256)},
                         {QStringLiteral("seed"), seed}}}});
}

void EditorBridge::applyColorBalance(qreal red, qreal green, qreal blue)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("color_balance")},
                         {QStringLiteral("red"), qBound(-100.0, red, 100.0)},
                         {QStringLiteral("green"), qBound(-100.0, green, 100.0)},
                         {QStringLiteral("blue"), qBound(-100.0, blue, 100.0)}}}});
}

void EditorBridge::applyColorTemperature(qreal amount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("color_temperature")},
                         {QStringLiteral("amount"), qBound(-100.0, amount, 100.0)}}}});
}

void EditorBridge::applyExposure(qreal stops)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("exposure")},
                         {QStringLiteral("stops"), qBound(-10.0, stops, 10.0)}}}});
}

void EditorBridge::applyHueChroma(qreal hueDegrees, qreal chroma)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("hue_chroma")},
                         {QStringLiteral("hue_degrees"), hueDegrees},
                         {QStringLiteral("chroma"), qBound(-100.0, chroma, 100.0)}}}});
}

void EditorBridge::applySaturation(qreal scale)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("saturation")},
                         {QStringLiteral("scale"), qBound(0.0, scale, 4.0)}}}});
}

void EditorBridge::applyDither(int levels)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("dither")},
                         {QStringLiteral("levels"), qBound(2, levels, 256)}}}});
}

void EditorBridge::applyOilify(int radius)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("oilify")},
                         {QStringLiteral("radius"), qBound(1, radius, 32)}}}});
}

void EditorBridge::applyCartoon(qreal amount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("cartoon")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 10.0)}}}});
}

void EditorBridge::applySoftGlow(int radius, qreal amount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("soft_glow")},
                         {QStringLiteral("radius"), qBound(1, radius, 256)},
                         {QStringLiteral("amount"), qBound(0.0, amount, 1.0)}}}});
}

void EditorBridge::applyPhotocopy(qreal amount)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("photocopy")},
                         {QStringLiteral("amount"), qBound(0.0, amount, 10.0)}}}});
}

void EditorBridge::applyApplyCanvas(qreal depth)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("apply_canvas")},
                         {QStringLiteral("depth"), qBound(0.0, depth, 1.0)}}}});
}

void EditorBridge::applyCubism(int tile, int seed)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("cubism")},
                         {QStringLiteral("tile"), qBound(1, tile, 256)},
                         {QStringLiteral("seed"), seed}}}});
}

// The map filters' optional map layer. Written as a helper rather than inline four times so the
// "empty means self-map" rule lives in ONE place -- sending an empty string through as a layer id
// would make the core refuse a command the user did not get wrong.
static void addMapLayer(QJsonObject &filter, const QString &mapLayerId)
{
    if (!mapLayerId.isEmpty())
        filter.insert(QStringLiteral("map"), mapLayerId);
}

void EditorBridge::applyBumpMap(qreal azimuthDegrees, qreal elevationDegrees, qreal depth,
                                const QString &mapLayerId)
{
    QJsonObject filter{
        {QStringLiteral("kind"), QStringLiteral("bump_map")},
        {QStringLiteral("azimuth_degrees"), azimuthDegrees},
        {QStringLiteral("elevation_degrees"), elevationDegrees},
        {QStringLiteral("depth"), qBound(0.0, depth, 100.0)}};
    addMapLayer(filter, mapLayerId);
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::applyDisplace(qreal amount, const QString &mapLayerId)
{
    QJsonObject filter{
        {QStringLiteral("kind"), QStringLiteral("displace")},
        {QStringLiteral("amount"), amount}};
    addMapLayer(filter, mapLayerId);
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::applyFractalTrace(int depth, qreal scale, const QString &mapLayerId)
{
    QJsonObject filter{
        {QStringLiteral("kind"), QStringLiteral("fractal_trace")},
        {QStringLiteral("depth"), qBound(1, depth, 32)},
        {QStringLiteral("scale"), scale}};
    addMapLayer(filter, mapLayerId);
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::applyWarpMap(qreal amount, int steps, const QString &mapLayerId)
{
    QJsonObject filter{
        {QStringLiteral("kind"), QStringLiteral("warp_map")},
        {QStringLiteral("amount"), amount},
        {QStringLiteral("steps"), qBound(1, steps, 32)}};
    addMapLayer(filter, mapLayerId);
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), filter}});
}

void EditorBridge::applyHalftone(int cell)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("halftone")},
                         {QStringLiteral("cell"), qBound(2, cell, 256)}}}});
}

void EditorBridge::applyPhongBump(qreal azimuthDegrees, qreal elevationDegrees, qreal depth,
                                  qreal shininess)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("phong_bump")},
                         {QStringLiteral("azimuth_degrees"), azimuthDegrees},
                         {QStringLiteral("elevation_degrees"), elevationDegrees},
                         {QStringLiteral("depth"), qBound(0.0, depth, 100.0)},
                         {QStringLiteral("shininess"), qBound(1.0, shininess, 128.0)}}}});
}

void EditorBridge::applyPalettize(int levels)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("palettize")},
                         {QStringLiteral("levels"), qBound(2, levels, 256)}}}});
}

void EditorBridge::applyNormalMap(qreal strength)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("normal_map")},
                         {QStringLiteral("strength"), strength}}}});
}

void EditorBridge::applyChannelMixer(const QVariantList &matrix, const QVariantList &offset)
{
    if (matrix.size() != 9 || offset.size() != 3) {
        setStatus(QStringLiteral("Channel mixer needs a 3x3 matrix and 3 offsets"));
        return;
    }
    QJsonArray m;
    for (const QVariant &v : matrix) {
        if (!isFiniteValue(v.toDouble()))
            return;
        m.append(v.toDouble());
    }
    QJsonArray o;
    for (const QVariant &v : offset) {
        if (!isFiniteValue(v.toDouble()))
            return;
        o.append(v.toDouble());
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("channel_mixer")},
                         {QStringLiteral("matrix"), m},
                         {QStringLiteral("offset"), o}}}});
}

void EditorBridge::applyLabAdjust(qreal lightness, qreal chroma)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_filter")},
                    {QStringLiteral("filter"), QJsonObject{
                         {QStringLiteral("kind"), QStringLiteral("lab_adjust")},
                         {QStringLiteral("lightness"), qBound(-100.0, lightness, 100.0)},
                         {QStringLiteral("chroma"), qBound(0.0, chroma, 4.0)}}}});
}

void EditorBridge::applyOpGraph(const QString &nodesJson)
{
    QJsonParseError error;
    const QJsonDocument parsed = QJsonDocument::fromJson(nodesJson.toUtf8(), &error);
    if (error.error != QJsonParseError::NoError || !parsed.isArray()) {
        setStatus(QStringLiteral("Operation graph must be a JSON array of nodes"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_graph")},
                    {QStringLiteral("graph"), QJsonObject{{QStringLiteral("nodes"), parsed.array()}}}});
}

void EditorBridge::applyLayerStyle(const QString &styleJson)
{
    QJsonParseError error;
    const QJsonDocument parsed = QJsonDocument::fromJson(styleJson.toUtf8(), &error);
    if (error.error != QJsonParseError::NoError || !parsed.isObject()) {
        setStatus(QStringLiteral("Layer style must be a JSON object"));
        return;
    }
    executeCommand({{QStringLiteral("type"), QStringLiteral("apply_layer_style")},
                    {QStringLiteral("style"), parsed.object()}});
}

QVariantMap EditorBridge::histogram() const
{
    QVector<quint32> r(256, 0), g(256, 0), b(256, 0), luma(256, 0);
    if (!m_renderImage.isNull()) {
        const QImage img = m_renderImage.convertToFormat(QImage::Format_RGBA8888);
        for (int y = 0; y < img.height(); ++y) {
            const uchar *line = img.constScanLine(y);
            for (int x = 0; x < img.width(); ++x) {
                const uchar cr = line[x * 4];
                const uchar cg = line[x * 4 + 1];
                const uchar cb = line[x * 4 + 2];
                r[cr]++;
                g[cg]++;
                b[cb]++;
                const int l = qBound(0, static_cast<int>(0.299 * cr + 0.587 * cg + 0.114 * cb), 255);
                luma[l]++;
            }
        }
    }
    const auto pack = [](const QVector<quint32> &bins) {
        QVariantList out;
        out.reserve(256);
        for (quint32 v : bins)
            out.append(static_cast<double>(v));
        return out;
    };
    QVariantMap result;
    result.insert(QStringLiteral("r"), pack(r));
    result.insert(QStringLiteral("g"), pack(g));
    result.insert(QStringLiteral("b"), pack(b));
    result.insert(QStringLiteral("luma"), pack(luma));
    return result;
}

void EditorBridge::scheduleProjectionRefresh(bool captureSelection)
{
    m_projectionStale = true;
    m_retryNeedsSelection = m_retryNeedsSelection || captureSelection || m_selectionMask.isNull();
    if (!m_refreshRetryTimer.isActive())
        m_refreshRetryTimer.start();
}

bool EditorBridge::refresh(bool captureSelection)
{
    // During a filter run the worker holds the engine; a refresh here would block the GUI thread
    // on its mutex. The retry timer and finishFilterRun refresh once it is done.
    if (!m_editor || m_filterBusy)
        return false;

    const bool needsSelection = captureSelection || m_selectionMask.isNull();
    for (int attempt = 0; attempt < kSnapshotAttempts; ++attempt) {
        RedrobBuffer stateBuffer{};
        if (redrob_editor_state_json(m_editor.get(), &stateBuffer) != REDROB_OK) {
            redrob_buffer_free(stateBuffer);
            setStatus(QStringLiteral("Snapshot failed: %1").arg(ffiError()));
            return false;
        }
        QJsonParseError stateError;
        const QJsonDocument stateJson = QJsonDocument::fromJson(takeBuffer(stateBuffer), &stateError);
        if (stateError.error != QJsonParseError::NoError || !stateJson.isObject()) {
            setStatus(QStringLiteral("Snapshot failed: malformed coherent state JSON"));
            return false;
        }
        const QJsonObject state = stateJson.object();
        const QJsonObject document = state.value(QStringLiteral("document")).toObject();
        const QJsonObject layers = state.value(QStringLiteral("layers")).toObject();
        const QJsonObject timeline = state.value(QStringLiteral("timeline")).toObject();
        const int width = document.value(QStringLiteral("width")).toInt();
        const int height = document.value(QStringLiteral("height")).toInt();
        const qulonglong generation = state.value(QStringLiteral("generation")).toVariant().toULongLong();
        const qulonglong documentGeneration = document.value(QStringLiteral("generation")).toVariant().toULongLong();
        const qulonglong layerGeneration = layers.value(QStringLiteral("generation")).toVariant().toULongLong();
        const qulonglong timelineGeneration = timeline.value(QStringLiteral("generation")).toVariant().toULongLong();
        FrameModel candidateFrames;
        const qreal fps = timeline.value(QStringLiteral("fps")).toDouble();
        bool rangeStartOk = false;
        bool rangeEndOk = false;
        const qulonglong rangeStart = timeline.value(QStringLiteral("range_start")).toVariant().toULongLong(&rangeStartOk);
        const qulonglong rangeEnd = timeline.value(QStringLiteral("range_end")).toVariant().toULongLong(&rangeEndOk);
        if (width <= 0 || height <= 0 || !candidateFrames.replaceFromSnapshot(timeline)
            || !isFiniteValue(fps) || fps <= 0.0 || fps > 240.0 || !rangeStartOk || !rangeEndOk
            || rangeStart > std::numeric_limits<quint32>::max()
            || rangeEnd > std::numeric_limits<quint32>::max()
            || !timeline.value(QStringLiteral("looping")).isBool()
            || !timeline.value(QStringLiteral("playing")).isBool()) {
            setStatus(QStringLiteral("Snapshot failed: invalid coherent state metadata"));
            return false;
        }

        RedrobRenderSnapshot render{};
        // Onion skin (H.2): a different picture, so a different symbol. The ghosted composite is not
        // cached in the core's projection, which is why it is only asked for while the animator has it
        // switched on.
        const int32_t renderStatus = m_onionSkinEnabled
            ? redrob_editor_render_onion_skin_rgba(
                  m_editor.get(), static_cast<uint32_t>(m_onionSkinBefore),
                  static_cast<uint32_t>(m_onionSkinAfter), kOnionTintBefore, kOnionTintAfter,
                  static_cast<float>(m_onionSkinOpacity), &render)
            : redrob_editor_render_rgba(m_editor.get(), &render);
        if (renderStatus != REDROB_OK) {
            redrob_buffer_free(render.rgba);
            setStatus(QStringLiteral("Render failed: %1").arg(ffiError()));
            return false;
        }
        RedrobSelectionMaskSnapshot selection{};
        if (needsSelection
            && redrob_editor_selection_mask(m_editor.get(), &selection) != REDROB_OK) {
            redrob_buffer_free(render.rgba);
            redrob_buffer_free(selection.mask);
            setStatus(QStringLiteral("Selection snapshot failed: %1").arg(ffiError()));
            return false;
        }

        const quint64 renderLength = quint64(render.stride) * quint64(render.height);
        const quint64 selectionLength = needsSelection
            ? quint64(selection.stride) * quint64(selection.height)
            : 0;
        const bool dimensionsValid = render.width == static_cast<uint32_t>(width)
            && render.height == static_cast<uint32_t>(height)
            && (needsSelection
                    ? selection.width == render.width && selection.height == render.height
                    : m_selectionMask.width() == width && m_selectionMask.height() == height);
        const bool renderLayoutValid = render.stride == render.width * 4u
            && renderLength == render.rgba.len
            && renderLength <= static_cast<quint64>(std::numeric_limits<qsizetype>::max())
            && render.rgba.data != nullptr;
        const bool selectionLayoutValid = !needsSelection
            || (selection.stride == selection.width && selectionLength == selection.mask.len
                && selectionLength <= static_cast<quint64>(std::numeric_limits<qsizetype>::max())
                && selection.mask.data != nullptr);
        const bool generationsValid = generation == documentGeneration
            && generation == layerGeneration && generation == timelineGeneration
            && generation == render.generation
            && (!needsSelection || generation == selection.generation);

        QImage renderCopy;
        QImage selectionCopy;
        if (dimensionsValid && renderLayoutValid && selectionLayoutValid) {
            const QImage borrowedRender(render.rgba.data, width, height,
                                        static_cast<qsizetype>(render.stride), QImage::Format_RGBA8888);
            renderCopy = borrowedRender.copy();
            if (needsSelection) {
                const QImage borrowedSelection(selection.mask.data, width, height,
                                               static_cast<qsizetype>(selection.stride),
                                               QImage::Format_Grayscale8);
                selectionCopy = borrowedSelection.copy();
            }
        }
        redrob_buffer_free(render.rgba);
        redrob_buffer_free(selection.mask);

        if (!dimensionsValid || !renderLayoutValid || !selectionLayoutValid) {
            setStatus(QStringLiteral("Snapshot failed strict dimensions/stride/length validation"));
            return false;
        }
        if (!generationsValid)
            continue;

        renderCopy.setDevicePixelRatio(1.0);
        if (needsSelection)
            selectionCopy.setDevicePixelRatio(1.0);
        const bool dimensionsChanged = width != m_width || height != m_height;
        const bool renderChanged = dimensionsChanged || renderCopy != m_renderImage;
        const bool selectionStateChanged = needsSelection
            && (dimensionsChanged || (selection.active != 0) != m_selectionActive
                || selectionCopy != m_selectionMask);
        const bool timelineStateChanged = candidateFrames.currentFrameId() != m_frames.currentFrameId()
            || candidateFrames.currentIndex() != m_frames.currentIndex()
            || candidateFrames.rowCount() != m_frames.rowCount() || !qFuzzyCompare(fps, m_fps)
            || static_cast<quint32>(rangeStart) != m_rangeStart
            || static_cast<quint32>(rangeEnd) != m_rangeEnd
            || timeline.value(QStringLiteral("looping")).toBool() != m_looping
            || timeline.value(QStringLiteral("playing")).toBool() != m_playing;
        m_width = width;
        m_height = height;
        m_generation = generation;
        m_canUndo = document.value(QStringLiteral("can_undo")).toBool();
        m_canRedo = document.value(QStringLiteral("can_redo")).toBool();
        m_undoDepth = document.value(QStringLiteral("undo_depth")).toInt();
        m_colorMode = document.value(QStringLiteral("color_mode")).toString();
        m_precision = document.value(QStringLiteral("precision")).toString();
        m_redoDepth = document.value(QStringLiteral("redo_depth")).toInt();
        m_historyLabels.clear();
        for (const QString key : {QStringLiteral("undo_labels"), QStringLiteral("redo_labels")}) {
            for (const QJsonValue &label : document.value(key).toArray())
                m_historyLabels.append(label.isString() ? label.toString() : QString());
        }
        // Kept in step with each other: a handle list of a different length than the anchor list
        // would pair a handle with the wrong anchor, so a mismatch drops both rather than drawing
        // the overlay somewhere the path is not.
        const QJsonArray anchors = document.value(QStringLiteral("active_vector_anchors")).toArray();
        const QJsonArray handles = document.value(QStringLiteral("active_vector_handles")).toArray();
        m_activeVectorAnchors.clear();
        m_activeVectorHandles.clear();
        if (anchors.size() == handles.size()) {
            for (const QJsonValue &value : anchors)
                m_activeVectorAnchors.append(value.toDouble());
            for (const QJsonValue &value : handles)
                m_activeVectorHandles.append(value.toDouble());
        }
        m_layers.replaceFromSnapshot(layers);
        if (!m_frames.replaceFromSnapshot(timeline)) {
            setStatus(QStringLiteral("Snapshot failed: malformed frame model"));
            return false;
        }
        m_fps = fps;
        m_rangeStart = static_cast<quint32>(rangeStart);
        m_rangeEnd = static_cast<quint32>(rangeEnd);
        m_looping = timeline.value(QStringLiteral("looping")).toBool();
        m_playing = timeline.value(QStringLiteral("playing")).toBool();
        if (m_playing) {
            m_playbackTimer.setInterval(qMax(1, qRound(1000.0 / m_fps)));
            if (!m_playbackTimer.isActive())
                m_playbackTimer.start();
        } else {
            m_playbackTimer.stop();
        }
        if (renderChanged)
            m_renderImage = std::move(renderCopy);
        if (selectionStateChanged) {
            m_selectionMask = std::move(selectionCopy);
            m_selectionActive = selection.active != 0;
        }
        if (dimensionsChanged) {
            m_mirrorXAxis = width / 2.0;
            m_mirrorYAxis = height / 2.0;
            m_brushSymmetryCenterX = width / 2.0;
            m_brushSymmetryCenterY = height / 2.0;
            emit brushSettingsChanged();
        }
        m_projectionStale = false;
        m_retryNeedsSelection = false;
        m_refreshRetryTimer.stop();
        emit documentChanged();
        if (timelineStateChanged)
            emit timelineChanged();
        if (renderChanged)
            emit renderImageChanged();
        if (selectionStateChanged)
            emit selectionChanged();
        return true;
    }

    setStatus(QStringLiteral("Snapshot refresh deferred: document changed during capture"));
    return false;
}

bool EditorBridge::replaceFromGenericBytes(const QByteArray &bytes,
                                               const QString &expectedFormat,
                                               const QString &sourceName,
                                               bool projectIdentity,
                                               const QString &projectPath)
{
    if (refuseWhileFilterRuns(QStringLiteral("Open")))
        return false;
    if (!m_editor) {
        setStatus(QStringLiteral("Open failed: editor is unavailable"));
        return false;
    }
    const QJsonObject options{
        {QStringLiteral("schema_version"), 1},
        {QStringLiteral("expected_format"), expectedFormat},
        {QStringLiteral("loss_policy"),
         projectIdentity ? QStringLiteral("reject_loss") : QStringLiteral("allow_loss")},
        {QStringLiteral("max_input_bytes"), kMaxFormatInputBytes},
    };
    const QByteArray optionsJson = canonicalJson(options);
    RedrobBuffer result{};
    const int status = redrob_editor_import_file(
        m_editor.get(), reinterpret_cast<const uint8_t *>(bytes.constData()),
        static_cast<size_t>(bytes.size()),
        reinterpret_cast<const uint8_t *>(optionsJson.constData()),
        static_cast<size_t>(optionsJson.size()), &result);
    if (status != REDROB_OK) {
        redrob_buffer_free(result);
        setStatus(QStringLiteral("Open failed: %1").arg(ffiError()));
        return false;
    }
    QJsonParseError parseError;
    const QJsonDocument outcomeDocument = QJsonDocument::fromJson(takeBuffer(result), &parseError);
    if (parseError.error != QJsonParseError::NoError || !outcomeDocument.isObject()) {
        setStatus(QStringLiteral("Open failed: invalid format result from core"));
        return false;
    }
    ++m_documentEpoch;
    m_lastMutationProjectionRefreshed = refresh(true);
    if (!m_lastMutationProjectionRefreshed)
        scheduleProjectionRefresh(true);

    const QString nextProject = projectIdentity ? projectPath : QString{};
    if (m_currentFile != nextProject) {
        m_currentFile = nextProject;
        emit currentFileChanged();
    }
    const QString verb = projectIdentity ? QStringLiteral("Opened project")
                                         : QStringLiteral("Imported");
    QString statusMessage = formatOutcomeStatus(verb, sourceName, outcomeDocument.object());
    if (!m_lastMutationProjectionRefreshed)
        statusMessage += QStringLiteral(" · display synchronization is retrying");
    setStatus(statusMessage);
    return true;
}

bool EditorBridge::openProject(const QUrl &url)
{
    if (!url.isLocalFile()) {
        setStatus(QStringLiteral("Open project failed: choose a local .rrg file"));
        return false;
    }
    const QString path = url.toLocalFile();
    const QFileInfo info(path);
    if (info.suffix().compare(QStringLiteral("rrg"), Qt::CaseInsensitive) != 0) {
        setStatus(QStringLiteral("Open project failed: project files must use .rrg"));
        return false;
    }
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        setStatus(QStringLiteral("Could not read %1: %2").arg(info.fileName(), file.errorString()));
        return false;
    }
    QString readError;
    const auto bytes = readBoundedFormatFile(file, &readError);
    if (!bytes) {
        setStatus(QStringLiteral("Open project failed: %1").arg(readError));
        return false;
    }
    return replaceFromGenericBytes(*bytes, QStringLiteral("rrg"), info.fileName(), true, path);
}

bool EditorBridge::importFile(const QUrl &url)
{
    if (!url.isLocalFile()) {
        setStatus(QStringLiteral("Import failed: choose a local interchange file"));
        return false;
    }
    const QString path = url.toLocalFile();
    const QFileInfo info(path);
    const QString format = canonicalFormatForSuffix(info.suffix());
    if (format.isEmpty() || format == QStringLiteral("rrg")) {
        setStatus(QStringLiteral("Import failed: expected png, jpg, webp, tiff, ora, svg, psd, kra, or xcf"));
        return false;
    }
    QFile file(path);
    if (!file.open(QIODevice::ReadOnly)) {
        setStatus(QStringLiteral("Could not read %1: %2").arg(info.fileName(), file.errorString()));
        return false;
    }
    QString readError;
    const auto bytes = readBoundedFormatFile(file, &readError);
    if (!bytes) {
        setStatus(QStringLiteral("Import failed: %1").arg(readError));
        return false;
    }
    return replaceFromGenericBytes(*bytes, format, info.fileName(), false);
}

bool EditorBridge::exportGenericBytes(const QString &format, bool allowLoss,
                                      std::optional<quint32> frameId, int jpegQuality,
                                      const QColor &matte, QByteArray *bytes,
                                      QJsonObject *result)
{
    if (refuseWhileFilterRuns(QStringLiteral("Export")))
        return false;
    if (!m_editor || !bytes || !result)
        return false;
    if (jpegQuality < 1 || jpegQuality > 100) {
        setStatus(QStringLiteral("Export failed: JPEG quality must be between 1 and 100"));
        return false;
    }
    if (format == QStringLiteral("jpeg") && (!matte.isValid() || matte.alpha() != 255)) {
        setStatus(QStringLiteral("Export failed: JPEG matte must be an opaque color"));
        return false;
    }
    QJsonObject options{
        {QStringLiteral("schema_version"), 1},
        {QStringLiteral("format"), format},
        {QStringLiteral("loss_policy"),
         allowLoss ? QStringLiteral("allow_loss") : QStringLiteral("reject_loss")},
        {QStringLiteral("jpeg_quality"), jpegQuality},
        {QStringLiteral("jpeg_alpha"),
         format == QStringLiteral("jpeg")
             ? QJsonValue(QJsonObject{{QStringLiteral("policy"), QStringLiteral("flatten")},
                                      {QStringLiteral("matte"), colorObject(matte)}})
             : QJsonValue(QJsonObject{{QStringLiteral("policy"),
                                       QStringLiteral("reject_non_opaque")}})},
    };
    if (frameId)
        options.insert(QStringLiteral("frame"), static_cast<qint64>(*frameId));
    const QByteArray optionsJson = canonicalJson(options);
    RedrobBuffer output{};
    RedrobBuffer outcome{};
    const int status = redrob_editor_export_file(
        m_editor.get(), reinterpret_cast<const uint8_t *>(optionsJson.constData()),
        static_cast<size_t>(optionsJson.size()), &output, &outcome);
    if (status != REDROB_OK) {
        redrob_buffer_free(output);
        redrob_buffer_free(outcome);
        setStatus(QStringLiteral("Export failed: %1").arg(ffiError()));
        return false;
    }
    const QByteArray encoded = takeBuffer(output);
    QJsonParseError parseError;
    const QJsonDocument outcomeDocument = QJsonDocument::fromJson(takeBuffer(outcome), &parseError);
    if (parseError.error != QJsonParseError::NoError || !outcomeDocument.isObject()) {
        setStatus(QStringLiteral("Export failed: invalid format result from core"));
        return false;
    }
    *bytes = encoded;
    *result = outcomeDocument.object();
    return true;
}

bool EditorBridge::writeAtomically(const QString &path, const QByteArray &bytes)
{
    QSaveFile file(path);
    if (!file.open(QIODevice::WriteOnly)) {
        setStatus(QStringLiteral("Could not write %1: %2")
                      .arg(QFileInfo(path).fileName(), file.errorString()));
        return false;
    }
    if (file.write(bytes) != bytes.size()) {
        file.cancelWriting();
        setStatus(QStringLiteral("Could not write %1: %2")
                      .arg(QFileInfo(path).fileName(), file.errorString()));
        return false;
    }
    if (!file.commit()) {
        setStatus(QStringLiteral("Could not commit %1: %2")
                      .arg(QFileInfo(path).fileName(), file.errorString()));
        return false;
    }
    return true;
}

bool EditorBridge::saveProject(const QUrl &url)
{
    QString path;
    if (url.isEmpty())
        path = m_currentFile;
    else if (url.isLocalFile())
        path = url.toLocalFile();
    else {
        setStatus(QStringLiteral("Save project failed: choose a local .rrg destination"));
        return false;
    }
    if (path.isEmpty()) {
        setStatus(QStringLiteral("Choose an RRG project destination first"));
        return false;
    }
    const QFileInfo info(path);
    if (info.suffix().compare(QStringLiteral("rrg"), Qt::CaseInsensitive) != 0) {
        setStatus(QStringLiteral("Save project failed: project files must use .rrg"));
        return false;
    }
    QByteArray bytes;
    QJsonObject result;
    if (!exportGenericBytes(QStringLiteral("rrg"), false, std::nullopt, 90, QColor(Qt::white),
                            &bytes, &result)
        || !writeAtomically(path, bytes))
        return false;
    if (m_currentFile != path) {
        m_currentFile = path;
        emit currentFileChanged();
    }
    setStatus(formatOutcomeStatus(QStringLiteral("Saved project"), info.fileName(), result));
    return true;
}

bool EditorBridge::exportFile(const QUrl &url, const QString &format, bool allowLoss,
                              quint32 frameId, int jpegQuality, const QColor &matte)
{
    if (!url.isLocalFile()) {
        setStatus(QStringLiteral("Export failed: choose a local destination"));
        return false;
    }
    QString normalizedFormat = format.trimmed().toLower();
    if (normalizedFormat == QStringLiteral("jpg"))
        normalizedFormat = QStringLiteral("jpeg");
    static const QStringList formats{QStringLiteral("png"), QStringLiteral("jpeg"),
                                     QStringLiteral("webp"), QStringLiteral("ora"),
                                     QStringLiteral("svg")};
    if (!formats.contains(normalizedFormat)) {
        setStatus(QStringLiteral("Export failed: unsupported format"));
        return false;
    }
    const QString path = url.toLocalFile();
    const QFileInfo info(path);
    if (!suffixMatchesFormat(info.suffix(), normalizedFormat)) {
        setStatus(QStringLiteral("Export failed: destination extension does not match %1")
                      .arg(normalizedFormat.toUpper()));
        return false;
    }
    QByteArray bytes;
    QJsonObject result;
    if (!exportGenericBytes(normalizedFormat, allowLoss, frameId, jpegQuality, matte, &bytes,
                            &result)
        || !writeAtomically(path, bytes))
        return false;
    setStatus(formatOutcomeStatus(QStringLiteral("Exported"), info.fileName(), result));
    return true;
}

bool EditorBridge::openFile(const QUrl &url)
{
    if (!url.isLocalFile()) {
        setStatus(QStringLiteral("Open failed: choose a local supported file"));
        return false;
    }
    const QString format = canonicalFormatForSuffix(QFileInfo(url.toLocalFile()).suffix());
    if (format == QStringLiteral("rrg"))
        return openProject(url);
    if (!format.isEmpty())
        return importFile(url);
    setStatus(QStringLiteral("Open failed: unknown file extension"));
    return false;
}

bool EditorBridge::saveFile(const QUrl &url)
{
    if (url.isEmpty())
        return saveProject();
    if (!url.isLocalFile()) {
        setStatus(QStringLiteral("Save failed: choose a local destination"));
        return false;
    }
    const QString format = canonicalFormatForSuffix(QFileInfo(url.toLocalFile()).suffix());
    if (format == QStringLiteral("rrg"))
        return saveProject(url);
    if (format == QStringLiteral("png"))
        return exportFile(url, QStringLiteral("png"), true, currentFrame(), 90,
                          QColor(Qt::white));
    setStatus(QStringLiteral("Save failed: use Export for this format"));
    return false;
}

QString EditorBridge::enqueueProposal(const QString &title, const QString &summary,
                                      const QString &commandJson)
{
    if (m_projectionStale) {
        setStatus(QStringLiteral("Proposal creation deferred until the committed document is visible"));
        return {};
    }
    QJsonParseError error;
    const QJsonDocument parsed = QJsonDocument::fromJson(commandJson.toUtf8(), &error);
    if (error.error != QJsonParseError::NoError || !parsed.isObject()) {
        setStatus(QStringLiteral("Proposal rejected: command JSON must be one object"));
        return {};
    }
    const QString canonical = QString::fromUtf8(canonicalJson(parsed.object()));
    const QString id = m_proposals.enqueue(title, summary, QStringLiteral("command"), canonical,
                                           m_generation, m_documentEpoch);
    if (id.isEmpty()) {
        setStatus(QStringLiteral("Proposal queue is full"));
        return {};
    }
    setStatus(QStringLiteral("Proposal queued — review before applying"));
    return id;
}

bool EditorBridge::proposeLocally(const QString &prompt)
{
    const QString normalized = prompt.trimmed().toLower();
    QJsonObject command;
    QString title;
    QString summary;
    QString actionType = QStringLiteral("command");

    if (normalized == QStringLiteral("undo") || normalized.contains(QStringLiteral("undo last"))) {
        title = QStringLiteral("Undo latest edit");
        summary = QStringLiteral("Undo the latest committed edit after approval.");
        actionType = QStringLiteral("undo");
    } else if (normalized == QStringLiteral("redo") || normalized.contains(QStringLiteral("redo last"))) {
        title = QStringLiteral("Redo latest edit");
        summary = QStringLiteral("Redo the latest undone edit after approval.");
        actionType = QStringLiteral("redo");
    } else if (normalized.contains(QStringLiteral("add layer"))) {
        title = QStringLiteral("Add a layer");
        summary = QStringLiteral("Create and activate a transparent layer named Agent layer.");
        command = {{QStringLiteral("type"), QStringLiteral("add_layer")},
                   {QStringLiteral("id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
                   {QStringLiteral("name"), QStringLiteral("Agent layer")},
                   {QStringLiteral("index"), m_layers.layerCount()}};
    } else if (normalized.contains(QStringLiteral("select"))) {
        title = QStringLiteral("Select the canvas center");
        summary = QStringLiteral("Replace the selection with a centered rectangle.");
        QJsonObject rect;
        if (!rectObject(m_width * 0.25, m_height * 0.25,
                        m_width * 0.5, m_height * 0.5, &rect)) {
            setStatus(QStringLiteral("Local selection proposal could not be constructed safely"));
            return false;
        }
        command = {{QStringLiteral("type"), QStringLiteral("select_rectangle")},
                   {QStringLiteral("rect"), rect},
                   {QStringLiteral("mode"), QStringLiteral("replace")}};
    } else if (normalized.contains(QStringLiteral("gradient"))) {
        title = QStringLiteral("Apply a two-stop gradient");
        summary = QStringLiteral("Fill with a dark-to-blue linear gradient.");
        command = {{QStringLiteral("type"), QStringLiteral("gradient_fill")},
                   {QStringLiteral("kind"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("linear")},
                                                         {QStringLiteral("start_x"), 0.0},
                                                         {QStringLiteral("start_y"), 0.0},
                                                         {QStringLiteral("end_x"), double(m_width)},
                                                         {QStringLiteral("end_y"), double(m_height)}}},
                   {QStringLiteral("stops"), QJsonArray{
                        QJsonObject{{QStringLiteral("position"), 0.0},
                                    {QStringLiteral("color"), colorObject(QColor(QStringLiteral("#161b24")))}},
                        QJsonObject{{QStringLiteral("position"), 1.0},
                                    {QStringLiteral("color"), colorObject(QColor(QStringLiteral("#5378dc")))}}}}};
    } else if (normalized.contains(QStringLiteral("threshold"))) {
        title = QStringLiteral("Threshold active layer");
        summary = QStringLiteral("Apply threshold 128 to the active layer.");
        command = {{QStringLiteral("type"), QStringLiteral("apply_filter")},
                   {QStringLiteral("filter"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("threshold")},
                                                           {QStringLiteral("threshold"), 128}}}};
    } else if (normalized.contains(QStringLiteral("blur"))) {
        title = QStringLiteral("Blur active layer");
        summary = QStringLiteral("Apply a Gaussian blur with sigma 4.");
        command = {{QStringLiteral("type"), QStringLiteral("apply_filter")},
                   {QStringLiteral("filter"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("gaussian_blur")},
                                                           {QStringLiteral("sigma"), 4.0}}}};
    } else if (normalized.contains(QStringLiteral("flip"))) {
        title = QStringLiteral("Flip active layer horizontally");
        summary = QStringLiteral("Mirror the active layer left-to-right.");
        command = {{QStringLiteral("type"), QStringLiteral("flip_active")},
                   {QStringLiteral("horizontal"), true}, {QStringLiteral("vertical"), false}};
    } else if (normalized.contains(QStringLiteral("rotate"))) {
        title = QStringLiteral("Rotate active layer clockwise");
        summary = QStringLiteral("Rotate the active layer by 90 degrees.");
        command = {{QStringLiteral("type"), QStringLiteral("rotate_active90")},
                   {QStringLiteral("clockwise"), true}};
    } else if (normalized.contains(QStringLiteral("clear"))) {
        title = QStringLiteral("Clear active layer");
        summary = QStringLiteral("Clear pixels on the active layer, respecting selection.");
        command = {{QStringLiteral("type"), QStringLiteral("clear")}};
    } else if (normalized.contains(QStringLiteral("grayscale")) || normalized.contains(QStringLiteral("greyscale"))) {
        title = QStringLiteral("Convert active layer to grayscale");
        summary = QStringLiteral("Apply the deterministic grayscale filter to the active layer.");
        command = {{QStringLiteral("type"), QStringLiteral("apply_filter")},
                   {QStringLiteral("filter"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("grayscale")}}}};
    } else if (normalized.contains(QStringLiteral("invert"))) {
        title = QStringLiteral("Invert active layer");
        summary = QStringLiteral("Invert RGB channels on the active layer while preserving alpha.");
        command = {{QStringLiteral("type"), QStringLiteral("apply_filter")},
                   {QStringLiteral("filter"), QJsonObject{{QStringLiteral("kind"), QStringLiteral("invert")}}}};
    } else if (normalized.contains(QStringLiteral("fill"))) {
        QColor color(QStringLiteral("#20262e"));
        const auto match = QRegularExpression(QStringLiteral("#[0-9a-f]{6,8}")).match(normalized);
        if (match.hasMatch())
            color = QColor(match.captured());
        title = QStringLiteral("Fill active layer");
        summary = QStringLiteral("Fill the active layer with %1.").arg(color.name(QColor::HexArgb));
        command = {{QStringLiteral("type"), QStringLiteral("fill")},
                   {QStringLiteral("color"), colorObject(color)}};
    } else {
        setAgentStatus(QStringLiteral("Local deterministic proposal mode · explicitly no network"));
        setAssistantText(QStringLiteral("No network request was made. Supported local proposals: add layer, fill, selection, gradient, grayscale, invert, threshold, blur, flip, rotate, clear, undo, and redo."));
        setStatus(QStringLiteral("Local proposal parser did not recognize that request"));
        return false;
    }

    const QString commandJson = command.isEmpty() ? QString{} : QString::fromUtf8(canonicalJson(command));
    const QString id = m_proposals.enqueue(title, summary, actionType, commandJson, m_generation,
                                           m_documentEpoch);
    if (id.isEmpty()) {
        setStatus(QStringLiteral("Proposal queue is full"));
        return false;
    }
    setAgentStatus(QStringLiteral("Local deterministic proposal mode · explicitly no network"));
    setAssistantText(QStringLiteral("A deterministic local proposal was created. No service, model, or network endpoint was contacted."));
    setStatus(QStringLiteral("Local proposal queued — review before applying"));
    return true;
}

bool EditorBridge::proposePrompt(const QString &prompt)
{
    if (m_projectionStale) {
        setStatus(QStringLiteral("Proposal request deferred until the committed document is visible"));
        return false;
    }
    if (m_agentBusy) {
        setStatus(QStringLiteral("A Redrob request is already in progress"));
        return false;
    }
    const QString trimmed = prompt.trimmed();
    if (trimmed.isEmpty()) {
        setStatus(QStringLiteral("Enter a proposal request first"));
        return false;
    }
    if (!m_liveAgentConfigured)
        return proposeLocally(trimmed);
    if (!m_editor) {
        setStatus(QStringLiteral("Editor is unavailable"));
        return false;
    }

    setAssistantText(QString{});
    setAgentBusy(true);
    setAgentStatus(QStringLiteral("Live Redrob · contacting model auto…"));
    setStatus(QStringLiteral("Requesting reviewable proposals…"));
    const QByteArray promptBytes = trimmed.toUtf8();
    auto future = QtConcurrent::run([editor = m_editor, apiKey = m_apiKey, promptBytes,
                                     documentEpoch = m_documentEpoch] {
        return requestAgentProposals(editor.get(), apiKey, promptBytes, documentEpoch);
    });
    m_agentWatcher.setFuture(future);
    return true;
}

void EditorBridge::finishAgentRequest(const AgentResult &result)
{
    setAgentBusy(false);
    if (m_projectionStale) {
        setAgentStatus(QStringLiteral("Redrob response held · display synchronization pending"));
        setAssistantText(QStringLiteral("The document committed while this request was running. No proposals were queued against the stale projection."));
        setStatus(QStringLiteral("Redrob proposals discarded because the visible document was stale"));
        return;
    }
    if (result.status != REDROB_OK) {
        const QString detail = result.error.isEmpty() ? QStringLiteral("request failed safely") : result.error;
        setAgentStatus(QStringLiteral("Live Redrob error · %1").arg(detail));
        setAssistantText(QStringLiteral("No proposals were created. You can continue editing and try again."));
        setStatus(QStringLiteral("Redrob request failed: %1").arg(detail));
        return;
    }

    QJsonParseError parseError;
    const QJsonDocument document = QJsonDocument::fromJson(result.payload, &parseError);
    if (parseError.error != QJsonParseError::NoError || !document.isObject()) {
        setAgentStatus(QStringLiteral("Live Redrob error · invalid response JSON"));
        setStatus(QStringLiteral("Redrob response could not be read safely"));
        return;
    }
    const QJsonObject root = document.object();
    if (!root.value(QStringLiteral("assistant_text")).isString()
        || !root.value(QStringLiteral("proposals")).isArray()) {
        setAgentStatus(QStringLiteral("Live Redrob error · incomplete response"));
        setStatus(QStringLiteral("Redrob response was missing required fields"));
        return;
    }

    struct ParsedProposal {
        QString sourceId;
        QString title;
        QString summary;
        QString actionType;
        QString commandJson;
        qulonglong baseGeneration = 0;
    };
    QVector<ParsedProposal> parsedProposals;
    const QJsonArray proposals = root.value(QStringLiteral("proposals")).toArray();
    parsedProposals.reserve(proposals.size());
    for (const auto &value : proposals) {
        if (!value.isObject()) {
            setStatus(QStringLiteral("Redrob returned a malformed proposal"));
            return;
        }
        const QJsonObject proposal = value.toObject();
        const QJsonObject action = proposal.value(QStringLiteral("action")).toObject();
        ParsedProposal parsed;
        parsed.sourceId = proposal.value(QStringLiteral("id")).toString();
        parsed.title = proposal.value(QStringLiteral("title")).toString();
        parsed.summary = proposal.value(QStringLiteral("summary")).toString();
        parsed.actionType = action.value(QStringLiteral("type")).toString();
        parsed.baseGeneration = proposal.value(QStringLiteral("base_generation")).toVariant().toULongLong();
        if (parsed.sourceId.isEmpty() || parsed.title.isEmpty() || parsed.summary.isEmpty()
            || !proposal.value(QStringLiteral("base_generation")).isDouble()
            || (parsed.actionType != QStringLiteral("command")
                && parsed.actionType != QStringLiteral("undo")
                && parsed.actionType != QStringLiteral("redo"))) {
            setStatus(QStringLiteral("Redrob returned a malformed proposal"));
            return;
        }
        if (parsed.actionType == QStringLiteral("command")) {
            const QJsonValue command = action.value(QStringLiteral("command"));
            if (!command.isObject()) {
                setStatus(QStringLiteral("Redrob returned invalid command JSON"));
                return;
            }
            parsed.commandJson = QString::fromUtf8(canonicalJson(command.toObject()));
        }
        parsedProposals.push_back(std::move(parsed));
    }

    int queued = 0;
    for (auto &proposal : parsedProposals) {
        if (!m_proposals.enqueue(std::move(proposal.title), std::move(proposal.summary),
                                 std::move(proposal.actionType), std::move(proposal.commandJson),
                                 proposal.baseGeneration, result.documentEpoch,
                                 std::move(proposal.sourceId)).isEmpty())
            ++queued;
    }
    setAssistantText(root.value(QStringLiteral("assistant_text")).toString());
    setAgentStatus(QStringLiteral("Live Redrob connected · model auto"));
    setStatus(queued == 0 ? QStringLiteral("Redrob responded without new edit proposals")
                          : QStringLiteral("%1 Redrob proposal(s) queued — review before applying").arg(queued));
}

void EditorBridge::applyProposal(const QString &id)
{
    if (m_projectionStale) {
        setStatus(QStringLiteral("Proposal apply deferred until the committed document is visible"));
        return;
    }
    QString actionType;
    QString commandJson;
    qulonglong baseGeneration = 0;
    qulonglong baseDocumentEpoch = 0;
    if (!m_proposals.lookup(id, &actionType, &commandJson, &baseGeneration, &baseDocumentEpoch)) {
        setStatus(QStringLiteral("Proposal is no longer available"));
        return;
    }
    if (baseGeneration != m_generation || baseDocumentEpoch != m_documentEpoch) {
        setStatus(QStringLiteral("Proposal is stale because the document changed; request it again"));
        return;
    }

    bool applied = false;
    if (actionType == QStringLiteral("command"))
        applied = executeCommand(commandJson);
    else if (actionType == QStringLiteral("undo"))
        applied = executeHistoryAction(false);
    else if (actionType == QStringLiteral("redo"))
        applied = executeHistoryAction(true);
    else
        setStatus(QStringLiteral("Proposal has an unsupported action type"));

    if (applied) {
        m_proposals.remove(id);
        setStatus(m_lastMutationProjectionRefreshed
                      ? QStringLiteral("Approved proposal applied")
                      : QStringLiteral("Approved proposal committed once; display synchronization is retrying"));
    }
}

void EditorBridge::rejectProposal(const QString &id)
{
    if (m_proposals.reject(id))
        setStatus(QStringLiteral("Proposal rejected without changing the document"));
}
