// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <QQuickImageProvider>

#include "EditorBridge.h"

// image://layerthumb/<node id>?g=<generation> -- a Layers panel row's thumbnail. The generation
// is only there so the URL changes after every edit and QML asks again; the provider ignores it.
// Synchronous on the GUI thread on purpose: the editor handle is owned by the bridge, which can
// close a document while a worker thread would still be reading it.
class LayerThumbnailProvider final : public QQuickImageProvider
{
public:
    explicit LayerThumbnailProvider(EditorBridge &editor)
        : QQuickImageProvider(QQuickImageProvider::Image)
        , m_editor(editor)
    {
    }

    QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override
    {
        const QString nodeId = id.section(QLatin1Char('?'), 0, 0);
        const int side = requestedSize.isValid()
            ? qMax(requestedSize.width(), requestedSize.height())
            : 64;
        QImage image = m_editor.layerThumbnail(nodeId, side > 0 ? side : 64);
        if (image.isNull()) {
            // An adjustment layer: 1x1 transparent, so the row shows its glyph instead.
            image = QImage(1, 1, QImage::Format_RGBA8888);
            image.fill(Qt::transparent);
        }
        if (size)
            *size = image.size();
        return image;
    }

private:
    EditorBridge &m_editor;
};
