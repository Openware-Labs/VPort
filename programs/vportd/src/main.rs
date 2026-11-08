// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

mod backend;
mod client;
mod compositor;
mod epoll;
mod framebuffer;
mod identity;
mod image;
mod input;
mod portal;
mod server;
mod text;

use std::path::PathBuf;
use std::process::ExitCode;

use backend::{Backend, PpmBackend};
use compositor::Compositor;

const DEFAULT_SOCKET: &str = "/run/vport/vport.sock";
const DEFAULT_PPM: &str = "/tmp/vport-frame.ppm";
const DEFAULT_SIZE: (u32, u32) = (800, 480);
const MAX_DIMENSION: u32 = 8192;

#[cfg(feature = "sdl2")]
const DEFAULT_BACKEND: &str = "sdl2";
#[cfg(not(feature = "sdl2"))]
const DEFAULT_BACKEND: &str = "ppm";

fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once('x')?;
    let w: u32 = w.parse().ok()?;
    let h: u32 = h.parse().ok()?;
    if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION {
        return None;
    }
    Some((w, h))
}

fn gateway_uids() -> Vec<u32> {
    match std::env::var("VPORT_GATEWAY_UIDS") {
        Ok(text) => text
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect(),
        Err(_) => vec![0, unsafe { libc::geteuid() }],
    }
}

fn build_backend(width: u32, height: u32) -> Result<Box<dyn Backend>, String> {
    let name = std::env::var("VPORT_BACKEND").unwrap_or_else(|_| DEFAULT_BACKEND.to_string());
    match name.as_str() {
        "ppm" => {
            let path = std::env::var("VPORT_PPM").unwrap_or_else(|_| DEFAULT_PPM.to_string());
            Ok(Box::new(PpmBackend::new(PathBuf::from(path), width, height)))
        }
        #[cfg(feature = "sdl2")]
        "sdl2" => backend::SdlBackend::new(width, height)
        .map(|sdl| Box::new(sdl) as Box<dyn Backend>)
        .map_err(|err| format!("could not start the SDL2 backend: {err}")),
        other => Err(format!("unknown or unavailable backend: {other}")),
    }
}

fn main() -> ExitCode {
    let socket = std::env::args()
    .nth(1)
    .or_else(|| std::env::var("VPORT_SOCKET").ok())
    .unwrap_or_else(|| DEFAULT_SOCKET.to_string());

    let gateway_socket = std::env::var("VPORT_GATEWAY_SOCKET").ok().map(PathBuf::from);

    let (width, height) = match std::env::var("VPORT_SIZE") {
        Ok(text) => match parse_size(&text) {
            Some(size) => size,
            None => {
                eprintln!("vportd: invalid VPORT_SIZE, expected WIDTHxHEIGHT");
                return ExitCode::FAILURE;
            }
        },
        Err(_) => DEFAULT_SIZE,
    };

    let backend = match build_backend(width, height) {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("vportd: {err}");
            return ExitCode::FAILURE;
        }
    };

    let compositor = Compositor::new(backend);

    let result = server::Server::bind(
        &PathBuf::from(&socket),
                                      gateway_socket.as_deref(),
                                      gateway_uids(),
                                      compositor,
    )
    .and_then(|mut server| server.run());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("vportd: {err}");
            ExitCode::FAILURE
        }
    }
}
