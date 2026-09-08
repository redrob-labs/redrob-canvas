// SPDX-License-Identifier: GPL-3.0-or-later
#include "LayerModel.h"

#include <QJsonArray>
#include <QJsonObject>

#include <algorithm>

namespace {
QColor colorFromJson(const QJsonValue &value)
{
    const auto object = value.toObject();
    if (object.isEmpty())
        return {};
    return QColor(object.value(QStringLiteral("r")).toInt(),
                  object.value(QStringLiteral("g")).toInt(),
                  object.value(QStringLiteral("b")).toInt(),
                  object.value(QStringLiteral("a")).toInt(255));
}
}

LayerModel::LayerModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

int LayerModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_layers.size();
}

QVariant LayerModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() < 0 || index.row() >= m_layers.size())
        return {};
    const auto &layer = m_layers.at(index.row());
    switch (role) {
    case IdRole: return layer.id;
    case NameRole: return layer.name;
    case VisibleRole: return layer.visible;
    case OpacityRole: return layer.opacity;
    case BlendModeRole: return layer.blendMode;
    case ActiveRole: return layer.active;
    case CoreIndexRole: return layer.coreIndex;
    case KindRole: return layer.kind;
    case ParentIdRole: return layer.parentId;
    case DepthRole: return layer.depth;
    case SiblingIndexRole: return layer.siblingIndex;
    case SiblingCountRole: return layer.siblingCount;
    case IsTopRole: return layer.siblingIndex + 1 == layer.siblingCount;
    case IsBottomRole: return layer.siblingIndex == 0;
    case HasChildrenRole: return layer.hasChildren;
    case HasMaskRole: return layer.hasMask;
    case MaskEnabledRole: return layer.maskEnabled;
    case CanEditRasterRole: return layer.canEditRaster;
    case CanEditTextRole: return layer.canEditText;
    case CanEditVectorRole: return layer.canEditVector;
    case CanRasterizeRole: return layer.canRasterize;
    case IsSemanticRole: return layer.isSemantic;
    case SemanticPreviewRole: return layer.semanticPreview;
    case SemanticTextRole: return layer.semanticText;
    case SemanticPreviewTruncatedRole: return layer.semanticPreviewTruncated;
    case SemanticPathCountRole: return layer.semanticPathCount;
    case SemanticCommandCountRole: return layer.semanticCommandCount;
    case SemanticFontIdRole: return layer.semanticFontId;
    case SemanticFontFamilyRole: return layer.semanticFontFamily;
    case SemanticFontSizeRole: return layer.semanticFontSize;
    case SemanticOriginXRole: return layer.semanticOriginX;
    case SemanticOriginYRole: return layer.semanticOriginY;
    case SemanticColorRole: return layer.semanticColor;
    case SemanticRectangleRecognizedRole: return layer.semanticRectangleRecognized;
    case SemanticRectangleXRole: return layer.semanticRectangleX;
    case SemanticRectangleYRole: return layer.semanticRectangleY;
    case SemanticRectangleWidthRole: return layer.semanticRectangleWidth;
    case SemanticRectangleHeightRole: return layer.semanticRectangleHeight;
    case SemanticRectangleFillRole: return layer.semanticRectangleFill;
    case SemanticRectangleStrokeRole: return layer.semanticRectangleStroke;
    case SemanticRectangleStrokeWidthRole: return layer.semanticRectangleStrokeWidth;
    default: return {};
    }
}

