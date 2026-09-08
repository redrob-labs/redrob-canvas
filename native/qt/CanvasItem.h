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
    Q_PROPERTY(QRectF imageRect READ imageRect NOTIFY geometryProjectionChanged)
    Q_PROPERTY(bool previewVisible READ previewVisible WRITE setPreviewVisible NOTIFY previewChanged)
    Q_PROPERTY(QString previewKind READ previewKind WRITE setPreviewKind NOTIFY previewChanged)
    Q_PROPERTY(QPointF previewStart READ previewStart WRITE setPreviewStart NOTIFY previewChanged)
    Q_PROPERTY(QPointF previewEnd READ previewEnd WRITE setPreviewEnd NOTIFY previewChanged)

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
    QRectF imageRect() const;
    bool previewVisible() const;
    void setPreviewVisible(bool visible);
    QString previewKind() const;
    void setPreviewKind(const QString &kind);
    QPointF previewStart() const;
    void setPreviewStart(const QPointF &point);
    QPointF previewEnd() const;
    void setPreviewEnd(const QPointF &point);

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

protected:
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private:
    void rebuildSelectionCache();
    void updateAntsTimer();
    void paintPreview(QPainter *painter, const QRectF &target);

    QImage m_image;
    QImage m_selectionMask;
    QImage m_selectionOverlay;
    QPainterPath m_selectionContour;
    QTimer m_antsTimer;
    qreal m_zoom = 1.0;
    qreal m_dashPhase = 0.0;
    bool m_selectionActive = false;
    bool m_previewVisible = false;
    QString m_previewKind;
    QPointF m_previewStart;
    QPointF m_previewEnd;
};
