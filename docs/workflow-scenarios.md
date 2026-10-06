# Workflow test scenarios

End-to-end GUI scenarios that follow how people actually work in a professional raster editor:
retouching, compositing, painting, web graphics, and print hand-off. Each step names the action
(menu path or key), what must happen, and how to check it. Run them on the real app on `:0`, one
scenario at a time, with a fresh document unless the scenario says otherwise.

Pass rule: a step passes only when the check is observed (status bar text, layer list, pixel
measurement). A step that "looks fine" without its check is not a pass.

Pixel checks: `magick shot.png -format "%[pixel:p{X,Y}]" info:` at the named point, or a crop diff
of before/after (`magick compare -metric AE`). Do not judge small colour changes by eye.

## S1. Photo retouch, non-destructive

Pattern: keep the original untouched, put fixes on their own layer, tone with adjustment layers.

1. File > Open (`Ctrl+O`) `build/samples/masks.psd`. Check: layers listed, status shows size.
2. `Ctrl+Shift+N` new layer, name it "Retouch". Check: new layer above, active.
3. `J` healing. `Alt+click` a clean source, paint over a mark. Check: changed pixels only on
   "Retouch" (hide it with its eye: canvas returns to the original).
4. `S` clone on the same layer, `Alt+click` source, two short strokes. Check: pixels copied.
   Hold `Ctrl`: the tool switches to Move; release: back to clone.
5. Filters > browser, pick Levels (or `Ctrl+L`) > "Add as adjustment layer". Check: adjustment
   layer appears; underlying layer pixels unchanged (hide adjustment = original).
6. Right-click the adjustment layer > Edit adjustment…, change a value, Update. Check: canvas
   changes; one `Ctrl+Z` restores the previous value.
7. Ctrl+Z through the history panel, then click the last row. Check: every step returns.
8. File > Save As project, reopen. Check: retouch layer and adjustment layer survive.

## S2. Composite: subject onto a new background

Pattern: select the subject, turn the selection into a mask, refine edges, match tone, clip
adjustments to the subject only.

1. `Ctrl+N`, 1280x800, white. Fill a background (`G` gradient, drag across).
2. `Ctrl+Shift+N` "Subject" layer; paint a shape with `B` (stands in for a photo subject).
3. `W` wand on the shape (or Select > Color range… on its colour). Check: "Selection active".
4. Select > Grow by 1 px, then Layers > add mask from selection. Check: outside hidden, mask row.
5. `Ctrl+J` duplicate the subject, `Ctrl+T` free transform: scale and move it. Check: copy moves;
   original stays.
6. New adjustment layer above the subject (Hue/Saturation, `Ctrl+U` > Add as adjustment layer),
   then `Ctrl+Alt+G` clip it. Check: only subject pixels change; background pixel identical.
7. Layer > Blending options…, set Blend If on the underlying layer. Check: subject fades over
   the bright part of the gradient.
8. `Ctrl+Shift+E` merge visible. Check: one merged layer added, sources kept.

## S3. Digital painting: sketch, line art, flats, shading

Pattern: separate layers per stage; shading clipped to flats; adjust brush with keys mid-stroke.

1. `Ctrl+N` 1920x1080. New layer "Sketch", `B`, draw loosely. Check: the stroke draws WHILE
   dragging (frame grabbed mid-drag shows it), not only on release.
2. `[` / `]` change size, `Shift+[` / `Shift+]` hardness, `Alt`+right-drag resize on canvas.
   Check: top bar size/hardness values move.
3. Lower Sketch opacity with `3` (30%). New layer "Lines", draw over it.
4. New layer "Flats" below Lines; `G` fill inside a closed shape (or Enclose and fill).
5. New layer "Shade", `Ctrl+Alt+G` clip to Flats, set blend Multiply, `Shift+5` flow 50%, paint.
   Check: shading never appears outside Flats.
6. `O` dodge on Flats a few strokes; `E` erase a mistake on Lines. Check: pixel changes on the
   right layer only.
7. Mixer brush (`Shift+B` until mixer) across two colours. Check: colours blend.
8. View: `R` rotate the canvas, draw a horizontal line, double-click R to reset. Check: line lands
   where drawn. `H` / Space-drag to pan, `Z` click to zoom; navigator box follows.

## S4. Web graphics: banner on artboards, exported

Pattern: artboard per size, text and shapes, export each artboard as PNG.

1. `Ctrl+N` 1600x900 transparent. Layer > New artboard (whole canvas).
2. Select a 1200x628 rectangle (`M`), Layer > New artboard. Check: two artboards in the list.
3. `T` text: click, type a title, pick a real installed font in the dialog, set box width and
   centre alignment. Check: wraps inside the box; status warns if the font is missing.
4. `U` shape: a button rectangle. `Ctrl+R` rulers, drag a guide to centre. Check: guide drawn.
5. `Ctrl+A`, `Ctrl+C`, `Ctrl+V`. Check: pasted as a new layer in place.
6. File > Export artboards… to a folder. Check: one PNG per artboard, each at its artboard size
   (`magick identify`).

## S5. Print hand-off

Pattern: proof against the press profile, fix out-of-gamut colour, deliver CMYK.

1. Open S4's project. View > Proof setup: choose a CMYK profile (Ghostscript's `default_cmyk.icc`).
2. `Ctrl+Y` proof colours, `Ctrl+Shift+Y` gamut warning. Check: saturated blue flagged.
3. Image > Mode > CMYK. Check: status shows the mode; a vivid RGB fill lands in gamut.
4. File > Export CMYK TIFF…. Check: `magick identify -verbose` reports CMYK and an embedded profile.

## S6. Crop and clean-up

1. Open any sample, `M` select a region, Image > Crop to selection. Check: canvas size equals the
   selection box; `Ctrl+Z` restores the size.
2. Select again, Edit > Clear outside selection. Check: alpha 0 outside, original inside; the
   selection is still active.
3. Edit > Content-aware fill (`Shift+F5`) on a small selection. Check: filled with texture, no blur.
4. Image size (`Ctrl+Alt+I`) 50% on a 16-bit document (Image > Precision > 16-bit first). Check: no
   garbled stripes.

## S7. Automation and agent

1. Actions > Start recording, do three edits, Stop, Save. Open a new document, Play. Check: same
   result; one `Ctrl+Z` undoes the whole action.
2. Agent tab > Connect to Redrob console. Check: code shown and the console page opens; after
   approval the status reads connected. Disconnect deletes the stored key.
3. Agent tab > allow redrob-code, enter a task, Run. Check: proposals appear in PENDING PROPOSALS
   and nothing changes on canvas until one is approved.
