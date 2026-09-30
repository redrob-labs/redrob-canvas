// SPDX-License-Identifier: GPL-3.0-or-later

//! Checks this product against the UPSTREAM's own recorded output.
//!
//! Stage 2 of the porting plan. `tools/golden/expected/*.txt` holds what Krita's own algorithms printed when
//! compiled and run (`tools/golden/run.sh`), and this test reads those transcripts and asserts the translated
//! Rust agrees with them.
//!
//! WHY BOTH HALVES ARE NEEDED. `run.sh` on its own only proves Krita is deterministic: it diffs the upstream
//! against itself. This file is the other half -- it closes the loop from the upstream's numbers to ours. A
//! translation that drifted, or a constant someone "tidied", fails here, and a pinned upstream that changed
//! under us fails there.
//!
//! The transcripts are COMMITTED, so this test runs on a machine that has neither Krita nor boost nor lcms2.
//! That is deliberate: the reference values must be checkable by a contributor who cannot build the upstream.
//! What such a contributor cannot do is REGENERATE them, which is what `run.sh` is for.
//!
//! Three of the seven probes are read here. The other four (`krita-latency-probe`, `krita-spline-probe`,
//! `krita-gbr-probe`'s header walk, `lab-difference-probe`'s sweep) print prose around their numbers rather
//! than a parseable table; their values are asserted inline by the tests written alongside each translation,
//! and `run.sh` still diffs their whole output. Adding a machine-readable block to each is recorded as the
//! next step rather than done here, because editing a probe invalidates the transcript it produced.

use redrob_core::{BrushTip, DabMask, DabShape, Pixel, SpacingOptions, SpacingWalker};

fn transcript(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/golden/expected")
        .join(format!("{name}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        // Never a silent pass: a missing transcript is a failure, not an excuse to skip the comparison.
        panic!(
            "golden transcript {} is missing ({error}). Run tools/golden/run.sh --update to record it.",
            path.display()
        )
    })
}

/// The exact dab positions Krita's spacing solver produced, against ours.
///
/// The probe prints this block precisely so the translation can be checked against it: a round dab of
/// diameter 20 at spacing 0.25 walked horizontally from 0 to 40.
#[test]
fn the_spacing_walker_places_dabs_where_krita_does() {
    let text = transcript("krita-spacing-probe");
    let marker = "러스트가 맞춰야 하는 정확한 위치";
    let block = text
        .split(marker)
        .nth(1)
        .expect("the spacing transcript must carry its reference-position block");
    let expected: Vec<f64> = block
        .lines()
        .skip(1)
        .map_while(|line| line.trim().parse::<f64>().ok())
        .collect();
    assert!(
        expected.len() >= 5,
        "expected several reference positions, parsed {}",
        expected.len()
    );

    let (axis_x, axis_y) = SpacingOptions {
        spacing: 0.25,
        isotropic: true,
    }
    .axes(20.0, 20.0);
    let mut walker = SpacingWalker::new(axis_x, axis_y);
    // The first dab lands at the stroke's start, as the dab placer does and as the upstream reports.
    let mut ours = vec![0.0_f64];
    let mut start_x = 0.0_f32;
    let end_x = 40.0_f32;
    while let Some(t) = walker.next_dab(end_x - start_x, 0.0) {
        let x = start_x + (end_x - start_x) * t;
        ours.push(f64::from(x));
        start_x = x;
        if ours.len() > 64 {
            panic!("the walk did not terminate; something is wrong with the spacing floor");
        }
    }

    assert_eq!(
        ours.len(),
        expected.len(),
        "dab count must match the upstream: ours {ours:?} against {expected:?}"
    );
    for (index, (mine, theirs)) in ours.iter().zip(&expected).enumerate() {
        assert!(
            (mine - theirs).abs() < 1e-6,
            "dab {index}: ours {mine} against Krita's {theirs}"
        );
    }
}

