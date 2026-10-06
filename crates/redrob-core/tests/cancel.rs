//! P8b. Cancelling a running filter: nothing is committed, and the next command is unaffected.

use std::time::{Duration, Instant};

use redrob_core::{CancelToken, Command, CoreError, Document, Editor, Filter, Pixel, with_cancel};

#[path = "common/canvas.rs"]
mod canvas;

fn small() -> Editor {
    canvas::editor(
        2,
        1,
        &[Pixel::rgba(10, 20, 30, 255), Pixel::rgba(200, 100, 50, 255)],
    )
}

fn invert() -> Command {
    Command::ApplyFilter {
        filter: Filter::Invert,
    }
}

#[test]
fn a_cancelled_filter_commits_nothing() {
    let mut editor = small();
    let before = editor.document().layers()[0].pixels().to_vec();
    let depth = editor.undo_depth();
    let generation = editor.generation();

    let token = CancelToken::new();
    token.cancel();
    let result = with_cancel(&token, || editor.execute(invert()));

    assert!(
        matches!(result, Err(CoreError::Cancelled)),
        "got {result:?}"
    );
    assert_eq!(editor.document().layers()[0].pixels(), before.as_slice());
    assert_eq!(
        editor.undo_depth(),
        depth,
        "a cancelled filter must not enter history"
    );
    assert_eq!(editor.generation(), generation);
}

#[test]
fn the_token_is_scoped_to_the_call_and_can_be_reset() {
    let mut editor = small();
    let token = CancelToken::new();
    token.cancel();
    assert!(with_cancel(&token, || editor.execute(invert())).is_err());

    // Outside `with_cancel` the thread has no token, so the same command runs.
    editor
        .execute(invert())
        .expect("no token outside the scope");

    // And a reset token lets the next call through.
    token.reset();
    with_cancel(&token, || editor.execute(invert())).expect("a reset token must not cancel");
}

#[test]
fn an_uncancelled_token_changes_nothing() {
    let mut with_token = small();
    let mut without = small();
    with_cancel(&CancelToken::new(), || with_token.execute(invert())).unwrap();
    without.execute(invert()).unwrap();
    assert_eq!(
        with_token.document().layers()[0].pixels(),
        without.document().layers()[0].pixels()
    );
}

#[test]
fn mosaic_stops_mid_run_when_cancelled_from_another_thread() {
    // Mosaic tests every pixel against every seed and took one to two minutes on 1280x800, which
    // is why it carries a checkpoint per row. At 1024x1024 it runs far longer than the delay below,
    // so a prompt Cancelled proves the row checkpoint, not the entry one.
    let mut editor = Editor::new(Document::new(1024, 1024).unwrap()).unwrap();
    let mosaic: Filter =
        serde_json::from_value(serde_json::json!({ "kind": "mosaic", "tile_size": 16 })).unwrap();
    let token = CancelToken::new();
    let remote = token.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        remote.cancel();
    });
    let started = Instant::now();
    let result = with_cancel(&token, || {
        editor.execute(Command::ApplyFilter { filter: mosaic })
    });
    canceller.join().unwrap();
    assert!(
        matches!(result, Err(CoreError::Cancelled)),
        "got {result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "cancel took {:?}; the row checkpoint is not being reached",
        started.elapsed()
    );
}
