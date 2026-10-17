// SPDX-License-Identifier: MIT OR Apache-2.0

//! How pixels reach the engine.
//!
//! IOPaint's own API takes every image as a base64 PNG inside a JSON body and answers with a PNG,
//! which at 4K costs more than the model does (`docs/gap/results/gpu-ai.toml`: 7.7 s round trip,
//! 0.7 s compute). The engine and the app run on the same machine, so the shim the engine starts
//! IOPaint through (`redrob_shim.py`) also accepts *paths*: the client writes the raw pixels to a
//! per-request file in an exchange directory the two share, sends the path, and reads the result
//! from the path the shim wrote. No encoding on either side.
//!
//! [`TransportMode`] is the user's setting; [`Transport`] is what a client ended up with (file
//! transport needs a loopback engine, the shim's routes and a writable exchange directory).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::{
    AiError, Result,
    pixels::{Mask, Rgba8Image},
};

/// The transport the user asked for. `File` is the default and falls back to `Base64` on its
/// own when the engine is not local or the shim's routes are missing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    #[default]
    File,
    Base64,
}

impl TransportMode {
    pub const ALL: [TransportMode; 2] = [TransportMode::File, TransportMode::Base64];

    pub fn as_str(self) -> &'static str {
        match self {
            TransportMode::File => "file",
            TransportMode::Base64 => "base64",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "file" => Some(TransportMode::File),
            "base64" => Some(TransportMode::Base64),
            _ => None,
        }
    }

    /// What the settings panel shows.
    pub fn label(self) -> &'static str {
        match self {
            TransportMode::File => "file (local engine, fastest)",
            TransportMode::Base64 => "base64 (works with a remote engine)",
        }
    }
}

/// Whether `base_url` points at this machine (the only case files can be shared).
pub fn is_loopback_url(base_url: &str) -> bool {
    let rest = base_url
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    let host_port = rest.split('/').next().unwrap_or_default();
    let host = if let Some(stripped) = host_port.strip_prefix('[') {
        stripped.split(']').next().unwrap_or_default()
    } else {
        host_port.rsplit_once(':').map_or(host_port, |(h, _)| h)
    };
    host.eq_ignore_ascii_case("localhost")
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The directory the client and the shim exchange raw pixel files through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileExchange {
    dir: PathBuf,
}

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

impl FileExchange {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Makes the directory (private to the user where the platform has modes).
    pub fn prepare(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    /// The three files of one request, removed when the handle drops.
    pub fn request(&self) -> RequestFiles {
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let stem = format!("req-{}-{id}", std::process::id());
        RequestFiles {
            image: self.dir.join(format!("{stem}.image.raw")),
            mask: self.dir.join(format!("{stem}.mask.raw")),
            output: self.dir.join(format!("{stem}.out.raw")),
        }
    }

    /// Removes every exchange file (at engine stop).
    pub fn clear(&self) {
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.dir);
    }
}

/// The per-request files; dropping the value deletes them.
#[derive(Debug)]
pub struct RequestFiles {
    pub image: PathBuf,
    pub mask: PathBuf,
    pub output: PathBuf,
}

impl RequestFiles {
    /// `path` as the JSON string the shim gets.
    pub fn string(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }
}

impl Drop for RequestFiles {
    fn drop(&mut self) {
        for p in [&self.image, &self.mask, &self.output] {
            let _ = fs::remove_file(p);
        }
    }
}

/// Writes `image` flattened over white as `width * height * 3` RGB bytes.
pub fn write_rgb(path: &Path, image: &Rgba8Image) -> Result<()> {
    fs::write(path, image.flatten_over_white())?;
    Ok(())
}

/// Writes `image` as `width * height * 4` RGBA bytes.
pub fn write_rgba(path: &Path, image: &Rgba8Image) -> Result<()> {
    fs::write(path, &image.data)?;
    Ok(())
}

/// Writes `mask` as `width * height` L8 bytes.
pub fn write_mask(path: &Path, mask: &Mask) -> Result<()> {
    fs::write(path, &mask.data)?;
    Ok(())
}

/// Reads a `width × height` result with `channels` (3 = RGB, 4 = RGBA) bytes per pixel.
pub fn read_image(path: &Path, width: u32, height: u32, channels: u32) -> Result<Rgba8Image> {
    let bytes = fs::read(path).map_err(|e| {
        AiError::Png(format!(
            "the AI engine wrote no result file ({}): {e}",
            path.display()
        ))
    })?;
    match channels {
        3 => Rgba8Image::from_rgb(width, height, &bytes),
        4 => Rgba8Image::new(width, height, bytes),
        other => Err(AiError::Png(format!(
            "the AI engine wrote {other} channels per pixel"
        ))),
    }
}

/// Reads a `width × height` L8 mask.
pub fn read_mask(path: &Path, width: u32, height: u32) -> Result<Mask> {
    let bytes = fs::read(path).map_err(|e| {
        AiError::Png(format!(
            "the AI engine wrote no mask file ({}): {e}",
            path.display()
        ))
    })?;
    Mask::new(width, height, bytes)
}

/// What a client sends pixels as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transport {
    /// PNG data URLs in the JSON body, PNG bytes back: IOPaint's own API.
    Base64,
    /// Raw pixel files in the exchange directory, paths in the JSON body: the shim's routes.
    File(FileExchange),
}

