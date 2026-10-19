# Changelog

The release workflow's notes point here, so this file is what a download's notes actually say.
Versions follow the `vMAJOR.MINOR.PATCH` tags that trigger a release; while the major is 0 a
minor bump is where behaviour may change.

## 0.11.1 — 2026-10-19

### Fixed

- **Windows setup now makes the Start menu shortcut.** The windowed installer ran the install on a
  worker thread without COM, so the shortcut writer failed silently and Redrob Canvas never showed
  up in Start or Windows search. The silent (launcher) install was not affected.

## 0.11.0 — 2026-10-18

### Added

- **Every remaining parity row.** The feature inventories in `docs/parity/` now score photo
  445 full / 5 partial, paint 327 / 7 and vector 305 / 4, with no row left at none; each partial
  row names its exact gap (private file formats, a codec the pinned toolchain cannot build, GPL
  plug-ins, stylus tilt, an HDR window surface, embedded scripting languages).
- **Photo.** ~200 more filters including the generated-option families, an animation docker with
  GIF / APNG / MNG / WebP export, a MyPaint-style brush engine, the Krita-family brush tools
  (multibrush, dynamic brush, enclose-and-fill, colourise mask, reference images, drawing
  assistants, pop-up palette), foreground select, an action search, tablet settings, and 30+
  import/export formats (XCF, PSD extras, EPS, SGI, XBM, palette and colour-book files, …).
- **Vector.** Live path effects (clothoid, knot, bend, B-spline / Spiro, measure, linked effects
  such as clone original, attach path and bounding box), 3D box, connectors, tile clones, an XML
  attribute editor, a stylesheet editor, spell checking, SVG fonts, text on path, and ten more
  export formats (FXG, POV, Synfig, XAML, ODG, TeX / PDF+TeX, HPGL, XCF, …).
- **Languages.** Fifteen complete UI languages for both crafts, menus and status messages included.

### Fixed

- The blend-mode list keeps wheel stepping, arrow keys, type-ahead, press-drag-release and hover
  preview with its families and favourites.
- Magic Wand no longer swallows Cmd-drags; Foreground Select's Alt-drag commits the background
  mark; the Paint Bucket's All Layers / Contiguous options reach the fill.
- Exports warn about an image the renderer cannot decode by checking the document itself.
- Formats written without a compiler in the loop (SGI, XBM, XCF, animated PNG/MNG CRC, level-1
  EPS, colour books, SVG paths) read and write correctly.

## 0.10.0 — 2026-10-17

### Added

- **Photo mode: tools and filters from the reference raster editor** (clean room, from its manual
  and black-box use). Desaturate with five modes, anti-erase, diagonal-neighbour and criterion
  options for the wand, bucket and magic eraser, square, conical and spiral gradients, line-art
  bucket fill, transform options (Lanczos, corrective mode, four clipping modes, guides), a filter
  dialog with split view and in-dialog blending, a free-select outline that stays editable, and 74
  new filters (noise, distort, artistic, blur, edge, light-and-shadow, render pattern generators).
- **Photo mode: brush depth from the reference painting app.** Dynamics presets (`.rdyn`) with GBR
  and GIH brush readers, a pop-up palette, drawing assistants, multibrush, wrap-around mode, `.ggr`
  gradients, resource tags, and the HSV/HSL, binary, modulo and quadratic blend families.
- **Vector mode: features from the reference vector editor.** Live stars, spirals and arcs with
  handles, node kinds and node operations, pen Spiro / B-spline / paraxial modes, hairline strokes
  with weight units and mid markers, clones, object properties, find and replace, live path effects
  (bend, B-spline, Spiro, dash, taper), SVG filter graphs with a colour matrix and a filter editor,
  an XML editor, axonometric and modular grids, canvas orientation, multi-scan trace, a palette
  strip and calligraphy.
- `docs/parity/`: feature inventories (`photo.toml`, `paint.toml`, `vector.toml`) that score every
  reference feature as none / partial / full with where ours lives.

### Fixed

- **PSD rendering** against the Photoshop merged image: shape layers follow Photoshop's path fill
  rules, CMYK gradients blend ink by ink, noise gradients render in colour, and scaled patterns
  resample about pixel centres. 78 of the 85 corpus files are now pixel-identical (6 minor, 1 open).
