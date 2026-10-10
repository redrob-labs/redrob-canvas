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
#include <QHash>
#include <QSet>
#include <QTimer>
#include <QUrl>
#include <QVariantList>

#include <memory>
#include <optional>

#include "ConsoleConnection.h"
#include "FrameModel.h"
#include "LayerModel.h"
#include "McpServer.h"
#include "ProposalModel.h"
#include "RedrobCodeRunner.h"
#include "IopaintEngine.h"
#include "redrob_ffi.h"

struct AgentResult
{
    int status = REDROB_ERROR;
    QByteArray payload;
    QString error;
    qulonglong documentEpoch = 0;
};

// A filter run on a worker thread: the engine's status, its change list, and the error text read
// on that same thread (the FFI's last error is thread-local).
// L11: a canvas render done on a worker (redrob_editor_render_rgba_detached).
struct AsyncRenderResult
{
    QImage image;
    quint64 generation = 0;
    qint64 elapsedMs = 0;
};

struct FilterRunResult
{
    int status = REDROB_ERROR;
    QByteArray changes;
    QString error;
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
    // The document's colour mode ("rgb", "grayscale", "indexed") and sample precision ("u8", "u16",
    // "f32"), as the engine names them.
    Q_PROPERTY(QString colorMode READ colorMode NOTIFY documentChanged)
    Q_PROPERTY(QString precision READ precision NOTIFY documentChanged)
    // The active vector node's anchors and their outgoing control points, flat [x,y,...] lists of
    // equal length (I.2 follow-up). Read from the document, so a committed path's handles stay on
    // canvas and an undo removes them -- unlike the pen's in-progress tool state, which is cleared
    // the moment the path is committed.
    Q_PROPERTY(QVariantList activeVectorAnchors READ activeVectorAnchors NOTIFY documentChanged)
    Q_PROPERTY(QVariantList activeVectorHandles READ activeVectorHandles NOTIFY documentChanged)
    Q_PROPERTY(int redoDepth READ redoDepth NOTIFY documentChanged)
    // History panel rows after "Opened", oldest first: the undo steps, then the redo steps in replay
    // order. Each is the engine's label for the step (the command's serde tag, or a group label);
    // an unlabelled step reads as an empty string.
    Q_PROPERTY(QStringList historyLabels READ historyLabels NOTIFY documentChanged)
    // Every engine filter as {kind, defaults}; defaults is null when a parameter has no default.
    Q_PROPERTY(QVariantList filterCatalog READ filterCatalog CONSTANT)
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
    // Filter browser preview: what Apply would produce, or a null image when none is shown.
    Q_PROPERTY(QImage filterPreview READ filterPreview NOTIFY filterPreviewChanged)
    Q_PROPERTY(bool filterPreviewBusy READ filterPreviewBusy NOTIFY filterPreviewChanged)
    Q_PROPERTY(bool hasFilterPreview READ hasFilterPreview NOTIFY filterPreviewChanged)
    Q_PROPERTY(QImage selectionMask READ selectionMask NOTIFY selectionChanged)
    Q_PROPERTY(bool selectionActive READ selectionActive NOTIFY selectionChanged)
    Q_PROPERTY(qreal brushSize READ brushSize WRITE setBrushSize NOTIFY brushSettingsChanged)
    Q_PROPERTY(QColor brushColor READ brushColor WRITE setBrushColor NOTIFY brushColorChanged)
    Q_PROPERTY(qreal brushOpacity READ brushOpacity WRITE setBrushOpacity NOTIFY brushSettingsChanged)
    // M3. Photoshop's brush Flow (Shift+digit), separate from opacity.
    Q_PROPERTY(qreal brushFlow READ brushFlow WRITE setBrushFlow NOTIFY brushSettingsChanged)
    // L3. Dab angle in degrees, and whether the pen's lean direction adds to it.
    Q_PROPERTY(qreal brushAngle READ brushAngle WRITE setBrushAngle NOTIFY brushSettingsChanged)
    Q_PROPERTY(bool brushAngleFromTilt READ brushAngleFromTilt WRITE setBrushAngleFromTilt NOTIFY brushSettingsChanged)
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
    // L9. Mixer brush (wet paint): on while the mixer tool is active; wet/load/mix 0..1.
    Q_PROPERTY(bool brushMixer MEMBER m_brushMixer NOTIFY brushSettingsChanged)
    Q_PROPERTY(double brushMixerWet MEMBER m_brushMixerWet NOTIFY brushSettingsChanged)
    Q_PROPERTY(double brushMixerLoad MEMBER m_brushMixerLoad NOTIFY brushSettingsChanged)
    Q_PROPERTY(double brushMixerMix MEMBER m_brushMixerMix NOTIFY brushSettingsChanged)
    // U2: Photoshop's mixer options. Sample All Layers; Load and Clean the brush after each
    // stroke (both on = every stroke starts with a clean, full brush of the brush colour).
    Q_PROPERTY(bool brushMixerSampleAll MEMBER m_brushMixerSampleAll NOTIFY brushSettingsChanged)
    Q_PROPERTY(bool brushMixerAutoLoad MEMBER m_brushMixerAutoLoad NOTIFY brushSettingsChanged)
    Q_PROPERTY(bool brushMixerAutoClean MEMBER m_brushMixerAutoClean NOTIFY brushSettingsChanged)
    // The paint on the brush now: its colour and how full it is (0..1).
    Q_PROPERTY(QColor mixerWellColor READ mixerWellColor NOTIFY mixerWellChanged)
    Q_PROPERTY(double mixerWellLevel READ mixerWellLevel NOTIFY mixerWellChanged)
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
    Q_PROPERTY(QString modelStatus READ modelStatus NOTIFY liveAgentChanged)
    Q_PROPERTY(bool liveAgentConfigured READ liveAgentConfigured NOTIFY liveAgentChanged)
    // A3: device-flow sign-in to the Redrob console for the in-app agent's key.
    Q_PROPERTY(ConsoleConnection *consoleConnection READ consoleConnection CONSTANT)
    Q_PROPERTY(bool agentBusy READ agentBusy NOTIFY agentBusyChanged)
    Q_PROPERTY(bool filterBusy READ filterBusy NOTIFY filterBusyChanged)
    // S2. Space / Alt held down (outside text fields): QML swaps to the hand / eyedropper.
    Q_PROPERTY(bool spaceHeld READ spaceHeld NOTIFY heldKeysChanged)
    Q_PROPERTY(bool altHeld READ altHeld NOTIFY heldKeysChanged)
    Q_PROPERTY(bool ctrlHeld READ ctrlHeld NOTIFY heldKeysChanged)
    // H8. Layers selected in the Layers panel (Ctrl/Shift-click); always includes the active one.
    Q_PROPERTY(QStringList selectedLayerIds READ selectedLayerIds NOTIFY layerSelectionChanged)
    // H7. Names of the outline fonts found on this machine (filled in by a background scan).
    Q_PROPERTY(QStringList fontFamilies READ fontFamilies NOTIFY fontFamiliesChanged)
    // L2. Guides as {id, vertical, position} maps, for the rulers and the guide overlay.
    Q_PROPERTY(QVariantList guides READ guides NOTIFY guidesChanged)
    // Channels and Paths panels: alpha channels as {id, name, visible, opacity}, stored paths as
    // {id, name, visible}. Thumbnails come from image://layerthumb/channel|path/<id>.
    Q_PROPERTY(QVariantList channels READ channels NOTIFY channelsChanged)
    Q_PROPERTY(QVariantList paths READ paths NOTIFY pathsChanged)
    // L5. CMYK soft proof (View > Proof Colors, Ctrl+Y) through a loaded CMYK ICC profile.
    Q_PROPERTY(bool proofColors READ proofColors WRITE setProofColors NOTIFY proofChanged)
    Q_PROPERTY(bool proofGamutWarning READ proofGamutWarning WRITE setProofGamutWarning NOTIFY proofChanged)
    Q_PROPERTY(QString proofProfileName READ proofProfileName NOTIFY proofChanged)
    // P14. Actions: recording state and how many steps the current recording holds.
    Q_PROPERTY(bool actionRecording READ actionRecording NOTIFY actionChanged)
    Q_PROPERTY(int actionStepCount READ actionStepCount NOTIFY actionChanged)
    // P13. The loopback MCP endpoint for redrob-code. Off at every launch.
    Q_PROPERTY(bool mcpEnabled READ mcpEnabled WRITE setMcpEnabled NOTIFY mcpChanged)
    Q_PROPERTY(QString mcpStatus READ mcpStatus NOTIFY mcpChanged)
    Q_PROPERTY(QString mcpConfigSnippet READ mcpConfigSnippet NOTIFY mcpChanged)
    Q_PROPERTY(RedrobCodeRunner *codeRunner READ codeRunner CONSTANT)
    // AI tools (IOPaint as a local engine): erase, replace, outpaint, remove background, upscale,
    // face restore, click-to-segment. Results land as new layers or as the selection.
    Q_PROPERTY(IopaintEngine *iopaint READ iopaint CONSTANT)
    Q_PROPERTY(QString agentStatus READ agentStatus NOTIFY agentStatusChanged)
    Q_PROPERTY(QString assistantText READ assistantText NOTIFY assistantTextChanged)
    Q_PROPERTY(QString currentFile READ currentFile NOTIFY currentFileChanged)
    Q_PROPERTY(QString formatCapabilities READ formatCapabilities CONSTANT)

public:
    explicit EditorBridge(QObject *parent = nullptr);
    ~EditorBridge() override;
    // P7. Installed on the application (main.cpp) to read pen tilt from tablet events, which Qt
    // Quick's pointer handlers drop. Never consumes an event.
    bool eventFilter(QObject *watched, QEvent *event) override;
    qreal penTiltX() const { return m_penTiltX; }
    qreal penTiltY() const { return m_penTiltY; }

