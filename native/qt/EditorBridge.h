// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QByteArray>
#include <QColor>
#include <QFutureWatcher>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QTimer>
#include <QUrl>
#include <QVariantList>

#include <memory>
#include <optional>

#include "FrameModel.h"
#include "LayerModel.h"
#include "ProposalModel.h"
#include "redrob_ffi.h"

struct AgentResult
{
    int status = REDROB_ERROR;
    QByteArray payload;
    QString error;
    qulonglong documentEpoch = 0;
};

class EditorBridge final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(int documentWidth READ documentWidth NOTIFY documentChanged)
    Q_PROPERTY(int documentHeight READ documentHeight NOTIFY documentChanged)
    Q_PROPERTY(qulonglong generation READ generation NOTIFY documentChanged)
    Q_PROPERTY(bool canUndo READ canUndo NOTIFY documentChanged)
    Q_PROPERTY(bool canRedo READ canRedo NOTIFY documentChanged)
    Q_PROPERTY(QAbstractItemModel *frames READ frames CONSTANT)
    Q_PROPERTY(quint32 currentFrame READ currentFrame NOTIFY timelineChanged)
    Q_PROPERTY(int currentFrameIndex READ currentFrameIndex NOTIFY timelineChanged)
    Q_PROPERTY(int frameCount READ frameCount NOTIFY timelineChanged)
    Q_PROPERTY(qreal fps READ fps NOTIFY timelineChanged)
    Q_PROPERTY(quint32 rangeStart READ rangeStart NOTIFY timelineChanged)
    Q_PROPERTY(quint32 rangeEnd READ rangeEnd NOTIFY timelineChanged)
    Q_PROPERTY(bool looping READ looping NOTIFY timelineChanged)
    Q_PROPERTY(bool playing READ playing NOTIFY timelineChanged)
    Q_PROPERTY(QString activeLayerId READ activeLayerId NOTIFY documentChanged)
    Q_PROPERTY(QString activeNodeKind READ activeNodeKind NOTIFY documentChanged)
    Q_PROPERTY(bool activeNodeCanEditRaster READ activeNodeCanEditRaster NOTIFY documentChanged)
    Q_PROPERTY(bool activeNodeCanEditText READ activeNodeCanEditText NOTIFY documentChanged)
    Q_PROPERTY(bool activeNodeCanEditVector READ activeNodeCanEditVector NOTIFY documentChanged)
    Q_PROPERTY(bool activeNodeCanRasterize READ activeNodeCanRasterize NOTIFY documentChanged)
    Q_PROPERTY(bool activeNodeHasMask READ activeNodeHasMask NOTIFY documentChanged)
    Q_PROPERTY(QAbstractItemModel *layers READ layers CONSTANT)
    Q_PROPERTY(QAbstractItemModel *proposals READ proposals CONSTANT)
    Q_PROPERTY(QImage renderImage READ renderImage NOTIFY renderImageChanged)
    Q_PROPERTY(QImage selectionMask READ selectionMask NOTIFY selectionChanged)
    Q_PROPERTY(bool selectionActive READ selectionActive NOTIFY selectionChanged)
    Q_PROPERTY(qreal brushSize READ brushSize WRITE setBrushSize NOTIFY brushSettingsChanged)
    Q_PROPERTY(QColor brushColor READ brushColor WRITE setBrushColor NOTIFY brushColorChanged)
    Q_PROPERTY(qreal brushOpacity READ brushOpacity WRITE setBrushOpacity NOTIFY brushSettingsChanged)
    Q_PROPERTY(QString brushSmoothingKind READ brushSmoothingKind WRITE setBrushSmoothingKind NOTIFY brushSettingsChanged)
    Q_PROPERTY(int brushSmoothingWindow READ brushSmoothingWindow WRITE setBrushSmoothingWindow NOTIFY brushSettingsChanged)
    Q_PROPERTY(bool mirrorXEnabled READ mirrorXEnabled WRITE setMirrorXEnabled NOTIFY brushSettingsChanged)
    Q_PROPERTY(bool mirrorYEnabled READ mirrorYEnabled WRITE setMirrorYEnabled NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal mirrorXAxis READ mirrorXAxis WRITE setMirrorXAxis NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal mirrorYAxis READ mirrorYAxis WRITE setMirrorYAxis NOTIFY brushSettingsChanged)
    Q_PROPERTY(QString statusMessage READ statusMessage NOTIFY statusMessageChanged)
    Q_PROPERTY(QString modelStatus READ modelStatus CONSTANT)
    Q_PROPERTY(bool liveAgentConfigured READ liveAgentConfigured CONSTANT)
    Q_PROPERTY(bool agentBusy READ agentBusy NOTIFY agentBusyChanged)
    Q_PROPERTY(QString agentStatus READ agentStatus NOTIFY agentStatusChanged)
    Q_PROPERTY(QString assistantText READ assistantText NOTIFY assistantTextChanged)
    Q_PROPERTY(QString currentFile READ currentFile NOTIFY currentFileChanged)
    Q_PROPERTY(QString formatCapabilities READ formatCapabilities CONSTANT)

