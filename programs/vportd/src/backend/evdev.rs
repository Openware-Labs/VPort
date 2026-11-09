// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::fs::{self, File, OpenOptions};
use std::io;
use std::mem::{size_of, size_of_val};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use vport_proto::{MOD_ALT, MOD_CTRL, MOD_SHIFT};

use super::sys::{ioc, IOC_READ, IOC_WRITE};
use super::BackendEvent;

const EV_KEY: u16 = 0x01;
const KEY_1: usize = 2;
const KEY_ENTER: usize = 28;
const KEY_BACKSPACE: u16 = 14;
const KEY_BYTES: usize = 0x300 / 8;
const EVENTS_PER_READ: usize = 64;
const EVIOCGRAB: u64 = ioc(IOC_WRITE, b'E', 0x90, size_of::<libc::c_int>());

const LEFT_SHIFT: u8 = 1;
const RIGHT_SHIFT: u8 = 2;
const LEFT_CTRL: u8 = 4;
const RIGHT_CTRL: u8 = 8;
const LEFT_ALT: u8 = 16;
const RIGHT_ALT: u8 = 32;

#[derive(Default)]
struct Modifiers {
    held: u8,
}

impl Modifiers {
    fn bit(code: u16) -> u8 {
        match code {
            42 => LEFT_SHIFT,
            54 => RIGHT_SHIFT,
            29 => LEFT_CTRL,
            97 => RIGHT_CTRL,
            56 => LEFT_ALT,
            100 => RIGHT_ALT,
            _ => 0,
        }
    }

    fn update(&mut self, code: u16, pressed: bool) {
        let bit = Self::bit(code);
        if bit == 0 {
            return;
        }
        if pressed {
            self.held |= bit;
        } else {
            self.held &= !bit;
        }
    }

    fn protocol(&self) -> u16 {
        let mut bits = 0;
        if self.held & (LEFT_SHIFT | RIGHT_SHIFT) != 0 {
            bits |= MOD_SHIFT;
        }
        if self.held & (LEFT_CTRL | RIGHT_CTRL) != 0 {
            bits |= MOD_CTRL;
        }
        if self.held & (LEFT_ALT | RIGHT_ALT) != 0 {
            bits |= MOD_ALT;
        }
        bits
    }

    fn emergency_exit(&self, code: u16, pressed: bool) -> bool {
        pressed
            && code == KEY_BACKSPACE
            && self.held & (LEFT_CTRL | RIGHT_CTRL) != 0
            && self.held & (LEFT_ALT | RIGHT_ALT) != 0
    }
}

pub struct Keyboards {
    devices: Vec<File>,
    modifiers: Modifiers,
}

impl Keyboards {
    pub fn open(grab: bool) -> Self {
        let mut devices = Vec::new();
        let entries = match fs::read_dir("/dev/input") {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("vportd: cannot list /dev/input: {e}");
                return Self {
                    devices,
                    modifiers: Modifiers::default(),
                };
            }
        };
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();

        for path in paths {
            let is_event = path
                .file_name()
                .and_then(|name| name.to_str())
                .map_or(false, |name| name.starts_with("event"));
            if !is_event {
                continue;
            }
            let file = match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(&path)
            {
                Ok(file) => file,
                Err(e) => {
                    eprintln!("vportd: cannot open {}: {e}", path.display());
                    continue;
                }
            };
            if !is_keyboard(&file) {
                continue;
            }
            if grab && !grab_device(&file) {
                eprintln!("vportd: cannot grab {}", path.display());
                continue;
            }
            eprintln!(
                "vportd: keyboard {}{}",
                path.display(),
                if grab { " (exclusive)" } else { "" }
            );
            devices.push(file);
        }
        if devices.is_empty() {
            eprintln!("vportd: no keyboard found");
        }
        Self {
            devices,
            modifiers: Modifiers::default(),
        }
    }

    pub fn poll(&mut self, out: &mut Vec<BackendEvent>) -> bool {
        let mut quit = false;
        let mut dead: Vec<usize> = Vec::new();
        let mut events: [libc::input_event; EVENTS_PER_READ] = unsafe { std::mem::zeroed() };
        let capacity = size_of_val(&events);
        let event_size = size_of::<libc::input_event>();

        for (index, file) in self.devices.iter().enumerate() {
            loop {
                let read = unsafe {
                    libc::read(
                        file.as_raw_fd(),
                        events.as_mut_ptr() as *mut libc::c_void,
                        capacity,
                    )
                };
                if read < 0 {
                    let err = io::Error::last_os_error();
                    match err.kind() {
                        io::ErrorKind::WouldBlock => break,
                        io::ErrorKind::Interrupted => continue,
                        _ => {
                            dead.push(index);
                            break;
                        }
                    }
                }
                if read == 0 {
                    dead.push(index);
                    break;
                }
                let count = read as usize / event_size;
                for event in &events[..count] {
                    if event.type_ != EV_KEY {
                        continue;
                    }
                    let pressed = event.value != 0;
                    self.modifiers.update(event.code, pressed);
                    if self.modifiers.emergency_exit(event.code, pressed) {
                        quit = true;
                    }
                    if let Some(scancode) = evdev_to_hid(event.code) {
                        out.push(BackendEvent::Key {
                            scancode,
                            modifiers: self.modifiers.protocol(),
                            pressed,
                            repeat: event.value == 2,
                        });
                    }
                }
            }
        }

        for index in dead.into_iter().rev() {
            self.devices.remove(index);
            eprintln!("vportd: keyboard disconnected");
        }
        quit
    }
}

