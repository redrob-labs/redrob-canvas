// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Controls

// The window's menu bar. Every item calls an existing bridge method or opens an existing dialog;
// none owns behaviour of its own. Moved out of Main.qml in P12.
MenuBar {
    id: root
    objectName: "mainMenuBar"
    // Everything the menus drive lives in Main.qml and arrives here by property (P12).
    required property var app
    required property var canvasView
    required property var sideTabs
    required property var saveDialog
    required property var openDialog
    required property var importFileDialog
    required property var exportDialog
    required property var filters
    required property var textDialog
    required property var actionSaveDialog
    required property var actionPlayDialog
    required property var shortcutsList
    required property var newDocument
    required property var proofDialog
    required property var cmykExport
    // U7.
    required property var cmykPsdExport
    required property var artboardExport
    required property var sizeDialog
    required property var strokeDialog
    required property var colorRangeDialog
    Menu {
        title: qsTr("&File")
        // No item sets a shortcut property: the window's Shortcut objects already own Ctrl+O/S/Z,
        // and a second binding of the same key makes Qt treat it as ambiguous and fire neither.
        Action { text: qsTr("&New…  (Ctrl+N)"); onTriggered: root.newDocument.openNew() }
        Action { text: qsTr("&Open…"); onTriggered: root.openDialog.open() }
        Action { text: qsTr("&Import…"); onTriggered: root.importFileDialog.open() }
        MenuSeparator {}
        Action {
            text: qsTr("&Save")
            onTriggered: editor.currentFile.length > 0 ? editor.saveProject() : root.saveDialog.open()
        }
        Action { text: qsTr("Save &As…"); onTriggered: root.saveDialog.open() }
        Action { text: qsTr("&Export…"); onTriggered: root.exportDialog.open() }
        Action {
            text: qsTr("Export C&MYK TIFF… (uses the proof profile)")
            enabled: editor.proofProfileName.length > 0
            onTriggered: root.cmykExport.open()
        }
        Action {
            text: qsTr("Export CMYK &PSD… (layers, uses the proof profile)")
            enabled: editor.proofProfileName.length > 0
            onTriggered: root.cmykPsdExport.open()
        }
        Action { text: qsTr("Export &artboards… (one PNG each)"); onTriggered: root.artboardExport.open() }
        MenuSeparator {}
        Action { text: qsTr("&Quit"); onTriggered: Qt.quit() }
    }
    Menu {
        title: qsTr("&Edit")
        Action { text: qsTr("&Undo"); enabled: editor.canUndo; onTriggered: editor.undo() }
        Action { text: qsTr("&Redo"); enabled: editor.canRedo; onTriggered: editor.redo() }
        MenuSeparator {}
        Action { text: qsTr("Cu&t  (Ctrl+X)"); enabled: editor.activeNodeCanEditRaster; onTriggered: editor.cutSelection() }
        Action { text: qsTr("&Copy  (Ctrl+C)"); onTriggered: editor.copySelection() }
        Action { text: qsTr("&Paste as new layer  (Ctrl+V)"); onTriggered: editor.pasteClipboard() }
        Action {
            text: qsTr("Content-a&ware fill  (Shift+F5)")
            enabled: editor.activeNodeCanEditRaster && editor.selectionActive
            onTriggered: editor.contentAwareFill()
        }
        Action {
            text: qsTr("&Puppet warp (rigid pins for the n-point tool)")
            checkable: true
            checked: root.app.puppetRigid
            onTriggered: {
                root.app.puppetRigid = !root.app.puppetRigid;
                if (root.app.puppetRigid)
                    root.app.activeTool = "npoint";
            }
        }
        Action {
            text: qsTr("&Stroke selection…")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: root.strokeDialog.open()
        }
        Action {
            text: qsTr("&Clear layer")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: editor.clearActiveLayer()
        }
        Action {
            text: qsTr("Clear &outside selection")
            enabled: editor.activeNodeCanEditRaster && editor.selectionActive
            onTriggered: editor.clearOutsideSelection()
        }
        MenuSeparator {}
        // Keyboard layout: Photoshop's or Illustrator's keys (Keymap.qml).
        Menu {
            title: qsTr("&Keyboard layout")
            Action {
                objectName: "keymapPhotoshopAction"
                text: qsTr("&Photoshop")
                checkable: true
                checked: !root.app.keymap.illustrator
                onTriggered: root.app.keymap.choose("photoshop")
            }
            Action {
                objectName: "keymapIllustratorAction"
                text: qsTr("&Illustrator")
                checkable: true
                checked: root.app.keymap.illustrator
                onTriggered: root.app.keymap.choose("illustrator")
            }
        }
    }
    Menu {
        title: qsTr("&Select")
        Action { text: qsTr("&All"); onTriggered: editor.selectAll() }
        Action { text: qsTr("&None"); onTriggered: editor.clearSelection() }
        Action { text: qsTr("&Invert"); onTriggered: editor.invertSelection() }
        MenuSeparator {}
        Action { text: qsTr("&Grow by 1 px"); onTriggered: editor.growSelection(1) }
        Action { text: qsTr("&Shrink by 1 px"); onTriggered: editor.shrinkSelection(1) }
        Action { text: qsTr("&Color range…"); onTriggered: root.colorRangeDialog.open() }
        Action { text: qsTr("&Feather by 2 px"); onTriggered: editor.featherSelection(2) }
    }
    Menu {
        title: qsTr("&Layer")
        Action { text: qsTr("New &raster layer"); onTriggered: editor.addLayer() }
        Action { text: qsTr("New &group"); onTriggered: editor.addGroup() }
        Action {
            // L8: the selection's box when there is one, else the whole canvas, on white.
            text: qsTr("New a&rtboard (selection or canvas)")
            onTriggered: editor.newArtboard(0, 0, 0, 0, "white")
        }
        Action { text: qsTr("New &text…"); onTriggered: root.textDialog.openNew(24, 24) }
        Action {
            text: qsTr("&Delete layer")
            enabled: editor.activeLayerId.length > 0
            onTriggered: editor.deleteSelectedLayers()
        }
        Action { text: qsTr("Group &layers  (Ctrl+G)"); onTriggered: editor.groupSelectedLayers() }
        Action { text: qsTr("Create / release clipping &mask  (Ctrl+Alt+G)"); onTriggered: editor.toggleClippingMask() }
        Action { text: qsTr("Lin&k layers"); onTriggered: editor.linkSelectedLayers(true) }
        Action {
            text: qsTr("Convert to smart &object")
            enabled: editor.activeNodeKind === "raster"
            onTriggered: editor.convertToSmartObject(editor.activeLayerId)
        }
        Action {
            text: qsTr("Rasteri&ze smart object")
            onTriggered: editor.rasterizeSmartObject(editor.activeLayerId)
        }
        Action { text: qsTr("U&nlink layers"); onTriggered: editor.linkSelectedLayers(false) }
        Action {
            text: qsTr("D&uplicate layer  (Ctrl+J)")
            enabled: editor.activeLayerId.length > 0
            onTriggered: editor.duplicateLayer(editor.activeLayerId)
        }
        Action {
            text: qsTr("&Merge down  (Ctrl+E)")
            enabled: editor.activeLayerId.length > 0
            onTriggered: editor.mergeDown(editor.activeLayerId)
        }
        Action { text: qsTr("Merge &visible  (Ctrl+Shift+E)"); onTriggered: editor.mergeVisible() }
        // Over the background colour (X / D set it), as a flattened image has no transparency.
        Action { text: qsTr("&Flatten image"); onTriggered: editor.flattenImage(root.app.backgroundColor) }
        MenuSeparator {}
        Action {
            text: qsTr("Flip &horizontally")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: editor.flipActive(true, false)
        }
        Action {
            text: qsTr("Flip &vertically")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: editor.flipActive(false, true)
        }
        Action {
            text: qsTr("Rotate 90° &clockwise")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: editor.rotateActive(90, root.app.samplingMode)
        }
        Action {
            text: qsTr("Rotate 90° counter-clock&wise")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: editor.rotateActive(-90, root.app.samplingMode)
        }
        Action {
            text: qsTr("Rotate &180°")
            enabled: editor.activeNodeCanEditRaster
            onTriggered: editor.rotateActive(180, root.app.samplingMode)
        }
    }
    Menu {
        // Image-wide conversions, as GIMP's and Photoshop's Image > Mode menus. Checked items show
        // the document's current mode and precision.
        title: qsTr("&Image")
        Action { text: qsTr("Image &size…  (Ctrl+Alt+I)"); onTriggered: root.sizeDialog.openFor("image") }
        Action { text: qsTr("&Canvas size…  (Ctrl+Alt+C)"); onTriggered: root.sizeDialog.openFor("canvas") }
        Action {
            text: qsTr("Crop to s&election")
            enabled: editor.selectionActive
            onTriggered: editor.cropToSelection()
        }
        MenuSeparator {}
        Menu {
            title: qsTr("&Mode")
            Action {
                text: qsTr("&RGB")
                checkable: true
                checked: editor.colorMode === "rgb"
                onTriggered: editor.convertColorMode("rgb")
            }
            Action {
                text: qsTr("&Grayscale")
                checkable: true
                checked: editor.colorMode === "grayscale"
                onTriggered: editor.convertColorMode("grayscale")
            }
            Action {
                // L5c: printable colours only, through the View > Proof setup profile.
                text: editor.proofProfileName.length > 0 ? qsTr("&CMYK (%1)").arg(editor.proofProfileName)
                                                         : qsTr("&CMYK (choose View > Proof setup first)")
                checkable: true
                checked: editor.colorMode === "cmyk"
                enabled: editor.proofProfileName.length > 0
                onTriggered: editor.convertColorMode("cmyk")
            }
            MenuSeparator {}
            Action {
                text: qsTr("&Indexed, 256 colours from the image")
                checkable: true
                checked: editor.colorMode === "indexed"
                onTriggered: editor.convertColorMode("indexed", "generate", 256, "floyd_steinberg")
            }
            Action { text: qsTr("Indexed, &web palette"); onTriggered: editor.convertColorMode("indexed", "web", 0, "floyd_steinberg") }
            Action { text: qsTr("Indexed, &black and white"); onTriggered: editor.convertColorMode("indexed", "mono", 0, "floyd_steinberg") }
        }
        Menu {
            title: qsTr("&Precision")
            Action {
                text: qsTr("&8-bit")
                checkable: true
                checked: editor.precision === "u8"
                onTriggered: editor.setDocumentPrecision("u8")
            }
            Action {
                text: qsTr("&16-bit")
                checkable: true
                checked: editor.precision === "u16"
                onTriggered: editor.setDocumentPrecision("u16")
            }
            Action {
                text: qsTr("&32-bit float")
                checkable: true
                checked: editor.precision === "f32"
                onTriggered: editor.setDocumentPrecision("f32")
            }
        }
    }
    Menu {
        // P14. Record edits, save them as an action file, play one back as a single undo step.
        title: qsTr("&Actions")
        Action {
            text: qsTr("Start &recording")
            enabled: !editor.actionRecording
            onTriggered: editor.startActionRecording()
        }
        Action {
            text: editor.actionRecording
                  ? qsTr("S&top recording (%1 steps)").arg(editor.actionStepCount)
                  : qsTr("S&top recording")
            enabled: editor.actionRecording
            onTriggered: editor.stopActionRecording()
        }
        Action {
            text: qsTr("&Save action…")
            enabled: !editor.actionRecording && editor.actionStepCount > 0
            onTriggered: root.actionSaveDialog.open()
        }
        MenuSeparator {}
        Action {
            text: qsTr("&Play action…")
            enabled: !editor.actionRecording && !editor.filterBusy
            onTriggered: root.actionPlayDialog.open()
        }
    }
    Menu {
        title: qsTr("Filte&rs")
        Action {
            text: qsTr("&Browse all filters…")
            onTriggered: root.filters.open()
        }
        Action {
            text: qsTr("&Adjustments panel")
            onTriggered: root.sideTabs.currentIndex = 0
        }
    }
    Menu {
        title: qsTr("&View")
        Action { text: qsTr("Zoom &in"); onTriggered: root.app.canvasZoom = Math.min(32, root.app.canvasZoom * 1.2) }
        Action { text: qsTr("Zoom &out"); onTriggered: root.app.canvasZoom = Math.max(0.05, root.app.canvasZoom / 1.2) }
        Action { text: qsTr("&Actual pixels"); onTriggered: { root.app.canvasZoom = 1; root.canvasView.pan = Qt.point(0, 0) } }
        MenuSeparator {}
        // View-only: the document's pixels and coordinates do not change. Keys 4, 6 and 5 as in
        // Krita, held by window Shortcuts below (a menu item binding them too would be ambiguous).
        // No keys: Photoshop's digits set opacity, so view rotation is reached from here only.
        Action { text: qsTr("Rotate view left 15°"); onTriggered: root.app.rotateView(-15) }
        Action { text: qsTr("Rotate view right 15°"); onTriggered: root.app.rotateView(15) }
        Action { text: qsTr("Reset view rotation"); onTriggered: root.canvasView.viewRotation = 0 }
        Action {
            text: qsTr("&Mirror view")
            checkable: true
            checked: root.canvasView.viewMirrored
            onTriggered: root.canvasView.viewMirrored = !root.canvasView.viewMirrored
        }
        MenuSeparator {}
        // Layers is always in view under Properties; these bring the panels back after Tab.
        Action { text: qsTr("&Layers panel"); onTriggered: root.app.panelsHidden = false }
        Action { text: qsTr("&Properties panel"); onTriggered: { root.app.panelsHidden = false; root.sideTabs.currentIndex = 0 } }
        Action { text: qsTr("A&gent panel"); onTriggered: { root.app.panelsHidden = false; root.sideTabs.currentIndex = 1 } }
        Action { text: qsTr("A&I panel"); onTriggered: { root.app.panelsHidden = false; root.sideTabs.currentIndex = 2 } }
        MenuSeparator {}
        Action { text: qsTr("Proof se&tup (CMYK profile)…"); onTriggered: root.proofDialog.open() }
        Action {
            text: editor.proofProfileName.length > 0 ? qsTr("Proof &colors: %1  (Ctrl+Y)").arg(editor.proofProfileName)
                                                     : qsTr("Proof &colors  (Ctrl+Y)")
            checkable: true
            checked: editor.proofColors
            onTriggered: editor.proofColors = !editor.proofColors
        }
        Action {
            text: qsTr("Gamut &warning  (Ctrl+Shift+Y)")
            checkable: true
            checked: editor.proofGamutWarning
            onTriggered: editor.proofGamutWarning = !editor.proofGamutWarning
        }
        Action {
            text: qsTr("&Rulers  (Ctrl+R)")
            checkable: true
            checked: root.app.rulersVisible
            onTriggered: root.app.rulersVisible = !root.app.rulersVisible
        }
        Action {
            text: qsTr("&Guides  (Ctrl+;)")
            checkable: true
            checked: root.app.guidesVisible
            onTriggered: root.app.guidesVisible = !root.app.guidesVisible
        }
        Action {
            text: qsTr("&Snap  (Ctrl+Shift+;)")
            checkable: true
            checked: root.app.snapEnabled
            onTriggered: root.app.snapEnabled = !root.app.snapEnabled
        }
        Action {
            text: qsTr("&Navigator")
            checkable: true
            checked: root.app.navigatorVisible
            onTriggered: root.app.navigatorVisible = !root.app.navigatorVisible
        }
        MenuSeparator {}
        Action { text: qsTr("&Light theme"); onTriggered: root.app.themeChoice = "light" }
        Action { text: qsTr("&Dark theme"); onTriggered: root.app.themeChoice = "dark" }
        Action { text: qsTr("&System theme"); onTriggered: root.app.themeChoice = "" }
    }
    Menu {
        title: qsTr("&Help")
        Action { text: qsTr("&Keyboard shortcuts…  (F1)"); onTriggered: root.shortcutsList.open() }
    }
}
