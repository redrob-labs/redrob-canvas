# Redrob Canvas

**English** · [한국어](./README.ko.md)

Redrob Canvas is a modern, cross-platform, agentic graphics editor built with Rust and Qt Quick.

redrob-canvas is a new product architecture, not a UI fork. Krita and GIMP remain pinned, executable sources of truth while their proven behavior is moved behind a typed Rust command system and a modern QML experience. Painting behavior follows Krita; image processing behavior follows GIMP/GEGL; product UX and agent workflows belong to redrob-canvas.

## Project structure

```text
crates/
  redrob-core/       Document, layers, raster engine, selection, commands, history
  redrob-agent/      Redrob streaming API, tool loop, approvals
  redrob-ffi/        Stable C ABI consumed by the Qt host
native/qt/            Qt 6 host, QObject models, canvas bridge
qml/                  Modern Qt Quick interface
docs/                 Architecture, compatibility matrix, pinned upstreams
```

## Architecture rules

1. QML and agents submit the same typed commands.
2. Every mutating command is transactional, auditable, and undoable.
3. The render thread consumes immutable snapshots; it never owns document state.
4. Upstream behavior is pinned and tracked feature-by-feature.
5. Krita/GIMP code is integrated only behind explicit adapters; no upstream UI is carried forward.
6. Destructive and agent-generated work requires preview or an explicit commit boundary.

## Build

Core and protocol checks do not require Qt:

```bash
cargo test --workspace
```

The desktop shell requires Qt 6.11+ with Quick, Quick Controls 2, and SVG:

```bash
cmake -S native -B build/qt -DCMAKE_BUILD_TYPE=Release
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
./build/qt/qt/redrob-canvas
```

The app links no GIMP, GEGL or Krita code; their behaviour is reimplemented in Rust. The optional native adapters are babl and Little-CMS 2 (`REDROB_ENABLE_BABL`, `REDROB_ENABLE_LCMS`, both OFF by default); capability reports remain authoritative. A headless capability-only check is:

```bash
cmake -S native -B build/adapters -DREDROB_BUILD_QT=OFF
cmake --build build/adapters
ctest --test-dir build/adapters --output-on-failure
```

## Core file formats

`redrob-core` exposes a content-detected, typed format boundary for RRG, PNG, JPEG, lossless WebP, OpenRaster, and a deliberately limited deterministic SVG subset. The ABI-v2-compatible `redrob_editor_import_file` and `redrob_editor_export_file` symbols route those formats through strict, bounded JSON options and return effective metadata plus machine-readable warnings. Default options reject representational loss. JPEG alpha removal requires an explicit opaque matte or a verified-opaque image, and ORA/SVG adapters reject unsupported semantics rather than guessing. Existing `save_project`/`load_project`, `import_png`/`export_png`, and their C ABI wrappers remain available with their compatibility behavior.

The Qt/QML shell separates editable RRG project identity from interchange. Open, Save and Save As in the header are RRG-only; Import and Export (the current frame) advertise exactly PNG, JPG/JPEG, lossless WebP, ORA, and limited SVG. Import clears the project path, export never changes it or navigates the timeline, output publication uses `QSaveFile`, and explicit allow-loss/JPEG quality/opaque-matte controls prevent silent degradation. See `docs/formats.md` for exact routes and limits.

## Redrob

