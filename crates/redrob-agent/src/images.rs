// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Redrob console's image models, through the same daemon and sign-in as chat.
//!
//! The console serves the OpenAI Images API (`POST /v1/images/generations` JSON and
//! `POST /v1/images/edits` multipart, `b64_json` answers) and lists its image models in
//! `GET /v1/models` with `kind: "image"`. These are the typed request and answer shapes; the
//! calls themselves are on [`crate::RedrobClient`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One `/v1/models` entry with `kind: "image"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageModel {
    /// `redrob/flux2-klein-4b`, `redrob/lama`, ...
    pub id: String,
    /// The console's display label, when it sends one.
    pub label: Option<String>,
    /// `generations` and / or `edits`.
    pub endpoints: Vec<String>,
    /// Whether an edit with this model must carry a mask (an erase model such as LaMa).
    pub mask_required: bool,
    /// USD per output megapixel, when the console states it.
    pub price_per_megapixel_usd: Option<f64>,
    pub description: Option<String>,
}

impl ImageModel {
    pub fn serves_generations(&self) -> bool {
        self.endpoints.iter().any(|e| e == "generations")
    }

    pub fn serves_edits(&self) -> bool {
        self.endpoints.iter().any(|e| e == "edits")
    }

    /// The label to show: the console's, else the id without its `redrob/` prefix.
    pub fn display_name(&self) -> &str {
        match &self.label {
            Some(label) if !label.trim().is_empty() => label,
            _ => self.id.rsplit('/').next().unwrap_or(&self.id),
        }
    }

    /// Reads the image entries of a `/v1/models` body (`data` or `models`); chat entries and
    /// entries without an id are skipped.
    pub fn from_models_list(list: &Value) -> Vec<Self> {
        let entries = list
            .get("data")
            .or_else(|| list.get("models"))
            .and_then(Value::as_array);
        let Some(entries) = entries else {
            return Vec::new();
        };
        entries
            .iter()
            .filter(|entry| entry.get("kind").and_then(Value::as_str) == Some("image"))
            .filter_map(|entry| {
                let id = entry.get("id").and_then(Value::as_str)?.trim();
                if id.is_empty() {
                    return None;
                }
                let image = entry.get("image");
                let endpoints = image
                    .and_then(|i| i.get("endpoints"))
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Self {
                    id: id.to_owned(),
                    label: entry
                        .get("label")
                        .or_else(|| entry.get("name"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    endpoints,
                    mask_required: image
                        .and_then(|i| i.get("maskRequired"))
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    price_per_megapixel_usd: image
                        .and_then(|i| i.get("pricePerMegapixelUsd"))
                        .and_then(Value::as_f64),
                    description: image
                        .and_then(|i| i.get("description"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect()
    }
}

/// `POST /v1/images/generations`.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageGenerationRequest {
    pub model: String,
    pub prompt: String,
    /// Output size; each side a multiple of 16 in 256..=2048, at most 2.1 MP.
    pub width: u32,
    pub height: u32,
    pub seed: Option<i64>,
}

/// `POST /v1/images/edits`. The mask follows OpenAI's convention: fully transparent pixels are
/// the area to change, opaque ones are kept.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageEditRequest {
    pub model: String,
    pub prompt: Option<String>,
    /// PNG bytes.
    pub image_png: Vec<u8>,
    /// PNG bytes with an alpha channel.
    pub mask_png: Option<Vec<u8>>,
    /// Output size; `None` is the input size rounded down to a multiple of 16.
    pub size: Option<(u32, u32)>,
    pub seed: Option<i64>,
}

/// An Images API answer: the images still base64-encoded, as the console sent them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageResponse {
    pub b64_images: Vec<String>,
    /// Output megapixels billed, when the console reports it.
    pub megapixels: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct ImageGenerationWire<'a> {
    pub model: &'a str,
    pub prompt: &'a str,
    pub size: String,
    pub n: u32,
    pub response_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

impl<'a> From<&'a ImageGenerationRequest> for ImageGenerationWire<'a> {
    fn from(request: &'a ImageGenerationRequest) -> Self {
        Self {
            model: &request.model,
            prompt: &request.prompt,
            size: format!("{}x{}", request.width, request.height),
            n: 1,
            response_format: "b64_json",
            seed: request.seed,
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ImageResponseWire {
    #[serde(default)]
    data: Vec<ImageDatumWire>,
    #[serde(default)]
    usage: Option<ImageUsageWire>,
}

#[derive(Deserialize)]
struct ImageDatumWire {
    #[serde(default)]
    b64_json: Option<String>,
}

#[derive(Deserialize)]
struct ImageUsageWire {
    #[serde(default)]
    megapixels: Option<f64>,
}

impl From<ImageResponseWire> for ImageResponse {
    fn from(wire: ImageResponseWire) -> Self {
        Self {
            b64_images: wire
                .data
                .into_iter()
                .filter_map(|d| d.b64_json)
                .filter(|b| !b.is_empty())
                .collect(),
            megapixels: wire.usage.and_then(|u| u.megapixels),
        }
    }
}

/// A `multipart/form-data` body for an edit, built by hand so the HTTP client needs no extra
/// feature. Returns the body and its boundary.
pub(crate) fn edit_multipart(request: &ImageEditRequest) -> (Vec<u8>, String) {
    // A per-request boundary; parsers look for CRLF--boundary, which a prompt or PNG bytes do
    // not realistically contain.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let boundary = format!("----redrob-canvas-{nanos:x}-{:x}", request.image_png.len());
    let mut body = Vec::with_capacity(
        request.image_png.len() + request.mask_png.as_ref().map_or(0, Vec::len) + 1024,
    );
    let text = |body: &mut Vec<u8>, name: &str, value: &str| {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    };
    text(&mut body, "model", &request.model);
    if let Some(prompt) = request.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
        text(&mut body, "prompt", prompt);
    }
    if let Some((w, h)) = request.size {
        text(&mut body, "size", &format!("{w}x{h}"));
    }
    text(&mut body, "n", "1");
    text(&mut body, "response_format", "b64_json");
    if let Some(seed) = request.seed {
        text(&mut body, "seed", &seed.to_string());
    }
    let file = |body: &mut Vec<u8>, name: &str, bytes: &[u8]| {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{name}.png\"\r\n\
                 Content-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    };
    file(&mut body, "image", &request.image_png);
    if let Some(mask) = &request.mask_png {
        file(&mut body, "mask", mask);
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (body, boundary)
}
