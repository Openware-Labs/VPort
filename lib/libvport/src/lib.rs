// VPort — Libvport
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

mod keys;

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use vport_proto::{
    frame_len, ClosePortal, Commit, CreateImage, DestroyImage, DrawImage, DrawRect, GrabKeyboard,
    Header, Hello, ImageData, InitPortal, Message, ReleaseKeyboard, ShowText, HEADER_SIZE,
    IMAGE_FORMAT_RGBA8, PROTOCOL_VERSION,
};

pub use keys::hid_to_ascii;
pub use vport_proto::{
    FocusChanged, GrabStatus, KeyEvent, PointerButton, GRAB_DENIED, GRAB_GRANTED, GRAB_RELEASED,
    MAX_TEXT_BYTES, MOD_ALT, MOD_CTRL, MOD_SHIFT,
};

pub const DEFAULT_SOCKET: &str = "/run/vport/vport.sock";

const RECV_CHUNK: usize = 512;
const IMAGE_CHUNK: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key(KeyEvent),
    PointerButton(PointerButton),
    FocusChanged(FocusChanged),
    GrabStatus(GrabStatus),
}

impl Event {
    fn from_message(message: Message) -> Option<Self> {
        match message {
            Message::KeyEvent(event) => Some(Self::Key(event)),
            Message::PointerButton(event) => Some(Self::PointerButton(event)),
            Message::FocusChanged(event) => Some(Self::FocusChanged(event)),
            Message::GrabStatus(event) => Some(Self::GrabStatus(event)),
            _ => None,
        }
    }
}

pub struct Connection {
    stream: UnixStream,
    buf: Vec<u8>,
    inbuf: Vec<u8>,
    nonblocking: bool,
}