public:
    explicit EditorBridge(QObject *parent = nullptr);
    ~EditorBridge() override;

    int documentWidth() const;
    int documentHeight() const;
    qulonglong generation() const;
    bool canUndo() const;
    bool canRedo() const;
    QAbstractItemModel *frames();
    quint32 currentFrame() const;
    int currentFrameIndex() const;
    int frameCount() const;
    qreal fps() const;
    quint32 rangeStart() const;
    quint32 rangeEnd() const;
    bool looping() const;
    bool playing() const;
    QString activeLayerId() const;
    QString activeNodeKind() const;
    bool activeNodeCanEditRaster() const;
    bool activeNodeCanEditText() const;
    bool activeNodeCanEditVector() const;
    bool activeNodeCanRasterize() const;
    bool activeNodeHasMask() const;
    QAbstractItemModel *layers();
    QAbstractItemModel *proposals();
    QImage renderImage() const;
    QImage selectionMask() const;
    bool selectionActive() const;
    qreal brushSize() const;
    void setBrushSize(qreal size);
    QColor brushColor() const;
    void setBrushColor(const QColor &color);
    qreal brushOpacity() const;
    void setBrushOpacity(qreal opacity);
    QString brushSmoothingKind() const;
    void setBrushSmoothingKind(const QString &kind);
    int brushSmoothingWindow() const;
    void setBrushSmoothingWindow(int window);
    bool mirrorXEnabled() const;
    void setMirrorXEnabled(bool enabled);
    bool mirrorYEnabled() const;
    void setMirrorYEnabled(bool enabled);
    qreal mirrorXAxis() const;
    void setMirrorXAxis(qreal axis);
    qreal mirrorYAxis() const;
    void setMirrorYAxis(qreal axis);
    QString statusMessage() const;
    QString modelStatus() const;
    bool liveAgentConfigured() const;
    bool agentBusy() const;
    QString agentStatus() const;
    QString assistantText() const;
    QString currentFile() const;
    QString formatCapabilities() const;

    Q_INVOKABLE bool executeCommand(const QString &commandJson);
    Q_INVOKABLE void undo();
    Q_INVOKABLE void redo();
    Q_INVOKABLE void addFrame(int index = -1);
    Q_INVOKABLE void duplicateFrame(quint32 sourceId, int index = -1);
    Q_INVOKABLE void removeFrame(quint32 id);
    Q_INVOKABLE void moveFrame(quint32 id, int newIndex);
    Q_INVOKABLE void setTimelineFps(qreal fps);
    Q_INVOKABLE void setPlaybackRange(quint32 start, quint32 end);
    Q_INVOKABLE void setLooping(bool looping);
    Q_INVOKABLE bool setCurrentFrame(quint32 id);
    Q_INVOKABLE bool setPlaying(bool playing);
    Q_INVOKABLE bool advancePlayback();
    Q_INVOKABLE void beginStroke(qreal x, qreal y, qreal pressure = 1.0);
    Q_INVOKABLE void addStrokePoint(qreal x, qreal y, qreal pressure = 1.0);
    Q_INVOKABLE void endStroke();
    Q_INVOKABLE void cancelStroke();
    Q_INVOKABLE void fill(const QColor &color);
    Q_INVOKABLE void clearActiveLayer();
    Q_INVOKABLE void addLayer(const QString &name = QStringLiteral("New layer"));
    Q_INVOKABLE void addGroup(const QString &name = QStringLiteral("New group"),
                              const QString &parentId = {}, int siblingIndex = -1);
    Q_INVOKABLE void addTextNode(const QString &name, const QString &text, qreal originX,
                                 qreal originY, qreal fontSize, const QColor &color,
                                 const QString &parentId = {}, int siblingIndex = -1);
    Q_INVOKABLE void setTextContent(
        const QString &id, const QString &text, qreal originX, qreal originY, qreal fontSize,
        const QColor &color, const QString &fontFamily = QStringLiteral("font8x8 Basic Latin"),
        const QString &fontId = QStringLiteral("font8x8-basic-0.3.1"));
    Q_INVOKABLE void addVectorRectangle(const QString &name, qreal x, qreal y, qreal width,
                                        qreal height, const QColor &fill, const QColor &stroke,
                                        qreal strokeWidth, const QString &parentId = {},
                                        int siblingIndex = -1);
    Q_INVOKABLE void setVectorRectangle(const QString &id, qreal x, qreal y, qreal width,
                                        qreal height, const QColor &fill, const QColor &stroke,
                                        qreal strokeWidth);
    Q_INVOKABLE void rasterizeSemanticNode(const QString &id);
    Q_INVOKABLE void moveNode(const QString &id, const QString &parentId, int siblingIndex);
    Q_INVOKABLE void addRasterMask(const QString &id);
    Q_INVOKABLE void rasterMaskFromSelection(const QString &id);
    Q_INVOKABLE void removeRasterMask(const QString &id);
    Q_INVOKABLE void setRasterMaskEnabled(const QString &id, bool enabled);
    Q_INVOKABLE void replaceRasterMask(const QString &id, int x, int y, int width, int height,
                                       const QVariantList &pixels);
    Q_INVOKABLE void deleteLayer(const QString &id);
    Q_INVOKABLE void setActiveLayer(const QString &id);
    Q_INVOKABLE void renameLayer(const QString &id, const QString &name);
    Q_INVOKABLE void setLayerOpacity(const QString &id, qreal opacity);
    Q_INVOKABLE void setLayerVisibility(const QString &id, bool visible);
    Q_INVOKABLE void setLayerBlendMode(const QString &id, const QString &mode);
    Q_INVOKABLE void reorderLayer(const QString &id, int newIndex);

    Q_INVOKABLE void selectRectangle(qreal x, qreal y, qreal width, qreal height,
                                     const QString &mode);
    Q_INVOKABLE void selectEllipse(qreal x, qreal y, qreal width, qreal height,
                                   const QString &mode);
    Q_INVOKABLE void selectAll();
    Q_INVOKABLE void invertSelection();
    Q_INVOKABLE void clearSelection();
    Q_INVOKABLE void featherSelection(int radius);
    Q_INVOKABLE void growSelection(int radius);
    Q_INVOKABLE void shrinkSelection(int radius);

    Q_INVOKABLE void linearGradient(qreal startX, qreal startY, qreal endX, qreal endY,
                                    const QColor &startColor, const QColor &endColor);
    Q_INVOKABLE void radialGradient(qreal centerX, qreal centerY, qreal radius,
                                    const QColor &startColor, const QColor &endColor);
    Q_INVOKABLE void cropCanvas(qreal x, qreal y, qreal width, qreal height);
    Q_INVOKABLE void padCanvas(int left, int top, int right, int bottom);
    Q_INVOKABLE void resizeCanvas(int width, int height, const QString &sampling);
    Q_INVOKABLE void flipActive(bool horizontal, bool vertical);
    Q_INVOKABLE void rotateActive90(bool clockwise);
    Q_INVOKABLE void transformActive(qreal m11, qreal m12, qreal m21, qreal m22,
                                     qreal tx, qreal ty, const QString &sampling);

    Q_INVOKABLE void applyFilter(const QString &kind);
    Q_INVOKABLE void applyBrightnessContrast(int brightness, qreal contrast);
    Q_INVOKABLE void applyGaussianBlur(qreal sigma);
    Q_INVOKABLE void applyThreshold(int threshold);
    Q_INVOKABLE void applyPosterize(int levels);
    Q_INVOKABLE void applyLevels(int inputBlack, int inputWhite, qreal gamma,
                                 int outputBlack, int outputWhite);
    Q_INVOKABLE void applyHueSaturation(qreal hueDegrees, qreal saturation, qreal lightness);
    Q_INVOKABLE void applyBoxBlur(int radius);
    Q_INVOKABLE void applySharpen(qreal amount);

    Q_INVOKABLE bool openProject(const QUrl &url);
    Q_INVOKABLE bool importFile(const QUrl &url);
    Q_INVOKABLE bool saveProject(const QUrl &url = {});
    Q_INVOKABLE bool exportFile(const QUrl &url, const QString &format, bool allowLoss,
                                quint32 frameId, int jpegQuality, const QColor &matte);
    // Compatibility routes: known RRG/PNG extensions only; unknown extensions fail.
    Q_INVOKABLE bool openFile(const QUrl &url);
    Q_INVOKABLE bool saveFile(const QUrl &url = {});

    // Both deterministic demo parsing and future live Redrob tool calls enter
    // through this queue. No proposal mutates state until applyProposal.
    Q_INVOKABLE QString enqueueProposal(const QString &title, const QString &summary,
                                        const QString &commandJson);
    Q_INVOKABLE bool proposePrompt(const QString &prompt);
    Q_INVOKABLE void applyProposal(const QString &id);
    Q_INVOKABLE void rejectProposal(const QString &id);
    Q_INVOKABLE void injectProjectionFailureForSmokeTest();