    int documentWidth() const;
    int documentHeight() const;
    qulonglong generation() const;
    bool canUndo() const;
    bool canRedo() const;
    int undoDepth() const;
    QString colorMode() const { return m_colorMode; }
    QString precision() const { return m_precision; }
    // Image > Mode. `palette` is "generate", "web" or "mono" and matters only for "indexed".
    Q_INVOKABLE void convertColorMode(const QString &mode, const QString &palette = QString(),
                                      int maxColors = 256, const QString &dither = QStringLiteral("none"));
    // Image > Precision: "u8", "u16" or "f32".
    Q_INVOKABLE void setDocumentPrecision(const QString &precision);
    QVariantList activeVectorAnchors() const;
    QVariantList activeVectorHandles() const;
    int redoDepth() const;
    QStringList historyLabels() const;
    QVariantList filterCatalog() const;
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
    // Layers panel thumbnail of node `id`, fit inside maxSide. Null for an adjustment layer.
    QImage layerThumbnail(const QString &id, int maxSide) const;
    // Mask / alpha-channel / path thumbnail, opaque greyscale (white = covered). kind: 0 layer
    // mask, 1 alpha channel, 2 path. Null when the source does not exist.
    QImage coverageThumbnail(int kind, const QString &id, int maxSide) const;
    // Composite channel thumbnail of the current picture: "rgb", "red", "green" or "blue".
    QImage compositeChannelThumbnail(const QString &channel, int maxSide) const;
    QVariantList channels() const;
    QVariantList paths() const;
    QImage filterPreview() const { return m_filterPreview; }
    bool filterPreviewBusy() const { return m_filterPreviewBusy; }
    bool hasFilterPreview() const { return !m_filterPreview.isNull(); }
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
    qreal brushFlow() const;
    void setBrushFlow(qreal flow);
    qreal brushAngle() const { return m_brushAngle; }
    void setBrushAngle(qreal degrees);
    bool brushAngleFromTilt() const { return m_brushAngleFromTilt; }
    void setBrushAngleFromTilt(bool on);
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
    // U2: fill the mixer brush with the brush colour, or wipe it clean (Photoshop's Load / Clean).
    Q_INVOKABLE void mixerLoadBrush();
    Q_INVOKABLE void mixerCleanBrush();
    QColor mixerWellColor() const;
    double mixerWellLevel() const;
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
    bool filterBusy() const;
    bool spaceHeld() const;
    bool altHeld() const;
    bool ctrlHeld() const;
    QStringList selectedLayerIds() const;
    QStringList fontFamilies() const;
    QVariantList guides() const;
    bool proofColors() const { return m_proofColors; }
    void setProofColors(bool on);
    bool proofGamutWarning() const { return m_proofGamutWarning; }
    void setProofGamutWarning(bool on);
    QString proofProfileName() const { return m_proofProfileName; }
    // intent: 0 perceptual, 1 relative colorimetric, 2 saturation, 3 absolute.
    Q_INVOKABLE bool loadProofProfile(const QUrl &fileUrl, int intent);
    // L5b: the composite as a CMYK TIFF through the proof profile (embedded), on white.
    Q_INVOKABLE bool exportCmykTiff(const QUrl &fileUrl);
    // U7: File > Export CMYK PSD… -- layers kept, separated through the proof profile.
    Q_INVOKABLE bool exportCmykPsd(const QUrl &fileUrl);
    // L8. Artboards: a group with its own rectangle (children clipped to it, optional background).
    // A transparent `background` (alpha 0) leaves the board transparent. width/height <= 0:
    // the selection's box, else the whole canvas.
    Q_INVOKABLE void newArtboard(int x, int y, int width, int height, const QColor &background);
    // U9: drag an artboard and its contents by whole pixels.
    Q_INVOKABLE void moveArtboard(const QString &id, int dx, int dy);
    // Writes each artboard as <folder>/<name>.png, cropped to its rectangle. Returns the count.
    Q_INVOKABLE int exportArtboards(const QUrl &folderUrl);
    Q_INVOKABLE void addGuide(bool vertical, int position);
    // Channels panel. `fromSelection` seeds the channel from the selection (Photoshop's "Save
    // selection as channel"); otherwise it starts empty. Loading combines with the selection by `mode`.
    Q_INVOKABLE void addChannel(bool fromSelection, const QString &name = {});
    Q_INVOKABLE void removeChannel(const QString &id);
    Q_INVOKABLE void renameChannel(const QString &id, const QString &name);
    Q_INVOKABLE void setChannelVisible(const QString &id, bool visible);
    Q_INVOKABLE void loadChannelSelection(const QString &id, const QString &mode = QStringLiteral("replace"));
    // Paths panel.
    Q_INVOKABLE void pathFromSelection(const QString &name = {});
    Q_INVOKABLE void removePath(const QString &id);
    Q_INVOKABLE void renamePath(const QString &id, const QString &name);
    Q_INVOKABLE void setPathVisible(const QString &id, bool visible);
    Q_INVOKABLE void loadPathSelection(const QString &id, const QString &mode = QStringLiteral("replace"));
    Q_INVOKABLE void strokePath(const QString &id);
    Q_INVOKABLE void moveGuide(const QString &id, int position);
    Q_INVOKABLE void removeGuide(const QString &id);
    // H7. Loads (registers) the font a name resolves to; false when it is not installed.
    Q_INVOKABLE bool ensureFont(const QString &name);
    // M7. Swatch files: .gpl or .aco in, .gpl out. replace=false appends.
    Q_INVOKABLE bool loadSwatches(const QUrl &fileUrl, bool replace);
    Q_INVOKABLE bool saveSwatches(const QUrl &fileUrl);
    // U1. Save pickers ask before replacing a file; this is what they ask about.
    Q_INVOKABLE bool fileExists(const QUrl &fileUrl) const;
    // U3: the active layer's opaque box [x0, y0, x1, y1], or [] when it has none.
    Q_INVOKABLE QVariantList activeLayerBounds() const;
    // U5: replaces the active smart object's smart filters ([{filter: {kind, ...}, visible}]).
    Q_INVOKABLE bool setSmartFilters(const QVariantList &filters);
    bool actionRecording() const;
    int actionStepCount() const;
    // P14. Record every successful edit as a step, save the steps as an action file, play one back
    // as a single undo step.
    Q_INVOKABLE void startActionRecording();
    Q_INVOKABLE void stopActionRecording();
    Q_INVOKABLE bool saveAction(const QUrl &fileUrl, const QString &name);
    Q_INVOKABLE bool playActionFile(const QUrl &fileUrl);
    bool mcpEnabled() const;
    void setMcpEnabled(bool enabled);
    QString mcpStatus() const;
    // The redrob-code config entry for this session's endpoint, token included. Empty when off.
    QString mcpConfigSnippet() const;
    // A2: run a redrob-code task against this window's endpoint (proposals only).
    RedrobCodeRunner *codeRunner() { return &m_codeRunner; }
    IopaintEngine *iopaint() { return m_iopaint.get(); }
    // AI tools: the flattened canvas (straight RGBA8888), or a null image while a filter runs or
    // the projection is stale. `aiSelectionMask` is the Grayscale8 selection, null when none.
    QImage aiSourceImage();
    QImage aiSelectionMask() const;
    // A new full-canvas layer named `name` from an RGBA8888 image the canvas's size.
    bool aiAddLayer(const QImage &rgba, const QString &name);
    // Combine the selection with a Grayscale8 canvas-sized mask; mode as selectRectangle's.
    bool aiSelectMask(const QImage &gray, const QString &mode);
    void aiStatus(const QString &message) { setStatus(message); }
    ConsoleConnection *consoleConnection() { return &m_console; }
    Q_INVOKABLE void runRedrobCodeTask(const QString &task);
    // The Agent chat: redrob-code when it is connected and installed, otherwise the Redrob agent
    // (or the local no-network subset). Both answer in codeRunner.messages.
    Q_INVOKABLE bool sendChatMessage(const QString &text);
    QString agentStatus() const;
    QString assistantText() const;
    QString currentFile() const;
    QString formatCapabilities() const;

