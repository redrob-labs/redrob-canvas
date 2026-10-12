# Qt in Redrob Canvas releases

Every Redrob Canvas release binary ships Qt **6.11.2** beside it, dynamically linked:

| Download | Where Qt is | Replacing it |
| --- | --- | --- |
| Windows `.zip` and setup | the `.dll` files and plugin folders next to `bin\Redrob Canvas.exe` (`windeployqt`) | put your own build of the same Qt modules in their place |
| macOS `.zip` and `.dmg` | `Redrob Canvas.app/Contents/Frameworks` and `PlugIns` (`macdeployqt`) | replace the frameworks; re-sign the bundle afterwards, since macOS checks the signature |
| Linux `.tar.gz` and `.deb` | `lib/`, `plugins/`, `qml/` beside `bin/` (`/opt/redrob-canvas` for the `.deb`) | replace the libraries; `bin/qt.conf` names the folders |

Modules the app itself links: Core, Concurrent, Gui, Network, Qml, Quick, QuickControls2, Svg. The deploy
tools add what those load at run time (platform, image-format, TLS and style plugins; the QML modules).

**Licence.** These Qt modules are used under the GNU Lesser General Public License version 3
(`LGPL-3.0.txt` in this folder, shipped with every download as `Qt-LGPL-3.0.txt`, and `Qt-README.md` for this
note, beside `LICENSE`, which is the GPL-3.0 the LGPL builds on). Qt is copyright The Qt Company Ltd. and other contributors. Qt's own
libraries carry third-party code (for example HarfBuzz, FreeType, libpng, zlib, PCRE2); their notices are
listed in Qt's documentation for 6.11 under "Licenses Used in Qt".

**Source.** Redrob Canvas does not modify Qt. Its corresponding source is the unmodified Qt 6.11.2 source the
Qt Project publishes at https://download.qt.io/archive/qt/6.11/6.11.2/single/ (`qt-everywhere-src-6.11.2`);
this note, shipped beside every binary, is the direction to it that GPL-3.0 section 6(d) asks for. That
server is the Qt Project's, so the duty to keep the source reachable stays ours: if it ever goes away, the
same archive is to be attached to the Redrob Canvas releases.