- The modulo blend family composites on the CPU: a sawtooth cannot be held to the same pixels on
  the GPU and the CPU.

## 0.9.0 — 2026-10-17

### Added

- **Control from Redrob Code.** The shell registers with the shared Redrob Code daemon as
  `app:canvas` and serves eight tools — `document_info`, `open_file`, `switch_mode`, `list_layers`,
  `select_layer`, `set_layer_visible`, `export` (PNG/SVG/PDF) and `save` — answered against the
  real editor state with the same operations the menus run. A daemon that cannot be reached is
  logged and the app runs without the bridge.
- **Generative Fill.** Selection + prompt into a diffusion inpaint model, result on a new layer
  clipped to the selection; its own prompt, negative prompt and model choice on the Tools tab and a
  `generative_fill` agent tool.
- **Launcher icon.** Each release ships `redrob-canvas.png` and names it in `redrob-launcher.json`,
  so Redrob Launcher's catalog shows this release's own logo.

### Changed

- **Faster local AI.** IOPaint is started through a shim that exchanges raw pixel files instead of
  base64 PNG; Erase, Inpaint and Generative Fill send only the mask's region of interest; outpaint
  walks a large canvas in feathered 1024 px tiles instead of one request; Remove background defaults
  to `isnet-general-use`.

### Fixed

- Dialogs that grow after placement keep their OK / Cancel row on screen; adjustment dialogs scroll
  inside the same height budget as filter dialogs.
- Type tool options format the type layer shown when the selection holds none, report a refused
  style instead of dropping it, and the orientation toggle reflects the target.
- Export As gains a Canvas Size pair under Image Size; Layer > Export As seeds its size from the
  layer.
- An SVG icon sprite (only `<symbol>`s, nothing used) imports every symbol into the Symbols panel
  with one instance on the artboard.
- Camera Raw white balance / exposure preview decodes with the document's own curve and agrees with
  the Open result; a developed raw carries its EXIF focal length into Lens Correction.
- Linux `.deb` no longer depends on `lib32gcc-s1`; it installs on a clean amd64 host.

## 0.8.1 — 2026-10-16

### Fixed

- **No blank frame when switching modes.** Fonts are built once at start-up with every editor's
  families (one `set_fonts` per run instead of eight), Vector and Layout no longer rescan system
  fonts or reset the global style on activation, and the editor area is painted while a mode warms.
- **One look for all three modes.** The shell owns a single egui style (the Photo look); Vector and
  Layout draw with it instead of their own spacing, rounding and text styles.
- **One language for the whole app.** The shell detects the system language, remembers a choice
  (Help > Language, the languages all three editors share) and pushes it into Photo, Vector and
  Layout. Layout gains a Korean catalog.
- **Quieter update check.** Trusts the system's root certificates and proxy, shows a small muted
  note with the reason on hover instead of a red badge, stays silent on GitHub rate limits, and
  checks automatically at most once a day (Help > Check for updates… always runs).
- **No upstream brand or Discord left in the UI.** Every ArtCraft/Discord string, icon and link is
  gone from menus, dialogs, help and translations; the dead Vector app-page link is removed and
  Help offers Report an Issue (GitHub) instead. `tools/rebrand_crafts.py --check` guards it.

## 0.8.0 — 2026-10-16

### Changed

- **Redrob names everywhere a user meets one.** The vendored editors' crates are `redrob-photo-*`,
  `redrob-vector-*`, `redrob-layout-*` and `redrob-craft-*`; documents save as `.rphoto`, `.rvector`
  and `.rlayout`; the MCP resource URIs are `redrob-photo://`, `redrob-vector://` and
  `redrob-layout://`; MIME types are `application/x-redrob-photo` (vector, layout alike) and
  `application/vnd.redrob-layout+zip`; environment variables are `REDROB_PHOTO_*`, `REDROB_VECTOR_*`,
  `REDROB_LAYOUT_*` and `REDROB_CRAFT_*`. Old documents (`.pcraft`, `.vectorcraft`, `.drawcraft`,
  `.designcraft`) still open, the old URI schemes are still answered and the old environment
  variables are read when the new ones are unset; nothing writes an old value. The file bytes and
  the `"format"` header value are unchanged, so documents stay interchangeable with upstream builds.
