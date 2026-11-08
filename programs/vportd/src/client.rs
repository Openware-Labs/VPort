// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;

use vport_proto::{
    frame_len, Header, Hello, Message, HEADER_SIZE, IMAGE_FORMAT_RGBA8, PROTOCOL_VERSION,
};

use crate::epoll::PeerCred;
use crate::identity::{valid_remote_name, Identity};
use crate::image::Image;
use crate::portal::{Command, Portal, Scene, MAX_PENDING_COMMANDS};

const READ_CHUNK: usize = 4096;
const READ_BUDGET_PER_WAKEUP: usize = 16;
const MAX_PORTALS_PER_CLIENT: usize = 8;
const MAX_OUTPUT_BACKLOG: usize = 64 * 1024;
const MAX_IMAGES_PER_CLIENT: usize = 32;
const MAX_IMAGE_BYTES_PER_CLIENT: usize = 8 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u16 = 4096;
const MAX_TEXT_HEIGHT: u16 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    PortalCreated(u32),
    PortalClosed(u32),
    GrabKeyboard(u32),
    ReleaseKeyboard(u32),
}

pub struct Client {
    stream: UnixStream,
    peer: PeerCred,
    identity: Identity,
    via_gateway: bool,
    identified: bool,
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    greeted: bool,
    portals: BTreeMap<u32, Portal>,
    images: BTreeMap<u32, Image>,
    image_bytes: usize,
    dirty: bool,
    doomed: bool,
    write_armed: bool,
    requests: Vec<Request>,
}

impl Client {
    pub fn new(stream: UnixStream, peer: PeerCred, via_gateway: bool) -> Self {
        Self {
            stream,
            peer,
            identity: Identity::Local { uid: peer.uid },
            via_gateway,
            identified: false,
            inbuf: Vec::new(),
            outbuf: Vec::new(),
            greeted: false,
            portals: BTreeMap::new(),
            images: BTreeMap::new(),
            image_bytes: 0,
            dirty: false,
            doomed: false,
            write_armed: false,
            requests: Vec::new(),
        }
    }

    pub fn peer(&self) -> PeerCred {
        self.peer
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    pub fn raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    pub fn scenes(&self) -> impl Iterator<Item = Scene<'_>> {
        self.portals.values().map(move |portal| Scene {
            portal,
            images: &self.images,
        })
    }

    pub fn portal_entries(&self) -> impl Iterator<Item = (u32, &Portal)> {
        self.portals.iter().map(|(id, portal)| (*id, portal))
    }

