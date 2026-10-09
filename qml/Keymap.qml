// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtCore

// Keyboard layouts. People arriving from Photoshop and from Illustrator expect different keys for
// the same letter (M is a rectangle selection in one and a rectangle shape in the other), so one
// table cannot serve both. This file is the only place a window shortcut's keys are written down:
// Main.qml binds `keymap.keys("<command>")`, the shortcut list reads the same rows, and a test
// checks the tables against each other and against Main.qml.
//
// Format, kept one entry per line because the test reads it as text:
//   "<command>": { ps: [<keys>], ai: [<keys>] },
// An empty list means "no key in that layout". StandardKey values are platform keys (Ctrl+Z on
// Linux and Windows, Cmd+Z on macOS).
Item {
    id: root
    visible: false

    // "photoshop" or "illustrator". `profileChosen` is false until the first-run question is answered.
    property string profile: "photoshop"
    property bool profileChosen: false
    readonly property bool illustrator: profile === "illustrator"
    readonly property string profileName: illustrator ? "Illustrator" : "Photoshop"

    Settings {
        category: "keymap"
        property alias profile: root.profile
        property alias profileChosen: root.profileChosen
    }

    function choose(name) {
        profile = name === "illustrator" ? "illustrator" : "photoshop";
        profileChosen = true;
    }

    // ---- Tool keys ----
    // Photoshop: a letter picks the tool last used in its group; Shift+letter steps through the group.
    readonly property var psTools: ({
        "V": ["transform"],
        "M": ["rectangle", "ellipse"],
        "L": ["lasso", "polygon", "scissors"],
        "W": ["wand", "fgselect"],
        "C": ["crop"],
        "I": ["picker", "measure"],
        "J": ["heal"],
        "B": ["brush", "lazybrush", "mixer"],
        "S": ["clone"],
        "E": ["eraser"],
        "G": ["gradient", "fill", "enclose"],
        "O": ["dodge", "burn"],
        "P": ["pen"],
        "T": ["text"],
        "U": ["shape"],
        "H": ["hand"],
        "R": ["rotateview"],
        "Z": ["zoom"]
    })
    // Illustrator: one tool per key. Shift+E (Eraser) and Shift+R (Warp) are Illustrator's own keys.
    readonly property var aiTools: ({
        "V": ["transform"],
        "P": ["pen"],
        "T": ["text"],
        "M": ["shape"],
        "B": ["brush"],
        "E": ["perspective"],
        "Shift+E": ["eraser"],
        "I": ["picker"],
        "G": ["gradient"],
        "K": ["fill"],
        "Q": ["lasso"],
        "Y": ["wand"],
        "H": ["hand"],
        "Z": ["zoom"],
        "Shift+R": ["warp"]
    })
    readonly property var toolKeys: illustrator ? aiTools : psTools

    // Keys of the other program that this app has no feature for. Pressing one says so in the status
    // bar instead of doing nothing, so a habit is never silently swallowed.
    readonly property var psMissing: ({
        "A": "Path Selection",
        "Q": "Quick Mask",
        "F": "Screen modes"
    })
    readonly property var aiMissing: ({
        "A": "Direct Selection (use the Pen tool, P)",
        "N": "Pencil (use the Brush, B)",
        "L": "Ellipse (use the Shape tool, M)",
        "R": "Rotate (use Free Transform, E)",
        "S": "Scale (use Free Transform, E)",
        "O": "Reflect",
        "W": "Blend",
        "C": "Scissors",
        "J": "Curvature",
        "U": "Mesh",
        "Shift+B": "Blob Brush",
        "Shift+C": "Anchor Point",
        "Shift+M": "Shape Builder",
        "Shift+O": "Artboard (Layer > New artboard)",
        "Shift+S": "Symbol Sprayer",
        "Shift+W": "Width",
        "Ctrl+D": "Transform Again",
        "Ctrl+J": "Join",
        "Ctrl+K": "Preferences",
        "Ctrl+T": "Character panel",
        "Ctrl+Y": "Outline view",
        "Ctrl+8": "Make Compound Path",
        "Ctrl+Shift+G": "Ungroup",
        "Ctrl+Shift+O": "Create Outlines"
    })
    readonly property var missing: illustrator ? aiMissing : psMissing

    // ---- Command keys ----
    readonly property var commands: ({
        "edit.undo": { ps: [StandardKey.Undo], ai: [StandardKey.Undo] },
        "edit.redo": { ps: [StandardKey.Redo], ai: [StandardKey.Redo] },
        "file.new": { ps: [StandardKey.New], ai: [StandardKey.New] },
        "file.open": { ps: [StandardKey.Open], ai: [StandardKey.Open] },
        "file.save": { ps: [StandardKey.Save], ai: [StandardKey.Save] },
        "file.saveAs": { ps: ["Ctrl+Shift+S"], ai: ["Ctrl+Shift+S"] },
        "edit.copy": { ps: [StandardKey.Copy], ai: [StandardKey.Copy] },
        "edit.cut": { ps: [StandardKey.Cut], ai: [StandardKey.Cut] },
        "edit.paste": { ps: [StandardKey.Paste], ai: [StandardKey.Paste, "Ctrl+F", "Ctrl+B", "Ctrl+Shift+V"] },
        "edit.clear": { ps: ["Delete", "Backspace"], ai: ["Delete", "Backspace"] },
        "edit.fillForeground": { ps: ["Alt+Backspace"], ai: [] },
        "edit.fillBackground": { ps: ["Ctrl+Backspace"], ai: [] },
        "edit.contentAware": { ps: ["Shift+F5"], ai: [] },
        "select.all": { ps: ["Ctrl+A"], ai: ["Ctrl+A"] },
        "select.none": { ps: ["Ctrl+D"], ai: ["Ctrl+Shift+A"] },
        "select.invert": { ps: ["Ctrl+Shift+I"], ai: [] },
        "layer.new": { ps: ["Ctrl+Shift+N"], ai: ["Ctrl+L"] },
        "layer.group": { ps: ["Ctrl+G"], ai: ["Ctrl+G"] },
        "layer.clip": { ps: ["Ctrl+Alt+G"], ai: ["Ctrl+7"] },
        "layer.duplicate": { ps: ["Ctrl+J"], ai: [] },
        "layer.mergeDown": { ps: ["Ctrl+E"], ai: [] },
        "layer.mergeVisible": { ps: ["Ctrl+Shift+E"], ai: [] },
        "layer.forward": { ps: ["Ctrl+]"], ai: ["Ctrl+]"] },
        "layer.backward": { ps: ["Ctrl+["], ai: ["Ctrl+["] },
        "layer.front": { ps: ["Ctrl+Shift+]", "Ctrl+}"], ai: ["Ctrl+Shift+]", "Ctrl+}"] },
        "layer.back": { ps: ["Ctrl+Shift+[", "Ctrl+{"], ai: ["Ctrl+Shift+[", "Ctrl+{"] },
        "layer.lock": { ps: [], ai: ["Ctrl+2"] },
        "layer.hide": { ps: [], ai: ["Ctrl+3"] },
        "layer.showAll": { ps: [], ai: ["Ctrl+Alt+3"] },
        "transform.free": { ps: ["Ctrl+T"], ai: [] },
        "image.size": { ps: ["Ctrl+Alt+I"], ai: [] },
        "image.canvasSize": { ps: ["Ctrl+Alt+C"], ai: ["Ctrl+Alt+P"] },
        "adjust.invert": { ps: ["Ctrl+I"], ai: [] },
        "adjust.desaturate": { ps: ["Ctrl+Shift+U"], ai: [] },
        "adjust.levels": { ps: ["Ctrl+L"], ai: [] },
        "adjust.curves": { ps: ["Ctrl+M"], ai: [] },
        "adjust.hueSaturation": { ps: ["Ctrl+U"], ai: [] },
        "adjust.colorBalance": { ps: ["Ctrl+B"], ai: [] },
        "view.fit": { ps: ["Ctrl+0"], ai: ["Ctrl+0"] },
        "view.actualPixels": { ps: ["Ctrl+1"], ai: ["Ctrl+1"] },
        "view.zoomIn": { ps: ["Ctrl+=", "Ctrl++", StandardKey.ZoomIn], ai: ["Ctrl+=", "Ctrl++", StandardKey.ZoomIn] },
        "view.zoomOut": { ps: ["Ctrl+-", StandardKey.ZoomOut], ai: ["Ctrl+-", StandardKey.ZoomOut] },
        "view.panels": { ps: ["Tab"], ai: ["Tab"] },
        "view.rulers": { ps: ["Ctrl+R"], ai: ["Ctrl+R"] },
        "view.guides": { ps: ["Ctrl+;"], ai: ["Ctrl+;"] },
        "view.snap": { ps: ["Ctrl+Shift+;", "Ctrl+:"], ai: ["Ctrl+U"] },
        "view.proof": { ps: ["Ctrl+Y"], ai: [] },
        "view.gamut": { ps: ["Ctrl+Shift+Y"], ai: [] },
        "color.swap": { ps: ["X"], ai: ["X", "Shift+X"] },
        "color.default": { ps: ["D"], ai: ["D"] },
        "brush.eraseMode": { ps: ["Shift+E"], ai: [] },
        "help.shortcuts": { ps: ["F1"], ai: ["F1"] }
    })

    function keys(command) {
        const entry = commands[command];
        if (entry === undefined) {
            console.warn("keymap: unknown command", command);
            return [];
        }
        return illustrator ? entry.ai : entry.ps;
    }

    // Shortcut-list rows for the Illustrator layout: [keys, what it does]. The Photoshop rows live in
    // ShortcutsDialog.qml, where they also record what this app lacks.
    readonly property var aiRows: [
        ["V", "Selection (move)"],
        ["P", "Pen"],
        ["T", "Type"],
        ["M", "Rectangle / shapes"],
        ["B", "Paintbrush"],
        ["E", "Free Transform"],
        ["Shift+E", "Eraser"],
        ["I", "Eyedropper"],
        ["G", "Gradient"],
        ["K", "Live Paint Bucket (fill)"],
        ["Q", "Lasso"],
        ["Y", "Magic Wand"],
        ["H", "Hand"],
        ["Z", "Zoom"],
        ["Shift+R", "Warp"],
        ["Space (hold)", "Hand while held"],
        ["Ctrl (hold)", "Selection tool while held"],
        ["X / Shift+X", "Swap fill and stroke (foreground and background)"],
        ["D", "Default fill and stroke"],
        ["Ctrl+N / Ctrl+O / Ctrl+S", "New / open / save"],
        ["Ctrl+Shift+S", "Save as"],
        ["Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"],
        ["Ctrl+X / Ctrl+C / Ctrl+V", "Cut / copy / paste"],
        ["Ctrl+F / Ctrl+B / Ctrl+Shift+V", "Paste in front / in back / in place"],
        ["Ctrl+A", "Select all"],
        ["Ctrl+Shift+A", "Deselect"],
        ["Ctrl+G", "Group"],
        ["Ctrl+7", "Make / release clipping mask"],
        ["Ctrl+] / Ctrl+[", "Bring forward / send backward"],
        ["Ctrl+Shift+] / Ctrl+Shift+[", "Bring to front / send to back"],
        ["Ctrl+2", "Lock the selected layer"],
        ["Ctrl+3", "Hide the selected layer"],
        ["Ctrl+Alt+3", "Show all layers"],
        ["Ctrl+L", "New layer"],
        ["Ctrl+Alt+P", "Document setup (canvas size)"],
        ["Ctrl+R", "Rulers"],
        ["Ctrl+;", "Show / hide guides"],
        ["Ctrl+U", "Smart guides (snap)"],
        ["Ctrl+0 / Ctrl+1", "Fit artboard / 100%"],
        ["Ctrl+= / Ctrl+-", "Zoom in / out"],
        ["Tab", "Hide / show panels"],
        ["Delete / Backspace", "Delete"],
        ["F1", "This list"]
    ]
}
