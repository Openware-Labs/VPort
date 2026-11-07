// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

impl Framebuffer {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn clear(&mut self, rgb: u32) {
        self.pixels.fill(rgb & 0x00ff_ffff);
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, rgba: u32, clip: Rect) {
        let x0 = x.max(clip.x).max(0);
        let y0 = y.max(clip.y).max(0);
        let x1 = (x + w).min(clip.x + clip.w).min(self.width as i32);
        let y1 = (y + h).min(clip.y + clip.h).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let alpha = rgba & 0xff;
        let src = (rgba >> 8) & 0x00ff_ffff;
        for row in y0..y1 {
            let start = row as usize * self.width as usize;
            let span = &mut self.pixels[start + x0 as usize..start + x1 as usize];
            for pixel in span {
                *pixel = if alpha == 255 {
                    src
                } else {
                    blend(src, *pixel, alpha)
                };
            }
        }
    }
}

fn blend(src: u32, dst: u32, alpha: u32) -> u32 {
    let inverse = 255 - alpha;
    let r = (((src >> 16) & 0xff) * alpha + ((dst >> 16) & 0xff) * inverse) / 255;
    let g = (((src >> 8) & 0xff) * alpha + ((dst >> 8) & 0xff) * inverse) / 255;
    let b = ((src & 0xff) * alpha + (dst & 0xff) * inverse) / 255;
    (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_is_clipped_to_screen() {
        let mut fb = Framebuffer::new(4, 4);
        let clip = Rect {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
        };
        fb.fill_rect(-2, -2, 4, 4, 0xff0000ff, clip);
        assert_eq!(fb.pixels()[0], 0xff0000);
        assert_eq!(fb.pixels()[5], 0xff0000);
        assert_eq!(fb.pixels()[2], 0);
        assert_eq!(fb.pixels()[8], 0);
    }

    #[test]
    fn fill_is_clipped_to_portal() {
        let mut fb = Framebuffer::new(4, 4);
        let clip = Rect {
            x: 1,
            y: 1,
            w: 2,
            h: 2,
        };
        fb.fill_rect(0, 0, 4, 4, 0x00ff00ff, clip);
        assert_eq!(fb.pixels()[0], 0);
        assert_eq!(fb.pixels()[5], 0x00ff00);
        assert_eq!(fb.pixels()[10], 0x00ff00);
        assert_eq!(fb.pixels()[15], 0);
    }

    #[test]
    fn alpha_blends_over_background() {
        let mut fb = Framebuffer::new(1, 1);
        let clip = Rect {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        fb.fill_rect(0, 0, 1, 1, 0xffffff80, clip);
        assert_eq!(fb.pixels()[0], 0x808080);
    }
}
