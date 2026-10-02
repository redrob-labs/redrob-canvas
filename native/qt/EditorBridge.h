// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QByteArray>
#include <QColor>
#include <QFutureWatcher>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QStringList>
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
    // The dab shape the core already draws (DabShape): 1.0 hard edge .. 0.0 softest, and height as a
    // fraction of width (1.0 round, smaller flatter).
    Q_PROPERTY(qreal brushHardness READ brushHardness WRITE setBrushHardness NOTIFY brushSettingsChanged)
    // Eraser mode: the brush removes paint (BrushSettings::erase), as Krita's E toggle does.
    Q_PROPERTY(bool brushErase READ brushErase WRITE setBrushErase NOTIFY brushSettingsChanged)
    // Pencil mode: a hard, aliased edge (GIMP's pencil vs paintbrush). Forces hardness 1 and no
    // edge antialiasing.
    Q_PROPERTY(bool brushPencil READ brushPencil WRITE setBrushPencil NOTIFY brushSettingsChanged)
    // Airbrush mode: paint builds up gradually while held (low per-dab flow + a repeat timer in QML).
    Q_PROPERTY(bool brushAirbrush READ brushAirbrush WRITE setBrushAirbrush NOTIFY brushSettingsChanged)
    // Smudge mode: drag the colour already on the layer instead of stamping the brush colour.
    Q_PROPERTY(bool brushSmudge READ brushSmudge WRITE setBrushSmudge NOTIFY brushSettingsChanged)
    // Clone mode: copy the layer from a source region offset from the stroke (set with setCloneSource).
    Q_PROPERTY(bool brushClone READ brushClone WRITE setBrushClone NOTIFY brushSettingsChanged)
    // Heal mode: like clone, but matches the cloned patch to the destination's local colour.
    Q_PROPERTY(bool brushHeal READ brushHeal WRITE setBrushHeal NOTIFY brushSettingsChanged)
    // Convolve: "off", "blur" or "sharpen" -- the dab processes pixels in place instead of painting.
    Q_PROPERTY(QString brushConvolveMode READ brushConvolveMode WRITE setBrushConvolveMode NOTIFY brushSettingsChanged)
    // Dodge/Burn: "off", "dodge" (lighten) or "burn" (darken) the pixels under the dab.
    Q_PROPERTY(QString brushDodgeBurnMode READ brushDodgeBurnMode WRITE setBrushDodgeBurnMode NOTIFY brushSettingsChanged)
    // Dodge/Burn tonal range: "shadows", "midtones" or "highlights".
    Q_PROPERTY(QString brushDodgeRange READ brushDodgeRange WRITE setBrushDodgeRange NOTIFY brushSettingsChanged)
    // Ink mode: a calligraphic nib whose line thins as the pen moves faster.
    Q_PROPERTY(bool brushInk READ brushInk WRITE setBrushInk NOTIFY brushSettingsChanged)
    // MyPaint mode: scatter jittered sub-dabs for a grainy, textured line.
    Q_PROPERTY(bool brushMyPaint READ brushMyPaint WRITE setBrushMyPaint NOTIFY brushSettingsChanged)
    // Size dynamics sensor: "off", "pressure", "speed" or "random" drives the brush size.
    Q_PROPERTY(QString brushSizeDynamic READ brushSizeDynamic WRITE setBrushSizeDynamic NOTIFY brushSettingsChanged)
    // GIH pipe: cycle through every loaded brush tip, one per dab, instead of a single tip.
    Q_PROPERTY(bool brushPipe READ brushPipe WRITE setBrushPipe NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal brushAspect READ brushAspect WRITE setBrushAspect NOTIFY brushSettingsChanged)
    // Image tips loaded from a GBR/ABR file. -1 draws the generated dab (hardness, roundness);
    // 0.. draws that tip instead, carried on each stroke because the core keeps no tip store.
    Q_PROPERTY(QStringList brushTipNames READ brushTipNames NOTIFY brushSettingsChanged)
    Q_PROPERTY(int brushTipIndex READ brushTipIndex WRITE setBrushTipIndex NOTIFY brushSettingsChanged)
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
    qreal brushHardness() const;
    bool brushErase() const;
    void setBrushErase(bool erase);
    bool brushPencil() const;
    void setBrushPencil(bool pencil);
    bool brushAirbrush() const;
    void setBrushAirbrush(bool airbrush);
    bool brushSmudge() const;
    void setBrushSmudge(bool smudge);
    bool brushClone() const;
    void setBrushClone(bool clone);
    bool brushHeal() const;
    void setBrushHeal(bool heal);
    QString brushConvolveMode() const;
    void setBrushConvolveMode(const QString &mode);
    QString brushDodgeBurnMode() const;
    void setBrushDodgeBurnMode(const QString &mode);
    QString brushDodgeRange() const;
    void setBrushDodgeRange(const QString &range);
    bool brushInk() const;
    void setBrushInk(bool ink);
    bool brushMyPaint() const;
    void setBrushMyPaint(bool mypaint);
    QString brushSizeDynamic() const;
    void setBrushSizeDynamic(const QString &sensor);
    bool brushPipe() const;
    void setBrushPipe(bool pipe);
    // Sets the clone source anchor (canvas coordinates), typically from a modifier-click.
    Q_INVOKABLE void setCloneSource(qreal x, qreal y);
    void setBrushHardness(qreal hardness);
    qreal brushAspect() const;
    void setBrushAspect(qreal aspect);
    QStringList brushTipNames() const;
    int brushTipIndex() const;
    void setBrushTipIndex(int index);
    // Reads a .gbr or .abr file and appends its tips; selects the first new one. Returns how many.
    Q_INVOKABLE int loadBrushTips(const QUrl &url);
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
    /// Bucket fill: the connected region of similar colour around (x, y), tolerance 0 to 255 (Lab).
    Q_INVOKABLE void floodFill(qreal x, qreal y, const QColor &color, int tolerance);
    // Colour of the visible image at a canvas point, as an opaque colour for the brush. Invalid
    // outside the canvas or where nothing is painted, so the caller keeps the current colour.
    Q_INVOKABLE QColor sampleColor(qreal x, qreal y) const;
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
    // Pen tool: a straight-segment vector path from clicked anchors (flat [x0,y0,...] list).
    Q_INVOKABLE void addVectorPath(const QVariantList &points, bool closed, const QString &name = {});
    Q_INVOKABLE void setVectorRectangle(const QString &id, qreal x, qreal y, qreal width,
                                        qreal height, const QColor &fill, const QColor &stroke,
                                        qreal strokeWidth);
    // Shapes built by the ported Graphite geometry. Two entry points, not six, because the
    // split follows the DRAG that creates them rather than the shape enum: a rectangle,
    // rounded rectangle, ellipse and line are all "two opposite corners", while a polygon
    // and a star are "a centre and a radius". A per-variant invokable would have made QML
    // choose between six near-identical calls for what the user does with one gesture.
    Q_INVOKABLE void addShapeFromBox(const QString &kind, const QString &name, qreal x1, qreal y1,
                                     qreal x2, qreal y2, qreal cornerRadius, const QColor &fill,
                                     const QColor &stroke, qreal strokeWidth,
                                     const QString &parentId = {}, int siblingIndex = -1);
    Q_INVOKABLE void addShapeFromRadius(const QString &kind, const QString &name, qreal centreX,
                                        qreal centreY, qreal radius, int sides, qreal innerRatio,
                                        const QColor &fill, const QColor &stroke,
                                        qreal strokeWidth, const QString &parentId = {},
                                        int siblingIndex = -1);
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
    // Free-form lasso / polygon selection. points is a flat [x0,y0,x1,y1,...] list from QML.
    Q_INVOKABLE void selectPolygon(const QVariantList &points, const QString &mode);
    // Magic wand: select by colour at (x, y). contiguous floods the connected region.
    Q_INVOKABLE void selectByColor(qreal x, qreal y, int tolerance, bool contiguous,
                                   const QString &mode);
    // Intelligent scissors: edge-snapping selection through anchors. anchors is a flat
    // [x0,y0,x1,y1,...] list from QML.
    Q_INVOKABLE void selectScissors(const QVariantList &anchors, const QString &mode);
    // Foreground select: classify pixels from scribbled fg/bg samples (flat [x0,y0,...] lists).
    Q_INVOKABLE void selectForeground(const QVariantList &fg, const QVariantList &bg,
                                      const QString &mode);
    // Align the active layer's opaque bounds to the canvas (or itself). h/v: 0 none, 1 min, 2
    // centre, 3 max.
    Q_INVOKABLE void alignActiveLayer(int horizontal, int vertical, bool toCanvas);
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
    // Affine convenience transforms about the layer's centre, built as a matrix for transformActive.
    Q_INVOKABLE void rotateActive(qreal degrees, const QString &sampling);
    Q_INVOKABLE void scaleActive(qreal sx, qreal sy, const QString &sampling);
    Q_INVOKABLE void shearActive(qreal shearX, qreal shearY, const QString &sampling);
    // Perspective / distort: eight destination-corner coordinates (TLx,TLy, TRx,TRy, BRx,BRy, BLx,BLy).
    Q_INVOKABLE void perspectiveActive(const QVariantList &corners, const QString &sampling);

    Q_INVOKABLE void applyFilter(const QString &kind);
    Q_INVOKABLE void applyBrightnessContrast(int brightness, qreal contrast);
    Q_INVOKABLE void applyGaussianBlur(qreal sigma);
    Q_INVOKABLE void applyThreshold(int threshold);
    Q_INVOKABLE void applyPosterize(int levels);
    Q_INVOKABLE void applyLevels(int inputBlack, int inputWhite, qreal gamma,
                                 int outputBlack, int outputWhite);
    /// Curves through five points: black and white stay, and the outputs at 25%, 50% and 75% input are
    /// given 0 to 255, so the curve can lift, darken or bend into an S.
    Q_INVOKABLE void applyCurves(int quarter, int middle, int threeQuarter);
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
    // DabShape::default(): a hard round dab, which is what strokes drew before the shape was exposed.
    qreal m_brushHardness = 1.0;
    bool m_brushErase = false;
    bool m_brushPencil = false;
    bool m_brushAirbrush = false;
    // Airbrush flow: each held dab deposits this fraction of the opacity, so paint builds up.
    double m_brushFlow = 0.08;
    bool m_brushSmudge = false;
    // Smudge rate: how fast the carried colour catches up to the pixel under the dab (0 smears far,
    // 1 just stamps the sample).
    double m_brushSmudgeRate = 0.25;
    bool m_brushClone = false;
    bool m_brushHeal = false;
    QString m_brushConvolveMode = QStringLiteral("off");
    QString m_brushDodgeBurnMode = QStringLiteral("off");
    QString m_brushDodgeRange = QStringLiteral("midtones");
    bool m_brushInk = false;
    bool m_brushMyPaint = false;
    QString m_brushSizeDynamic = QStringLiteral("off");
    bool m_brushPipe = false;
    bool m_cloneSourceSet = false;
    double m_cloneSourceX = 0.0;
    double m_cloneSourceY = 0.0;
    // Offset captured at stroke start: source = point - offset, held constant across the stroke.
    double m_cloneOffsetX = 0.0;
    double m_cloneOffsetY = 0.0;
    qreal m_brushAspect = 1.0;
    QJsonArray m_brushTips;
    int m_brushTipIndex = -1;
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
