// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::HashMap;
use std::fs;

use fontdue::{Font, FontSettings, Metrics};

use crate::framebuffer::{Framebuffer, Rect};

const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;

const DEFAULT_FONT_PATHS: [&str; 6] = [
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
];

struct Glyph {
    metrics: Metrics,
    coverage: Vec<u8>,
}

pub struct TextRenderer {
    font: Option<Font>,
    cache: HashMap<(char, u16), Glyph>,
    cache_bytes: usize,
}

impl TextRenderer {
    pub fn load() -> Self {
        Self {
            font: load_font(),
            cache: HashMap::new(),
            cache_bytes: 0,
        }
    }

    pub fn draw(
        &mut self,
        framebuffer: &mut Framebuffer,
        text: &str,
        x: i32,
        y: i32,
        height: u16,
        rgba: u32,
        clip: Rect,
    ) {
        let font = match self.font.as_ref() {
            Some(font) => font,
            None => return,
        };
        let size = height as f32;
        let ascent = font
            .horizontal_line_metrics(size)
            .map(|metrics| metrics.ascent)
            .unwrap_or(size * 0.8);
        let baseline = y + ascent.round() as i32;
        let color_alpha = rgba & 0xff;
        let rgb = (rgba >> 8) & 0x00ff_ffff;
        let mut pen = x as f32;

        for ch in text.chars() {
            if ch.is_control() {
                continue;
            }
            let key = (ch, height);
            if !self.cache.contains_key(&key) {
                let (metrics, coverage) = font.rasterize(ch, size);
                if self.cache_bytes + coverage.len() > MAX_CACHE_BYTES {
                    self.cache.clear();
                    self.cache_bytes = 0;
                }
                self.cache_bytes += coverage.len();
                self.cache.insert(key, Glyph { metrics, coverage });
            }
            let glyph = &self.cache[&key];
            let left = pen.round() as i32 + glyph.metrics.xmin;
            let top = baseline - glyph.metrics.ymin - glyph.metrics.height as i32;
            for row in 0..glyph.metrics.height {
                for col in 0..glyph.metrics.width {
                    let coverage = glyph.coverage[row * glyph.metrics.width + col] as u32;
                    if coverage == 0 {
                        continue;
                    }
                    let alpha = coverage * color_alpha / 255;
                    framebuffer.blend_pixel(left + col as i32, top + row as i32, rgb, alpha, clip);
                }
            }
            pen += glyph.metrics.advance_width;
        }
    }
}

fn load_font() -> Option<Font> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(path) = std::env::var("VPORT_FONT") {
        candidates.push(path);
    }
    candidates.extend(DEFAULT_FONT_PATHS.iter().map(|path| path.to_string()));

    for path in candidates {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        match Font::from_bytes(bytes, FontSettings::default()) {
            Ok(font) => {
                eprintln!("vportd: font loaded from {path}");
                return Some(font);
            }
            Err(e) => eprintln!("vportd: could not parse font {path}: {e}"),
        }
    }
    eprintln!("vportd: no font found, text will not be drawn (set VPORT_FONT)");
    None
}
