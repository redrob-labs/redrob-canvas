// SPDX-License-Identifier: MIT OR Apache-2.0

//! The IOPaint 1.6.0 HTTP API (`/api/v1/...`) over reqwest.
//!
//! Images travel as PNG data URLs (`data:image/png;base64,...`); a 200 answer is raw PNG bytes,
//! anything else is JSON with a `detail` (or `errors`) field, surfaced as [`AiError::Server`].
//! Diffusion on a CPU can take minutes, so requests have no transfer timeout.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    AiError, Result,
    pixels::{Expand, Mask, Rgba8Image, pad_for_outpaint},
};

/// The fields of IOPaint's `InpaintRequest` an application sets. Every field is optional and
/// serialises under IOPaint's own key, so a request carries only what was chosen; the server's
/// defaults fill in the rest. Erase models ignore all of them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InpaintOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_strength: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_guidance_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_seed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_sampler: Option<String>,
    /// Keep everything outside the mask (default true on the server). When true the result layer
    /// is clipped to the mask by [`Client::inpaint`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_keep_unmasked_area: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_match_histograms: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sd_lcm_lora: Option<bool>,
    /// PowerPaint only: one of [`super::models::POWERPAINT_TASKS`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub powerpaint_task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_controlnet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub controlnet_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_brushnet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brushnet_method: Option<String>,
    /// Paint by Example: the example image, sent as `paint_by_example_example_image`.
    #[serde(skip)]
    pub paint_by_example_image: Option<Rgba8Image>,
}

impl InpaintOptions {
    /// Whether a prompt was given (an "AI replace" rather than an "AI erase").
    pub fn is_prompted(&self) -> bool {
        self.prompt.as_deref().is_some_and(|p| !p.trim().is_empty())
    }

    /// Whether the result should be clipped to the mask (`sd_keep_unmasked_area`, default true).
    pub fn keeps_unmasked_area(&self) -> bool {
        self.sd_keep_unmasked_area.unwrap_or(true)
    }

    /// The JSON body fields these options contribute (without `image` and `mask`).
    pub fn to_body(&self) -> Result<Map<String, Value>> {
        let mut body = match serde_json::to_value(self)? {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        if let Some(example) = &self.paint_by_example_image {
            body.insert(
                "paint_by_example_example_image".into(),
                Value::String(example.to_opaque_data_url()?),
            );
        }
        Ok(body)
    }
}

/// A plugin's current model and the models it can switch to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginModels {
    pub current: String,
    pub choices: Vec<String>,
}

/// What `GET /api/v1/server-config` reports, reduced to what the panels need.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServerConfig {
    /// Downloaded models by name; each value is IOPaint's `ModelInfo` object (`need_prompt`,
    /// `support_controlnet`, `controlnets`, `support_brushnet`, `brushnets`, `support_lcm_lora`...).
    pub model_infos: BTreeMap<String, Value>,
    /// Names of the plugins the server was started with.
    pub plugins: Vec<String>,
    /// Current and available models for RemoveBG, RealESRGAN and InteractiveSeg.
    pub plugin_models: BTreeMap<String, PluginModels>,
    pub samplers: Vec<String>,
}

