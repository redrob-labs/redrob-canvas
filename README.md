<picture>
  <source media="(prefers-color-scheme: dark)" srcset="resources/brand/redrob-canvas-lockup-white.png">
  <img src="resources/brand/redrob-canvas-lockup.png" alt="Redrob Canvas" height="48">
</picture>

# Redrob Canvas

**English** · [한국어](./README.ko.md)

Redrob Canvas is an agentic graphics editor: photo, vector and page layout in one window, with an AI
tools layer across all three and an agent that edits the document beside you.

It is one desktop application (`apps/redrob-canvas`, Rust and egui) with three modes, each a complete
editor from the ArtCraft family by [storytold](https://github.com/storytold), vendored under `crafts/`
and built from source:

| Mode | Editor | What it is for |
| --- | --- | --- |
| **Photo** | [PhotoCraft](https://github.com/storytold/photocraft) | Layers, masks, adjustment layers, layer styles, brushes, type, colour management; real PSD files |
| **Vector** | [VectorCraft](https://github.com/storytold/vectorcraft) | Paths, booleans, text on paths, gradients, artboards; SVG, AI, EPS, PDF, DXF |
| **Layout** | [DesignCraft](https://github.com/storytold/designcraft) | Master pages, threaded text frames, hyphenation; IDML, PDF, PPTX, EPUB |

The shell owns the window, the mode switch (`Ctrl+1` / `Ctrl+2` / `Ctrl+3`, or `--mode photo|vector|layout`),
one config directory (`ai.redrob.canvas`) and the agent. Switching modes keeps every editor's open
documents. The shared docking, colour-management and printing code the three editors have in common
is [craft-libs](https://github.com/storytold/craft-libs), the fourth subtree.

## AI tools and the agent

Two layers sit across the modes rather than inside any one of them:

- **Agent** (`crates/redrob-agent`): a chat panel whose model edits the document through typed tool
  calls, each reviewable before it lands. **Requires Redrob Code.** The agent talks to the one shared
  Redrob Code daemon on this machine (`~/.redrob/run/server.json`, started with `redrob service start`)
  at `<daemon>/v1/chat/completions`, OpenAI-compatible and streaming, with `x-redrob-app: canvas`. The
  daemon holds the Redrob console key; this app never has a key of its own and has no direct path to
  the console. Sign in once, in Redrob Code, in Redrob Launcher, or here. Without Redrob Code the panel
  says so and links to [the launcher's latest release](https://github.com/redrob-labs/redrob-launcher/releases/latest).
- **AI tools** (`crates/redrob-ai`, in progress for 0.7): inpaint, outpaint, erase, upscale, remove
  background and select subject through a local IOPaint engine the app installs and runs on
  `127.0.0.1`; vectorize for the Vector mode. Everything runs on the machine; nothing is uploaded.

## Formats

What each mode opens and saves comes from its editor. The short list:

| Mode | Open | Save / export |
| --- | --- | --- |
| Photo | PSD, PSB, `.pcraft`, ORA, KRA, layered TIFF, PDN, SVG, PNG, JPEG, TIFF, WebP, GIF, BMP, OpenEXR, HDR, HEIF/HEIC (read), camera RAW, Affinity (read) | `.pcraft`, PSD, PSB, ORA, TIFF, PNG, JPEG, WebP, GIF, BMP, OpenEXR, AVIF (optional feature) |
| Vector | `.vectorcraft`, SVG, SVGZ, AI (PDF-compatible), EPS, PDF, DXF, EMF/WMF, raster images, PSD/PSB as merged image | `.vectorcraft`, SVG, PDF (with PDF/X), AI, EPS, DXF, PNG, JPEG, WebP, PSD, Print, Package |
| Layout | `.designcraft`, IDML, images, PDFs and vectors as placed assets | `.designcraft`, PDF (PDF/X-4, PDF/A-2b), IDML, PPTX, EPUB, PNG |

The authoritative tables are the editors' own: `crafts/photocraft/crates/codecs/README.md`,
`crafts/vectorcraft/README.md` and `crafts/designcraft/README.md`. `docs/formats.md` says how the
shell routes a file to a mode.

## Build

Rust 1.95 (`rust-toolchain.toml` pins it; `rustup` picks it up). On Linux the egui, wgpu and rfd
stack needs the X11, Wayland, GL and GTK development headers:

```bash
sudo apt-get install -y libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev
cargo build --release -p redrob-canvas
target/release/redrob-canvas
```

The gates every pull request must pass, in the order CI runs them (`.github/workflows/verify-upstream.yml`):

```bash
scripts/verify-upstream.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo fetch --locked && python3 tools/generate_distribution_artifacts.py --check
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"
```

There is no Qt, CMake or Node anywhere in the build. The root `Cargo.toml` is **generated** by
`tools/merge_workspaces.py` from the four subtrees' parked manifests (`crafts/*/Cargo.upstream.toml`);
edit the script, not the file.

## Project structure

```text
apps/redrob-canvas/     The shell: window, mode switch, services, agent panel (egui / eframe)
crates/redrob-agent/    Redrob Code client: streaming chat completions, tool loop, approvals
crates/redrob-ai/       AI tools layer (IOPaint engine, vectorize); in progress
crafts/photocraft/      storytold/photocraft  (subtree)  Photo mode
crafts/vectorcraft/     storytold/vectorcraft (subtree)  Vector mode
crafts/designcraft/     storytold/designcraft (subtree)  Layout mode
crafts/craft-libs/      storytold/craft-libs  (subtree)  shared UI, CMS, print, PDF crates
installer/              macOS disk image and Windows setup
tools/                  Workspace generator, notice generator, design-system guard, release packaging
docs/                   Architecture, formats, licensing, pinned upstreams
```

## Upstream

The four editors are taken as git subtrees and updated with `git subtree pull --squash`; the pins live
in `upstream-base.json` and `docs/upstream-sources.toml`, the process in [UPSTREAM.md](UPSTREAM.md),
and `scripts/verify-upstream.sh` checks the two pin files against each other, against the subtree
squash commits and against GitHub. Redrob Canvas adds the shell, the agent and the AI layer; it does
not fork any editor's internals, so upstream improvements arrive by merge.

Copyright (c) 2026 ArtCraft Team and the PhotoCraft, VectorCraft, DesignCraft and craft-libs
contributors, MIT OR Apache-2.0. Their licence and NOTICE files stay intact under `crafts/<name>/`
and are reproduced in [NOTICE](NOTICE). The ArtCraft name and logos under `crafts/*/docs/brand/` are
trademarks of the ArtCraft Team, not open source, and are not used here.

## Roadmap

Where the project is going, milestone by milestone: [ROADMAP.md](ROADMAP.md). The change history
is [CHANGELOG.md](CHANGELOG.md).

## Releases

Push a `vMAJOR.MINOR.PATCH` tag on `main`, then run
`gh workflow run release.yml --ref main -f tag=vMAJOR.MINOR.PATCH`. The workflow
(`.github/workflows/release.yml`) checks the tag against `origin/main`, `Cargo.toml` and the
changelog, runs every gate, builds `redrob-canvas` for each platform with `cargo build --release`,
signs and notarizes macOS with the organization's Developer ID, signs Windows through Azure Artifact
Signing with the job's OpenID Connect login (the executable before the zip, then the uninstaller,
then the setup), and hands the artifacts to the shared redrob-labs publish job
(`redrob-labs/.github/.github/workflows/release-publish.yml`), which writes `SHA256SUMS.txt` and
`redrob-launcher.json` and publishes the release as latest. `-f dry_run=true` builds and signs the
dispatched branch and publishes nothing. Which platforms a run builds is decided by
`tools/release_matrix.py`; Windows x64 is built only once the repository variable
`WINDOWS_SIGNING_READY` is `"true"`, never unsigned.

| Platform | Assets | Signing |
| --- | --- | --- |
| Linux x86_64 | `.tar.gz`, `.deb` (`tools/release/make_deb.py`) | none; verify the published SHA-256 sums |
| macOS arm64 (Apple silicon) | `.dmg` (`installer/macos/`), `.zip` | Developer ID, hardened runtime, notarized and stapled, app and disk image each |
| Windows x64 | `-setup.exe` (`installer/windows/`), `-portable.zip` | Azure Artifact Signing (OIDC) on the app, the uninstaller and the setup |

### Installing

| Platform | Download | Install | Remove |
| --- | --- | --- | --- |
| Windows | `…-windows-x64-setup.exe` | Run it. Per user, no administrator rights | Settings > Apps > Redrob Canvas |
| macOS | `…-macos-arm64.dmg` | Open it and drag Redrob Canvas onto Applications | Open `Uninstall Redrob Canvas` on the disk image |
| Linux | `…-linux-x86_64.deb` | `sudo apt install ./redrob-canvas-…-linux-x86_64.deb` | `sudo apt remove redrob-canvas` |

Redrob Launcher installs from the `-portable.zip` on Windows, the `.dmg` on macOS and the `.tar.gz`
on Linux. Asset names follow `redrob-canvas-<version>-<os>-<arch>[-portable].<ext>`. Every archive
carries `LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`, `NOTICE` and `THIRD_PARTY_NOTICES.md` beside
the binary.

## License

MIT OR Apache-2.0, at your option: [LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE),
[COPYRIGHT](COPYRIGHT). Third-party attribution is in [NOTICE](NOTICE),
[UPSTREAM_NOTICES.md](UPSTREAM_NOTICES.md) and the generated
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md); the policy is in [docs/licensing.md](docs/licensing.md).
