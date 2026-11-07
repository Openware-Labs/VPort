// VPort — Libvport
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

const DIGIT_SYMBOLS: [char; 10] = ['!', '@', '#', '$', '%', '^', '&', '*', '(', ')'];

pub fn hid_to_ascii(scancode: u16, shift: bool) -> Option<char> {
    match scancode {
        0x04..=0x1d => {
            let letter = (b'a' + (scancode - 0x04) as u8) as char;
            Some(if shift {
                letter.to_ascii_uppercase()
            } else {
                letter
            })
        }
        0x1e..=0x27 => {
            let index = (scancode - 0x1e) as usize;
            if shift {
                Some(DIGIT_SYMBOLS[index])
            } else if index == 9 {
                Some('0')
            } else {
                Some((b'1' + index as u8) as char)
            }
        }
        0x28 => Some('\n'),
        0x2c => Some(' '),
        0x2d => Some(if shift { '_' } else { '-' }),
        0x37 => Some(if shift { '>' } else { '.' }),
        0x38 => Some(if shift { '?' } else { '/' }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_and_enter() {
        assert_eq!(hid_to_ascii(0x1e, false), Some('1'));
        assert_eq!(hid_to_ascii(0x26, false), Some('9'));
        assert_eq!(hid_to_ascii(0x27, false), Some('0'));
        assert_eq!(hid_to_ascii(0x28, false), Some('\n'));
    }

    #[test]
    fn letters_follow_shift() {
        assert_eq!(hid_to_ascii(0x04, false), Some('a'));
        assert_eq!(hid_to_ascii(0x04, true), Some('A'));
        assert_eq!(hid_to_ascii(0x1d, false), Some('z'));
    }

    #[test]
    fn unknown_scancode_is_none() {
        assert_eq!(hid_to_ascii(0x00, false), None);
        assert_eq!(hid_to_ascii(0xe0, false), None);
    }
}
