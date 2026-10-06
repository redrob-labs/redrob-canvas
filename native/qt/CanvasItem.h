// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QImage>
#include <QPainterPath>
#include <QPointF>
#include <QQuickPaintedItem>
#include <QTimer>

class CanvasItem : public QQuickPaintedItem
{
    Q_OBJECT
    Q_PROPERTY(QImage image READ image WRITE setImage NOTIFY imageChanged)
    Q_PROPERTY(QImage selectionMask READ selectionMask WRITE setSelectionMask NOTIFY selectionMaskChanged)
    Q_PROPERTY(bool selectionActive READ selectionActive WRITE setSelectionActive NOTIFY selectionActiveChanged)
    Q_PROPERTY(qreal zoom READ zoom WRITE setZoom NOTIFY zoomChanged)
    // Hand tool: how far the image is moved from the centre, in item pixels.
    Q_PROPERTY(QPointF pan READ pan WRITE setPan NOTIFY zoomChanged)
    Q_PROPERTY(QRectF imageRect READ imageRect NOTIFY geometryProjectionChanged)
    Q_PROPERTY(bool previewVisible READ previewVisible WRITE setPreviewVisible NOTIFY previewChanged)
    Q_PROPERTY(QString previewKind READ previewKind WRITE setPreviewKind NOTIFY previewChanged)
    Q_PROPERTY(QPointF previewStart READ previewStart WRITE setPreviewStart NOTIFY previewChanged)
    Q_PROPERTY(QPointF previewEnd READ previewEnd WRITE setPreviewEnd NOTIFY previewChanged)
    // Control-handle overlay (H.21): a flat [x0,y0,x1,y1,...] list in CANVAS coordinates, drawn as a
    // closed outline with a grab square at each vertex. Canvas coordinates rather than item
    // coordinates so the handles stay on the pixels they control at any zoom or scroll position.
    Q_PROPERTY(QVariantList handlePoints READ handlePoints WRITE setHandlePoints NOTIFY handlesChanged)
    // Each handle's OUTGOING control point, flat [x,y,...] and the same length as handlePoints, or
    // empty for the tools whose handles are plain vertices (perspective, cage, n-point). A control
    // point sitting on its own anchor is a corner and draws nothing, which is how the pen encodes
    // one -- so this needs no separate "absent" value.
    Q_PROPERTY(QVariantList controlPoints READ controlPoints WRITE setControlPoints NOTIFY handlesChanged)
    // Which handle index (in POINT units, not list slots) is being dragged, or -1. Drawn filled, so
    // the user can see which corner they grabbed when two sit close together.
    Q_PROPERTY(int activeHandle READ activeHandle WRITE setActiveHandle NOTIFY handlesChanged)

public:
    explicit CanvasItem(QQuickItem *parent = nullptr);
    void paint(QPainter *painter) override;

    QImage image() const;
    void setImage(const QImage &image);
    QImage selectionMask() const;
    void setSelectionMask(const QImage &mask);
    bool selectionActive() const;
    void setSelectionActive(bool active);
    qreal zoom() const;
    void setZoom(qreal zoom);
    QPointF pan() const { return m_pan; }
    void setPan(const QPointF &pan);
    QRectF imageRect() const;
    bool previewVisible() const;
    void setPreviewVisible(bool visible);
    QString previewKind() const;
    void setPreviewKind(const QString &kind);
    QPointF previewStart() const;
    void setPreviewStart(const QPointF &point);
    QPointF previewEnd() const;
    void setPreviewEnd(const QPointF &point);
    QVariantList handlePoints() const;
    void setHandlePoints(const QVariantList &points);
    QVariantList controlPoints() const;
    void setControlPoints(const QVariantList &points);
    int activeHandle() const;
    void setActiveHandle(int index);

    Q_INVOKABLE QPointF canvasPoint(const QPointF &itemPoint) const;
    Q_INVOKABLE bool containsCanvasPoint(const QPointF &itemPoint) const;
    Q_INVOKABLE void clearPreview();

signals:
    void imageChanged();
    void selectionMaskChanged();
    void selectionActiveChanged();
    void zoomChanged();
    void geometryProjectionChanged();
    void previewChanged();
    void handlesChanged();

protected:
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private:
    void rebuildSelectionCache();
    void updateAntsTimer();
    void paintPreview(QPainter *painter, const QRectF &target);
    void paintHandles(QPainter *painter, const QRectF &target);

    QImage m_image;
    QImage m_selectionMask;
    QImage m_selectionOverlay;
    QPainterPath m_selectionContour;
    QTimer m_antsTimer;
    qreal m_zoom = 1.0;
    QPointF m_pan;
    qreal m_dashPhase = 0.0;
    bool m_selectionActive = false;
    bool m_previewVisible = false;
    QString m_previewKind;
    QPointF m_previewStart;
    QPointF m_previewEnd;
    QVariantList m_handlePoints;
    QVariantList m_controlPoints;
    int m_activeHandle = -1;
};
