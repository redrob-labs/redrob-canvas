// SPDX-License-Identifier: GPL-3.0-or-later
#include "CanvasItem.h"

#include <QLineF>
#include <QPainter>
#include <QPen>
#include <QVector>

#include <algorithm>
#include <cmath>
#include <cstring>

namespace {
constexpr qsizetype kMaxContourCells = 65'536;
constexpr qsizetype kMaxContourSegments = kMaxContourCells * 4;
} // namespace

CanvasItem::CanvasItem(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    setAntialiasing(true);
    setOpaquePainting(false);
    setRenderTarget(QQuickPaintedItem::FramebufferObject);
    m_antsTimer.setInterval(140);
    m_antsTimer.setTimerType(Qt::CoarseTimer);
    connect(&m_antsTimer, &QTimer::timeout, this, [this] {
        m_dashPhase = std::fmod(m_dashPhase + 1.0, 8.0);
        update();
    });
}

QImage CanvasItem::image() const { return m_image; }

void CanvasItem::setImage(const QImage &image)
{
    QImage normalized = image;
    normalized.setDevicePixelRatio(1.0);
    if (normalized == m_image)
        return;
    m_image = normalized.copy();
    emit imageChanged();
    emit geometryProjectionChanged();
    update();
}

QImage CanvasItem::selectionMask() const { return m_selectionMask; }

void CanvasItem::setSelectionMask(const QImage &mask)
{
    QImage normalized = mask.convertToFormat(QImage::Format_Grayscale8);
    normalized.setDevicePixelRatio(1.0);
    if (normalized == m_selectionMask)
        return;
    m_selectionMask = std::move(normalized);
    if (m_selectionActive)
        rebuildSelectionCache();
    else {
        m_selectionContour = {};
        m_selectionOverlay = {};
        updateAntsTimer();
    }
    emit selectionMaskChanged();
    update();
}

bool CanvasItem::selectionActive() const { return m_selectionActive; }

void CanvasItem::setSelectionActive(bool active)
{
    if (m_selectionActive == active)
        return;
    m_selectionActive = active;
    if (active)
        rebuildSelectionCache();
    else {
        m_selectionContour = {};
        m_selectionOverlay = {};
        updateAntsTimer();
    }
    emit selectionActiveChanged();
    update();
}

qreal CanvasItem::zoom() const { return m_zoom; }

void CanvasItem::setZoom(qreal zoom)
{
    const qreal bounded = qBound(0.05, zoom, 32.0);
    if (qFuzzyCompare(m_zoom, bounded))
        return;
    m_zoom = bounded;
    emit zoomChanged();
    emit geometryProjectionChanged();
    update();
}

QRectF CanvasItem::imageRect() const
{
    if (m_image.isNull())
        return {};
    const QSizeF logicalSize(m_image.width() * m_zoom, m_image.height() * m_zoom);
    return QRectF((width() - logicalSize.width()) / 2.0,
                  (height() - logicalSize.height()) / 2.0,
                  logicalSize.width(), logicalSize.height());
}

bool CanvasItem::previewVisible() const { return m_previewVisible; }
void CanvasItem::setPreviewVisible(bool visible)
{
    if (m_previewVisible == visible)
        return;
    m_previewVisible = visible;
    emit previewChanged();
    update();
}
QString CanvasItem::previewKind() const { return m_previewKind; }
void CanvasItem::setPreviewKind(const QString &kind)
{
    if (m_previewKind == kind)
        return;
    m_previewKind = kind;
    emit previewChanged();
    update();
}
QPointF CanvasItem::previewStart() const { return m_previewStart; }
void CanvasItem::setPreviewStart(const QPointF &point)
{
    if (m_previewStart == point)
        return;
    m_previewStart = point;
    emit previewChanged();
    update();
}
QPointF CanvasItem::previewEnd() const { return m_previewEnd; }
void CanvasItem::setPreviewEnd(const QPointF &point)
{
    if (m_previewEnd == point)
        return;
    m_previewEnd = point;
    emit previewChanged();
    update();
}

void CanvasItem::clearPreview()
{
    setPreviewVisible(false);
    setPreviewKind(QString{});
}

QPointF CanvasItem::canvasPoint(const QPointF &itemPoint) const
{
    const QRectF target = imageRect();
    if (target.isEmpty())
        return {-1.0, -1.0};
    return {(itemPoint.x() - target.left()) / m_zoom,
            (itemPoint.y() - target.top()) / m_zoom};
}

bool CanvasItem::containsCanvasPoint(const QPointF &itemPoint) const
{
    return imageRect().contains(itemPoint);
}

void CanvasItem::updateAntsTimer()
{
    const bool shouldRun = m_selectionActive && !m_selectionContour.isEmpty();
    if (shouldRun) {
        if (!m_antsTimer.isActive())
            m_antsTimer.start();
    } else {
        m_antsTimer.stop();
        m_dashPhase = 0.0;
    }
}

