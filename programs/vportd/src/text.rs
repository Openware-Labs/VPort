// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use fontdue::{Font, FontSettings, Metrics};

use crate::framebuffer::{Framebuffer, Rect};

const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SCAN_DEPTH: usize = 6;
const MAX_SCANNED_FILES: usize = 4096;

const FONT_ROOTS: [&str; 3] = [
    "/usr/share/fonts",
    "/usr/local/share/fonts",
    "/usr/share/X11/fonts",
];

const PREFERRED_FONTS: [&str; 6] = [
    "DejaVuSans.ttf",
    "NotoSans-Regular.ttf",
    "LiberationSans-Regular.ttf",
    "Roboto-Regular.ttf",
    "OpenSans-Regular.ttf",
    "Arial.ttf",
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

fn collect_fonts(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH || found.len() >= MAX_SCANNED_FILES {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fonts(&path, depth + 1, found);
        } else if path
            .extension()
            .map_or(false, |extension| extension.eq_ignore_ascii_case("ttf"))
        {
            found.push(path);
        }
    }
}

fn font_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path) = std::env::var("VPORT_FONT") {
        candidates.push(PathBuf::from(path));
    }

    let mut scanned: Vec<PathBuf> = Vec::new();
    for root in FONT_ROOTS {
        collect_fonts(Path::new(root), 0, &mut scanned);
    }
    scanned.sort();

    for name in PREFERRED_FONTS {
        let position = scanned
            .iter()
            .position(|path| path.file_name().map_or(false, |file| file == name));
        if let Some(position) = position {
            candidates.push(scanned.remove(position));
        }
    }
    candidates.extend(scanned);
    candidates
}

fn load_font() -> Option<Font> {
    for path in font_candidates() {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("vportd: could not read font {}: {e}", path.display());
                continue;
            }
        };
        match Font::from_bytes(bytes, FontSettings::default()) {
            Ok(font) => {
                eprintln!("vportd: font loaded from {}", path.display());
                return Some(font);
            }
            Err(e) => eprintln!("vportd: could not parse font {}: {e}", path.display()),
        }
    }
    eprintln!("vportd: no font found, text will not be drawn (set VPORT_FONT)");
    None
}
