# Architecture

Redrob Canvas is one desktop process: an egui/eframe shell that hosts three complete editors as
modes, plus two cross-mode layers (the agent and the AI tools) that are Redrob's own. The editors are
upstream code, vendored whole; the shell owns the window and the seams between them.

```text
 apps/redrob-canvas  (eframe window, ai.redrob.canvas)
 +-----------------------------------------------------------------------+
 | top bar: "Redrob Canvas"  [Photo] [Vector] [Layout]       right slot  |
 +-----------------------------------------------------------------------+
 | active mode's ui()                                                    |
 |   Photo  = photocraft_ui_egui::PhotocraftApp   (crafts/photocraft)    |
 |   Vector = vectorcraft_ui_egui::VectorcraftApp (crafts/vectorcraft)   |
 |   Layout = designcraft_ui_egui::DesignApp      (crafts/designcraft)   |
 +-----------------------------------------------------------------------+
   services: config dir, prefs, persistence       crates/redrob-agent  (Redrob Code daemon)
   crafts/craft-libs: docking, CMS, print, PDF    crates/redrob-ai     (IOPaint, vectorize)
```

## The shell (`apps/redrob-canvas`)

`main.rs` parses `--mode photo|vector|layout` and `--version`, builds the eframe `NativeOptions`
(1440 x 900, minimum 760 x 480, wgpu renderer, `persistence_path` under the config directory) and
runs `ShellApp`. `shell.rs` holds the `Mode` enum, the three lazily created editors and the top bar;
`services.rs` is the one place the config directory is resolved
(`~/Library/Application Support/Redrob Canvas`, `%APPDATA%\Redrob Canvas`,
`$XDG_CONFIG_HOME/redrob-canvas`), with each editor given a folder inside it so their own
preferences and presets live where their upstream code expects.

Each editor is constructed on first switch (`Session::new()` plus the ported `Services`) and kept
alive, so switching modes preserves open documents. Only the active mode receives input and paints;
PhotoCraft additionally gets `logic()` and `raw_input_hook()` forwarded and the wgpu render state
handed to `set_wgpu`, as its own binary does. The editors draw their own side panels inside the `Ui`
the shell gives them; the shell does not reach into their document models.

What the shell deliberately does not do yet: run the editors' MCP/control servers (each upstream
binary has one; a TODO names the path), and open a file into a mode by extension (`docs/formats.md`
says how that routing will work).

## The editors (`crafts/*`)

Four git subtrees, each squash-merged at a pinned upstream commit (`UPSTREAM.md`). Only their
`crates/*` are workspace members; their `apps/`, `xtask` and web shells are excluded by
`tools/merge_workspaces.py`, which generates the root `Cargo.toml` from the four parked
`Cargo.upstream.toml` files: dependencies unioned, craft-libs git dependencies turned into path
dependencies on the subtree, and the `krilla` / `hayro` patches pointed at
`crafts/craft-libs/vendor/`. The result is one cargo workspace, one lock file, one
`THIRD_PARTY_NOTICES.md`.

Nothing in Redrob Canvas edits a file under `crafts/` directly; a change an editor needs goes
upstream, and arrives back by `git subtree pull`. That is what keeps each pull a three-way merge
instead of a conflict hunt.

## The agent (`crates/redrob-agent`)

Pure Rust, no UI. A streaming client for the shared Redrob Code daemon's OpenAI-compatible
`/v1/chat/completions` (`redrob-engine-client`, pinned by tag), a tool loop, and an approvals model:
a mutating tool call becomes a proposal the person applies, never a silent edit. Every request
carries the daemon's own authorization and `x-redrob-app: canvas`; this app holds no console key and
has no direct console path. The daemon record and binary are found through `redrob-engine-client`
(`REDROB_RUN_DIR` and `REDROB_CODE_BIN` point tests at a private daemon). The chat panel that
surfaces this in the shell's right slot is 0.7 work.

## The AI tools (`crates/redrob-ai`, in progress)

Pure Rust, no UI. Lifecycle of a local IOPaint engine the app installs into its data directory
(`uv venv` or `python3 -m venv`, pinned torch and iopaint versions) and starts on a free loopback
port with the plug-ins each tool needs; an HTTP client for inpaint, outpaint, erase, upscale
(RealESRGAN), face restore, remove background and subject select; and a vectorizer for the Vector
mode. Images travel as PNG data URLs to `127.0.0.1` only. The egui panel that drives it lives in the
shell and reaches whichever mode is active through a small trait the editors already satisfy
(current image and selection in, pixels out).

## Design system

The shell paints from the Redrob design-system tokens vendored under `third_party/redrob-ui/` and
pinned in `DESIGN_SYSTEM_PIN.json`; the app icon, lockups and splash under `resources/` are built by
`scripts/build-brand-assets.py` from the design system's shipped files (the Redrob favicon tile, the
Primary lockup) and pinned in `brand/SOURCES.json`. `tools/check_design_system.py` fails CI on a drifted
pin, a hand-edited or unpinned brand file, GPL text in a generated SVG, or a colour literal in the
shell's chrome. The editors keep their own themes inside their modes.

## What is gone

Everything before 0.7.0: the Qt Quick shell and its C ABI, `redrob-core` (the GPL-era document
model that translated Krita and copied Graphite and GEGL), the babl and lcms2 adapters, the
golden-output harnesses against Krita and GIMP, and the `.rrg` project format. The reasons and the
feature-by-feature account of what was lost and regained are in `CHANGELOG.md` under 0.7.0.