**Requires Redrob Code.** The in-app agent talks to the one shared Redrob Code daemon on this machine (`~/.redrob/run/server.json`; started with `redrob service start` when none is running) at `<daemon>/v1/chat/completions`, OpenAI-compatible and streaming, default model `auto`, with function tools. Every request carries the daemon's own authorization, `x-redrob-app: canvas` and the document id as `x-redrob-session`. The daemon holds the Redrob console key and calls the console with it; this app never has a key of its own and has no direct path to the console for chat, so there is one place masking applies. Sign in once -- in Redrob Code, in Redrob Launcher, or here: the agent panel's "Connect to Redrob console" runs the console's device flow and hands the approved key to the engine (`PUT /auth/redrob`). The panel reads whether the engine is signed in from `GET /provider` and says "Signed in with Redrob Launcher" when that is whose key the engine uses. Without Redrob Code the panel says "Redrob Code is not installed. Install it with Redrob Launcher." with a link to [the launcher's latest release](https://github.com/redrob-labs/redrob-launcher/releases/latest), and the deterministic local proposal mode runs instead. A key an earlier version of this app stored in its own config folder is handed to the engine once and deleted. The daemon record and binary come from `redrob-engine-client` (redrob-code `docs/LOCAL-ENGINE-API.md`, section 3); `REDROB_RUN_DIR` and `REDROB_CODE_BIN` point tests at a private daemon.

## Distribution artifacts

The complete lockfile notice inventory and source offer are generated only from `Cargo.lock`, locked offline Cargo metadata, and local crate source/license files. The corresponding-source archive includes checksum-verified copies of every locked registry package, the checked-out source of every git-pinned package verified at the commit the lock names (with the repository-root licence texts), plus an offline Cargo source-replacement configuration:

```bash
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF
cmake --build build/qt --target redrob_source_bundle
cmake --install build/qt --prefix /desired/prefix
```

`redrob_distribution_check` is available as a standalone CMake target. Installation reruns the deterministic check and requires the source bundle, `THIRD_PARTY_NOTICES.md`, and `SOURCE_OFFER.md`; see `docs/licensing.md` for the dependency-status policy.

## Roadmap

Where the project is going, milestone by milestone: [ROADMAP.md](ROADMAP.md).

## Releases

Push a `vMAJOR.MINOR.PATCH` tag on `main`, then run `gh workflow run release.yml --ref main -f tag=vMAJOR.MINOR.PATCH`. The workflow (`.github/workflows/release.yml`) checks the tag against `origin/main`, `Cargo.toml` and the changelog, builds and tests every platform on GitHub runners against Qt 6.11.2 from `aqtinstall`, signs and notarizes macOS with the organization's Developer ID, signs Windows through Azure Artifact Signing with the job's OpenID Connect login (no certificate file, no client secret; the app is signed before it is zipped, then the uninstaller, then the setup), and hands the artifacts to the shared redrob-labs publish job (`redrob-labs/.github/.github/workflows/release-publish.yml`), which writes `SHA256SUMS.txt` and `redrob-launcher.json` and **publishes** the release as latest. `gh workflow run release.yml --ref <branch> -f dry_run=true` builds and signs the dispatched branch and publishes nothing. Pull requests are gated by the shared gitflow rules (`redrob-labs/.github/.github/workflows/gitflow.yml`): branch names are `<type>/<slug>` from the org-wide type list, and `main` must be contained in `develop`. The local build scripts in `tools/release/` remain for rehearsing a build on our own machines; they are not the release path.

Which platforms a run builds is decided by `tools/release_matrix.py`, and the same list drives what the verification job demands and what the release notes claim — so the notes cannot promise a binary that was not built. **Windows x64 is only built once the repository variable `WINDOWS_SIGNING_READY` is `"true"`**; until then that leg is skipped with a notice in the run log rather than shipping an unsigned executable. Linux x86_64 and macOS arm64 build unconditionally.

The Qt pin is the newest patch of a settled branch rather than the newest branch: 6.12.0's Windows archives were still missing their checksums on `download.qt.io` days after release, which failed every platform leg. `redrob_qml_restricted_lint` probes `qmllint` for `--only-explicit-categories` and uses it where present, so the stricter lint applies on Qt 6.12+ without making it a build requirement.

| Platform | Build | Signing |
| --- | --- | --- |
| Linux x86_64 | Ninja, Qt bundled (`tools/release/bundle_linux_qt.py`), `.deb` and `.tar.gz` | none — nothing on Linux is code-signed, so verify the published SHA-256 sums |
| macOS arm64 (Apple silicon) | Ninja, `macdeployqt`, `.dmg` (`installer/macos/`) and `.zip` | Developer ID, hardened runtime, notarized with the App Store Connect API key and stapled — the app and the disk image each; `spctl` asserts Gatekeeper's own verdict |
| Windows x64 | Ninja with MSVC, `windeployqt`, `-setup.exe` (`installer/windows/`) and `-portable.zip` | Azure Artifact Signing (OIDC) through `redrob-labs/.github/actions/sign-windows`: Authenticode SHA-256 with an RFC 3161 timestamp on the app, the uninstaller and the setup; `signtool verify /pa` must read Valid and the signer CN `McKinley Rice, Inc.` |

### Installing

| Platform | Download | Install | Remove |
| --- | --- | --- | --- |
| Windows | `…-windows-x64-setup.exe` | Run it. Per user, no administrator rights; Start-menu and desktop shortcuts | Settings > Apps > Redrob Canvas, or `Uninstall Redrob Canvas.exe` in the install folder |
| macOS | `…-macos-arm64.dmg` | Open it and drag Redrob Canvas onto Applications | Open `Uninstall Redrob Canvas` on the disk image |
| Linux | `…-linux-x86_64.deb` | `sudo apt install ./redrob-canvas-…-linux-x86_64.deb` | `sudo apt remove redrob-canvas` |

Both uninstallers ask first and keep settings unless asked to delete them; neither closes a running Canvas. The `-portable.zip` and `.tar.gz` stay for anyone who would rather unpack by hand, and Redrob Launcher installs from them: the `-portable.zip` on Windows, the `.dmg` on macOS, the `.tar.gz` on Linux. Asset names follow `redrob-canvas-<version>-<os>-<arch>[-portable].<ext>` with the launcher's os-arch tokens (`linux-x86_64`, `macos-arm64`, `windows-x64`); `tools/tests/test_release_asset_names.py` keeps them so.

The macOS build is arm64 only; Intel Macs are not supported. Signing material comes from the organization secrets, and a missing credential fails the build rather than publishing an unsigned artifact.

Because this project is GPL-3.0-or-later, shipping a binary without its corresponding source would be a licence violation rather than an omission. Every archive carries `LICENSE`, `COPYRIGHT`, `THIRD_PARTY_NOTICES.md` and `SOURCE_OFFER.md` alongside the binary, the corresponding-source archive is attached to the release, and a separate job fails the release unless an archive is present for every platform the run built, together with every one of those licence artifacts.

## License

GPL-3.0-or-later. `LICENSE` is the verbatim GNU General Public License version 3 as required by section 4; `COPYRIGHT` carries this project's copyright notice and warranty disclaimer. See also `docs/licensing.md`, `THIRD_PARTY_NOTICES.md`, and `SOURCE_OFFER.md`.