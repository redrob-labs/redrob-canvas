// SPDX-License-Identifier: GPL-3.0-or-later
#include <QDebug>
#include <QEventLoop>
#include <QFile>
#include <QGuiApplication>
#include <QIcon>
#include <QInputDevice>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickWindow>
#include <QTemporaryDir>
#include <QTimer>

#include <limits>

#include "CanvasItem.h"
#include "EditorBridge.h"
#include "FrameIdAllocator.h"
#include "redrob_ffi.h"

namespace {
bool frameIdAllocatorIsValid()
{
    int repeatedCalls = 0;
    const auto repeatedCollision = FrameIdAllocator::allocate(
        QVector<quint32>{7, 8}, [&repeatedCalls] {
            ++repeatedCalls;
            return quint32{7};
        });
    const auto wrappedProbe = FrameIdAllocator::allocate(
        QVector<quint32>{std::numeric_limits<quint32>::max(), 0}, [] {
            return std::numeric_limits<quint32>::max();
        });
    const auto zeroCandidate = FrameIdAllocator::allocate(
        QVector<quint32>{std::numeric_limits<quint32>::max()}, [] {
            return std::numeric_limits<quint32>::max();
        });

    QVector<quint32> fullTimeline;
    fullTimeline.reserve(FrameIdAllocator::kMaxFrameCount);
    for (int id = 0; id < FrameIdAllocator::kMaxFrameCount; ++id)
        fullTimeline.push_back(static_cast<quint32>(id));
    bool fullGeneratorCalled = false;
    const auto fullResult = FrameIdAllocator::allocate(fullTimeline, [&fullGeneratorCalled] {
        fullGeneratorCalled = true;
        return quint32{0};
    });

    return repeatedCollision && *repeatedCollision == 9
        && repeatedCalls == FrameIdAllocator::kRandomAttempts
        && wrappedProbe && *wrappedProbe == 1 && zeroCandidate && *zeroCandidate == 0
        && !fullResult && !fullGeneratorCalled;
}

bool hasSelectedPixel(const QImage &mask)
{
    for (int y = 0; y < mask.height(); ++y) {
        const uchar *row = mask.constScanLine(y);
        for (int x = 0; x < mask.width(); ++x) {
            if (row[x] != 0)
                return true;
        }
    }
    return false;
}

bool rasterActionsMatch(QObject *root, bool expected)
{
    static const QStringList actionNames{
        QStringLiteral("brushToolAction"),
        QStringLiteral("fillToolAction"),
        QStringLiteral("gradientToolAction"),
        QStringLiteral("transformToolAction"),
        QStringLiteral("flipHorizontalAction"),
        QStringLiteral("flipVerticalAction"),
        QStringLiteral("rotateCounterclockwiseAction"),
        QStringLiteral("rotateClockwiseAction"),
        QStringLiteral("affineTransformAction"),
        QStringLiteral("invertFilterAction"),
        QStringLiteral("grayscaleFilterAction"),
        QStringLiteral("clearLayerAction"),
        QStringLiteral("brightnessContrastAction"),
        QStringLiteral("gaussianBlurAction"),
        QStringLiteral("thresholdAction"),
        QStringLiteral("posterizeAction"),
        QStringLiteral("levelsAction"),
        QStringLiteral("hueSaturationAction"),
        QStringLiteral("boxBlurAction"),
        QStringLiteral("sharpenAction"),
    };
    for (const QString &name : actionNames) {
        QObject *action = root ? root->findChild<QObject *>(name) : nullptr;
        if (!action || action->property("enabled").toBool() != expected)
            return false;
    }
    return true;
}

bool hierarchyAndMaskBridgeIsValid(EditorBridge &editor, QObject *root)
{
    const QString rasterId = editor.activeLayerId();
    const qulonglong initialGeneration = editor.generation();
    editor.fill(QColor(220, 70, 55, 255));
    editor.addGroup(QStringLiteral("Smoke group"));
    const QString groupId = editor.activeLayerId();
    editor.moveNode(rasterId, groupId, 0);
    editor.selectRectangle(1, 0, 1, 1, QStringLiteral("replace"));
    const QVariant groupArgument(groupId);
    if (!root || !QMetaObject::invokeMethod(root, "maskNodeFromSelection",
                                            Q_ARG(QVariant, groupArgument))) {
        qWarning() << "mask smoke: QML invocation failed";
        return false;
    }

    QAbstractItemModel *model = editor.layers();
    if (!model || model->rowCount() != 2 || editor.generation() != initialGeneration + 5
        || editor.activeNodeKind() != QStringLiteral("group") || editor.activeNodeCanEditRaster()
        || !editor.activeNodeHasMask() || !rasterActionsMatch(root, false)) {
        qWarning() << "mask smoke: initial projection failed" << (model ? model->rowCount() : -1)
                   << editor.generation() << initialGeneration << editor.activeNodeKind()
                   << editor.activeNodeCanEditRaster() << editor.activeNodeHasMask()
                   << rasterActionsMatch(root, false);
        return false;
    }
    const QModelIndex groupIndex = model->index(0, 0);
    const QModelIndex rasterIndex = model->index(1, 0);
    const bool modelValid = model->data(groupIndex, LayerModel::IdRole).toString() == groupId
        && model->data(groupIndex, LayerModel::KindRole).toString() == QStringLiteral("group")
        && model->data(groupIndex, LayerModel::HasMaskRole).toBool()
        && model->data(groupIndex, LayerModel::MaskEnabledRole).toBool()
        && model->data(groupIndex, LayerModel::SiblingCountRole).toInt() == 1
        && model->data(groupIndex, LayerModel::IsTopRole).toBool()
        && model->data(groupIndex, LayerModel::IsBottomRole).toBool()
        && model->data(rasterIndex, LayerModel::IdRole).toString() == rasterId
        && model->data(rasterIndex, LayerModel::ParentIdRole).toString() == groupId
        && model->data(rasterIndex, LayerModel::DepthRole).toInt() == 1
        && model->data(rasterIndex, LayerModel::SiblingCountRole).toInt() == 1;
    const QImage maskedRender = editor.renderImage();
    const bool maskedRenderValid = !maskedRender.isNull()
        && maskedRender.pixelColor(0, 0).alpha() == 0
        && maskedRender.pixelColor(1, 0) == QColor(220, 70, 55, 255);
    if (!modelValid || !maskedRenderValid) {
        qWarning() << "mask smoke: model or render failed" << modelValid << maskedRenderValid;
        return false;
    }

    editor.setRasterMaskEnabled(groupId, false);
    const bool disabledMaskProjected = !model->data(model->index(0, 0), LayerModel::MaskEnabledRole).toBool()
        && editor.renderImage().pixelColor(0, 0) == QColor(220, 70, 55, 255);
    editor.setRasterMaskEnabled(groupId, true);
    editor.setActiveLayer(rasterId);
    const bool rasterCapabilityProjected = editor.activeNodeCanEditRaster()
        && rasterActionsMatch(root, true);
    if (!disabledMaskProjected || !rasterCapabilityProjected) {
        qWarning() << "mask smoke: disabled mask or raster capability failed"
                   << disabledMaskProjected << rasterCapabilityProjected;
        return false;
    }

    for (int index = 0; index < 8; ++index)
        editor.undo();
    return model->rowCount() == 1 && editor.activeLayerId() == rasterId
        && editor.activeNodeKind() == QStringLiteral("raster") && editor.activeNodeCanEditRaster()
        && editor.renderImage().pixelColor(0, 0).alpha() == 0;
}

bool semanticBridgeIsValid(EditorBridge &editor, QObject *root)
{
    QObject *addTextAction = root ? root->findChild<QObject *>(QStringLiteral("addTextNodeAction")) : nullptr;
    QObject *addVectorAction = root ? root->findChild<QObject *>(QStringLiteral("addVectorNodeAction")) : nullptr;
    QObject *textEditor = root ? root->findChild<QObject *>(QStringLiteral("textSemanticEditor")) : nullptr;
    QObject *vectorEditor = root ? root->findChild<QObject *>(QStringLiteral("vectorRectangleEditor")) : nullptr;
    QObject *warning = root ? root->findChild<QObject *>(QStringLiteral("rasterizeSemanticWarning")) : nullptr;
    if (!addTextAction || !addVectorAction || !textEditor || !vectorEditor || !warning) {
        qWarning() << "semantic smoke: QML object names missing";
        return false;
    }

    const qulonglong initialGeneration = editor.generation();
    const bool initialCanUndo = editor.canUndo();
    QJsonArray expensiveCommands;
    expensiveCommands.append(QJsonObject{{QStringLiteral("type"), QStringLiteral("move_to")},
                                         {QStringLiteral("x"), 0.0},
                                         {QStringLiteral("y"), 0.0}});
    for (int index = 0; index < 4000; ++index) {
        expensiveCommands.append(QJsonObject{
            {QStringLiteral("type"), QStringLiteral("line_to")},
            {QStringLiteral("x"), index % 2 == 0 ? editor.documentWidth() : 0},
            {QStringLiteral("y"), (index % qMax(1, editor.documentHeight())) + 0.5}});
    }
    const QJsonObject expensiveCommand{
        {QStringLiteral("type"), QStringLiteral("add_vector_node")},
        {QStringLiteral("id"), QStringLiteral("00000000-0000-0000-0000-00000000ffff")},
        {QStringLiteral("name"), QStringLiteral("Over budget")},
        {QStringLiteral("parent"), QJsonValue(QJsonValue::Null)},
        {QStringLiteral("sibling_index"), 1},
        {QStringLiteral("vector"), QJsonObject{{QStringLiteral("paths"), QJsonArray{QJsonObject{
             {QStringLiteral("commands"), expensiveCommands},
             {QStringLiteral("fill"), QJsonObject{{QStringLiteral("r"), 1}, {QStringLiteral("g"), 2},
                                                     {QStringLiteral("b"), 3}, {QStringLiteral("a"), 255}}},
             {QStringLiteral("fill_rule"), QStringLiteral("non_zero")}}}}}}};
    if (editor.executeCommand(QString::fromUtf8(QJsonDocument(expensiveCommand).toJson(QJsonDocument::Compact)))
        || editor.generation() != initialGeneration || editor.canUndo() != initialCanUndo) {
        qWarning() << "semantic smoke: over-budget command mutated generation/history";
        return false;
    }

    editor.addTextNode(QStringLiteral("Smoke text"), QStringLiteral("A"), 7.25, 9.5, 13.5,
                       QColor(19, 73, 211, 187));
    const QString textId = editor.activeLayerId();
    QAbstractItemModel *model = editor.layers();
    if (!model || model->rowCount() != 2 || editor.activeNodeKind() != QStringLiteral("text")
        || !editor.activeNodeCanEditText() || editor.activeNodeCanEditRaster()
        || !editor.activeNodeCanRasterize()) {
        qWarning() << "semantic smoke: text capability projection failed";
        return false;
    }
    const QModelIndex textIndex = model->index(0, 0);
    auto *layerModel = qobject_cast<LayerModel *>(model);
    if (!layerModel || !model->data(textIndex, LayerModel::CanEditTextRole).toBool()
        || !model->data(textIndex, LayerModel::CanRasterizeRole).toBool()
        || !model->data(textIndex, LayerModel::IsSemanticRole).toBool()
        || model->data(textIndex, LayerModel::SemanticPreviewRole).toString() != QStringLiteral("A")
        || model->data(textIndex, LayerModel::SemanticTextRole).toString() != QStringLiteral("A")
        || model->data(textIndex, LayerModel::SemanticFontIdRole).toString()
            != QStringLiteral("font8x8-basic-0.3.1")
        || model->data(textIndex, LayerModel::SemanticFontFamilyRole).toString()
            != QStringLiteral("font8x8 Basic Latin")
        || !qFuzzyCompare(model->data(textIndex, LayerModel::SemanticFontSizeRole).toDouble(), 13.5)
        || !qFuzzyCompare(model->data(textIndex, LayerModel::SemanticOriginXRole).toDouble(), 7.25)
        || !qFuzzyCompare(model->data(textIndex, LayerModel::SemanticOriginYRole).toDouble(), 9.5)
        || model->data(textIndex, LayerModel::SemanticColorRole).value<QColor>()
            != QColor(19, 73, 211, 187)) {
        qWarning() << "semantic smoke: text roles failed";
        return false;
    }
    const QVariantMap textSource = layerModel->semanticSource(textId);
    if (textSource.value(QStringLiteral("kind")).toString() != QStringLiteral("text")
        || textSource.value(QStringLiteral("text")).toString() != QStringLiteral("A")
        || textSource.value(QStringLiteral("fontId")).toString()
            != QStringLiteral("font8x8-basic-0.3.1")
        || textSource.value(QStringLiteral("fontFamily")).toString()
            != QStringLiteral("font8x8 Basic Latin")
        || !qFuzzyCompare(textSource.value(QStringLiteral("fontSize")).toDouble(), 13.5)
        || !qFuzzyCompare(textSource.value(QStringLiteral("originX")).toDouble(), 7.25)
        || !qFuzzyCompare(textSource.value(QStringLiteral("originY")).toDouble(), 9.5)
        || textSource.value(QStringLiteral("color")).value<QColor>()
            != QColor(19, 73, 211, 187)) {
        qWarning() << "semantic smoke: immutable text source accessor failed";
        return false;
    }
    const QImage firstText = editor.renderImage();
    editor.setTextContent(
        textId, QStringLiteral("AB"),
        model->data(textIndex, LayerModel::SemanticOriginXRole).toDouble(),
        model->data(textIndex, LayerModel::SemanticOriginYRole).toDouble(),
        model->data(textIndex, LayerModel::SemanticFontSizeRole).toDouble(),
        model->data(textIndex, LayerModel::SemanticColorRole).value<QColor>(),
        model->data(textIndex, LayerModel::SemanticFontFamilyRole).toString(),
        model->data(textIndex, LayerModel::SemanticFontIdRole).toString());
    const QModelIndex editedTextIndex = model->index(0, 0);
    if (editor.renderImage() == firstText
        || !qFuzzyCompare(model->data(editedTextIndex, LayerModel::SemanticOriginXRole).toDouble(), 7.25)
        || !qFuzzyCompare(model->data(editedTextIndex, LayerModel::SemanticOriginYRole).toDouble(), 9.5)
        || !qFuzzyCompare(model->data(editedTextIndex, LayerModel::SemanticFontSizeRole).toDouble(), 13.5)
        || model->data(editedTextIndex, LayerModel::SemanticColorRole).value<QColor>()
            != QColor(19, 73, 211, 187)) {
        qWarning() << "semantic smoke: text edit did not change render";
        return false;
    }

    editor.addVectorRectangle(QStringLiteral("Smoke vector"), 50, 30, 40, 24,
                              QColor(20, 170, 220, 230), QColor(250, 250, 250, 255), 2);
    const QString vectorId = editor.activeLayerId();
    if (model->rowCount() != 3 || editor.activeNodeKind() != QStringLiteral("vector")
        || !editor.activeNodeCanEditVector() || !editor.activeNodeCanRasterize()
        || !model->data(model->index(0, 0), LayerModel::SemanticRectangleRecognizedRole).toBool()
        || !qFuzzyCompare(model->data(model->index(0, 0), LayerModel::SemanticRectangleXRole).toDouble(), 50.0)
        || !qFuzzyCompare(model->data(model->index(0, 0), LayerModel::SemanticRectangleYRole).toDouble(), 30.0)
        || !qFuzzyCompare(model->data(model->index(0, 0), LayerModel::SemanticRectangleWidthRole).toDouble(), 40.0)
        || !qFuzzyCompare(model->data(model->index(0, 0), LayerModel::SemanticRectangleHeightRole).toDouble(), 24.0)
        || model->data(model->index(0, 0), LayerModel::SemanticRectangleFillRole).value<QColor>()
            != QColor(20, 170, 220, 230)
        || model->data(model->index(0, 0), LayerModel::SemanticRectangleStrokeRole).value<QColor>()
            != QColor(250, 250, 250, 255)
        || !qFuzzyCompare(model->data(model->index(0, 0), LayerModel::SemanticRectangleStrokeWidthRole).toDouble(), 2.0)) {
        qWarning() << "semantic smoke: vector capability projection failed";
        return false;
    }
    const QVariantMap vectorSource = layerModel->semanticSource(vectorId);
    if (vectorSource.value(QStringLiteral("kind")).toString() != QStringLiteral("vector")
        || !vectorSource.value(QStringLiteral("rectangleRecognized")).toBool()
        || !qFuzzyCompare(vectorSource.value(QStringLiteral("x")).toDouble(), 50.0)
        || !qFuzzyCompare(vectorSource.value(QStringLiteral("y")).toDouble(), 30.0)
        || !qFuzzyCompare(vectorSource.value(QStringLiteral("width")).toDouble(), 40.0)
        || !qFuzzyCompare(vectorSource.value(QStringLiteral("height")).toDouble(), 24.0)
        || vectorSource.value(QStringLiteral("fill")).value<QColor>() != QColor(20, 170, 220, 230)
        || vectorSource.value(QStringLiteral("stroke")).value<QColor>() != QColor(250, 250, 250, 255)
        || !qFuzzyCompare(vectorSource.value(QStringLiteral("strokeWidth")).toDouble(), 2.0)) {
        qWarning() << "semantic smoke: immutable vector source accessor failed";
        return false;
    }

    const QString fillOnlyId = QStringLiteral("00000000-0000-0000-0000-00000000fffe");
    const QJsonObject fillOnlyCommand{
        {QStringLiteral("type"), QStringLiteral("add_vector_node")},
        {QStringLiteral("id"), fillOnlyId},
        {QStringLiteral("name"), QStringLiteral("Fill-only rectangle")},
        {QStringLiteral("parent"), QJsonValue(QJsonValue::Null)},
        {QStringLiteral("sibling_index"), 3},
        {QStringLiteral("vector"), QJsonObject{{QStringLiteral("paths"), QJsonArray{QJsonObject{
             {QStringLiteral("commands"), QJsonArray{
                  QJsonObject{{QStringLiteral("type"), QStringLiteral("move_to")},
                              {QStringLiteral("x"), 1.0}, {QStringLiteral("y"), 1.0}},
                  QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")},
                              {QStringLiteral("x"), 5.0}, {QStringLiteral("y"), 1.0}},
                  QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")},
                              {QStringLiteral("x"), 5.0}, {QStringLiteral("y"), 4.0}},
                  QJsonObject{{QStringLiteral("type"), QStringLiteral("line_to")},
                              {QStringLiteral("x"), 1.0}, {QStringLiteral("y"), 4.0}},
                  QJsonObject{{QStringLiteral("type"), QStringLiteral("close")}}}},
             {QStringLiteral("fill"), QJsonObject{{QStringLiteral("r"), 1},
                                                   {QStringLiteral("g"), 2},
                                                   {QStringLiteral("b"), 3},
                                                   {QStringLiteral("a"), 255}}},
             {QStringLiteral("fill_rule"), QStringLiteral("non_zero")}}}}}}};
    if (!editor.executeCommand(
            QString::fromUtf8(QJsonDocument(fillOnlyCommand).toJson(QJsonDocument::Compact)))
        || editor.activeLayerId() != fillOnlyId || editor.activeNodeCanEditVector()
        || layerModel->semanticSource(fillOnlyId)
               .value(QStringLiteral("rectangleRecognized"))
               .toBool()) {
        qWarning() << "semantic smoke: optional-paint rectangle was advertised as editable";
        return false;
    }
    editor.deleteLayer(fillOnlyId);
    editor.setActiveLayer(vectorId);

    const QImage semanticVisual = editor.renderImage();
    editor.rasterizeSemanticNode(vectorId);
    const QImage rasterVisual = editor.renderImage();
    if (semanticVisual != rasterVisual || editor.activeLayerId() != vectorId
        || editor.activeNodeKind() != QStringLiteral("raster") || !editor.activeNodeCanEditRaster()) {
        qWarning() << "semantic smoke: same-id raster visual equivalence failed";
        return false;
    }
    const qulonglong rasterGeneration = editor.generation();
    editor.setVectorRectangle(vectorId, 1, 1, 2, 2, QColor(Qt::red), QColor(Qt::black), 1);
    if (editor.generation() != rasterGeneration || !editor.statusMessage().contains(QStringLiteral("rejected"))) {
        qWarning() << "semantic smoke: invalid edit was not transactional";
        return false;
    }
    editor.undo();
    if (editor.activeNodeKind() != QStringLiteral("vector") || editor.renderImage() != semanticVisual) {
        qWarning() << "semantic smoke: undo failed";
        return false;
    }
    editor.redo();
    if (editor.activeNodeKind() != QStringLiteral("raster") || editor.renderImage() != rasterVisual) {
        qWarning() << "semantic smoke: redo failed";
        return false;
    }

    QTemporaryDir recoveryDirectory;
    const QString recoveryPath = recoveryDirectory.filePath(QStringLiteral("recovery.rrg"));
    if (!recoveryDirectory.isValid() || !editor.saveFile(QUrl::fromLocalFile(recoveryPath))) {
        qWarning() << "semantic smoke: recovery fixture save failed";
        return false;
    }
    const qulonglong beforeRecoveryUndo = editor.generation();
    editor.injectProjectionFailureForSmokeTest();
    editor.undo();
    if (editor.generation() <= beforeRecoveryUndo || editor.activeNodeKind() != QStringLiteral("vector")) {
        qWarning() << "semantic smoke: stale undo recovery failed" << editor.statusMessage();
        return false;
    }
    editor.injectProjectionFailureForSmokeTest();
    if (!editor.openFile(QUrl::fromLocalFile(recoveryPath))
        || editor.activeNodeKind() != QStringLiteral("raster") || editor.renderImage() != rasterVisual) {
        qWarning() << "semantic smoke: stale open recovery failed" << editor.statusMessage();
        return false;
    }

    editor.setActiveLayer(textId);
    editor.rasterizeSemanticNode(textId);
    editor.addFrame();
    QAbstractItemModel *frames = editor.frames();
    const quint32 blankFrame = frames->data(frames->index(1, 0), FrameModel::FrameIdRole).toUInt();
    editor.setCurrentFrame(blankFrame);
    const QImage blank = editor.renderImage();
    bool blankOnly = !blank.isNull();
    for (int y = 0; blankOnly && y < blank.height(); ++y) {
        for (int x = 0; x < blank.width(); ++x) {
            if (blank.pixelColor(x, y).alpha() != 0) {
                blankOnly = false;
                break;
            }
        }
    }
    if (!blankOnly) {
        qWarning() << "semantic smoke: non-current frame was not blank";
        return false;
    }

    editor.setCurrentFrame(0);
    editor.removeFrame(blankFrame);
    editor.deleteLayer(vectorId);
    editor.deleteLayer(textId);
    return model->rowCount() == 1 && editor.activeNodeKind() == QStringLiteral("raster");
}