QHash<int, QByteArray> LayerModel::roleNames() const
{
    return {
        {IdRole, "layerId"}, {NameRole, "layerName"}, {VisibleRole, "layerVisible"},
        {OpacityRole, "layerOpacity"}, {BlendModeRole, "blendMode"},
        {ActiveRole, "activeLayer"}, {CoreIndexRole, "coreIndex"},
        {KindRole, "nodeKind"}, {ParentIdRole, "parentId"}, {DepthRole, "nodeDepth"},
        {SiblingIndexRole, "siblingIndex"}, {SiblingCountRole, "siblingCount"},
        {IsTopRole, "isTopSibling"}, {IsBottomRole, "isBottomSibling"},
        {HasChildrenRole, "hasChildren"},
        {HasMaskRole, "hasMask"}, {MaskEnabledRole, "maskEnabled"},
        {CanEditRasterRole, "canEditRaster"},
        {CanEditTextRole, "canEditText"},
        {CanEditVectorRole, "canEditVector"},
        {CanRasterizeRole, "canRasterize"},
        {IsSemanticRole, "isSemantic"},
        {SemanticPreviewRole, "semanticPreview"},
        {SemanticTextRole, "semanticTextSource"},
        {SemanticPreviewTruncatedRole, "semanticPreviewTruncated"},
        {SemanticPathCountRole, "semanticPathCount"},
        {SemanticCommandCountRole, "semanticCommandCount"},
        {SemanticFontIdRole, "semanticFontId"},
        {SemanticFontFamilyRole, "semanticFontFamily"},
        {SemanticFontSizeRole, "semanticFontSize"},
        {SemanticOriginXRole, "semanticOriginX"},
        {SemanticOriginYRole, "semanticOriginY"},
        {SemanticColorRole, "semanticColor"},
        {SemanticRectangleRecognizedRole, "semanticRectangleRecognized"},
        {SemanticRectangleXRole, "semanticRectangleX"},
        {SemanticRectangleYRole, "semanticRectangleY"},
        {SemanticRectangleWidthRole, "semanticRectangleWidth"},
        {SemanticRectangleHeightRole, "semanticRectangleHeight"},
        {SemanticRectangleFillRole, "semanticRectangleFill"},
        {SemanticRectangleStrokeRole, "semanticRectangleStroke"},
        {SemanticRectangleStrokeWidthRole, "semanticRectangleStrokeWidth"},
    };
}

void LayerModel::replaceFromSnapshot(const QJsonObject &snapshot)
{
    QVector<LayerRow> rows;
    m_activeLayerId = snapshot.value(QStringLiteral("active_node_id")).toString();
    if (m_activeLayerId.isEmpty())
        m_activeLayerId = snapshot.value(QStringLiteral("active_layer_id")).toString();
    const auto layers = snapshot.value(QStringLiteral("layers")).toArray();
    rows.reserve(layers.size());
    QHash<QString, int> siblingCounts;
    for (const auto &value : layers) {
        const auto layer = value.toObject();
        LayerRow row;
        row.id = layer.value(QStringLiteral("id")).toString();
        row.name = layer.value(QStringLiteral("name")).toString();
        row.visible = layer.value(QStringLiteral("visible")).toBool();
        row.opacity = layer.value(QStringLiteral("opacity")).toDouble();
        row.blendMode = layer.value(QStringLiteral("blend_mode")).toString();
        row.kind = layer.value(QStringLiteral("kind")).toString();
        row.parentId = layer.value(QStringLiteral("parent_id")).toString();
        row.depth = layer.value(QStringLiteral("depth")).toInt();
        row.hasMask = layer.value(QStringLiteral("has_mask")).toBool();
        row.maskEnabled = layer.value(QStringLiteral("mask_enabled")).toBool();
        const auto group = layer.value(QStringLiteral("group")).toObject();
        row.hasChildren = group.value(QStringLiteral("child_count")).toInt() > 0;
        const auto capabilities = layer.value(QStringLiteral("capabilities")).toObject();
        row.canEditRaster = capabilities.value(QStringLiteral("can_edit_raster")).toBool();
        row.canEditText = capabilities.value(QStringLiteral("can_edit_text")).toBool();
        row.canEditVector = capabilities.value(QStringLiteral("can_edit_vector")).toBool();
        row.canRasterize = capabilities.value(QStringLiteral("can_rasterize")).toBool();
        row.isSemantic = capabilities.value(QStringLiteral("is_semantic")).toBool();
        const auto semantic = layer.value(QStringLiteral("semantic")).toObject();
        row.semanticPreview = semantic.value(QStringLiteral("preview")).toString();
        row.semanticText = semantic.value(QStringLiteral("text")).toString();
        row.semanticPreviewTruncated = semantic.value(QStringLiteral("preview_truncated")).toBool();
        row.semanticPathCount = semantic.value(QStringLiteral("path_count")).toInt();
        row.semanticCommandCount = semantic.value(QStringLiteral("command_count")).toInt();
        row.semanticFontId = semantic.value(QStringLiteral("font_id")).toString();
        row.semanticFontFamily = semantic.value(QStringLiteral("font_family")).toString();
        row.semanticFontSize = semantic.value(QStringLiteral("font_size")).toDouble();
        row.semanticOriginX = semantic.value(QStringLiteral("origin_x")).toDouble();
        row.semanticOriginY = semantic.value(QStringLiteral("origin_y")).toDouble();
        row.semanticColor = colorFromJson(semantic.value(QStringLiteral("color")));
        row.semanticRectangleRecognized = semantic.value(QStringLiteral("rectangle_recognized")).toBool();
        const auto rectangle = semantic.value(QStringLiteral("rectangle")).toObject();
        row.semanticRectangleX = rectangle.value(QStringLiteral("x")).toDouble();
        row.semanticRectangleY = rectangle.value(QStringLiteral("y")).toDouble();
        row.semanticRectangleWidth = rectangle.value(QStringLiteral("width")).toDouble();
        row.semanticRectangleHeight = rectangle.value(QStringLiteral("height")).toDouble();
        row.semanticRectangleFill = colorFromJson(rectangle.value(QStringLiteral("fill")));
        row.semanticRectangleStroke = colorFromJson(rectangle.value(QStringLiteral("stroke_color")));
        row.semanticRectangleStrokeWidth = rectangle.value(QStringLiteral("stroke_width")).toDouble();
        row.active = row.id == m_activeLayerId;
        row.coreIndex = layer.value(QStringLiteral("index")).toInt();
        row.siblingIndex = siblingCounts[row.parentId]++;
        rows.push_back(std::move(row));
    }
    for (auto &row : rows)
        row.siblingCount = siblingCounts.value(row.parentId);
    // Core order is canonical bottom-to-top post-order. Reversing it presents
    // each group before its top-first descendants while retaining coreIndex.
    std::reverse(rows.begin(), rows.end());
    beginResetModel();
    m_layers = std::move(rows);
    endResetModel();
}

