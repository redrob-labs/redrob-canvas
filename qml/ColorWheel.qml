// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick

// A hue ring with an inner saturation/value square, re-derived from Krita's advanced colour
// selector and GIMP's colour wheel (behaviour studied, not copied). Emits colorPicked(color) and
// tracks an external color via the `current` property.
Item {
    id: root
    implicitWidth: 220
    implicitHeight: 220

    property color current: "#ffffff"
    signal colorPicked(color picked)

    // Gamut mask (Krita): when gamutSpan > 0, hue selection is constrained to the arc
    // [gamutStart, gamutStart + gamutSpan] degrees, and the ring dims outside it.
    property real gamutStart: 0      // degrees
    property real gamutSpan: 0       // degrees; 0 = no mask

    // Decomposed HSV of the current colour (kept so dragging S/V does not lose the hue at grey).
    property real hue: 0        // 0..1
    property real sat: 0        // 0..1
    property real val: 1        // 0..1

    function syncFromCurrent() {
        // Qt's color exposes hsvHue/hsvSaturation/hsvValue in 0..1 (hue is -1 for achromatic).
        var h = current.hsvHue;
        if (h >= 0)
            hue = h;
        sat = current.hsvSaturation;
        val = current.hsvValue;
        ring.requestPaint();
        square.requestPaint();
    }
    onCurrentChanged: syncFromCurrent()
    Component.onCompleted: syncFromCurrent()

    function emitColor() {
        root.colorPicked(Qt.hsva(hue, sat, val, 1));
    }

    // Repaint both canvases (called when the gamut mask changes from outside).
    function repaint() {
        ring.requestPaint();
        square.requestPaint();
    }

    // Gamut-mask helpers: is a hue (degrees) inside the arc, and snap it to the nearest edge if not.
    function hueInGamut(deg) {
        if (gamutSpan <= 0) return true;
        var d = ((deg - gamutStart) % 360 + 360) % 360;
        return d <= gamutSpan;
    }
    function clampHueDeg(deg) {
        if (gamutSpan <= 0) return deg;
        if (hueInGamut(deg)) return deg;
        var d = ((deg - gamutStart) % 360 + 360) % 360;
        // Snap to whichever arc end is nearer.
        var distToStart = Math.min(d, 360 - d);
        var distToEnd = Math.min(Math.abs(d - gamutSpan), 360 - Math.abs(d - gamutSpan));
        return distToStart < distToEnd ? gamutStart : (gamutStart + gamutSpan);
    }

    readonly property real ringThickness: width * 0.14
    readonly property real outerR: width / 2
    readonly property real innerR: outerR - ringThickness
    // The SV square inscribed in the inner circle.
    readonly property real squareSize: innerR * Math.SQRT2 * 0.92
    readonly property real squareX: width / 2 - squareSize / 2
    readonly property real squareY: height / 2 - squareSize / 2

    // Hue ring.
    Canvas {
        id: ring
        anchors.fill: parent
        onPaint: {
            var ctx = getContext("2d");
            ctx.clearRect(0, 0, width, height);
            var cx = width / 2, cy = height / 2;
            var steps = 360;
            for (var i = 0; i < steps; i++) {
                var a0 = (i / steps) * 2 * Math.PI;
                var a1 = ((i + 1.5) / steps) * 2 * Math.PI;
                ctx.beginPath();
                ctx.arc(cx, cy, root.outerR, a0, a1, false);
                ctx.arc(cx, cy, root.innerR, a1, a0, true);
                ctx.closePath();
                var hueDeg = (i / steps) * 360;
                var inMask = root.gamutSpan <= 0 || root.hueInGamut(hueDeg);
                ctx.globalAlpha = inMask ? 1.0 : 0.25;
                ctx.fillStyle = Qt.hsva(i / steps, 1, 1, 1);
                ctx.fill();
            }
            ctx.globalAlpha = 1.0;
        }
    }

    // Hue handle.
    Rectangle {
        width: 10; height: 10; radius: 5
        border.color: "#000"; border.width: 1; color: "transparent"
        x: width / 2 + Math.cos(root.hue * 2 * Math.PI) * (root.innerR + root.ringThickness / 2) + root.width / 2 - width
        y: height / 2 + Math.sin(root.hue * 2 * Math.PI) * (root.innerR + root.ringThickness / 2) + root.height / 2 - height
        // (Positioned approximately; the ring drag below is the authoritative input.)
    }

    // Saturation/Value square.
    Canvas {
        id: square
        x: root.squareX; y: root.squareY
        width: root.squareSize; height: root.squareSize
        onPaint: {
            var ctx = getContext("2d");
            // Base hue, white to the left, black to the bottom.
            var base = Qt.hsva(root.hue, 1, 1, 1);
            var gx = ctx.createLinearGradient(0, 0, width, 0);
            gx.addColorStop(0, "#ffffff");
            gx.addColorStop(1, base);
            ctx.fillStyle = gx;
            ctx.fillRect(0, 0, width, height);
            var gy = ctx.createLinearGradient(0, 0, 0, height);
            gy.addColorStop(0, "rgba(0,0,0,0)");
            gy.addColorStop(1, "#000000");
            ctx.fillStyle = gy;
            ctx.fillRect(0, 0, width, height);
        }
    }

    MouseArea {
        anchors.fill: parent
        onPressed: handle(mouse)
        onPositionChanged: if (pressed) handle(mouse)
        function handle(mouse) {
            var cx = root.width / 2, cy = root.height / 2;
            var dx = mouse.x - cx, dy = mouse.y - cy;
            var dist = Math.sqrt(dx * dx + dy * dy);
            if (dist >= root.innerR && dist <= root.outerR) {
                // In the hue ring.
                var a = Math.atan2(dy, dx);
                if (a < 0) a += 2 * Math.PI;
                var deg = root.clampHueDeg(a / (2 * Math.PI) * 360);
                root.hue = deg / 360;
                square.requestPaint();
                root.emitColor();
            } else if (mouse.x >= root.squareX && mouse.x <= root.squareX + root.squareSize
                       && mouse.y >= root.squareY && mouse.y <= root.squareY + root.squareSize) {
                root.sat = Math.max(0, Math.min(1, (mouse.x - root.squareX) / root.squareSize));
                root.val = Math.max(0, Math.min(1, 1 - (mouse.y - root.squareY) / root.squareSize));
                root.emitColor();
            }
        }
    }
}