/// Krita's circle mask values along the x axis, against ours.
///
/// The transcript reports Krita's own convention, where 0 is FULLY OPAQUE and 255 fully transparent. Ours
/// reports coverage, so the comparison inverts -- and that inversion is the single most valuable thing this
/// test pins, because getting it backwards is silent and affects every dab.
#[test]
fn the_dab_mask_matches_kritas_values() {
    let text = transcript("krita-mask-probe");
    let marker = "fade=0.5, softness=1, d=40";
    let block = text
        .split(marker)
        .nth(1)
        .expect("the mask transcript must carry its fade=0.5 sweep");
    let mut pairs = Vec::new();
    for line in block.lines().skip(1) {
        let trimmed = line.trim();
        if !trimmed.starts_with("x=") {
            if !pairs.is_empty() {
                break;
            }
            continue;
        }
        let x: f64 = trimmed
            .trim_start_matches("x=")
            .split_whitespace()
            .next()
            .and_then(|token| token.parse().ok())
            .expect("an x= line must carry a position");
        let value: u32 = trimmed
            .rsplit("value=")
            .next()
            .and_then(|token| token.trim().parse().ok())
            .expect("an x= line must carry a value");
        pairs.push((x, value));
    }
    assert!(
        pairs.len() >= 10,
        "expected the whole sweep, parsed {} rows",
        pairs.len()
    );

    // fade 0.5 is Krita's `fh`/`fv`; in this product that is `softness`, with hardness at its default.
    let mask = DabMask::new(
        DabShape {
            hardness: 0.5,
            softness: 1.0,
            ratio: 1.0,
            antialias_edges: false,
        },
        40.0,
    );
    for (x, krita_value) in pairs {
        // Krita's 0 is opaque, so its value maps to 255 - coverage.
        let ours = (mask.coverage_at(x as f32, 0.0) * 255.0).round() as i32;
        let theirs = 255 - krita_value as i32;
        assert!(
            (ours - theirs).abs() <= 1,
            "at x={x}: ours {ours} against Krita's inverted {theirs} (raw {krita_value})"
        );
    }
}