bool timelineBridgeIsValid(EditorBridge &editor, QObject *root)
{
    QAbstractItemModel *frames = editor.frames();
    QObject *add = root ? root->findChild<QObject *>(QStringLiteral("addFrameAction")) : nullptr;
    QObject *duplicate = root ? root->findChild<QObject *>(QStringLiteral("duplicateFrameAction")) : nullptr;
    QObject *remove = root ? root->findChild<QObject *>(QStringLiteral("deleteFrameAction")) : nullptr;
    QObject *moveLeft = root ? root->findChild<QObject *>(QStringLiteral("moveFrameLeftAction")) : nullptr;
    QObject *moveRight = root ? root->findChild<QObject *>(QStringLiteral("moveFrameRightAction")) : nullptr;
    if (!frames || frames->rowCount() != 1 || editor.currentFrame() != 0 || !add || !duplicate
        || !remove || !moveLeft || !moveRight || remove->property("enabled").toBool()
        || moveLeft->property("enabled").toBool() || moveRight->property("enabled").toBool()) {
        qWarning() << "timeline smoke initial" << (frames ? frames->rowCount() : -1)
                   << editor.currentFrame() << (remove ? remove->property("enabled") : QVariant{});
        return false;
    }

    editor.fill(QColor(210, 40, 30, 255));
    if (!QMetaObject::invokeMethod(add, "clicked") || frames->rowCount() != 2) {
        qWarning() << "timeline smoke add" << frames->rowCount();
        return false;
    }
    const quint32 blank = frames->data(frames->index(1, 0), FrameModel::FrameIdRole).toUInt();
    const bool undoBeforeNavigation = editor.canUndo();
    const bool redoBeforeNavigation = editor.canRedo();
    const qulonglong generationBeforeNavigation = editor.generation();
    if (!editor.setCurrentFrame(blank) || editor.generation() <= generationBeforeNavigation
        || editor.canUndo() != undoBeforeNavigation || editor.canRedo() != redoBeforeNavigation
        || editor.renderImage().pixelColor(0, 0).alpha() != 0) {
        qWarning() << "timeline smoke navigation" << editor.statusMessage() << editor.generation()
                   << generationBeforeNavigation << editor.renderImage().pixelColor(0, 0);
        return false;
    }

    if (!editor.setCurrentFrame(0) || !QMetaObject::invokeMethod(duplicate, "clicked")
        || frames->rowCount() != 3) {
        qWarning() << "timeline smoke duplicate" << editor.statusMessage() << frames->rowCount();
        return false;
    }
    const quint32 copied = frames->data(frames->index(1, 0), FrameModel::FrameIdRole).toUInt();
    if (!editor.setCurrentFrame(copied)
        || editor.renderImage().pixelColor(0, 0) != QColor(210, 40, 30, 255)) {
        qWarning() << "timeline smoke copied render" << editor.renderImage().pixelColor(0, 0);
        return false;
    }

    editor.setPlaybackRange(copied, blank);
    if (!QMetaObject::invokeMethod(remove, "clicked") || frames->rowCount() != 2
        || editor.currentFrame() != blank || editor.rangeStart() != blank
        || editor.rangeEnd() != blank) {
        qWarning() << "timeline smoke remove" << frames->rowCount() << editor.currentFrame()
                   << editor.rangeStart() << editor.rangeEnd() << blank << editor.statusMessage();
        return false;
    }
    editor.setTimelineFps(24.0);
    editor.setLooping(true);
    if (qAbs(editor.fps() - 24.0) > 0.001 || !editor.looping()
        || frames->data(frames->index(0, 0), FrameModel::DurationRole).toInt() != 42
        || frames->data(frames->index(1, 0), FrameModel::DurationRole).toInt() != 42
        || moveRight->property("enabled").toBool()
        || !moveLeft->property("enabled").toBool()) {
        qWarning() << "timeline smoke metadata" << editor.fps() << editor.looping()
                   << editor.currentFrameIndex() << moveLeft->property("enabled")
                   << moveRight->property("enabled");
        return false;
    }

    if (!QMetaObject::invokeMethod(moveLeft, "clicked")
        || frames->data(frames->index(0, 0), FrameModel::FrameIdRole).toUInt() != blank
        || moveLeft->property("enabled").toBool()
        || !moveRight->property("enabled").toBool()
        || !QMetaObject::invokeMethod(moveRight, "clicked")
        || frames->data(frames->index(1, 0), FrameModel::FrameIdRole).toUInt() != blank
        || moveRight->property("enabled").toBool()) {
        qWarning() << "timeline smoke reorder" << editor.currentFrameIndex()
                   << moveLeft->property("enabled") << moveRight->property("enabled");
        return false;
    }

    editor.setPlaybackRange(0, blank);
    editor.setLooping(true);
    if (!editor.setCurrentFrame(blank))
        return false;
    const bool undoBeforeLoopTick = editor.canUndo();
    const bool redoBeforeLoopTick = editor.canRedo();
    if (!editor.setPlaying(true))
        return false;
    const qulonglong beforeLoopTick = editor.generation();
    if (!editor.advancePlayback() || editor.currentFrame() != 0 || !editor.playing()
        || editor.generation() != beforeLoopTick + 1
        || editor.canUndo() != undoBeforeLoopTick || editor.canRedo() != redoBeforeLoopTick) {
        qWarning() << "timeline smoke loop playback" << editor.playing() << editor.currentFrame()
                   << editor.generation() << beforeLoopTick << editor.statusMessage();
        return false;
    }

    if (!editor.setCurrentFrame(blank))
        return false;
    QAbstractItemModel *proposals = editor.proposals();
    const int proposalsBefore = proposals ? proposals->rowCount() : -1;
    const qreal fpsBeforeProposal = editor.fps();
    const quint32 frameBeforeProposal = editor.currentFrame();
    const QString currentFileBeforeProposal = editor.currentFile();
    const QString proposalId = editor.enqueueProposal(
        QStringLiteral("Current playback proposal"), QStringLiteral("Applies atomically"),
        QStringLiteral(R"({"type":"set_timeline_fps","fps":12})"));
    const qulonglong proposalGeneration = editor.generation();
    editor.applyProposal(proposalId);
    if (proposalId.isEmpty() || editor.playing()
        || editor.generation() != proposalGeneration + 1
        || qAbs(editor.fps() - 12.0) > 0.001 || editor.currentFrame() != frameBeforeProposal
        || editor.currentFile() != currentFileBeforeProposal || !editor.canUndo()
        || !proposals || proposals->rowCount() != proposalsBefore
        || !editor.statusMessage().contains(QStringLiteral("Approved"))) {
        qWarning() << "timeline smoke current proposal" << editor.playing()
                   << editor.generation() << proposalGeneration << editor.fps()
                   << editor.currentFrame() << frameBeforeProposal
                   << (proposals ? proposals->rowCount() : -1) << proposalsBefore
                   << editor.statusMessage();
        return false;
    }
    const qulonglong committedProposalGeneration = editor.generation();
    QEventLoop stoppedTimerWait;
    QTimer::singleShot(150, &stoppedTimerWait, &QEventLoop::quit);
    stoppedTimerWait.exec();
    if (editor.generation() != committedProposalGeneration || editor.playing()) {
        qWarning() << "timeline smoke committed proposal timer"
                   << editor.generation() << committedProposalGeneration << editor.playing();
        return false;
    }

    if (!editor.setPlaying(true))
        return false;
    const QString staleProposalId = editor.enqueueProposal(
        QStringLiteral("Stale playback proposal"), QStringLiteral("Must remain inert"),
        QStringLiteral(R"({"type":"set_timeline_fps","fps":18})"));
    if (staleProposalId.isEmpty() || !editor.setCurrentFrame(editor.currentFrame()))
        return false;
    const qulonglong staleGeneration = editor.generation();
    const bool staleUndo = editor.canUndo();
    const bool staleRedo = editor.canRedo();
    const quint32 staleFrame = editor.currentFrame();
    const qreal staleFps = editor.fps();
    const QString staleCurrentFile = editor.currentFile();
    const QImage staleRender = editor.renderImage();
    const int staleProposalCount = proposals->rowCount();
    editor.applyProposal(staleProposalId);
    if (!editor.playing() || editor.generation() != staleGeneration
        || editor.currentFrame() != staleFrame || qAbs(editor.fps() - staleFps) > 0.001
        || editor.canUndo() != staleUndo || editor.canRedo() != staleRedo
        || editor.currentFile() != staleCurrentFile || editor.renderImage() != staleRender
        || proposals->rowCount() != staleProposalCount
        || !editor.statusMessage().contains(QStringLiteral("stale"))) {
        qWarning() << "timeline smoke stale proposal" << editor.playing()
                   << editor.generation() << staleGeneration << editor.currentFrame() << staleFrame
                   << editor.fps() << staleFps << proposals->rowCount() << staleProposalCount
                   << editor.statusMessage();
        return false;
    }
    editor.rejectProposal(staleProposalId);

    const QString rejectedCommand =
        QStringLiteral(R"({"type":"add_frame","id":0,"index":1})");
    const QString invalidProposalId = editor.enqueueProposal(
        QStringLiteral("Invalid playback proposal"), QStringLiteral("Must remain inert"),
        rejectedCommand);
    const qulonglong invalidGeneration = editor.generation();
    const bool invalidUndo = editor.canUndo();
    const bool invalidRedo = editor.canRedo();
    const quint32 invalidFrame = editor.currentFrame();
    const qreal invalidFps = editor.fps();
    const QString invalidCurrentFile = editor.currentFile();
    const QImage invalidRender = editor.renderImage();
    const int invalidProposalCount = proposals->rowCount();
    editor.applyProposal(invalidProposalId);
    if (invalidProposalId.isEmpty() || !editor.playing()
        || editor.generation() != invalidGeneration || editor.currentFrame() != invalidFrame
        || qAbs(editor.fps() - invalidFps) > 0.001 || editor.canUndo() != invalidUndo
        || editor.canRedo() != invalidRedo || editor.currentFile() != invalidCurrentFile
        || editor.renderImage() != invalidRender
        || proposals->rowCount() != invalidProposalCount) {
        qWarning() << "timeline smoke invalid proposal" << editor.playing()
                   << editor.generation() << invalidGeneration << editor.currentFrame()
                   << invalidFrame << proposals->rowCount() << invalidProposalCount
                   << editor.statusMessage();
        return false;
    }
    editor.rejectProposal(invalidProposalId);

    const qulonglong rejectedGeneration = editor.generation();
    const bool rejectedUndo = editor.canUndo();
    const bool rejectedRedo = editor.canRedo();
    const quint32 rejectedFrame = editor.currentFrame();
    const qreal rejectedFps = editor.fps();
    const QString rejectedCurrentFile = editor.currentFile();
    const QImage rejectedRender = editor.renderImage();
    if (editor.executeCommand(rejectedCommand) || !editor.playing()
        || editor.generation() != rejectedGeneration || editor.currentFrame() != rejectedFrame
        || qAbs(editor.fps() - rejectedFps) > 0.001 || editor.canUndo() != rejectedUndo
        || editor.canRedo() != rejectedRedo || editor.currentFile() != rejectedCurrentFile
        || editor.renderImage() != rejectedRender) {
        qWarning() << "timeline smoke rejected command" << editor.playing()
                   << editor.generation() << rejectedGeneration << editor.currentFrame()
                   << rejectedFrame << editor.statusMessage();
        return false;
    }

    const qulonglong unavailableRedoGeneration = editor.generation();
    const QImage unavailableRedoRender = editor.renderImage();
    editor.redo();
    if (!editor.playing() || editor.generation() != unavailableRedoGeneration
        || editor.currentFrame() != rejectedFrame || editor.canUndo() != rejectedUndo
        || editor.canRedo() != rejectedRedo || editor.currentFile() != rejectedCurrentFile
        || editor.renderImage() != unavailableRedoRender) {
        qWarning() << "timeline smoke unavailable redo" << editor.playing()
                   << editor.generation() << unavailableRedoGeneration << editor.statusMessage();
        return false;
    }

    const qulonglong beforePlaybackUndo = editor.generation();
    editor.undo();
    if (editor.playing() || editor.generation() != beforePlaybackUndo + 1
        || editor.currentFrame() != frameBeforeProposal
        || qAbs(editor.fps() - fpsBeforeProposal) > 0.001 || !editor.canRedo()) {
        qWarning() << "timeline smoke playback undo" << editor.playing()
                   << editor.generation() << beforePlaybackUndo << editor.currentFrame()
                   << frameBeforeProposal << editor.fps() << fpsBeforeProposal
                   << editor.statusMessage();
        return false;
    }

    editor.setLooping(false);
    const qulonglong beforeTick = editor.generation();
    if (!editor.setPlaying(true))
        return false;
    QEventLoop playbackWait;
    QTimer::singleShot(150, &playbackWait, &QEventLoop::quit);
    playbackWait.exec();
    if (editor.playing()
        || editor.currentFrame() != blank || editor.generation() < beforeTick + 2) {
        qWarning() << "timeline smoke playback" << editor.playing() << editor.currentFrame()
                   << blank << editor.generation() << beforeTick << editor.statusMessage();
        return false;
    }

    editor.removeFrame(blank);
    editor.setTimelineFps(10.0);
    editor.setLooping(false);
    editor.setCurrentFrame(0);
    editor.clearActiveLayer();
    return frames->rowCount() == 1 && editor.currentFrame() == 0
        && !remove->property("enabled").toBool() && !editor.playing()
        && editor.renderImage().pixelColor(0, 0).alpha() == 0;
}

