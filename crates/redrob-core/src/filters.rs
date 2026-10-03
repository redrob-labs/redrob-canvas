// SPDX-License-Identifier: GPL-3.0-or-later

use image::{ImageBuffer, Rgba};

use crate::precision::Precision;
use crate::{CoreError, Document, Filter, Result};

const MAX_FILTER_RADIUS: u32 = 4_096;

/// Upper bound on `NoiseReduction`'s window, taken from Krita's own declared range (0..10) rather
/// than from this crate's `MAX_FILTER_RADIUS`.
///
/// The filter is derived from that source, so its range comes from there too — inventing a wider
/// one would let a caller ask for something upstream never offers.
const KRITA_NOISE_MAX_WINDOW: u32 = 10;

/// Cap on `EdgeNeon`'s gain.
///
/// Upstream's own range is not recoverable -- the po file gives the parameter's NAME and dialog
/// position but no bounds -- so this is ours, chosen to refuse a value that could only be a
/// mistake while leaving every useful gain reachable. Recorded as a choice, not as a reading.
const MAX_NEON_AMOUNT: f64 = 100.0;

/// Cap on `MeanCurvatureBlur` iterations.
///
/// Each pass is a full image sweep over a 9-point stencil, so cost is linear in this number with
/// no window to amortise it — unlike a radius, where one large pass replaces many small ones. The
/// cap is here so a mistyped iteration count cannot turn into a hang that looks like a crash.
const MAX_CURVATURE_ITERATIONS: u32 = 256;

/// The layer a map filter reads its height field from, when it names one (H.18).
///
/// Separate from the filter's own match arm because it must run BEFORE the active layer is prepared
/// for editing: that borrow covers the document, and the map is a different layer.
fn map_source(filter: &Filter) -> Option<crate::NodeId> {
    match *filter {
        Filter::BumpMap { map, .. }
        | Filter::Displace { map, .. }
        | Filter::FractalTrace { map, .. }
        | Filter::WarpMap { map, .. }
        | Filter::VariableBlur { map, .. } => map,
        _ => None,
    }
}

/// Reads the named layer's canvas-sized pixels for the current frame.
///
/// A named layer that does not exist is an ERROR, not a silent fall back to the self-map: the command
/// asked for a specific map, and quietly shading a picture by its own brightness would look like the
/// filter working badly rather than like a missing layer.
fn resolve_map_plane(document: &Document, filter: &Filter) -> Result<Option<Vec<u8>>> {
    let Some(id) = map_source(filter) else {
        return Ok(None);
    };
    let layer = document.layer(id).ok_or(CoreError::LayerNotFound(id))?;
    let frame = document.current_frame_id();
    let pixels = match layer.kind() {
        crate::NodeKind::Raster => layer.raster_pixels(frame)?.to_vec(),
        // A text or vector layer is rasterised, so a shape can be a height field too -- that is one of
        // the useful cases (emboss a logo onto a surface), not an edge case to refuse.
        crate::NodeKind::Text | crate::NodeKind::Vector => {
            crate::semantic::rasterize(layer.content(), document.width(), document.height())?
        }
        // A group has no pixels of its own; its children do. Refused by name rather than read as empty,
        // which would silently flatten the filter into a no-op.
        crate::NodeKind::Group => return Err(CoreError::InvalidFilterParameter),
    };
    // The map must cover the canvas, since every map filter indexes it by destination pixel.
    if pixels.len() != document.width() as usize * document.height() as usize * 4 {
        return Err(CoreError::InvalidFilterParameter);
    }
    Ok(Some(pixels))
}

