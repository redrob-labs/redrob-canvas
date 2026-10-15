<!-- SPDX-License-Identifier: MIT OR Apache-2.0 -->
# Roadmap

Redrob Canvas aims to be the graphics editor a Photoshop, Illustrator or InDesign user can open and
start using on day one: their own files, the habits they already have, and an agent that does the
tedious parts beside them.

This roadmap is ordered by that goal. Each milestone is a release. Detailed work lives in
[GitHub Issues](https://github.com/redrob-labs/redrob-canvas/issues), grouped by milestone. Dates
are not promised; a milestone ships when its items are done.

## Where we are (0.7, in progress)

0.7 is the rewrite. The Qt shell and the GPL-era core that ported Krita and GIMP behaviour are gone;
in their place one egui window hosts three complete editors from the upstream family as modes, taken
as git subtrees and updated by merge rather than reimplemented:

- **Photo** mode (the Photo editor): layers, masks, adjustment layers, layer styles, brushes, type,
  selections, channels, paths, colour management; PSD/PSB, ORA, KRA, layered TIFF, RAW and the rest.
- **Vector** mode (the Vector editor): paths, booleans, text on paths, gradients, artboards; SVG, AI, EPS,
  PDF, DXF.
- **Layout** mode (the Layout editor): master pages, threaded text, hyphenation; IDML, PDF, PPTX, EPUB.
- The shell: one window, `Ctrl+1/2/3` mode switch, one config directory, documents preserved across
  switches.
- The project is MIT OR Apache-2.0, the upstream pins follow the organisation shape, and CI and the
  release pipeline build without Qt.

Still to land inside 0.7:

- `crates/redrob-ai`: the AI tools layer (inpaint, outpaint, erase, upscale, remove background, select
  subject through a local IOPaint engine; vectorize), ported from the 0.5 implementation, with its
  panel in the shell's right slot reaching whichever mode is active.
- The agent panel in the shell, with a tool set that spans the three modes.
- Open-by-extension routing of a file to its mode (`docs/formats.md`).

## 0.8: habits

- Shell-level keymap chooser (Photoshop, Illustrator, InDesign sets) applied over each mode's own.
- Send-to: a selection or an artboard from one mode opened or placed in another without an export
  dialog in between.
- The editors' MCP/control servers reachable through the shell, so the agent and outside tools drive
  all three the way each upstream binary allows.

## 0.9: trust and reach

- AI engine hardening: per-session token, no orphaned engine after a crash, clear install and model
  download progress.
- Linux builds that run on older distributions; a Linux aarch64 leg.
- macOS build for Intel Macs, or a clear statement that there will not be one.

## 1.0: stable

- Each mode's native format is its upstream's, versioned there; the shell promises to open anything
  a 0.x shell wrote.
- Every upstream pin is at a tagged upstream release, and `scripts/fetch-upstream.sh` has moved each
  subtree at least once without a hand-resolved conflict.
- A GUI check per mode on all three platforms in the release pipeline.

## Not planned

- Forking an editor's internals. A change an editor needs goes upstream and comes back by merge.
- Copying Adobe code or assets. Formats are implemented from public specifications and permissively
  licensed open-source code, with attribution.
- Requiring an account or the cloud. Everything works offline; AI runs locally.
- Timeline and raster animation. 0.3 to 0.6 had one; none of the three editors is an animation tool,
  and the Redrob product for that is a different app.

## Proposing changes

Open an issue to propose a change to this plan. See [CONTRIBUTING.md](CONTRIBUTING.md).
