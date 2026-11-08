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

    pub fn blend_pixel(&mut self, x: i32, y: i32, rgb: u32, alpha: u32, clip: Rect) {
        if alpha == 0 {
            return;
        }
        if x < clip.x || y < clip.y || x >= clip.x + clip.w || y >= clip.y + clip.h {
            return;
        }
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = y as usize * self.width as usize + x as usize;
        self.pixels[index] = if alpha >= 255 {
            rgb
        } else {
            blend(rgb, self.pixels[index], alpha)
        };
    }

    pub fn blit_rgba(&mut self, x: i32, y: i32, w: i32, h: i32, rgba: &[u8], clip: Rect) {
        let x0 = x.max(clip.x).max(0);
        let y0 = y.max(clip.y).max(0);
        let x1 = (x + w).min(clip.x + clip.w).min(self.width as i32);
        let y1 = (y + h).min(clip.y + clip.h).min(self.height as i32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for row in y0..y1 {
            for col in x0..x1 {
                let source = ((row - y) as usize * w as usize + (col - x) as usize) * 4;
                let alpha = rgba[source + 3] as u32;
                if alpha == 0 {
                    continue;
                }
                let src = ((rgba[source] as u32) << 16)
                    | ((rgba[source + 1] as u32) << 8)
                    | rgba[source + 2] as u32;
                let index = row as usize * self.width as usize + col as usize;
                self.pixels[index] = if alpha == 255 {
                    src
                } else {
                    blend(src, self.pixels[index], alpha)
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

    #[test]
    fn blit_blends_straight_alpha() {
        let mut fb = Framebuffer::new(2, 1);
        let clip = Rect {
            x: 0,
            y: 0,
            w: 2,
            h: 1,
        };
        let rgba = [255, 0, 0, 255, 255, 255, 255, 128];
        fb.blit_rgba(0, 0, 2, 1, &rgba, clip);
        assert_eq!(fb.pixels()[0], 0xff0000);
        assert_eq!(fb.pixels()[1], 0x808080);
    }

    #[test]
    fn blit_is_clipped() {
        let mut fb = Framebuffer::new(4, 4);
        let clip = Rect {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
        };
        let rgba = [1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255];
        fb.blit_rgba(-1, -1, 2, 2, &rgba, clip);
        assert_eq!(fb.pixels()[0], 0x040000);
        assert_eq!(fb.pixels()[1], 0);
        assert_eq!(fb.pixels()[4], 0);
    }

    #[test]
    fn pixel_outside_clip_is_ignored() {
        let mut fb = Framebuffer::new(4, 4);
        let clip = Rect {
            x: 1,
            y: 1,
            w: 1,
            h: 1,
        };
        fb.blend_pixel(0, 0, 0xffffff, 255, clip);
        fb.blend_pixel(1, 1, 0xffffff, 255, clip);
        assert_eq!(fb.pixels()[0], 0);
        assert_eq!(fb.pixels()[5], 0xffffff);
    }
}
