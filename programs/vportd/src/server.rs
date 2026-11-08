// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use crate::backend::BackendEvent;
use crate::client::Client;
use crate::compositor::Compositor;
use crate::epoll::{peer_cred, Epoll, Event};
use crate::input::{GrabPolicy, Input};

const LISTENER_TOKEN: u64 = 0;
const MAX_CLIENTS: usize = 64;
const POLL_INTERVAL_MS: i32 = 16;

pub struct Server {
    listener: UnixListener,
    epoll: Epoll,
    clients: BTreeMap<u64, Client>,
    next_token: u64,
    socket_path: PathBuf,
    compositor: Compositor,
    input: Input,
}

impl Server {
    pub fn bind(path: &Path, mut compositor: Compositor) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another vportd is already listening on {}", path.display()),
                ));
            }
            fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;

        let epoll = Epoll::new()?;
        epoll.add(listener.as_raw_fd(), LISTENER_TOKEN)?;

        compositor.render(std::iter::empty())?;

        let input = Input::new(GrabPolicy::from_env());

        eprintln!("vportd: listening on {}", path.display());

        Ok(Self {
            listener,
            epoll,
            clients: BTreeMap::new(),
            next_token: LISTENER_TOKEN + 1,
            socket_path: path.to_path_buf(),
            compositor,
            input,
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut events: Vec<Event> = Vec::with_capacity(64);
        let mut backend_events: Vec<BackendEvent> = Vec::with_capacity(8);
        loop {
            let timeout = if self.compositor.needs_polling() {
                POLL_INTERVAL_MS
            } else {
                -1
            };
            self.epoll.wait(&mut events, timeout)?;

            let mut changed = false;
            for event in events.drain(..) {
                if event.token == LISTENER_TOKEN {
                    self.accept_all();
                } else {
                    changed |= self.service_client(event);
                }
            }

            let mut redraw = false;
            self.compositor.poll_events(&mut backend_events);
            for event in backend_events.drain(..) {
                match event {
                    BackendEvent::Quit => {
                        eprintln!("vportd: backend requested shutdown");
                        return Ok(());
                    }
                    BackendEvent::Redraw => redraw = true,
                    BackendEvent::Key {
                        scancode,
                        modifiers,
                        pressed,
                        repeat,
                    } => self
                        .input
                        .key(&mut self.clients, scancode, modifiers, pressed, repeat),
                    BackendEvent::Button {
                        x,
                        y,
                        button,
                        pressed,
                    } => self.input.button(&mut self.clients, x, y, button, pressed),
                }
            }

            changed |= self.sweep();

            if changed {
                self.recompose();
            } else if redraw {
                self.present_again();
            }
        }
    }

    fn accept_all(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => self.register(stream),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    eprintln!("vportd: accept failed: {e}");
                    return;
                }
            }
        }
    }

    fn register(&mut self, stream: UnixStream) {
        if self.clients.len() >= MAX_CLIENTS {
            eprintln!("vportd: client limit reached, connection refused");
            return;
        }
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        let peer = match peer_cred(stream.as_raw_fd()) {
            Ok(peer) => peer,
            Err(e) => {
                eprintln!("vportd: could not read peer credentials: {e}");
                return;
            }
        };
        let token = self.next_token;
        self.next_token += 1;
        if let Err(e) = self.epoll.add(stream.as_raw_fd(), token) {
            eprintln!("vportd: could not register client: {e}");
            return;
        }
        eprintln!(
            "vportd: client connected (pid {}, uid {}, gid {})",
            peer.pid, peer.uid, peer.gid
        );
        self.clients.insert(token, Client::new(stream, peer));
    }

    fn service_client(&mut self, event: Event) -> bool {
        let (keep, mut dirty, requests) = match self.clients.get_mut(&event.token) {
            Some(client) => {
                let mut open = true;
                if event.readable {
                    match client.pump() {
                        Ok(still_open) => open = still_open,
                        Err(e) => {
                            eprintln!("vportd: dropping pid {}: {e}", client.peer().pid);
                            open = false;
                        }
                    }
                }
                if open && event.writable {
                    if let Err(e) = client.flush() {
                        eprintln!("vportd: dropping pid {}: {e}", client.peer().pid);
                        open = false;
                    }
                }
                (
                    open && !event.hangup,
                    client.take_dirty(),
                    client.take_requests(),
                )
            }
            None => return false,
        };
        for request in requests {
            self.input
                .handle_request(event.token, request, &mut self.clients);
        }
        if !keep {
            dirty |= self.drop_client(event.token);
        }
        dirty
    }

    fn drop_client(&mut self, token: u64) -> bool {
        match self.clients.remove(&token) {
            Some(client) => {
                let _ = self.epoll.remove(client.raw_fd());
                eprintln!("vportd: client disconnected (pid {})", client.peer().pid);
                let had_portals = client.has_portals();
                self.input.client_gone(token, &mut self.clients);
                had_portals
            }
            None => false,
        }
    }

    fn sweep(&mut self) -> bool {
        let mut dirty = false;
        loop {
            let doomed: Vec<u64> = self
                .clients
                .iter()
                .filter(|(_, client)| client.is_doomed())
                .map(|(token, _)| *token)
                .collect();
            if doomed.is_empty() {
                break;
            }
            for token in doomed {
                dirty |= self.drop_client(token);
            }
        }
        for (token, client) in self.clients.iter_mut() {
            let wanted = client.wants_write();
            if wanted != client.write_armed()
                && self
                    .epoll
                    .set_writable(client.raw_fd(), *token, wanted)
                    .is_ok()
            {
                client.set_write_armed(wanted);
            }
        }
        dirty
    }

    fn recompose(&mut self) {
        let scenes = self.clients.values().flat_map(|client| client.scenes());
        if let Err(e) = self.compositor.render(scenes) {
            eprintln!("vportd: render failed: {e}");
        }
    }

    fn present_again(&mut self) {
        if let Err(e) = self.compositor.present() {
            eprintln!("vportd: present failed: {e}");
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}
