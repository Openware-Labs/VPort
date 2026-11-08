// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::io;

use crate::backend::{Backend, BackendEvent};
use crate::framebuffer::{Framebuffer, Rect};
use crate::portal::{Command, Scene};
use crate::text::TextRenderer;

const BACKGROUND: u32 = 0x000000;

pub struct Compositor {
    framebuffer: Framebuffer,
    backend: Box<dyn Backend>,
    text: TextRenderer,
}

impl Compositor {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        let (width, height) = backend.size();
        Self {
            framebuffer: Framebuffer::new(width, height),
            backend,
            text: TextRenderer::load(),
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

    pub fn render<'a, I>(&mut self, scenes: I) -> io::Result<()>
    where
        I: Iterator<Item = Scene<'a>>,
    {
        self.framebuffer.clear(BACKGROUND);
        for scene in scenes {
            let portal = scene.portal;
            let clip = Rect {
                x: portal.x as i32,
                y: portal.y as i32,
                w: portal.w as i32,
                h: portal.h as i32,
            };
            for command in portal.committed() {
                match command {
                    Command::Rect(rect) => self.framebuffer.fill_rect(
                        clip.x + rect.x as i32,
                        clip.y + rect.y as i32,
                        rect.w as i32,
                        rect.h as i32,
                        rect.rgba,
                        clip,
                    ),
                    Command::Image(draw) => {
                        if let Some(image) = scene.images.get(&draw.image_id) {
                            self.framebuffer.blit_rgba(
                                clip.x + draw.x as i32,
                                clip.y + draw.y as i32,
                                image.width as i32,
                                image.height as i32,
                                &image.pixels,
                                clip,
                            );
                        }
                    }
                    Command::Text(show) => self.text.draw(
                        &mut self.framebuffer,
                        &show.text,
                        clip.x + show.x as i32,
                        clip.y + show.y as i32,
                        show.height,
                        show.rgba,
                        clip,
                    ),
                }
            }
        }
        self.backend.present(&self.framebuffer)
    }
}
