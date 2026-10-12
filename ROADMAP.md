<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Roadmap

Redrob Canvas aims to be the image editor a Photoshop or Illustrator user can open and start using
on day one: the same layout, the same shortcuts, and their own files.

This roadmap is ordered by that goal. Each milestone is a release. Detailed work lives in
[GitHub Issues](https://github.com/redrob-labs/redrob-canvas/issues), grouped by milestone. Dates
are not promised; a milestone ships when its items are done.

## Where we are (0.5.x)

- Photoshop-style layout, plus Photoshop and Illustrator shortcut sets chosen on first launch.
- Layers with masks, groups, blend modes, locks, thumbnails, adjustments; Channels and Paths panels.
- Opens and saves PSD, KRA, XCF and ORA with their layer stacks.
- Brushes with ABR/GBR tips, perspective, puppet warp and liquify, outline fonts.
- AI tools (inpaint, outpaint, select subject, remove background) through a local IOPaint engine.
- A chat agent that edits the document with reviewable steps.
- Installers for Windows, macOS (Apple silicon) and Linux.

## 0.6 — Your PSD comes back intact

Opening a file is not enough; a round trip must not lose work.

- PSD export writes layer masks.
- PSD adjustment layers are applied on import, not only kept.
- Layer styles stay live and editable (today they are baked into pixels), and round-trip via PSD.
- View a single colour channel on the canvas (#142).
- Pen tool draws work paths into the Paths panel (#143).

## 0.7 — Photographers

- 16-bit and 32-bit float documents (today everything is 8-bit, and deeper files are narrowed).
- Missing Photoshop keys: Quick Mask (`Q`), screen modes (`F`), Path Selection (`A`) (#144).

## 0.8 — Illustrator basics

- Direct Selection and anchor-point editing (`A`, `Shift+C`).
- Path booleans and compound paths (`Ctrl+8`), outline stroke, join (`Ctrl+J`).
- Remaining Illustrator keys from #144.

## 0.9 — Trust and reach

- AI engine: per-session token (#146) and no orphaned engine after a crash (#145).
- Linux builds that run on older distributions (#147).
- macOS build for Intel Macs (#148).

## 1.0 — Stable

- The `.rrg` project format is frozen, with a migration promise for older files.
- [docs/compatibility.md](docs/compatibility.md) and [docs/formats.md](docs/formats.md) match
  what the app actually does, and stay checked in CI.
- Every scenario in [docs/workflow-scenarios.md](docs/workflow-scenarios.md) passes on all three
  platforms.

## Not planned

- Copying Adobe code or assets. Formats are implemented from public specifications and permissively
  licensed open-source code, with attribution.
- Requiring an account or the cloud. Everything works offline; AI runs locally.

## Proposing changes

Open an issue to propose a change to this plan. See [CONTRIBUTING.md](CONTRIBUTING.md).
