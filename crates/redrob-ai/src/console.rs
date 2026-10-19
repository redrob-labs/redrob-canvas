// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Redrob Console image models, the second backend beside the local IOPaint engine.
//!
//! The console serves the OpenAI Images API on Redrob's own GPU box; the calls go through the
//! shared Redrob Code daemon with the user's existing console sign-in (the same path the mode
//! agent's chat takes), so nothing here holds a key. Each operation maps onto one request:
//!
//! - Generative fill: `edits` with a mask whose selected pixels are transparent.
//! - Expand canvas: `edits` over the enlarged canvas, whose new border is transparent in both
//!   the image and the mask (the console has no separate outpaint route).
//! - Erase: `edits` with `redrob/lama` and the selection mask; no prompt.
//! - Generate: `generations` from a prompt.
//!
//! The box takes sides that are multiples of 16 in 256..=2048 and at most 2.1 MP, and an upload
//! of at most 4 MB, so every request is scaled into those bounds and the answer scaled back to
//! the pixels it replaces.

use std::time::Duration;

use base64::Engine as _;
use image::{RgbaImage, imageops::FilterType};
/// The daemon client's error, which [`AiError::Agent`] carries: the shell branches on its
/// `InsufficientCredit` (402) and `Busy` (429) variants.
pub use redrob_agent::{Error as AgentError, ImageModel};
use redrob_agent::{
    EngineConfig, ImageEditRequest, ImageGenerationRequest, ImageResponse, RedrobClient,
};

use crate::{AiError, Expand, Mask, Result, Rgba8Image, pixels::MAX_EXPAND, region};

/// The erase model (prompt ignored, mask required).
pub const ERASE_MODEL: &str = "redrob/lama";
/// The model fill and expand pick first when the console lists it.
pub const PREFERRED_EDIT_MODEL: &str = "redrob/flux2-klein-4b";
/// The model Generate picks first when the console lists it.
pub const PREFERRED_GENERATE_MODEL: &str = "redrob/flux2-klein-4b";

/// The box's output bounds.
pub const MIN_SIDE: u32 = 256;
pub const MAX_SIDE: u32 = 2048;
pub const SIDE_STEP: u32 = 16;
pub const MAX_OUTPUT_PIXELS: u64 = 2_100_000;
/// What an edit sends at most: the size the box is measured at, which keeps a photo's PNG and
/// mask well under the upload cap.
pub const MAX_EDIT_PIXELS: u64 = 1024 * 1024;
/// The console's cap on image + mask bytes.
pub const MAX_UPLOAD_BYTES: usize = 4_000_000;

/// How long one image request may take (the box queues; a cold model swap takes a while).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Which console operation a model list is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Fill,
    Expand,
    Erase,
    Generate,
}

impl Operation {
    /// Whether `model` can serve this operation.
    pub fn accepts(self, model: &ImageModel) -> bool {
        match self {
            // A prompted edit; an erase-only model (mask required, prompt ignored) would drop
            // the prompt, so it is left to Erase.
            Operation::Fill | Operation::Expand => model.serves_edits() && !model.mask_required,
            Operation::Erase => model.serves_edits(),
            Operation::Generate => model.serves_generations(),
        }
    }

    fn preferred(self) -> &'static str {
        match self {
            Operation::Fill | Operation::Expand => PREFERRED_EDIT_MODEL,
            Operation::Erase => ERASE_MODEL,
            Operation::Generate => PREFERRED_GENERATE_MODEL,
        }
    }

    /// The models `models` offers for this operation, in the console's order.
    pub fn models(self, models: &[ImageModel]) -> Vec<ImageModel> {
        let mut list: Vec<ImageModel> = models
            .iter()
            .filter(|m| self.accepts(m))
            .cloned()
            .collect();
        if self == Operation::Erase {
            // Erase models first: they are what Erase is for.
            list.sort_by_key(|m| !m.mask_required);
        }
        list
    }

    /// The model to preselect: the preferred one when listed, else the first that fits.
    pub fn default_model(self, models: &[ImageModel]) -> Option<String> {
        let list = self.models(models);
        list.iter()
            .find(|m| m.id == self.preferred())
            .or_else(|| list.first())
            .map(|m| m.id.clone())
    }
}

/// The console image client. Cheap to clone; takes no network action until a call.
#[derive(Clone, Debug)]
pub struct ConsoleImages {
    client: RedrobClient,
}

impl ConsoleImages {
    /// A client that labels its requests as `app` (`x-redrob-app`).
    pub fn new(app: &str) -> Result<Self> {
        let config = EngineConfig::new(app)?.with_request_timeout(REQUEST_TIMEOUT);
        Ok(Self {
            client: RedrobClient::new(config)?,
        })
    }