pub(crate) fn apply_filter(document: &mut Document, filter: &Filter) -> Result<()> {
    // J.1b. Filters are being moved onto the document's declared precision one at a time, and this
    // is the fork that makes "one at a time" safe.
    //
    // A filter on the NATIVE list has a single implementation that works in unit floats and is
    // therefore correct at every precision; it runs below and returns. Everything else is still
    // written against 8-bit bytes, and at a wider precision it is REFUSED by name.
    //
    // Refusing is the point. The tempting alternative — narrow to 8-bit, run the old arm, widen
    // back — makes every filter appear to work at 16-bit while throwing away the depth on each
    // application, with nothing reported. A user would have chosen 16-bit precisely to avoid that.
    // An error naming the filter is recoverable; silently flattened pixels are not.
    if filter.is_precision_native() {
        return apply_precision_native_filter(document, filter);
    }
    if document.precision() != Precision::U8 {
        return Err(CoreError::FilterPrecisionUnsupported(filter.name()));
    }
    let width = document.width();
    let height = document.height();
    // The map plane is resolved BEFORE the active layer is prepared for editing, because it reads a
    // DIFFERENT layer and the edit borrow would otherwise exclude it (H.18).
    let map_plane = resolve_map_plane(document, filter)?;
    document.prepare_active_raster_edit()?;
    let original = document.active_raster_pixels()?.to_vec();
    let mut filtered = original.clone();
    // The height field the map filters read: another layer when one is named, the layer's own pixels
    // otherwise. Borrowed rather than copied, so the common self-map case costs nothing.
    let map: &[u8] = map_plane.as_deref().unwrap_or(&original);

    match *filter {
        // Precision-native filters return before this match; the arm exists only so the
        // exhaustiveness check keeps working, which is what will catch the NEXT variant added
        // without an implementation. A wildcard here would silence exactly that.
        Filter::Invert => return Err(CoreError::FilterPrecisionUnsupported(filter.name())),
        // Same reason as Invert: `rgb_clip` is on PRECISION_NATIVE_FILTERS, so it returns before
        // this match and only exists here to keep the exhaustiveness check honest (K.1).
        Filter::RgbClip { .. } => return Err(CoreError::FilterPrecisionUnsupported(filter.name())),
        Filter::InvertLinear => return Err(CoreError::FilterPrecisionUnsupported(filter.name())),
        Filter::ColorEnhance => {
            // K.1. `gegl:color-enhance`. No parameters — it sits in `filters-actions.c`'s
            // non-interactive array and applies immediately.
            //
            // DERIVATION. The body is GEGL's and not vendored, as with the rest of K.1. But this
            // filter's one hard BEHAVIOURAL rule is in vendored source and is honoured exactly:
            // `filters-actions.c:1056` reads
            // `SET_SENSITIVE ("filters-color-enhance", writable && !force_nde && !gray)`. Upstream
            // DISABLES it on a greyscale image.
            //
            // That guard is not incidental. Twelve filters carry `!gray` and every one of them is
            // a chroma operation — c2g, color-balance, colorize, color-temperature, desaturate,
            // hue-saturation, mono-mixer, noise-hsv, red-eye-removal, saturation, sepia, and this.
            // So the guard also tells us WHAT the filter is: it works on saturation, which a grey
            // image does not have.
            //
            // Refused rather than silently doing nothing. A filter that runs and changes no pixel
            // is indistinguishable from one that is broken, and upstream greys the menu item out
            // precisely so the user is told instead of guessing.
            if document.color_mode() == crate::ColorMode::Grayscale {
                return Err(CoreError::FilterRequiresColor(filter.name()));
            }
            // SATURATION only, to its full range. Hue and value are left alone, which is what
            // separates this from `StretchContrastHsv` — that one stretches value as well, and a
            // filter called "colour enhance" that also changed brightness would be doing two
            // things under one name.
            let mut low = f32::MAX;
            let mut high = 0.0f32;
            let mut any = false;
            for pixel in original.chunks_exact(4) {
                // Transparent pixels excluded, as everywhere else in K.1: their stored colour is
                // usually zero, which would peg the minimum and leave the filter inert on any
                // cut-out image.
                if pixel[3] == 0 {
                    continue;
                }
                any = true;
                let (_, s, _) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);
                low = low.min(s);
                high = high.max(s);
            }
            let span = high - low;
            if any && span > 1e-6 {
                for (output, input) in filtered.chunks_exact_mut(4).zip(original.chunks_exact(4)) {
                    let (h, s, v) = rgb_to_hsv(input[0], input[1], input[2]);
                    let stretched = ((s - low) / span).clamp(0.0, 1.0);
                    let (r, g, b) = hsv_to_rgb(h, stretched, v);
                    output[0] = r;
                    output[1] = g;
                    output[2] = b;
                }
            }
        }
        Filter::HighPass { std_dev, contrast } => {
            // K.1. `gegl:high-pass`. Interactive (it sits past line 131 in
            // `filters-actions.c`'s dialog array), so it is parameterised — unlike the
            // non-interactive filters earlier in K.1.
            //
            // DERIVATION. The body is GEGL's and not vendored. Two things the local tree does
            // settle: it carries NO `!gray` sensitivity guard, so unlike `color-enhance` it is
            // valid on a greyscale image; and `app/gegl/gimp-gegl-apply-operation.c:642` shows the
            // blur it is built on, `gegl:gaussian-blur`, taking `std-dev-x` / `std-dev-y` — which
            // is where the `std_dev` name here comes from rather than from my own invention.
            //
            // (That same call also passes an explicit `abyss-policy`, which is independent
            // confirmation that the edge policy K.0 made shared is a real axis upstream names too.)
            //
            // A high pass is the image MINUS a blurred copy of itself: the blur keeps the low
            // spatial frequencies, so subtracting it leaves the high ones. The result is centred
            // on mid-grey because the difference is signed and an unsigned buffer cannot hold
            // negative detail — without the offset every darker-than-local pixel would clamp to
            // black and half the detail would be gone.
            if !std_dev.is_finite()
                || !(0.1..=1500.0).contains(&std_dev)
                || !contrast.is_finite()
                || !(0.0..=10.0).contains(&contrast)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            // Blurred through the SAME path `Filter::GaussianBlur` uses, so high-pass and the blur
            // filter cannot disagree about what a blur of a given std-dev is. A box blur would
            // have been cheaper and is the wrong kernel: its square support puts visible ringing
            // along every edge, which in a filter whose entire output IS edges would be the only
            // thing anyone saw.
            let premultiplied = premultiply(&original);
            let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(width, height, premultiplied).ok_or_else(|| {
                    CoreError::MalformedProject("could not construct filter raster".into())
                })?;
            let blurred = unpremultiply(image::imageops::blur(&image, std_dev).into_raw());

            for (index, (output, input)) in filtered
                .chunks_exact_mut(4)
                .zip(original.chunks_exact(4))
                .enumerate()
            {
                for channel in 0..3 {
                    let detail =
                        f64::from(input[channel]) - f64::from(blurred[index * 4 + channel]);
                    let scaled = 128.0 + detail * f64::from(contrast);
                    output[channel] = scaled.round().clamp(0.0, 255.0) as u8;
                }
                // Alpha untouched: the detail being extracted is colour detail, and rewriting
                // coverage would change the shape of the layer rather than its content.
            }
        }
        Filter::ValueInvert => {
            // K.1. `gegl:value-invert`. Non-interactive (line 85 of `filters-actions.c`, inside
            // the array that applies with no dialog), so parameterless — and the vendored invert
            // wrappers confirm that shape for this family: `gimp_gegl_apply_invert_gamma` and
            // `gimp_gegl_apply_invert_linear` at `gimp-gegl-apply-operation.c:653` and `:674` each
            // build their node with `gegl_node_new_child(NULL, "operation", <name>, NULL)` — no
            // properties at all.
            //
            // No `!gray` guard upstream, unlike the twelve chroma filters, and that is consistent:
            // inverting VALUE is meaningful on a grey image, where inverting saturation would not
            // be.
            //
            // Inverts the HSV VALUE, keeping hue and saturation. That makes it a third, distinct
            // member of the invert family: `invert-gamma` (our `Filter::Invert`) complements each
            // stored channel, `invert-linear` complements in linear light, and this one complements
            // only the brightness and leaves the colour's identity alone.
            //
            // THE CONSEQUENCE WORTH KNOWING: HSV saturation is RELATIVE to value (`delta / max`),
            // so holding S while inverting V does NOT preserve the absolute channel spread — a
            // dark saturated colour becomes a light colour of the same hue and the same
            // *proportional* saturation, which is a much wider absolute spread. That is the
            // operation as defined, not a rounding artefact, and it is why the result looks
            // different from an RGB invert rather than merely lighter.
            for pixel in filtered.chunks_exact_mut(4) {
                let (h, s, v) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);
                let (r, g, b) = hsv_to_rgb(h, s, 1.0 - v);
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
                // Alpha untouched: inverting coverage would turn a transparent area opaque, which
                // is not what inverting a colour means.
            }
        }
        Filter::AlienMap {
            model,
            cpn1_frequency,
            cpn1_phase,
            cpn1_enabled,
            cpn2_frequency,
            cpn2_phase,
            cpn2_enabled,
            cpn3_frequency,
            cpn3_phase,
            cpn3_enabled,
        } => {
            // K.2. `gegl:alien-map`, a sinusoidal remap per channel.
            //
            // The unit choices below are not free: both come from upstream's own blurbs, preserved
            // in the vendored translation catalogues. "Number of cycles covering full value range"
            // makes frequency count FULL cycles over 0..1, so the argument advances by 2*pi per
            // unit of frequency. "Phase angle, range 0-360" makes phase degrees.
            //
            // The remap sends a channel to `0.5 * (1 + sin(theta))`, which is the only reading of
            // "map a value through a sine" that keeps the output inside 0..1 for every input. Note
            // it is NOT an identity at any setting -- a flat 0.5 at frequency 0 is the operation
            // working, not a bug -- which is why the per-channel enable exists and why it defaults
            // to on for all three.
            // Takes and returns a BYTE, because this is the byte path. The sine is computed in
            // f64 and only the final value narrows, so the 8-bit step is the single rounding in
            // the chain rather than one per term.
            let remap = |value: u8, frequency: f32, phase_degrees: f32| -> u8 {
                let unit = f64::from(value) / 255.0;
                let theta = unit * f64::from(frequency) * std::f64::consts::TAU
                    + f64::from(phase_degrees).to_radians();
                let mapped = 0.5 * (1.0 + theta.sin());
                (mapped * 255.0).round().clamp(0.0, 255.0) as u8
            };

            for pixel in filtered.chunks_exact_mut(4) {
                match model {
                    crate::command::AlienMapModel::Rgb => {
                        if cpn1_enabled {
                            pixel[0] = remap(pixel[0], cpn1_frequency, cpn1_phase);
                        }
                        if cpn2_enabled {
                            pixel[1] = remap(pixel[1], cpn2_frequency, cpn2_phase);
                        }
                        if cpn3_enabled {
                            pixel[2] = remap(pixel[2], cpn3_frequency, cpn3_phase);
                        }
                    }
                    crate::command::AlienMapModel::Hsl => {
                        // Uses the HSL pair, NOT the HSV one beside it. Upstream's third slider is
                        // "Luminosity" and HSL lightness is (max+min)/2 where HSV value is max --
                        // different numbers for any colour that is not a pure tint, so the HSV
                        // helper would remap a different channel than the one upstream names.
                        //
                        // My first draft used HSV and justified it with a comment claiming our hue
                        // was already a unit turn. It is not: rgb_to_hsv returns DEGREES, so the
                        // frequency would have been scaled by 360 on the hue slider alone.
                        //
                        // The rgb_to_hsl pair below this function already returns all three
                        // channels in 0..1, with hue as a fraction of a turn, which is exactly the
                        // domain alien-map needs: its frequency counts cycles "covering full value
                        // range", so a hue in degrees beside a 0..1 saturation would make one
                        // frequency unit mean 1/360th as much on one slider as on its neighbour.
                        let remap_unit = |value: f32, frequency: f32, phase_degrees: f32| -> f32 {
                            let theta =
                                f64::from(value) * f64::from(frequency) * std::f64::consts::TAU
                                    + f64::from(phase_degrees).to_radians();
                            (0.5 * (1.0 + theta.sin())) as f32
                        };
                        let (mut h, mut s, mut v) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
                        if cpn1_enabled {
                            h = remap_unit(h, cpn1_frequency, cpn1_phase);
                        }
                        if cpn2_enabled {
                            s = remap_unit(s, cpn2_frequency, cpn2_phase);
                        }
                        if cpn3_enabled {
                            v = remap_unit(v, cpn3_frequency, cpn3_phase);
                        }
                        let rgb = hsl_to_rgb(h, s, v);
                        pixel[0] = rgb[0];
                        pixel[1] = rgb[1];
                        pixel[2] = rgb[2];
                    }
                }
                // Alpha untouched, as with every colour operation here.
            }
        }
        Filter::ColorExchange {
            from,
            to,
            red_threshold,
            green_threshold,
            blue_threshold,
        } => {
            // K.2. `gegl:color-exchange`.
            //
            // The three thresholds are INDEPENDENT, so the matched region is an axis-aligned box
            // in RGB space. Using a single Euclidean distance would be a different operation, and
            // would accept colours this one rejects: with every threshold at 10, (10, 10, 10) away
            // from the target is inside the box but 17.3 away by distance.
            //
            // `abs_diff` on u8 rather than a signed subtraction, so a target near 0 or 255 cannot
            // wrap. That is the kind of thing that works on mid-tones and fails only at the ends.
            for pixel in filtered.chunks_exact_mut(4) {
                let matches = pixel[0].abs_diff(from.r) <= red_threshold
                    && pixel[1].abs_diff(from.g) <= green_threshold
                    && pixel[2].abs_diff(from.b) <= blue_threshold;
                if matches {
                    pixel[0] = to.r;
                    pixel[1] = to.g;
                    pixel[2] = to.b;
                    // Alpha untouched. Both colours' own alpha is ignored: this exchanges colour,
                    // and replacing coverage would let the operation erase or reveal pixels, which
                    // "swap one color with another" does not mean.
                }
            }
        }
        Filter::ColorRotate {
            source_from,
            source_to,
            dest_from,
            dest_to,
            gray_mode,
            gray_threshold,
            gray_hue,
            gray_saturation,
        } => {
            // K.2. `gegl:color-rotate`. Maps a hue arc onto another hue arc.
            //
            // Arcs on a circle are DIRECTIONAL, and that is the whole difficulty. An arc runs from
            // `from` in the increasing direction and may wrap past 360, so 300 -> 60 is a
            // 120-degree arc through red, not a 240-degree one the other way. Computing the span
            // as a plain subtraction would give -240 there and invert the mapping; `rem_euclid`
            // gives the arc that was actually asked for.
            let span = |from: f32, to: f32| -> f32 {
                let raw = (to - from).rem_euclid(360.0);
                // A `from` equal to `to` means the whole circle, not an empty arc: the dialog's
                // two handles coincide when the user selects everything. An empty arc would make
                // the filter a no-op at the setting where it should do the most.
                if raw < f32::EPSILON { 360.0 } else { raw }
            };
            let source_span = span(source_from, source_to);
            let dest_span = span(dest_from, dest_to);

            for pixel in filtered.chunks_exact_mut(4) {
                let (hue, saturation, value) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);

                // HSV, not HSL. No model is named in the strings -- both spaces have a saturation,
                // so "Gray Threshold" does not settle it -- but the angles are in degrees and our
                // `rgb_to_hsv` already returns degrees, where the HSL pair returns a unit turn.
                // Recorded as a choice, as with alien-map's HSL, which upstream DID name.
                let (mut hue, saturation) = if saturation < gray_threshold {
                    match gray_mode {
                        crate::command::GrayMode::ChangeToThis => {
                            // Replaced outright, no rotation. Written straight out so the arc
                            // logic below cannot touch it.
                            let (r, g, b) = hsv_to_rgb(gray_hue, gray_saturation, value);
                            pixel[0] = r;
                            pixel[1] = g;
                            pixel[2] = b;
                            continue;
                        }
                        // Lent the configured colour, then rotated like any other pixel.
                        crate::command::GrayMode::TreatAsThis => (gray_hue, gray_saturation),
                    }
                } else {
                    (hue, saturation)
                };

                // Position along the source arc. Outside it, the pixel is untouched -- "replace a
                // range of colors" means the rest of the wheel is not a range.
                let offset = (hue - source_from).rem_euclid(360.0);
                if offset <= source_span {
                    let fraction = offset / source_span;
                    hue = (dest_from + fraction * dest_span).rem_euclid(360.0);
                }
                // Outside the arc the hue is left as it stands -- which for a grey under
                // "treat as this" is the hue it was just lent. That is what makes the two gray
                // modes differ ONLY in whether the rotation can apply, rather than in the colour
                // the grey receives.

                let (r, g, b) = hsv_to_rgb(hue, saturation, value);
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
                // Alpha untouched.
            }
        }
        Filter::ColorToAlpha {
            color,
            transparency_threshold,
            opacity_threshold,
        } => {
            // K.2. `gegl:color-to-alpha`.
            //
            // Chebyshev distance -- the MAX of the per-channel absolute differences -- taken from
            // the vendored prop GUI's own pick callback. Not Euclidean, and not the three
            // independent thresholds `color-exchange` has.
            //
            // A degenerate band (opacity at or below transparency) becomes a hard cutoff rather
            // than a division by zero or a negative ramp.
            let band = opacity_threshold - transparency_threshold;

            for pixel in filtered.chunks_exact_mut(4) {
                let distance = [
                    (f32::from(pixel[0]) - f32::from(color.r)).abs(),
                    (f32::from(pixel[1]) - f32::from(color.g)).abs(),
                    (f32::from(pixel[2]) - f32::from(color.b)).abs(),
                ]
                .into_iter()
                .fold(0.0f32, f32::max)
                    / 255.0;

                // How opaque this pixel should END UP, before its existing alpha is accounted for.
                let coverage = if band <= f32::EPSILON {
                    if distance <= transparency_threshold {
                        0.0
                    } else {
                        1.0
                    }
                } else {
                    ((distance - transparency_threshold) / band).clamp(0.0, 1.0)
                };

                if coverage >= 1.0 {
                    continue;
                }

                if coverage <= 0.0 {
                    pixel[3] = 0;
                    // The colour is left as it stands. At zero coverage it is invisible, and
                    // inventing a value for it would be a guess that only shows up if something
                    // later un-multiplies it.
                    continue;
                }

                // UNMIXING, which is what makes this "color to alpha" and not "color to mask".
                //
                // The pixel is being read as the target colour composited UNDER some unknown
                // colour at `coverage`, so the unknown is recovered by inverting source-over:
                //   observed = c' * coverage + target * (1 - coverage)
                //   c'       = (observed - target * (1 - coverage)) / coverage
                //
                // That inversion is derivable from the compositing law rather than guessed. It is
                // what stops the kept pixels carrying a tint of the removed colour -- the whole
                // point when knocking a background out, and the reason a plain alpha mask is not a
                // substitute.
                let unmix = |observed: u8, target: u8| -> u8 {
                    let observed = f32::from(observed) / 255.0;
                    let target = f32::from(target) / 255.0;
                    let recovered = (observed - target * (1.0 - coverage)) / coverage;
                    // Clamped because the inversion can overshoot the representable range when
                    // the observed pixel is not in fact a mixture of the target and anything
                    // displayable -- a real case, not a theoretical one, since the user picks the
                    // target by eye.
                    (recovered.clamp(0.0, 1.0) * 255.0).round() as u8
                };
                pixel[0] = unmix(pixel[0], color.r);
                pixel[1] = unmix(pixel[1], color.g);
                pixel[2] = unmix(pixel[2], color.b);

                // Composed with the alpha the pixel already had, so running this on an already
                // part-transparent area cannot make it MORE opaque.
                pixel[3] = ((f32::from(pixel[3]) * coverage).round()).clamp(0.0, 255.0) as u8;
            }
        }
        Filter::ComponentExtract { component } => {
            // K.2. `gegl:component-extract`. One component, rendered as a mono image.
            use crate::command::ColorComponent as Cc;

            for pixel in filtered.chunks_exact_mut(4) {
                let (r, g, b) = (pixel[0], pixel[1], pixel[2]);

                let sample: u8 = match component {
                    Cc::Red => r,
                    Cc::Green => g,
                    Cc::Blue => b,
                    Cc::Alpha => pixel[3],
                    Cc::Hue => {
                        let (hue, _, _) = rgb_to_hsv(r, g, b);
                        // Degrees scaled into a byte. A grey has no hue and reports 0, which is
                        // red's position rather than a meaningful value -- unavoidable when the
                        // component is undefined and the output is one byte wide.
                        ((hue / 360.0) * 255.0).round().clamp(0.0, 255.0) as u8
                    }
                    Cc::Saturation => {
                        let (_, saturation, _) = rgb_to_hsv(r, g, b);
                        (saturation * 255.0).round().clamp(0.0, 255.0) as u8
                    }
                    Cc::Value => {
                        let (_, _, value) = rgb_to_hsv(r, g, b);
                        (value * 255.0).round().clamp(0.0, 255.0) as u8
                    }
                    Cc::Lightness => {
                        // HSL, not HSV: lightness is (max+min)/2 where value is max.
                        let (_, _, lightness) = rgb_to_hsl(r, g, b);
                        (lightness * 255.0).round().clamp(0.0, 255.0) as u8
                    }
                    Cc::Luminance => {
                        // Rec. 709, matching the weights `channel.rs` and `color_mode.rs` already
                        // use. Not the mean: pure green is 182, not 85.
                        (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b))
                            .round()
                            .clamp(0.0, 255.0) as u8
                    }
                    Cc::CmykCyan | Cc::CmykMagenta | Cc::CmykYellow | Cc::CmykKey => {
                        // On the STORED, gamma-encoded values: an unprofiled separation has no
                        // colorimetry to speak of, so decoding to linear first would add a step
                        // that means nothing here.
                        let (c, m, y, k) = crate::color::srgb_to_device_cmyk(
                            f64::from(r) / 255.0,
                            f64::from(g) / 255.0,
                            f64::from(b) / 255.0,
                        );
                        let channel = match component {
                            Cc::CmykCyan => c,
                            Cc::CmykMagenta => m,
                            Cc::CmykYellow => y,
                            _ => k,
                        };
                        (channel * 255.0).round().clamp(0.0, 255.0) as u8
                    }
                    Cc::LabLightness
                    | Cc::LabA
                    | Cc::LabB
                    | Cc::LchChroma
                    | Cc::LchHue
                    | Cc::YuvU
                    | Cc::YuvV
                    | Cc::XyyX
                    | Cc::XyyY => {
                        // Lab is reached the way the rest of this crate reaches it: decode to
                        // linear, to XYZ, to Lab against D65. Reusing that path rather than a
                        // shortcut keeps one definition of Lab in the codebase.
                        let linear = |c: u8| crate::color::srgb_to_linear(f64::from(c) / 255.0);
                        let (x, y, z) =
                            crate::color::linear_srgb_to_xyz(linear(r), linear(g), linear(b));
                        let (l, a, bb) = crate::color::xyz_to_lab(x, y, z, crate::color::D65);
                        (match component {
                            // L* is 0..100.
                            Cc::LabLightness => ((l / 100.0) * 255.0).round().clamp(0.0, 255.0),
                            // a* and b* are SIGNED, roughly -128..127, so they are offset by 128
                            // to be displayable. Without the offset every negative value would
                            // clamp to 0 and half of each axis would render as flat black.
                            Cc::LabA => (a + 128.0).round().clamp(0.0, 255.0),
                            Cc::LabB => (bb + 128.0).round().clamp(0.0, 255.0),
                            Cc::LchChroma => {
                                let (_, chroma, _) = crate::color::lab_to_lch(l, a, bb);
                                // Chroma is unbounded in principle but sRGB cannot exceed about
                                // 133, so 150 is the scale -- chosen so no in-gamut colour clips,
                                // rather than so the common case fills the range.
                                ((chroma / 150.0) * 255.0).round().clamp(0.0, 255.0)
                            }
                            Cc::LchHue => {
                                let (_, _, hue) = crate::color::lab_to_lch(l, a, bb);
                                ((hue / 360.0) * 255.0).round().clamp(0.0, 255.0)
                            }
                            Cc::YuvU | Cc::YuvV => {
                                let (_, u, v) = crate::color::xyz_to_yuv(x, y, z);
                                // u' spans about 0..0.62 and v' about 0..0.59 over the visible
                                // locus, so both are scaled by 0.7: one shared scale keeps the two
                                // axes comparable, which is the whole point of a UNIFORM
                                // chromaticity diagram.
                                let channel = if matches!(component, Cc::YuvU) { u } else { v };
                                ((channel / 0.7) * 255.0).round().clamp(0.0, 255.0)
                            }
                            _ => {
                                let (cx, cy, _) = crate::color::xyz_to_xyy(x, y, z);
                                let channel = if matches!(component, Cc::XyyX) {
                                    cx
                                } else {
                                    cy
                                };
                                (channel * 255.0).round().clamp(0.0, 255.0)
                            }
                        }) as u8
                    }
                };

                pixel[0] = sample;
                pixel[1] = sample;
                pixel[2] = sample;
                // Alpha is preserved, even when alpha is the component being extracted: the
                // result is an IMAGE of the component, so making it transparent where the
                // component is dark would hide the very thing being inspected.
            }
        }
        Filter::MonoMixer {
            red_gain,
            green_gain,
            blue_gain,
            preserve_luminosity,
        } => {
            // K.2. `gegl:mono-mixer`. Three channels to one grey.
            let (mut wr, mut wg, mut wb) = (red_gain, green_gain, blue_gain);

            if preserve_luminosity {
                // Normalise so the weights sum to 1, which is what keeps a change of BALANCE from
                // also being a change of brightness.
                let sum = wr + wg + wb;
                if sum.abs() > f32::EPSILON {
                    wr /= sum;
                    wg /= sum;
                    wb /= sum;
                }
                // A zero sum is left alone rather than divided by: gains of (1, 0, -1) sum to zero
                // and are a legitimate difference-of-channels setting, so normalising them is
                // impossible and refusing would be worse than passing them through.
            }

            for pixel in filtered.chunks_exact_mut(4) {
                let grey =
                    wr * f32::from(pixel[0]) + wg * f32::from(pixel[1]) + wb * f32::from(pixel[2]);
                // Clamped, because the gains are unbounded and are MEANT to be: a gain above one
                // or below zero is how the filter emphasises or subtracts a channel, so overflow
                // is the normal case rather than an error.
                let grey = grey.round().clamp(0.0, 255.0) as u8;
                pixel[0] = grey;
                pixel[1] = grey;
                pixel[2] = grey;
                // Alpha untouched.
            }
        }
        Filter::Sepia { strength } => {
            // K.2, last of the group. `gegl:sepia`.
            //
            // Built from this crate's own Rec. 709 luminance -- the same weights `channel.rs`,
            // `color_mode.rs` and `ComponentExtract::Luminance` use -- then tinted. Reusing that
            // one definition of luminance matters more here than anywhere else in the group,
            // because the tone is OUR choice: at least the monochrome underneath it is the same
            // monochrome the rest of the codebase produces.
            //
            // The tint multiplies, so black stays black and white becomes the warm end of the
            // ramp. An additive tint would lift the blacks into a grey-brown haze, which is what
            // a cheap sepia filter looks like and is not what toned silver does.
            const TINT: (f32, f32, f32) = (1.0, 0.85, 0.65);

            let blend = strength.clamp(0.0, 1.0);

            for pixel in filtered.chunks_exact_mut(4) {
                let luma = 0.2126 * f32::from(pixel[0])
                    + 0.7152 * f32::from(pixel[1])
                    + 0.0722 * f32::from(pixel[2]);

                let toned = [luma * TINT.0, luma * TINT.1, luma * TINT.2];
                for (channel, target) in pixel[0..3].iter_mut().zip(toned) {
                    // Blended against the ORIGINAL channel, so strength 0 is exactly the input
                    // image rather than approximately it.
                    let mixed = f32::from(*channel) * (1.0 - blend) + target * blend;
                    *channel = mixed.round().clamp(0.0, 255.0) as u8;
                }
                // Alpha untouched.
            }
        }
        Filter::Colorize {
            hue,
            saturation,
            lightness,
        } => {
            // K.2, last of the group. `gimp:colorize`, ported from
            // `app/operations/gimpoperationcolorize.c` -- the only filter in this group whose
            // exact arithmetic is vendored rather than reconstructed.
            //
            // GIMP'S OWN LUMINANCE WEIGHTS, not Rec. 709. From
            // `libgimpcolor/gimpcolor-private.h`:
            //     GIMP_RGB_LUMINANCE_RED    0.22248840
            //     GIMP_RGB_LUMINANCE_GREEN  0.71690369
            //     GIMP_RGB_LUMINANCE_BLUE   0.06060791
            // against Rec. 709's 0.2126 / 0.7152 / 0.0722, which every other filter in this crate
            // uses. The divergence is deliberate HERE and only here: this is a faithful port and
            // the weights are part of what is being ported. Do not "unify" them -- the result
            // would stop matching upstream for no gain.
            const LUMA_R: f32 = 0.222_488_4;
            const LUMA_G: f32 = 0.716_903_7;
            const LUMA_B: f32 = 0.060_607_91;

            for pixel in filtered.chunks_exact_mut(4) {
                // Luminance is computed on LINEAR values. Upstream's `prepare()` says why, in as
                // many words: "GIMP_RGB_LUMINANCE() requires the input to be linear RGB for
                // correctness." Our samples are stored gamma-encoded, so they are decoded first.
                let linear = |c: u8| crate::color::srgb_to_linear(f64::from(c) / 255.0) as f32;
                let mut lum = LUMA_R * linear(pixel[0])
                    + LUMA_G * linear(pixel[1])
                    + LUMA_B * linear(pixel[2]);

                // Two different operations, not one signed one, exactly as upstream writes them.
                // Positive lightness LERPS toward white; negative SCALES toward black. (Upstream
                // spells the positive case as `lum * (1 - L)` then `lum += 1 - (1 - L)`, which is
                // the same thing written in two statements.)
                if lightness > 0.0 {
                    lum = lum * (1.0 - lightness) + lightness;
                } else if lightness < 0.0 {
                    lum *= lightness + 1.0;
                }

                // ...and the result is written as NON-LINEAR, which is upstream's documented
                // inconsistency rather than ours. Its `prepare()` comment: "Technically it looks
                // like our code is returning non-linear RGB so we should set the output format to
                // R'G'B'A float. I leave this like this for now as it's the algorithm we used for
                // years."
                //
                // Our buffer IS non-linear, so writing the HSL conversion's output directly is
                // what reproduces the pixels GIMP actually produces. Decoding it as linear
                // instead would "correct" the filter into disagreeing with upstream.
                let rgb = hsl_to_rgb(hue, saturation, lum);
                pixel[0] = rgb[0];
                pixel[1] = rgb[1];
                pixel[2] = rgb[2];
                // Alpha copied through, as upstream does with `dest[ALPHA] = src[ALPHA]`.
            }
        }
        Filter::MedianBlur {
            radius,
            edge_policy,
        } => {
            // K.3. `gegl:median-blur`, square neighbourhood.
            //
            // A median is not a weighted sum, so this cannot go through `convolve`: the whole
            // point is that it picks an EXISTING sample rather than mixing samples. That is also
            // why it removes salt-and-pepper noise where a box blur only spreads it -- an
            // out-of-range outlier cannot survive a median, but it always moves a mean.
            validate_radius(radius)?;
            let view =
                crate::neighbourhood::Neighbourhood::new(&original, width, height, edge_policy);
            let reach = radius as i64;

            // Each channel is taken independently. Deliberate, and worth stating: a per-channel
            // median can emit a colour that appears nowhere in the neighbourhood, because the red
            // winner and the green winner may come from different pixels. The alternative --
            // ranking whole pixels by some scalar -- needs a definition of "middle colour" that
            // does not exist, so every implementation of this filter takes channels separately.
            let mut samples: Vec<u8> =
                Vec::with_capacity(((2 * reach + 1) * (2 * reach + 1)) as usize);

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;
                    for channel in 0..4 {
                        samples.clear();
                        for dy in -reach..=reach {
                            for dx in -reach..=reach {
                                // A policy that resolves no coordinate contributes no sample,
                                // rather than contributing a zero: a transparent-black edge must
                                // not drag the median of an opaque region toward black.
                                if let Some(offset) = view.offset(x + dx, y + dy) {
                                    samples.push(original[offset + channel]);
                                }
                            }
                        }
                        if samples.is_empty() {
                            continue;
                        }
                        // `select_nth_unstable` is a partial sort: it places the middle element
                        // correctly without ordering the rest, which is all a median needs.
                        let middle = samples.len() / 2;
                        let (_, median, _) = samples.select_nth_unstable(middle);
                        filtered[target + channel] = *median;
                    }
                }
            }
        }
        Filter::MeanCurvatureBlur {
            iterations,
            edge_policy,
        } => {
            // K.3. `gegl:mean-curvature-blur`.
            //
            // Mean curvature motion, from the equation the operation's name states:
            //
            //         I_xx * I_y^2  -  2 * I_x * I_y * I_xy  +  I_yy * I_x^2
            // I_t  =  -------------------------------------------------------
            //                        I_x^2 + I_y^2
            //
            // Derivatives by central differences. The denominator is the squared gradient
            // MAGNITUDE, which is why this smooths along edges rather than across them: where the
            // gradient is strong the denominator is large and the pixel barely moves, so an edge
            // survives while the noise beside it is flattened. That is the whole reason to prefer
            // it over a box blur, and it falls out of the equation rather than being tuned in.
            if iterations == 0 || iterations > MAX_CURVATURE_ITERATIONS {
                return Err(CoreError::InvalidFilterParameter);
            }

            let w = width as i64;
            let h = height as i64;
            // Each pass must read the PREVIOUS pass's output, not its own partial results, or the
            // flow propagates across the image within one pass and the iteration count stops
            // meaning anything.
            let mut source = original.clone();

            for _ in 0..iterations {
                let mut next = source.clone();
                let view =
                    crate::neighbourhood::Neighbourhood::new(&source, width, height, edge_policy);

                for y in 0..h {
                    for x in 0..w {
                        let target = (y as usize * width as usize + x as usize) * 4;
                        for channel in 0..3 {
                            let at = |dx: i64, dy: i64| -> f64 {
                                view.channel_or_zero(x + dx, y + dy, channel)
                            };

                            let dx = (at(1, 0) - at(-1, 0)) / 2.0;
                            let dy = (at(0, 1) - at(0, -1)) / 2.0;
                            let magnitude = dx * dx + dy * dy;

                            // A flat neighbourhood has no level set to move, and the equation is
                            // 0/0 there. Leaving the pixel alone is the limit, not a guess: with
                            // no gradient there is no curve to shorten.
                            if magnitude < 1e-9 {
                                continue;
                            }

                            let centre = at(0, 0);
                            let dxx = at(1, 0) - 2.0 * centre + at(-1, 0);
                            let dyy = at(0, 1) - 2.0 * centre + at(0, -1);
                            let dxy = (at(1, 1) - at(1, -1) - at(-1, 1) + at(-1, -1)) / 4.0;

                            let flow =
                                (dxx * dy * dy - 2.0 * dx * dy * dxy + dyy * dx * dx) / magnitude;

                            // A quarter step. The step size is the one thing the operation's name
                            // does NOT fix, and a quarter is the largest that keeps the explicit
                            // scheme stable on a 4-neighbour stencil -- taking a full step makes
                            // the flow oscillate instead of converge, which looks like noise being
                            // added rather than removed.
                            next[target + channel] =
                                (centre + 0.25 * flow).clamp(0.0, 255.0).round() as u8;
                        }
                        // Alpha untouched: coverage has no level sets to smooth here, and moving
                        // it would soften the edge of a mask the user drew deliberately.
                    }
                }
                source = next;
            }
            filtered = source;
        }
        Filter::FocusBlur {
            shape,
            x: centre_x,
            y: centre_y,
            radius,
            aspect_ratio,
            rotation,
            focus,
            midpoint,
            blur_radius,
            edge_policy,
        } => {
            // K.3. `gegl:focus-blur`.
            use crate::command::FocusShape;

            validate_radius(blur_radius)?;
            if ![
                centre_x,
                centre_y,
                radius,
                aspect_ratio,
                rotation,
                focus,
                midpoint,
            ]
            .iter()
            .all(|v| v.is_finite())
                || radius <= 0.0
                || aspect_ratio <= 0.0
                || !(0.0..=1.0).contains(&focus)
                || !(0.0..=1.0).contains(&midpoint)
            {
                return Err(CoreError::InvalidFilterParameter);
            }

            let view =
                crate::neighbourhood::Neighbourhood::new(&original, width, height, edge_policy);

            // Upstream's GUI stores the centre as a fraction of the canvas and the region as a
            // fraction of the WIDTH, halved into a radius. Reproducing that here is what makes a
            // saved focus region land in the same place on a resized canvas.
            let cx = f64::from(centre_x) * f64::from(width);
            let cy = f64::from(centre_y) * f64::from(height);
            let extent = f64::from(radius) * f64::from(width) / 2.0;
            let (sin_r, cos_r) = f64::from(rotation).to_radians().sin_cos();

            // Inside this fraction of the region nothing is blurred at all.
            let inner = f64::from(focus);
            let mid = f64::from(midpoint).clamp(0.001, 0.999);

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    // Into the region's own frame: translate, then UN-rotate, then undo the
                    // aspect ratio. Dividing by the ratio after rotating is what makes a rotated
                    // ellipse an ellipse rather than a sheared one.
                    let dx = x as f64 - cx;
                    let dy = y as f64 - cy;
                    let rx = dx * cos_r + dy * sin_r;
                    let ry = (-dx * sin_r + dy * cos_r) / f64::from(aspect_ratio);

                    let distance = match shape {
                        FocusShape::Circle => (rx * rx + ry * ry).sqrt(),
                        FocusShape::Square => rx.abs().max(ry.abs()),
                        FocusShape::Diamond => rx.abs() + ry.abs(),
                        // A band ignores the along-band axis entirely, which is what makes it a
                        // band rather than a very flat ellipse.
                        FocusShape::Horizontal => ry.abs(),
                        FocusShape::Vertical => rx.abs(),
                    } / extent;

                    // Sharp inside `focus`, fully blurred at the region's edge, ramping between.
                    let strength = if distance <= inner {
                        0.0
                    } else if distance >= 1.0 {
                        1.0
                    } else {
                        let t = (distance - inner) / (1.0 - inner);
                        // `midpoint` moves the half-blur point without moving either limit, so a
                        // power curve is the right shape: it pins t=0 and t=1 and slides
                        // everything between. A linear bias would have to clamp and would flatten
                        // one end.
                        t.powf(mid.ln() / 0.5f64.ln())
                    };

                    let target = (y as usize * width as usize + x as usize) * 4;
                    if strength <= 0.0 {
                        continue;
                    }

                    // Per-pixel radius: this is a VARIABLE blur, so the window grows with the
                    // distance rather than the whole image being blurred and then cross-faded.
                    // Cross-fading would leave sharp detail ghosting through the blurred edges.
                    let local = (strength * f64::from(blur_radius)).round() as i64;
                    if local < 1 {
                        continue;
                    }

                    for channel in 0..4 {
                        let (sum, count) = view.window_sum(x, y, local, channel);
                        if count > 0 {
                            filtered[target + channel] =
                                (sum / count as f64).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        Filter::VariableBlur {
            radius,
            map: _,
            edge_policy,
        } => {
            // K.3. `gegl:variable-blur`.
            //
            // The map is already resolved into `map` above -- falling back to the layer's own
            // pixels when none is named -- so this arm never reads the layer list itself. The
            // field is matched as `_` for that reason, not because it is unused.
            validate_radius(radius)?;
            let view =
                crate::neighbourhood::Neighbourhood::new(&original, width, height, edge_policy);

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;

                    // The map's LUMA drives the radius, not one channel: a grey map is the normal
                    // case and reading only red would make a coloured map behave surprisingly.
                    // Rec. 709, the same weights the rest of the crate uses.
                    let m = &map[target..target + 4];
                    let amount = (0.2126 * f64::from(m[0])
                        + 0.7152 * f64::from(m[1])
                        + 0.0722 * f64::from(m[2]))
                        / 255.0;

                    let local = (amount * f64::from(radius)).round() as i64;
                    // Black map means sharp. Skipping rather than averaging a 1-pixel window
                    // keeps the untouched region BYTE-identical, so a map with hard edges gives a
                    // hard edge in the result instead of a faint seam.
                    if local < 1 {
                        continue;
                    }

                    for channel in 0..4 {
                        let (sum, count) = view.window_sum(x, y, local, channel);
                        if count > 0 {
                            filtered[target + channel] =
                                (sum / count as f64).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        Filter::SelectiveGaussianBlur {
            radius,
            max_delta,
            edge_policy,
        } => {
            // K.3. `gegl:gaussian-blur-selective`.
            validate_radius(radius)?;
            let view =
                crate::neighbourhood::Neighbourhood::new(&original, width, height, edge_policy);
            let reach = radius as i64;

            // Upstream's parameter is a RADIUS, so a sigma is chosen from it: a third of the
            // radius puts the window edge at three standard deviations, where the kernel is
            // already negligible. Picking a larger sigma would truncate a kernel that still had
            // weight at the boundary, which shows up as a faint square halo.
            let sigma = f64::from(radius) / 3.0;
            let two_sigma_squared = 2.0 * sigma * sigma;

            // The spatial weights are fixed, so they are built once rather than per pixel.
            let side = (2 * reach + 1) as usize;
            let mut weights = vec![0.0f64; side * side];
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let d2 = (dx * dx + dy * dy) as f64;
                    weights[((dy + reach) as usize) * side + (dx + reach) as usize] =
                        (-d2 / two_sigma_squared).exp();
                }
            }

            let delta = f64::from(max_delta);

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;
                    for channel in 0..4 {
                        let centre = f64::from(original[target + channel]);
                        let mut sum = 0.0;
                        let mut total_weight = 0.0;

                        for dy in -reach..=reach {
                            for dx in -reach..=reach {
                                let Some(value) = view.channel(x + dx, y + dy, channel) else {
                                    continue;
                                };
                                // THE SELECTIVE PART: a neighbour that differs from the centre by
                                // more than the delta does not contribute at all. That is what
                                // preserves an edge of any shape without the filter being told
                                // where one is.
                                if (value - centre).abs() > delta {
                                    continue;
                                }
                                let w =
                                    weights[((dy + reach) as usize) * side + (dx + reach) as usize];
                                sum += w * value;
                                total_weight += w;
                            }
                        }

                        // The centre always passes its own test, so the weight is never zero and
                        // there is no empty-window case to guard. Stated because the guard's
                        // absence would otherwise look like an oversight.
                        filtered[target + channel] =
                            (sum / total_weight).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::SnnMean {
            radius,
            edge_policy,
        } => {
            // K.3. `gegl:snn-mean`, symmetric nearest neighbour.
            validate_radius(radius)?;
            let view =
                crate::neighbourhood::Neighbourhood::new(&original, width, height, edge_policy);
            let reach = radius as i64;

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;
                    for channel in 0..4 {
                        let centre = f64::from(original[target + channel]);
                        // The centre is always its own nearest neighbour, so it counts once.
                        let mut sum = centre;
                        let mut count = 1usize;

                        // Half the window, so each pair is visited exactly once.
                        //
                        // My first comment here claimed that visiting the whole window would
                        // "reduce the filter to an ordinary mean". Reverse-verification showed
                        // that is WRONG and the claim is corrected rather than left standing:
                        // visiting both halves picks the SAME nearer member twice, so the picks'
                        // own mean is unchanged and the edge is preserved identically. What it
                        // actually changes is the CENTRE's weight against the picks — 1/(1+pairs)
                        // becomes 1/(1+2*pairs) — which halves the centre's influence and is
                        // visible on a lone speck: 54 against 47 at radius 2. Plus it does twice
                        // the work for that.
                        //
                        // So the symmetry is about weighting and cost, not about whether edges
                        // survive. A test pins the speck value so this choice is covered by a
                        // measurement rather than by an assertion in a comment.
                        for dy in -reach..=reach {
                            for dx in -reach..=reach {
                                // Skip the centre and the half already covered by its partner.
                                if dy < 0 || (dy == 0 && dx <= 0) {
                                    continue;
                                }

                                let a = view.channel(x + dx, y + dy, channel);
                                let b = view.channel(x - dx, y - dy, channel);

                                // When the policy resolves only one side, that side IS the nearer
                                // of what exists. Dropping the pair entirely would thin the
                                // sample set along every border and lighten the edge of the
                                // result; inventing the missing side would be worse.
                                let pick = match (a, b) {
                                    (Some(a), Some(b)) => {
                                        if (a - centre).abs() <= (b - centre).abs() {
                                            Some(a)
                                        } else {
                                            Some(b)
                                        }
                                    }
                                    (Some(only), None) | (None, Some(only)) => Some(only),
                                    (None, None) => None,
                                };

                                if let Some(value) = pick {
                                    sum += value;
                                    count += 1;
                                }
                            }
                        }

                        filtered[target + channel] =
                            (sum / count as f64).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::NoiseReduction {
            threshold,
            window_size,
            edge_policy,
        } => {
            // K.3. Purpose from `gegl:noise-reduction`, method from Krita's
            // `kis_simple_noise_reducer.cpp` -- see the variant's doc comment for why.
            if window_size > KRITA_NOISE_MAX_WINDOW {
                return Err(CoreError::InvalidFilterParameter);
            }
            // Zero is a legitimate no-op in upstream's own range: a one-pixel window makes the
            // blur the identity, so no pixel can differ from it. Returning early rather than
            // running the loop keeps that exact.
            if window_size == 0 {
                return Ok(());
            }

            let view =
                crate::neighbourhood::Neighbourhood::new(&original, width, height, edge_policy);
            let reach = window_size as i64;

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;

                    // A CIRCULAR window, as Krita's mask generator makes: samples outside the
                    // radius are excluded. A square window would pull in the corners, which sit
                    // sqrt(2) further away and so belong to a different neighbourhood than the
                    // one the window size names.
                    let mut blurred = [0.0f64; 4];
                    let mut count = 0usize;
                    for dy in -reach..=reach {
                        for dx in -reach..=reach {
                            if dx * dx + dy * dy > reach * reach {
                                continue;
                            }
                            let Some(offset) = view.offset(x + dx, y + dy) else {
                                continue;
                            };
                            for channel in 0..4 {
                                blurred[channel] += f64::from(original[offset + channel]);
                            }
                            count += 1;
                        }
                    }
                    if count == 0 {
                        continue;
                    }
                    for value in &mut blurred {
                        *value /= count as f64;
                    }

                    // The difference between the pixel and its own blurred self, as the MAXIMUM
                    // over the channels. Krita asks its colour space for a `difference`, which is
                    // space-specific and not readable here; a Chebyshev distance is the reading
                    // taken, consistent with `color-to-alpha`, whose metric was established from
                    // GIMP's own source. Recorded as a choice.
                    let difference = (0..3)
                        .map(|c| (blurred[c] - f64::from(original[target + c])).abs())
                        .fold(0.0f64, f64::max);

                    // THE POLARITY, and it is the opposite of a selective blur: a pixel is
                    // replaced only when it differs from its surroundings by MORE than the
                    // threshold. Below it, the pixel is left exactly as it was -- not re-averaged,
                    // not nudged -- so a clean image is byte-identical and only outliers move.
                    if difference > f64::from(threshold) {
                        for channel in 0..4 {
                            filtered[target + channel] =
                                blurred[channel].round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        Filter::DifferenceOfGaussians {
            radius1,
            radius2,
            normalize,
            invert,
        } => {
            // K.3. Both radii validated against the SAME bounds `Filter::GaussianBlur` uses, so a
            // std-dev this filter accepts is one the blur filter accepts too.
            for radius in [radius1, radius2] {
                if !radius.is_finite() || radius <= 0.0 || radius > 1_024.0 {
                    return Err(CoreError::InvalidFilterParameter);
                }
            }

            // Blurred through the SAME path `Filter::GaussianBlur` and high-pass use, for the same
            // reason: three filters that blur must not disagree about what a blur of a given
            // std-dev is.
            let blur_at = |std_dev: f64| -> Result<Vec<u8>> {
                let premultiplied = premultiply(&original);
                let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
                    ImageBuffer::from_raw(width, height, premultiplied).ok_or_else(|| {
                        CoreError::MalformedProject("could not construct filter raster".into())
                    })?;
                Ok(unpremultiply(
                    image::imageops::blur(&image, std_dev as f32).into_raw(),
                ))
            };
            let first = blur_at(radius1)?;
            let second = blur_at(radius2)?;

            // The difference is SIGNED and is kept that way until the end. Clamping here would
            // throw away the negative lobe before `normalize` could map it back into range, and
            // the negative lobe is half the output: the response across an edge is bipolar, a
            // trough on one side and a peak on the other.
            let mut difference = vec![0.0f64; width as usize * height as usize * 3];
            for (index, value) in difference.iter_mut().enumerate() {
                let pixel = index / 3;
                let channel = index % 3;
                *value =
                    f64::from(first[pixel * 4 + channel]) - f64::from(second[pixel * 4 + channel]);
            }

            // `_Normalize`: map the ACTUAL extremes onto 0..255, computed across all three channels
            // together rather than per channel. A per-channel stretch would pull the channels apart
            // by different amounts and tint every edge in the image.
            let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
            for value in &difference {
                low = low.min(*value);
                high = high.max(*value);
            }

            for (index, (output, input)) in filtered
                .chunks_exact_mut(4)
                .zip(original.chunks_exact(4))
                .enumerate()
            {
                for channel in 0..3 {
                    let raw = difference[index * 3 + channel];
                    let mut value = if normalize {
                        // A flat image has no range to stretch. Mapping it to anything but its own
                        // zero would invent contrast out of nothing.
                        if high - low > f64::EPSILON {
                            (raw - low) / (high - low) * 255.0
                        } else {
                            0.0
                        }
                    } else {
                        raw
                    };
                    if invert {
                        // Applied AFTER the stretch. Reverse-verification showed the two orders are
                        // IDENTICAL whenever `normalize` is on -- negating before a stretch that
                        // uses the data's own extremes gives (high - raw)/(high - low), the exact
                        // complement of (raw - low)/(high - low) -- so an earlier version of this
                        // comment claiming the flag would "do nothing at all" was simply wrong.
                        //
                        // The order IS observable with `normalize` off, which is the case a test
                        // now covers: a flat field has raw 0, so complementing after the clamp
                        // gives 255 (white) where negating before it would give 0 (black).
                        value = 255.0 - value;
                    }
                    output[channel] = value.round().clamp(0.0, 255.0) as u8;
                }
                // Alpha untouched: this filter reports where edges are, and rewriting coverage
                // would change the layer's shape rather than its content.
                output[3] = input[3];
            }
        }
        Filter::Antialias => {
            // K.3. `gegl:antialias`, the Scale3X edge-extrapolation algorithm named by the
            // replaced plug-in's own description.
            //
            // Clamp is used internally rather than exposed: the operation is parameterless
            // upstream, so offering an edge policy would be inventing a parameter. Clamp is also
            // the right answer here -- replicating the border keeps the equality tests meaningful,
            // where transparent black would invent a spurious edge along every side.
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );

            // Scale3X compares for EXACT equality, as the algorithm is defined. That is what makes
            // it safe on hard-edged art and nearly inert on a photograph.
            let same = |a: Option<usize>, b: Option<usize>| match (a, b) {
                (Some(a), Some(b)) => original[a..a + 4] == original[b..b + 4],
                _ => false,
            };

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;

                    // The 3x3 neighbourhood, named as Scale3X names it:
                    //   a b c
                    //   d e f
                    //   g h i
                    let a = view.offset(x - 1, y - 1);
                    let b = view.offset(x, y - 1);
                    let c = view.offset(x + 1, y - 1);
                    let d = view.offset(x - 1, y);
                    let e = view.offset(x, y);
                    let f = view.offset(x + 1, y);
                    let g = view.offset(x - 1, y + 1);
                    let h = view.offset(x, y + 1);
                    let i = view.offset(x + 1, y + 1);

                    let Some(centre) = e else {
                        continue;
                    };

                    // The four corner rules, each firing only on a genuine diagonal step: two
                    // neighbours equal to each other and both unequal to the opposite pair. A
                    // straight edge satisfies the first clause and fails the third, which is why
                    // straight edges come back untouched.
                    let db = same(d, b) && !same(b, f) && !same(d, h);
                    let bf = same(b, f) && !same(b, d) && !same(f, h);
                    let dh = same(d, h) && !same(d, b) && !same(h, f);
                    let hf = same(h, f) && !same(d, h) && !same(b, f);

                    // The nine subpixels. The edge-midpoint rules (1, 3, 5, 7) each take an extra
                    // "and the centre differs from the far corner" clause, which is what stops the
                    // extrapolation running across a corner that is already filled in.
                    let subpixels = [
                        if db { d } else { e },
                        if (db && !same(e, c)) || (bf && !same(e, a)) {
                            b
                        } else {
                            e
                        },
                        if bf { f } else { e },
                        if (db && !same(e, g)) || (dh && !same(e, a)) {
                            d
                        } else {
                            e
                        },
                        e,
                        if (bf && !same(e, i)) || (hf && !same(e, c)) {
                            f
                        } else {
                            e
                        },
                        if dh { d } else { e },
                        if (dh && !same(e, i)) || (hf && !same(e, g)) {
                            h
                        } else {
                            e
                        },
                        if hf { f } else { e },
                    ];

                    // Average the nine back down to one. Where no rule fired every subpixel is the
                    // centre, so this is exactly the centre again -- the byte-identical case.
                    for channel in 0..4 {
                        let mut sum = 0.0f64;
                        for subpixel in subpixels {
                            let offset = subpixel.unwrap_or(centre);
                            sum += f64::from(original[offset + channel]);
                        }
                        filtered[target + channel] =
                            (sum / subpixels.len() as f64).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::EdgeNeon { radius, amount } => {
            // K.4. Radius validated against the SAME bounds `Filter::GaussianBlur` uses.
            if !radius.is_finite() || radius <= 0.0 || radius > 1_024.0 {
                return Err(CoreError::InvalidFilterParameter);
            }
            if !amount.is_finite() || !(0.0..=MAX_NEON_AMOUNT).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }

            // Blurred through the SAME path GaussianBlur, high-pass and difference-of-gaussians
            // use, so four filters that blur cannot disagree about what a blur of a given std-dev
            // is.
            let premultiplied = premultiply(&original);
            let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(width, height, premultiplied).ok_or_else(|| {
                    CoreError::MalformedProject("could not construct filter raster".into())
                })?;
            let blurred = unpremultiply(image::imageops::blur(&image, radius as f32).into_raw());

            // Clamp internally rather than exposed: upstream has exactly two parameters, so an
            // edge policy would be a third one it does not have.
            let view = crate::neighbourhood::Neighbourhood::new(
                &blurred,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );

            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let target = (y as usize * width as usize + x as usize) * 4;
                    for channel in 0..3 {
                        // Central differences on the BLURRED image. Differentiating a Gaussian
                        // blur IS the Gaussian derivative, which is why this reuses the shared
                        // blur rather than building a derivative kernel of its own.
                        let gx = (view.channel_or_zero(x + 1, y, channel)
                            - view.channel_or_zero(x - 1, y, channel))
                            / 2.0;
                        let gy = (view.channel_or_zero(x, y + 1, channel)
                            - view.channel_or_zero(x, y - 1, channel))
                            / 2.0;
                        // BOTH axes, combined as a magnitude -- so the response does not depend on
                        // which way the edge runs. A gx-only version would read zero on every
                        // horizontal edge, which a test names.
                        let magnitude = gx.hypot(gy) * amount;
                        filtered[target + channel] = magnitude.round().clamp(0.0, 255.0) as u8;
                    }
                    // Alpha untouched: the filter reports where edges are, and rewriting coverage
                    // would change the layer's shape rather than its content.
                    filtered[target + 3] = original[target + 3];
                }
            }
        }
        Filter::Grayscale => {
            for pixel in filtered.chunks_exact_mut(4) {
                let luminance = luminance(pixel);
                pixel[0..3].fill(luminance);
            }
        }
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => {
            if !(-255..=255).contains(&brightness)
                || !contrast.is_finite()
                || !(-100.0..=100.0).contains(&contrast)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let contrast_255 = contrast * 2.55;
            let factor = (259.0 * (contrast_255 + 255.0)) / (255.0 * (259.0 - contrast_255));
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    *channel =
                        (factor * (f32::from(*channel) - 128.0) + 128.0 + f32::from(brightness))
                            .round()
                            .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::GaussianBlur { sigma } => {
            if !sigma.is_finite() || sigma <= 0.0 || sigma > 1_024.0 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let premultiplied = premultiply(&original);
            let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(width, height, premultiplied).ok_or_else(|| {
                    CoreError::MalformedProject("could not construct filter raster".into())
                })?;
            filtered = unpremultiply(image::imageops::blur(&image, sigma).into_raw());
        }
        Filter::Threshold { threshold } => {
            for pixel in filtered.chunks_exact_mut(4) {
                let value = if luminance(pixel) >= threshold {
                    255
                } else {
                    0
                };
                pixel[0..3].fill(value);
            }
        }
        Filter::Posterize { levels } => {
            if !(2..=256).contains(&levels) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let last = u32::from(levels - 1);
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    let bucket = (u32::from(*channel) * last + 127) / 255;
                    *channel = ((bucket * 255 + last / 2) / last) as u8;
                }
            }
        }
        Filter::Curves { ref points } => {
            // The curve is rebuilt per application rather than cached. Measured: a 256-entry table from a
            // dozen control points is a tridiagonal solve of ten unknowns plus 256 evaluations, which is
            // nothing beside the per-pixel loop below, and a cache keyed on a point list would have to be
            // invalidated on every edit.
            let curve = crate::ToneCurve::new(points.clone())
                .map_err(|_| CoreError::InvalidFilterParameter)?;
            let table = curve.transfer_table_8bit();
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    *channel = table[usize::from(*channel)];
                }
            }
        }
        Filter::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => {
            if input_black >= input_white
                || output_black > output_white
                || !gamma.is_finite()
                || !(0.01..=100.0).contains(&gamma)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let input_range = f32::from(input_white - input_black);
            let output_range = f32::from(output_white - output_black);
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    let normalized = ((f32::from(*channel) - f32::from(input_black)) / input_range)
                        .clamp(0.0, 1.0)
                        .powf(1.0 / gamma);
                    *channel = (f32::from(output_black) + normalized * output_range)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::HueSaturation {
            hue_degrees,
            saturation,
            lightness,
            hue_sectors,
            saturation_sectors,
            lightness_sectors,
            overlap,
        } => {
            // K.15. Ported from `app/operations/gimpoperationhuesaturation.c` and its config
            // object, replacing an ALL-range-only approximation.
            if !hue_degrees.is_finite()
                || !saturation.is_finite()
                || !lightness.is_finite()
                || !overlap.is_finite()
                || !(-180.0..=180.0).contains(&hue_degrees)
                || !(-100.0..=100.0).contains(&saturation)
                || !(-100.0..=100.0).contains(&lightness)
                || !(0.0..=1.0).contains(&overlap)
                || !hue_sectors.iter().all(|v| v.is_finite())
                || !saturation_sectors.iter().all(|v| v.is_finite())
                || !lightness_sectors.iter().all(|v| v.is_finite())
            {
                return Err(CoreError::InvalidFilterParameter);
            }

            // Upstream halves the overlap before using it.
            let overlap = overlap / 2.0;

            // The three maps, each taking the ALL contribution and ONE sector's. The way the two
            // combine differs per channel and is upstream's, not a simplification:
            //   hue        averages them -- `(hue[ALL] + hue[range]) / 2`
            //   saturation sums them, then scales -- `value *= (sum + 1)`
            //   lightness  sums them, then lifts or scales depending on the sign
            let map_hue = |value: f32, sector: usize| -> f32 {
                let shift = (hue_degrees / 360.0 + hue_sectors[sector] / 360.0) / 2.0;
                (value + shift).rem_euclid(1.0)
            };
            let map_saturation = |value: f32, sector: usize| -> f32 {
                let v = saturation / 100.0 + saturation_sectors[sector] / 100.0;
                (value * (v + 1.0)).clamp(0.0, 1.0)
            };
            let map_lightness = |value: f32, sector: usize| -> f32 {
                let v = lightness / 100.0 + lightness_sectors[sector] / 100.0;
                if v < 0.0 {
                    value * (v + 1.0)
                } else {
                    value + v * (1.0 - value)
                }
            };

            for pixel in filtered.chunks_exact_mut(4) {
                let (hue, sat, lit) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);

                // Which of the six sectors the hue falls in, found the way upstream finds it: the
                // unit hue times six, then the first threshold at `sector + 0.5` it falls below.
                // The half-offsets are why red spans the wrap point rather than starting at it.
                let h = hue * 6.0;
                let mut sector = 0usize;
                let mut secondary = 0usize;
                let mut use_secondary = false;
                let mut primary_intensity = 0.0f32;
                let mut secondary_intensity = 0.0f32;

                for counter in 0..7 {
                    let threshold = counter as f32 + 0.5;
                    if h < threshold + overlap {
                        sector = counter;
                        if overlap > 0.0 && h > threshold - overlap {
                            use_secondary = true;
                            secondary = counter + 1;
                            secondary_intensity = (h - threshold + overlap) / (2.0 * overlap);
                            primary_intensity = 1.0 - secondary_intensity;
                        }
                        break;
                    }
                }
                // Sector 6 is the wrap of sector 0 -- the seventh threshold exists only so the
                // top of the wheel is caught, and it maps back to red.
                if sector >= 6 {
                    sector = 0;
                    use_secondary = false;
                }
                if secondary >= 6 {
                    secondary = 0;
                }

                let (mut hue, mut sat, mut lit) = (hue, sat, lit);
                if use_secondary {
                    // Hue gets its own blended map because averaging two wrapped angles is not
                    // the same as blending two already-averaged results.
                    let primary_hue = map_hue(hue, sector);
                    let secondary_hue = map_hue(hue, secondary);
                    hue = (primary_hue * primary_intensity + secondary_hue * secondary_intensity)
                        .rem_euclid(1.0);
                    sat = map_saturation(sat, sector) * primary_intensity
                        + map_saturation(sat, secondary) * secondary_intensity;
                    lit = map_lightness(lit, sector) * primary_intensity
                        + map_lightness(lit, secondary) * secondary_intensity;
                } else if sat <= 0.0 {
                    // A GREY has no hue to shift and no saturation to scale, so only the ALL
                    // range's lightness applies -- upstream's `map_lightness_achromatic`. Running
                    // the sector maps here would read a sector chosen from an undefined hue.
                    let v = lightness / 100.0;
                    lit = if v < 0.0 {
                        lit * (v + 1.0)
                    } else {
                        lit + v * (1.0 - lit)
                    };
                } else {
                    hue = map_hue(hue, sector);
                    lit = map_lightness(lit, sector);
                    sat = map_saturation(sat, sector);
                }

                let [red, green, blue] = hsl_to_rgb(hue, sat, lit);
                pixel[0] = red;
                pixel[1] = green;
                pixel[2] = blue;
                // Alpha copied through, as upstream's `dest[3] = src[3]` does.
            }
        }
        Filter::BoxBlur { radius } => {
            validate_radius(radius)?;
            filtered = box_blur_rgba(&original, width, height, radius);
        }
        Filter::StretchContrast { keep_colors } => {
            // K.1. `gegl:stretch-contrast`.
            //
            // DERIVATION NOTE. This is a `gegl:` operation and GEGL is a separate project that
            // this repository does not vendor — `app/operations/` carries only the `gimp:` ops, so
            // there was no upstream body to re-derive from. What the local tree does establish is
            // that the operation exists and is required (`app/sanity.c` lists it), that it is
            // applied with no dialog (it sits in `filters-actions.c`'s non-interactive array), and
            // its name. The behaviour below follows from the operation's meaning rather than from
            // read source, and the one genuine design decision is named rather than buried.
            //
            // THE DECISION: `keep_colors` shares ONE range across the three channels; without it
            // each channel is stretched to its own range. Independent stretching moves the
            // channels by different amounts, so it shifts hue — on a photograph with a colour cast
            // that is a white balance, which is a useful operation and NOT this one. Shared is the
            // default for that reason, and because an image whose contrast is being fixed should
            // not silently change colour.
            //
            // Transparent pixels are EXCLUDED from the range. A fully transparent pixel's stored
            // colour is usually zero, so including it would peg the minimum at black and the
            // stretch would do nothing on any image with a transparent border.
            let mut low = [u8::MAX; 3];
            let mut high = [0u8; 3];
            let mut any = false;
            for pixel in original.chunks_exact(4) {
                if pixel[3] == 0 {
                    continue;
                }
                any = true;
                for channel in 0..3 {
                    low[channel] = low[channel].min(pixel[channel]);
                    high[channel] = high[channel].max(pixel[channel]);
                }
            }
            if any {
                let (mut lo, mut hi) = (low, high);
                if keep_colors {
                    let shared_low = lo[0].min(lo[1]).min(lo[2]);
                    let shared_high = hi[0].max(hi[1]).max(hi[2]);
                    lo = [shared_low; 3];
                    hi = [shared_high; 3];
                }
                for (output, input) in filtered.chunks_exact_mut(4).zip(original.chunks_exact(4)) {
                    for channel in 0..3 {
                        let span = f64::from(hi[channel]) - f64::from(lo[channel]);
                        // A flat channel has nothing to stretch. Dividing by its zero span would
                        // be a division by zero, and the alternative of forcing it to black or
                        // white would destroy a deliberately flat image.

                        if span <= 0.0 {
                            continue;
                        }
                        let scaled =
                            (f64::from(input[channel]) - f64::from(lo[channel])) * 255.0 / span;
                        output[channel] = scaled.round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::StretchContrastHsv => {
            // K.1. `gegl:stretch-contrast-hsv`.
            //
            // DERIVATION NOTE, same as `StretchContrast`: GEGL is a separate project this
            // repository does not vendor, so there is no upstream body. What the local tree
            // establishes is that the operation exists and is required (`app/sanity.c`), that it
            // applies with no dialog (`filters-actions.c`'s non-interactive array), that it has a
            // help id of its own, and its NAME — which is unusually informative here, because
            // "in HSV" says exactly which part differs from the RGB version.
            //
            // THE DECISION: saturation and value are stretched; HUE IS NOT. Hue is an angle, and
            // "stretch an angle to fill its range" is not a meaningful operation — a picture whose
            // hues happened to span 20°..60° would have them fanned out across the whole colour
            // wheel, turning a photograph of autumn leaves into a rainbow. Leaving hue alone is
            // also what makes this filter different from the RGB one rather than a slower spelling
            // of it: the RGB version cannot avoid moving hue when the channels differ, and this one
            // cannot move it at all.
            //
            // Converted through the module's existing `rgb_to_hsv`/`hsv_to_rgb` rather than a
            // second conversion written here, so this filter and the hue/saturation filters cannot
            // disagree about what a colour's saturation is.
            let mut low = (f32::MAX, f32::MAX);
            let mut high = (0.0f32, 0.0f32);
            let mut any = false;
            for pixel in original.chunks_exact(4) {
                // Transparent pixels excluded for the same reason as the RGB version: their stored
                // colour is usually zero, which would peg both minima and leave the filter inert
                // on any cut-out image.
                if pixel[3] == 0 {
                    continue;
                }
                any = true;
                let (_, s, v) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);
                low = (low.0.min(s), low.1.min(v));
                high = (high.0.max(s), high.1.max(v));
            }
            if any {
                let s_span = high.0 - low.0;
                let v_span = high.1 - low.1;
                for (output, input) in filtered.chunks_exact_mut(4).zip(original.chunks_exact(4)) {
                    let (h, s, v) = rgb_to_hsv(input[0], input[1], input[2]);
                    // A flat channel is left alone rather than forced to an extreme — a zero span
                    // would be a division by zero, and a deliberately flat image should survive.
                    let s = if s_span > 1e-6 {
                        (s - low.0) / s_span
                    } else {
                        s
                    };
                    let v = if v_span > 1e-6 {
                        (v - low.1) / v_span
                    } else {
                        v
                    };
                    let (r, g, b) = hsv_to_rgb(h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
                    output[0] = r;
                    output[1] = g;
                    output[2] = b;
                }
            }
        }
        Filter::ShadowsHighlights {
            shadows,
            highlights,
            radius,
            whitepoint,
            compress,
            shadows_ccorrect,
            highlights_ccorrect,
        } => {
            // K.1. `gegl:shadows-highlights`.
            //
            // DERIVATION. Unlike the two stretch filters, this one's CONTRACT is in vendored
            // source: `app/pdb/drawable-color-cmds.c` registers a deprecated wrapper whose
            // `g_param_spec_double` calls give every parameter's name, blurb and range —
            // `shadows` and `highlights` -100..100, `whitepoint` -10..10 ("Shift white point"),
            // `radius` 0.1..1500 ("Spatial extent"), `compress` 0..100, and `shadows-ccorrect`
            // and `highlights-ccorrect` 0..100. The operation BODY is still GEGL's and still not
            // vendored, so the arithmetic below follows from the parameter meanings, not read code.
            //
            // THREE of the seven parameters are exposed, and the other four are deliberately NOT.
            // Accepting a parameter and then ignoring it is worse than not offering it: the caller
            // has no way to tell, and a UI would grow four controls that do nothing. The absent
            // four are filed as their own backlog item.
            //
            // The defaults in that PDB registration are each equal to the parameter's MINIMUM
            // (shadows -100, radius 0.1), which is a `g_param_spec` artefact rather than a
            // considered default — a filter whose identity setting is "shadows fully down" would
            // be a strange thing to open. Zero is the neutral value here and does nothing, which
            // is the property a default should have.
            if !shadows.is_finite()
                || !highlights.is_finite()
                || !(-100.0..=100.0).contains(&shadows)
                || !(-100.0..=100.0).contains(&highlights)
                || !radius.is_finite()
                || !(0.1..=1500.0).contains(&radius)
                || !whitepoint.is_finite()
                || !(-10.0..=10.0).contains(&whitepoint)
                || !compress.is_finite()
                || !(0.0..=100.0).contains(&compress)
                || !shadows_ccorrect.is_finite()
                || !(0.0..=100.0).contains(&shadows_ccorrect)
                || !highlights_ccorrect.is_finite()
                || !(0.0..=100.0).contains(&highlights_ccorrect)
            {
                return Err(CoreError::InvalidFilterParameter);
            }

            // The mask is a BLURRED luminance plane: "spatial extent" is what makes this a local
            // operator rather than a curve. Without the blur, lifting shadows would raise every
            // dark pixel including the dark side of a sharp edge, which is what flattens an image
            // instead of opening it up.
            //
            // Built by filling an RGBA buffer with luminance in all four channels and reusing the
            // module's own box blur, which is O(1) per pixel through prefix sums. A direct window
            // sum over the neighbourhood reader would be O(radius²) per pixel, and the radius here
            // reaches 1500.
            let mut luma_plane = vec![0u8; original.len()];
            for (destination, source) in
                luma_plane.chunks_exact_mut(4).zip(original.chunks_exact(4))
            {
                let value = luminance(source);
                destination.fill(value);
            }
            let blur_radius = radius.round().clamp(1.0, 1500.0) as u32;
            let mask = box_blur_rgba(&luma_plane, width, height, blur_radius);

            let shadow_gain = f64::from(shadows) / 100.0;
            let highlight_gain = f64::from(highlights) / 100.0;
            for (index, (output, input)) in filtered
                .chunks_exact_mut(4)
                .zip(original.chunks_exact(4))
                .enumerate()
            {
                // The mask says how bright this pixel's NEIGHBOURHOOD is, which is what decides
                // whether it counts as shadow or highlight. Using the pixel's own value instead
                // would make the filter a tone curve and the radius meaningless.
                let local = f64::from(mask[index * 4]) / 255.0;
                // Two one-sided weights so a pixel in the midtones is barely touched by either
                // control, and the two controls cannot fight over the same pixel.
                let mut shadow_weight = (1.0 - local).clamp(0.0, 1.0);
                let mut highlight_weight = local.clamp(0.0, 1.0);
                // `compress` — upstream's blurb: "Compress the effect on shadows/highlights and
                // preserve midtones". Raising the weights to a power pushes them toward the
                // extremes: at 0 the exponent is 1 and nothing changes, which is why zero is the
                // neutral value and an older command without this field behaves exactly as before.
                //
                // A power rather than a narrowing window because the weights must stay continuous.
                // Clipping them to a band would put a visible edge in the image wherever the mask
                // crossed the band's boundary — a hard line through a gradient, which is the
                // artefact this kind of filter exists to avoid.
                if compress > 0.0 {
                    let exponent = 1.0 + f64::from(compress) / 100.0 * 4.0;
                    shadow_weight = shadow_weight.powf(exponent);
                    highlight_weight = highlight_weight.powf(exponent);
                }
                // The pixel's own saturation before the tone change, kept so `ccorrect` can put it
                // back. Measured as the channel spread over the maximum, which is HSV saturation —
                // the same definition the saturation filters use.
                let before_max = input[0].max(input[1]).max(input[2]);
                let before_min = input[0].min(input[1]).min(input[2]);
                let before_saturation = if before_max == 0 {
                    0.0
                } else {
                    f64::from(before_max - before_min) / f64::from(before_max)
                };
                let mut adjusted = [0.0f64; 3];
                for channel in 0..3 {
                    let value = f64::from(input[channel]) / 255.0;
                    // Positive shadows lift, negative deepen; the lift is applied toward white in
                    // proportion to how much headroom the pixel has, so a lifted shadow approaches
                    // white without ever passing it and no clamp is doing the work.
                    let lifted = if shadow_gain >= 0.0 {
                        value + (1.0 - value) * shadow_gain * shadow_weight
                    } else {
                        value + value * shadow_gain * shadow_weight
                    };
                    // Highlights the same way, mirrored: positive pulls DOWN, because the control
                    // is "recover highlights" and recovering means bringing detail back out of
                    // white. A positive highlights value that brightened would be the opposite of
                    // what the name promises.
                    let recovered = if highlight_gain >= 0.0 {
                        lifted - lifted * highlight_gain * highlight_weight
                    } else {
                        lifted - (1.0 - lifted) * highlight_gain * highlight_weight
                    };
                    adjusted[channel] = recovered;
                }

                // `ccorrect` — "Adjust saturation of shadows/highlights". The tone change above
                // compresses the differences BETWEEN channels, so it desaturates; this restores
                // the original saturation in proportion to how much of the region the control
                // governs. 100 restores fully (the default, because an unset parameter should not
                // wash the image out), 0 leaves it desaturated.
                let correction = (f64::from(shadows_ccorrect) / 100.0) * shadow_weight
                    + (f64::from(highlights_ccorrect) / 100.0) * highlight_weight;
                let total_weight = shadow_weight + highlight_weight;
                if total_weight > 0.0 {
                    let correction = correction / total_weight;
                    let after_max = adjusted[0].max(adjusted[1]).max(adjusted[2]);
                    let after_min = adjusted[0].min(adjusted[1]).min(adjusted[2]);
                    let after_saturation = if after_max <= 0.0 {
                        0.0
                    } else {
                        (after_max - after_min) / after_max
                    };
                    // Only ever pushes saturation back UP toward what it was. Allowing it to
                    // increase saturation past the original would make the control a vibrance
                    // slider, which is not what "adjust saturation of shadows" promises.
                    if after_saturation > 0.0 && before_saturation > after_saturation {
                        let target =
                            after_saturation + (before_saturation - after_saturation) * correction;
                        let scale = target / after_saturation;
                        for channel in &mut adjusted {
                            *channel = after_max - (after_max - *channel) * scale;
                        }
                    }
                }

                for channel in 0..3 {
                    // `whitepoint` — "Shift white point". A multiplicative scale about black, so
                    // it moves where white lands without bending the curve between. Applied LAST,
                    // because shifting the white point before the tone work would change which
                    // pixels count as highlights and the radius-driven mask would then describe an
                    // image that no longer exists.
                    let shifted = adjusted[channel] * (1.0 + f64::from(whitepoint) / 100.0);
                    output[channel] = (shifted * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::Sharpen { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            if amount > 0.0 {
                let blurred = box_blur_rgba(&original, width, height, 1);
                for (output, blur) in filtered.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                    for channel in 0..3 {
                        output[channel] = (f32::from(output[channel])
                            + amount * (f32::from(output[channel]) - f32::from(blur[channel])))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::MotionBlur {
            angle_degrees,
            distance,
        } => {
            if !angle_degrees.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            validate_radius(distance)?;
            let angle = f64::from(angle_degrees).to_radians();
            let (dx, dy) = (angle.cos(), angle.sin());
            let steps = distance as i64;
            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let (mut acc, mut n) = ([0.0f64; 4], 0.0f64);
                    // Sample the line centred on the pixel, from -distance/2 to +distance/2.
                    for s in -steps / 2..=steps / 2 {
                        let sx = (f64::from(x as i32) + dx * s as f64).round() as i64;
                        let sy = (f64::from(y as i32) + dy * s as f64).round() as i64;
                        if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
                            continue;
                        }
                        let o = ((sy as usize) * width as usize + sx as usize) * 4;
                        for c in 0..4 {
                            acc[c] += f64::from(original[o + c]);
                        }
                        n += 1.0;
                    }
                    if n <= 0.0 {
                        continue;
                    }
                    let o = ((y as usize) * width as usize + x as usize) * 4;
                    for c in 0..4 {
                        filtered[o + c] = (acc[c] / n).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::LensBlur { radius } => {
            validate_radius(radius)?;
            let r = radius as i64;
            let r2 = (r * r) as f64;
            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let (mut acc, mut n) = ([0.0f64; 4], 0.0f64);
                    for oy in -r..=r {
                        for ox in -r..=r {
                            if (ox * ox + oy * oy) as f64 > r2 {
                                continue; // outside the disc
                            }
                            let sx = x + ox;
                            let sy = y + oy;
                            if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
                                continue;
                            }
                            let o = ((sy as usize) * width as usize + sx as usize) * 4;
                            for c in 0..4 {
                                acc[c] += f64::from(original[o + c]);
                            }
                            n += 1.0;
                        }
                    }
                    if n <= 0.0 {
                        continue;
                    }
                    let o = ((y as usize) * width as usize + x as usize) * 4;
                    for c in 0..4 {
                        filtered[o + c] = (acc[c] / n).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::EdgeDetect { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            // Ported onto the shared reader (K.0-b). Clamp, which is what the closure this
            // replaces was doing; the luminance weights move with it so the two filters that
            // needed them cannot drift apart.
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let lum = |x: i64, y: i64| -> f64 { view.luminance(x, y) };
            for y in 0..h {
                for x in 0..w {
                    // Sobel gradients.
                    let gx = (lum(x + 1, y - 1) + 2.0 * lum(x + 1, y) + lum(x + 1, y + 1))
                        - (lum(x - 1, y - 1) + 2.0 * lum(x - 1, y) + lum(x - 1, y + 1));
                    let gy = (lum(x - 1, y + 1) + 2.0 * lum(x, y + 1) + lum(x + 1, y + 1))
                        - (lum(x - 1, y - 1) + 2.0 * lum(x, y - 1) + lum(x + 1, y - 1));
                    let mag = ((gx * gx + gy * gy).sqrt() * f64::from(amount))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let o = (y as usize * width as usize + x as usize) * 4;
                    filtered[o] = mag;
                    filtered[o + 1] = mag;
                    filtered[o + 2] = mag;
                    // Alpha kept from the source.
                }
            }
        }
        Filter::Emboss { angle_degrees } => {
            if !angle_degrees.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let angle = f64::from(angle_degrees).to_radians();
            let (lx, ly) = (angle.cos(), angle.sin());
            // Ported onto the shared reader (K.0-b). Clamp, which is what the closure this
            // replaces was doing; the luminance weights move with it so the two filters that
            // needed them cannot drift apart.
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let lum = |x: i64, y: i64| -> f64 { view.luminance(x, y) };
            for y in 0..h {
                for x in 0..w {
                    // Surface gradient dotted with the light direction, biased to mid-grey.
                    let gx = lum(x + 1, y) - lum(x - 1, y);
                    let gy = lum(x, y + 1) - lum(x, y - 1);
                    let shade = (128.0 + (gx * lx + gy * ly)).round().clamp(0.0, 255.0) as u8;
                    let o = (y as usize * width as usize + x as usize) * 4;
                    filtered[o] = shade;
                    filtered[o + 1] = shade;
                    filtered[o + 2] = shade;
                }
            }
        }
        Filter::Laplace => {
            let w = width as i64;
            let h = height as i64;
            // Ported onto the shared neighbourhood (K.0). Clamp, because that is what the
            // hand-rolled closure this replaces was doing — the port is behaviour-preserving and a
            // test asserts the pixels are byte-identical to the inline version it came from.
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            // 3x3 Laplacian: 8*centre − 8 neighbours. The kernel sums to zero, so `convolve`
            // returns the raw response rather than dividing by a zero weight.
            const LAPLACIAN: [f64; 9] = [
                -1.0, -1.0, -1.0, //
                -1.0, 8.0, -1.0, //
                -1.0, -1.0, -1.0,
            ];
            for y in 0..h {
                for x in 0..w {
                    let o = (y as usize * width as usize + x as usize) * 4;
                    for c in 0..3 {
                        let lap = view.convolve(x, y, &LAPLACIAN, 3, c);
                        filtered[o + c] = lap.abs().round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::Pixelize { block } => {
            validate_radius(block)?;
            let b = block as usize;
            let w = width as usize;
            let h = height as usize;
            let mut by = 0;
            while by < h {
                let mut bx = 0;
                while bx < w {
                    let (mut acc, mut n) = ([0u64; 4], 0u64);
                    for y in by..(by + b).min(h) {
                        for x in bx..(bx + b).min(w) {
                            let o = (y * w + x) * 4;
                            for c in 0..4 {
                                acc[c] += u64::from(original[o + c]);
                            }
                            n += 1;
                        }
                    }
                    if n > 0 {
                        let avg = [
                            (acc[0] / n) as u8,
                            (acc[1] / n) as u8,
                            (acc[2] / n) as u8,
                            (acc[3] / n) as u8,
                        ];
                        for y in by..(by + b).min(h) {
                            for x in bx..(bx + b).min(w) {
                                let o = (y * w + x) * 4;
                                filtered[o..o + 4].copy_from_slice(&avg);
                            }
                        }
                    }
                    bx += b;
                }
                by += b;
            }
        }
        Filter::Waves {
            amplitude,
            wavelength,
        } => {
            if !amplitude.is_finite() || !wavelength.is_finite() || wavelength.abs() < 1e-3 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let cx = f64::from(width) / 2.0;
            let cy = f64::from(height) / 2.0;
            let amp = f64::from(amplitude);
            let wl = f64::from(wavelength);
            for y in 0..height {
                for x in 0..width {
                    let dx = f64::from(x) + 0.5 - cx;
                    let dy = f64::from(y) + 0.5 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    // Displace radially by a sine of distance (concentric ripples from the centre).
                    let shift = amp * (dist / wl * std::f64::consts::TAU).sin();
                    let (nx, ny) = if dist > 1e-6 {
                        (
                            f64::from(x) + 0.5 + dx / dist * shift,
                            f64::from(y) + 0.5 + dy / dist * shift,
                        )
                    } else {
                        (f64::from(x) + 0.5, f64::from(y) + 0.5)
                    };
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        nx - 0.5,
                        ny - 0.5,
                        x,
                        y,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::Ripple {
            amplitude,
            wavelength,
            horizontal,
        } => {
            if !amplitude.is_finite() || !wavelength.is_finite() || wavelength.abs() < 1e-3 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let amp = f64::from(amplitude);
            let wl = f64::from(wavelength);
            for y in 0..height {
                for x in 0..width {
                    let (nx, ny) = if horizontal {
                        // Shift x by a sine of y.
                        let s = amp * (f64::from(y) / wl * std::f64::consts::TAU).sin();
                        (f64::from(x) + 0.5 + s, f64::from(y) + 0.5)
                    } else {
                        let s = amp * (f64::from(x) / wl * std::f64::consts::TAU).sin();
                        (f64::from(x) + 0.5, f64::from(y) + 0.5 + s)
                    };
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        nx - 0.5,
                        ny - 0.5,
                        x,
                        y,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::WhirlPinch {
            whirl_degrees,
            pinch,
        } => {
            if !whirl_degrees.is_finite() || !pinch.is_finite() || !(-1.0..=1.0).contains(&pinch) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let cx = f64::from(width) / 2.0;
            let cy = f64::from(height) / 2.0;
            let radius = cx.min(cy);
            let whirl = f64::from(whirl_degrees).to_radians();
            let pinch = f64::from(pinch);
            for y in 0..height {
                for x in 0..width {
                    let dx = f64::from(x) + 0.5 - cx;
                    let dy = f64::from(y) + 0.5 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= radius || dist < 1e-6 {
                        let o = (y as usize * width as usize + x as usize) * 4;
                        filtered[o..o + 4].copy_from_slice(&original[o..o + 4]);
                        continue;
                    }
                    let factor = 1.0 - dist / radius; // 1 at centre, 0 at rim
                    let angle = whirl * factor * factor;
                    // Pinch: pull the source toward (positive) or away from the centre.
                    let scale = factor.powf(-pinch);
                    let (s, c) = angle.sin_cos();
                    let sx = cx + (dx * c - dy * s) * scale;
                    let sy = cy + (dx * s + dy * c) * scale;
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        sx - 0.5,
                        sy - 0.5,
                        x,
                        y,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::LensDistortion { main_amount } => {
            if !main_amount.is_finite() || !(-100.0..=100.0).contains(&main_amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let cx = f64::from(width) / 2.0;
            let cy = f64::from(height) / 2.0;
            let norm = cx.hypot(cy);
            let k = f64::from(main_amount) / 100.0;
            for y in 0..height {
                for x in 0..width {
                    let dx = (f64::from(x) + 0.5 - cx) / norm;
                    let dy = (f64::from(y) + 0.5 - cy) / norm;
                    let r2 = dx * dx + dy * dy;
                    // Radial polynomial: barrel/pincushion by k*r^2.
                    let factor = 1.0 + k * r2;
                    let sx = cx + dx * norm * factor;
                    let sy = cy + dy * norm * factor;
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        sx - 0.5,
                        sy - 0.5,
                        x,
                        y,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::RgbNoise { amount, seed } => {
            if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let range = f64::from(amount) * 255.0;
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                for (c, channel) in pixel.iter_mut().take(3).enumerate() {
                    let r = noise_unit(seed, i as u32, c as u32);
                    let delta = (r * 2.0 - 1.0) * range;
                    *channel = (f64::from(*channel) + delta).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::HsvNoise {
            hue,
            saturation,
            value,
            seed,
        } => {
            if ![hue, saturation, value].iter().all(|v| v.is_finite()) {
                return Err(CoreError::InvalidFilterParameter);
            }
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                let (mut h, mut s, mut v) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);
                h = (h + (noise_unit(seed, i as u32, 0) * 2.0 - 1.0) as f32 * hue * 360.0)
                    .rem_euclid(360.0);
                s = (s + (noise_unit(seed, i as u32, 1) * 2.0 - 1.0) as f32 * saturation)
                    .clamp(0.0, 1.0);
                v = (v + (noise_unit(seed, i as u32, 2) * 2.0 - 1.0) as f32 * value)
                    .clamp(0.0, 1.0);
                let (r, g, b) = hsv_to_rgb(h, s, v);
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
            }
        }
        Filter::Hurl { amount, seed } => {
            if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                if noise_unit(seed, i as u32, 0) < f64::from(amount) {
                    pixel[0] = (noise_unit(seed, i as u32, 1) * 255.0) as u8;
                    pixel[1] = (noise_unit(seed, i as u32, 2) * 255.0) as u8;
                    pixel[2] = (noise_unit(seed, i as u32, 3) * 255.0) as u8;
                }
            }
        }
        Filter::Pick { amount, seed } => {
            if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let offsets: [(i64, i64); 8] = [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ];
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) as u32;
                    if noise_unit(seed, i, 0) >= f64::from(amount) {
                        continue;
                    }
                    let pick = (noise_unit(seed, i, 1) * 8.0) as usize % 8;
                    let (ox, oy) = offsets[pick];
                    // Ported onto the shared reader (K.0-b). `offset` hands back the resolved byte
                    // position so the copy stays byte-exact.
                    let so = view
                        .offset(x + ox, y + oy)
                        .expect("the clamp policy resolves every coordinate");
                    let d = (y as usize * width as usize + x as usize) * 4;
                    filtered[d..d + 3].copy_from_slice(&original[so..so + 3]);
                }
            }
        }
        Filter::Spread { amount, seed } => {
            let w = width as i64;
            let h = height as i64;
            let a = amount as i64;
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) as u32;
                    let ox = if a > 0 {
                        ((noise_unit(seed, i, 0) * (2 * a + 1) as f64) as i64) - a
                    } else {
                        0
                    };
                    let oy = if a > 0 {
                        ((noise_unit(seed, i, 1) * (2 * a + 1) as f64) as i64) - a
                    } else {
                        0
                    };
                    // Ported onto the shared reader (K.0-b). `offset` hands back the resolved byte
                    // position so the copy stays byte-exact.
                    let so = view
                        .offset(x + ox, y + oy)
                        .expect("the clamp policy resolves every coordinate");
                    let d = (y as usize * width as usize + x as usize) * 4;
                    filtered[d..d + 4].copy_from_slice(&original[so..so + 4]);
                }
            }
        }
        Filter::Checkerboard {
            size,
            color_a,
            color_b,
        } => {
            validate_radius(size)?;
            let s = size as usize;
            let w = width as usize;
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                let x = i % w;
                let y = i / w;
                let c = if ((x / s) + (y / s)).is_multiple_of(2) {
                    color_a
                } else {
                    color_b
                };
                pixel.copy_from_slice(&[c.r, c.g, c.b, c.a]);
            }
        }
        Filter::GradientMap { low, high } => {
            for pixel in filtered.chunks_exact_mut(4) {
                let t = f64::from(luminance(pixel)) / 255.0;
                pixel[0] = lerp_u8(low.r, high.r, t);
                pixel[1] = lerp_u8(low.g, high.g, t);
                pixel[2] = lerp_u8(low.b, high.b, t);
                // Alpha kept.
            }
        }
        Filter::Plasma { turbulence, seed } => {
            if !turbulence.is_finite() || turbulence <= 0.0 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as f64;
            let h = height as f64;
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                let x = (i % width as usize) as f64 / w;
                let y = (i / width as usize) as f64 / h;
                // Three colour channels from fractal value noise at different seeds.
                let scale = f64::from(turbulence) * 6.0;
                pixel[0] = (fractal_noise(x * scale, y * scale, seed, 4) * 255.0) as u8;
                pixel[1] = (fractal_noise(x * scale, y * scale, seed ^ 0x1111, 4) * 255.0) as u8;
                pixel[2] = (fractal_noise(x * scale, y * scale, seed ^ 0x2222, 4) * 255.0) as u8;
                pixel[3] = 255;
            }
        }
        Filter::SolidNoise { detail, seed } => {
            let octaves = detail.clamp(1, 8);
            let w = width as f64;
            let h = height as f64;
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                let x = (i % width as usize) as f64 / w;
                let y = (i / width as usize) as f64 / h;
                let v = (fractal_noise(x * 6.0, y * 6.0, seed, octaves) * 255.0) as u8;
                pixel.copy_from_slice(&[v, v, v, 255]);
            }
        }
        Filter::CellNoise { density, seed } => {
            let cells = density.clamp(1, 256) as f64;
            let w = width as f64;
            let h = height as f64;
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                let px = (i % width as usize) as f64 / w * cells;
                let py = (i / width as usize) as f64 / h * cells;
                // Worley: nearest feature point among the 3x3 surrounding cells.
                let (cx, cy) = (px.floor() as i64, py.floor() as i64);
                let mut nearest = f64::INFINITY;
                for oy in -1..=1 {
                    for ox in -1..=1 {
                        let gx = cx + ox;
                        let gy = cy + oy;
                        let idx = (gx.rem_euclid(1 << 16) as u32).wrapping_mul(73856093)
                            ^ (gy.rem_euclid(1 << 16) as u32).wrapping_mul(19349663);
                        let fx = gx as f64 + noise_unit(seed, idx, 0);
                        let fy = gy as f64 + noise_unit(seed, idx, 1);
                        let d = (px - fx) * (px - fx) + (py - fy) * (py - fy);
                        if d < nearest {
                            nearest = d;
                        }
                    }
                }
                let v = (nearest.sqrt().clamp(0.0, 1.0) * 255.0) as u8;
                pixel.copy_from_slice(&[v, v, v, 255]);
            }
        }
        Filter::ColorBalance {
            red,
            green,
            blue,
            red_shadows,
            green_shadows,
            blue_shadows,
            red_highlights,
            green_highlights,
            blue_highlights,
            preserve_luminosity,
        } => {
            // K.15. Ported from `app/operations/gimpoperationcolorbalance.c`, replacing our own
            // approximation. What was here before applied ONE shift with an ad-hoc weight
            // `1 - |2v - 1|` keyed on the CHANNEL's own value. Upstream keys on the pixel's HSL
            // LIGHTNESS and applies three masks whose constants are its own.
            //
            // Upstream's comment on those masks, which is why they are shaped as they are:
            //
            //     Apply masks to the corrections for shadows, midtones and highlights so that
            //     each correction affects only one range. Those masks look like this:
            //         ‾\___
            //         _/‾\_
            //         ___/‾
            //     with ramps of width a at x = b and x = 1 - b. The sum of these masks equals 1
            //     for x in 0..1, so applying the same correction in the shadows and in the
            //     midtones is equivalent to applying this correction on a virtual
            //     shadows_and_midtones range.
            //
            // The three constants are upstream's verbatim.
            const A: f64 = 0.25;
            const B: f64 = 0.333;
            const SCALE: f64 = 0.7;

            let all = [
                red,
                green,
                blue,
                red_shadows,
                green_shadows,
                blue_shadows,
                red_highlights,
                green_highlights,
                blue_highlights,
            ];
            if !all.iter().all(|v| v.is_finite()) {
                return Err(CoreError::InvalidFilterParameter);
            }

            // Our fields are -100..100 where upstream's are -1..1, so each is scaled on the way
            // in. Keeping our range is what lets commands saved before the two extra ranges
            // existed go on meaning what they meant.
            let shadows = [red_shadows, green_shadows, blue_shadows].map(|v| f64::from(v) / 100.0);
            let midtones = [red, green, blue].map(|v| f64::from(v) / 100.0);
            let highlights =
                [red_highlights, green_highlights, blue_highlights].map(|v| f64::from(v) / 100.0);

            for pixel in filtered.chunks_exact_mut(4) {
                // The masks are driven by HSL LIGHTNESS, not by each channel's own value: the
                // three ranges are ranges of the PIXEL's tone, so all three channels must be
                // weighted by the same number or a saturated colour would land in a different
                // range per channel.
                let (_, _, lightness) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
                let lightness = f64::from(lightness);

                let shadow_mask = ((lightness - B) / -A + 0.5).clamp(0.0, 1.0) * SCALE;
                let midtone_mask = ((lightness - B) / A + 0.5).clamp(0.0, 1.0)
                    * ((lightness + B - 1.0) / -A + 0.5).clamp(0.0, 1.0)
                    * SCALE;
                let highlight_mask = ((lightness + B - 1.0) / A + 0.5).clamp(0.0, 1.0) * SCALE;

                let original = [pixel[0], pixel[1], pixel[2]];
                for channel in 0..3 {
                    let value = f64::from(original[channel]) / 255.0
                        + shadows[channel] * shadow_mask
                        + midtones[channel] * midtone_mask
                        + highlights[channel] * highlight_mask;
                    pixel[channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                }

                if preserve_luminosity {
                    // Upstream converts the RESULT to HSL, copies the ORIGINAL lightness in, and
                    // converts back. Not a weight normalisation -- the shift has already
                    // happened and this undoes only its effect on lightness, keeping the hue and
                    // saturation it produced.
                    let (hue, saturation, _) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
                    let rgb = hsl_to_rgb(hue, saturation, lightness as f32);
                    pixel[0] = rgb[0];
                    pixel[1] = rgb[1];
                    pixel[2] = rgb[2];
                }
                // Alpha copied through, as upstream's `*(dest + 3) = *(src + 3)` does.
            }
        }
        Filter::ColorTemperature { amount } => {
            if !amount.is_finite() || !(-100.0..=100.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let a = f64::from(amount) / 100.0;
            // Warm: more red, less blue. Cool: the reverse.
            let rf = 1.0 + 0.3 * a;
            let bf = 1.0 - 0.3 * a;
            for pixel in filtered.chunks_exact_mut(4) {
                pixel[0] = (f64::from(pixel[0]) * rf).round().clamp(0.0, 255.0) as u8;
                pixel[2] = (f64::from(pixel[2]) * bf).round().clamp(0.0, 255.0) as u8;
            }
        }
        Filter::Exposure { stops } => {
            if !stops.is_finite() || !(-10.0..=10.0).contains(&stops) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let factor = 2.0_f64.powf(f64::from(stops));
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in pixel[..3].iter_mut() {
                    *channel = (f64::from(*channel) * factor).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::HueChroma {
            hue_degrees,
            chroma,
        } => {
            if !hue_degrees.is_finite()
                || !chroma.is_finite()
                || !(-100.0..=100.0).contains(&chroma)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let chroma_scale = 1.0 + chroma / 100.0;
            for pixel in filtered.chunks_exact_mut(4) {
                let (mut h, mut s, v) = rgb_to_hsv(pixel[0], pixel[1], pixel[2]);
                h = (h + hue_degrees).rem_euclid(360.0);
                s = (s * chroma_scale).clamp(0.0, 1.0);
                let (r, g, b) = hsv_to_rgb(h, s, v);
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = b;
            }
        }
        Filter::Saturation { scale } => {
            if !scale.is_finite() || !(0.0..=4.0).contains(&scale) {
                return Err(CoreError::InvalidFilterParameter);
            }
            for pixel in filtered.chunks_exact_mut(4) {
                let grey = f64::from(luminance(pixel));
                for channel in pixel[..3].iter_mut() {
                    let v = f64::from(*channel);
                    *channel = (grey + (v - grey) * f64::from(scale))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::Dither { levels } => {
            if levels < 2 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as usize;
            let h = height as usize;
            let step = 255.0 / f64::from(levels - 1);
            // Floyd-Steinberg error diffusion per channel, on a working float buffer.
            let mut buf: Vec<f64> = original.iter().map(|&b| f64::from(b)).collect();
            for y in 0..h {
                for x in 0..w {
                    let i = (y * w + x) * 4;
                    for c in 0..3 {
                        let old = buf[i + c];
                        let q = (old / step).round() * step;
                        let err = old - q;
                        buf[i + c] = q;
                        filtered[i + c] = q.round().clamp(0.0, 255.0) as u8;
                        // Distribute the error to neighbours (7/16, 3/16, 5/16, 1/16).
                        let mut spread = |nx: usize, ny: usize, f: f64| {
                            if nx < w && ny < h {
                                buf[(ny * w + nx) * 4 + c] += err * f;
                            }
                        };
                        if x + 1 < w {
                            spread(x + 1, y, 7.0 / 16.0);
                        }
                        if y + 1 < h {
                            if x > 0 {
                                spread(x - 1, y + 1, 3.0 / 16.0);
                            }
                            spread(x, y + 1, 5.0 / 16.0);
                            spread(x + 1, y + 1, 1.0 / 16.0);
                        }
                    }
                }
            }
        }
        Filter::Oilify { radius } => {
            validate_radius(radius)?;
            let r = radius as i64;
            let w = width as i64;
            let h = height as i64;
            const BINS: usize = 16;
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            for y in 0..h {
                for x in 0..w {
                    // Histogram of luma bins; keep the summed colour of the most-populated bin.
                    let mut counts = [0u32; BINS];
                    let mut sums = [[0u64; 3]; BINS];
                    for oy in -r..=r {
                        for ox in -r..=r {
                            // K.0-b: the edge policy comes from the shared reader; the luminance
                            // helper is left alone, because it is the module's own u8 version and
                            // substituting a different one would not be a behaviour-preserving port.
                            let o = view
                                .offset(x + ox, y + oy)
                                .expect("the clamp policy resolves every coordinate");
                            let lum = luminance(&original[o..o + 4]) as usize * BINS / 256;
                            let bin = lum.min(BINS - 1);
                            counts[bin] += 1;
                            for c in 0..3 {
                                sums[bin][c] += u64::from(original[o + c]);
                            }
                        }
                    }
                    let best = (0..BINS).max_by_key(|&b| counts[b]).unwrap_or(0);
                    let n = counts[best].max(1) as u64;
                    let d = (y as usize * width as usize + x as usize) * 4;
                    for c in 0..3 {
                        filtered[d + c] = (sums[best][c] / n) as u8;
                    }
                }
            }
        }
        Filter::Cartoon { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            // Darken where the pixel is much darker than its blurred neighbourhood (edges).
            let blurred = box_blur_rgba(&original, width, height, 3);
            for (out, blur) in filtered.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                let lum = f64::from(luminance(out));
                let blum = f64::from(luminance(blur)).max(1.0);
                let ratio = lum / blum;
                // ratio < 1 means darker than surroundings -> an edge; darken proportionally.
                let darken = if ratio < 1.0 {
                    1.0 - (1.0 - ratio) * f64::from(amount)
                } else {
                    1.0
                }
                .clamp(0.0, 1.0);
                for channel in out[..3].iter_mut() {
                    *channel = (f64::from(*channel) * darken).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::SoftGlow { radius, amount } => {
            validate_radius(radius)?;
            if !amount.is_finite() || !(0.0..=1.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let blurred = box_blur_rgba(&original, width, height, radius);
            let a = f64::from(amount);
            // Screen the blurred (brightened) copy over the original: 1-(1-a)(1-b).
            for (out, blur) in filtered.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                for c in 0..3 {
                    let base = f64::from(out[c]) / 255.0;
                    let glow = (f64::from(blur[c]) / 255.0) * a;
                    let screened = 1.0 - (1.0 - base) * (1.0 - glow);
                    out[c] = (screened * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::Photocopy { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            // Local brightness vs a blurred mean -> hard black/white sketch.
            let blurred = box_blur_rgba(&original, width, height, 5);
            for (out, blur) in filtered.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                let lum = f64::from(luminance(out));
                let blum = f64::from(luminance(blur)).max(1.0);
                let ratio = (lum / blum).powf(f64::from(amount).max(0.1));
                let v = (ratio * 255.0).round().clamp(0.0, 255.0) as u8;
                out[0] = v;
                out[1] = v;
                out[2] = v;
            }
        }
        Filter::ApplyCanvas { depth } => {
            if !depth.is_finite() || !(0.0..=1.0).contains(&depth) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as usize;
            let d = f64::from(depth);
            for (i, pixel) in filtered.chunks_exact_mut(4).enumerate() {
                let x = i % w;
                let y = i / w;
                // A woven pattern: two offset sine ridges give a +/- shade.
                let weave = ((x as f64 / 4.0).sin() + (y as f64 / 4.0).sin()) * 0.5;
                let shade = 1.0 + weave * d * 0.5;
                for channel in pixel[..3].iter_mut() {
                    *channel = (f64::from(*channel) * shade).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::Cubism { tile, seed } => {
            validate_radius(tile)?;
            let t = tile as usize;
            let w = width as usize;
            let h = height as usize;
            // Each tile samples one jittered colour and paints its whole cell with it.
            let mut ty = 0;
            while ty < h {
                let mut tx = 0;
                while tx < w {
                    let idx = (ty / t * (w / t.max(1) + 1) + tx / t) as u32;
                    let jx = (noise_unit(seed, idx, 0) * t as f64) as usize;
                    let jy = (noise_unit(seed, idx, 1) * t as f64) as usize;
                    let sx = (tx + jx).min(w - 1);
                    let sy = (ty + jy).min(h - 1);
                    let so = (sy * w + sx) * 4;
                    let colour = [
                        original[so],
                        original[so + 1],
                        original[so + 2],
                        original[so + 3],
                    ];
                    for y in ty..(ty + t).min(h) {
                        for x in tx..(tx + t).min(w) {
                            let o = (y * w + x) * 4;
                            filtered[o..o + 4].copy_from_slice(&colour);
                        }
                    }
                    tx += t;
                }
                ty += t;
            }
        }
        Filter::BumpMap {
            azimuth_degrees,
            elevation_degrees,
            depth,
            map: _,
        } => {
            if ![azimuth_degrees, elevation_degrees, depth]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let az = f64::from(azimuth_degrees).to_radians();
            let el = f64::from(elevation_degrees).to_radians();
            // Light vector.
            let lx = az.cos() * el.cos();
            let ly = az.sin() * el.cos();
            let lz = el.sin();
            let d = f64::from(depth);
            let view = crate::neighbourhood::Neighbourhood::new(
                map,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let height_at = |x: i64, y: i64| -> f64 {
                let o = view
                    .offset(x, y)
                    .expect("the clamp policy resolves every coordinate");
                f64::from(luminance(&map[o..][..4])) / 255.0
            };
            for y in 0..h {
                for x in 0..w {
                    // Surface normal from the height gradient.
                    let gx = (height_at(x + 1, y) - height_at(x - 1, y)) * d;
                    let gy = (height_at(x, y + 1) - height_at(x, y - 1)) * d;
                    let len = (gx * gx + gy * gy + 1.0).sqrt();
                    let (nx, ny, nz) = (-gx / len, -gy / len, 1.0 / len);
                    let shade = (nx * lx + ny * ly + nz * lz).clamp(0.0, 1.0);
                    let o = (y as usize * width as usize + x as usize) * 4;
                    for c in 0..3 {
                        filtered[o + c] = (f64::from(original[o + c]) * shade)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::Displace { amount, map: _ } => {
            if !amount.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let a = f64::from(amount);
            let view = crate::neighbourhood::Neighbourhood::new(
                map,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let lum = |x: i64, y: i64| -> f64 {
                let o = view
                    .offset(x, y)
                    .expect("the clamp policy resolves every coordinate");
                f64::from(luminance(&map[o..][..4])) / 255.0
            };
            for y in 0..h {
                for x in 0..w {
                    let gx = lum(x + 1, y) - lum(x - 1, y);
                    let gy = lum(x, y + 1) - lum(x, y - 1);
                    let sx = f64::from(x as i32) + 0.5 + gx * a;
                    let sy = f64::from(y as i32) + 0.5 + gy * a;
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        sx - 0.5,
                        sy - 0.5,
                        x as u32,
                        y as u32,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::FractalTrace {
            depth,
            scale,
            map: _,
        } => {
            if !scale.is_finite() || scale.abs() < 1e-3 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let iters = depth.clamp(1, 32);
            let w = f64::from(width);
            let h = f64::from(height);
            let s = f64::from(scale);
            for y in 0..height {
                for x in 0..width {
                    // Map the pixel to the complex plane, iterate z = z^2 + c once per depth, map back.
                    let mut zx = (f64::from(x) / w * 2.0 - 1.0) * s;
                    let mut zy = (f64::from(y) / h * 2.0 - 1.0) * s;
                    let cx = zx;
                    let cy = zy;
                    for _ in 0..iters {
                        let nx = zx * zx - zy * zy + cx;
                        let ny = 2.0 * zx * zy + cy;
                        zx = nx;
                        zy = ny;
                        if zx * zx + zy * zy > 4.0 {
                            break;
                        }
                    }
                    // Fold the escaped coordinate back into the image via fract.
                    let sx = ((zx / s + 1.0) * 0.5).rem_euclid(1.0) * w;
                    let sy = ((zy / s + 1.0) * 0.5).rem_euclid(1.0) * h;
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        sx - 0.5,
                        sy - 0.5,
                        x,
                        y,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::WarpMap {
            amount,
            steps,
            map: _,
        } => {
            if !amount.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            let n = steps.clamp(1, 32);
            let a = f64::from(amount);
            // Iteratively trace back along the luma gradient from each destination pixel.
            let view = crate::neighbourhood::Neighbourhood::new(
                map,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let lum = |x: f64, y: f64| -> f64 {
                // K.0-b. The ROUNDING stays here rather than moving into the reader: this filter
                // traces a continuous path and chooses to sample the nearest pixel, which is its
                // decision and not an edge policy. A reader that rounded for its callers would
                // make the next filter's choice of bilinear sampling impossible to express.
                let o = view
                    .offset(x.round() as i64, y.round() as i64)
                    .expect("the clamp policy resolves every coordinate");
                f64::from(luminance(&map[o..][..4])) / 255.0
            };
            for y in 0..height {
                for x in 0..width {
                    let mut px = f64::from(x) + 0.5;
                    let mut py = f64::from(y) + 0.5;
                    for _ in 0..n {
                        let gx = lum(px + 1.0, py) - lum(px - 1.0, py);
                        let gy = lum(px, py + 1.0) - lum(px, py - 1.0);
                        px += gx * a / f64::from(n);
                        py += gy * a / f64::from(n);
                    }
                    sample_bilinear(
                        &original,
                        width,
                        height,
                        px - 0.5,
                        py - 0.5,
                        x,
                        y,
                        &mut filtered,
                    );
                }
            }
        }
        Filter::Halftone { cell } => {
            validate_radius(cell)?;
            let c = cell as usize;
            let w = width as usize;
            let h = height as usize;
            // For each cell, the mean darkness sets a dot radius; paint black within that radius.
            let mut cy0 = 0;
            while cy0 < h {
                let mut cx0 = 0;
                while cx0 < w {
                    let (mut sum, mut n) = (0u64, 0u64);
                    for y in cy0..(cy0 + c).min(h) {
                        for x in cx0..(cx0 + c).min(w) {
                            sum += u64::from(luminance(&original[(y * w + x) * 4..][..4]));
                            n += 1;
                        }
                    }
                    let mean = if n > 0 {
                        sum as f64 / n as f64 / 255.0
                    } else {
                        1.0
                    };
                    // Darker cell -> bigger dot. Radius up to half the cell diagonal.
                    let max_r = c as f64 * 0.6;
                    let dot_r = (1.0 - mean).sqrt() * max_r;
                    let ccx = cx0 as f64 + c as f64 / 2.0;
                    let ccy = cy0 as f64 + c as f64 / 2.0;
                    for y in cy0..(cy0 + c).min(h) {
                        for x in cx0..(cx0 + c).min(w) {
                            let d = ((x as f64 + 0.5 - ccx).powi(2)
                                + (y as f64 + 0.5 - ccy).powi(2))
                            .sqrt();
                            let v = if d <= dot_r { 0u8 } else { 255u8 };
                            let o = (y * w + x) * 4;
                            filtered[o] = v;
                            filtered[o + 1] = v;
                            filtered[o + 2] = v;
                        }
                    }
                    cx0 += c;
                }
                cy0 += c;
            }
        }
        Filter::PhongBump {
            azimuth_degrees,
            elevation_degrees,
            depth,
            shininess,
        } => {
            if ![azimuth_degrees, elevation_degrees, depth, shininess]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let az = f64::from(azimuth_degrees).to_radians();
            let el = f64::from(elevation_degrees).to_radians();
            let (lx, ly, lz) = (az.cos() * el.cos(), az.sin() * el.cos(), el.sin());
            let d = f64::from(depth);
            let shin = f64::from(shininess).max(1.0);
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let height_at = |x: i64, y: i64| -> f64 {
                let o = view
                    .offset(x, y)
                    .expect("the clamp policy resolves every coordinate");
                f64::from(luminance(&original[o..][..4])) / 255.0
            };
            for y in 0..h {
                for x in 0..w {
                    let gx = (height_at(x + 1, y) - height_at(x - 1, y)) * d;
                    let gy = (height_at(x, y + 1) - height_at(x, y - 1)) * d;
                    let len = (gx * gx + gy * gy + 1.0).sqrt();
                    let (nx, ny, nz) = (-gx / len, -gy / len, 1.0 / len);
                    let diffuse = (nx * lx + ny * ly + nz * lz).max(0.0);
                    // Reflect light about the normal; specular is (R·V)^shininess with V = +z.
                    let dot = nx * lx + ny * ly + nz * lz;
                    let rz = 2.0 * dot * nz - lz;
                    let spec = rz.max(0.0).powf(shin);
                    let o = (y as usize * width as usize + x as usize) * 4;
                    for ch in 0..3 {
                        let base = f64::from(original[o + ch]) * diffuse;
                        filtered[o + ch] = (base + spec * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::Palettize { levels } => {
            if levels < 2 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let step = 255.0 / f64::from(levels - 1);
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in pixel[..3].iter_mut() {
                    *channel = ((f64::from(*channel) / step).round() * step)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::NormalMap { strength } => {
            if !strength.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let s = f64::from(strength);
            let view = crate::neighbourhood::Neighbourhood::new(
                &original,
                width,
                height,
                crate::neighbourhood::EdgePolicy::Clamp,
            );
            let height_at = |x: i64, y: i64| -> f64 {
                let o = view
                    .offset(x, y)
                    .expect("the clamp policy resolves every coordinate");
                f64::from(luminance(&original[o..][..4])) / 255.0
            };
            for y in 0..h {
                for x in 0..w {
                    let gx = (height_at(x + 1, y) - height_at(x - 1, y)) * s;
                    let gy = (height_at(x, y + 1) - height_at(x, y - 1)) * s;
                    // Tangent-space normal (-gx, -gy, 1) normalised, packed to 0..255.
                    let len = (gx * gx + gy * gy + 1.0).sqrt();
                    let nx = -gx / len;
                    let ny = -gy / len;
                    let nz = 1.0 / len;
                    let o = (y as usize * width as usize + x as usize) * 4;
                    filtered[o] = ((nx * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                    filtered[o + 1] = ((ny * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                    filtered[o + 2] = ((nz * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::ChannelMixer {
            matrix,
            offset,
            preserve_luminosity,
        } => {
            // Each output row's three weights are normalised to sum to 1, so a change of BALANCE
            // between inputs is not also a change of that channel's brightness. Per-row rather
            // than over all nine, because upstream's property GUI groups the gains into three
            // frames -- one per output channel -- with the checkbox outside them all.
            //
            // A row summing to zero is left alone: (1, 0, -1) is a legitimate
            // difference-of-channels row, normalising it is impossible, and refusing a setting the
            // user is entitled to would be worse than passing it through. Same rule as MonoMixer.
            let matrix = if preserve_luminosity {
                let mut normalised = matrix;
                for row in normalised.chunks_exact_mut(3) {
                    let sum = row[0] + row[1] + row[2];
                    if sum.abs() > f32::EPSILON {
                        for weight in row {
                            *weight /= sum;
                        }
                    }
                }
                normalised
            } else {
                matrix
            };

            if !matrix.iter().chain(offset.iter()).all(|v| v.is_finite()) {
                return Err(CoreError::InvalidFilterParameter);
            }
            for pixel in filtered.chunks_exact_mut(4) {
                let r = f64::from(pixel[0]);
                let g = f64::from(pixel[1]);
                let b = f64::from(pixel[2]);
                let out_r = matrix[0] as f64 * r
                    + matrix[1] as f64 * g
                    + matrix[2] as f64 * b
                    + offset[0] as f64 * 255.0;
                let out_g = matrix[3] as f64 * r
                    + matrix[4] as f64 * g
                    + matrix[5] as f64 * b
                    + offset[1] as f64 * 255.0;
                let out_b = matrix[6] as f64 * r
                    + matrix[7] as f64 * g
                    + matrix[8] as f64 * b
                    + offset[2] as f64 * 255.0;
                pixel[0] = out_r.round().clamp(0.0, 255.0) as u8;
                pixel[1] = out_g.round().clamp(0.0, 255.0) as u8;
                pixel[2] = out_b.round().clamp(0.0, 255.0) as u8;
            }
        }
        Filter::LabAdjust { lightness, chroma } => {
            if !lightness.is_finite()
                || !chroma.is_finite()
                || !(-100.0..=100.0).contains(&lightness)
                || !(0.0..=4.0).contains(&chroma)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let dl = f64::from(lightness);
            let cs = f64::from(chroma);
            for pixel in filtered.chunks_exact_mut(4) {
                let (mut l, mut a, mut b) =
                    crate::color::srgb8_to_lab(pixel[0], pixel[1], pixel[2]);
                l = (l + dl).clamp(0.0, 100.0);
                a *= cs;
                b *= cs;
                let (r, g, bl) = crate::color::lab_to_srgb8(l, a, b);
                pixel[0] = r;
                pixel[1] = g;
                pixel[2] = bl;
            }
        }
    }

    blend_selection(document, &original, &mut filtered);
    document.replace_active_pixels(filtered)
}

/// Runs a precision-native filter (J.1b) on the active layer at the document's own sample width.
///
/// Samples are decoded to unit floats, the filter works there, and the result is encoded back at
/// the SAME precision. Float is the working form rather than the widest integer because it is the
/// only one that needs no per-precision arithmetic: one implementation, and the encode step is what
/// knows about widths.
///
/// At 8-bit this is the same answer the old byte arm gave — `invert_is_identical_at_every_precision`
/// pins that, because a migration whose first step changes 8-bit output would have to be rolled
/// back rather than continued.
fn apply_precision_native_filter(document: &mut Document, filter: &Filter) -> Result<()> {
    let precision = document.precision();
    let width = document.width();
    document.prepare_active_raster_edit()?;
    let stored = document.active_raster_pixels()?.to_vec();
    let samples = stored.len() / precision.bytes_per_sample();
    let original: Vec<f32> = (0..samples)
        .map(|index| precision.read_sample(&stored, index))
        .collect();
    let mut filtered = original.clone();

    match *filter {
        Filter::Invert => {
            // Unit complement, and alpha is left alone: inverting coverage would turn a
            // transparent area opaque, which is not what inverting a colour means.
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    *channel = 1.0 - *channel;
                }
            }
        }
        Filter::RgbClip {
            clip_low,
            clip_high,
            low_limit,
            high_limit,
        } => {
            // K.1. `gegl:rgb-clip`. Interactive upstream (line 583 of `filters-actions.c`, past
            // the dialog boundary), so parameterised; no `!gray` guard, so valid on greyscale.
            //
            // DERIVATION. The body is GEGL's and not vendored. What makes this filter's PLACEMENT
            // derivable rather than guessed is the storage: at 8- and 16-bit an encoding cannot
            // hold a value outside 0..1 at all, so clipping is inert there by construction. At F32
            // `Precision::write_sample` stores a raw `f32` with no clamping, so out-of-range
            // samples genuinely exist — which is the only condition under which this operation
            // means anything.
            //
            // That is why it is precision-NATIVE. Routing it through the 8-bit path would narrow
            // the buffer first, and the narrowing itself clips; the filter would appear to work
            // while the conversion had already done the job and destroyed everything above the
            // limit, including on the 16-bit documents where nothing needed clipping.
            //
            // Only the colour channels. Alpha is coverage and is always in range by construction;
            // clipping it would be either a no-op or, at F32, a silent change to a layer's shape.
            if !low_limit.is_finite() || !high_limit.is_finite() || low_limit > high_limit {
                return Err(CoreError::InvalidFilterParameter);
            }
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    if clip_low && *channel < low_limit {
                        *channel = low_limit;
                    }
                    if clip_high && *channel > high_limit {
                        *channel = high_limit;
                    }
                }
            }
        }
        Filter::InvertLinear => {
            // K.1, last of the group. `gegl:invert-linear`.
            //
            // Parameterless, settled by reading the vendored wrapper rather than assuming:
            // `gimp_gegl_apply_invert_linear` at `gimp-gegl-apply-operation.c:674` builds its node
            // with `gegl_node_new_child(NULL, "operation", "gegl:invert-linear", NULL)` and no
            // properties. Its sibling `gimp_gegl_apply_invert_gamma` sits directly above it, which
            // is what establishes the two as a deliberate pair rather than one filter with a flag.
            //
            // `Filter::Invert` is the gamma half: it complements the STORED value, which is
            // sRGB-encoded. This is the same complement in LINEAR light — decode, complement,
            // re-encode. The two therefore agree only at 0, 1 and the single value whose encoded
            // and linear complements coincide; everywhere else they differ, and a mid-tone is where
            // the gap is widest.
            //
            // Precision is the component TYPE only and carries no opinion about encoding — stored
            // samples are sRGB-encoded at U8, U16 and F32 alike — so decode/complement/re-encode is
            // correct at every precision, which is why this can be precision-native like its
            // sibling.
            //
            // Out-of-range F32 samples (which rgb-clip established genuinely exist) survive this
            // without producing NaN: both transfer functions take their LINEAR branch below the
            // breakpoint, so a negative input stays negative rather than reaching `powf` with a
            // negative base. The complement of an out-of-range value is another out-of-range value,
            // which is correct — clipping it is rgb-clip's job, not this filter's.
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    let linear = crate::color::srgb_to_linear(f64::from(*channel));
                    *channel = crate::color::linear_to_srgb(1.0 - linear) as f32;
                }
                // Alpha left alone, as in every other invert.
            }
        }
        // Unreachable while `is_precision_native` and this match agree, and
        // `native_filters_all_have_an_implementation` is the test that keeps them agreeing. A
        // filter added to the list without an arm must not silently do nothing.
        _ => return Err(CoreError::FilterPrecisionUnsupported(filter.name())),
    }

    blend_selection_unit(document, width, &original, &mut filtered);

    let mut out = vec![0u8; stored.len()];
    for (index, value) in filtered.iter().enumerate() {
        precision.write_sample(&mut out, index, *value);
    }
    document.replace_active_pixels(out)
}

/// Selection-weighted blend of a filtered unit buffer back over its original.
///
/// The same rule as the byte path: outside the selection the original wins, at partial coverage the
/// two are mixed. Written against unit floats so a precision-native filter needs no byte round trip
/// just to honour a selection.
fn blend_selection_unit(document: &Document, width: u32, original: &[f32], filtered: &mut [f32]) {
    for (index, (output, input)) in filtered
        .chunks_exact_mut(4)
        .zip(original.chunks_exact(4))
        .enumerate()
    {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let coverage = f32::from(document.selection().coverage(x, y)) / 255.0;
        if coverage < 1.0 {
            for channel in 0..4 {
                output[channel] = output[channel] * coverage + input[channel] * (1.0 - coverage);
            }
        }
    }
}

/// Linear interpolation between two bytes at `t` in 0..1.
fn lerp_u8(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + (f64::from(b) - f64::from(a)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Smooth value noise at `(x, y)` in `[0, 1)`: hash the four surrounding lattice points with
/// `noise_unit` and smootherstep-interpolate between them.
fn value_noise(x: f64, y: f64, seed: u32) -> f64 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = x - x0;
    let fy = y - y0;
    let lattice = |ix: f64, iy: f64| -> f64 {
        // Fold to integers FIRST, then wrap. `rem_euclid` on the float would keep a fraction, and two
        // lattice points a whole period apart must hash identically or the noise seams.
        let fold = |v: f64| (v as i64).rem_euclid(1 << 16) as u32;
        let idx = fold(ix).wrapping_mul(0x1F1F_1F1F) ^ fold(iy).wrapping_mul(0x9E37_79B9);
        noise_unit(seed, idx, 0)
    };
    let n00 = lattice(x0, y0);
    let n10 = lattice(x0 + 1.0, y0);
    let n01 = lattice(x0, y0 + 1.0);
    let n11 = lattice(x0 + 1.0, y0 + 1.0);
    // Smootherstep weights.
    let sx = fx * fx * fx * (fx * (fx * 6.0 - 15.0) + 10.0);
    let sy = fy * fy * fy * (fy * (fy * 6.0 - 15.0) + 10.0);
    let top = n00 + (n10 - n00) * sx;
    let bottom = n01 + (n11 - n01) * sx;
    top + (bottom - top) * sy
}

/// Fractal (fBm) value noise: `octaves` layers of `value_noise` at doubling frequency and halving
/// amplitude, normalised to 0..1.
fn fractal_noise(x: f64, y: f64, seed: u32, octaves: u32) -> f64 {
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut freq = 1.0;
    let mut total = 0.0;
    for o in 0..octaves {
        sum += value_noise(x * freq, y * freq, seed.wrapping_add(o * 101)) * amp;
        total += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    if total > 0.0 { sum / total } else { 0.0 }
}
/// channel/stream index). A small integer hash (splitmix-style finaliser) — no global RNG state, so a
/// given (seed, index, stream) always yields the same number and the filter is reproducible.
fn noise_unit(seed: u32, index: u32, stream: u32) -> f64 {
    let mut z = seed
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(index.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(stream.wrapping_mul(0xC2B2_AE35))
        .wrapping_add(0x1656_67B1);
    z ^= z >> 16;
    z = z.wrapping_mul(0x7FEB_352D);
    z ^= z >> 15;
    z = z.wrapping_mul(0x846C_A68B);
    z ^= z >> 16;
    f64::from(z) / f64::from(u32::MAX)
}

/// RGB (0..255) to HSV with hue in degrees 0..360 and saturation/value in 0..1.
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = f32::from(r) / 255.0;
    let gf = f32::from(g) / 255.0;
    let bf = f32::from(b) / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;
    let hue = if delta < 1e-6 {
        0.0
    } else if (max - rf).abs() < 1e-6 {
        60.0 * (((gf - bf) / delta) % 6.0)
    } else if (max - gf).abs() < 1e-6 {
        60.0 * (((bf - rf) / delta) + 2.0)
    } else {
        60.0 * (((rf - gf) / delta) + 4.0)
    };
    let hue = hue.rem_euclid(360.0);
    let sat = if max < 1e-6 { 0.0 } else { delta / max };
    (hue, sat, max)
}

/// HSV (hue degrees, sat/value 0..1) back to RGB (0..255).
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = h.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (
        ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}
/// dy)`. Out-of-bounds reads clamp to the edge, so the warp filters do not tear at the borders.
#[allow(clippy::too_many_arguments)]
fn sample_bilinear(
    src: &[u8],
    width: u32,
    height: u32,
    fx: f64,
    fy: f64,
    dx: u32,
    dy: u32,
    out: &mut [u8],
) {
    let x0 = fx.floor() as i64;
    let y0 = fy.floor() as i64;
    let tx = fx - x0 as f64;
    let ty = fy - y0 as f64;
    // K.0-b. Clamp, which is what bilinear sampling needs at a border: transparent black would
    // make every edge pixel fade toward nothing as the sample moved off the image, which is a
    // transform artefact rather than anything in the picture.
    let view = crate::neighbourhood::Neighbourhood::new(
        src,
        width,
        height,
        crate::neighbourhood::EdgePolicy::Clamp,
    );
    let at = |x: i64, y: i64, c: usize| -> f64 { view.channel_or_zero(x, y, c) };
    let o = (dy as usize * width as usize + dx as usize) * 4;
    for c in 0..4 {
        let top = at(x0, y0, c) * (1.0 - tx) + at(x0 + 1, y0, c) * tx;
        let bottom = at(x0, y0 + 1, c) * (1.0 - tx) + at(x0 + 1, y0 + 1, c) * tx;
        out[o + c] = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
    }
}

fn validate_radius(radius: u32) -> Result<()> {
    if radius == 0 || radius > MAX_FILTER_RADIUS {
        Err(CoreError::InvalidFilterParameter)
    } else {
        Ok(())
    }
}

fn luminance(pixel: &[u8]) -> u8 {
    (0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2]))
        .round()
        .clamp(0.0, 255.0) as u8
}

fn premultiply(input: &[u8]) -> Vec<u8> {
    let mut output = input.to_vec();
    for pixel in output.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[0..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    output
}

fn unpremultiply(mut input: Vec<u8>) -> Vec<u8> {
    for pixel in input.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel.fill(0);
        } else {
            for channel in &mut pixel[0..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    input
}

fn box_blur_rgba(input: &[u8], width: u32, height: u32, radius: u32) -> Vec<u8> {
    let premultiplied = premultiply(input);
    let horizontal = box_blur_rgba_pass(&premultiplied, width, height, radius, true);
    unpremultiply(box_blur_rgba_pass(
        &horizontal,
        width,
        height,
        radius,
        false,
    ))
}

fn box_blur_rgba_pass(
    input: &[u8],
    width: u32,
    height: u32,
    radius: u32,
    horizontal: bool,
) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    let lines = if horizontal { height } else { width };
    let line_len = if horizontal { width } else { height };
    for line in 0..lines {
        let mut prefix = vec![[0_u64; 4]; line_len as usize + 1];
        for position in 0..line_len {
            let index = if horizontal {
                (line as usize * width as usize + position as usize) * 4
            } else {
                (position as usize * width as usize + line as usize) * 4
            };
            for channel in 0..4 {
                prefix[position as usize + 1][channel] =
                    prefix[position as usize][channel] + u64::from(input[index + channel]);
            }
        }
        for position in 0..line_len {
            let start = position.saturating_sub(radius);
            let end = position
                .saturating_add(radius)
                .saturating_add(1)
                .min(line_len);
            let count = u64::from(end - start);
            let index = if horizontal {
                (line as usize * width as usize + position as usize) * 4
            } else {
                (position as usize * width as usize + line as usize) * 4
            };
            for channel in 0..4 {
                let sum = prefix[end as usize][channel] - prefix[start as usize][channel];
                output[index + channel] = ((sum + count / 2) / count) as u8;
            }
        }
    }
    output
}

fn blend_selection(document: &Document, original: &[u8], filtered: &mut [u8]) {
    let width = document.width();
    for (index, (output, input)) in filtered
        .chunks_exact_mut(4)
        .zip(original.chunks_exact(4))
        .enumerate()
    {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let amount = u16::from(document.selection().coverage(x, y));
        if amount != 255 {
            for channel in 0..4 {
                output[channel] = ((u16::from(output[channel]) * amount
                    + u16::from(input[channel]) * (255 - amount)
                    + 127)
                    / 255) as u8;
            }
        }
    }
}

fn rgb_to_hsl(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let lightness = (max + min) * 0.5;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue_sector = if max == red {
        ((green - blue) / delta).rem_euclid(6.0)
    } else if max == green {
        (blue - red) / delta + 2.0
    } else {
        (red - green) / delta + 4.0
    };
    (hue_sector / 6.0, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [u8; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue * 6.0;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector.floor() as i32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = lightness - chroma * 0.5;
    [red, green, blue].map(|channel| ((channel + offset) * 255.0).round().clamp(0.0, 255.0) as u8)
}
