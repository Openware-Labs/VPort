// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

const MAX_EVENTS: usize = 64;

pub struct Epoll {
    fd: OwnedFd,
}

pub struct Event {
    pub token: u64,
    pub readable: bool,
    pub writable: bool,
    pub hangup: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PeerCred {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

impl Epoll {
    pub fn new() -> io::Result<Self> {
        let raw = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(raw) },
        })
    }

    pub fn add(&self, fd: RawFd, token: u64) -> io::Result<()> {
        let mut event = libc::epoll_event {
            events: (libc::EPOLLIN | libc::EPOLLRDHUP) as u32,
            u64: token,
        };
        let rc =
            unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut event) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn set_writable(&self, fd: RawFd, token: u64, writable: bool) -> io::Result<()> {
        let mut events = (libc::EPOLLIN | libc::EPOLLRDHUP) as u32;
        if writable {
            events |= libc::EPOLLOUT as u32;
        }
        let mut event = libc::epoll_event { events, u64: token };
        let rc =
            unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), libc::EPOLL_CTL_MOD, fd, &mut event) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn remove(&self, fd: RawFd) -> io::Result<()> {
        let rc = unsafe {
            libc::epoll_ctl(
                self.fd.as_raw_fd(),
                libc::EPOLL_CTL_DEL,
                fd,
                std::ptr::null_mut(),
            )
        };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn wait(&self, out: &mut Vec<Event>, timeout_ms: i32) -> io::Result<()> {
        out.clear();
        let mut raw: [libc::epoll_event; MAX_EVENTS] = unsafe { std::mem::zeroed() };
        let n = unsafe {
            libc::epoll_wait(
                self.fd.as_raw_fd(),
                raw.as_mut_ptr(),
                MAX_EVENTS as i32,
                timeout_ms,
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                return Ok(());
            }
            return Err(err);
        }
        for entry in &raw[..n as usize] {
            let flags = entry.events;
            let token = entry.u64;
            out.push(Event {
                token,
                readable: flags & (libc::EPOLLIN as u32) != 0,
                writable: flags & (libc::EPOLLOUT as u32) != 0,
                hangup: flags & ((libc::EPOLLHUP | libc::EPOLLERR | libc::EPOLLRDHUP) as u32) != 0,
            });
        }
        Ok(())
    }
}

pub fn peer_cred(fd: RawFd) -> io::Result<PeerCred> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCred {
        pid: cred.pid,
        uid: cred.uid,
        gid: cred.gid,
    })
}