    /// The image models the console lists (empty when it serves none). Fails when the daemon is
    /// not running or not signed in.
    pub async fn models(&self) -> Result<Vec<ImageModel>> {
        Ok(self.client.image_models().await?)
    }

    /// Generative fill (or, with an erase model and no prompt, erase) of the selection. The
    /// result covers the region the request carried, clipped to the selection, at its offset.
    pub async fn fill(
        &self,
        model: &str,
        image: &Rgba8Image,
        mask: &Mask,
        prompt: Option<&str>,
        seed: Option<i64>,
    ) -> Result<(Rgba8Image, (u32, u32))> {
        if mask.width != image.width || mask.height != image.height {
            return Err(AiError::Geometry(
                "AI fill: the mask does not match the image".into(),
            ));
        }
        let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());
        let margin = if prompt.is_some() {
            region::ROI_MARGIN_PROMPTED
        } else {
            region::ROI_MARGIN
        };
        let roi = region::inpaint_region(mask, margin)
            .ok_or_else(|| AiError::Geometry("AI fill: the selection is empty".into()))?;
        let (sub_image, sub_mask) = if roi == image.rect() {
            (image.clone(), mask.clone())
        } else {
            (image.crop(roi)?, mask.crop(roi)?)
        };
        let opaque = Rgba8Image::from_rgb(
            sub_image.width,
            sub_image.height,
            &sub_image.flatten_over_white(),
        )?;
        let edit_mask = openai_mask(&sub_mask);
        let mut result = self
            .edit_scaled(model, prompt, &opaque, &edit_mask, seed)
            .await?;
        result.clip_alpha(&sub_mask)?;
        Ok((result, (roi.x, roi.y)))
    }

    /// Expand canvas: the picture inside a canvas grown by `expand`, the new border transparent
    /// in image and mask. The result is the grown size with alpha only on the border, like the
    /// local outpaint's.
    pub async fn expand(
        &self,
        model: &str,
        image: &Rgba8Image,
        expand: Expand,
        prompt: Option<&str>,
        seed: Option<i64>,
    ) -> Result<Rgba8Image> {
        if !expand.is_valid() {
            return Err(AiError::Geometry(format!(
                "give 1 to {MAX_EXPAND} pixels on at least one side"
            )));
        }
        let width = image
            .width
            .checked_add(expand.left)
            .and_then(|w| w.checked_add(expand.right))
            .ok_or_else(|| AiError::Geometry("expanded width overflows".into()))?;
        let height = image
            .height
            .checked_add(expand.top)
            .and_then(|h| h.checked_add(expand.bottom))
            .ok_or_else(|| AiError::Geometry("expanded height overflows".into()))?;
        let opaque =
            Rgba8Image::from_rgb(image.width, image.height, &image.flatten_over_white())?;
        let mut canvas = Rgba8Image::transparent(width, height)?;
        canvas.blit(&opaque, expand.left, expand.top)?;
        // 255 = fill (the new border), 0 = keep (the picture).
        let mut border = Mask::filled(width, height, 255)?;
        border.fill_rect(
            crate::Rect::new(expand.left, expand.top, image.width, image.height),
            0,
        );
        let edit_mask = openai_mask(&border);
        let prompt = prompt.map(str::trim).filter(|p| !p.is_empty());
        let mut result = self
            .edit_scaled(model, prompt, &canvas, &edit_mask, seed)
            .await?;
        result.clip_alpha(&border)?;
        Ok(result)
    }

    /// Generate: one `width × height` image from `prompt`, rendered at the nearest size the box
    /// takes and scaled to the asked size.
    pub async fn generate(
        &self,
        model: &str,
        prompt: &str,
        width: u32,
        height: u32,
        seed: Option<i64>,
    ) -> Result<Rgba8Image> {
        if width == 0 || height == 0 {
            return Err(AiError::Geometry("the image has no size".into()));
        }
        let (tw, th) = fit_size(width, height, MAX_OUTPUT_PIXELS);
        let answer = self
            .client
            .generate_image(&ImageGenerationRequest {
                model: model.to_owned(),
                prompt: prompt.trim().to_owned(),
                width: tw,
                height: th,
                seed,
            })
            .await?;
        let image = first_image(&answer)?;
        resize(&image, width, height, FilterType::Lanczos3)
    }

    /// One edit of `image` with the OpenAI-convention `mask` (same size), scaled into the box's
    /// bounds and the upload cap, the answer scaled back to `image`'s size.
    async fn edit_scaled(
        &self,
        model: &str,
        prompt: Option<&str>,
        image: &Rgba8Image,
        mask: &Rgba8Image,
        seed: Option<i64>,
    ) -> Result<Rgba8Image> {
        let (width, height) = (image.width, image.height);
        let mut budget = MAX_EDIT_PIXELS;
        let floor = u64::from(MIN_SIDE) * u64::from(MIN_SIDE);
        let (image_png, mask_png, size) = loop {
            let (tw, th) = fit_size(width, height, budget);
            let scaled_image = resize(image, tw, th, FilterType::Lanczos3)?;
            // Nearest keeps the mask's alpha exactly 0 or 255.
            let scaled_mask = resize(mask, tw, th, FilterType::Nearest)?;
            let image_png = scaled_image.to_png()?;
            let mask_png = scaled_mask.to_png()?;
            if image_png.len() + mask_png.len() <= MAX_UPLOAD_BYTES || budget <= floor {
                break (image_png, mask_png, (tw, th));
            }
            budget = (budget * 3 / 5).max(floor);
        };
        let answer = self
            .client
            .edit_image(&ImageEditRequest {
                model: model.to_owned(),
                prompt: prompt.map(str::to_owned),
                image_png,
                mask_png: Some(mask_png),
                size: Some(size),
                seed,
            })
            .await?;
        let result = first_image(&answer)?;
        resize(&result, width, height, FilterType::Lanczos3)
    }
}