- **The upstream repository furniture is gone from `crafts/*`** (docs, book, packaging, CI, READMEs,
  examples, perf, scorecard, scripts, xtask, Dockerfile); licences, NOTICE and ATTRIBUTION files,
  assets, vendored crates and contributor credits stay. `tools/prune_upstream.py`,
  `tools/rename_crates.py` and `tools/rename_identifiers.py` re-apply all of this after a
  `git subtree pull` (run by `tools/merge_workspaces.py`), and `scripts/check_upstream_ids.py` fails
  CI while any upstream identifier is left outside the attribution files and `// legacy:` read paths.
- **CI is one pull-request workflow** (`.github/workflows/ci.yml`: `checks` and `rust`, our crates
  only, `--profile ci`, skipped when no Rust input changed or the PR targets `main`). The gitflow,
  verify-upstream and cargo-audit workflows and every push trigger are gone; the release workflow's
  validate job runs the scripts and `cargo fmt --check` only before the three target builds.

## 0.7.0 — 2026-10-15

The rewrite. Redrob Canvas is no longer a Qt Quick editor that ports Krita and GIMP behaviour into
its own Rust core; it is an egui shell hosting three complete, actively developed editors as modes,
with Redrob's agent and AI layer across them. The product idea (one window, Photoshop and Illustrator
habits, an agent that edits beside you) is the same; almost every line under it is different.

### Changed

- **One window, three modes.** `apps/redrob-canvas` (eframe) hosts the Photo editor as the Photo mode,
  the Vector editor as the Vector mode and the Layout editor as the Layout mode, switched with `Ctrl+1/2/3` or
  `--mode`. Each editor is created on first use and kept alive, so switching preserves open documents.
  All four upstream repositories (the three editors and craft-libs) are vendored as git subtrees under
  `crafts/` at pinned commits; the root `Cargo.toml` is generated from their manifests by
  `tools/merge_workspaces.py`.
- **Licence: GPL-3.0-or-later becomes MIT OR Apache-2.0.** Every input is now permissive, so the
  project is too. `LICENSE` is replaced by `LICENSE-MIT` and `LICENSE-APACHE`; `COPYRIGHT` and `NOTICE`
  name the the upstream authors beside us and reproduce each upstream NOTICE verbatim; `SOURCE_OFFER.md` and
  the corresponding-source bundle are gone because nothing copyleft remains to offer source for.
  `THIRD_PARTY_NOTICES.md` is still generated from `Cargo.lock`, now without the babl/lcms2/Qt notes.
- **Upstream pins** move to the organisation shape: `UPSTREAM.md` plus `upstream-base.json`, with
  `docs/upstream-sources.toml` reduced to the four subtrees (`kind = "code"`) and the Redrob Code
  protocol pin. `scripts/verify-upstream.sh` checks the two pin files against each other, against the
  subtree squash commits and against GitHub, refuses any non-permissive `code` licence, and checks
  that nothing outside `crafts/` touches the upstream brand assets, which are trademarks.
  `scripts/fetch-upstream.sh` is now the `git subtree pull --squash` recipe.
- **Build and CI without Qt.** No CMake, no aqtinstall, no `windeployqt` or `macdeployqt`. CI runs the
  pins, `cargo fmt`, `clippy -D warnings`, `cargo test`, the notice check, the design-system guard,
  the tool tests and `cargo audit`. The release workflow builds the single `redrob-canvas` binary per
  platform, stages it with `tools/release/stage.py` (the macOS `.app` is written there), packages the
  Linux `.tar.gz` and `.deb`, the macOS `.dmg` (signed, notarized, stapled) and the Windows
  `-portable.zip` and `-setup.exe` (signed through Azure Artifact Signing over OIDC), and calls the
  shared `redrob-labs/.github` publish job, which writes `SHA256SUMS.txt` and `redrob-launcher.json`
  and publishes the release as latest. `gitflow.yml` and `pr-policy.yml` remain the org templates.
- **Design-system guard** no longer looks for QML. It checks the vendored token and icon pins, the
  generated mark and lockups, stray files under `resources/icons`, and colour literals in the shell.