    pub fn has_portals(&self) -> bool {
        !self.portals.is_empty()
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    pub fn is_doomed(&self) -> bool {
        self.doomed
    }

    pub fn doom(&mut self) {
        self.doomed = true;
    }

    pub fn wants_write(&self) -> bool {
        !self.outbuf.is_empty()
    }

    pub fn write_armed(&self) -> bool {
        self.write_armed
    }

    pub fn set_write_armed(&mut self, armed: bool) {
        self.write_armed = armed;
    }

    pub fn queue(&mut self, message: &Message) -> io::Result<()> {
        message.encode_into(&mut self.outbuf);
        if self.outbuf.len() > MAX_OUTPUT_BACKLOG {
            return Err(violation("output backlog limit exceeded"));
        }
        self.flush()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        let mut written = 0;
        let result = loop {
            if written == self.outbuf.len() {
                break Ok(());
            }
            match self.stream.write(&self.outbuf[written..]) {
                Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => break Err(e),
            }
        };
        self.outbuf.drain(..written);
        result
    }

    pub fn pump(&mut self) -> io::Result<bool> {
        let mut chunk = [0u8; READ_CHUNK];
        for _ in 0..READ_BUDGET_PER_WAKEUP {
            match self.stream.read(&mut chunk) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    self.inbuf.extend_from_slice(&chunk[..n]);
                    self.drain_frames()?;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(true),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    fn drain_frames(&mut self) -> io::Result<()> {
        let mut offset = 0;
        loop {
            let rest = &self.inbuf[offset..];
            let total = match frame_len(rest).map_err(invalid)? {
                Some(total) if rest.len() >= total => total,
                _ => break,
            };
            let header = Header::decode(rest).map_err(invalid)?;
            let message = Message::decode(&header, &rest[HEADER_SIZE..total]).map_err(invalid)?;
            offset += total;
            self.handle(message)?;
        }
        self.inbuf.drain(..offset);
        Ok(())
    }

    fn push_command(&mut self, portal_id: u32, command: Command) -> io::Result<()> {
        let portal = self
            .portals
            .get_mut(&portal_id)
            .ok_or_else(|| violation("unknown portal id"))?;
        if !portal.push(command) {
            return Err(violation(&format!(
                "more than {MAX_PENDING_COMMANDS} uncommitted commands"
            )));
        }
        Ok(())
    }

    fn handle(&mut self, message: Message) -> io::Result<()> {
        match message {
            Message::Hello(hello) => {
                if self.greeted {
                    return Err(violation("duplicate Hello"));
                }
                if self.via_gateway && !self.identified {
                    return Err(violation("gateway connection without an identity"));
                }
                if hello.version != PROTOCOL_VERSION {
                    return Err(violation("unsupported protocol version"));
                }
                self.greeted = true;
                self.queue(&Message::Hello(Hello {
                    version: PROTOCOL_VERSION,
                    capabilities: 0,
                }))
            }
            Message::ProxyIdentity(proxy) => {
                if !self.via_gateway {
                    return Err(violation("identity assertion on a local connection"));
                }
                if self.greeted || self.identified {
                    return Err(violation("identity assertion out of order"));
                }
                if !valid_remote_name(&proxy.identity) {
                    return Err(violation("invalid identity"));
                }
                self.identity = Identity::Remote {
                    name: proxy.identity,
                };
                self.identified = true;
                Ok(())
            }
            _ if !self.greeted => Err(violation("Hello required before any other message")),
            Message::InitPortal(init) => {
                if self.portals.len() >= MAX_PORTALS_PER_CLIENT {
                    return Err(violation("portal limit reached"));
                }
                if self.portals.contains_key(&init.portal_id) {
                    return Err(violation("portal id already in use"));
                }
                self.portals.insert(init.portal_id, Portal::new(&init));
                self.requests.push(Request::PortalCreated(init.portal_id));
                Ok(())
            }
            Message::ClosePortal(close) => {
                if self.portals.remove(&close.portal_id).is_none() {
                    return Err(violation("unknown portal id"));
                }
                self.dirty = true;
                self.requests.push(Request::PortalClosed(close.portal_id));
                Ok(())
            }
            Message::DrawRect(rect) => self.push_command(rect.portal_id, Command::Rect(rect)),
            Message::DrawImage(draw) => {
                if !self.images.contains_key(&draw.image_id) {
                    return Err(violation("unknown image id"));
                }
                self.push_command(draw.portal_id, Command::Image(draw))
            }
            Message::ShowText(show) => {
                if show.height == 0 || show.height > MAX_TEXT_HEIGHT {
                    return Err(violation("text height out of range"));
                }
                self.push_command(show.portal_id, Command::Text(show))
            }
            Message::CreateImage(create) => {
                if create.format != IMAGE_FORMAT_RGBA8 {
                    return Err(violation("unsupported image format"));
                }
                if self.images.contains_key(&create.image_id) {
                    return Err(violation("image id already in use"));
                }
                if self.images.len() >= MAX_IMAGES_PER_CLIENT {
                    return Err(violation("image limit reached"));
                }
                let dimensions_ok = create.width >= 1
                    && create.height >= 1
                    && create.width <= MAX_IMAGE_DIMENSION
                    && create.height <= MAX_IMAGE_DIMENSION;
                if !dimensions_ok {
                    return Err(violation("image dimensions out of range"));
                }
                let bytes = create.width as usize * create.height as usize * 4;
                if self.image_bytes + bytes > MAX_IMAGE_BYTES_PER_CLIENT {
                    return Err(violation("image memory limit exceeded"));
                }
                self.image_bytes += bytes;
                self.images
                    .insert(create.image_id, Image::new(create.width, create.height));
                Ok(())
            }
            Message::ImageData(upload) => {
                let image = self
                    .images
                    .get_mut(&upload.image_id)
                    .ok_or_else(|| violation("unknown image id"))?;
                if !image.write(upload.offset as usize, &upload.data) {
                    return Err(violation("image data out of bounds"));
                }
                Ok(())
            }
            Message::DestroyImage(destroy) => {
                let image = self
                    .images
                    .remove(&destroy.image_id)
                    .ok_or_else(|| violation("unknown image id"))?;
                self.image_bytes -= image.byte_len();
                self.dirty = true;
                Ok(())
            }
            Message::Commit(commit) => {
                let portal = self
                    .portals
                    .get_mut(&commit.portal_id)
                    .ok_or_else(|| violation("unknown portal id"))?;
                portal.commit();
                self.dirty = true;
                Ok(())
            }
            Message::GrabKeyboard(grab) => {
                if !self.portals.contains_key(&grab.portal_id) {
                    return Err(violation("unknown portal id"));
                }
                self.requests.push(Request::GrabKeyboard(grab.portal_id));
                Ok(())
            }
            Message::ReleaseKeyboard(release) => {
                if !self.portals.contains_key(&release.portal_id) {
                    return Err(violation("unknown portal id"));
                }
                self.requests
                    .push(Request::ReleaseKeyboard(release.portal_id));
                Ok(())
            }
            Message::KeyEvent(_)
            | Message::PointerButton(_)
            | Message::FocusChanged(_)
            | Message::GrabStatus(_) => Err(violation("server-only message sent by a client")),
        }
    }
}

fn invalid<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err.to_string())
}

fn violation(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.to_string())
}
