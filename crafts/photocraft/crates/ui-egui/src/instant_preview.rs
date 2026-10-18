//! Instant preview (Preferences › Performance): while a heavy operation is in progress on a
//! large document (a brush stroke, a transform, a move, a running filter job) the canvas texture
//! is rebuilt at a reduced resolution, and refined to full resolution once the operation ends.
//! The reduction factor adapts to how long the last full-resolution frame took.

use redrob_photo_doc::Document;

use crate::PhotocraftApp;

/// The preview's state: the factor in force and when the operation ended (for the refine).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InstantPreview {
    /// The reduction factor the last frame used (1 = full resolution).
    pub factor: u32,
    /// Milliseconds of the last full-resolution composite; drives the factor.
    pub last_full_ms: f64,
    /// Whether a heavy operation was in progress last frame.
    pub busy: bool,
}

/// Is a heavy operation in progress: a painting or transforming drag, a running job, an
/// adjustment preview still settling?
pub fn busy(app: &PhotocraftApp) -> bool {
    let dragging = app.drag.as_ref().is_some_and(|d| d.tool.is_brushlike() || matches!(d.tool, crate::state::Tool::Move | crate::state::Tool::Gradient));
    dragging || app.ui.transform.is_some() || app.live_stroke.is_some() || !app.session.jobs().is_empty() || crate::adjust_preview::settling(app, 200.0)
}

/// The reduction factor for `doc` this frame: 1 when the preview is off, the document is small or
/// nothing heavy is happening; else 2, 4 or 8 by how slow the last full composite was.
pub fn factor(app: &PhotocraftApp, doc: &Document) -> u32 {
    let perf = &app.session.prefs().performance;
    if !perf.instant_preview {
        return 1;
    }
    let megapixels = doc.size.area() as f64 / 1_000_000.0;
    if megapixels < f64::from(perf.instant_preview_megapixels.max(0.0)) {
        return 1;
    }
    if !busy(app) {
        return 1;
    }
    let ms = app.instant_preview.last_full_ms;
    if ms > 400.0 {
        8
    } else if ms > 120.0 {
        4
    } else {
        2
    }
}

/// Fold the factor into a canvas cache key so the refine re-renders when the factor returns to 1.
pub fn key(factor: u32) -> u64 {
    if factor <= 1 { 0 } else { 0x5f3b_ab49_0000_0000 ^ u64::from(factor) }
}

/// Record a full-resolution composite's cost and this frame's factor; asks for a repaint when a
/// reduced frame was shown and the operation ended, so the refine lands at once.
pub fn note(app: &mut PhotocraftApp, ctx: &egui::Context, factor: u32, full_ms: Option<f64>) {
    let ip = &mut app.instant_preview;
    if let Some(ms) = full_ms {
        ip.last_full_ms = ms;
    }
    let busy_now = busy(app);
    let ip = &mut app.instant_preview;
    if ip.busy && !busy_now && ip.factor > 1 {
        ctx.request_repaint();
    }
    ip.busy = busy_now;
    ip.factor = factor;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_distinguish_factors() {
        assert_eq!(key(1), 0);
        assert_ne!(key(2), key(4));
        assert_ne!(key(2), 0);
    }
}
