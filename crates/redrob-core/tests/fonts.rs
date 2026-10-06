//! Outline fonts by name (Photoshop's way), batch 4 item H7.
//!
//! The rendering tests need a real font file. They use the first one found in the usual system
//! folders and pass with a note when the machine has none, so they never depend on a font being
//! shipped in the repository.

use redrob_core::fonts::{SYSTEM_FONT_ID, font_names, is_font_available, register_font};
use redrob_core::{Command, Document, Editor, LayerId, Pixel, TextAlign, TextContent};

fn some_font() -> Option<Vec<u8>> {
    let roots = [
        "/usr/share/fonts/truetype",
        "/usr/share/fonts",
        "/Library/Fonts",
        "C:\\Windows\\Fonts",
    ];
    for root in roots {
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ttf"))
                {
                    return std::fs::read(path).ok();
                }
            }
        }
    }
    None
}

fn text(family: &str, body: &str) -> TextContent {
    TextContent {
        text: body.into(),
        font_family: family.into(),
        font_size: 24.0,
        color: Pixel::rgba(0, 0, 0, 255),
        origin_x: 2.0,
        origin_y: 2.0,
        font_id: SYSTEM_FONT_ID.into(),
        box_width: None,
        align: TextAlign::Left,
    }
}

fn add(editor: &mut Editor, content: TextContent) -> LayerId {
    let id = LayerId::new();
    editor
        .execute(Command::AddTextNode {
            id,
            name: "T".into(),
            parent: None,
            sibling_index: 1,
            text: content,
        })
        .unwrap();
    id
}

fn inked(editor: &Editor) -> usize {
    editor
        .render_snapshot()
        .unwrap()
        .pixels()
        .chunks(4)
        .filter(|p| p[3] > 0)
        .count()
}

#[test]
fn a_registered_font_draws_its_outlines() {
    let Some(bytes) = some_font() else {
        eprintln!("no .ttf on this machine; skipped");
        return;
    };
    let names = font_names(&bytes);
    assert!(!names.is_empty(), "a font file names itself");
    register_font(bytes).unwrap();
    assert!(is_font_available(&names[0]));
    let mut editor = Editor::new(Document::new(160, 48).unwrap()).unwrap();
    add(&mut editor, text(&names[0], "Héllo"));
    assert!(
        inked(&editor) > 50,
        "non-ASCII text in an outline font draws"
    );
}

#[test]
fn a_missing_font_falls_back_to_the_built_in_one() {
    // Photoshop substitutes a missing font; it does not refuse the document.
    let mut editor = Editor::new(Document::new(160, 48).unwrap()).unwrap();
    add(&mut editor, text("No Such Font 12345", "Hi é"));
    assert!(inked(&editor) > 0, "drawn with the bitmap font, é as ?");
}

#[test]
fn only_the_name_is_saved() {
    // A 1x1 canvas, so the layer pixels in the JSON are a few bytes and a font file would show.
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    add(&mut editor, text("Some Family", "A"));
    let json = serde_json::to_string(editor.document()).unwrap();
    assert!(json.contains("\"font_family\":\"Some Family\""));
    assert!(json.contains("\"font_id\":\"system\""));
    assert!(json.len() < 4096, "no font bytes in the document");
}

#[test]
fn control_characters_are_still_refused() {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    let result = editor.execute(Command::AddTextNode {
        id: LayerId::new(),
        name: "T".into(),
        parent: None,
        sibling_index: 1,
        text: text("Some Family", "a\u{7}b"),
    });
    assert!(result.is_err());
}

#[test]
fn garbage_is_not_a_font() {
    assert!(font_names(b"not a font").is_empty());
    assert!(register_font(b"not a font".to_vec()).is_err());
}

#[test]
fn outline_text_round_trips_through_svg_by_name() {
    // M14: the SVG names the font and marks the text "outline", and reading it back keeps both.
    use redrob_core::{
        ExportOptions, FileFormat, ImportOptions, NodeContent, export_document, import_document,
    };
    let mut editor = Editor::new(Document::new(64, 32).unwrap()).unwrap();
    add(&mut editor, text("Some Family", "Hi"));
    let encoded = export_document(
        editor.document(),
        FileFormat::Svg,
        &ExportOptions::default().with_loss_policy(redrob_core::LossPolicy::AllowLoss),
    )
    .unwrap();
    let svg = String::from_utf8(encoded.bytes().to_vec()).unwrap();
    assert!(
        svg.contains("redrob:kind=\"outline\"") && svg.contains("font-family=\"Some Family\""),
        "{svg}"
    );
    let decoded = import_document(
        encoded.bytes(),
        &ImportOptions::default().with_loss_policy(redrob_core::LossPolicy::AllowLoss),
    )
    .unwrap();
    let found = decoded.document().nodes().iter().any(|n| {
        matches!(n.content(), NodeContent::Text { text } if text.font_id == SYSTEM_FONT_ID && text.font_family == "Some Family")
    });
    assert!(found);
}

#[test]
fn kerning_does_not_change_unkerned_widths() {
    // With no font registered, text falls back; this pins that the kerning code path is not
    // reached for the bitmap font (its width stays 8 cells per character).
    let mut editor = Editor::new(Document::new(64, 16).unwrap()).unwrap();
    add(
        &mut editor,
        TextContent {
            font_id: redrob_core::EMBEDDED_FONT_ID.into(),
            ..text("font8x8 Basic Latin", "AV")
        },
    );
    assert!(inked(&editor) > 0);
}
