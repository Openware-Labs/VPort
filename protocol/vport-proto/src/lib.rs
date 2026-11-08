// VPort — VPort proto
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::fmt;

pub const PROTOCOL_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 8;
pub const MAX_PAYLOAD: u32 = 1 << 20;
pub const MAX_TEXT_BYTES: usize = 256;
pub const MAX_IDENTITY_BYTES: usize = 128;

pub const IMAGE_FORMAT_RGBA8: u8 = 0;

pub const GRAB_DENIED: u8 = 0;
pub const GRAB_GRANTED: u8 = 1;
pub const GRAB_RELEASED: u8 = 2;

pub const MOD_SHIFT: u16 = 0x0001;
pub const MOD_CTRL: u16 = 0x0002;
pub const MOD_ALT: u16 = 0x0004;

const SHOW_TEXT_FIXED: usize = 14;
const IMAGE_DATA_FIXED: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadSize {
    Fixed(usize),
    Variable { min: usize, max: usize },
}

impl PayloadSize {
    pub fn accepts(self, len: usize) -> bool {
        match self {
            Self::Fixed(expected) => len == expected,
            Self::Variable { min, max } => len >= min && len <= max,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Opcode {
    Hello = 0x0001,
    ProxyIdentity = 0x0002,
    InitPortal = 0x0010,
    ClosePortal = 0x0011,
    DrawRect = 0x0020,
    DrawImage = 0x0021,
    ShowText = 0x0022,
    CreateImage = 0x0030,
    ImageData = 0x0031,
    DestroyImage = 0x0032,
    Commit = 0x0040,
    GrabKeyboard = 0x0050,
    ReleaseKeyboard = 0x0051,
    KeyEvent = 0x8001,
    PointerButton = 0x8002,
    FocusChanged = 0x8003,
    GrabStatus = 0x8004,
}

impl Opcode {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0x0001 => Some(Self::Hello),
            0x0002 => Some(Self::ProxyIdentity),
            0x0010 => Some(Self::InitPortal),
            0x0011 => Some(Self::ClosePortal),
            0x0020 => Some(Self::DrawRect),
            0x0021 => Some(Self::DrawImage),
            0x0022 => Some(Self::ShowText),
            0x0030 => Some(Self::CreateImage),
            0x0031 => Some(Self::ImageData),
            0x0032 => Some(Self::DestroyImage),
            0x0040 => Some(Self::Commit),
            0x0050 => Some(Self::GrabKeyboard),
            0x0051 => Some(Self::ReleaseKeyboard),
            0x8001 => Some(Self::KeyEvent),
            0x8002 => Some(Self::PointerButton),
            0x8003 => Some(Self::FocusChanged),
            0x8004 => Some(Self::GrabStatus),
            _ => None,
        }
    }

    pub fn payload_size(self) -> PayloadSize {
        match self {
            Self::Hello => PayloadSize::Fixed(4),
            Self::ProxyIdentity => PayloadSize::Variable {
                min: 1,
                max: MAX_IDENTITY_BYTES,
            },
            Self::InitPortal => PayloadSize::Fixed(12),
            Self::ClosePortal => PayloadSize::Fixed(4),
            Self::DrawRect => PayloadSize::Fixed(16),
            Self::DrawImage => PayloadSize::Fixed(12),
            Self::ShowText => PayloadSize::Variable {
                min: SHOW_TEXT_FIXED + 1,
                max: SHOW_TEXT_FIXED + MAX_TEXT_BYTES,
            },
            Self::CreateImage => PayloadSize::Fixed(9),
            Self::ImageData => PayloadSize::Variable {
                min: IMAGE_DATA_FIXED + 1,
                max: MAX_PAYLOAD as usize,
            },
            Self::DestroyImage => PayloadSize::Fixed(4),
            Self::Commit => PayloadSize::Fixed(4),
            Self::GrabKeyboard => PayloadSize::Fixed(4),
            Self::ReleaseKeyboard => PayloadSize::Fixed(4),
            Self::KeyEvent => PayloadSize::Fixed(10),
            Self::PointerButton => PayloadSize::Fixed(10),
            Self::FocusChanged => PayloadSize::Fixed(5),
            Self::GrabStatus => PayloadSize::Fixed(5),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProtoError {
    Truncated,
    PayloadTooLarge(u32),
    UnknownOpcode(u16),
    BadPayloadLength {
        opcode: u16,
        expected: usize,
        got: usize,
    },
    PayloadOutOfRange {
        opcode: u16,
        min: usize,
        max: usize,
        got: usize,
    },
    InvalidUtf8,
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated header"),
            Self::PayloadTooLarge(n) => write!(f, "payload too large: {n} bytes"),
            Self::UnknownOpcode(op) => write!(f, "unknown opcode: {op:#06x}"),
            Self::BadPayloadLength {
                opcode,
                expected,
                got,
            } => write!(
                f,
                "bad payload length for opcode {opcode:#06x}: expected {expected}, got {got}"
            ),
            Self::PayloadOutOfRange {
                opcode,
                min,
                max,
                got,
            } => write!(
                f,
                "payload length out of range for opcode {opcode:#06x}: expected {min} to {max}, got {got}"
            ),
            Self::InvalidUtf8 => write!(f, "text is not valid UTF-8"),
        }
    }
}

impl std::error::Error for ProtoError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub opcode: u16,
    pub flags: u16,
    pub payload_len: u32,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut out = [0u8; HEADER_SIZE];
        out[0..2].copy_from_slice(&self.opcode.to_le_bytes());
        out[2..4].copy_from_slice(&self.flags.to_le_bytes());
        out[4..8].copy_from_slice(&self.payload_len.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        if buf.len() < HEADER_SIZE {
            return Err(ProtoError::Truncated);
        }
        let opcode = u16::from_le_bytes([buf[0], buf[1]]);
        let flags = u16::from_le_bytes([buf[2], buf[3]]);
        let payload_len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if payload_len > MAX_PAYLOAD {
            return Err(ProtoError::PayloadTooLarge(payload_len));
        }
        Ok(Self {
            opcode,
            flags,
            payload_len,
        })
    }
}

pub fn frame_len(buf: &[u8]) -> Result<Option<usize>, ProtoError> {
    if buf.len() < HEADER_SIZE {
        return Ok(None);
    }
    let header = Header::decode(buf)?;
    Ok(Some(HEADER_SIZE + header.payload_len as usize))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    pub version: u16,
    pub capabilities: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyIdentity {
    pub identity: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitPortal {
    pub portal_id: u32,
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClosePortal {
    pub portal_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawRect {
    pub portal_id: u32,
    pub x: i16,
    pub y: i16,
    pub w: u16,
    pub h: u16,
    pub rgba: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawImage {
    pub portal_id: u32,
    pub image_id: u32,
    pub x: i16,
    pub y: i16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShowText {
    pub portal_id: u32,
    pub x: i16,
    pub y: i16,
    pub height: u16,
    pub rgba: u32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateImage {
    pub image_id: u32,
    pub width: u16,
    pub height: u16,
    pub format: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData {
    pub image_id: u32,
    pub offset: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DestroyImage {
    pub image_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Commit {
    pub portal_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrabKeyboard {
    pub portal_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseKeyboard {
    pub portal_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub portal_id: u32,
    pub scancode: u16,
    pub modifiers: u16,
    pub pressed: bool,
    pub repeat: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerButton {
    pub portal_id: u32,
    pub x: i16,
    pub y: i16,
    pub button: u8,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusChanged {
    pub portal_id: u32,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrabStatus {
    pub portal_id: u32,
    pub status: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Hello(Hello),
    ProxyIdentity(ProxyIdentity),
    InitPortal(InitPortal),
    ClosePortal(ClosePortal),
    DrawRect(DrawRect),
    DrawImage(DrawImage),
    ShowText(ShowText),
    CreateImage(CreateImage),
    ImageData(ImageData),
    DestroyImage(DestroyImage),
    Commit(Commit),
    GrabKeyboard(GrabKeyboard),
    ReleaseKeyboard(ReleaseKeyboard),
    KeyEvent(KeyEvent),
    PointerButton(PointerButton),
    FocusChanged(FocusChanged),
    GrabStatus(GrabStatus),
}

impl Message {
    pub fn opcode(&self) -> Opcode {
        match self {
            Self::Hello(_) => Opcode::Hello,
            Self::ProxyIdentity(_) => Opcode::ProxyIdentity,
            Self::InitPortal(_) => Opcode::InitPortal,
            Self::ClosePortal(_) => Opcode::ClosePortal,
            Self::DrawRect(_) => Opcode::DrawRect,
            Self::DrawImage(_) => Opcode::DrawImage,
            Self::ShowText(_) => Opcode::ShowText,
            Self::CreateImage(_) => Opcode::CreateImage,
            Self::ImageData(_) => Opcode::ImageData,
            Self::DestroyImage(_) => Opcode::DestroyImage,
            Self::Commit(_) => Opcode::Commit,
            Self::GrabKeyboard(_) => Opcode::GrabKeyboard,
            Self::ReleaseKeyboard(_) => Opcode::ReleaseKeyboard,
            Self::KeyEvent(_) => Opcode::KeyEvent,
            Self::PointerButton(_) => Opcode::PointerButton,
            Self::FocusChanged(_) => Opcode::FocusChanged,
            Self::GrabStatus(_) => Opcode::GrabStatus,
        }
    }

    fn payload_len(&self) -> usize {
        match self {
            Self::ProxyIdentity(m) => m.identity.len(),
            Self::ImageData(m) => IMAGE_DATA_FIXED + m.data.len(),
            Self::ShowText(m) => SHOW_TEXT_FIXED + m.text.len(),
            other => match other.opcode().payload_size() {
                PayloadSize::Fixed(len) => len,
                PayloadSize::Variable { min, .. } => min,
            },
        }
    }

    pub fn encode_into(&self, out: &mut Vec<u8>) {
        let header = Header {
            opcode: self.opcode() as u16,
            flags: 0,
            payload_len: self.payload_len() as u32,
        };
        out.extend_from_slice(&header.encode());
        match self {
            Self::Hello(m) => {
                put_u16(out, m.version);
                put_u16(out, m.capabilities);
            }
            Self::ProxyIdentity(m) => out.extend_from_slice(m.identity.as_bytes()),
            Self::InitPortal(m) => {
                put_u32(out, m.portal_id);
                put_i16(out, m.x);
                put_i16(out, m.y);
                put_u16(out, m.w);
                put_u16(out, m.h);
            }
            Self::ClosePortal(m) => put_u32(out, m.portal_id),
            Self::DrawRect(m) => {
                put_u32(out, m.portal_id);
                put_i16(out, m.x);
                put_i16(out, m.y);
                put_u16(out, m.w);
                put_u16(out, m.h);
                put_u32(out, m.rgba);
            }
            Self::DrawImage(m) => {
                put_u32(out, m.portal_id);
                put_u32(out, m.image_id);
                put_i16(out, m.x);
                put_i16(out, m.y);
            }
            Self::ShowText(m) => {
                put_u32(out, m.portal_id);
                put_i16(out, m.x);
                put_i16(out, m.y);
                put_u16(out, m.height);
                put_u32(out, m.rgba);
                out.extend_from_slice(m.text.as_bytes());
            }
            Self::CreateImage(m) => {
                put_u32(out, m.image_id);
                put_u16(out, m.width);
                put_u16(out, m.height);
                put_u8(out, m.format);
            }
            Self::ImageData(m) => {
                put_u32(out, m.image_id);
                put_u32(out, m.offset);
                out.extend_from_slice(&m.data);
            }
            Self::DestroyImage(m) => put_u32(out, m.image_id),
            Self::Commit(m) => put_u32(out, m.portal_id),
            Self::GrabKeyboard(m) => put_u32(out, m.portal_id),
            Self::ReleaseKeyboard(m) => put_u32(out, m.portal_id),
            Self::KeyEvent(m) => {
                put_u32(out, m.portal_id);
                put_u16(out, m.scancode);
                put_u16(out, m.modifiers);
                put_u8(out, m.pressed as u8);
                put_u8(out, m.repeat as u8);
            }
            Self::PointerButton(m) => {
                put_u32(out, m.portal_id);
                put_i16(out, m.x);
                put_i16(out, m.y);
                put_u8(out, m.button);
                put_u8(out, m.pressed as u8);
            }
            Self::FocusChanged(m) => {
                put_u32(out, m.portal_id);
                put_u8(out, m.focused as u8);
            }
            Self::GrabStatus(m) => {
                put_u32(out, m.portal_id);
                put_u8(out, m.status);
            }
        }
    }

    pub fn decode(header: &Header, payload: &[u8]) -> Result<Self, ProtoError> {
        let opcode =
            Opcode::from_u16(header.opcode).ok_or(ProtoError::UnknownOpcode(header.opcode))?;
        let size = opcode.payload_size();
        if !size.accepts(payload.len()) {
            return Err(match size {
                PayloadSize::Fixed(expected) => ProtoError::BadPayloadLength {
                    opcode: header.opcode,
                    expected,
                    got: payload.len(),
                },
                PayloadSize::Variable { min, max } => ProtoError::PayloadOutOfRange {
                    opcode: header.opcode,
                    min,
                    max,
                    got: payload.len(),
                },
            });
        }
        let mut r = Reader {
            buf: payload,
            pos: 0,
        };
        Ok(match opcode {
            Opcode::Hello => Self::Hello(Hello {
                version: r.u16(),
                capabilities: r.u16(),
            }),
            Opcode::ProxyIdentity => {
                let identity =
                String::from_utf8(payload.to_vec()).map_err(|_| ProtoError::InvalidUtf8)?;
                Self::ProxyIdentity(ProxyIdentity { identity })
            }
            Opcode::InitPortal => Self::InitPortal(InitPortal {
                portal_id: r.u32(),
                x: r.i16(),
                y: r.i16(),
                w: r.u16(),
                h: r.u16(),
            }),
            Opcode::ClosePortal => Self::ClosePortal(ClosePortal { portal_id: r.u32() }),
            Opcode::DrawRect => Self::DrawRect(DrawRect {
                portal_id: r.u32(),
                x: r.i16(),
                y: r.i16(),
                w: r.u16(),
                h: r.u16(),
                rgba: r.u32(),
            }),
            Opcode::DrawImage => Self::DrawImage(DrawImage {
                portal_id: r.u32(),
                image_id: r.u32(),
                x: r.i16(),
                y: r.i16(),
            }),
            Opcode::ShowText => {
                let portal_id = r.u32();
                let x = r.i16();
                let y = r.i16();
                let height = r.u16();
                let rgba = r.u32();
                let text = String::from_utf8(payload[SHOW_TEXT_FIXED..].to_vec())
                    .map_err(|_| ProtoError::InvalidUtf8)?;
                Self::ShowText(ShowText {
                    portal_id,
                    x,
                    y,
                    height,
                    rgba,
                    text,
                })
            }
            Opcode::CreateImage => Self::CreateImage(CreateImage {
                image_id: r.u32(),
                width: r.u16(),
                height: r.u16(),
                format: r.u8(),
            }),
            Opcode::ImageData => {
                let image_id = r.u32();
                let offset = r.u32();
                Self::ImageData(ImageData {
                    image_id,
                    offset,
                    data: payload[IMAGE_DATA_FIXED..].to_vec(),
                })
            }
            Opcode::DestroyImage => Self::DestroyImage(DestroyImage { image_id: r.u32() }),
            Opcode::Commit => Self::Commit(Commit { portal_id: r.u32() }),
            Opcode::GrabKeyboard => Self::GrabKeyboard(GrabKeyboard { portal_id: r.u32() }),
            Opcode::ReleaseKeyboard => {
                Self::ReleaseKeyboard(ReleaseKeyboard { portal_id: r.u32() })
            }
            Opcode::KeyEvent => Self::KeyEvent(KeyEvent {
                portal_id: r.u32(),
                scancode: r.u16(),
                modifiers: r.u16(),
                pressed: r.u8() != 0,
                repeat: r.u8() != 0,
            }),
            Opcode::PointerButton => Self::PointerButton(PointerButton {
                portal_id: r.u32(),
                x: r.i16(),
                y: r.i16(),
                button: r.u8(),
                pressed: r.u8() != 0,
            }),
            Opcode::FocusChanged => Self::FocusChanged(FocusChanged {
                portal_id: r.u32(),
                focused: r.u8() != 0,
            }),
            Opcode::GrabStatus => Self::GrabStatus(GrabStatus {
                portal_id: r.u32(),
                status: r.u8(),
            }),
        })
    }
}

fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> [u8; N] {
        let mut a = [0u8; N];
        a.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        a
    }

    fn u8(&mut self) -> u8 {
        let [byte] = self.take::<1>();
        byte
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take())
    }

    fn i16(&mut self) -> i16 {
        i16::from_le_bytes(self.take())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(message: &Message) -> Vec<u8> {
        let mut buf = Vec::new();
        message.encode_into(&mut buf);
        buf
    }

    fn roundtrip(message: Message) {
        let buf = encode(&message);
        assert_eq!(frame_len(&buf), Ok(Some(buf.len())));
        let header = Header::decode(&buf).unwrap();
        let decoded = Message::decode(&header, &buf[HEADER_SIZE..]).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn roundtrip_all_messages() {
        roundtrip(Message::Hello(Hello {
            version: PROTOCOL_VERSION,
            capabilities: 0,
        }));
        roundtrip(Message::ProxyIdentity(ProxyIdentity {
            identity: "totem-12".to_string(),
        }));
        roundtrip(Message::InitPortal(InitPortal {
            portal_id: 7,
            x: -10,
            y: 20,
            w: 640,
            h: 360,
        }));
        roundtrip(Message::ClosePortal(ClosePortal { portal_id: 7 }));
        roundtrip(Message::DrawRect(DrawRect {
            portal_id: 7,
            x: 1,
            y: -2,
            w: 300,
            h: 100,
            rgba: 0xdeadbeef,
        }));
        roundtrip(Message::DrawImage(DrawImage {
            portal_id: 7,
            image_id: 3,
            x: -4,
            y: 9,
        }));
        roundtrip(Message::ShowText(ShowText {
            portal_id: 7,
            x: 10,
            y: 20,
            height: 80,
            rgba: 0xffffffff,
            text: "R$ 12,90".to_string(),
        }));
        roundtrip(Message::CreateImage(CreateImage {
            image_id: 3,
            width: 256,
            height: 128,
            format: IMAGE_FORMAT_RGBA8,
        }));
        roundtrip(Message::ImageData(ImageData {
            image_id: 3,
            offset: 4096,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        }));
        roundtrip(Message::DestroyImage(DestroyImage { image_id: 3 }));
        roundtrip(Message::Commit(Commit { portal_id: 7 }));
        roundtrip(Message::GrabKeyboard(GrabKeyboard { portal_id: 7 }));
        roundtrip(Message::ReleaseKeyboard(ReleaseKeyboard { portal_id: 7 }));
        roundtrip(Message::KeyEvent(KeyEvent {
            portal_id: 7,
            scancode: 0x1e,
            modifiers: MOD_SHIFT | MOD_ALT,
            pressed: true,
            repeat: false,
        }));
        roundtrip(Message::PointerButton(PointerButton {
            portal_id: 7,
            x: 25,
            y: -3,
            button: 1,
            pressed: true,
        }));
        roundtrip(Message::FocusChanged(FocusChanged {
            portal_id: 7,
            focused: true,
        }));
        roundtrip(Message::GrabStatus(GrabStatus {
            portal_id: 7,
            status: GRAB_GRANTED,
        }));
    }

    #[test]
    fn partial_header_needs_more_bytes() {
        assert_eq!(frame_len(&[1, 0, 0]), Ok(None));
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let header = Header {
            opcode: Opcode::Hello as u16,
            flags: 0,
            payload_len: MAX_PAYLOAD + 1,
        };
        assert_eq!(
            Header::decode(&header.encode()),
            Err(ProtoError::PayloadTooLarge(MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn wrong_payload_length_is_rejected() {
        let header = Header {
            opcode: Opcode::DrawRect as u16,
            flags: 0,
            payload_len: 4,
        };
        assert!(matches!(
            Message::decode(&header, &[0u8; 4]),
            Err(ProtoError::BadPayloadLength { .. })
        ));
    }

    #[test]
    fn unknown_opcode_is_rejected() {
        let header = Header {
            opcode: 0x7777,
            flags: 0,
            payload_len: 0,
        };
        assert_eq!(
            Message::decode(&header, &[]),
            Err(ProtoError::UnknownOpcode(0x7777))
        );
    }

    #[test]
    fn text_longer_than_limit_is_rejected() {
        let payload = vec![b'a'; SHOW_TEXT_FIXED + MAX_TEXT_BYTES + 1];
        let header = Header {
            opcode: Opcode::ShowText as u16,
            flags: 0,
            payload_len: payload.len() as u32,
        };
        assert!(matches!(
            Message::decode(&header, &payload),
            Err(ProtoError::PayloadOutOfRange { .. })
        ));
    }

    #[test]
    fn empty_text_is_rejected() {
        let payload = vec![0u8; SHOW_TEXT_FIXED];
        let header = Header {
            opcode: Opcode::ShowText as u16,
            flags: 0,
            payload_len: payload.len() as u32,
        };
        assert!(matches!(
            Message::decode(&header, &payload),
            Err(ProtoError::PayloadOutOfRange { .. })
        ));
    }

    #[test]
    fn invalid_utf8_text_is_rejected() {
        let mut payload = vec![0u8; SHOW_TEXT_FIXED];
        payload.push(0xff);
        let header = Header {
            opcode: Opcode::ShowText as u16,
            flags: 0,
            payload_len: payload.len() as u32,
        };
        assert_eq!(
            Message::decode(&header, &payload),
            Err(ProtoError::InvalidUtf8)
        );
    }

    #[test]
    fn identity_must_be_between_one_and_128_bytes() {
        for len in [0usize, MAX_IDENTITY_BYTES + 1] {
            let payload = vec![b'a'; len];
            let header = Header {
                opcode: Opcode::ProxyIdentity as u16,
                flags: 0,
                payload_len: len as u32,
            };
            assert!(matches!(
                Message::decode(&header, &payload),
                             Err(ProtoError::PayloadOutOfRange { .. })
            ));
        }
    }
}
