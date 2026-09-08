// SPDX-License-Identifier: GPL-3.0-or-later
#include "EditorBridge.h"

#include "FrameIdAllocator.h"

#include <QFile>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonParseError>
#include <QRegularExpression>
#include <QRandomGenerator>
#include <QSaveFile>
#include <QStringList>
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

QString canonicalFormatForSuffix(const QString &suffix)
{
    const QString lower = suffix.toLower();
    if (lower == QStringLiteral("rrg") || lower == QStringLiteral("png")
        || lower == QStringLiteral("webp") || lower == QStringLiteral("ora")
        || lower == QStringLiteral("svg"))
        return lower;
    if (lower == QStringLiteral("jpg") || lower == QStringLiteral("jpeg"))
        return QStringLiteral("jpeg");
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
{
    m_apiKey = qgetenv("REDROB_API_KEY");
    m_liveAgentConfigured = !m_apiKey.trimmed().isEmpty();
    setAgentStatus(m_liveAgentConfigured
                       ? QStringLiteral("Live Redrob · API key configured · model auto")
                       : QStringLiteral("Local deterministic proposal mode · explicitly no network"));
    connect(&m_agentWatcher, &QFutureWatcher<AgentResult>::finished, this, [this] {
        finishAgentRequest(m_agentWatcher.result());
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
    return {{QStringLiteral("smoothing"), smoothing},
            {QStringLiteral("mirror_x"), m_mirrorXEnabled ? QJsonValue(m_mirrorXAxis)
                                                          : QJsonValue(QJsonValue::Null)},
            {QStringLiteral("mirror_y"), m_mirrorYEnabled ? QJsonValue(m_mirrorYAxis)
                                                          : QJsonValue(QJsonValue::Null)}};
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
    const QByteArray json = canonicalJson(command);
    RedrobBuffer changes{};
    const int status = redrob_editor_execute_json(
        m_editor.get(), reinterpret_cast<const uint8_t *>(json.constData()),
        static_cast<size_t>(json.size()), &changes);
    if (status != REDROB_OK) {
        redrob_buffer_free(changes);
        setStatus(QStringLiteral("Edit rejected: %1").arg(ffiError()));
        return false;
    }
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

bool EditorBridge::executeHistoryAction(bool redoAction)
{
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
    if (!m_editor || m_projectionStale)
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
    addStrokePoint(x, y, pressure);
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
    m_strokePoints.append(QJsonObject{{QStringLiteral("x"), x},
                                      {QStringLiteral("y"), y},
                                      {QStringLiteral("pressure"), boundedPressure}});
}

void EditorBridge::endStroke()
{
    if (!m_strokeActive)
        return;
    m_strokeActive = false;
    if (m_strokePoints.isEmpty())
        return;
    const bool truncated = m_strokeTruncated;
    const QJsonObject command{{QStringLiteral("type"), QStringLiteral("brush_stroke")},
                              {QStringLiteral("points"), m_strokePoints},
                              {QStringLiteral("color"), colorObject(m_brushColor)},
                              {QStringLiteral("size"), m_brushSize},
                              {QStringLiteral("opacity"), m_brushOpacity},
                              {QStringLiteral("settings"), brushSettingsObject()}};
    m_strokePoints = {};
    m_strokeTruncated = false;
    if (executeCommand(command) && truncated)
        setStatus(QStringLiteral("Stroke applied using the first 4096 points"));
}

void EditorBridge::cancelStroke()
{
    m_strokeActive = false;
    m_strokeTruncated = false;
    m_strokePoints = {};
}

void EditorBridge::fill(const QColor &color)
{
    executeCommand({{QStringLiteral("type"), QStringLiteral("fill")},
                    {QStringLiteral("color"), colorObject(color)}});
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

void EditorBridge::addTextNode(const QString &name, const QString &text, qreal originX,
                               qreal originY, qreal fontSize, const QColor &color,
                               const QString &parentId, int siblingIndex)
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
    const QJsonObject content{{QStringLiteral("text"), text},
                              {QStringLiteral("font_family"), QStringLiteral("font8x8 Basic Latin")},
                              {QStringLiteral("font_size"), fontSize},
                              {QStringLiteral("color"), colorObject(color)},
                              {QStringLiteral("origin_x"), originX},
                              {QStringLiteral("origin_y"), originY},
                              {QStringLiteral("font_id"), QStringLiteral("font8x8-basic-0.3.1")}};
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
                                  const QString &fontFamily, const QString &fontId)
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
    executeCommand({{QStringLiteral("type"), QStringLiteral("set_text_content")},
                    {QStringLiteral("id"), id},
                    {QStringLiteral("text"), QJsonObject{{QStringLiteral("text"), text},
                                                           {QStringLiteral("font_family"), fontFamily},
                                                           {QStringLiteral("font_size"), fontSize},
                                                           {QStringLiteral("color"), colorObject(color)},
                                                           {QStringLiteral("origin_x"), originX},
                                                           {QStringLiteral("origin_y"), originY},
                                                           {QStringLiteral("font_id"), fontId}}}});
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
                                   QStringLiteral("add")};
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

void EditorBridge::scheduleProjectionRefresh(bool captureSelection)
{
    m_projectionStale = true;
    m_retryNeedsSelection = m_retryNeedsSelection || captureSelection || m_selectionMask.isNull();
    if (!m_refreshRetryTimer.isActive())
        m_refreshRetryTimer.start();
}

bool EditorBridge::refresh(bool captureSelection)
{
    if (!m_editor)
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
        if (redrob_editor_render_rgba(m_editor.get(), &render) != REDROB_OK) {
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
        setStatus(QStringLiteral("Import failed: expected png, jpg/jpeg, webp, ora, or svg"));
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
