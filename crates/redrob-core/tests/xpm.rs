// SPDX-License-Identifier: GPL-3.0-or-later

//! X PixMap (M.7c, third of M.7's four formats).
//!
//! Re-derived from `plug-ins/common/file-xpm.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. No usable pure-Rust codec exists — `xpm 0.1.0` on the registry
//! is a **package manager**, not this format — so `crate::xpm` carries the implementation.
//!
//! **Upstream does not parse XPM itself; it calls libXpm.** So the text grammar comes from the
//! format, and what is re-derived here is everything GIMP does *around* that call. Four rules,
//! one test each:
//!
//! 1. The colour spec is chosen by **preference order** `c > g > g4 > m`, not first-present.
//! 2. The default spec is the literal `"None"`, so an entry with no usable visual is transparent.
//! 3. A transparent entry is **all zeros, alpha included**.
//! 4. Export derives `cpp` from a **92-character alphabet whose first character is a SPACE**, and
//!    indexes it **least-significant digit first**; with alpha, **entry 0 is reserved as `None`**.

use redrob_core::{
    ExportOptions, FileFormat, ImportOptions, Pixel, detect_format, export_document,
    import_document,
};

#[path = "common/canvas.rs"]
mod canvas;

const RED: Pixel = Pixel {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: Pixel = Pixel {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};
const CLEAR: Pixel = Pixel {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

fn export(width: u32, height: u32, pixels: &[Pixel]) -> String {
    let bytes = export_document(
        canvas::editor(width, height, pixels).document(),
        FileFormat::Xpm,
        &ExportOptions::default(),
    )
    .expect("export")
    .into_bytes();
    String::from_utf8(bytes).expect("XPM is text")
}

fn decode(text: &str) -> (u32, u32, Vec<u8>) {
    let outcome = import_document(text.as_bytes(), &ImportOptions::default()).expect("import");
    let document = outcome.document();
    (
        document.width(),
        document.height(),
        document.layers()[0].pixels().to_vec(),
    )
}

/// One-colour fixture, so the colour-spec rules can be stated as text and read as text.
fn one_colour(spec: &str) -> String {
    format!("/* XPM */\nstatic char * i[] = {{\n\"1 1 1 1\",\n\"a\t{spec}\",\n\"a\"\n}};\n")
}

/// The export round-trips, and **the first palette key is a SPACE**.
///
/// Upstream's `linenoise` alphabet begins with a space — measured at 92 characters, not counted by
/// eye — so index 0's key is `" "`, a space sitting inside a quoted C string. That is legal and
/// surprising, which is why it is asserted rather than assumed.
#[test]
fn the_first_palette_key_is_a_space() {
    let text = export(2, 1, &[RED, BLUE]);

    assert!(text.starts_with("/* XPM */"), "the magic is the comment");
    assert!(
        text.contains("\" \tc #FF0000\""),
        "index 0's key is a space; got:\n{text}"
    );
    assert!(text.contains("\"2 1 2 1\""), "width height ncolors cpp");

    assert_eq!(detect_format(text.as_bytes()).unwrap(), FileFormat::Xpm);
    let (width, height, pixels) = decode(&text);
    assert_eq!((width, height), (2, 1));
    assert_eq!(&pixels[..8], &[255, 0, 0, 255, 0, 0, 255, 255]);
}

/// **With alpha, entry 0 is reserved as `None` and the real colours shift up by one.**
///
/// Upstream does `if (alpha_used) ncolors++;` then `set_XpmImage (colormap, 0, "None")`. Asserted
/// against the opaque case above, where index 0 is a real colour instead — a reservation that
/// always happened, or never, would pass one of the two alone.
#[test]
fn alpha_reserves_the_first_entry_as_none() {
    let text = export(2, 1, &[RED, CLEAR]);

    assert!(
        text.contains("\" \tc None\""),
        "index 0 is the transparent entry; got:\n{text}"
    );
    assert!(
        text.contains("\".\tc #FF0000\""),
        "so the real colour moved to index 1; got:\n{text}"
    );

    let (_, _, pixels) = decode(&text);
    assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
    // Rule 3: a transparent entry is all zeros, alpha included.
    assert_eq!(
        &pixels[4..8],
        &[0, 0, 0, 0],
        "transparent is zeros, not black"
    );
}

/// **The palette index is converted LEAST-SIGNIFICANT DIGIT FIRST.**
///
/// Upstream writes `charnum = indtemp % base; indtemp /= base; *p++ = linenoise[charnum]`, so the
/// low digit lands in the first character. Most implementations would write it the other way, so
/// this needs a case where the two differ — which takes more than 92 colours, since below that
/// every key is one character and the orders are indistinguishable.
///
/// With 93 colours `cpp` becomes 2 and two indices discriminate:
///
/// * index 91 — low digit 91 (the alphabet's LAST character, a backtick), high digit 0 (a space),
///   so `` "` " `` least-significant first and `" `"` the other way round;
/// * index 92 — low digit 0 (space), high digit 1 (a dot), so `" ."` against `". "`.
#[test]
fn the_palette_index_is_written_least_significant_digit_first() {
    let many: Vec<Pixel> = (0..93u32)
        .map(|index| Pixel {
            r: (index * 2) as u8,
            g: (index * 3) as u8,
            b: (index * 5) as u8,
            a: 255,
        })
        .collect();
    let text = export(93, 1, &many);

    // cpp = 1 + (gint) (log (93) / log (92)) = 2.
    assert!(
        text.contains("\"93 1 93 2\""),
        "93 colours need two characters"
    );

    let lines: Vec<&str> = text.lines().collect();
    // Line 0 is the magic, 1 the declaration, 2 the values line; colours start at 3.
    assert_eq!(
        lines[3 + 91],
        "\"` \tc #B611C7\",",
        "index 91: the low digit (a backtick) comes FIRST"
    );
    assert_eq!(
        lines[3 + 92],
        "\" .\tc #B814CC\",",
        "index 92: low digit 0 (space) first, high digit 1 (dot) second"
    );

    let (width, _, pixels) = decode(&text);
    assert_eq!(width, 93);
    assert_eq!(&pixels[4..8], &[2, 3, 5, 255], "and it still round-trips");
}

/// **The colour spec is chosen by PREFERENCE ORDER, not by taking the first one present.**
///
/// `parse_colors` tries `c_color`, then `g_color`, then `g4_color`, then `m_color`. The fixture
/// puts `m white` BEFORE `c red` in the text, so a reader taking the first key would answer white.
#[test]
fn the_colour_spec_is_chosen_by_preference_not_by_position() {
    let (_, _, pixels) = decode(&one_colour("m white c red"));
    assert_eq!(
        &pixels[..4],
        &[255, 0, 0, 255],
        "c wins over m even though m is written first"
    );

    // And with no `c`, the grey spec is used rather than the mono one.
    let (_, _, grey) = decode(&one_colour("m white g #808080"));
    assert_eq!(&grey[..4], &[128, 128, 128, 255], "g beats m");
}

/// **`"None"` is the DEFAULT as well as the transparent marker.**
///
/// `parse_colors` initialises `colorspec = "None"` before examining anything, so an entry that
/// declares only a symbolic name — `s`, which is never a colour — becomes transparent rather than
/// an error or black. Both spellings are checked: the explicit `c None` and the implicit default.
#[test]
fn an_entry_with_no_visual_spec_defaults_to_transparent() {
    let (_, _, symbolic) = decode(&one_colour("s mysymbol"));
    assert_eq!(
        &symbolic[..4],
        &[0, 0, 0, 0],
        "no visual spec means None, which means transparent"
    );

    let (_, _, explicit) = decode(&one_colour("c None"));
    assert_eq!(&explicit[..4], &[0, 0, 0, 0]);
}

/// The three hex widths X11 defines all parse, and each scales up to eight bits.
#[test]
fn the_three_hex_widths_all_parse() {
    for spec in ["c #f00", "c #FF0000", "c #FFFF00000000"] {
        let (_, _, pixels) = decode(&one_colour(spec));
        assert_eq!(&pixels[..4], &[255, 0, 0, 255], "{spec}");
    }

    // A width that is not 3, 6 or 12 is refused rather than half-read.
    let bad = one_colour("c #FF00");
    assert!(import_document(bad.as_bytes(), &ImportOptions::default()).is_err());
}

/// An unrecognised colour NAME is refused rather than guessed at.
///
/// Upstream hands names to `XParseColor` or `gdk_rgba_parse`, which carry the whole X11 colour
/// database — reproducing that here would be inventing, not re-deriving. Worse, upstream **ignores
/// `gdk_rgba_parse`'s failure return** and uses whatever was left in the struct, which is
/// undefined. Refusing is the honest answer; replicating undefined behaviour is not.
#[test]
fn an_unknown_colour_name_is_refused_not_guessed() {
    let text = one_colour("c cornflowerblue");
    assert_eq!(detect_format(text.as_bytes()).unwrap(), FileFormat::Xpm);
    assert!(
        import_document(text.as_bytes(), &ImportOptions::default()).is_err(),
        "a name this product cannot resolve must not become a silently wrong colour"
    );

    // The handful of names that ARE resolved still work, so this is not a blanket refusal.
    let (_, _, pixels) = decode(&one_colour("c white"));
    assert_eq!(&pixels[..4], &[255, 255, 255, 255]);
}

/// A PNG presented as an XPM is refused rather than decoded as what it really is.
#[test]
fn a_png_presented_as_an_xpm_is_refused() {
    let png = export_document(
        canvas::editor(1, 1, &[RED]).document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::Xpm);
    assert!(import_document(&png, &options).is_err());
}
