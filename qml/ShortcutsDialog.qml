// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Help > Keyboard Shortcuts (S2). Photoshop's layout is the reference. Rows marked "not yet" are
// Photoshop shortcuts whose feature this app does not have; they are listed so the gap is visible
// rather than a key that silently does nothing. A test checks this table against the real
// bindings in Main.qml in both directions.
Dialog {
    id: root
    objectName: "shortcutsDialog"
    required property var tokens
    title: "Keyboard Shortcuts (Photoshop layout)"
    modal: true
    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(720, parent.width - 40)
    height: Math.min(620, parent.height - 40)
    standardButtons: Dialog.Close

    // [keys, what it does, state]. state: "" = works, "not yet" = no such feature here, or a note.
    readonly property var rows: [
        ["V", "Move layer", ""],
        ["M / Shift+M", "Rectangle / ellipse selection", ""],
        ["L / Shift+L", "Lasso / polygon / intelligent scissors", ""],
        ["W / Shift+W", "Magic wand / foreground select", ""],
        ["C", "Crop", ""],
        ["I / Shift+I", "Eyedropper / measure", ""],
        ["J", "Healing brush", ""],
        ["B / Shift+B", "Brush / lazybrush", ""],
        ["S", "Clone stamp", ""],
        ["E", "Eraser", ""],
        ["Shift+E", "Toggle erase mode while painting", ""],
        ["G / Shift+G", "Gradient / paint bucket / enclose and fill", ""],
        ["O / Shift+O", "Dodge / burn", ""],
        ["P", "Pen", ""],
        ["T", "Text", ""],
        ["U", "Shape", ""],
        ["H", "Hand", ""],
        ["Z", "Zoom (Alt-click zooms out)", ""],
        ["R", "Rotate view tool", "not yet: use View > Rotate view"],
        ["[ / ]", "Brush size down / up", ""],
        ["Shift+[ / Shift+]", "Brush hardness down / up", ""],
        ["Alt+right-drag", "Brush size (left/right) and hardness (up/down)", ""],
        ["Ctrl+Alt+drag", "Brush size and hardness (when the desktop takes Alt+right-drag)", ""],
        ["1 … 9, 0", "Opacity 10% … 90%, 100% (brush, or the layer for other tools)", ""],
        ["Shift+1 … 0", "Brush flow", "not yet: no flow setting"],
        ["Space (hold)", "Hand tool while held", ""],
        ["Alt (hold)", "Eyedropper while held, with a painting tool", ""],
        ["Ctrl (hold)", "Move tool while held", "not yet: Ctrl-click sets the clone source"],
        ["X", "Swap foreground and background colours", ""],
        ["D", "Default colours (black / white)", ""],
        ["Ctrl+N", "New document", ""],
        ["Ctrl+O", "Open", ""],
        ["Ctrl+S", "Save", ""],
        ["Ctrl+Shift+S", "Save as", ""],
        ["Ctrl+Z / Ctrl+Shift+Z", "Undo / redo", ""],
        ["Ctrl+X / Ctrl+C", "Cut / copy (the selection, or the whole layer)", ""],
        ["Ctrl+V", "Paste as a new layer (in place if it came from here)", ""],
        ["Ctrl+A", "Select all", ""],
        ["Ctrl+D", "Deselect", ""],
        ["Ctrl+Shift+I", "Invert selection", ""],
        ["Ctrl+T", "Free transform (perspective handles)", ""],
        ["Ctrl+Shift+N", "New layer", ""],
        ["Ctrl+G", "New group", "adds an empty group: no multi-layer selection yet"],
        ["Ctrl+J", "Duplicate layer (a group with everything in it)", ""],
        ["Ctrl+E", "Merge down (raster into the raster layer below)", ""],
        ["Ctrl+Shift+E", "Merge visible (hidden layers stay; Layer > Flatten image drops them)", ""],
        ["Ctrl+I", "Invert colours", ""],
        ["Ctrl+Shift+U", "Desaturate", ""],
        ["Ctrl+L", "Levels", ""],
        ["Ctrl+M", "Curves", ""],
        ["Ctrl+U", "Hue / saturation", ""],
        ["Ctrl+B", "Colour balance", ""],
        ["Delete / Backspace", "Clear", ""],
        ["Alt+Backspace", "Fill with the foreground colour", ""],
        ["Ctrl+Backspace", "Fill with the background colour", ""],
        ["Ctrl+0", "Fit on screen", ""],
        ["Ctrl+1", "100%", ""],
        ["Ctrl+= / Ctrl+-", "Zoom in / out", ""],
        ["Tab", "Hide / show panels", ""],
        ["F1", "This list", ""]
    ]

    ListView {
        anchors.fill: parent
        clip: true
        model: root.rows
        delegate: RowLayout {
            required property var modelData
            width: ListView.view.width
            spacing: 12
            Label {
                text: modelData[0]
                font.family: "monospace"
                color: root.tokens.inkPrimary
                Layout.preferredWidth: 190
            }
            Label {
                text: modelData[1]
                color: modelData[2].startsWith("not yet") ? root.tokens.inkMuted : root.tokens.inkPrimary
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
            Label {
                text: modelData[2]
                color: root.tokens.inkMuted
                wrapMode: Text.Wrap
                Layout.preferredWidth: 200
            }
        }
    }
}