/// A selection (255 = change) as an OpenAI edit mask: fully transparent where the selection has
/// any coverage (the area to change), opaque black elsewhere (kept).
pub fn openai_mask(selection: &Mask) -> Rgba8Image {
    let mut data = Vec::with_capacity(selection.data.len() * 4);
    for &m in &selection.data {
        if m > 0 {
            data.extend_from_slice(&[255, 255, 255, 0]);
        } else {
            data.extend_from_slice(&[0, 0, 0, 255]);
        }
    }
    Rgba8Image {
        width: selection.width,
        height: selection.height,
        data,
    }
}

/// The size the box renders a `width × height` request at: aspect kept as well as multiples of
/// [`SIDE_STEP`] allow, each side in [`MIN_SIDE`]..=[`MAX_SIDE`], at most `max_pixels` (and never
/// more than [`MAX_OUTPUT_PIXELS`]).
pub fn fit_size(width: u32, height: u32, max_pixels: u64) -> (u32, u32) {
    let max_pixels = max_pixels.min(MAX_OUTPUT_PIXELS);
    let w = f64::from(width.max(1));
    let h = f64::from(height.max(1));
    let mut scale = (max_pixels as f64 / (w * h)).sqrt();
    scale = scale.min(f64::from(MAX_SIDE) / w.max(h));
    scale = scale.max(f64::from(MIN_SIDE) / w.min(h));
    let side = |v: f64| -> u32 {
        let steps = (v * scale / f64::from(SIDE_STEP)).round().max(1.0) as u32;
        steps.saturating_mul(SIDE_STEP).clamp(MIN_SIDE, MAX_SIDE)
    };
    let (mut out_w, mut out_h) = (side(w), side(h));
    // The side clamps can push an extreme aspect over the cap: trim the longer side.
    while u64::from(out_w) * u64::from(out_h) > max_pixels.max(u64::from(MIN_SIDE).pow(2)) {
        if out_w >= out_h && out_w > MIN_SIDE {
            out_w -= SIDE_STEP;
        } else if out_h > MIN_SIDE {
            out_h -= SIDE_STEP;
        } else {
            break;
        }
    }
    (out_w, out_h)
}

/// `image` resampled to `width × height` (itself when it already is that size).
fn resize(image: &Rgba8Image, width: u32, height: u32, filter: FilterType) -> Result<Rgba8Image> {
    if image.width == width && image.height == height {
        return Ok(image.clone());
    }
    let source = RgbaImage::from_raw(image.width, image.height, image.data.clone())
        .ok_or_else(|| AiError::Png("RGBA buffer does not match its size".into()))?;
    let scaled = image::imageops::resize(&source, width, height, filter);
    Rgba8Image::new(width, height, scaled.into_raw())
}

/// The answer's first image, decoded (PNG, or whatever format the bytes are).
fn first_image(answer: &ImageResponse) -> Result<Rgba8Image> {
    let encoded = answer
        .b64_images
        .first()
        .ok_or_else(|| AiError::Png("the image answer carried no image".into()))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|e| AiError::Png(e.to_string()))?;
    let decoded = image::load_from_memory(&bytes)
        .map_err(|e| AiError::Png(e.to_string()))?
        .into_rgba8();
    Rgba8Image::new(decoded.width(), decoded.height(), decoded.into_raw())
}
