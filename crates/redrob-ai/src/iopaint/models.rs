// SPDX-License-Identifier: MIT OR Apache-2.0

//! The models IOPaint 1.6.0 can be started with.

/// The default model: LaMa, the fastest erase model.
pub const DEFAULT_MODEL: &str = "lama";

/// Erase models (`iopaint/const.py` AVAILABLE_MODELS, plus anime-lama): remove what is selected
/// and fill it in from the surroundings. No prompt.
pub const ERASE_MODELS: &[&str] = &[
    "lama",
    "anime-lama",
    "migan",
    "mat",
    "fcf",
    "zits",
    "ldm",
    "manga",
    "cv2",
];

/// Diffusion models (the `name` of each model class under `iopaint/model/`): replace the selection
/// from a prompt. Slow on a CPU; several GB to download.
pub const DIFFUSION_MODELS: &[&str] = &[
    "Sanster/PowerPaint-V1-stable-diffusion-inpainting",
    "runwayml/stable-diffusion-inpainting",
    "stabilityai/stable-diffusion-2-inpainting",
    "Sanster/Realistic_Vision_V1.4-inpainting",
    "Sanster/anything-4.0-inpainting",
    "diffusers/stable-diffusion-xl-1.0-inpainting-0.1",
    "kandinsky-community/kandinsky-2-2-decoder-inpaint",
    "Sanster/AnyText",
    "Fantasy-Studio/Paint-by-Example",
    "timbrooks/instruct-pix2pix",
];

/// PowerPaint's task choices (`powerpaint_task` in an inpaint request).
pub const POWERPAINT_TASKS: &[&str] = &[
    "text-guided",
    "object-remove",
    "context-aware",
    "shape-guided",
    "outpainting",
];

/// Whether `name` is one of the built-in lists (a downloaded model the server reports but we do
/// not list is also startable; check the server config for those).
pub fn is_known_model(name: &str) -> bool {
    ERASE_MODELS.contains(&name) || DIFFUSION_MODELS.contains(&name)
}

/// Whether `name` is a diffusion model, which needs a prompt.
pub fn is_diffusion_model(name: &str) -> bool {
    DIFFUSION_MODELS.contains(&name)
}

/// Whether `name` is the Paint-by-Example model (the only one taking an example image).
pub fn is_paint_by_example(name: &str) -> bool {
    name.contains("Paint-by-Example")
}

/// Whether `name` is a PowerPaint model (the only one taking `powerpaint_task`).
pub fn is_powerpaint(name: &str) -> bool {
    name.contains("PowerPaint")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_match_the_qt_engine() {
        assert_eq!(ERASE_MODELS.len(), 9);
        assert_eq!(DIFFUSION_MODELS.len(), 10);
        assert_eq!(ERASE_MODELS[0], DEFAULT_MODEL);
        assert!(is_known_model("lama"));
        assert!(is_known_model("Sanster/AnyText"));
        assert!(!is_known_model("nope"));
        assert!(is_diffusion_model("runwayml/stable-diffusion-inpainting"));
        assert!(!is_diffusion_model("lama"));
        assert!(is_paint_by_example("Fantasy-Studio/Paint-by-Example"));
        assert!(is_powerpaint(DIFFUSION_MODELS[0]));
    }
}
