// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn handler(_signal: libc::c_int) {
    REQUESTED.store(true, Ordering::SeqCst);
}

pub fn install() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe {
            libc::signal(
                signal,
                handler as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }
}

pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}