signals:
    void documentChanged();
    void timelineChanged();
    void renderImageChanged();
    void selectionChanged();
    void brushSettingsChanged();
    void brushColorChanged();
    void statusMessageChanged();
    void agentBusyChanged();
    void agentStatusChanged();
    void assistantTextChanged();
    void currentFileChanged();

private:
    bool executeCommand(const QJsonObject &command);
    bool refresh(bool captureSelection = true);
    void scheduleProjectionRefresh(bool captureSelection);
    bool replaceFromGenericBytes(const QByteArray &bytes, const QString &expectedFormat,
                                 const QString &sourceName, bool projectIdentity,
                                 const QString &projectPath = {});
    bool exportGenericBytes(const QString &format, bool allowLoss,
                            std::optional<quint32> frameId, int jpegQuality,
                            const QColor &matte, QByteArray *bytes,
                            QJsonObject *result);
    bool writeAtomically(const QString &path, const QByteArray &bytes);
    bool executeHistoryAction(bool redoAction);
    bool executeNavigation(int kind, quint32 frameId = 0, bool playing = false);
    std::optional<quint32> allocateFrameId();
    bool proposeLocally(const QString &prompt);
    void finishAgentRequest(const AgentResult &result);
    void setStatus(QString status);
    void setAgentBusy(bool busy);
    void setAgentStatus(QString status);
    void setAssistantText(QString text);
    QString ffiError() const;
    QJsonObject brushSettingsObject() const;
    static QJsonObject colorObject(const QColor &color);
    static bool rectObject(qreal x, qreal y, qreal width, qreal height, QJsonObject *outRect);
    static QByteArray canonicalJson(const QJsonObject &object);
    static QByteArray takeBuffer(RedrobBuffer buffer);
    static bool validSelectionMode(const QString &mode);
    static bool validSampling(const QString &sampling);

    std::shared_ptr<RedrobEditor> m_editor;
    LayerModel m_layers;
    FrameModel m_frames;
    ProposalModel m_proposals;
    QFutureWatcher<AgentResult> m_agentWatcher;
    QTimer m_refreshRetryTimer;
    QTimer m_playbackTimer;
    QImage m_renderImage;
    QImage m_selectionMask;
    QJsonArray m_strokePoints;
    int m_width = 0;
    int m_height = 0;
    qulonglong m_generation = 0;
    qulonglong m_documentEpoch = 0;
    bool m_canUndo = false;
    bool m_canRedo = false;
    bool m_strokeActive = false;
    bool m_strokeTruncated = false;
    bool m_selectionActive = false;
    bool m_looping = false;
    bool m_playing = false;
    bool m_liveAgentConfigured = false;
    bool m_agentBusy = false;
    bool m_projectionStale = false;
    bool m_retryNeedsSelection = false;
    bool m_lastMutationProjectionRefreshed = true;
    bool m_mirrorXEnabled = false;
    bool m_mirrorYEnabled = false;
    qreal m_brushSize = 18.0;
    qreal m_fps = 10.0;
    quint32 m_rangeStart = 0;
    quint32 m_rangeEnd = 0;
    qreal m_brushOpacity = 1.0;
    int m_brushSmoothingWindow = 4;
    QString m_brushSmoothingKind = QStringLiteral("none");
    qreal m_mirrorXAxis = 640.0;
    qreal m_mirrorYAxis = 400.0;
    QColor m_brushColor = QColor(QStringLiteral("#f1f3f5"));
    QString m_statusMessage;
    QString m_agentStatus;
    QString m_assistantText;
    QString m_currentFile;
    QString m_formatCapabilities;
    QByteArray m_apiKey;
};
