// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;
use std::io::Cursor;

use image::{ImageDecoder, ImageReader, Limits};
use serde::de::{Error as DeError, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::document::{MAX_DIMENSION, MAX_PIXELS};
use crate::{
    BlendMode, CoreError, Document, DocumentMetadata, Layer, LayerId, MAX_NODES, Result, Selection,
};

const PROJECT_MAGIC: &str = "REDROB_GRAPHICS_PROJECT";
const PROJECT_VERSION: u32 = 2;
/// Maximum encoded JSON size accepted by the project loader. Raster bytes are
/// JSON arrays in v2, so 512 MiB permits practical projects while bounding all
/// decode-time allocations before field-level semantic visitors run.
pub const MAX_PROJECT_JSON_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Deserialize)]
struct ProjectHeader {
    magic: String,
    version: u32,
}

#[derive(Serialize)]
struct ProjectRef<'a> {
    magic: &'static str,
    version: u32,
    document: &'a Document,
}

#[derive(Deserialize)]
struct ProjectV2 {
    document: Document,
}

fn deserialize_v1_layers<'de, D>(deserializer: D) -> std::result::Result<Vec<V1Layer>, D::Error>
where
    D: Deserializer<'de>,
{
    struct V1LayersVisitor;

    impl<'de> Visitor<'de> for V1LayersVisitor {
        type Value = Vec<V1Layer>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "version-1 layer array with at most {MAX_NODES} entries"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Vec<V1Layer>, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // Ignore the untrusted size hint and do not deserialize the first
            // layer beyond the current document node limit.
            let mut layers = Vec::new();
            while layers.len() < MAX_NODES {
                let Some(layer) = sequence.next_element()? else {
                    return Ok(layers);
                };
                layers.push(layer);
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format_args!(
                    "version-1 layer count exceeds {MAX_NODES}"
                )));
            }
            Ok(layers)
        }
    }

    deserializer.deserialize_seq(V1LayersVisitor)
}

/// Literal version-1 wire document. Keep this independent from the evolving model.
#[derive(Deserialize)]
struct V1Document {
    id: Uuid,
    width: u32,
    height: u32,
    metadata: DocumentMetadata,
    #[serde(deserialize_with = "deserialize_v1_layers")]
    layers: Vec<V1Layer>,
    active_layer: LayerId,
    selection: V1Selection,
}

#[derive(Deserialize)]
struct V1Layer {
    id: LayerId,
    name: String,
    visible: bool,
    opacity: f32,
    blend_mode: BlendMode,
    pixels: Vec<u8>,
}

#[derive(Deserialize)]
struct V1Selection {
    width: u32,
    height: u32,
    active: bool,
    mask: Vec<u8>,
}

#[derive(Deserialize)]
struct ProjectV1 {
    document: V1Document,
}

impl V1Document {
    fn migrate(self) -> Document {
        Document::from_v1_parts(
            self.id,
            self.width,
            self.height,
            self.metadata,
            self.layers
                .into_iter()
                .map(|layer| {
                    Layer::from_v1_parts(
                        layer.id,
                        layer.name,
                        layer.visible,
                        layer.opacity,
                        layer.blend_mode,
                        layer.pixels,
                    )
                })
                .collect(),
            self.active_layer,
            Selection::from_v1_parts(
                self.selection.width,
                self.selection.height,
                self.selection.active,
                self.selection.mask,
            ),
        )
    }
}

/// Serializes a validated document as a version-2 `.rrg` project.
pub fn save_project(document: &Document) -> Result<Vec<u8>> {
    document.validate()?;
    let bytes = serde_json::to_vec(&ProjectRef {
        magic: PROJECT_MAGIC,
        version: PROJECT_VERSION,
        document,
    })?;
    if bytes.len() as u64 > MAX_PROJECT_JSON_BYTES {
        return Err(CoreError::DocumentLimitExceeded("encoded project bytes"));
    }
    Ok(bytes)
}

/// Loads a `.rrg` project, validating its envelope before decoding its document body.
pub fn load_project(bytes: &[u8]) -> Result<Document> {
    if bytes.len() as u64 > MAX_PROJECT_JSON_BYTES {
        return Err(CoreError::DocumentLimitExceeded("encoded project bytes"));
    }
    let header: ProjectHeader = serde_json::from_slice(bytes)?;
    if header.magic != PROJECT_MAGIC {
        return Err(CoreError::InvalidProjectMagic);
    }
    let document = match header.version {
        1 => serde_json::from_slice::<ProjectV1>(bytes)?
            .document
            .migrate(),
        PROJECT_VERSION => serde_json::from_slice::<ProjectV2>(bytes)?.document,
        version => return Err(CoreError::UnsupportedProjectVersion(version)),
    };
    document.validate()?;
    Ok(document)
}

/// Decodes a PNG into a one-layer document with guarded dimensions/allocation.
pub fn import_png(bytes: &[u8]) -> Result<Document> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if reader.format() != Some(image::ImageFormat::Png) {
        return Err(CoreError::UnsupportedImageFormat);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_PIXELS.saturating_mul(8));
    reader.limits(limits);
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    crate::document::pixel_count(width, height)?;
    let image = image::DynamicImage::from_decoder(decoder)?.to_rgba8();
    Document::from_single_layer(width, height, image.into_raw(), String::new())
}

/// Flattens supported current-frame raster nodes and encodes lossless RGBA PNG bytes.
pub fn export_png(document: &Document) -> Result<Vec<u8>> {
    crate::formats::export_png_compatible(document)
}
