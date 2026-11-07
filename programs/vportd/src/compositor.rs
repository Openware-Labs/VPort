// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::io;

use crate::backend::{Backend, BackendEvent};
use crate::framebuffer::{Framebuffer, Rect};
use crate::portal::Portal;

const BACKGROUND: u32 = 0x000000;

pub struct Compositor {
    framebuffer: Framebuffer,
    backend: Box<dyn Backend>,
}

impl Compositor {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        let (width, height) = backend.size();
        Self {
            framebuffer: Framebuffer::new(width, height),
            backend,
        }
    }

    pub fn needs_polling(&self) -> bool {
        self.backend.needs_polling()
    }

    pub fn poll_events(&mut self, out: &mut Vec<BackendEvent>) {
        self.backend.poll_events(out);
    }

    pub fn present(&mut self) -> io::Result<()> {
        self.backend.present(&self.framebuffer)
    }

    pub fn render<'a, I>(&mut self, portals: I) -> io::Result<()>
    where
        I: Iterator<Item = &'a Portal>,
    {
        self.framebuffer.clear(BACKGROUND);
        for portal in portals {
            let clip = Rect {
                x: portal.x as i32,
                y: portal.y as i32,
                w: portal.w as i32,
                h: portal.h as i32,
            };
            for rect in portal.committed() {
                self.framebuffer.fill_rect(
                    clip.x + rect.x as i32,
                    clip.y + rect.y as i32,
                    rect.w as i32,
                    rect.h as i32,
                    rect.rgba,
                    clip,
                );
            }
        }
        self.backend.present(&self.framebuffer)
    }
}