bool genericFormatBridgeIsValid(EditorBridge &editor, QObject *root)
{
    const QStringList fileActions{QStringLiteral("openProjectAction"),
                                  QStringLiteral("importFileAction"),
                                  QStringLiteral("saveProjectAction"),
                                  QStringLiteral("saveProjectAsAction"),
                                  QStringLiteral("exportCurrentFrameAction"),
                                  QStringLiteral("exportOptionsDialog"),
                                  QStringLiteral("allowLossControl"),
                                  QStringLiteral("jpegQualityControl")};
    for (const QString &name : fileActions) {
        if (!root || !root->findChild<QObject *>(name)) {
            qWarning() << "format smoke: missing QML control" << name;
            return false;
        }
    }

    QJsonParseError capabilitiesError;
    const QJsonDocument capabilities =
        QJsonDocument::fromJson(editor.formatCapabilities().toUtf8(), &capabilitiesError);
    if (capabilitiesError.error != QJsonParseError::NoError || !capabilities.isObject()
        || capabilities.object().value(QStringLiteral("formats")).toArray().size() != 6
        || capabilities.object()
               .value(QStringLiteral("adapters"))
               .toObject()
               .value(QStringLiteral("gegl"))
               .toObject()
               .value(QStringLiteral("ready"))
               .toBool(true)
        || capabilities.object()
               .value(QStringLiteral("adapters"))
               .toObject()
               .value(QStringLiteral("krita"))
               .toObject()
               .value(QStringLiteral("ready"))
               .toBool(true)) {
        qWarning() << "format smoke: capability projection is not truthful";
        return false;
    }

    QTemporaryDir directory;
    if (!directory.isValid())
        return false;
    const QString sourceSvg = directory.filePath(QStringLiteral("source.svg"));
    QFile source(sourceSvg);
    static constexpr char svg[] =
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"1\">"
        "<rect width=\"2\" height=\"1\" fill=\"#00000000\"/></svg>";
    if (!source.open(QIODevice::WriteOnly)
        || source.write(svg, static_cast<qint64>(sizeof(svg) - 1))
            != static_cast<qint64>(sizeof(svg) - 1))
        return false;
    source.close();
    if (!editor.importFile(QUrl::fromLocalFile(sourceSvg)) || !editor.currentFile().isEmpty()
        || editor.documentWidth() != 2 || editor.documentHeight() != 1) {
        qWarning() << "format smoke: limited SVG import failed" << editor.statusMessage();
        return false;
    }
    editor.rasterizeSemanticNode(editor.activeLayerId());
    editor.fill(QColor(220, 30, 20, 255));
    editor.addFrame();
    QAbstractItemModel *frames = editor.frames();
    if (!frames || frames->rowCount() != 2)
        return false;
    const quint32 alternate =
        frames->data(frames->index(1, 0), FrameModel::FrameIdRole).toUInt();
    if (!editor.setCurrentFrame(alternate))
        return false;
    editor.fill(QColor(20, 210, 40, 128));
    if (!editor.setCurrentFrame(0))
        return false;

    const QString projectPath = directory.filePath(QStringLiteral("animation.rrg"));
    if (!editor.saveProject(QUrl::fromLocalFile(projectPath))
        || editor.currentFile() != projectPath) {
        qWarning() << "format smoke: project save identity failed" << editor.statusMessage();
        return false;
    }

    struct ExportRoute {
        const char *format;
        const char *extension;
    };
    static constexpr ExportRoute routes[] = {{"png", "png"},
                                              {"webp", "webp"},
                                              {"jpeg", "jpg"},
                                              {"ora", "ora"},
                                              {"svg", "svg"}};
    const quint32 currentBefore = editor.currentFrame();
    const qulonglong generationBefore = editor.generation();
    const bool undoBefore = editor.canUndo();
    const bool redoBefore = editor.canRedo();
    QStringList exportedPaths;
    for (const ExportRoute &route : routes) {
        const QString format = QString::fromLatin1(route.format);
        const QString path = directory.filePath(
            QStringLiteral("frame.%1").arg(QString::fromLatin1(route.extension)));
        if (!editor.exportFile(QUrl::fromLocalFile(path), format, true, alternate, 88,
                               QColor(250, 240, 230, 255))
            || !QFile::exists(path) || editor.currentFile() != projectPath
            || editor.currentFrame() != currentBefore || editor.generation() != generationBefore
            || editor.canUndo() != undoBefore || editor.canRedo() != redoBefore
            || !editor.statusMessage().contains(format.toUpper())) {
            qWarning() << "format smoke: export failed/nonmutating contract violated" << format
                       << editor.statusMessage() << editor.currentFrame() << editor.generation();
            return false;
        }
        exportedPaths.append(path);
    }

    const QString unknown = directory.filePath(QStringLiteral("unknown.bin"));
    if (editor.openFile(QUrl::fromLocalFile(unknown))) {
        qWarning() << "format smoke: unknown extension was accepted";
        return false;
    }

    const QString oversizedPath = directory.filePath(QStringLiteral("oversized.png"));
    QFile oversized(oversizedPath);
    if (!oversized.open(QIODevice::WriteOnly)
        || !oversized.resize(64LL * 1024LL * 1024LL + 1))
        return false;
    oversized.close();
    if (editor.importFile(QUrl::fromLocalFile(oversizedPath))
        || editor.currentFile() != projectPath || editor.currentFrame() != currentBefore
        || editor.generation() != generationBefore
        || !editor.statusMessage().contains(QStringLiteral("64 MiB input limit"))) {
        qWarning() << "format smoke: bounded native read failed" << editor.statusMessage();
        return false;
    }

    const QString mismatchPath = directory.filePath(QStringLiteral("renamed-png.rrg"));
    if (!QFile::copy(exportedPaths.constFirst(), mismatchPath) || !editor.setPlaying(true))
        return false;
    const bool mismatchPlaying = editor.playing();
    const qulonglong mismatchGeneration = editor.generation();
    const quint32 mismatchFrame = editor.currentFrame();
    const QString mismatchProject = editor.currentFile();
    const bool mismatchUndo = editor.canUndo();
    const bool mismatchRedo = editor.canRedo();
    const QImage mismatchRender = editor.renderImage();
    const int mismatchFrameCount = editor.frameCount();
    const int mismatchWidth = editor.documentWidth();
    const int mismatchHeight = editor.documentHeight();
    if (editor.openProject(QUrl::fromLocalFile(mismatchPath))
        || editor.playing() != mismatchPlaying || editor.generation() != mismatchGeneration
        || editor.currentFrame() != mismatchFrame || editor.currentFile() != mismatchProject
        || editor.canUndo() != mismatchUndo || editor.canRedo() != mismatchRedo
        || editor.renderImage() != mismatchRender || editor.frameCount() != mismatchFrameCount
        || editor.documentWidth() != mismatchWidth || editor.documentHeight() != mismatchHeight
        || !editor.statusMessage().contains(QStringLiteral("does not match"),
                                            Qt::CaseInsensitive)) {
        qWarning() << "format smoke: failed import mutated active playback state"
                   << editor.playing() << editor.generation() << mismatchGeneration
                   << editor.currentFrame() << mismatchFrame << editor.currentFile()
                   << editor.statusMessage();
        return false;
    }

    for (const QString &path : exportedPaths) {
        if (!editor.importFile(QUrl::fromLocalFile(path)) || !editor.currentFile().isEmpty()
            || editor.frameCount() != 1 || editor.documentWidth() != 2
            || editor.documentHeight() != 1) {
            qWarning() << "format smoke: interchange import failed" << path
                       << editor.statusMessage();
            return false;
        }
    }

    if (!editor.setPlaying(true) || !editor.openProject(QUrl::fromLocalFile(projectPath))
        || editor.playing() || editor.currentFile() != projectPath || editor.frameCount() != 2) {
        qWarning() << "format smoke: project open/playback identity failed" << editor.statusMessage();
        return false;
    }
    return true;
}

