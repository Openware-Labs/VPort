// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use vport_proto::{DrawRect, InitPortal};

pub const MAX_PENDING_RECTS: usize = 4096;

pub struct Portal {
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pending: Vec<DrawRect>,
    committed: Vec<DrawRect>,
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

    pub fn push(&mut self, rect: DrawRect) -> bool {
        if self.pending.len() >= MAX_PENDING_RECTS {
            return false;
        }
        self.pending.push(rect);
        true
    }

    pub fn commit(&mut self) {
        self.committed = std::mem::take(&mut self.pending);
    }

    pub fn committed(&self) -> &[DrawRect] {
        &self.committed
    }
}