- **Installers** install `bin/redrob-canvas` and the licence set; the uninstallers also remove the
  shell's new config directory (`Redrob Canvas` under Application Support / `%APPDATA%`) and still
  clean up what the 0.6 Qt shell left behind.

### Removed

- The Qt Quick shell (`native/`, `qml/`), the C ABI (`crates/redrob-ffi`) and `crates/redrob-core`:
  the GPL-era document model, raster engine, selection, command bus, history, `.rrg` format and the
  bounded PNG/JPEG/WebP/ORA/SVG adapters. Nothing reads `.rrg` any more.
- Every Krita, GIMP, GEGL and Graphite derived file, the babl and lcms2 adapters, the golden-output
  harnesses, the compatibility matrix, the latency harness and the user-path and workflow audits
  (`docs/compatibility.md`, `docs/latency-measurement.md`, `docs/user-paths.md`,
  `docs/workflow-scenarios.md`).
- The Qt-based rehearsal scripts (`tools/release/build-macos.sh`, `build-windows.ps1`, `mac-setup.sh`,
  `bundle_linux_qt.py`); `build-linux.sh` remains, on cargo.
- Dependabot configuration was already gone; the advisory job stays.

### Lost and regained, feature by feature

What 0.6 did that 0.7 does through an upstream editor instead, and what is not back yet:

- Layers, masks, groups, blend modes, adjustment layers, layer styles, brushes with ABR/GBR tips,
  selections, channels and paths, type layers, PSD/PSB round trips, ORA and KRA: **regained** in the
  Photo mode (the Photo editor), in most cases with more than 0.6 had (16/32-bit, CMYK and Lab documents,
  layered TIFF, PDN, Affinity import, camera RAW).
- Vector paths, booleans, text on paths, gradients, SVG/AI/EPS/PDF/DXF: **regained** in the Vector mode
  (the Vector editor), replacing the limited SVG subset and the Graphite port.
- Page layout: **new**, in the Layout mode (the Layout editor).
- Photoshop and Illustrator shortcut sets, the workspace switcher, the Essentials/Photography/Painting
  /Motion layouts and the panel icon strip of 0.5: **not back** in that form; each editor ships its own
  keymap and layout. A shell-level keymap chooser is on the roadmap.
- The timeline and raster animation (frames, FPS, playback) of 0.3 to 0.6: **not back**. None of the
  three editors is an animation tool.
- The agent panel and its reviewable proposals: `crates/redrob-agent` is kept and unchanged in what
  it speaks; the panel in the shell and the tool set over the three modes are **in progress**.
- The AI tools (inpaint, outpaint, erase, upscale, remove background, select subject through a local
  IOPaint engine): **in progress** as `crates/redrob-ai`, ported from the 0.5 implementation.
- The `.rrg` project format and the deterministic `font8x8` text path: **gone**, replaced by each
  editor's native format and real text engines.
- The release pipeline of 0.6.1 (one `release.yml` on the shared redrob-labs shape, Azure Artifact
  Signing over OIDC, the shared publish job, gitflow rules called from the org): **kept**, now without
  the Qt legs.

## 0.6.0 — 2026-10-14

### Changed

