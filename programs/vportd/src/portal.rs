// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::BTreeMap;

use vport_proto::{DrawImage, DrawRect, InitPortal, ShowText};

use crate::image::Image;

pub const MAX_PENDING_COMMANDS: usize = 4096;

pub enum Command {
    Rect(DrawRect),
    Image(DrawImage),
    Text(ShowText),
}

pub struct Portal {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pending: Vec<Command>,
    committed: Vec<Command>,
}

impl Portal {
    pub fn new(init: &InitPortal) -> Self {
        Self {
            x: init.x,
            y: init.y,
            w: init.w,
            h: init.h,
            pending: Vec::new(),
            committed: Vec::new(),
        }
    }

    pub fn push(&mut self, command: Command) -> bool {
        if self.pending.len() >= MAX_PENDING_COMMANDS {
            return false;
        }
        self.pending.push(command);
        true
    }

    pub fn commit(&mut self) {
        self.committed = std::mem::take(&mut self.pending);
    }

    pub fn committed(&self) -> &[Command] {
        &self.committed
    }
}

pub struct Scene<'a> {
    pub portal: &'a Portal,
    pub images: &'a BTreeMap<u32, Image>,
}
