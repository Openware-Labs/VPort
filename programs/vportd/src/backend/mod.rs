// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

#[cfg(feature = "kms")]
mod evdev;
#[cfg(feature = "kms")]
mod kms;
mod ppm;
#[cfg(feature = "sdl2")]
mod sdl;
#[cfg(feature = "kms")]
mod sys;

use std::io;

use crate::framebuffer::Framebuffer;

#[cfg(feature = "kms")]
pub use kms::KmsBackend;
pub use ppm::PpmBackend;
#[cfg(feature = "sdl2")]
pub use sdl::SdlBackend;

#[cfg_attr(not(feature = "sdl2"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendEvent {
    Quit,
    Redraw,
    Key {
        scancode: u16,
        modifiers: u16,
        pressed: bool,
        repeat: bool,
    },
    Button {
        x: i32,
        y: i32,
        button: u8,
        pressed: bool,
    },
}

pub trait Backend {
    fn size(&self) -> (u32, u32);

    fn present(&mut self, framebuffer: &Framebuffer) -> io::Result<()>;

    fn needs_polling(&self) -> bool {
        false
    }

    fn poll_events(&mut self, _out: &mut Vec<BackendEvent>) {}
}
