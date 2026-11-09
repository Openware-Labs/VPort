// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

pub const IOC_NONE: u32 = 0;
pub const IOC_WRITE: u32 = 1;
pub const IOC_READ: u32 = 2;

pub const fn ioc(direction: u32, kind: u8, number: u32, size: usize) -> u64 {
    ((direction as u64) << 30) | ((size as u64) << 16) | ((kind as u64) << 8) | number as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_grab_request_matches_the_kernel() {
        assert_eq!(ioc(IOC_WRITE, b'E', 0x90, 4), 0x4004_4590);
    }

    #[test]
    fn drm_set_master_request_matches_the_kernel() {
        assert_eq!(ioc(IOC_NONE, b'd', 0x1e, 0), 0x641e);
    }
}
