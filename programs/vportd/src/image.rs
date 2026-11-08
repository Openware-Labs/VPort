// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

pub struct Image {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    pub fn write(&mut self, offset: usize, data: &[u8]) -> bool {
        match offset.checked_add(data.len()) {
            Some(end) if end <= self.pixels.len() => {
                self.pixels[offset..end].copy_from_slice(data);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_image_is_transparent() {
        let image = Image::new(2, 2);
        assert_eq!(image.byte_len(), 16);
        assert!(image.pixels.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn write_inside_bounds() {
        let mut image = Image::new(2, 1);
        assert!(image.write(4, &[9, 8, 7, 6]));
        assert_eq!(&image.pixels[4..8], &[9, 8, 7, 6]);
    }

    #[test]
    fn write_outside_bounds_is_refused() {
        let mut image = Image::new(2, 1);
        assert!(!image.write(5, &[1, 2, 3, 4]));
        assert!(!image.write(usize::MAX, &[1]));
    }
}