void CanvasItem::rebuildSelectionCache()
{
    m_selectionContour = {};
    m_selectionOverlay = {};
    if (!m_selectionActive || m_selectionMask.isNull()) {
        updateAntsTimer();
        return;
    }

    const int width = m_selectionMask.width();
    const int height = m_selectionMask.height();

    // Keep the visual overlay at one byte per pixel rather than expanding the
    // selection into a full ARGB image. The palette maps mask coverage directly
    // to the blue overlay's alpha channel.
    QVector<QRgb> tintTable(256);
    for (int coverage = 0; coverage < tintTable.size(); ++coverage)
        tintTable[coverage] = qRgba(64, 151, 255, qRound(coverage * 0.20));
    m_selectionOverlay = QImage(width, height, QImage::Format_Indexed8);
    m_selectionOverlay.setColorTable(tintTable);
    for (int y = 0; y < height; ++y)
        std::memcpy(m_selectionOverlay.scanLine(y), m_selectionMask.constScanLine(y),
                    static_cast<size_t>(width));

    // Preserve exact pixel contours whenever they fit the hard path budget.
    // Typical selections (including large ellipses and full-canvas bounds) are
    // therefore crisp; only adversarial high-frequency masks use the coarse,
    // bounded fallback below.
    const auto selectedPixel = [this, width, height](int x, int y) {
        return x >= 0 && y >= 0 && x < width && y < height
            && m_selectionMask.constScanLine(y)[x] != 0;
    };
    qsizetype exactSegments = 0;
    bool exactBudgetExceeded = false;
    const auto appendExactEdge = [this, &exactSegments, &exactBudgetExceeded](
                                     qreal x1, qreal y1, qreal x2, qreal y2) {
        if (exactSegments >= kMaxContourSegments) {
            exactBudgetExceeded = true;
            return;
        }
        m_selectionContour.moveTo(x1, y1);
        m_selectionContour.lineTo(x2, y2);
        ++exactSegments;
    };
    for (int y = 0; y < height && !exactBudgetExceeded; ++y) {
        const uchar *row = m_selectionMask.constScanLine(y);
        for (int x = 0; x < width && !exactBudgetExceeded; ++x) {
            if (row[x] == 0)
                continue;
            if (!selectedPixel(x, y - 1))
                appendExactEdge(x, y, x + 1, y);
            if (!selectedPixel(x + 1, y))
                appendExactEdge(x + 1, y, x + 1, y + 1);
            if (!selectedPixel(x, y + 1))
                appendExactEdge(x + 1, y + 1, x, y + 1);
            if (!selectedPixel(x - 1, y))
                appendExactEdge(x, y + 1, x, y);
        }
    }
    if (!exactBudgetExceeded) {
        updateAntsTimer();
        return;
    }

    // An adversarial mask exceeded the exact budget. Rebuild from a bounded
    // occupancy grid that represents the complete mask while guaranteeing no
    // more than 262,144 path segments.
    m_selectionContour = {};
    int step = 1;
    const quint64 maskPixels = quint64(width) * quint64(height);
    if (maskPixels > quint64(kMaxContourCells))
        step = qMax(1, static_cast<int>(std::ceil(
                           std::sqrt(static_cast<double>(maskPixels)
                                     / static_cast<double>(kMaxContourCells)))));
    int gridWidth = (width + step - 1) / step;
    int gridHeight = (height + step - 1) / step;
    while (quint64(gridWidth) * quint64(gridHeight) > quint64(kMaxContourCells)) {
        ++step;
        gridWidth = (width + step - 1) / step;
        gridHeight = (height + step - 1) / step;
    }

    QVector<quint8> occupied(qsizetype(gridWidth) * qsizetype(gridHeight), 0);
    for (int y = 0; y < height; ++y) {
        const uchar *row = m_selectionMask.constScanLine(y);
        quint8 *gridRow = occupied.data() + qsizetype(y / step) * gridWidth;
        for (int x = 0; x < width; ++x) {
            if (row[x] != 0)
                gridRow[x / step] = 1;
        }
    }

    const auto selectedCell = [&occupied, gridWidth, gridHeight](int x, int y) {
        return x >= 0 && y >= 0 && x < gridWidth && y < gridHeight
            && occupied.at(qsizetype(y) * gridWidth + x) != 0;
    };
    qsizetype segments = 0;
    const auto appendEdge = [this, &segments](qreal x1, qreal y1, qreal x2, qreal y2) {
        if (segments >= kMaxContourSegments)
            return;
        m_selectionContour.moveTo(x1, y1);
        m_selectionContour.lineTo(x2, y2);
        ++segments;
    };
    for (int cellY = 0; cellY < gridHeight; ++cellY) {
        const int y1 = cellY * step;
        const int y2 = qMin(height, y1 + step);
        for (int cellX = 0; cellX < gridWidth; ++cellX) {
            if (!selectedCell(cellX, cellY))
                continue;
            const int x1 = cellX * step;
            const int x2 = qMin(width, x1 + step);
            if (!selectedCell(cellX, cellY - 1))
                appendEdge(x1, y1, x2, y1);
            if (!selectedCell(cellX + 1, cellY))
                appendEdge(x2, y1, x2, y2);
            if (!selectedCell(cellX, cellY + 1))
                appendEdge(x2, y2, x1, y2);
            if (!selectedCell(cellX - 1, cellY))
                appendEdge(x1, y2, x1, y1);
        }
    }
    updateAntsTimer();
}