bool pressureNormalizationIsValid(QObject *root)
{
    QObject *handler = root ? root->findChild<QObject *>(QStringLiteral("canvasPointer")) : nullptr;
    if (!handler)
        return false;
    const auto normalized = [handler](double pressure, QInputDevice::DeviceType type,
                                      double *result) {
        QVariant returned;
        const QVariant pressureArgument(pressure);
        const QVariant typeArgument(static_cast<int>(type));
        if (!QMetaObject::invokeMethod(handler, "normalizedPressure",
                                       Q_RETURN_ARG(QVariant, returned),
                                       Q_ARG(QVariant, pressureArgument),
                                       Q_ARG(QVariant, typeArgument)))
            return false;
        *result = returned.toDouble();
        return true;
    };

    double penZero = -1.0;
    double penFraction = -1.0;
    double mouseFallback = -1.0;
    return normalized(0.0, QInputDevice::DeviceType::Stylus, &penZero)
        && normalized(0.35, QInputDevice::DeviceType::Stylus, &penFraction)
        && normalized(0.0, QInputDevice::DeviceType::Mouse, &mouseFallback)
        && qAbs(penZero) < 0.000001 && qAbs(penFraction - 0.35) < 0.000001
        && qAbs(mouseFallback - 1.0) < 0.000001;
}
} // namespace