impl Connection {
    pub fn connect<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        let mut conn = Self {
            stream,
            buf: Vec::with_capacity(64),
            inbuf: Vec::with_capacity(RECV_CHUNK),
            nonblocking: false,
        };
        conn.send(&Message::Hello(Hello {
            version: PROTOCOL_VERSION,
            capabilities: 0,
        }))?;
        match conn.recv_handshake()? {
            Message::Hello(reply) if reply.version == PROTOCOL_VERSION => Ok(conn),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected handshake reply",
            )),
        }
    }

    pub fn connect_default() -> io::Result<Self> {
        let path = std::env::var("VPORT_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET.to_string());
        Self::connect(path)
    }

    pub fn init_portal(
        &mut self,
        portal_id: u32,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
    ) -> io::Result<()> {
        self.send(&Message::InitPortal(InitPortal {
            portal_id,
            x,
            y,
            w,
            h,
        }))
    }

    pub fn close_portal(&mut self, portal_id: u32) -> io::Result<()> {
        self.send(&Message::ClosePortal(ClosePortal { portal_id }))
    }

    pub fn draw_rect(
        &mut self,
        portal_id: u32,
        x: i16,
        y: i16,
        w: u16,
        h: u16,
        rgba: u32,
    ) -> io::Result<()> {
        self.send(&Message::DrawRect(DrawRect {
            portal_id,
            x,
            y,
            w,
            h,
            rgba,
        }))
    }

    pub fn draw_image(&mut self, portal_id: u32, image_id: u32, x: i16, y: i16) -> io::Result<()> {
        self.send(&Message::DrawImage(DrawImage {
            portal_id,
            image_id,
            x,
            y,
        }))
    }

    pub fn show_text(
        &mut self,
        portal_id: u32,
        x: i16,
        y: i16,
        height: u16,
        rgba: u32,
        text: &str,
    ) -> io::Result<()> {
        if text.is_empty() || text.len() > MAX_TEXT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("text must be between 1 and {MAX_TEXT_BYTES} bytes"),
            ));
        }
        self.send(&Message::ShowText(ShowText {
            portal_id,
            x,
            y,
            height,
            rgba,
            text: text.to_string(),
        }))
    }

    pub fn create_image(&mut self, image_id: u32, width: u16, height: u16) -> io::Result<()> {
        self.send(&Message::CreateImage(CreateImage {
            image_id,
            width,
            height,
            format: IMAGE_FORMAT_RGBA8,
        }))
    }

    pub fn image_data(&mut self, image_id: u32, offset: u32, data: &[u8]) -> io::Result<()> {
        self.send(&Message::ImageData(ImageData {
            image_id,
            offset,
            data: data.to_vec(),
        }))
    }

    pub fn upload_image(
        &mut self,
        image_id: u32,
        width: u16,
        height: u16,
        rgba: &[u8],
    ) -> io::Result<()> {
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pixel buffer does not match the image size",
            ));
        }
        self.create_image(image_id, width, height)?;
        let mut offset = 0usize;
        for chunk in rgba.chunks(IMAGE_CHUNK) {
            self.image_data(image_id, offset as u32, chunk)?;
            offset += chunk.len();
        }
        Ok(())
    }

    pub fn destroy_image(&mut self, image_id: u32) -> io::Result<()> {
        self.send(&Message::DestroyImage(DestroyImage { image_id }))
    }

    pub fn commit(&mut self, portal_id: u32) -> io::Result<()> {
        self.send(&Message::Commit(Commit { portal_id }))
    }

    pub fn grab_keyboard(&mut self, portal_id: u32) -> io::Result<()> {
        self.send(&Message::GrabKeyboard(GrabKeyboard { portal_id }))
    }

    pub fn release_keyboard(&mut self, portal_id: u32) -> io::Result<()> {
        self.send(&Message::ReleaseKeyboard(ReleaseKeyboard { portal_id }))
    }

    pub fn poll_event(&mut self) -> io::Result<Option<Event>> {
        loop {
            if let Some(event) = self.parse_event()? {
                return Ok(Some(event));
            }
            if !self.read_some(false)? {
                return Ok(None);
            }
        }
    }

    pub fn wait_event(&mut self) -> io::Result<Event> {
        loop {
            if let Some(event) = self.parse_event()? {
                return Ok(event);
            }
            self.read_some(true)?;
        }
    }

    fn set_nonblocking(&mut self, value: bool) -> io::Result<()> {
        if self.nonblocking != value {
            self.stream.set_nonblocking(value)?;
            self.nonblocking = value;
        }
        Ok(())
    }

    fn send(&mut self, message: &Message) -> io::Result<()> {
        self.set_nonblocking(false)?;
        self.buf.clear();
        message.encode_into(&mut self.buf);
        self.stream.write_all(&self.buf)
    }

    fn recv_handshake(&mut self) -> io::Result<Message> {
        let mut header_bytes = [0u8; HEADER_SIZE];
        self.stream.read_exact(&mut header_bytes)?;
        let header = Header::decode(&header_bytes).map_err(invalid)?;
        let mut payload = vec![0u8; header.payload_len as usize];
        self.stream.read_exact(&mut payload)?;
        Message::decode(&header, &payload).map_err(invalid)
    }

    fn read_some(&mut self, block: bool) -> io::Result<bool> {
        self.set_nonblocking(!block)?;
        let mut chunk = [0u8; RECV_CHUNK];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "server closed the connection",
                    ))
                }
                Ok(n) => {
                    self.inbuf.extend_from_slice(&chunk[..n]);
                    return Ok(true);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }

    fn parse_event(&mut self) -> io::Result<Option<Event>> {
        loop {
            let total = match frame_len(&self.inbuf).map_err(invalid)? {
                Some(total) if self.inbuf.len() >= total => total,
                _ => return Ok(None),
            };
            let header = Header::decode(&self.inbuf).map_err(invalid)?;
            let message =
                Message::decode(&header, &self.inbuf[HEADER_SIZE..total]).map_err(invalid)?;
            self.inbuf.drain(..total);
            if let Some(event) = Event::from_message(message) {
                return Ok(Some(event));
            }
        }
    }
}

fn invalid<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err.to_string())
}
