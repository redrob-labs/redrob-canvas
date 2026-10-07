// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls

// View > Rulers (Ctrl+R) and the guides (batch 4 L2), over the canvas item. Photoshop's behaviour:
// drag out of a ruler to place a guide, drag a guide to move it, drag it back onto a ruler to
// remove it. Positions are canvas pixels. With the view rotated the rulers stay axis-aligned to the
// screen and read the canvas coordinate along that edge, so they are only exact at 0 degrees.
Item {
    id: root
    objectName: "rulersOverlay"
    required property var app
    required property var view
    readonly property int band: 20

    // Canvas coordinate at an item point, and the scale (canvas px per item px), from the view.
    function canvasAt(x, y) { return view.canvasPoint(Qt.point(x, y)); }
    readonly property point origin: {
        view.zoom; view.pan; view.viewRotation; view.width; view.height;
        return canvasAt(0, 0);
    }
    readonly property real perPixel: {
        view.zoom; view.pan; view.viewRotation;
        const a = canvasAt(0, 0);
        const b = canvasAt(1, 0);
        return Math.max(1e-6, Math.hypot(b.x - a.x, b.y - a.y));
    }
    function itemX(canvasX) { return (canvasX - origin.x) / perPixel; }
    function itemY(canvasY) { return (canvasY - origin.y) / perPixel; }
    // A tick step in canvas px that keeps labels ~80 screen px apart: 1, 2, 5 x 10^n.
    function step() {
        const raw = 80 * perPixel;
        const p = Math.pow(10, Math.floor(Math.log(raw) / Math.LN10));
        return raw <= p ? p : raw <= 2 * p ? 2 * p : raw <= 5 * p ? 5 * p : 10 * p;
    }

    // U9: each artboard's outline and its name tag, Photoshop's artboard handle. Dragging the tag
    // moves the artboard and everything in it; the outline follows the hand and the move is one
    // undo step on release, rounded to whole canvas pixels.
    Repeater {
        model: editor.layers
        delegate: Item {
            id: board
            required property string layerId
            required property string layerName
            required property var artboard
            readonly property bool isBoard: artboard !== undefined && artboard !== null
                                            && artboard.width !== undefined
            property real dragX: 0
            property real dragY: 0
            objectName: "artboardHandle-" + layerId
            visible: isBoard
            x: isBoard ? root.itemX(artboard.x) + dragX : 0
            y: isBoard ? root.itemY(artboard.y) + dragY : 0
            width: isBoard ? artboard.width / root.perPixel : 0
            height: isBoard ? artboard.height / root.perPixel : 0
            Rectangle {
                anchors.fill: parent
                color: "transparent"
                border.color: root.app.tokens.borderStrong
                border.width: 1
            }
            Label {
                id: boardTag
                objectName: "artboardTag-" + board.layerId
                text: board.layerName
                y: -height - 2
                color: root.app.tokens.inkSecondary
                font.pixelSize: 11
                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -3
                    cursorShape: Qt.SizeAllCursor
                    property point start
                    onPressed: mouse => {
                        start = mapToItem(root, mouse.x, mouse.y);
                        editor.setActiveLayer(board.layerId);
                    }
                    onPositionChanged: mouse => {
                        const p = mapToItem(root, mouse.x, mouse.y);
                        board.dragX = p.x - start.x;
                        board.dragY = p.y - start.y;
                    }
                    onReleased: {
                        const dx = Math.round(board.dragX * root.perPixel);
                        const dy = Math.round(board.dragY * root.perPixel);
                        board.dragX = 0;
                        board.dragY = 0;
                        editor.moveArtboard(board.layerId, dx, dy);
                    }
                    onCanceled: {
                        board.dragX = 0;
                        board.dragY = 0;
                    }
                }
            }
        }
    }

    // The guide lines.
    Repeater {
        model: root.app.guidesVisible ? editor.guides : []
        delegate: Rectangle {
            required property var modelData
            readonly property bool vertical: modelData.vertical
            objectName: "guideLine"
            color: root.app.tokens.focusRing
            x: vertical ? root.itemX(modelData.position) - 0.5 : 0
            y: vertical ? 0 : root.itemY(modelData.position) - 0.5
            width: vertical ? 1 : root.width
            height: vertical ? root.height : 1
            // A wider grab strip than the 1 px line, for moving it.
            MouseArea {
                anchors.centerIn: parent
                width: parent.vertical ? 7 : parent.width
                height: parent.vertical ? parent.height : 7
                cursorShape: parent.vertical ? Qt.SplitHCursor : Qt.SplitVCursor
                onReleased: mouse => {
                    const p = mapToItem(root, mouse.x, mouse.y);
                    const onRuler = root.app.rulersVisible && (parent.vertical ? p.y < root.band : p.x < root.band);
                    if (onRuler)
                        editor.removeGuide(parent.modelData.id);
                    else
                        editor.moveGuide(parent.modelData.id, Math.round(parent.vertical ? root.canvasAt(p.x, p.y).x
                                                                                         : root.canvasAt(p.x, p.y).y));
                }
            }
        }
    }

    // A guide being dragged out of a ruler, shown before it is placed.
    property int dragAxis: 0 // 0 none, 1 vertical guide (from the left ruler), 2 horizontal
    property point dragAt: Qt.point(0, 0)
    Rectangle {
        visible: root.dragAxis !== 0
        color: root.app.tokens.focusRing
        opacity: 0.6
        x: root.dragAxis === 1 ? root.dragAt.x : 0
        y: root.dragAxis === 2 ? root.dragAt.y : 0
        width: root.dragAxis === 1 ? 1 : root.width
        height: root.dragAxis === 2 ? 1 : root.height
    }

    Repeater {
        model: root.app.rulersVisible ? [false, true] : []
        delegate: Rectangle {
            required property bool modelData // true = left (vertical) ruler
            objectName: modelData ? "rulerLeft" : "rulerTop"
            readonly property bool isVertical: modelData
            x: 0
            y: 0
            width: isVertical ? root.band : root.width
            height: isVertical ? root.height : root.band
            color: root.app.tokens.surfaceRaised
            border.color: root.app.tokens.borderSubtle
            Canvas {
                anchors.fill: parent
                property var deps: [root.origin, root.perPixel, root.width, root.height, root.app.tokens.inkSecondary]
                onDepsChanged: requestPaint()
                onPaint: {
                    const ctx = getContext("2d");
                    ctx.clearRect(0, 0, width, height);
                    ctx.strokeStyle = root.app.tokens.inkSecondary;
                    ctx.fillStyle = root.app.tokens.inkSecondary;
                    ctx.font = "9px sans-serif";
                    const s = root.step();
                    const length = parent.isVertical ? height : width;
                    const start = parent.isVertical ? root.origin.y : root.origin.x;
                    const end = start + length * root.perPixel;
                    ctx.beginPath();
                    for (let v = Math.floor(start / s) * s; v <= end; v += s / 5) {
                        const at = (v - start) / root.perPixel;
                        const major = Math.abs(v / s - Math.round(v / s)) < 1e-6;
                        const tick = major ? root.band : root.band / 3;
                        if (parent.isVertical) {
                            ctx.moveTo(root.band - tick, at + 0.5);
                            ctx.lineTo(root.band, at + 0.5);
                        } else {
                            ctx.moveTo(at + 0.5, root.band - tick);
                            ctx.lineTo(at + 0.5, root.band);
                        }
                        if (major) {
                            if (parent.isVertical) {
                                ctx.save();
                                ctx.translate(9, at + 2);
                                ctx.rotate(-Math.PI / 2);
                                ctx.fillText(String(Math.round(v)), 0, 0);
                                ctx.restore();
                            } else {
                                ctx.fillText(String(Math.round(v)), at + 2, 9);
                            }
                        }
                    }
                    ctx.stroke();
                }
            }
            // Drag out of the ruler to place a guide: the top ruler makes horizontal guides.
            MouseArea {
                anchors.fill: parent
                cursorShape: parent.isVertical ? Qt.SplitHCursor : Qt.SplitVCursor
                onPressed: mouse => {
                    root.dragAxis = parent.isVertical ? 1 : 2;
                    root.dragAt = mapToItem(root, mouse.x, mouse.y);
                }
                onPositionChanged: mouse => root.dragAt = mapToItem(root, mouse.x, mouse.y)
                onReleased: mouse => {
                    const p = mapToItem(root, mouse.x, mouse.y);
                    const placed = parent.isVertical ? p.x > root.band : p.y > root.band;
                    if (placed) {
                        const c = root.canvasAt(p.x, p.y);
                        editor.addGuide(parent.isVertical, Math.round(parent.isVertical ? c.x : c.y));
                        root.app.guidesVisible = true;
                    }
                    root.dragAxis = 0;
                }
            }
            Accessible.name: isVertical ? "Vertical ruler" : "Horizontal ruler"
        }
    }
}