int main(int argc, char *argv[])
{
    QGuiApplication application(argc, argv);
    application.setApplicationName(QStringLiteral("Redrob Graphics"));
    application.setOrganizationName(QStringLiteral("Redrob"));
    application.setWindowIcon(QIcon(QStringLiteral(":/icons/redrob.svg")));

    const uint32_t runtimeAbi = redrob_ffi_abi_version();
    if (runtimeAbi != REDROB_FFI_ABI_VERSION) {
        qCritical().nospace() << "redrob-ffi ABI mismatch: native expects "
                              << REDROB_FFI_ABI_VERSION << ", runtime provides " << runtimeAbi;
        return EXIT_FAILURE;
    }

    qmlRegisterType<CanvasItem>("Redrob.Graphics", 1, 0, "CanvasItem");

    EditorBridge editor;
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("editor"), &editor);
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed,
                     &application, [] { QCoreApplication::exit(EXIT_FAILURE); },
                     Qt::QueuedConnection);
    engine.load(QUrl(QStringLiteral("qrc:/qml/Main.qml")));

    if (qEnvironmentVariableIsSet("REDROB_SMOKE_TEST")) {
        // Exercise pressure normalization plus the complete selection command,
        // snapshot, copy, cache, and overlay path before deterministic capture.
        QObject *root = engine.rootObjects().isEmpty() ? nullptr : engine.rootObjects().constFirst();
        const bool allocatorValid = frameIdAllocatorIsValid();
        const bool pressureValid = root && pressureNormalizationIsValid(root);
        const bool hierarchyValid = root && hierarchyAndMaskBridgeIsValid(editor, root);
        const bool semanticValid = root && hierarchyValid && semanticBridgeIsValid(editor, root);
        const bool timelineValid = root && semanticValid && timelineBridgeIsValid(editor, root);
        const bool formatValid = root && timelineValid && genericFormatBridgeIsValid(editor, root);
        if (!root || !allocatorValid || !pressureValid || !hierarchyValid || !semanticValid
            || !timelineValid || !formatValid) {
            qCritical() << "Native smoke bridge assertion failed" << allocatorValid << pressureValid
                        << hierarchyValid << semanticValid << timelineValid << formatValid;
            return EXIT_FAILURE;
        }
        const qulonglong initialGeneration = editor.generation();
        const int expectedWidth = editor.documentWidth();
        const int expectedHeight = editor.documentHeight();
        editor.selectEllipse(expectedWidth * 0.22, expectedHeight * 0.18,
                             expectedWidth * 0.56, expectedHeight * 0.64,
                             QStringLiteral("replace"));
        const QImage mask = editor.selectionMask();
        const bool smokeStateValid = editor.generation() > initialGeneration
            && editor.selectionActive() && mask.width() == expectedWidth
            && mask.height() == expectedHeight && hasSelectedPixel(mask);
        if (!smokeStateValid) {
            qCritical().nospace() << "Native smoke selection assertion failed: generation "
                                  << initialGeneration << " -> " << editor.generation()
                                  << ", active=" << editor.selectionActive() << ", mask="
                                  << mask.width() << 'x' << mask.height() << ", document="
                                  << expectedWidth << 'x' << expectedHeight;
            return EXIT_FAILURE;
        }
        QTimer::singleShot(1200, &application, [&application, &engine] {
            if (engine.rootObjects().isEmpty()) {
                application.exit(EXIT_FAILURE);
                return;
            }
            const QString screenshotPath = qEnvironmentVariable("REDROB_SCREENSHOT_PATH");
            if (!screenshotPath.isEmpty()) {
                auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().constFirst());
                if (!window || !window->grabWindow().save(screenshotPath)) {
                    application.exit(EXIT_FAILURE);
                    return;
                }
            }
            application.exit(EXIT_SUCCESS);
        });
    }
    return application.exec();
}
