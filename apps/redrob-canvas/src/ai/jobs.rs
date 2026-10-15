// SPDX-License-Identifier: MIT OR Apache-2.0

//! One AI job at a time: the UI gathers the inputs from the mode bridge, hands them to the tokio
//! runtime, and polls the result each frame. The queue itself ([`JobQueue`]) is a plain state
//! machine over `std::sync::mpsc`, so it is testable without an engine or a runtime.

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

use redrob_ai::{
    Expand, Mask, Rgba8Image,
    iopaint::{Engine, InpaintOptions},
    vectorize::TraceResult,
};

use super::tools::ToolReply;

/// The tools the panel, the menu and the agent share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Erase,
    Inpaint,
    Outpaint,
    RemoveBackground,
    Upscale,
    FaceRestore,
    SelectSubject,
    Vectorize,
}

impl ToolKind {
    pub const ALL: [ToolKind; 8] = [
        ToolKind::Erase,
        ToolKind::Inpaint,
        ToolKind::Outpaint,
        ToolKind::RemoveBackground,
        ToolKind::Upscale,
        ToolKind::FaceRestore,
        ToolKind::SelectSubject,
        ToolKind::Vectorize,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Erase => "Erase",
            ToolKind::Inpaint => "Inpaint",
            ToolKind::Outpaint => "Outpaint / Expand canvas",
            ToolKind::RemoveBackground => "Remove background",
            ToolKind::Upscale => "Upscale",
            ToolKind::FaceRestore => "Face restore",
            ToolKind::SelectSubject => "Select subject",
            ToolKind::Vectorize => "Vectorize",
        }
    }

    /// The agent's tool name.
    pub fn tool_name(self) -> &'static str {
        match self {
            ToolKind::Erase => "erase_selection",
            ToolKind::Inpaint => "inpaint",
            ToolKind::Outpaint => "outpaint",
            ToolKind::RemoveBackground => "remove_background",
            ToolKind::Upscale => "upscale",
            ToolKind::FaceRestore => "face_restore",
            ToolKind::SelectSubject => "select_subject",
            ToolKind::Vectorize => "vectorize",
        }
    }

    pub fn from_tool_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.tool_name() == name)
    }

    /// Whether the tool needs the local IOPaint engine (Vectorize traces in-process).
    pub fn needs_engine(self) -> bool {
        !matches!(self, ToolKind::Vectorize)
    }

    /// Whether the tool reads the selection mask.
    pub fn needs_selection(self) -> bool {
        matches!(self, ToolKind::Erase | ToolKind::Inpaint)
    }

    /// Whether the tool writes a selection.
    pub fn writes_selection(self) -> bool {
        matches!(self, ToolKind::SelectSubject)
    }

    /// Whether the tool adds a layer / image to the document.
    pub fn writes_layer(self) -> bool {
        matches!(
            self,
            ToolKind::Erase
                | ToolKind::Inpaint
                | ToolKind::Outpaint
                | ToolKind::RemoveBackground
                | ToolKind::Upscale
                | ToolKind::FaceRestore
        )
    }

    /// Whether the tool grows the canvas.
    pub fn grows_canvas(self) -> bool {
        matches!(self, ToolKind::Outpaint)
    }

    /// Whether `caps` (the current mode and document) can serve this tool. Pure, so the menu
    /// test can table it.
    pub fn available(self, caps: &super::bridge::Capabilities) -> bool {
        if !caps.has_document {
            return false;
        }
        if self.needs_selection() && !caps.selection {
            return false;
        }
        if self.writes_selection() && !caps.selection {
            return false;
        }
        if self.writes_layer() && !caps.place_layer {
            return false;
        }
        if self.grows_canvas() && !caps.expand {
            return false;
        }
        true
    }

    /// Why the tool is unavailable, for the card's disabled hover text.
    pub fn unavailable_reason(self, caps: &super::bridge::Capabilities) -> Option<&'static str> {
        if !caps.has_document {
            return Some("open a document first");
        }
        if (self.needs_selection() || self.writes_selection()) && !caps.selection {
            return Some("pixel selections are not available in this mode");
        }
        if self.writes_layer() && !caps.place_layer {
            return Some("this mode cannot take an image layer");
        }
        if self.grows_canvas() && !caps.expand {
            return Some("expanding the canvas is not available in this mode");
        }
        if self.needs_selection() && !caps.has_selection {
            return Some("select the area first");
        }
        None
    }
}

/// Everything a job needs, gathered on the UI thread before it starts.
#[derive(Clone, Debug, PartialEq)]
pub enum JobInput {
    Erase {
        image: Rgba8Image,
        mask: Mask,
    },
    Inpaint {
        image: Rgba8Image,
        mask: Mask,
        options: InpaintOptions,
    },
    Outpaint {
        image: Rgba8Image,
        expand: Expand,
        options: InpaintOptions,
    },
    /// `plugin` is RemoveBG / AnimeSeg / RealESRGAN / GFPGAN / RestoreFormer; `scale` is for
    /// RealESRGAN.
    PluginImage {
        plugin: &'static str,
        image: Rgba8Image,
        scale: f64,
        layer_name: String,
    },
    SelectSubject {
        plugin: &'static str,
        image: Rgba8Image,
    },
    Vectorize {
        image: Rgba8Image,
        preset: String,
    },
}

