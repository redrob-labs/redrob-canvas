//! OpenType shaping of outline text, batch 4 item L13. Uses fonts the machine has and passes
//! with a note when it has none.

use redrob_core::fonts::{font_names, register_font, shaped_glyphs};

fn load(paths: &[&str]) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    for path in paths {
        let path = path.replace('~', &home);
        if let Ok(bytes) = std::fs::read(&path) {
            let name = font_names(&bytes).into_iter().next()?;
            register_font(bytes).ok()?;
            return Some(name);
        }
    }
    None
}

#[test]
fn fi_becomes_one_ligature_glyph() {
    let Some(family) = load(&["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]) else {
        eprintln!("no DejaVu Sans; skipped");
        return;
    };
    let glyphs = shaped_glyphs(&family, "fi").unwrap();
    assert_eq!(glyphs.len(), 1, "liga: {glyphs:?}");
    assert_eq!(shaped_glyphs(&family, "f i").unwrap().len(), 3, "a space breaks it");
}

#[test]
fn hangul_jamo_compose_into_a_syllable() {
    let Some(family) = load(&["~/.local/share/fonts/NotoSansKR.ttf", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"]) else {
        eprintln!("no Korean font; skipped");
        return;
    };
    // 한 written as conjoining jamo shapes to the same single glyph as the precomposed syllable.
    let jamo = shaped_glyphs(&family, "\u{1112}\u{1161}\u{11AB}").unwrap();
    let syllable = shaped_glyphs(&family, "한").unwrap();
    assert_eq!(syllable.len(), 1);
    assert_eq!(jamo, syllable);
}

#[test]
fn an_unregistered_font_shapes_nothing() {
    assert!(shaped_glyphs("No Such Font 123", "abc").is_none());
}
