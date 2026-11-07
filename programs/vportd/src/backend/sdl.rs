// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::io;

use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::{Keycode, Mod};
use sdl2::mouse::MouseButton;
use sdl2::pixels::PixelFormatEnum;
use sdl2::video::Window;
use sdl2::{EventPump, Sdl};
use vport_proto::{MOD_ALT, MOD_CTRL, MOD_SHIFT};

use super::{Backend, BackendEvent};
use crate::framebuffer::Framebuffer;

pub struct SdlBackend {
    _sdl: Sdl,
    window: Window,
    event_pump: EventPump,
    width: u32,
    height: u32,
}

impl SdlBackend {
    pub fn new(width: u32, height: u32) -> io::Result<Self> {
        let sdl = sdl2::init().map_err(other)?;
        let video = sdl.video().map_err(other)?;
        let window = video
            .window("VPort", width, height)
            .position_centered()
            .build()
            .map_err(other)?;
        let event_pump = sdl.event_pump().map_err(other)?;
        Ok(Self {
            _sdl: sdl,
            window,
            event_pump,
            width,
            height,
        })
    }
}

impl Backend for SdlBackend {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn present(&mut self, framebuffer: &Framebuffer) -> io::Result<()> {
        let mut surface = self.window.surface(&self.event_pump).map_err(other)?;
        let swap_red_blue = match surface.pixel_format_enum() {
            PixelFormatEnum::ARGB8888 | PixelFormatEnum::RGB888 => false,
            PixelFormatEnum::ABGR8888 | PixelFormatEnum::BGR888 => true,
            format => {
                return Err(other(format!(
                    "unsupported window pixel format: {format:?}"
                )));
            }
        };
        let pitch = surface.pitch() as usize;
        let width = framebuffer.width().min(surface.width()) as usize;
        let height = framebuffer.height().min(surface.height()) as usize;
        let stride = framebuffer.width() as usize;
        let bytes = surface
            .without_lock_mut()
            .ok_or_else(|| other("window surface is not directly writable"))?;
        for y in 0..height {
            let source = &framebuffer.pixels()[y * stride..y * stride + width];
            let target = &mut bytes[y * pitch..y * pitch + width * 4];
            for (index, &pixel) in source.iter().enumerate() {
                let value = if swap_red_blue { swap_rb(pixel) } else { pixel } | 0xff00_0000;
                target[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
        surface.update_window().map_err(other)
    }

    fn needs_polling(&self) -> bool {
        true
    }

    fn poll_events(&mut self, out: &mut Vec<BackendEvent>) {
        for event in self.event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => out.push(BackendEvent::Quit),
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => out.push(BackendEvent::Quit),
                Event::KeyDown {
                    scancode: Some(scancode),
                    keymod,
                    repeat,
                    ..
                } => out.push(BackendEvent::Key {
                    scancode: scancode as i32 as u16,
                    modifiers: modifiers(keymod),
                    pressed: true,
                    repeat,
                }),
                Event::KeyUp {
                    scancode: Some(scancode),
                    keymod,
                    ..
                } => out.push(BackendEvent::Key {
                    scancode: scancode as i32 as u16,
                    modifiers: modifiers(keymod),
                    pressed: false,
                    repeat: false,
                }),
                Event::MouseButtonDown {
                    mouse_btn, x, y, ..
                } => out.push(BackendEvent::Button {
                    x,
                    y,
                    button: button_code(mouse_btn),
                    pressed: true,
                }),
                Event::MouseButtonUp {
                    mouse_btn, x, y, ..
                } => out.push(BackendEvent::Button {
                    x,
                    y,
                    button: button_code(mouse_btn),
                    pressed: false,
                }),
                Event::Window {
                    win_event:
                        WindowEvent::Exposed
                        | WindowEvent::Shown
                        | WindowEvent::Restored
                        | WindowEvent::SizeChanged(..),
                    ..
                } => out.push(BackendEvent::Redraw),
                _ => {}
            }
        }
    }
}

fn modifiers(keymod: Mod) -> u16 {
    let mut bits = 0;
    if keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) {
        bits |= MOD_SHIFT;
    }
    if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) {
        bits |= MOD_CTRL;
    }
    if keymod.intersects(Mod::LALTMOD | Mod::RALTMOD) {
        bits |= MOD_ALT;
    }
    bits
}

fn button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::X1 => 4,
        MouseButton::X2 => 5,
        MouseButton::Unknown => 0,
    }
}

fn swap_rb(pixel: u32) -> u32 {
    ((pixel & 0xff) << 16) | (pixel & 0xff00) | ((pixel >> 16) & 0xff)
}

fn other<E: ToString>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}
