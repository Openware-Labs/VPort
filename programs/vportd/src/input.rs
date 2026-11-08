// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::collections::BTreeMap;

use vport_proto::{
    FocusChanged, GrabStatus, KeyEvent, Message, PointerButton, GRAB_DENIED, GRAB_GRANTED,
    GRAB_RELEASED,
};

use crate::client::{Client, Request};
use crate::identity::Identity;

pub struct GrabPolicy {
    uids: Vec<u32>,
    identities: Vec<String>,
}

impl GrabPolicy {
    pub fn from_env() -> Self {
        let uids = match std::env::var("VPORT_GRAB_UIDS") {
            Ok(text) => text
            .split(',')
            .filter_map(|part| part.trim().parse::<u32>().ok())
            .collect(),
            Err(_) => vec![0, unsafe { libc::geteuid() }],
        };
        let identities = match std::env::var("VPORT_GRAB_IDENTITIES") {
            Ok(text) => text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect(),
            Err(_) => Vec::new(),
        };
        Self { uids, identities }
    }

    pub fn allows(&self, identity: &Identity) -> bool {
        match identity {
            Identity::Local { uid } => self.uids.contains(uid),
            Identity::Remote { name } => self.identities.iter().any(|allowed| allowed == name),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PortalRef {
    token: u64,
    portal_id: u32,
}

pub struct Input {
    policy: GrabPolicy,
    focus: Option<PortalRef>,
    grab: Option<PortalRef>,
}

impl Input {
    pub fn new(policy: GrabPolicy) -> Self {
        eprintln!(
            "vportd: keyboard grab allowed for local uids {:?} and remote identities {:?}",
            policy.uids, policy.identities
        );
        Self {
            policy,
            focus: None,
            grab: None,
        }
    }

    pub fn handle_request(
        &mut self,
        token: u64,
        request: Request,
        clients: &mut BTreeMap<u64, Client>,
    ) {
        let identity = match clients.get(&token) {
            Some(client) => client.identity().clone(),
            None => return,
        };
        match request {
            Request::PortalCreated(portal_id) => {
                if self.grab.is_none() {
                    self.set_focus(Some(PortalRef { token, portal_id }), clients);
                }
            }
            Request::PortalClosed(portal_id) => {
                self.portal_gone(PortalRef { token, portal_id }, clients);
            }
            Request::GrabKeyboard(portal_id) => {
                self.grab_keyboard(PortalRef { token, portal_id }, &identity, clients);
            }
            Request::ReleaseKeyboard(portal_id) => {
                self.release_keyboard(PortalRef { token, portal_id }, clients);
            }
        }
    }

    pub fn client_gone(&mut self, token: u64, clients: &mut BTreeMap<u64, Client>) {
        if self.grab.map_or(false, |grab| grab.token == token) {
            self.grab = None;
            eprintln!("vportd: keyboard grab released because its client is gone");
        }
        if self.focus.map_or(false, |focus| focus.token == token) {
            self.focus = None;
            let next = topmost(clients);
            self.set_focus(next, clients);
        }
    }

    pub fn key(
        &mut self,
        clients: &mut BTreeMap<u64, Client>,
        scancode: u16,
        modifiers: u16,
        pressed: bool,
        repeat: bool,
    ) {
        let target = match self.grab.or(self.focus) {
            Some(target) => target,
            None => return,
        };
        send(
            clients,
             target.token,
             &Message::KeyEvent(KeyEvent {
                 portal_id: target.portal_id,
                 scancode,
                 modifiers,
                 pressed,
                 repeat,
             }),
        );
    }

    pub fn button(
        &mut self,
        clients: &mut BTreeMap<u64, Client>,
        x: i32,
        y: i32,
        button: u8,
        pressed: bool,
    ) {
        let (target, local_x, local_y) = match hit_test(clients, x, y) {
            Some(hit) => hit,
            None => return,
        };
        if pressed && self.grab.is_none() {
            self.set_focus(Some(target), clients);
        }
        send(
            clients,
             target.token,
             &Message::PointerButton(PointerButton {
                 portal_id: target.portal_id,
                 x: local_x,
                 y: local_y,
                 button,
                 pressed,
             }),
        );
    }

    fn grab_keyboard(
        &mut self,
        target: PortalRef,
        identity: &Identity,
        clients: &mut BTreeMap<u64, Client>,
    ) {
        let status = if !self.policy.allows(identity) {
            eprintln!("vportd: keyboard grab denied for {identity}: not allowed by policy");
            GRAB_DENIED
        } else {
            match self.grab {
                Some(holder) if holder != target => {
                    eprintln!("vportd: keyboard grab denied for {identity}: already held");
                    GRAB_DENIED
                }
                _ => GRAB_GRANTED,
            }
        };
        send(
            clients,
             target.token,
             &Message::GrabStatus(GrabStatus {
                 portal_id: target.portal_id,
                 status,
             }),
        );
        if status == GRAB_GRANTED {
            if self.grab != Some(target) {
                eprintln!("vportd: keyboard grab granted to {identity}");
            }
            self.grab = Some(target);
            self.set_focus(Some(target), clients);
        }
    }

    fn release_keyboard(&mut self, target: PortalRef, clients: &mut BTreeMap<u64, Client>) {
        if self.grab == Some(target) {
            self.grab = None;
            eprintln!("vportd: keyboard grab released");
            send(
                clients,
                 target.token,
                 &Message::GrabStatus(GrabStatus {
                     portal_id: target.portal_id,
                     status: GRAB_RELEASED,
                 }),
            );
        }
    }

    fn portal_gone(&mut self, gone: PortalRef, clients: &mut BTreeMap<u64, Client>) {
        if self.grab == Some(gone) {
            self.grab = None;
            eprintln!("vportd: keyboard grab released because its portal was closed");
        }
        if self.focus == Some(gone) {
            self.focus = None;
            let next = topmost(clients);
            self.set_focus(next, clients);
        }
    }

    fn set_focus(&mut self, next: Option<PortalRef>, clients: &mut BTreeMap<u64, Client>) {
        if self.focus == next {
            return;
        }
        if let Some(old) = self.focus {
            send(
                clients,
                 old.token,
                 &Message::FocusChanged(FocusChanged {
                     portal_id: old.portal_id,
                     focused: false,
                 }),
            );
        }
        self.focus = next;
        if let Some(new) = next {
            send(
                clients,
                 new.token,
                 &Message::FocusChanged(FocusChanged {
                     portal_id: new.portal_id,
                     focused: true,
                 }),
            );
        }
    }
}

fn send(clients: &mut BTreeMap<u64, Client>, token: u64, message: &Message) {
    if let Some(client) = clients.get_mut(&token) {
        if let Err(e) = client.queue(message) {
            eprintln!("vportd: dropping pid {}: {e}", client.peer().pid);
            client.doom();
        }
    }
}

fn topmost(clients: &BTreeMap<u64, Client>) -> Option<PortalRef> {
    let mut top = None;
    for (token, client) in clients {
        for (portal_id, _) in client.portal_entries() {
            top = Some(PortalRef {
                token: *token,
                portal_id,
            });
        }
    }
    top
}

fn hit_test(clients: &BTreeMap<u64, Client>, x: i32, y: i32) -> Option<(PortalRef, i16, i16)> {
    let mut found = None;
    for (token, client) in clients {
        for (portal_id, portal) in client.portal_entries() {
            let left = portal.x as i32;
            let top = portal.y as i32;
            let inside = x >= left
            && x < left + portal.w as i32
            && y >= top
            && y < top + portal.h as i32;
            if inside {
                found = Some((
                    PortalRef {
                        token: *token,
                        portal_id,
                    },
                    (x - left).min(i16::MAX as i32) as i16,
                              (y - top).min(i16::MAX as i32) as i16,
                ));
            }
        }
    }
    found
}