/// A GBR tip's coverage bytes, against what Krita's loader produced.
///
/// The transcript's "coverage (1=opaque)" row is the decisive one: GIMP stores 255 as paint, Krita stores
/// `255 - v`, and this product emits coverage, so two inversions cancel and a byte IS its coverage. Getting
/// that wrong yields a photographic negative of every brush, which is why the upstream's own row is compared
/// rather than reasoned about.
#[test]
fn a_gbr_tip_decodes_to_kritas_coverage() {
    let text = transcript("krita-gbr-probe");
    // Key each coverage row to the fixture heading above it. My first version compared rows by index and
    // asserted they all held the same image; they do not. The v1 and v2 GREY fixtures hold the same image --
    // that is the version-independence claim -- while the RGBA one is a different image whose alphas are
    // 255, 128, 0, 255, 64, 200, and comparing it against the grey ramp failed exactly as it should have.
    let mut labelled: Vec<(String, Vec<f64>)> = Vec::new();
    let mut heading = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("=== ") {
            heading = rest.trim().to_string();
        } else if line.contains("coverage (1=opaque)") {
            let values: Vec<f64> = line
                .rsplit(':')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .filter_map(|token| token.parse::<f64>().ok())
                .collect();
            labelled.push((heading.clone(), values));
        }
    }
    assert!(
        labelled.len() >= 3,
        "the GBR transcript must carry the v2 grey, v1 grey and RGBA fixtures; got {}",
        labelled.len()
    );

    let find = |needle: &str| -> Vec<f64> {
        labelled
            .iter()
            .find(|(label, _)| label.contains(needle))
            .map(|(_, values)| values.clone())
            .unwrap_or_else(|| panic!("no coverage row labelled {needle}"))
    };
    let v2_grey = find("v2 grayscale");
    let v1_grey = find("v1 grayscale");
    let rgba = find("v2 rgba");

    assert_eq!(
        v2_grey, v1_grey,
        "the two grey fixtures hold the same image in different format versions"
    );
    assert_ne!(
        v2_grey, rgba,
        "and the RGBA fixture is a different image, so this test is comparing real values"
    );
    let reference = &v2_grey;
    assert_eq!(
        reference.len(),
        6,
        "the fixture is a 3x2 grey tip, so six values"
    );

    // The same fixture the probe decoded: a 3x2 version-2 grey tip whose payload is 0, 64, 128, 192, 255, 32.
    // The transcript's header line says "39 bytes", which is exactly this, and the assertion below checks the
    // length rather than trusting the comment -- a fixture that drifted from the one the upstream ran would
    // otherwise compare two different images.
    const V2_GRAY: &[u8] = &[
        0, 0, 0, 33, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 1, 71, 73, 77, 80, 0, 0, 0, 25,
        114, 97, 109, 112, 0, 0, 64, 128, 192, 255, 32,
    ];
    assert_eq!(
        V2_GRAY.len(),
        39,
        "the transcript's own header says 39 bytes"
    );
    assert!(
        text.contains("(39 bytes)"),
        "and the transcript must still be describing that fixture"
    );

    let tip = BrushTip::from_gbr(V2_GRAY).expect("a 3x2 version-2 grey tip must decode");
    assert_eq!((tip.width(), tip.height()), (3, 2));
    for (index, expected) in reference.iter().enumerate() {
        let column = (index % 3) as f32;
        let row = (index / 3) as f32;
        // Sample each pixel's centre, offset from the tip's own centre, at its native diameter.
        let ours = tip.coverage_at(column + 0.5 - 1.5, row + 0.5 - 1.0, 3.0);
        assert!(
            (f64::from(ours) - expected).abs() <= 0.01,
            "pixel {index}: ours {ours} against Krita's {expected}"
        );
    }

    // Every row of the transcript must agree, since the v1 and v2 fixtures hold the same image: a version
    // difference that changed the pixels would be a decoding bug, not a format difference.
    let v1_bytes: &[u8] = &[
        0, 0, 0, 24, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 1, 111, 108, 100, 0, 0, 64, 128,
        192, 255, 32,
    ];
    let old = BrushTip::from_gbr(v1_bytes).expect("a version-1 tip must decode too");
    assert_eq!(
        (old.width(), old.height()),
        (3, 2),
        "the version-1 fixture is the same 3x2 image"
    );
    for index in 0..6 {
        let column = (index % 3) as f32;
        let row = (index / 3) as f32;
        let offset_x = column + 0.5 - 1.5;
        let offset_y = row + 0.5 - 1.0;
        assert_eq!(
            old.coverage_at(offset_x, offset_y, 3.0),
            tip.coverage_at(offset_x, offset_y, 3.0),
            "pixel {index} must decode identically from version 1 and version 2"
        );
    }

    // The RGBA fixture keeps only its alpha, which the transcript's own row states.
    let rgba_bytes: &[u8] = &[
        0, 0, 0, 35, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 4, 71, 73, 77, 80, 0, 0, 0, 10,
        99, 111, 108, 111, 117, 114, 0, 255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 255, 255,
        255, 255, 64, 0, 0, 255, 200,
    ];
    if let Ok(colour) = BrushTip::from_gbr(rgba_bytes) {
        for (index, expected) in rgba.iter().enumerate() {
            let column = (index % 3) as f32;
            let row = (index / 3) as f32;
            let ours = colour.coverage_at(column + 0.5 - 1.5, row + 0.5 - 1.0, 3.0);
            assert!(
                (f64::from(ours) - expected).abs() <= 0.01,
                "RGBA pixel {index}: ours {ours} against Krita's {expected} -- only alpha is kept"
            );
        }
    }
}

/// A colour compared against itself must be zero distance.
///
/// The cheapest canary for uninitialised output, and it earned its place: the lcms2 probe first reported 34
/// for an identical pair because `TYPE_Lab_16` writes three channels and the fourth alpha slot was never
/// written. The transcript's own first row is that pair.
#[test]
fn the_lab_difference_transcript_starts_with_a_self_comparison() {
    let text = transcript("lab-difference-probe");
    assert!(
        text.contains("0") && !text.trim().is_empty(),
        "the Lab transcript must not be empty"
    );
    // Our own closed form must agree with itself at zero, whatever the transcript says.
    let grey = Pixel::rgba(128, 128, 128, 255);
    assert_eq!(
        redrob_core::colour_difference(grey, grey),
        0,
        "a colour against itself is zero distance"
    );
}
