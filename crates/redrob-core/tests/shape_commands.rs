// SPDX-License-Identifier: GPL-3.0-or-later
//! `AddShapeNode` end to end: command in, vector node out, undo puts it back.
//!
//! This is the test that says the ported geometry is CONNECTED rather than merely compiled.
//! Three cycles of Graphite geometry landed with their own unit tests passing while nothing
//! in the product could reach any of it; these assertions are what distinguishes the two
//! states, and they would have failed at any point before the bridge existed.

use redrob_core::{Command, Document, Editor, NodeContent, NodeId, Pixel, Shape, VectorPath};

fn editor() -> Editor {
    Editor::new(Document::new(64, 64).unwrap()).unwrap()
}

fn shape_command(shape: Shape) -> Command {
    Command::AddShapeNode {
        id: NodeId::new(),
        name: "Shape".into(),
        parent: None,
        sibling_index: 1,
        shape,
        paint: VectorPath {
            fill: Some(Pixel::rgba(10, 20, 30, 255)),
            ..VectorPath::default()
        },
    }
}

#[test]
fn a_shape_command_creates_a_vector_node_with_a_real_path() {
    let mut editor = editor();
    let command = shape_command(Shape::Ellipse {
        x1: 8.0,
        y1: 8.0,
        x2: 56.0,
        y2: 40.0,
    });
    let id = match &command {
        Command::AddShapeNode { id, .. } => *id,
        _ => unreachable!(),
    };

    let changes = editor.execute(command).unwrap();
    assert!(
        changes.structure_changed,
        "a new node changes the structure"
    );
    assert!(changes.canvas_changed, "and what is drawn");

    let node = editor
        .document()
        .nodes()
        .iter()
        .find(|n| n.id() == id)
        .expect("the node is in the document");

    let NodeContent::Vector { vector } = node.content() else {
        panic!(
            "a shape node holds vector content, got {:?}",
            node.content()
        );
    };
    assert_eq!(vector.paths.len(), 1, "one shape, one path");

    let path = &vector.paths[0];
    assert_eq!(
        path.commands.len(),
        6,
        "an ellipse is a move, four cubics and a close"
    );
    assert_eq!(
        path.fill,
        Some(Pixel::rgba(10, 20, 30, 255)),
        "the paint carried through"
    );
}

#[test]
fn undo_removes_the_shape_and_redo_brings_it_back() {
    let mut editor = editor();
    let before = editor.document().nodes().len();

    editor
        .execute(shape_command(Shape::RegularPolygon {
            center_x: 32.0,
            center_y: 32.0,
            sides: 6,
            radius: 20.0,
        }))
        .unwrap();
    assert_eq!(editor.document().nodes().len(), before + 1);

    editor.undo().unwrap();
    assert_eq!(
        editor.document().nodes().len(),
        before,
        "undo removed the shape node"
    );

    editor.redo().unwrap();
    assert_eq!(
        editor.document().nodes().len(),
        before + 1,
        "redo restored it"
    );
}

#[test]
fn a_degenerate_shape_is_refused_and_changes_nothing() {
    let mut editor = editor();
    let before = editor.document().nodes().len();

    let result = editor.execute(shape_command(Shape::RegularPolygon {
        center_x: 32.0,
        center_y: 32.0,
        sides: 2,
        radius: 10.0,
    }));

    assert!(result.is_err(), "two sides is not a polygon");
    assert_eq!(
        editor.document().nodes().len(),
        before,
        "a refused command leaves the document alone"
    );
    assert!(
        !editor.can_undo(),
        "and leaves nothing on the undo stack either"
    );
}

#[test]
fn the_shape_survives_the_project_file() {
    // The point of recording the SHAPE rather than its expanded coordinates: a command has
    // to round trip through the project format, so this asserts the serialised form comes
    // back as the same command.
    let command = shape_command(Shape::Star {
        center_x: 32.0,
        center_y: 32.0,
        sides: 5,
        radius: 24.0,
        inner_radius: 10.0,
    });
    let json = serde_json::to_string(&command).unwrap();
    let back: Command = serde_json::from_str(&json).unwrap();

    match (&command, &back) {
        (Command::AddShapeNode { shape: a, .. }, Command::AddShapeNode { shape: b, .. }) => {
            assert_eq!(a, b, "the shape came back as itself: {json}")
        }
        _ => panic!("the command changed variant across a round trip"),
    }

    // And the deserialised command still builds the same path.
    let mut one = editor();
    let mut two = editor();
    one.execute(command).unwrap();
    two.execute(back).unwrap();
    assert_eq!(
        one.render_snapshot().unwrap().pixels(),
        two.render_snapshot().unwrap().pixels(),
        "the same shape renders the same pixels before and after serialisation"
    );
}

#[test]
fn every_shape_variant_reaches_the_document() {
    // A closed enum is only worth having if every arm works. One unreachable variant is a
    // format promise the product cannot keep.
    let shapes = [
        Shape::Rectangle {
            x1: 4.0,
            y1: 4.0,
            x2: 30.0,
            y2: 20.0,
        },
        Shape::RoundedRectangle {
            x1: 4.0,
            y1: 4.0,
            x2: 30.0,
            y2: 20.0,
            radius: 3.0,
        },
        Shape::Ellipse {
            x1: 4.0,
            y1: 4.0,
            x2: 30.0,
            y2: 20.0,
        },
        Shape::RegularPolygon {
            center_x: 20.0,
            center_y: 20.0,
            sides: 7,
            radius: 12.0,
        },
        Shape::Star {
            center_x: 20.0,
            center_y: 20.0,
            sides: 6,
            radius: 12.0,
            inner_radius: 5.0,
        },
        Shape::Line {
            x1: 2.0,
            y1: 2.0,
            x2: 40.0,
            y2: 40.0,
        },
    ];

    for shape in shapes {
        let mut editor = editor();
        editor
            .execute(shape_command(shape))
            .unwrap_or_else(|error| panic!("{shape:?} was refused: {error}"));
        let node = editor.document().nodes().last().unwrap();
        let NodeContent::Vector { vector } = node.content() else {
            panic!("{shape:?} did not produce vector content");
        };
        assert!(
            !vector.paths[0].commands.is_empty(),
            "{shape:?} produced an empty path"
        );
    }
}