void CanvasItem::paintPreview(QPainter *painter, const QRectF &target)
{
    if (!m_previewVisible || m_previewKind.isEmpty())
        return;

    painter->save();
    painter->translate(target.topLeft());
    painter->scale(m_zoom, m_zoom);
    QPen pen(QColor(QStringLiteral("#f4c95d")), 1.0, Qt::DashLine);
    pen.setCosmetic(true);
    painter->setPen(pen);
    painter->setBrush(QColor(244, 201, 93, 24));

    const QRectF bounds(m_previewStart, m_previewEnd);
    const QRectF normalized = bounds.normalized();
    if (m_previewKind == QStringLiteral("rectangle") || m_previewKind == QStringLiteral("crop")) {
        painter->drawRect(normalized);
    } else if (m_previewKind == QStringLiteral("ellipse")) {
        painter->drawEllipse(normalized);
    } else if (m_previewKind == QStringLiteral("linear")) {
        painter->drawLine(m_previewStart, m_previewEnd);
        painter->drawEllipse(m_previewStart, 3.0 / m_zoom, 3.0 / m_zoom);
        painter->drawEllipse(m_previewEnd, 3.0 / m_zoom, 3.0 / m_zoom);
    } else if (m_previewKind == QStringLiteral("radial")) {
        const qreal radius = QLineF(m_previewStart, m_previewEnd).length();
        painter->drawEllipse(m_previewStart, radius, radius);
        painter->drawLine(m_previewStart, m_previewEnd);
    } else if (m_previewKind == QStringLiteral("transform")) {
        const QPointF delta = m_previewEnd - m_previewStart;
        painter->setBrush(Qt::NoBrush);
        painter->drawRect(QRectF(delta, QSizeF(m_image.width(), m_image.height())));
        painter->drawLine(m_previewStart, m_previewEnd);
    }
    painter->restore();
}

void CanvasItem::paint(QPainter *painter)
{
    const QRectF target = imageRect();
    if (target.isEmpty())
        return;

    QRectF visibleTarget = target.intersected(boundingRect());
    if (painter->hasClipping())
        visibleTarget = visibleTarget.intersected(painter->clipBoundingRect());
    if (visibleTarget.isEmpty())
        return;

    painter->save();
    painter->setClipRect(visibleTarget);
    constexpr int tile = 12;
    const QColor light(QStringLiteral("#35383d"));
    const QColor dark(QStringLiteral("#292c31"));
    const int left = static_cast<int>(std::floor(visibleTarget.left() / tile)) * tile;
    const int top = static_cast<int>(std::floor(visibleTarget.top() / tile)) * tile;
    for (int y = top; y < visibleTarget.bottom(); y += tile) {
        for (int x = left; x < visibleTarget.right(); x += tile) {
            const bool alternate = ((x / tile) + (y / tile)) & 1;
            painter->fillRect(QRectF(x, y, tile, tile), alternate ? light : dark);
        }
    }
    painter->setRenderHint(QPainter::SmoothPixmapTransform, m_zoom < 1.0);
    painter->drawImage(target, m_image, m_image.rect());
    if (m_selectionActive && !m_selectionOverlay.isNull())
        painter->drawImage(target, m_selectionOverlay, m_selectionOverlay.rect());
    painter->restore();

    if (m_selectionActive && !m_selectionContour.isEmpty()) {
        painter->save();
        painter->translate(target.topLeft());
        painter->scale(m_zoom, m_zoom);
        QPen shadow(Qt::black, 1.0);
        shadow.setCosmetic(true);
        painter->setPen(shadow);
        painter->drawPath(m_selectionContour);
        QPen ants(Qt::white, 1.0, Qt::CustomDashLine);
        ants.setCosmetic(true);
        ants.setDashPattern({4.0, 4.0});
        ants.setDashOffset(m_dashPhase);
        painter->setPen(ants);
        painter->drawPath(m_selectionContour);
        painter->restore();
    }

    paintPreview(painter, target);
    painter->setPen(QPen(QColor(QStringLiteral("#0d0e10")), 1.0));
    painter->drawRect(target.adjusted(0.5, 0.5, -0.5, -0.5));
}

void CanvasItem::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.size() != oldGeometry.size()) {
        emit geometryProjectionChanged();
        update();
    }
}