/// What a finished job hands back to the UI thread, to apply through the bridge.
#[derive(Clone, Debug, PartialEq)]
pub enum JobOutput {
    /// A new layer at `offset`.
    Layer {
        name: String,
        image: Rgba8Image,
        offset: (i32, i32),
    },
    /// A new layer after growing the canvas by `expand` (the image is the padded size).
    Expanded {
        name: String,
        image: Rgba8Image,
        expand: Expand,
    },
    Selection(Mask),
    Trace {
        preset: String,
        result: TraceResult,
        /// The traced image's size, for the Vector document that may have to be created.
        size: (u32, u32),
    },
}

/// A job's result, delivered through the queue.
pub struct JobResult {
    pub kind: ToolKind,
    pub output: Result<JobOutput, String>,
    pub elapsed: Duration,
    /// The agent's pending tool call, when the job was started by one.
    pub reply: Option<ToolReply>,
}

struct Running {
    kind: ToolKind,
    started: Instant,
    reply: Option<ToolReply>,
}

/// Where the queue is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
pub enum JobState {
    Idle,
    Running(ToolKind),
}

/// The one-at-a-time queue: `start` refuses while a job runs; the job sends its result through
/// the [`JobSender`]; `poll` takes finished results and frees the queue.
pub struct JobQueue {
    tx: mpsc::Sender<(ToolKind, Result<JobOutput, String>)>,
    rx: mpsc::Receiver<(ToolKind, Result<JobOutput, String>)>,
    running: Option<Running>,
}

/// Hands one job's result back (clone of the queue's sender).
#[derive(Clone)]
pub struct JobSender {
    kind: ToolKind,
    tx: mpsc::Sender<(ToolKind, Result<JobOutput, String>)>,
}

impl JobSender {
    pub fn send(&self, output: Result<JobOutput, String>) {
        let _ = self.tx.send((self.kind, output));
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            tx,
            rx,
            running: None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))] // exercised by the unit tests
    pub fn state(&self) -> JobState {
        match &self.running {
            Some(r) => JobState::Running(r.kind),
            None => JobState::Idle,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.running.is_some()
    }

    /// How long the running job has been running.
    pub fn elapsed(&self) -> Option<Duration> {
        self.running.as_ref().map(|r| r.started.elapsed())
    }

    /// Claims the queue for `kind`; `Err` names the job still running.
    pub fn start(
        &mut self,
        kind: ToolKind,
        reply: Option<ToolReply>,
    ) -> Result<JobSender, ToolKind> {
        if let Some(r) = &self.running {
            return Err(r.kind);
        }
        self.running = Some(Running {
            kind,
            started: Instant::now(),
            reply,
        });
        Ok(JobSender {
            kind,
            tx: self.tx.clone(),
        })
    }

    /// The finished job, if its result arrived. Frees the queue.
    pub fn poll(&mut self) -> Option<JobResult> {
        let (kind, output) = self.rx.try_recv().ok()?;
        let running = self.running.take();
        let (elapsed, reply) = match running {
            Some(r) if r.kind == kind => (r.started.elapsed(), r.reply),
            Some(r) => {
                // A stale result from a job the queue no longer tracks: keep the queue claimed.
                self.running = Some(r);
                return None;
            }
            None => (Duration::ZERO, None),
        };
        Some(JobResult {
            kind,
            output,
            elapsed,
            reply,
        })
    }

    /// Drops the running job's claim (its result, if it ever arrives, is ignored as stale).
    pub fn cancel(&mut self) -> Option<ToolReply> {
        self.running.take().and_then(|r| r.reply)
    }
}

