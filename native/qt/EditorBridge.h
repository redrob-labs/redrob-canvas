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
    Q_PROPERTY(int undoDepth READ undoDepth NOTIFY documentChanged)
    Q_PROPERTY(int redoDepth READ redoDepth NOTIFY documentChanged)
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
    // Opacity and flow bindings (I.1), each one sensor bound to its own channel. Separate from the
    // size binding because pressure already drives the diameter: with only a size binding, pressing
    // harder makes a dab both bigger and more opaque and the two cannot be asked for independently.
    Q_PROPERTY(QString brushOpacityDynamic READ brushOpacityDynamic WRITE setBrushOpacityDynamic NOTIFY brushSettingsChanged)
    Q_PROPERTY(QString brushFlowDynamic READ brushFlowDynamic WRITE setBrushFlowDynamic NOTIFY brushSettingsChanged)
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
    // Multihand radial symmetry (Krita multibrush). order 0/1 = off; centre defaults to canvas middle.
    Q_PROPERTY(int brushSymmetryOrder READ brushSymmetryOrder WRITE setBrushSymmetryOrder NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal brushSymmetryCenterX READ brushSymmetryCenterX WRITE setBrushSymmetryCenterX NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal brushSymmetryCenterY READ brushSymmetryCenterY WRITE setBrushSymmetryCenterY NOTIFY brushSettingsChanged)
    // Onion skin (H.2): the render the canvas shows is the ghosted composite while this is on, so the
    // signal is renderImageChanged rather than a settings signal -- a change repaints the canvas.
    Q_PROPERTY(bool onionSkinEnabled READ onionSkinEnabled WRITE setOnionSkinEnabled NOTIFY onionSkinChanged)
    Q_PROPERTY(int onionSkinBefore READ onionSkinBefore WRITE setOnionSkinBefore NOTIFY onionSkinChanged)
    Q_PROPERTY(int onionSkinAfter READ onionSkinAfter WRITE setOnionSkinAfter NOTIFY onionSkinChanged)
    Q_PROPERTY(qreal onionSkinOpacity READ onionSkinOpacity WRITE setOnionSkinOpacity NOTIFY onionSkinChanged)
    // Drawing assistant (Krita assistants): "none" | "vanishing" | "parallel" | "ellipse" plus up to
    // four parameters interpreted per kind (vanishing: p0,p1 = point; parallel: p0..p3 = two points;
    // ellipse: p0,p1 = centre, p2,p3 = radii).
    Q_PROPERTY(QString brushAssistantKind READ brushAssistantKind WRITE setBrushAssistantKind NOTIFY brushSettingsChanged)
    // Brush presets (F.2): a QVariantList of {name, size, hardness, opacity, pencil, aspect} maps.
    Q_PROPERTY(QVariantList brushPresets READ brushPresets NOTIFY brushPresetsChanged)
    // Colour palette (F.3): a QVariantList of QColor swatches.
    Q_PROPERTY(QVariantList palette READ palette NOTIFY paletteChanged)
    // Dyna brush (GIMP dynamic brush): mass-spring smoothing. Off unless enabled.
    Q_PROPERTY(bool brushDynaEnabled READ brushDynaEnabled WRITE setBrushDynaEnabled NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal brushDynaMass READ brushDynaMass WRITE setBrushDynaMass NOTIFY brushSettingsChanged)
    Q_PROPERTY(qreal brushDynaDrag READ brushDynaDrag WRITE setBrushDynaDrag NOTIFY brushSettingsChanged)
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
    int undoDepth() const;
    int redoDepth() const;
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
    QString brushOpacityDynamic() const;
    void setBrushOpacityDynamic(const QString &sensor);
    QString brushFlowDynamic() const;
    void setBrushFlowDynamic(const QString &sensor);
    bool brushPipe() const;
    void setBrushPipe(bool pipe);
    // Sets the clone source anchor (canvas coordinates), typically from a modifier-click.
    Q_INVOKABLE void setCloneSource(qreal x, qreal y);
    void setBrushHardness(qreal hardness);
    qreal brushAspect() const;
    // Brush presets (F.2).
    QVariantList brushPresets() const;
    Q_INVOKABLE void saveBrushPreset(const QString &name);
    Q_INVOKABLE void applyBrushPreset(int index);
    Q_INVOKABLE void removeBrushPreset(int index);
    // Colour palette (F.3).
    QVariantList palette() const;
    Q_INVOKABLE void addPaletteColor(const QColor &color);
    Q_INVOKABLE void removePaletteColor(int index);
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
    int brushSymmetryOrder() const;
    void setBrushSymmetryOrder(int order);
    qreal brushSymmetryCenterX() const;
    void setBrushSymmetryCenterX(qreal x);
    qreal brushSymmetryCenterY() const;
    void setBrushSymmetryCenterY(qreal y);
    bool onionSkinEnabled() const;
    void setOnionSkinEnabled(bool enabled);
    int onionSkinBefore() const;
    void setOnionSkinBefore(int count);
    int onionSkinAfter() const;
    void setOnionSkinAfter(int count);
    qreal onionSkinOpacity() const;
    void setOnionSkinOpacity(qreal opacity);
    QString brushAssistantKind() const;
    void setBrushAssistantKind(const QString &kind);
    // Four parameters interpreted per assistant kind (see the Q_PROPERTY comment).
    Q_INVOKABLE void setBrushAssistantParams(qreal p0, qreal p1, qreal p2, qreal p3);
    bool brushDynaEnabled() const;
    void setBrushDynaEnabled(bool enabled);
    qreal brushDynaMass() const;
    void setBrushDynaMass(qreal mass);
    qreal brushDynaDrag() const;
    void setBrushDynaDrag(qreal drag);
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
    // Cage transform: two equal-length flat coordinate lists [x0,y0,x1,y1,...] for the source cage
    // and the destination cage it is dragged to.
    Q_INVOKABLE void cageTransform(const QVariantList &srcCage, const QVariantList &dstCage,
                                   const QString &sampling);
    // Warp / liquify brush: a stroke (flat [x0,y0,...]) pushes/grows/shrinks/swirls pixels. mode is
    // "move" | "grow" | "shrink" | "swirl_cw" | "swirl_ccw".
    Q_INVOKABLE void warpBrush(const QVariantList &points, const QString &mode, qreal radius,
                               qreal strength, const QString &sampling);
    // N-point deformation: two equal-length flat coordinate lists for the source control points and
    // the destination positions they were dragged to.
    Q_INVOKABLE void nPointTransform(const QVariantList &srcPts, const QVariantList &dstPts,
                                     const QString &sampling);
    // 3D transform: rotate the layer about its centre (degrees about X/Y/Z) and project through a
    // pinhole camera `distance` canvas-widths away.
    Q_INVOKABLE void transform3d(qreal rotXDeg, qreal rotYDeg, qreal rotZDeg, qreal distance,
                                 const QString &sampling);
    // Enclose-and-fill (Krita): fill regions inside the rectangle closed off from its border.
    Q_INVOKABLE void encloseAndFill(qreal x, qreal y, qreal w, qreal h, const QColor &color,
                                    int alphaThreshold);
    // Smart patch (Krita): content-aware fill of the current selection.
    Q_INVOKABLE void smartPatch(int searchRadius);
    // Lazybrush (Krita): colour regions from scribbles. `scribbles` is a flat list [x0,y0,r0,g0,b0,
    // a0, x1,y1,...] — six numbers per seed.
    Q_INVOKABLE void lazybrush(const QVariantList &scribbles);

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
    Q_INVOKABLE void applyMotionBlur(qreal angleDegrees, int distance);
    Q_INVOKABLE void applyLensBlur(int radius);
    Q_INVOKABLE void applyEdgeDetect(qreal amount);
    Q_INVOKABLE void applyEmboss(qreal angleDegrees);
    Q_INVOKABLE void applyLaplace();
    Q_INVOKABLE void applyPixelize(int block);
    Q_INVOKABLE void applyWaves(qreal amplitude, qreal wavelength);
    Q_INVOKABLE void applyRipple(qreal amplitude, qreal wavelength, bool horizontal);
    Q_INVOKABLE void applyWhirlPinch(qreal whirlDegrees, qreal pinch);
    Q_INVOKABLE void applyLensDistortion(qreal mainAmount);
    Q_INVOKABLE void applyRgbNoise(qreal amount, int seed);
    Q_INVOKABLE void applyHsvNoise(qreal hue, qreal saturation, qreal value, int seed);
    Q_INVOKABLE void applyHurl(qreal amount, int seed);
    Q_INVOKABLE void applyPick(qreal amount, int seed);
    Q_INVOKABLE void applySpread(int amount, int seed);
    Q_INVOKABLE void applyCheckerboard(int size, const QColor &a, const QColor &b);
    Q_INVOKABLE void applyGradientMap(const QColor &low, const QColor &high);
    Q_INVOKABLE void applyPlasma(qreal turbulence, int seed);
    Q_INVOKABLE void applySolidNoise(int detail, int seed);
    Q_INVOKABLE void applyCellNoise(int density, int seed);
    Q_INVOKABLE void applyColorBalance(qreal red, qreal green, qreal blue);
    Q_INVOKABLE void applyColorTemperature(qreal amount);
    Q_INVOKABLE void applyExposure(qreal stops);
    Q_INVOKABLE void applyHueChroma(qreal hueDegrees, qreal chroma);
    Q_INVOKABLE void applySaturation(qreal scale);
    Q_INVOKABLE void applyDither(int levels);
    Q_INVOKABLE void applyOilify(int radius);
    Q_INVOKABLE void applyCartoon(qreal amount);
    Q_INVOKABLE void applySoftGlow(int radius, qreal amount);
    Q_INVOKABLE void applyPhotocopy(qreal amount);
    Q_INVOKABLE void applyApplyCanvas(qreal depth);
    Q_INVOKABLE void applyCubism(int tile, int seed);
    // Map filters take an OPTIONAL map layer (H.18). An empty string means the layer's own luma, which
    // is what every existing caller meant, so the default keeps those call sites working unchanged.
    Q_INVOKABLE void applyBumpMap(qreal azimuthDegrees, qreal elevationDegrees, qreal depth,
                                  const QString &mapLayerId = QString());
    Q_INVOKABLE void applyDisplace(qreal amount, const QString &mapLayerId = QString());
    Q_INVOKABLE void applyFractalTrace(int depth, qreal scale,
                                       const QString &mapLayerId = QString());
    Q_INVOKABLE void applyWarpMap(qreal amount, int steps,
                                  const QString &mapLayerId = QString());
    Q_INVOKABLE void applyHalftone(int cell);
    Q_INVOKABLE void applyPhongBump(qreal azimuthDegrees, qreal elevationDegrees, qreal depth, qreal shininess);
    Q_INVOKABLE void applyPalettize(int levels);
    Q_INVOKABLE void applyNormalMap(qreal strength);
    // Channel mixer: nine row-major coefficients (rr,rg,rb, gr,gg,gb, br,bg,bb) and three offsets.
    Q_INVOKABLE void applyChannelMixer(const QVariantList &matrix, const QVariantList &offset);
    Q_INVOKABLE void applyLabAdjust(qreal lightness, qreal chroma);
    // Histogram of the current render: returns {r:[256], g:[256], b:[256], luma:[256]} as a map of
    // QVariantList bins. Computed from the composited image.
    // Operation graph (G.1): apply a chain of filter ops. `nodesJson` is a JSON array of
    // {filter:{kind,...}, amount, enabled} objects.
    Q_INVOKABLE void applyOpGraph(const QString &nodesJson);
    // Layer style (G.3): bake drop shadow / outer glow / bevel. `styleJson` is a JSON object
    // {drop_shadow?, outer_glow?, bevel?} with the per-effect fields.
    Q_INVOKABLE void applyLayerStyle(const QString &styleJson);
    Q_INVOKABLE QVariantMap histogram() const;

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
    void onionSkinChanged();
    void brushPresetsChanged();
    void paletteChanged();
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
    int m_undoDepth = 0;
    int m_redoDepth = 0;
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
    QString m_brushOpacityDynamic = QStringLiteral("off");
    QString m_brushFlowDynamic = QStringLiteral("off");
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
    int m_brushSymmetryOrder = 0;
    qreal m_brushSymmetryCenterX = 640.0;
    qreal m_brushSymmetryCenterY = 400.0;
    bool m_onionSkinEnabled = false;
    int m_onionSkinBefore = 1;
    int m_onionSkinAfter = 1;
    qreal m_onionSkinOpacity = 0.4;
    QString m_brushAssistantKind = QStringLiteral("none");
    qreal m_brushAssistantP0 = 0.0;
    qreal m_brushAssistantP1 = 0.0;
    qreal m_brushAssistantP2 = 0.0;
    qreal m_brushAssistantP3 = 0.0;
    bool m_brushDynaEnabled = false;
    qreal m_brushDynaMass = 0.4;
    qreal m_brushDynaDrag = 0.3;
    QColor m_brushColor = QColor(QStringLiteral("#f1f3f5"));
    QVariantList m_brushPresets;
    QVariantList m_palette;
    QString m_statusMessage;
    QString m_agentStatus;
    QString m_assistantText;
    QString m_currentFile;
    QString m_formatCapabilities;
    QByteArray m_apiKey;
};