    Q_INVOKABLE bool executeCommand(const QString &commandJson);
    Q_INVOKABLE void undo();
    Q_INVOKABLE void redo();
    // Move to history position `stepsDone` (0 = as opened, undoDepth + redoDepth = newest) by
    // repeated undo or redo. Stops at the first step that fails.
    Q_INVOKABLE void jumpToHistory(int stepsDone);
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
                                 const QString &parentId = {}, int siblingIndex = -1,
                                 qreal boxWidth = -1.0,
                                 const QString &align = QStringLiteral("left"),
                                 const QString &fontFamily = QStringLiteral("font8x8 Basic Latin"),
                                 const QString &fontId = QStringLiteral("font8x8-basic-0.3.1"));
    // boxWidth < 0 is point text; otherwise lines wrap at that width (paragraph text, P10).
    Q_INVOKABLE void setTextContent(
        const QString &id, const QString &text, qreal originX, qreal originY, qreal fontSize,
        const QColor &color, const QString &fontFamily = QStringLiteral("font8x8 Basic Latin"),
        const QString &fontId = QStringLiteral("font8x8-basic-0.3.1"), qreal boxWidth = -1.0,
        const QString &align = QStringLiteral("left"));
    Q_INVOKABLE void addVectorRectangle(const QString &name, qreal x, qreal y, qreal width,
                                        qreal height, const QColor &fill, const QColor &stroke,
                                        qreal strokeWidth, const QString &parentId = {},
                                        int siblingIndex = -1);
    // Pen tool: a straight-segment vector path from clicked anchors (flat [x0,y0,...] list).
    Q_INVOKABLE void addVectorPath(const QVariantList &points, bool closed, const QString &name = {});
    // Pen with handles (I.2): `handles` holds each anchor's OUTGOING control point and must be the
    // same length as `anchors`. A handle on its own anchor is a corner.
    Q_INVOKABLE void addVectorPathBezier(const QVariantList &anchors, const QVariantList &handles,
                                         bool closed, const QString &name = {});
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
    Q_INVOKABLE void duplicateLayer(const QString &id);
    Q_INVOKABLE void mergeDown(const QString &id);
    // File > New (H4). Replaces the document; the caller asks about unsaved work first.
    Q_INVOKABLE bool newDocument(int width, int height, const QColor &background);
    // H5. Edit > Copy / Cut / Paste, through the system clipboard. Paste adds a new layer.
    Q_INVOKABLE bool copySelection();
    Q_INVOKABLE bool cutSelection();
    Q_INVOKABLE bool pasteClipboard();
    // H8. mode 0 = plain click, 1 = Ctrl-click (toggle), 2 = Shift-click (range).
    Q_INVOKABLE void selectLayer(const QString &id, int mode);
    Q_INVOKABLE void groupSelectedLayers();
    Q_INVOKABLE void deleteSelectedLayers();
    Q_INVOKABLE void toggleClippingMask();
    // M10. Edit > Content-Aware Fill (Shift+F5): PatchMatch fill of the selection, on the worker.
    Q_INVOKABLE void contentAwareFill();
    // M11. Link (or unlink) the selected layers so they move together.
    Q_INVOKABLE void linkSelectedLayers(bool link);
    // L4. Smart objects: transforms re-render from the original; painting needs rasterize.
    Q_INVOKABLE void convertToSmartObject(const QString &id);
    Q_INVOKABLE void rasterizeSmartObject(const QString &id);
    // L6. Blend If: each range is {black_low, black_high, white_low, white_high}.
    Q_INVOKABLE void setLayerBlendIf(const QString &id, const QVariantMap &thisLayer, const QVariantMap &underlying);
    Q_INVOKABLE void clearLayerBlendIf(const QString &id);
    // M5. Edit > Stroke; location is "inside", "center" or "outside".
    Q_INVOKABLE void strokeSelection(int width, const QColor &color, const QString &location);
    // M6. Select > Color Range; range is sampled / shadows / midtones / highlights.
    Q_INVOKABLE void selectColorRange(const QColor &color, int fuzziness, const QString &range,
                                      const QString &mode);
    // M2. Layer locks.
    Q_INVOKABLE void setLayerLocks(const QString &id, bool transparent, bool pixels, bool position);
    Q_INVOKABLE void mergeVisible();
    Q_INVOKABLE void flattenImage(const QColor &background);
    Q_INVOKABLE void setActiveLayer(const QString &id);
    Q_INVOKABLE void renameLayer(const QString &id, const QString &name);
    Q_INVOKABLE void setLayerOpacity(const QString &id, qreal opacity);
    Q_INVOKABLE void setLayerVisibility(const QString &id, bool visible);
    // Illustrator's Show All (Ctrl+Alt+3): makes every hidden node visible.
    Q_INVOKABLE void showAllLayers();
    // Where a node sits among its siblings: {parentId, siblingIndex (0 = bottom), siblingCount}.
    // Empty for an unknown id. Feeds the arrange keys (Ctrl+] / Ctrl+[ and their Shift forms).
    Q_INVOKABLE QVariantMap layerPlacement(const QString &id) const;
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
    // A1: Image > Crop to Selection and Edit > Clear outside selection. Both refuse with a status
    // line when nothing is selected.
    Q_INVOKABLE void cropToSelection();
    Q_INVOKABLE void clearOutsideSelection();
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
    // L7: rigid = Photoshop's Puppet Warp (as-rigid-as-possible) instead of the thin-plate bend.
    Q_INVOKABLE void nPointTransform(const QVariantList &srcPts, const QVariantList &dstPts,
                                     const QString &sampling, bool rigid = false);
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
    // UI-1 filter browser: apply `kind` with an explicit parameter object (the engine validates).
    Q_INVOKABLE void applyFilterParams(const QString &kind, const QVariantMap &params);
    // P8b: stop the filter running in the background; nothing is committed.
    Q_INVOKABLE void cancelFilter();
    // P11: the same filter as a non-destructive adjustment node above the active node.
    Q_INVOKABLE void addAdjustmentNode(const QString &kind, const QVariantMap &params);
    Q_INVOKABLE void setAdjustmentFilter(const QString &id, const QString &kind,
                                         const QVariantMap &params);
    // Start a preview of these parameters off the GUI thread; the result lands in filterPreview.
    Q_INVOKABLE void previewFilterParams(const QString &kind, const QVariantMap &params);
    // Drop the shown preview and any preview still running.
    Q_INVOKABLE void clearFilterPreview();
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
    void filterPreviewChanged();
    void selectionChanged();
    void brushSettingsChanged();
    void onionSkinChanged();
    void brushPresetsChanged();
    void paletteChanged();
    void brushColorChanged();
    void statusMessageChanged();
    void agentBusyChanged();
    void filterBusyChanged();
    void heldKeysChanged();
    void layerSelectionChanged();
    void fontFamiliesChanged();
    void guidesChanged();
    void channelsChanged();
    void pathsChanged();
    void mixerWellChanged();
    void proofChanged();
    void actionChanged();
    void mcpChanged();
    void agentStatusChanged();
    void liveAgentChanged();
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
    bool startFilterRun(const QByteArray &json);
    void finishFilterRun(const FilterRunResult &result);
    bool refuseWhileFilterRuns(const QString &what);
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
    // P14. The recording in progress, and the filter command waiting for its worker to succeed
    // before it is recorded (a rejected or cancelled filter is not a step).
    static constexpr qsizetype kMaxActionSteps = 4096;
    static constexpr qint64 kMaxActionFileBytes = 16 * 1024 * 1024;
    bool m_actionRecording = false;
    QJsonArray m_actionSteps;
    QJsonObject m_pendingFilterCommand;
    void recordActionStep(const QJsonObject &command);
    // P13. Its tools/call handler is handleMcpToolCall, which only ever queues proposals.
    McpServer m_mcp;
    RedrobCodeRunner m_codeRunner;
    std::unique_ptr<IopaintEngine> m_iopaint;
    ConsoleConnection m_console;
    QJsonObject mcpServerEntry() const;
    QString m_mcpStatus = QStringLiteral("Off");
    QJsonObject handleMcpToolCall(const QString &name, const QJsonObject &arguments);
    QFutureWatcher<AgentResult> m_agentWatcher;
    QFutureWatcher<FilterRunResult> m_filterWatcher;
    // L11. Once one render takes longer than kAsyncRenderMs the canvas renders on a worker:
    // refresh() keeps the last picture and asks for a new one, and the GUI stays responsive.
    // It goes back to inline rendering when renders are fast again.
    QFutureWatcher<AsyncRenderResult> m_renderWatcher;
    bool m_asyncRender = false;
    bool m_renderAgain = false;
    quint64 m_renderGeneration = 0;
    void startAsyncRender();
    void finishAsyncRender();
    QTimer m_refreshRetryTimer;
    QTimer m_playbackTimer;
    // S1. Live strokes: moves are coalesced and painted at most once per frame by this timer.
    QTimer m_liveStrokeTimer;
    bool m_liveStroke = false;
    QJsonArray m_livePending;
    void flushLiveStroke();
    bool refreshLiveRender();
    QJsonObject strokeCommand(const QJsonArray &points) const;
    QImage m_renderImage;
    QImage m_filterPreview;
    bool m_filterPreviewBusy = false;
    // Bumped by every new preview request and by clearFilterPreview(); a finished preview whose
    // ticket is not the latest is dropped, which is how a running preview is cancelled.
    quint64 m_filterPreviewTicket = 0;
    QImage m_selectionMask;
    QJsonArray m_strokePoints;
    int m_width = 0;
    int m_height = 0;
    qulonglong m_generation = 0;
    qulonglong m_documentEpoch = 0;
    bool m_canUndo = false;
    bool m_canRedo = false;
    int m_undoDepth = 0;
    QString m_colorMode;
    QString m_precision;
    QVariantList m_activeVectorAnchors;
    QVariantList m_activeVectorHandles;
    int m_redoDepth = 0;
    QStringList m_historyLabels;
    QVariantList m_filterCatalog;
    bool m_strokeActive = false;
    // P7. The pen's last reported tilt, in degrees. Qt Quick's pointer handlers do not carry tilt,
    // so it is read from the raw tablet events (eventFilter) and attached to each stroke point.
    // A mouse event resets it, so a mouse stroke after a pen stroke does not inherit a lean.
    qreal m_penTiltX = 0.0;
    qreal m_penTiltY = 0.0;
    // S2. Photoshop's spring-loaded keys, read in eventFilter.
    bool m_spaceHeld = false;
    bool m_altHeld = false;
    bool m_ctrlHeld = false;
    // H5. Where the last copy came from, so pasting it back lands in place.
    QPoint m_clipOrigin;
    QSize m_clipSize;
    // H8. Selected layers; always holds the active node.
    QStringList m_selectedLayers;
    void pruneLayerSelection();
    QStringList selectedRoots() const;
    bool runAsOneStep(const QJsonArray &commands, const QString &done);
    // H7. Font index: lower-cased name -> file, built off the GUI thread.
    struct FontIndex {
        QHash<QString, QString> paths;
        QStringList names;
    };
    QFutureWatcher<FontIndex> m_fontScanWatcher;
    QHash<QString, QString> m_fontPaths;
    QStringList m_fontFamilies;
    QVariantList m_guides;
    QVariantList m_channels;
    QVariantList m_paths;
    std::shared_ptr<RedrobCmykProof> m_proof;
    bool m_proofColors = false;
    bool m_proofGamutWarning = false;
    QString m_proofProfileName;
    QByteArray m_proofProfileBytes;
    QImage m_proofedImage;
    void updateProofImage();
    QSet<QString> m_registeredFontFiles;
    void startFontScan();
    void ensureDocumentFonts();
    static bool knownFontId(const QString &fontId);
    static bool textCharactersAllowed(const QString &text, const QString &fontId);
    void setHeldKey(bool &held, bool value);
    bool m_strokeTruncated = false;
    bool m_selectionActive = false;
    bool m_looping = false;
    bool m_playing = false;
    bool m_liveAgentConfigured = false;
    bool m_agentBusy = false;
    bool m_filterBusy = false;
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
    // M3: Photoshop's Flow, 0.01..1. 1 sends no flow, so a default stroke's command is unchanged.
    double m_brushFlowSetting = 1.0;
    double m_brushAngle = 0.0;
    bool m_brushAngleFromTilt = false;
    bool m_brushSmudge = false;
    bool m_brushMixer = false;
    double m_brushMixerWet = 0.5;
    double m_brushMixerLoad = 0.9;
    double m_brushMixerMix = 0.5;
    bool m_brushMixerSampleAll = false;
    bool m_brushMixerAutoLoad = true;
    bool m_brushMixerAutoClean = true;
    // U2: the paint on the mixer brush, rgba 0..255 + level. m_mixerWellSet is false until a
    // mixer stroke or the Load/Clean buttons set it.
    bool m_mixerWellSet = false;
    double m_mixerWell[4] = {0, 0, 0, 0};
    double m_mixerWellLevel = 1.0;
    // The engine's last reported well, so a state read only adopts a NEW stroke's end state and
    // does not undo a Load/Clean pressed since.
    QJsonValue m_engineMixerWell;
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