/// Runs `input` against `engine`, returning what to apply. Async: call on the runtime.
pub async fn run_job(engine: Option<Engine>, input: JobInput) -> Result<JobOutput, String> {
    match input {
        JobInput::Vectorize { image, preset } => {
            let size = (image.width, image.height);
            let result =
                redrob_ai::vectorize::vectorize(&image, &preset).map_err(|e| e.to_string())?;
            Ok(JobOutput::Trace {
                preset,
                result,
                size,
            })
        }
        other => {
            let engine = engine.ok_or_else(|| "the AI engine has no data directory".to_string())?;
            run_engine_job(&engine, other)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

async fn run_engine_job(engine: &Engine, input: JobInput) -> redrob_ai::Result<JobOutput> {
    match input {
        JobInput::Erase { image, mask } => {
            let options = InpaintOptions::default();
            let result = engine
                .request("Erase", "Erasing the selection…", |client| async move {
                    client.inpaint(&image, &mask, &options).await
                })
                .await?;
            Ok(JobOutput::Layer {
                name: "AI erase".into(),
                image: result,
                offset: (0, 0),
            })
        }
        JobInput::Inpaint {
            image,
            mask,
            options,
        } => {
            let result = engine
                .request(
                    "Inpaint",
                    "Painting the selection from the prompt…",
                    |client| async move { client.inpaint(&image, &mask, &options).await },
                )
                .await?;
            Ok(JobOutput::Layer {
                name: "AI inpaint".into(),
                image: result,
                offset: (0, 0),
            })
        }
        JobInput::Outpaint {
            image,
            expand,
            options,
        } => {
            let result = engine
                .request("Outpaint", "Expanding the picture…", |client| async move {
                    client.outpaint(&image, expand, &options).await
                })
                .await?;
            Ok(JobOutput::Expanded {
                name: "AI expand".into(),
                image: result,
                expand,
            })
        }
        JobInput::PluginImage {
            plugin,
            image,
            scale,
            layer_name,
        } => {
            let busy = format!("Running {plugin}…");
            let result = engine
                .request(plugin, busy, |client| async move {
                    client.plugin_gen_image(plugin, &image, scale).await
                })
                .await?;
            Ok(JobOutput::Layer {
                name: layer_name,
                image: result,
                offset: (0, 0),
            })
        }
        JobInput::SelectSubject { plugin, image } => {
            let mask = engine
                .request(
                    "Select subject",
                    "Finding the subject…",
                    |client| async move { client.plugin_gen_mask(plugin, &image, &[]).await },
                )
                .await?;
            Ok(JobOutput::Selection(mask))
        }
        JobInput::Vectorize { .. } => Err(redrob_ai::AiError::Geometry(
            "Vectorize does not use the engine".into(),
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::ai::bridge::Capabilities;
    use crate::shell::Mode;

    #[test]
    fn one_job_at_a_time_and_results_free_the_queue() {
        let mut queue = JobQueue::new();
        assert_eq!(queue.state(), JobState::Idle);
        assert!(queue.poll().is_none());
        let sender = queue.start(ToolKind::Erase, None).unwrap();
        assert_eq!(queue.state(), JobState::Running(ToolKind::Erase));
        assert!(queue.is_busy());
        // A second job is refused and names the one running.
        assert_eq!(
            queue.start(ToolKind::Upscale, None).err(),
            Some(ToolKind::Erase)
        );
        // Nothing arrived yet.
        assert!(queue.poll().is_none());
        assert!(queue.is_busy());
        sender.send(Ok(JobOutput::Selection(Mask::filled(1, 1, 255).unwrap())));
        let result = queue.poll().unwrap();
        assert_eq!(result.kind, ToolKind::Erase);
        assert!(matches!(result.output, Ok(JobOutput::Selection(_))));
        assert!(result.reply.is_none());
        assert_eq!(queue.state(), JobState::Idle);
        // Free again: the next job starts, and an error result is delivered the same way.
        let sender = queue.start(ToolKind::Vectorize, None).unwrap();
        sender.send(Err("boom".into()));
        let result = queue.poll().unwrap();
        assert_eq!(result.output, Err("boom".to_string()));
        assert!(!queue.is_busy());
    }

    #[test]
    fn a_cancelled_jobs_late_result_is_ignored() {
        let mut queue = JobQueue::new();
        let stale = queue.start(ToolKind::Upscale, None).unwrap();
        assert!(queue.cancel().is_none());
        assert_eq!(queue.state(), JobState::Idle);
        let fresh = queue.start(ToolKind::Erase, None).unwrap();
        stale.send(Err("too late".into()));
        // The stale result does not free the running job.
        assert!(queue.poll().is_none());
        assert_eq!(queue.state(), JobState::Running(ToolKind::Erase));
        fresh.send(Err("done".into()));
        assert_eq!(queue.poll().unwrap().output, Err("done".to_string()));
    }

    #[test]
    fn tool_availability_per_mode() {
        let photo = Capabilities::photo(true, true);
        let photo_no_sel = Capabilities::photo(true, false);
        let vector = Capabilities::vector(true);
        let layout = Capabilities::layout(true);
        let empty = Capabilities::none(Mode::Photo);
        for kind in ToolKind::ALL {
            assert!(kind.available(&photo), "{kind:?} in Photo");
            assert!(!kind.available(&empty), "{kind:?} without a document");
        }
        // Erase / Inpaint are enabled without a selection (the card says to select first).
        assert!(ToolKind::Erase.available(&photo_no_sel));
        assert_eq!(
            ToolKind::Erase.unavailable_reason(&photo_no_sel),
            Some("select the area first")
        );
        for caps in [&vector, &layout] {
            assert!(!ToolKind::Erase.available(caps));
            assert!(!ToolKind::Inpaint.available(caps));
            assert!(!ToolKind::Outpaint.available(caps));
            assert!(!ToolKind::SelectSubject.available(caps));
            assert!(ToolKind::RemoveBackground.available(caps));
            assert!(ToolKind::Upscale.available(caps));
            assert!(ToolKind::FaceRestore.available(caps));
            assert!(ToolKind::Vectorize.available(caps));
        }
        assert_eq!(
            ToolKind::from_tool_name("remove_background"),
            Some(ToolKind::RemoveBackground)
        );
        assert_eq!(ToolKind::from_tool_name("nope"), None);
        assert!(!ToolKind::Vectorize.needs_engine());
    }
}
