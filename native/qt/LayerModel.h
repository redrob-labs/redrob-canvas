// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QAbstractListModel>
#include <QColor>
#include <QJsonObject>
#include <QVariantMap>
#include <QVector>

class LayerModel final : public QAbstractListModel
{
    Q_OBJECT

public:
    enum Role {
        IdRole = Qt::UserRole + 1,
        NameRole,
        VisibleRole,
        OpacityRole,
        BlendModeRole,
        ActiveRole,
        CoreIndexRole,
        KindRole,
        ParentIdRole,
        DepthRole,
        SiblingIndexRole,
        SiblingCountRole,
        IsTopRole,
        IsBottomRole,
        HasChildrenRole,
        HasMaskRole,
        MaskEnabledRole,
        CanEditRasterRole,
        CanEditTextRole,
        CanEditVectorRole,
        CanRasterizeRole,
        IsSemanticRole,
        SemanticPreviewRole,
        SemanticTextRole,
        SemanticPreviewTruncatedRole,
        SemanticPathCountRole,
        SemanticCommandCountRole,
        SemanticFontIdRole,
        SemanticFontFamilyRole,
        SemanticFontSizeRole,
        SemanticOriginXRole,
        SemanticOriginYRole,
        SemanticColorRole,
        SemanticRectangleRecognizedRole,
        SemanticRectangleXRole,
        SemanticRectangleYRole,
        SemanticRectangleWidthRole,
        SemanticRectangleHeightRole,
        SemanticRectangleFillRole,
        SemanticRectangleStrokeRole,
        SemanticRectangleStrokeWidthRole,
    };
    Q_ENUM(Role)

    explicit LayerModel(QObject *parent = nullptr);
    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    void replaceFromSnapshot(const QJsonObject &snapshot);
    QString activeLayerId() const;
    int layerCount() const;
    QString layerIdAt(int row) const;
    QString activeNodeKind() const;
    bool activeNodeCanEditRaster() const;
    bool activeNodeCanEditText() const;
    bool activeNodeCanEditVector() const;
    bool activeNodeCanRasterize() const;
    bool activeNodeHasMask() const;
    int siblingCount(const QString &parentId) const;
    Q_INVOKABLE QVariantMap semanticSource(const QString &id) const;

private:
    struct LayerRow {
        QString id;
        QString name;
        QString blendMode;
        QString kind;
        QString parentId;
        bool visible = true;
        double opacity = 1.0;
        bool active = false;
        bool hasChildren = false;
        bool hasMask = false;
        bool maskEnabled = false;
        bool canEditRaster = false;
        bool canEditText = false;
        bool canEditVector = false;
        bool canRasterize = false;
        bool isSemantic = false;
        QString semanticPreview;
        QString semanticText;
        bool semanticPreviewTruncated = false;
        int semanticPathCount = 0;
        int semanticCommandCount = 0;
        QString semanticFontId;
        QString semanticFontFamily;
        double semanticFontSize = 0.0;
        double semanticOriginX = 0.0;
        double semanticOriginY = 0.0;
        QColor semanticColor;
        bool semanticRectangleRecognized = false;
        double semanticRectangleX = 0.0;
        double semanticRectangleY = 0.0;
        double semanticRectangleWidth = 0.0;
        double semanticRectangleHeight = 0.0;
        QColor semanticRectangleFill;
        QColor semanticRectangleStroke;
        double semanticRectangleStrokeWidth = 0.0;
        int coreIndex = 0;
        int depth = 0;
        int siblingIndex = 0;
        int siblingCount = 0;
    };
    QVector<LayerRow> m_layers;
    QString m_activeLayerId;
};