- The in-app agent talks to the shared Redrob Code daemon (`<daemon>/v1/chat/completions`, with the
  daemon's authorization, `x-redrob-app: canvas` and the document id as `x-redrob-session`) and the
  daemon holds the Redrob console key. This app no longer carries a key of its own: `REDROB_API_KEY`,
  `REDROB_BASE_URL`, `.env.example`, the stored `console-key` file and the launcher shared-account
  reader are gone (the engine reads the launcher's sign-in itself). The direct path to
  `console.redrob.ai` for chat is removed, so masking applies in one place. "Connect to Redrob
  console" still runs the console's device flow, but the approved key goes to the engine
  (`PUT /auth/redrob`); the panel reads signed-in status from the engine (`GET /provider`) and says
  "Signed in with Redrob Launcher" when that is whose key it uses. A key an earlier version stored
  is handed to the engine once and deleted. **Requires Redrob Code**: without it the agent panel says
  "Redrob Code is not installed. Install it with Redrob Launcher." with the launcher link, and the
  deterministic local proposal mode runs.
- FFI ABI v3: `redrob_agent_propose` takes no key, `redrob_device_flow_wait` returns no key, and
  `redrob_engine_account_status` / `redrob_engine_sign_in` / `redrob_engine_sign_out` are new.
- `tools/generate_distribution_artifacts.py` carries git-pinned dependencies in the notices and the
  corresponding-source bundle (`redrob-engine-client` from redrob-code), verified at the commit
  `Cargo.lock` names, with the repository-root licence texts and a `cargo vendor`-shaped source table.

## 0.5.7 — 2026-10-13

### Added

- Signed in once through Redrob Launcher, this app uses that workspace too. When it has no console
  key of its own (no `REDROB_API_KEY`, no stored sign-in), it reads the launcher's shared account
  file -- `~/.config/redrob-shared/account.json` on Linux, `~/Library/Application Support/ai.redrob.shared/account.json`
  on macOS, `%APPDATA%\Redrob\Shared\account.json` on Windows; `REDROB_SHARED_DIR` replaces the folder.
  The file is read again whenever the key is needed, never written, and the agent panel says
  "Signed in with Redrob Launcher" instead of offering to disconnect. This app's own key still wins.
- Every release carries `redrob-launcher.json` (`{ "schema": 1, "sharedAccount": true }`), listed in
  the Linux `SHA256SUMS` file, so Redrob Launcher learns by itself that the release uses its sign-in.
  `tools/release_manifest.py` writes it (`build-linux.sh`, the `release-ci.yml` Linux leg) and both
  release workflows refuse a draft without it; `tools/tests/test_release_manifest.py` checks the content.

### Changed

- Release asset names are what Redrob Launcher installs: `redrob-canvas-<version>-<os>-<arch>[-portable].<ext>`
  with the launcher's tokens. The Windows archive is now `…-windows-x64-portable.zip` (was
  `…-windows-x86_64.zip`) and the setup `…-windows-x64-setup.exe` (was `…-windows-x86_64-setup.exe`);
  the launcher installs the portable zip on Windows, the `.dmg` on macOS and the `.tar.gz` on Linux.
  Linux and macOS names were already in that form. `tools/tests/test_release_asset_names.py` keeps the
  generator and the hand-written names in the workflows and release scripts in step.

## 0.5.6 — 2026-10-12

### Added

- Installers. Windows: a branded setup and uninstaller (per user, no administrator rights; Start-menu and
  desktop shortcuts; a Settings > Apps entry). macOS: a disk image with an uninstaller. Linux: a `.deb`.
  The setup window and the disk image carry the Redrob Canvas lockup. The zip and tar.gz archives are
  still published.
- The Windows exe and the macOS app now carry the product icon; both showed a blank one before.

### Fixed

- The Linux download started only where Qt 6.11 was already installed system-wide, because Qt was never
  copied into it. Qt is now bundled in the `.tar.gz` and the `.deb`.
- The licence notices said Qt was not shipped. Every download ships Qt 6.11.2, and now says so: the
  LGPL-3.0 text and where to get Qt's source are included as `Qt-LGPL-3.0.txt` and `Qt-README.md`.

## 0.5.5 — 2026-10-10

### Added

- **Mask thumbnails.** A layer with a mask shows the mask beside its picture in the Layers panel:
  white shows, black hides. A disabled mask is crossed out in red; Shift+click toggles it.
- **Channels panel**, a tab beside Layers (also Window > Channels): RGB, Red, Green and Blue
  thumbnails of the picture, then the document's alpha channels with their own thumbnails.
  Save the selection as a channel, make an empty one, rename (double-click), show or hide its
  overlay, delete. Ctrl+click a channel's thumbnail to load it as the selection; Shift adds,
  Alt subtracts, both intersect. A new channel starts hidden, as in Photoshop.
- **Paths panel**, a tab beside Layers (also Window > Paths): stored paths with their filled
  thumbnails. Make a path from the selection, load a path as the selection (Ctrl+click, same
  modifiers), stroke it with the current brush, rename, delete.

### Fixed

- Loading a path as the selection failed with a work-limit error when the path was large and
  curve-fitted (the default for a path made from a selection). It is now filled in bands.

## 0.5.4 — 2026-10-09

### Added

- **Layer thumbnails.** Each row in the Layers panel shows the layer's own picture over a
  checkerboard, as in Photoshop: a hidden or faded layer still shows what it holds, and a group
  shows its layers together. Adjustment layers keep their symbol.
- **Search** in the title bar finds tools, panels and workspaces by name; Enter opens the first.
- **Workspaces** (Essentials, Photography, Painting, Motion) in the title bar and under
  Window > Workspace. Each shows its own set of panels; Motion opens the timeline. Remembered.
- **Panel icon strip** beside the dock: Properties, Agent, AI tools, Layers and History. A click
  opens a panel; clicking an open one folds it away. Window > Layers does the same for Layers.
- **Kind buttons** in the Layers panel (pixel, adjustment, type, shape, smart object): one click
  shows only that kind, a second shows all.

### Changed

- **The menus share one row** with the document name, search and workspace, as in Photoshop.
- **Layers panel buttons are icons**: the footer (link, style, mask, adjustment, group, new,
  delete) and the lock row.

### Removed

- **GEGL and Krita native adapters.** Both were OFF in every release build and never reachable
  from the app. The `REDROB_ENABLE_GEGL` and `REDROB_ENABLE_KRITA` CMake options and the
  generator's `--gegl` / `--krita` flags are gone, and the capability report no longer lists them.
  Nothing from GIMP, GEGL or Krita is linked; their behaviour stays reimplemented in Rust.

## 0.5.3 — 2026-10-08

### Changed

- **The app is named Redrob Canvas** on macOS (`Redrob Canvas.app`) and Windows
  (`Redrob Canvas.exe`, with the name in its file properties), where the Dock, Finder, Explorer
  and Task Manager showed `redrob-canvas`. The Linux command is still `redrob-canvas`.
- **macOS download renamed `macos-arm64`.** The build has only ever run on Apple silicon; the old
  `macos-universal` name was wrong. Intel Macs are not supported.
- **Linux build** now needs a distribution with glibc 2.43 or newer, such as Ubuntu 26.04.

### Fixed

- **macOS and Windows controls** now look the same as on Linux. They used the system's native
  style, which ignored the panel colours and shapes; every platform now uses the same style.

## 0.5.2 — 2026-10-08

### Fixed

- **Tab in a text field** (a number box, a layer name) moves to the next field, as in Photoshop.
  It used to hide the panels. Outside a text field, Tab still hides and shows them.
- **macOS:** the Dock icon is the same size as every other app's. The tile now sits on Apple's
  icon grid (824 of 1024, centred); it used to fill the whole square and stood about a fifth
  larger.

## 0.5.1 — 2026-10-08

A patch on the version number, but it adds two things: an AI tab and a chat-style Agent tab. It
also fixes the Windows build that kept 0.5.0 from shipping there.

### Added

- **AI tab (IOPaint).** IOPaint 1.6.0 (Apache-2.0) runs on this computer. "Install AI engine"
  puts it and PyTorch in the app's own folder (about 1.5 GB; needs uv or Python 3.11). Then:
  - erase or replace what is selected, with IOPaint's erase models (LaMa, MI-GAN, MAT, FcF, ZITS,
    LDM, Manga, OpenCV) or its diffusion models (Stable Diffusion 1.5/2, SDXL, PowerPaint,
    Kandinsky, AnyText, Paint by Example, InstructPix2Pix), with prompt, steps, strength,
    guidance, seed, sampler, ControlNet, BrushNet and LCM LoRA;
  - expand the canvas and fill the new edges;
  - select a subject by clicking it (Segment Anything), or in one click (background removal or
    anime segmentation);
  - remove the background, upscale with RealESRGAN (×2 to ×4), restore faces (GFPGAN,
    RestoreFormer), and erase a whole folder in a batch.
  Every result is a new layer (or the selection), so the original stays and Undo takes it back.
  Models download the first time they are used.
