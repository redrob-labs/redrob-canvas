# PSD round-trip fidelity — Redrob Canvas 0.8.1

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

## Third-party read-back (GIMP opens the Canvas-written PSD; GIMP flatten of round-trip vs GIMP flatten of original)

| band | files |
|---|---|
| identical (<0.5 %) | 77 |
| minor (0.5–5 %) | 2 |
| broken (≥5 %) | 26 |
| GIMP failed to open the round-tripped PSD | 0 |
| no comparison | 0 |

Direction of the read-back differences (checked on `unicode_pathname`, `4x4_16bit_rgb`, `300dpi`, `mask-density-vectormask`, `pen-text`, `transparentbg`): GIMP's flatten of the Canvas-written PSD moves TOWARDS Canvas's own render and away from GIMP's flatten of the original (e.g. `transparentbg`: GIMP-original fully transparent, GIMP-round-trip pixel-identical to Canvas). The Canvas writer rasterises fill, shape and vector-mask layers into channel pixel data that GIMP can read, where the Photoshop/psd-tools originals carried none; so these rows are a compatibility gain for third-party readers, not a loss. Residual tone differences are GIMP-vs-Canvas renderer differences (alpha/background handling, 16/32-bit → 8-bit).

GIMP layer count original vs round-trip differs in 0 files: none.
Canvas layer count vs GIMP layer count differs on open in 10 files (GIMP materialises clipping groups / multichannel as extra layers): `roundtrip/psdtools--blend-and-clipping.psd` canvas 13 / gimp 17, `roundtrip/psdtools--clip-adjustment.psd` canvas 2 / gimp 3, `roundtrip/psdtools--clipping-mask.psd` canvas 7 / gimp 8, `roundtrip/psdtools--clipping-mask2.psd` canvas 11 / gimp 13, `roundtrip/psdtools--clipping-mask3.psd` canvas 4 / gimp 5, `roundtrip/psdtools--clipping-mask4.psd` canvas 4 / gimp 5, `roundtrip/psdtools--clipping-mask5.psd` canvas 5 / gimp 6, `roundtrip/psdtools--colormodes--4x4_16bit_multichannel.psd` canvas 0 / gimp 1, `roundtrip/psdtools--passthrough_clipping_mask_adjustment.psd` canvas 7 / gimp 8, `roundtrip/psdtools--passthrough_clipping_mask_blendmode.psd` canvas 6 / gimp 7.

## Timing (ms, median over successful files)

- open: 28
- save: 26
- reopen: 5
- flatten: 3

- large file `large-8k.psd` (303 MB, 7680×4320): open 1263 ms, save 2262 ms, reopen 782 ms, flatten 1327 ms

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