impl Transport {
    /// The transport for `mode` against `base_url`: `File` only when the engine is on this
    /// machine, the shim answered that it has the file routes (`shim_has_files`) and an exchange
    /// directory is known; `Base64` otherwise.
    pub fn choose(
        mode: TransportMode,
        base_url: &str,
        exchange_dir: Option<&Path>,
        shim_has_files: bool,
    ) -> Transport {
        match (mode, exchange_dir) {
            (TransportMode::File, Some(dir)) if shim_has_files && is_loopback_url(base_url) => {
                Transport::File(FileExchange::new(dir))
            }
            _ => Transport::Base64,
        }
    }

    pub fn mode(&self) -> TransportMode {
        match self {
            Transport::Base64 => TransportMode::Base64,
            Transport::File(_) => TransportMode::File,
        }
    }

    pub fn is_file(&self) -> bool {
        matches!(self, Transport::File(_))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn loopback_urls() {
        for url in [
            "http://127.0.0.1:8080",
            "http://127.0.0.1",
            "http://localhost:1/",
            "http://[::1]:9",
            "http://127.5.5.5:3000/api",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
        for url in ["http://10.0.0.5:8080", "https://ai.example.com", "http://[::2]:1"] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn mode_names_round_trip_and_choice_needs_everything() {
        for mode in TransportMode::ALL {
            assert_eq!(TransportMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(TransportMode::parse(" FILE "), Some(TransportMode::File));
        assert_eq!(TransportMode::parse("shm"), None);
        assert_eq!(TransportMode::default(), TransportMode::File);
        assert_eq!(
            serde_json::to_string(&TransportMode::Base64).unwrap(),
            "\"base64\""
        );
        let dir = Path::new("/x");
        let local = "http://127.0.0.1:1";
        assert!(Transport::choose(TransportMode::File, local, Some(dir), true).is_file());
        assert!(!Transport::choose(TransportMode::Base64, local, Some(dir), true).is_file());
        assert!(!Transport::choose(TransportMode::File, local, None, true).is_file());
        assert!(!Transport::choose(TransportMode::File, local, Some(dir), false).is_file());
        assert!(!Transport::choose(TransportMode::File, "http://10.1.1.1:1", Some(dir), true).is_file());
    }

    #[test]
    fn request_files_round_trip_raw_pixels_and_vanish_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let exchange = FileExchange::new(tmp.path().join("exchange"));
        exchange.prepare().unwrap();
        let image = Rgba8Image::new(2, 1, vec![0, 0, 0, 0, 10, 20, 30, 255]).unwrap();
        let mask = Mask::new(2, 1, vec![0, 255]).unwrap();
        let files = exchange.request();
        write_rgb(&files.image, &image).unwrap();
        write_mask(&files.mask, &mask).unwrap();
        assert_eq!(fs::read(&files.image).unwrap(), vec![255, 255, 255, 10, 20, 30]);
        // The shim writes RGB back; the reader attaches alpha.
        fs::write(&files.output, [1, 2, 3, 4, 5, 6]).unwrap();
        let back = read_image(&files.output, 2, 1, 3).unwrap();
        assert_eq!(back.data, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        assert!(read_image(&files.output, 2, 1, 4).is_err());
        assert_eq!(read_mask(&files.mask, 2, 1).unwrap(), mask);
        write_rgba(&files.image, &image).unwrap();
        assert_eq!(read_image(&files.image, 2, 1, 4).unwrap(), image);
        let image_path = files.image.clone();
        drop(files);
        assert!(!image_path.exists());
        assert!(read_image(&image_path, 2, 1, 3).is_err());
        exchange.clear();
        assert!(!exchange.dir().exists());
    }
}
