//! L11: a render taken off the editor (detach_render / RenderJob::run / finish_detached_render).

use redrob_core::{Command, Document, Editor, Pixel};

fn red_editor() -> Editor {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    editor.execute(Command::SelectAll).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor
}

#[test]
fn a_detached_render_shows_the_state_it_was_taken_from() {
    let editor = red_editor();
    let generation = editor.generation();
    let done = editor.detach_render().run();
    editor.finish_detached_render(&done);
    let snapshot = done.into_result().unwrap();
    assert_eq!(snapshot.generation(), generation);
    assert_eq!(&snapshot.rgba8()[0..4], &[255, 0, 0, 255]);
}

#[test]
fn an_edit_during_a_detached_render_still_reaches_the_next_frame() {
    let mut editor = red_editor();
    let _ = editor.render_snapshot().unwrap();
    let job = editor.detach_render();
    // The user keeps painting while the worker renders the old state.
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 255, 255),
        })
        .unwrap();
    let done = job.run();
    assert_eq!(
        &done.result().as_ref().unwrap().rgba8()[0..4],
        &[255, 0, 0, 255],
        "old state"
    );
    editor.finish_detached_render(&done);
    drop(done);
    assert_eq!(
        &editor.render_snapshot().unwrap().rgba8()[0..4],
        &[0, 0, 255, 255],
        "the edit is not lost"
    );
}

#[test]
fn an_older_detached_frame_never_replaces_a_newer_inline_one() {
    let mut editor = red_editor();
    let job = editor.detach_render();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 255, 0, 255),
        })
        .unwrap();
    let _inline = editor.render_snapshot().unwrap();
    let done = job.run();
    editor.finish_detached_render(&done);
    drop(done);
    assert_eq!(
        &editor.render_snapshot().unwrap().rgba8()[0..4],
        &[0, 255, 0, 255]
    );
}

#[test]
fn render_jobs_can_cross_threads() {
    fn is_send<T: Send>() {}
    is_send::<redrob_core::RenderJob>();
    is_send::<redrob_core::RenderDone>();
}
