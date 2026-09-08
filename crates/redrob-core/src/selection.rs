// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::{CoreError, RasterBytes, Rect, Result, SamplingMode};

const MAX_SELECTION_RADIUS: u32 = 4_096;

/// How a new shape combines with the existing selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

/// An optional 8-bit grayscale selection mask.
///
/// An inactive selection means every canvas pixel is selected. Once active,
/// mask value 0 excludes a pixel and 255 includes it completely.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    width: u32,
    height: u32,
    active: bool,
    mask: RasterBytes,
}

impl Selection {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self> {
        let len = crate::document::pixel_count(width, height)?;
        Ok(Self {
            width,
            height,
            active: false,
            mask: RasterBytes::zeroed(len),
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn mask(&self) -> &[u8] {
        &self.mask
    }

    pub(crate) fn mask_storage(&self) -> &RasterBytes {
        &self.mask
    }

    /// Returns an owned selection mask suitable for snapshots across edits.
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        self.mask.to_vec()
    }

    pub fn coverage(&self, x: u32, y: u32) -> u8 {
        if !self.active {
            return u8::MAX;
        }
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.mask[(y as usize) * (self.width as usize) + (x as usize)]
    }

    pub(crate) fn clear(&mut self) {
        self.active = false;
        self.mask.fill(0);
    }

    pub(crate) fn select_all(&mut self) {
        self.active = true;
        self.mask.fill(u8::MAX);
    }

    pub(crate) fn invert(&mut self) {
        if self.active {
            for value in self.mask.iter_mut() {
                *value = u8::MAX - *value;
            }
        } else {
            self.mask.fill(0);
            self.active = true;
        }
    }

    pub(crate) fn apply_rect(&mut self, rect: Rect, mode: SelectionMode) {
        let mut shape = vec![0_u8; self.mask.len()];
        if let Some((x0, y0, x1, y1)) = rect.clipped_bounds(self.width, self.height) {
            for y in y0..y1 {
                let row = y as usize * self.width as usize;
                shape[row + x0 as usize..row + x1 as usize].fill(u8::MAX);
            }
        }
        self.combine(shape, mode);
    }

    pub(crate) fn apply_ellipse(&mut self, rect: Rect, mode: SelectionMode) {
        let mut shape = vec![0_u8; self.mask.len()];
        if rect.width != 0
            && rect.height != 0
            && let Some((x0, y0, x1, y1)) = rect.clipped_bounds(self.width, self.height)
        {
            let center_x = f64::from(rect.x) + f64::from(rect.width) * 0.5;
            let center_y = f64::from(rect.y) + f64::from(rect.height) * 0.5;
            let radius_x = f64::from(rect.width) * 0.5;
            let radius_y = f64::from(rect.height) * 0.5;
            for y in y0..y1 {
                for x in x0..x1 {
                    let mut inside = 0_u16;
                    for sy in 0..4 {
                        for sx in 0..4 {
                            let sample_x = f64::from(x) + (f64::from(sx) + 0.5) * 0.25;
                            let sample_y = f64::from(y) + (f64::from(sy) + 0.5) * 0.25;
                            let dx = (sample_x - center_x) / radius_x;
                            let dy = (sample_y - center_y) / radius_y;
                            inside += u16::from(dx * dx + dy * dy <= 1.0);
                        }
                    }
                    shape[y as usize * self.width as usize + x as usize] =
                        ((inside * 255 + 8) / 16) as u8;
                }
            }
        }
        self.combine(shape, mode);
    }

    fn combine(&mut self, shape: Vec<u8>, mode: SelectionMode) {
        if !self.active || mode == SelectionMode::Replace {
            self.mask = shape.into();
        } else {
            for (current, incoming) in self.mask.iter_mut().zip(shape) {
                *current = match mode {
                    SelectionMode::Replace => incoming,
                    SelectionMode::Add => current.saturating_add(incoming),
                    SelectionMode::Subtract => current.saturating_sub(incoming),
                    SelectionMode::Intersect => (*current).min(incoming),
                };
            }
        }
        self.active = true;
    }

    pub(crate) fn feather(&mut self, radius: u32) -> Result<()> {
        validate_radius(radius)?;
        if radius == 0 || !self.active {
            return Ok(());
        }
        let horizontal = box_blur_pass(&self.mask, self.width, self.height, radius, true);
        self.mask = box_blur_pass(&horizontal, self.width, self.height, radius, false).into();
        Ok(())
    }

    pub(crate) fn grow(&mut self, radius: u32) -> Result<()> {
        validate_radius(radius)?;
        if radius == 0 || !self.active {
            return Ok(());
        }
        let horizontal = extreme_pass(&self.mask, self.width, self.height, radius, true, true);
        self.mask = extreme_pass(&horizontal, self.width, self.height, radius, false, true).into();
        Ok(())
    }

    pub(crate) fn shrink(&mut self, radius: u32) -> Result<()> {
        validate_radius(radius)?;
        if radius == 0 || !self.active {
            return Ok(());
        }
        let horizontal = extreme_pass(&self.mask, self.width, self.height, radius, true, false);
        self.mask = extreme_pass(&horizontal, self.width, self.height, radius, false, false).into();
        Ok(())
    }

    pub(crate) fn crop(&mut self, rect: Rect) -> Result<()> {
        let new_len = crate::document::pixel_count(rect.width, rect.height)?;
        let mut output = vec![0; new_len];
        if self.active {
            for y in 0..rect.height {
                for x in 0..rect.width {
                    let source_x = i64::from(rect.x) + i64::from(x);
                    let source_y = i64::from(rect.y) + i64::from(y);
                    if source_x >= 0
                        && source_y >= 0
                        && source_x < i64::from(self.width)
                        && source_y < i64::from(self.height)
                    {
                        output[y as usize * rect.width as usize + x as usize] =
                            self.mask[source_y as usize * self.width as usize + source_x as usize];
                    }
                }
            }
        }
        self.width = rect.width;
        self.height = rect.height;
        self.mask = output.into();
        Ok(())
    }

    pub(crate) fn resize(&mut self, width: u32, height: u32, sampling: SamplingMode) -> Result<()> {
        let new_len = crate::document::pixel_count(width, height)?;
        let mut output = vec![0; new_len];
        if self.active {
            for y in 0..height {
                for x in 0..width {
                    let source_x =
                        (f64::from(x) + 0.5) * f64::from(self.width) / f64::from(width) - 0.5;
                    let source_y =
                        (f64::from(y) + 0.5) * f64::from(self.height) / f64::from(height) - 0.5;
                    output[y as usize * width as usize + x as usize] = match sampling {
                        SamplingMode::Nearest => {
                            let sx = source_x.round().clamp(0.0, f64::from(self.width - 1)) as u32;
                            let sy = source_y.round().clamp(0.0, f64::from(self.height - 1)) as u32;
                            self.mask[sy as usize * self.width as usize + sx as usize]
                        }
                        SamplingMode::Bilinear => sample_mask_for_raster(
                            &self.mask,
                            self.width,
                            self.height,
                            source_x,
                            source_y,
                        ),
                    };
                }
            }
        }
        self.width = width;
        self.height = height;
        self.mask = output.into();
        Ok(())
    }

    pub(crate) fn from_import_parts(
        width: u32,
        height: u32,
        active: bool,
        mask: Vec<u8>,
    ) -> Result<Self> {
        let selection = Self {
            width,
            height,
            active,
            mask: mask.into(),
        };
        selection.validate(width, height)?;
        Ok(selection)
    }

    pub(crate) fn from_v1_parts(width: u32, height: u32, active: bool, mask: Vec<u8>) -> Self {
        Self {
            width,
            height,
            active,
            mask: mask.into(),
        }
    }

    pub(crate) fn validate(&self, width: u32, height: u32) -> Result<()> {
        if self.width != width || self.height != height {
            return Err(CoreError::MalformedProject(
                "selection dimensions do not match the canvas".into(),
            ));
        }
        let expected = crate::document::pixel_count(width, height)?;
        if self.mask.len() != expected {
            return Err(CoreError::InvalidBufferLength {
                expected,
                actual: self.mask.len(),
            });
        }
        Ok(())
    }
}

fn validate_radius(radius: u32) -> Result<()> {
    if radius > MAX_SELECTION_RADIUS {
        Err(CoreError::InvalidSelectionRadius(radius))
    } else {
        Ok(())
    }
}

fn box_blur_pass(input: &[u8], width: u32, height: u32, radius: u32, horizontal: bool) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    let lines = if horizontal { height } else { width };
    let line_len = if horizontal { width } else { height };
    let divisor = u64::from(radius) * 2 + 1;
    for line in 0..lines {
        let mut prefix = vec![0_u64; line_len as usize + 1];
        for position in 0..line_len {
            let index = if horizontal {
                line as usize * width as usize + position as usize
            } else {
                position as usize * width as usize + line as usize
            };
            prefix[position as usize + 1] = prefix[position as usize] + u64::from(input[index]);
        }
        for position in 0..line_len {
            let start = position.saturating_sub(radius);
            let end = position
                .saturating_add(radius)
                .saturating_add(1)
                .min(line_len);
            let sum = prefix[end as usize] - prefix[start as usize];
            let index = if horizontal {
                line as usize * width as usize + position as usize
            } else {
                position as usize * width as usize + line as usize
            };
            output[index] = ((sum + divisor / 2) / divisor) as u8;
        }
    }
    output
}

fn extreme_pass(
    input: &[u8],
    width: u32,
    height: u32,
    radius: u32,
    horizontal: bool,
    maximum: bool,
) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    let lines = if horizontal { height } else { width };
    let line_len = if horizontal { width } else { height };
    let padding = radius as usize;
    let window = padding * 2 + 1;
    for line in 0..lines {
        let padded_len = line_len as usize + padding * 2;
        let mut deque: VecDeque<(usize, u8)> = VecDeque::new();
        for padded_index in 0..padded_len {
            let value = if padded_index < padding || padded_index >= padding + line_len as usize {
                0
            } else {
                let position = padded_index - padding;
                let index = if horizontal {
                    line as usize * width as usize + position
                } else {
                    position * width as usize + line as usize
                };
                input[index]
            };
            while deque.back().is_some_and(|&(_, back)| {
                if maximum {
                    back <= value
                } else {
                    back >= value
                }
            }) {
                deque.pop_back();
            }
            deque.push_back((padded_index, value));
            if padded_index + 1 >= window {
                let start = padded_index + 1 - window;
                while deque.front().is_some_and(|&(index, _)| index < start) {
                    deque.pop_front();
                }
                if start < line_len as usize {
                    let index = if horizontal {
                        line as usize * width as usize + start
                    } else {
                        start * width as usize + line as usize
                    };
                    output[index] = deque.front().map_or(0, |&(_, value)| value);
                }
            }
        }
    }
    output
}

pub(crate) fn sample_mask_for_raster(input: &[u8], width: u32, height: u32, x: f64, y: f64) -> u8 {
    let clamp = true;
    let x = if clamp {
        x.clamp(0.0, f64::from(width - 1))
    } else {
        x
    };
    let y = if clamp {
        y.clamp(0.0, f64::from(height - 1))
    } else {
        y
    };
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let tx = x - x.floor();
    let ty = y - y.floor();
    let sample = |sx: i64, sy: i64| -> f64 {
        if sx < 0 || sy < 0 || sx >= i64::from(width) || sy >= i64::from(height) {
            0.0
        } else {
            f64::from(input[sy as usize * width as usize + sx as usize])
        }
    };
    let top = sample(x0, y0) * (1.0 - tx) + sample(x0 + 1, y0) * tx;
    let bottom = sample(x0, y0 + 1) * (1.0 - tx) + sample(x0 + 1, y0 + 1) * tx;
    (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
}