- **Agent tab is a chat.** Type and press Enter. With redrob-code connected, each message
  continues the same conversation, so it remembers what came before; "New chat" starts over.
  Without it, the Redrob agent answers in the same thread. Edits still wait for Apply.

### Fixed

- **Windows:** the token file's owner-only check now reads the Windows access list itself; Qt's
  permission bits always reported "everyone", so the start-up check failed and the Windows build
  of 0.5.0 never shipped.
- **macOS:** the app reported version 0.3 in Finder and crash reports; it now takes the version
  from the build.

### Notes

- The AI engine listens on 127.0.0.1 only and has no password of its own; any program on this
  computer can use it while it runs. It stops with the app.

## 0.5.0 — 2026-10-07

A minor bump: the editor gains the Photoshop-style working surface -- tool groups, menus, keys,
layer operations and dialogs -- over the engine that 0.3.0 and 0.4.0 filled in. 249 commits.

### Added

- **Layers:** duplicate (Ctrl+J), merge down (Ctrl+E), merge visible (Ctrl+Shift+E), flatten,
  select several layers to group, delete and align, clipping masks (Ctrl+Alt+G), locks
  (transparent pixels, image, position, all), linked layers, blend-if ranges, artboards with a
  draggable handle and PNG export per artboard, smart objects that re-render transforms and warps
  from the source, and smart filters stacked on a smart object.