fn test_bit(bits: &[u8], bit: usize) -> bool {
    bits.get(bit / 8)
        .map_or(false, |byte| byte & (1 << (bit % 8)) != 0)
}

fn is_keyboard(file: &File) -> bool {
    let mut bits = [0u8; KEY_BYTES];
    let request = ioc(IOC_READ, b'E', 0x20 + EV_KEY as u32, KEY_BYTES);
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), request as _, bits.as_mut_ptr()) };
    rc >= 0 && test_bit(&bits, KEY_ENTER) && test_bit(&bits, KEY_1)
}

fn grab_device(file: &File) -> bool {
    let value: libc::c_ulong = 1;
    unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGRAB as _, value) >= 0 }
}

pub fn evdev_to_hid(code: u16) -> Option<u16> {
    let hid = match code {
        30 => 0x04,
        48 => 0x05,
        46 => 0x06,
        32 => 0x07,
        18 => 0x08,
        33 => 0x09,
        34 => 0x0a,
        35 => 0x0b,
        23 => 0x0c,
        36 => 0x0d,
        37 => 0x0e,
        38 => 0x0f,
        50 => 0x10,
        49 => 0x11,
        24 => 0x12,
        25 => 0x13,
        16 => 0x14,
        19 => 0x15,
        31 => 0x16,
        20 => 0x17,
        22 => 0x18,
        47 => 0x19,
        17 => 0x1a,
        45 => 0x1b,
        21 => 0x1c,
        44 => 0x1d,
        2..=10 => 0x1e + (code - 2),
        11 => 0x27,
        28 => 0x28,
        1 => 0x29,
        14 => 0x2a,
        15 => 0x2b,
        57 => 0x2c,
        12 => 0x2d,
        13 => 0x2e,
        26 => 0x2f,
        27 => 0x30,
        43 => 0x31,
        39 => 0x33,
        40 => 0x34,
        41 => 0x35,
        51 => 0x36,
        52 => 0x37,
        53 => 0x38,
        58 => 0x39,
        59..=68 => 0x3a + (code - 59),
        87 => 0x44,
        88 => 0x45,
        110 => 0x49,
        102 => 0x4a,
        104 => 0x4b,
        111 => 0x4c,
        107 => 0x4d,
        109 => 0x4e,
        106 => 0x4f,
        105 => 0x50,
        108 => 0x51,
        103 => 0x52,
        98 => 0x54,
        55 => 0x55,
        74 => 0x56,
        78 => 0x57,
        96 => 0x58,
        79 => 0x59,
        80 => 0x5a,
        81 => 0x5b,
        75 => 0x5c,
        76 => 0x5d,
        77 => 0x5e,
        71 => 0x5f,
        72 => 0x60,
        73 => 0x61,
        82 => 0x62,
        83 => 0x63,
        29 => 0xe0,
        42 => 0xe1,
        56 => 0xe2,
        125 => 0xe3,
        97 => 0xe4,
        54 => 0xe5,
        100 => 0xe6,
        126 => 0xe7,
        _ => return None,
    };
    Some(hid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_map_to_hid_digits() {
        assert_eq!(evdev_to_hid(2), Some(0x1e));
        assert_eq!(evdev_to_hid(10), Some(0x26));
        assert_eq!(evdev_to_hid(11), Some(0x27));
    }

    #[test]
    fn letters_and_enter_map_to_hid() {
        assert_eq!(evdev_to_hid(30), Some(0x04));
        assert_eq!(evdev_to_hid(44), Some(0x1d));
        assert_eq!(evdev_to_hid(28), Some(0x28));
    }

    #[test]
    fn keypad_digits_map_to_hid_keypad() {
        assert_eq!(evdev_to_hid(79), Some(0x59));
        assert_eq!(evdev_to_hid(82), Some(0x62));
        assert_eq!(evdev_to_hid(96), Some(0x58));
    }

    #[test]
    fn unknown_code_is_none() {
        assert_eq!(evdev_to_hid(0), None);
        assert_eq!(evdev_to_hid(0x2ff), None);
    }

    #[test]
    fn emergency_chord_needs_ctrl_alt_and_backspace() {
        let mut modifiers = Modifiers::default();
        modifiers.update(29, true);
        assert!(!modifiers.emergency_exit(KEY_BACKSPACE, true));
        modifiers.update(56, true);
        assert!(modifiers.emergency_exit(KEY_BACKSPACE, true));
        assert!(!modifiers.emergency_exit(KEY_BACKSPACE, false));
        assert!(!modifiers.emergency_exit(30, true));
    }
}
