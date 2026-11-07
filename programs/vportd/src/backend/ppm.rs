// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;

use super::Backend;
use crate::framebuffer::Framebuffer;

pub struct PpmBackend {
    path: PathBuf,
    tmp_path: PathBuf,
    width: u32,
    height: u32,
    scratch: Vec<u8>,
}

impl PpmBackend {
    pub fn new(path: PathBuf, width: u32, height: u32) -> Self {
        let mut tmp_name: OsString = path.clone().into_os_string();
        tmp_name.push(".tmp");
        Self {
            path,
            tmp_path: PathBuf::from(tmp_name),
            width,
            height,
            scratch: Vec::new(),
        }
    }
}

impl Backend for PpmBackend {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn present(&mut self, framebuffer: &Framebuffer) -> io::Result<()> {
        self.scratch.clear();
        self.scratch.extend_from_slice(
            format!(
                "P6\n{} {}\n255\n",
                framebuffer.width(),
                framebuffer.height()
            )
            .as_bytes(),
        );
        for &pixel in framebuffer.pixels() {
            self.scratch.push((pixel >> 16) as u8);
            self.scratch.push((pixel >> 8) as u8);
            self.scratch.push(pixel as u8);
        }
        fs::write(&self.tmp_path, &self.scratch)?;
        fs::rename(&self.tmp_path, &self.path)
    }
}
