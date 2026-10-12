# Redrob Canvas — Windows setup

`redrob_canvas_setup.cpp` is the branded Windows setup and its uninstaller, in one small native Win32 app
(Direct2D, DirectWrite, WIC; nothing beyond the Windows SDK). The window is adapted from the Redrob Browser
setup (`redrob-labs/redrob-browser`, `installer/windows/`), our own code; the install itself is new, because
Canvas has no Chromium `mini_installer` to hand it to.

## What it does

- **Install**, per user, no administrator rights, into `%LOCALAPPDATA%\Programs\Redrob Canvas`. The payload is
  the release's own signed zip, unpacked with System32's `tar.exe` beside the old install and swapped in, so a
  failed update never leaves a half-updated folder. A running Canvas is never closed for the person: the
  setup asks them to save and close it, then Retry.
- **Shortcuts** in the Start menu and on the desktop, and a **Settings > Apps** entry
  (`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\RedrobCanvas`: name, version, publisher, icon,
  size, `UninstallString`).
- **Uninstall**: the same app built without a payload, written into the install folder as
  `Uninstall Redrob Canvas.exe`. It asks first, with "Also delete settings, the console connection and the AI
  engine" off by default, then removes the shortcuts, the entry and the folder. Settings are Qt's for
  organization `Redrob`, application `Redrob Canvas`: `HKCU\Software\Redrob\Redrob Canvas` and
  `Redrob\Redrob Canvas` under `%LOCALAPPDATA%` and `%APPDATA%`; the shared `Redrob` parents are left alone.
- **Redrob Code**, opt-in on the first screen, off by default: downloaded from its latest release, checked
  against that release's `SHA256SUMS`, installed silently per user. Its failure never fails the Canvas install.

## Flags

| Flag | Does |
| --- | --- |
| (none) | the setup window; the uninstaller build opens its confirm screen |
| `--uninstall` | the uninstall window (what Settings > Apps runs) |
| `--silent` | the same work with no window; the exit code is the result (0 = success, `0xE0000001` = Canvas is running). Never opens Canvas afterwards |
| `--delete-data` | with `--uninstall --silent`: also delete settings and the AI engine |
| `--with-redrob-code` | start with the Redrob Code box ticked |
| `--demo`, `--demo-error`, `--demo-code-error`, `--demo-uninstall` | the window's states on a timer, touching nothing |

## Build

Visual Studio with the C++ x64 toolset (found with `vswhere`) and Python 3:

```
py -3 installer\windows\build.py --out <dir> --payload redrob-canvas-X.Y.Z-windows-x86_64.zip --version X.Y.Z
```

The release workflow builds it in two stages so every binary is signed before it is embedded:
`--uninstaller-only`, sign that, then `--payload … --uninstaller <signed exe> --name …-setup.exe`, sign that
(`.github/workflows/release.yml`, Windows job).

The art is committed: `res/background.png` is `res/background.html` rasterised at 1320x800, `res/icon.png` and
`resources/icons/redrob-canvas.ico` come from `tools/build_installer_assets.py`.

## Verify

`verify-install.ps1` runs on a Windows machine against a built setup: install, shortcuts, the Settings entry,
the signed exe, an upgrade over the install, an update refused while Canvas runs, and the uninstall. It does
not exercise `--delete-data`, which would delete that machine's real Canvas settings.