impl ServerConfig {
    /// Reads IOPaint's server-config JSON the way the Qt engine's `readConfig()` did.
    pub fn from_json(config: &Value) -> Self {
        let strings = |key: &str| -> Vec<String> {
            config[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let model_infos = config["modelInfos"]
            .as_array()
            .map(|infos| {
                infos
                    .iter()
                    .filter_map(|info| Some((info["name"].as_str()?.to_owned(), info.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let plugins = config["plugins"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v["name"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let choice = |current: &str, all: &str| PluginModels {
            current: config[current].as_str().unwrap_or_default().to_owned(),
            choices: strings(all),
        };
        let plugin_models = [
            ("RemoveBG", choice("removeBGModel", "removeBGModels")),
            ("RealESRGAN", choice("realesrganModel", "realesrganModels")),
            (
                "InteractiveSeg",
                choice("interactiveSegModel", "interactiveSegModels"),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        Self {
            model_infos,
            plugins,
            plugin_models,
            samplers: strings("samplers"),
        }
    }

    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| p == name)
    }

    pub fn has_model(&self, name: &str) -> bool {
        self.model_infos.contains_key(name)
    }

    pub fn downloaded_models(&self) -> Vec<String> {
        self.model_infos.keys().cloned().collect()
    }
}

/// A click for InteractiveSeg: canvas pixel and whether it is a point to include (`true`) or to
/// leave out (`false`).
pub type Click = (i32, i32, bool);

/// `[[x, y, 1|0], ...]`, the `clicks` field of a `run_plugin_gen_mask` body.
pub fn encode_clicks(clicks: &[Click]) -> Value {
    Value::Array(
        clicks
            .iter()
            .map(|&(x, y, include)| json!([x, y, i32::from(include)]))
            .collect(),
    )
}

/// The user-facing layer name for a gen-image plugin.
pub fn plugin_layer_name(plugin: &str) -> &str {
    match plugin {
        "RemoveBG" => "Remove background",
        "AnimeSeg" => "Anime cut-out",
        "RealESRGAN" => "AI upscale",
        "GFPGAN" => "Face restore (GFPGAN)",
        "RestoreFormer" => "Face restore (RestoreFormer)",
        other => other,
    }
}

/// An upscale result may not pass this many pixels (64 Mi-pixels).
pub const MAX_UPSCALE_PIXELS: u64 = 64 * 1024 * 1024;

/// Client for one running IOPaint server.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
}

impl Client {
    /// `base_url` like `http://127.0.0.1:8080` (no trailing slash needed).
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        // No timeout: diffusion on a CPU can take minutes.
        let http = reqwest::Client::builder().build()?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        })
    }

    /// A client for the server on loopback port `port`.
    pub fn for_port(port: u16) -> Result<Self> {
        Self::new(format!("http://127.0.0.1:{port}"))
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn api(&self, path: &str) -> String {
        format!("{}/api/v1/{}", self.base_url, path.trim_start_matches('/'))
    }

    /// `GET /api/v1/server-config`; also the readiness probe.
    pub async fn server_config(&self) -> Result<ServerConfig> {
        let response = self.http.get(self.api("server-config")).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(server_error(status.as_u16(), &bytes));
        }
        let value: Value = serde_json::from_slice(&bytes)?;
        Ok(ServerConfig::from_json(&value))
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Vec<u8>> {
        let response = self.http.post(self.api(path)).json(body).send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if status.as_u16() != 200 {
            return Err(server_error(status.as_u16(), &bytes));
        }
        Ok(bytes.to_vec())
    }

    async fn post_png(&self, path: &str, body: &Value) -> Result<Rgba8Image> {
        let bytes = self.post(path, body).await?;
        if bytes.is_empty() {
            return Err(AiError::Png("the AI engine returned no image".into()));
        }
        Rgba8Image::from_png(&bytes)
    }

    /// `POST /api/v1/model {name}`: switch to a downloaded model in place.
    pub async fn set_model(&self, name: &str) -> Result<()> {
        self.post("model", &json!({ "name": name })).await.map(drop)
    }

    /// `POST /api/v1/switch_plugin_model {plugin_name, model_name}`.
    pub async fn switch_plugin_model(&self, plugin: &str, model: &str) -> Result<()> {
        self.post(
            "switch_plugin_model",
            &json!({ "plugin_name": plugin, "model_name": model }),
        )
        .await
        .map(drop)
    }

    /// `POST /api/v1/inpaint`: erase (no prompt) or replace (prompt) inside `mask` with the
    /// current model. `image` is sent opaque (alpha over white); when `options` keep the unmasked
    /// area the result's alpha is `mask`, so it lands as a layer that only changes the selection.
    pub async fn inpaint(
        &self,
        image: &Rgba8Image,
        mask: &Mask,
        options: &InpaintOptions,
    ) -> Result<Rgba8Image> {
        if mask.width != image.width || mask.height != image.height {
            return Err(AiError::Geometry(
                "AI fill: the mask does not match the image".into(),
            ));
        }
        let mut body = options.to_body()?;
        body.insert("image".into(), Value::String(image.to_opaque_data_url()?));
        body.insert("mask".into(), Value::String(mask.to_data_url()?));
        let mut result = self.post_png("inpaint", &Value::Object(body)).await?;
        if result.width != image.width || result.height != image.height {
            return Err(AiError::Geometry(format!(
                "AI fill: the result is {} × {}, the image {} × {}",
                result.width, result.height, image.width, image.height
            )));
        }
        if options.keeps_unmasked_area() {
            result.clip_alpha(mask)?;
        }
        Ok(result)
    }

    /// Outpaint: pad the canvas by `expand`, mask the new border and inpaint it. The result is
    /// the padded size with alpha only on the border (the shell pads its own canvas by `expand`
    /// and lands the result as a layer).
    pub async fn outpaint(
        &self,
        image: &Rgba8Image,
        expand: Expand,
        options: &InpaintOptions,
    ) -> Result<Rgba8Image> {
        let (padded, border) = pad_for_outpaint(image, expand)?;
        let mut body = options.to_body()?;
        body.insert("image".into(), Value::String(padded.to_opaque_data_url()?));
        body.insert("mask".into(), Value::String(border.to_data_url()?));
        let mut result = self.post_png("inpaint", &Value::Object(body)).await?;
        if result.width != padded.width || result.height != padded.height {
            return Err(AiError::Geometry(
                "AI expand: the result has the wrong size".into(),
            ));
        }
        result.clip_alpha(&border)?;
        Ok(result)
    }

    /// `POST /api/v1/run_plugin_gen_image {name, image, scale}` for RemoveBG, AnimeSeg,
    /// RealESRGAN (`scale` 2..4), GFPGAN and RestoreFormer. The result keeps whatever size the
    /// plugin produces (an upscaler returns a larger image).
    pub async fn plugin_gen_image(
        &self,
        plugin: &str,
        image: &Rgba8Image,
        scale: f64,
    ) -> Result<Rgba8Image> {
        let factor = scale.clamp(1.0, 4.0);
        if plugin == "RealESRGAN" {
            let out_pixels = (f64::from(image.width) * factor).floor() as u64
                * (f64::from(image.height) * factor).floor() as u64;
            if out_pixels > MAX_UPSCALE_PIXELS {
                return Err(AiError::Geometry(
                    "Upscale: the result would pass 64 Mi-pixels".into(),
                ));
            }
        }
        self.post_png(
            "run_plugin_gen_image",
            &json!({ "name": plugin, "image": image.to_data_url()?, "scale": factor }),
        )
        .await
    }

    /// `POST /api/v1/run_plugin_gen_mask {name, image[, clicks]}`: RemoveBG / AnimeSeg find the
    /// subject (no clicks); InteractiveSeg segments around `clicks`. The returned mask is 255
    /// where the plugin painted.
    pub async fn plugin_gen_mask(
        &self,
        plugin: &str,
        image: &Rgba8Image,
        clicks: &[Click],
    ) -> Result<Mask> {
        let mut body = Map::new();
        body.insert("name".into(), Value::String(plugin.into()));
        body.insert("image".into(), Value::String(image.to_data_url()?));
        if plugin == "InteractiveSeg" {
            if clicks.is_empty() {
                return Err(AiError::Geometry(
                    "Click to select: click on the subject first".into(),
                ));
            }
            body.insert("clicks".into(), encode_clicks(clicks));
        }
        let bytes = self
            .post("run_plugin_gen_mask", &Value::Object(body))
            .await?;
        let mask = Mask::from_alpha_png(&bytes)?;
        if mask.width != image.width || mask.height != image.height {
            return Err(AiError::Geometry(
                "AI selection: the mask does not match the canvas".into(),
            ));
        }
        Ok(mask)
    }
}

/// IOPaint's error body is `{detail: "..."}` (FastAPI) or `{errors: "..."}`; fall back to the text.
fn server_error(status: u16, bytes: &[u8]) -> AiError {
    let detail = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|v| {
            v["detail"]
                .as_str()
                .or_else(|| v["errors"].as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(bytes).trim().to_owned());
    let detail: String = detail.chars().take(300).collect();
    AiError::Server { status, detail }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn options_serialise_only_what_is_set() {
        let body = InpaintOptions::default().to_body().unwrap();
        assert!(body.is_empty());
        let opts = InpaintOptions {
            prompt: Some("a bench".into()),
            sd_steps: Some(20),
            sd_seed: Some(-1),
            sd_keep_unmasked_area: Some(true),
            powerpaint_task: Some("text-guided".into()),
            enable_controlnet: Some(true),
            controlnet_method: Some("canny".into()),
            paint_by_example_image: Some(Rgba8Image::filled(1, 1, [1, 2, 3, 255]).unwrap()),
            ..InpaintOptions::default()
        };
        let body = opts.to_body().unwrap();
        assert_eq!(body["prompt"], "a bench");
        assert_eq!(body["sd_steps"], 20);
        assert_eq!(body["sd_seed"], -1);
        assert_eq!(body["sd_keep_unmasked_area"], true);
        assert_eq!(body["powerpaint_task"], "text-guided");
        assert_eq!(body["enable_controlnet"], true);
        assert_eq!(body["controlnet_method"], "canny");
        assert!(
            body["paint_by_example_example_image"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(!body.contains_key("negative_prompt"));
        assert!(opts.is_prompted());
        assert!(!InpaintOptions::default().is_prompted());
    }

    #[test]
    fn clicks_encode_as_triples() {
        let v = encode_clicks(&[(3, 4, true), (10, 20, false)]);
        assert_eq!(v, json!([[3, 4, 1], [10, 20, 0]]));
    }

    #[test]
    fn server_config_parses_like_qt() {
        let cfg = ServerConfig::from_json(&json!({
            "modelInfos": [
                {"name": "lama", "need_prompt": false},
                {"name": "Sanster/AnyText", "need_prompt": true}
            ],
            "plugins": [{"name": "RemoveBG"}, {"name": "RealESRGAN"}],
            "removeBGModel": "u2net", "removeBGModels": ["u2net", "isnet"],
            "realesrganModel": "realesr-general-x4v3", "realesrganModels": ["realesr-general-x4v3"],
            "interactiveSegModel": "mobile_sam", "interactiveSegModels": ["mobile_sam", "vit_b"],
            "samplers": ["ddim", "plms"]
        }));
        assert_eq!(
            cfg.downloaded_models(),
            vec!["Sanster/AnyText".to_owned(), "lama".to_owned()]
        );
        assert_eq!(cfg.model_infos["Sanster/AnyText"]["need_prompt"], true);
        assert!(cfg.has_plugin("RemoveBG"));
        assert!(!cfg.has_plugin("GFPGAN"));
        assert_eq!(cfg.plugin_models["RemoveBG"].current, "u2net");
        assert_eq!(cfg.plugin_models["InteractiveSeg"].choices.len(), 2);
        assert_eq!(cfg.samplers, vec!["ddim", "plms"]);
        // Missing fields are empty, not an error; the three plugin entries always exist.
        let empty = ServerConfig::from_json(&json!({}));
        assert!(empty.model_infos.is_empty() && empty.plugins.is_empty());
        assert_eq!(empty.plugin_models.len(), 3);
        assert_eq!(empty.plugin_models["RemoveBG"], PluginModels::default());
    }

    #[test]
    fn server_error_reads_detail_then_errors_then_text() {
        match server_error(422, br#"{"detail":"bad mask"}"#) {
            AiError::Server { status, detail } => {
                assert_eq!(status, 422);
                assert_eq!(detail, "bad mask");
            }
            other => panic!("{other:?}"),
        }
        match server_error(500, br#"{"errors":"boom"}"#) {
            AiError::Server { detail, .. } => assert_eq!(detail, "boom"),
            other => panic!("{other:?}"),
        }
        match server_error(502, b"Bad Gateway") {
            AiError::Server { detail, .. } => assert_eq!(detail, "Bad Gateway"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn layer_names() {
        assert_eq!(plugin_layer_name("RealESRGAN"), "AI upscale");
        assert_eq!(plugin_layer_name("Other"), "Other");
    }
}
