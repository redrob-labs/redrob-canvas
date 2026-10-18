// SPDX-License-Identifier: MIT OR Apache-2.0
//! Small typed records attached to a layer or a document without a model change: a layer record
//! rides in [`Layer::psd_blocks`] under a private four-byte key (a PSD reader skips keys it does
//! not know, and the native format keeps every block), a document record in
//! [`Metadata::text`] under a `redrob.` key. Both survive duplicate, save and load.
//!
//! Used by the live layers (clone, filter, transform and colorize), the file-layer scaling mode,
//! the generated fill layers and the animation records.

use std::sync::Arc;

use redrob_photo_doc::{Document, Layer};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// A layer record key: four ASCII bytes starting with `r` (none of Photoshop's keys do).
pub type Key = [u8; 4];

/// Read the record stored under `key`, if any and if it still parses.
pub fn get<T: DeserializeOwned>(l: &Layer, key: Key) -> Option<T> {
    l.psd_blocks.iter().find(|(k, _)| *k == key).and_then(|(_, d)| serde_json::from_slice(d).ok())
}

/// Whether the layer carries a record under `key`.
pub fn has(l: &Layer, key: Key) -> bool {
    l.psd_blocks.iter().any(|(k, _)| *k == key)
}

/// Store `value` under `key`, replacing an earlier record.
pub fn set<T: Serialize>(l: &mut Layer, key: Key, value: &T) {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    match l.psd_blocks.iter_mut().find(|(k, _)| *k == key) {
        Some((_, d)) => *d = Arc::new(bytes),
        None => l.psd_blocks.push((key, Arc::new(bytes))),
    }
}

/// Drop the record under `key`. Returns whether there was one.
pub fn remove(l: &mut Layer, key: Key) -> bool {
    let n = l.psd_blocks.len();
    l.psd_blocks.retain(|(k, _)| *k != key);
    l.psd_blocks.len() != n
}

/// The document-level record under `name` (`redrob.<name>` in the image text).
pub fn doc_get<T: DeserializeOwned>(doc: &Document, name: &str) -> Option<T> {
    let key = format!("redrob.{name}");
    doc.metadata.text.iter().find(|(k, _)| *k == key).and_then(|(_, v)| serde_json::from_str(v).ok())
}

/// Store a document-level record under `name`.
pub fn doc_set<T: Serialize>(doc: &mut Document, name: &str, value: &T) {
    let key = format!("redrob.{name}");
    let text = serde_json::to_string(value).unwrap_or_default();
    match doc.metadata.text.iter_mut().find(|(k, _)| *k == key) {
        Some((_, v)) => *v = text,
        None => doc.metadata.text.push((key, text)),
    }
}

/// Drop the document-level record under `name`.
pub fn doc_remove(doc: &mut Document, name: &str) {
    let key = format!("redrob.{name}");
    doc.metadata.text.retain(|(k, _)| *k != key);
}

/// Every layer of the document, bottom-to-top, with a record under `key`.
pub fn layers_with<T: DeserializeOwned>(doc: &Document, key: Key) -> Vec<(redrob_photo_doc::LayerId, T)> {
    doc.walk().into_iter().filter_map(|(_, _, l)| get::<T>(l, key).map(|t| (l.id, t))).collect()
}