- `roundtrip/psdtools--unicode_pathname.psd` — GIMP rt-vs-orig 0.8124, GIMP layers 1→1; features: fill=1; save warnings: 
- `roundtrip/psdtools--colormodes--4x4_16bit_grayscale.psd` — GIMP rt-vs-orig 0.7443, GIMP layers 2→2; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--colormodes--4x4_32bit_grayscale.psd` — GIMP rt-vs-orig 0.7112, GIMP layers 2→2; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--vector-mask2.psd` — GIMP rt-vs-orig 0.6784, GIMP layers 10→10; features: mask=1, vector_mask=3, fill=1, shape=7; save warnings: 
- `roundtrip/psdtools--colormodes--4x4_16bit_lab.psd` — GIMP rt-vs-orig 0.6648, GIMP layers 2→2; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--colormodes--4x4_16bit_rgb.psd` — GIMP rt-vs-orig 0.6469, GIMP layers 2→2; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--colormodes--4x4_16bit_cmyk.psd` — GIMP rt-vs-orig 0.6270, GIMP layers 2→2; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--colormodes--4x4_32bit_rgb.psd` — GIMP rt-vs-orig 0.6258, GIMP layers 2→2; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--mask-density-vectormask.psd` — GIMP rt-vs-orig 0.4994, GIMP layers 5→5; features: vector_mask=4; save warnings: 
- `roundtrip/psdtools--mask-vector-density.psd` — GIMP rt-vs-orig 0.4994, GIMP layers 5→5; features: mask=1, vector_mask=4; save warnings: 
- `roundtrip/psdtools--layer_mask_data.psd` — GIMP rt-vs-orig 0.4986, GIMP layers 5→5; features: mask=3, vector_mask=2, fill=2, shape=2; save warnings: 
- `roundtrip/psdtools--mask_parameters.psd` — GIMP rt-vs-orig 0.4243, GIMP layers 2→2; features: mask=1, vector_mask=1; save warnings: 
- `roundtrip/psdtools--300dpi.psd` — GIMP rt-vs-orig 0.4053, GIMP layers 3→3; features: shape=2; save warnings: 
- `roundtrip/psdtools--cmyk-spot.psd` — GIMP rt-vs-orig 0.4042, GIMP layers 1→1; features: alpha_channel=2; save warnings: 
- `roundtrip/psdtools--mask-density-layervectormask.psd` — GIMP rt-vs-orig 0.3618, GIMP layers 5→5; features: mask=4, vector_mask=4; save warnings: 
- `roundtrip/psdtools--note.psd` — GIMP rt-vs-orig 0.3412, GIMP layers 5→5; features: shape=4; save warnings: 
- `roundtrip/psdtools--pen-text.psd` — GIMP rt-vs-orig 0.3177, GIMP layers 5→5; features: text=2, shape=2; save warnings: 
- `roundtrip/psdtools--mask-parameters-no-real-channel.psd` — GIMP rt-vs-orig 0.3067, GIMP layers 2→2; features: mask=1, vector_mask=1, fill=2; save warnings: 
- `roundtrip/psdtools--32bit.psd` — GIMP rt-vs-orig 0.2649, GIMP layers 2→2; features: shape=1; save warnings: 
- `roundtrip/psdtools--multichannel-pattern-fill.psd` — GIMP rt-vs-orig 0.2568, GIMP layers 1→1; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--adjustment-fillers.psd` — GIMP rt-vs-orig 0.2381, GIMP layers 6→6; features: mask=1, vector_mask=1, text=1, fill=1, shape=3; save warnings: 
- `roundtrip/psdtools--transparentbg.psd` — GIMP rt-vs-orig 0.2056, GIMP layers 1→1; features: shape=1; save warnings: 
- `roundtrip/psdtools--blend-and-clipping.psd` — GIMP rt-vs-orig 0.1836, GIMP layers 17→17; features: text=4, shape=8, layer_style=8, clipping=8; save warnings: 
- `roundtrip/psdtools--gradient-fill.psd` — GIMP rt-vs-orig 0.1167, GIMP layers 3→3; features: mask=1, fill=1; save warnings: 
- `roundtrip/psdtools--clipping-mask2.psd` — GIMP rt-vs-orig 0.0774, GIMP layers 13→13; features: mask=4, vector_mask=1, group=1, adjustment=2, fill=1, shape=5, layer_style=1, clipping=6, hidden=1; save warnings: 
- `roundtrip/psdtools--passthrough_vector_mask.psd` — GIMP rt-vs-orig 0.0742, GIMP layers 5→5; features: mask=2, vector_mask=1, group=1, adjustment=1, fill=1; save warnings: 
- `roundtrip/psdtools--cmyk-alpha-spot.psd` — GIMP rt-vs-orig 0.0467, GIMP layers 1→1; features: alpha_channel=1; save warnings: 
- `roundtrip/psdtools--masks.psd` — GIMP rt-vs-orig 0.0112, GIMP layers 22→22; features: mask=3, group=3, text=6, fill=2, shape=11, layer_style=4, hidden=2; save warnings: 

## Largest Canvas-vs-GIMP differences (top 10, successful files)

- `roundtrip/psdtools--unicode_pathname.psd` — vs GIMP 0.6403 (orig vs GIMP 0.6403, self 0.0000); features: fill=1
- `roundtrip/psdtools--vector-mask2.psd` — vs GIMP 0.6274 (orig vs GIMP 0.6274, self 0.0000); features: mask=1, vector_mask=3, fill=1, shape=7
- `roundtrip/psdtools--adjustment-fillers.psd` — vs GIMP 0.5321 (orig vs GIMP 0.5321, self 0.0000); features: mask=1, vector_mask=1, text=1, fill=1, shape=3
- `roundtrip/psdtools--artboard.psd` — vs GIMP 0.5137 (orig vs GIMP 0.5137, self 0.0000); features: group=1, text=1
- `roundtrip/psdtools--artboard-bgcolor.psd` — vs GIMP 0.4469 (orig vs GIMP 0.4469, self 0.0000); features: group=2
- `roundtrip/psdtools--colormodes--4x4_16bit_lab.psd` — vs GIMP 0.4391 (orig vs GIMP 0.4391, self 0.0000); features: mask=1, fill=1
- `roundtrip/psdtools--colormodes--4x4_16bit_rgb.psd` — vs GIMP 0.4105 (orig vs GIMP 0.4105, self 0.0000); features: mask=1, fill=1
- `roundtrip/psdtools--300dpi.psd` — vs GIMP 0.4053 (orig vs GIMP 0.4053, self 0.0000); features: shape=2
- `roundtrip/psdtools--cmyk-spot.psd` — vs GIMP 0.4042 (orig vs GIMP 0.4042, self 0.0000); features: alpha_channel=2
- `adjustment-layers.psd` — vs GIMP 0.3794 (orig vs GIMP 0.3794, self 0.0000); features: mask=16, adjustment=16, shape=3, layer_style=3, hidden=5

Full per-file rows: `psd-roundtrip.toml` (this directory); raw work files under `canvas-gap/results/psd/_work` (probe JSON, GIMP references, round-tripped PSDs).
