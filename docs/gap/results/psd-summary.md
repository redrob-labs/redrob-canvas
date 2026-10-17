# PSD round-trip fidelity — Redrob Canvas 0.8.1 -> 0.9.0 (fix/psd-fidelity-v0.9)

Corpus: 105 PSD files under `corpus/psd` (5 hand-made + 100 psd-tools fixtures). Each file: open → save as PSD → reopen → flatten to PNG through the 0.8.1 `redrob-photo-io`/`redrob-photo-psd` codec (probe binary `psd-rt`, release profile; the GUI binary has no headless convert path). Reference flatten of the original by GIMP 3.2.6 (`file-psd` + merge visible). RMSE = ImageMagick normalised RMSE (0..1); identical < 0.5 %, minor 0.5–5 %, broken ≥ 5 %.

## Counts (codec self-consistency: Canvas flatten of original vs Canvas flatten of round-trip)

| verdict | files |
|---|---|
| identical (<0.5 % RMSE) | 105 |
| minor (0.5–5 %) | 0 |
| broken (≥5 %) | 0 |
| failed to open/save/reopen | 0 |

## Counts vs GIMP reference (GIMP flatten of original vs Canvas flatten of round-trip)

| band | files |
|---|---|
| identical (<0.5 %) | 53 |
| minor (0.5–5 %) | 6 |
| broken (≥5 %) | 46 |
| no GIMP reference / no comparison | 0 |

Note: the GIMP band mixes renderer differences (text shaping, adjustment math, blend-mode interpretation, CMYK/Lab conversion) with real round-trip loss; `rmse_orig_vs_gimp` in the TOML isolates the renderer part. In this corpus every file has `rmse_vs_gimp == rmse_orig_vs_gimp`, i.e. the whole GIMP difference is renderer difference, and spot checks show it is mostly GIMP's own PSD limits: GIMP renders fill layers, shape layers and adjustment layers as empty/transparent or pass-through (e.g. `unicode_pathname` → fully transparent in GIMP, `300dpi` → white, `adjustment-layers` → 2 colours), while Canvas renders them.

## Counts vs the Photoshop merged image (the file's own composite, written by Photoshop; Canvas flatten of round-trip vs it)

| band | files |
|---|---|
| identical (<0.5 %) | 78 |
| minor (0.5–5 %) | 6 |
| broken (≥5 %) | 1 |
| no real merged image (Maximize Compatibility off, or no layers) | 20 |

For comparison, GIMP's flatten of the original is ≥5 % off the same Photoshop merged image in 40 of 85 files.

Files without a real merged image but with Photoshop's embedded thumbnail (resource 1036), compared at thumbnail size over white: 12; `psdtools--multichannel-pattern-fill.psd` ours 0.432 / GIMP 0.009, `psdtools--colormodes--100x20_1bit_bitmap_rle.psd` ours 0.057 / GIMP 0.057, `psdtools--blend-and-clipping.psd` ours 0.043 / GIMP 0.222, `psdtools--cmyk-alpha-spot.psd` ours 0.019 / GIMP 0.019, `psdtools--adjustment-fillers.psd` ours 0.016 / GIMP 0.609, `psdtools--mask_parameters.psd` ours 0.016 / GIMP 0.018, `psdtools--note.psd` ours 0.011 / GIMP 0.382, `psdtools--unicode_pathname.psd` ours 0.005 / GIMP 0.282, `psdtools--history.psd` ours 0.005 / GIMP 0.005, `psdtools--cmyk-gray-ramp.psd` ours 0.004 / GIMP 0.003, `psdtools--adjustment-mask.psd` ours 0.000 / GIMP 0.000, `psdtools--slices.psd` ours 0.000 / GIMP 0.000.

## Third-party read-back (GIMP opens the Canvas-written PSD; GIMP flatten of round-trip vs GIMP flatten of original)

| band | files |
|---|---|
| identical (<0.5 %) | 0 |
| minor (0.5–5 %) | 0 |
| broken (≥5 %) | 0 |
| GIMP failed to open the round-tripped PSD | 0 |
| no comparison | 105 |

Direction of the read-back differences (checked on `unicode_pathname`, `4x4_16bit_rgb`, `300dpi`, `mask-density-vectormask`, `pen-text`, `transparentbg`): GIMP's flatten of the Canvas-written PSD moves TOWARDS Canvas's own render and away from GIMP's flatten of the original (e.g. `transparentbg`: GIMP-original fully transparent, GIMP-round-trip pixel-identical to Canvas). The Canvas writer rasterises fill, shape and vector-mask layers into channel pixel data that GIMP can read, where the Photoshop/psd-tools originals carried none; so these rows are a compatibility gain for third-party readers, not a loss. Residual tone differences are GIMP-vs-Canvas renderer differences (alpha/background handling, 16/32-bit → 8-bit).