QString LayerModel::activeLayerId() const
{
    return m_activeLayerId;
}

int LayerModel::layerCount() const
{
    return m_layers.size();
}

QString LayerModel::layerIdAt(int row) const
{
    return row >= 0 && row < m_layers.size() ? m_layers.at(row).id : QString{};
}

QString LayerModel::activeNodeKind() const
{
    for (const auto &layer : m_layers) {
        if (layer.active)
            return layer.kind;
    }
    return {};
}

bool LayerModel::activeNodeCanEditRaster() const
{
    for (const auto &layer : m_layers) {
        if (layer.active)
            return layer.canEditRaster;
    }
    return false;
}

bool LayerModel::activeNodeCanEditText() const
{
    for (const auto &layer : m_layers) {
        if (layer.active)
            return layer.canEditText;
    }
    return false;
}

bool LayerModel::activeNodeCanEditVector() const
{
    for (const auto &layer : m_layers) {
        if (layer.active)
            return layer.canEditVector;
    }
    return false;
}

bool LayerModel::activeNodeCanRasterize() const
{
    for (const auto &layer : m_layers) {
        if (layer.active)
            return layer.canRasterize;
    }
    return false;
}

bool LayerModel::activeNodeHasMask() const
{
    for (const auto &layer : m_layers) {
        if (layer.active)
            return layer.hasMask;
    }
    return false;
}

int LayerModel::siblingCount(const QString &parentId) const
{
    int count = 0;
    for (const auto &layer : m_layers) {
        if (layer.parentId == parentId)
            ++count;
    }
    return count;
}

QVariantMap LayerModel::semanticSource(const QString &id) const
{
    for (const auto &layer : m_layers) {
        if (layer.id != id || !layer.isSemantic)
            continue;

        QVariantMap source{{QStringLiteral("kind"), layer.kind}};
        if (layer.kind == QStringLiteral("text")) {
            source.insert(QStringLiteral("text"), layer.semanticText);
            source.insert(QStringLiteral("fontId"), layer.semanticFontId);
            source.insert(QStringLiteral("fontFamily"), layer.semanticFontFamily);
            source.insert(QStringLiteral("fontSize"), layer.semanticFontSize);
            source.insert(QStringLiteral("originX"), layer.semanticOriginX);
            source.insert(QStringLiteral("originY"), layer.semanticOriginY);
            source.insert(QStringLiteral("color"), layer.semanticColor);
        } else if (layer.kind == QStringLiteral("vector")) {
            source.insert(QStringLiteral("pathCount"), layer.semanticPathCount);
            source.insert(QStringLiteral("commandCount"), layer.semanticCommandCount);
            source.insert(QStringLiteral("rectangleRecognized"), layer.semanticRectangleRecognized);
            if (layer.semanticRectangleRecognized) {
                source.insert(QStringLiteral("x"), layer.semanticRectangleX);
                source.insert(QStringLiteral("y"), layer.semanticRectangleY);
                source.insert(QStringLiteral("width"), layer.semanticRectangleWidth);
                source.insert(QStringLiteral("height"), layer.semanticRectangleHeight);
                source.insert(QStringLiteral("fill"), layer.semanticRectangleFill);
                source.insert(QStringLiteral("stroke"), layer.semanticRectangleStroke);
                source.insert(QStringLiteral("strokeWidth"), layer.semanticRectangleStrokeWidth);
            }
        }
        return source;
    }
    return {};
}
