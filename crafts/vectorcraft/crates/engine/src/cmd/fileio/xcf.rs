//! Layered GIMP XCF export (`document.export {format: xcf}`): each top-level layer a pixel layer
//! with its name, opacity, visibility and blend mode (`layers: false`: one flat image), rendered
//! at `ppi` (else `scale` pixels per point) over `background`; one file per chosen artboard with
//! `useArtboards`. The writer is `redrob_vector_render::encode::xcf`.

use redrob_vector_doc::Document;
use redrob_vector_render::encode::xcf::XcfOptions;
use redrob_vector_render::encode::{RasterExportOptions, RasterFormat};
use serde::Deserialize;
use serde_json::Value;

use super::super::*;
use super::{ArtboardPick, Encoded};

const C: &str = "document.export";

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Options {
    #[serde(flatten)]
    boards: ArtboardPick,
    scale: Option<f64>,
    ppi: Option<f64>,
    background: Option<Value>,
    anti_alias: Option<String>,
    layers: Option<bool>,
}

pub(super) fn encode(doc: &Document, p: &Value, use_artboards: Option<bool>) -> Result<Encoded> {
    let f = super::format("xcf").ok_or_else(|| bad(C, "no XCF format"))?;
    let o: Options = super::encode::options(f, p)?;
    if let Some(ppi) = o.ppi.filter(|p| !(p.is_finite() && *p > 0.0)) {
        return Err(bad(C, format!("ppi must be a positive number, not {ppi}")));
    }
    let ppi = (o.ppi.unwrap_or(72.0 * o.scale.unwrap_or(1.0)) / 72.0).clamp(0.01, 64.0) * 72.0;
    let page = (doc.setup.background == redrob_vector_doc::Background::White).then_some([255; 3]);
    let background = match &o.background {
        Some(v) => super::encode::background(v).map_err(|e| bad(C, e))?,
        None => page,
    };
    let anti_alias = o.anti_alias.as_deref().map(super::encode::anti_alias).transpose().map_err(|e| bad(C, e))?.unwrap_or_default();
    let render = RasterExportOptions { ppi, background: None, anti_alias, ..RasterExportOptions::default() }.render_options(RasterFormat::Png);
    let xo = XcfOptions { scale: ppi / 72.0, ppi, background, layers: o.layers.unwrap_or(true), render };
    let n = doc.artboards.len();
    let chosen = match use_artboards {
        Some(true) => o.boards.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect()),
        _ => vec![o.boards.one(n).map_err(|e| bad(C, e))?],
    };
    let mut enc = Encoded::default();
    let mut renderer = redrob_vector_render::Renderer::new();
    for b in chosen {
        let region = doc.artboards.get(b).ok_or_else(|| bad(C, format!("no artboard {}", b + 1)))?.rect;
        redrob_vector_render::raster_size(region, xo.scale).map_err(|e| bad(C, e))?;
        let bytes = redrob_vector_render::encode::xcf::export(&mut renderer, doc, region, &xo).map_err(|e| bad(C, e))?;
        enc.files.push((Some(b), bytes));
    }
    if enc.files.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    if let Some(w) = crate::cmd::fonts::substitution_warning(doc) {
        enc.warnings.push(w);
    }
    Ok(enc)
}