- **Edit:** new document (Ctrl+N), copy, cut and paste through the system clipboard, stroke
  selection, content-aware fill (Shift+F5), puppet warp, crop to selection, clear outside the
  selection, and editing an adjustment layer's filter in place.
- **Image and view:** image size and canvas size dialogs, colour mode and precision menus, navigator,
  rotate view (R), rulers and guides (Ctrl+R, Ctrl+;), snapping to guides and canvas edges.
- **Brushes:** paint shows while the pointer is down, flow with Shift+digit keys, dab angle and
  angle from pen tilt, a mixer brush with Photoshop's drying, tip pickup, Sample All Layers and
  Load/Clean.
- **Text:** installed outline fonts, stored by name as Photoshop does, with kerning, OpenType
  shaping (ligatures, Hangul, Arabic), paragraph width and alignment.
- **Colour:** CMYK soft proof through Little CMS (Ctrl+Y), an Image > Mode > CMYK document mode,
  and export to CMYK TIFF and layered CMYK PSD through the proof profile.
- **Files:** open PSD, KRA, XCF and TIFF; PSD export keeps clipping and blend modes; swatches
  persist and load `.gpl` / `.aco`.
- **Selection:** color range by sampled colour or tone range.
- **Actions:** record edits, save them and play them back as one step.
- **Agent:** a loopback MCP endpoint for `redrob-code`, running a multi-step task against the
  canvas with every edit held for approval, and connecting to the Redrob console with a device code.
- **Keys:** Photoshop's shortcut layout, Alt-click for the clone source, Ctrl held for Move.

### Changed

- The window, menus, layer panel, filter browser and option tabs are split out of `Main.qml`.
- File and colour pickers open inside the window, so keys typed in them no longer reach the canvas,
  and every save picker asks before replacing an existing file.
- Slow frames render on a worker so painting does not stall under heavy adjustment layers.

### Fixed

- 16- and 32-bit documents: resize, crop and the 8-bit tools no longer corrupt deep layers, and
  faint paint keeps the deep detail beneath it.

## 0.4.0 — 2026-10-03

A minor bump rather than a patch: the pen overlay now shows something it never showed, and the
document projection gained fields for it.

### Added

- **A committed pen path keeps its bezier handles on canvas.** Committing a path used to clear the
  pen's tool state, and the overlay had nothing else to read, so the handles that shaped the curve
  disappeared at the moment the curve existed. The overlay now reads the active vector node's own
  geometry back out of the document — which also means an undo removes the handles with the node
  instead of leaving a ghost frame over an empty canvas.
- Each anchor's outgoing control point is drawn as a leash and a round knob, distinct from the
  square anchor handles. A control point sitting on its own anchor is a corner and draws nothing.

### Changed

- The engine provenance pin moves to `redrob-code` **v0.4.1** (`cc0c4bea`), from v0.1.0.

### Fixed

- **The pin guard never checked that a recorded tag points at the recorded commit.** The pins file
  says the tag is the durable half of a pin, and that half was the unchecked one: a tag is a movable
  ref, so it can be repointed and the two halves then disagree while each one, read alone, still
  resolves. `scripts/verify-upstream.sh` now asserts the pairing, dereferencing annotated tags, and
  reports a moved tag and a deleted tag differently.

## 0.3.0 — 2026-10-03

