// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;

use vport_proto::{frame_len, Header, Hello, Message, HEADER_SIZE, PROTOCOL_VERSION};

use crate::epoll::PeerCred;
use crate::portal::{Portal, MAX_PENDING_RECTS};

const READ_CHUNK: usize = 4096;
const READ_BUDGET_PER_WAKEUP: usize = 16;
const MAX_PORTALS_PER_CLIENT: usize = 8;
const MAX_OUTPUT_BACKLOG: usize = 64 * 1024;

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
    inbuf: Vec<u8>,
    outbuf: Vec<u8>,
    greeted: bool,
    portals: BTreeMap<u32, Portal>,
    dirty: bool,
    doomed: bool,
    write_armed: bool,
    requests: Vec<Request>,
}

impl Client {
    pub fn new(stream: UnixStream, peer: PeerCred) -> Self {
        Self {
            stream,
            peer,
            inbuf: Vec::with_capacity(READ_CHUNK),
            outbuf: Vec::with_capacity(64),
            greeted: false,
            portals: BTreeMap::new(),
            dirty: false,
            doomed: false,
            write_armed: false,
            requests: Vec::new(),
        }
    }

    pub fn peer(&self) -> PeerCred {
        self.peer
    }

    pub fn raw_fd(&self) -> RawFd {
        self.stream.as_raw_fd()
    }

    pub fn portals(&self) -> impl Iterator<Item = &Portal> {
        self.portals.values()
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

    fn handle(&mut self, message: Message) -> io::Result<()> {
        match message {
            Message::Hello(hello) => {
                if self.greeted {
                    return Err(violation("duplicate Hello"));
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
            Message::DrawRect(rect) => {
                let portal = self
                    .portals
                    .get_mut(&rect.portal_id)
                    .ok_or_else(|| violation("unknown portal id"))?;
                if !portal.push(rect) {
                    return Err(violation(&format!(
                        "more than {MAX_PENDING_RECTS} uncommitted rects"
                    )));
                }
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