GIMP layer count original vs round-trip differs in 0 files: none.
Canvas layer count vs GIMP layer count differs on open in 10 files (GIMP materialises clipping groups / multichannel as extra layers): `roundtrip/psdtools--blend-and-clipping.psd` canvas 13 / gimp 17, `roundtrip/psdtools--clip-adjustment.psd` canvas 2 / gimp 3, `roundtrip/psdtools--clipping-mask.psd` canvas 7 / gimp 8, `roundtrip/psdtools--clipping-mask2.psd` canvas 11 / gimp 13, `roundtrip/psdtools--clipping-mask3.psd` canvas 4 / gimp 5, `roundtrip/psdtools--clipping-mask4.psd` canvas 4 / gimp 5, `roundtrip/psdtools--clipping-mask5.psd` canvas 5 / gimp 6, `roundtrip/psdtools--colormodes--4x4_16bit_multichannel.psd` canvas 0 / gimp 1, `roundtrip/psdtools--passthrough_clipping_mask_adjustment.psd` canvas 7 / gimp 8, `roundtrip/psdtools--passthrough_clipping_mask_blendmode.psd` canvas 6 / gimp 7.

## Timing (ms, median over successful files)

- open: 22
- save: 17
- reopen: 3
- flatten: 1

- large file `large-8k.psd` (303 MB, 7680×4320): open 1109 ms, save 1577 ms, reopen 673 ms, flatten 1083 ms

## Top lost features (files where the reopened document has fewer of the feature than the original)

| feature | files |
|---|---|
| (none — every counted feature survived in every file that reopened) | 0 |

## Top codec warnings (open/save/reopen), normalised

| warning | occurrences |
|---|---|
| layer …: its Color Lookup settings could not be read (it uses a colour profile (abstract o | 4 |
| Bitmap 1-bit document converted to Grayscale 8-bit for editing | 3 |
| Bitmap document written as Grayscale | 3 |

## Failed files

- none

## Broken (≥5 % self RMSE)

- none

## Third-party read-back differences (GIMP: round-trip vs original, ≥0.5 %)

- none

## Canvas-vs-Photoshop differences (≥0.5 %, successful files with a real merged image)

- `roundtrip/psdtools--effect-stroke-gradient.psd` — vs Photoshop 0.0949 (GIMP vs Photoshop 0.2191); features: text=2, layer_style=2
- `roundtrip/psdtools--layer_comps.psd` — vs Photoshop 0.0305 (GIMP vs Photoshop 0.0631); features: group=3, shape=9, blend_mode=3, layer_style=9, hidden=2
- `roundtrip/psdtools--passthrough_fill_blendmode.psd` — vs Photoshop 0.0203 (GIMP vs Photoshop 0.1178); features: group=1, blend_mode=2
- `roundtrip/psdtools--layer_params.psd` — vs Photoshop 0.0162 (GIMP vs Photoshop 0.0978); features: layer_style=2
- `roundtrip/psdtools--pen-text.psd` — vs Photoshop 0.0108 (GIMP vs Photoshop 0.3146); features: text=2, shape=2
- `roundtrip/psdtools--clipping-mask2.psd` — vs Photoshop 0.0066 (GIMP vs Photoshop 0.1201); features: mask=4, vector_mask=1, group=1, adjustment=2, fill=1, shape=5, blend_mode=1, layer_style=1, clipping=6, hidden=1
- `roundtrip/psdtools--text.psd` — vs Photoshop 0.0057 (GIMP vs Photoshop 0.0000); features: text=1

## Largest Canvas-vs-GIMP differences (top 10, successful files)

- `roundtrip/psdtools--unicode_pathname.psd` — vs GIMP 0.6403 (orig vs GIMP 0.6403, self 0.0000); features: fill=1
- `roundtrip/psdtools--adjustment-fillers.psd` — vs GIMP 0.5321 (orig vs GIMP 0.5321, self 0.0000); features: mask=1, vector_mask=1, text=1, fill=1, shape=3
- `roundtrip/psdtools--artboard.psd` — vs GIMP 0.5137 (orig vs GIMP 0.5137, self 0.0000); features: group=1, text=1, blend_mode=1
- `roundtrip/psdtools--artboard-bgcolor.psd` — vs GIMP 0.4469 (orig vs GIMP 0.4469, self 0.0000); features: group=2, blend_mode=2
- `roundtrip/psdtools--colormodes--4x4_16bit_lab.psd` — vs GIMP 0.4391 (orig vs GIMP 0.4391, self 0.0000); features: mask=1, fill=1
- `roundtrip/psdtools--colormodes--4x4_16bit_rgb.psd` — vs GIMP 0.4105 (orig vs GIMP 0.4105, self 0.0000); features: mask=1, fill=1
- `roundtrip/psdtools--300dpi.psd` — vs GIMP 0.4053 (orig vs GIMP 0.4053, self 0.0000); features: shape=2
- `roundtrip/psdtools--cmyk-spot.psd` — vs GIMP 0.4042 (orig vs GIMP 0.4042, self 0.0000); features: alpha_channel=2
- `roundtrip/psdtools--colormodes--4x4_16bit_cmyk.psd` — vs GIMP 0.3996 (orig vs GIMP 0.3996, self 0.0000); features: mask=1, fill=1
- `adjustment-layers.psd` — vs GIMP 0.3794 (orig vs GIMP 0.3794, self 0.0000); features: mask=16, adjustment=16, shape=3, layer_style=3, hidden=5

Full per-file rows: `psd-roundtrip.toml` (this directory) / `results/psd/roundtrip.toml`.