Re-derives the GIMP and Krita feature set this product was missing, as Rust and QML written
from the upstream behaviour rather than copied from it. Groups A through I of the porting plan,
90 commits.

### Added

- **Blend modes: 9 → 41.** The GIMP/Krita union, including the dodge/burn/light family, the
  HSV and LCh component modes, and the arithmetic set.
- **Filters: 12 → 71.** Blurs (motion, lens, box), edge detect, distorts (displace, fractal
  trace, warp map, bump map), noise, tone and colour operations. The map-reading filters take a
  layer to read the map FROM, not just the layer's own pixels.
- **Rail tools: 11 → 24.** Lasso, polygon and colour selection, intelligent scissors,
  foreground select, pen, measure, align, perspective, cage, warp/liquify, n-point deformation,
  enclose-and-fill, lazybrush.
- **File formats: ~6 → 23.** PSD (all colour modes and bit depths, layer masks), XCF (v11, read
  and write, indexed and greyscale, masks), KRA (Krita's native tiled layers, both directions),
  DDS (BC1/BC3 encoder), animated WebP, JPEG-XL, PDF first-page image, camera RAW, ICC profiles,
  AVIF/HEIF container classification.
- **Brush engine.** Dodge/burn, ink, MyPaint-style scatter, GIH image pipes, Krita sensor
  bindings for size AND, new in this release, for opacity and flow as separate channels.
- **Pen handles.** Anchors placed by click stay corners; anchors placed by drag pull a bezier
  handle, whose incoming control is mirrored so the curve passes smoothly through the point.
- **Draggable transform handles** on the canvas for perspective, cage and n-point, drawn as an
  overlay that stays the same size to aim at under any zoom.
- **Fourteen tool glyphs**, drawn here in the design system's geometry, for the tools redrob-ui
  has no icon for. Before this, five different tools drew the same square.
- **Linear-light compositing** in the op graph: a part-strength effect blends in linear light,
  not over display-encoded bytes.

### Fixed

Everything in this section was found by compiling and running the work, which groups A–H
deliberately deferred to the end.

- A densely-sampled stroke painted a single dab. The dab-spacing walker was rebuilt per stroke
  segment, discarding the distance it had walked, so a stroke whose samples sit closer together
  than the dab spacing never reached a second dab — exactly what a graphics tablet produces.
- Every Krita file was rejected before its importer saw it, including files this product had
  just written: KRA borrowed ORA's archive reader, which also enforced ORA's own file list.
- EXR export could not work at all: a float-only encoder was being handed an 8-bit buffer.
- "Press Enter to close the shape" did nothing, in seven tools. A QML array mutated in place
  emits no change signal, so the shortcut's enabled binding was frozen at the empty array.
- The wide-gamut colour swatch was unbound, resolving its conversion through the wrong parent.
- Encoder failures were all reported as `OutputTooLarge`, which hid the EXR bug for a whole
  porting group.

### Changed

- `SizeDynamic` is now `BrushDynamic` and `SizeSensor` is `DynamicSensor`, since the same
  binding drives three channels. The old names remain as aliases.
- `BrushSettings` is no longer `Copy` — it owns binding lists. Everything that reads settings
  takes a reference, so the paint path is unaffected.
- DDS export is reported as lossy, because block compression is.

### Known gaps

Written down rather than left to be discovered:

- HEIF is container-only. There is no pure-Rust HEVC decoder; `rav1d` covers AV1, so AVIF is
  classified separately and HEIF reports its dimensions without decoding.
- A DDS this product writes for a canvas whose width or height is not a multiple of 4 is valid
  and readable by other tools, but not by this product: the `image` crate's DXT decoder refuses
  such dimensions.
- A committed pen handle is not drawn until its path is re-opened; the handle overlay draws one
  outline through its points and does not yet understand anchor/handle pairs.
- ICC table-based profiles are refused, matrix-shaper profiles are applied. A file with a table
  profile still opens, untagged.

## 0.2.0 — 2026-10-01

Packaging and distribution hardening on top of 0.1.0: the corresponding-source bundle and
third-party notices are pinned by hash and gated in the build, text is read and written as UTF-8
regardless of host locale, line endings are not converted, and the system libraries the Rust
staticlib needs on Windows are named explicitly.

## 0.1.0 — 2026-09-17

First tagged build. Draft release.
