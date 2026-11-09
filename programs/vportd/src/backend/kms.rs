// VPort — VPortd
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use std::fs::{self, File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::ptr;

use super::evdev::Keyboards;
use super::sys::{ioc, IOC_NONE, IOC_READ, IOC_WRITE};
use super::{Backend, BackendEvent};
use crate::framebuffer::Framebuffer;

const DRM_TYPE: u8 = b'd';
const GET_RESOURCES: u32 = 0xa0;
const GET_CRTC: u32 = 0xa1;
const SET_CRTC: u32 = 0xa2;
const GET_ENCODER: u32 = 0xa6;
const GET_CONNECTOR: u32 = 0xa7;
const ADD_FB: u32 = 0xae;
const REMOVE_FB: u32 = 0xaf;
const CREATE_DUMB: u32 = 0xb2;
const MAP_DUMB: u32 = 0xb3;
const DESTROY_DUMB: u32 = 0xb4;
const SET_MASTER: u32 = 0x1e;
const DROP_MASTER: u32 = 0x1f;

const KD_GET_MODE: u64 = 0x4b3b;
const KD_SET_MODE: u64 = 0x4b3a;
const KD_TEXT: libc::c_ulong = 0;
const KD_GRAPHICS: libc::c_ulong = 1;

const MODE_TYPE_PREFERRED: u32 = 1 << 3;
const CONNECTOR_CONNECTED: u32 = 1;

#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ModeInfo {
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
    vscan: u16,
    vrefresh: u32,
    flags: u32,
    kind: u32,
    name: [u8; 32],
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct CardRes {
    fb_id_ptr: u64,
    crtc_id_ptr: u64,
    connector_id_ptr: u64,
    encoder_id_ptr: u64,
    count_fbs: u32,
    count_crtcs: u32,
    count_connectors: u32,
    count_encoders: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64,
    modes_ptr: u64,
    props_ptr: u64,
    prop_values_ptr: u64,
    count_modes: u32,
    count_props: u32,
    count_encoders: u32,
    encoder_id: u32,
    connector_id: u32,
    connector_type: u32,
    connector_type_id: u32,
    connection: u32,
    mm_width: u32,
    mm_height: u32,
    subpixel: u32,
    pad: u32,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct GetEncoder {
    encoder_id: u32,
    encoder_type: u32,
    crtc_id: u32,
    possible_crtcs: u32,
    possible_clones: u32,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct CreateDumb {
    height: u32,
    width: u32,
    bpp: u32,
    flags: u32,
    handle: u32,
    pitch: u32,
    size: u64,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct MapDumb {
    handle: u32,
    pad: u32,
    offset: u64,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct DestroyDumb {
    handle: u32,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Default)]
struct FbCmd {
    fb_id: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    depth: u32,
    handle: u32,
}

#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Crtc {
    set_connectors_ptr: u64,
    count_connectors: u32,
    crtc_id: u32,
    fb_id: u32,
    x: u32,
    y: u32,
    gamma_size: u32,
    mode_valid: u32,
    mode: ModeInfo,
}

struct ConnectorInfo {
    id: u32,
    connected: bool,
    encoder_id: u32,
    encoders: Vec<u32>,
    modes: Vec<ModeInfo>,
}

fn other(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, message.to_string())
}

fn context(err: io::Error, what: &str) -> io::Error {
    io::Error::new(err.kind(), format!("{what}: {err}"))
}

fn drm_request<T>(number: u32) -> u64 {
    ioc(IOC_READ | IOC_WRITE, DRM_TYPE, number, size_of::<T>())
}

fn drm_ioctl<T>(fd: RawFd, number: u32, argument: &mut T) -> io::Result<()> {
    let request = drm_request::<T>(number);
    loop {
        let rc = unsafe { libc::ioctl(fd, request as _, argument as *mut T) };
        if rc >= 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

fn set_master(fd: RawFd) -> io::Result<()> {
    let request = ioc(IOC_NONE, DRM_TYPE, SET_MASTER, 0);
    if unsafe { libc::ioctl(fd, request as _) } < 0 {
        return Err(context(
            io::Error::last_os_error(),
            "cannot become DRM master, is another display server using this screen?",
        ));
    }
    Ok(())
}

fn drop_master(fd: RawFd) {
    let request = ioc(IOC_NONE, DRM_TYPE, DROP_MASTER, 0);
    unsafe {
        libc::ioctl(fd, request as _);
    }
}

fn resources(fd: RawFd) -> io::Result<(Vec<u32>, Vec<u32>)> {
    let mut sizes = CardRes::default();
    drm_ioctl(fd, GET_RESOURCES, &mut sizes)?;
    let mut crtcs = vec![0u32; sizes.count_crtcs as usize];
    let mut connectors = vec![0u32; sizes.count_connectors as usize];
    let mut fill = CardRes {
        crtc_id_ptr: crtcs.as_mut_ptr() as u64,
        count_crtcs: sizes.count_crtcs,
        connector_id_ptr: connectors.as_mut_ptr() as u64,
        count_connectors: sizes.count_connectors,
        ..Default::default()
    };
    drm_ioctl(fd, GET_RESOURCES, &mut fill)?;
    Ok((crtcs, connectors))
}

fn connector(fd: RawFd, id: u32) -> io::Result<ConnectorInfo> {
    let mut sizes = GetConnector {
        connector_id: id,
        ..Default::default()
    };
    drm_ioctl(fd, GET_CONNECTOR, &mut sizes)?;
    let mut modes = vec![ModeInfo::default(); sizes.count_modes as usize];
    let mut encoders = vec![0u32; sizes.count_encoders as usize];
    let mut fill = GetConnector {
        connector_id: id,
        modes_ptr: modes.as_mut_ptr() as u64,
        count_modes: sizes.count_modes,
        encoders_ptr: encoders.as_mut_ptr() as u64,
        count_encoders: sizes.count_encoders,
        ..Default::default()
    };
    drm_ioctl(fd, GET_CONNECTOR, &mut fill)?;
    modes.truncate(fill.count_modes as usize);
    encoders.truncate(fill.count_encoders as usize);
    Ok(ConnectorInfo {
        id,
        connected: fill.connection == CONNECTOR_CONNECTED,
        encoder_id: fill.encoder_id,
        encoders,
        modes,
    })
}

fn encoder(fd: RawFd, id: u32) -> io::Result<GetEncoder> {
    let mut info = GetEncoder {
        encoder_id: id,
        ..Default::default()
    };
    drm_ioctl(fd, GET_ENCODER, &mut info)?;
    Ok(info)
}

fn find_crtc(fd: RawFd, connector: &ConnectorInfo, crtcs: &[u32]) -> io::Result<u32> {
    if connector.encoder_id != 0 {
        let current = encoder(fd, connector.encoder_id)?;
        if current.crtc_id != 0 {
            return Ok(current.crtc_id);
        }
    }
    for encoder_id in &connector.encoders {
        let candidate = encoder(fd, *encoder_id)?;
        for (index, crtc) in crtcs.iter().enumerate() {
            if candidate.possible_crtcs & (1u32 << index) != 0 {
                return Ok(*crtc);
            }
        }
    }
    Err(other("no usable CRTC for the connected display"))
}

fn open_console() -> io::Result<File> {
    for path in ["/dev/tty", "/dev/tty0"] {
        let file = match OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) => file,
            Err(_) => continue,
        };
        let mut mode: libc::c_int = 0;
        let rc = unsafe {
            libc::ioctl(
                file.as_raw_fd(),
                KD_GET_MODE as _,
                &mut mode as *mut libc::c_int,
            )
        };
        if rc >= 0 {
            return Ok(file);
        }
    }
    Err(other(
        "no Linux virtual console found, run vportd from a TTY (Ctrl+Alt+F3)",
    ))
}

fn set_console_mode(console: &File, mode: libc::c_ulong) -> bool {
    unsafe { libc::ioctl(console.as_raw_fd(), KD_SET_MODE as _, mode) >= 0 }
}

pub struct KmsBackend {
    drm: File,
    console: File,
    connector_id: u32,
    saved: Crtc,
    fb_id: u32,
    handle: u32,
    pitch: u32,
    width: u32,
    height: u32,
    map: *mut libc::c_void,
    map_len: usize,
    keyboards: Keyboards,
}

impl KmsBackend {
    pub fn auto(grab_input: bool) -> io::Result<Self> {
        let mut paths: Vec<_> = fs::read_dir("/dev/dri")?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map_or(false, |name| name.starts_with("card"))
            })
            .collect();
        paths.sort();

        let mut last = other("no graphics device found in /dev/dri");
        for path in paths {
            let name = path.to_string_lossy().to_string();
            match Self::new(&name, grab_input) {
                Ok(backend) => return Ok(backend),
                Err(e) => {
                    eprintln!("vportd: {name}: {e}");
                    last = e;
                }
            }
        }
        Err(last)
    }
    pub fn new(device: &str, grab_input: bool) -> io::Result<Self> {
        let console = open_console()?;
        let drm = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(device)
            .map_err(|e| context(e, &format!("cannot open {device}")))?;
        let fd = drm.as_raw_fd();
        if let Err(e) = set_master(fd) {
            eprintln!("vportd: {e}");
        }

        let (crtcs, connector_ids) = resources(fd)?;
        let mut chosen: Option<ConnectorInfo> = None;
        for id in connector_ids {
            let info = connector(fd, id)?;
            if info.connected && !info.modes.is_empty() {
                chosen = Some(info);
                break;
            }
        }
        let connector = chosen.ok_or_else(|| other("no connected display found"))?;
        let mode = connector
            .modes
            .iter()
            .find(|candidate| candidate.kind & MODE_TYPE_PREFERRED != 0)
            .copied()
            .unwrap_or(connector.modes[0]);
        let crtc_id = find_crtc(fd, &connector, &crtcs)?;

        let mut saved = Crtc {
            crtc_id,
            ..Default::default()
        };
        drm_ioctl(fd, GET_CRTC, &mut saved)?;

        let width = mode.hdisplay as u32;
        let height = mode.vdisplay as u32;
        eprintln!(
            "vportd: display {width}x{height} at {} Hz on connector {}",
            mode.vrefresh, connector.id
        );

        let mut dumb = CreateDumb {
            height,
            width,
            bpp: 32,
            ..Default::default()
        };
        drm_ioctl(fd, CREATE_DUMB, &mut dumb)?;

        let mut add = FbCmd {
            width,
            height,
            pitch: dumb.pitch,
            bpp: 32,
            depth: 24,
            handle: dumb.handle,
            ..Default::default()
        };
        drm_ioctl(fd, ADD_FB, &mut add)?;

        let mut map_request = MapDumb {
            handle: dumb.handle,
            ..Default::default()
        };
        drm_ioctl(fd, MAP_DUMB, &mut map_request)?;

        let map_len = dumb.size as usize;
        let map = unsafe {
            libc::mmap(
                ptr::null_mut(),
                map_len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                map_request.offset as libc::off_t,
            )
        };
        if map == libc::MAP_FAILED {
            return Err(context(
                io::Error::last_os_error(),
                "cannot map the frame buffer",
            ));
        }
        unsafe {
            ptr::write_bytes(map as *mut u8, 0, map_len);
        }

        let connector_id = connector.id;
        let mut crtc = Crtc {
            set_connectors_ptr: &connector_id as *const u32 as u64,
            count_connectors: 1,
            crtc_id,
            fb_id: add.fb_id,
            mode_valid: 1,
            mode,
            ..Default::default()
        };
        drm_ioctl(fd, SET_CRTC, &mut crtc).map_err(|e| {
            context(
                e,
                "cannot set the video mode, is another display server using this screen?",
            )
        })?;

        if !set_console_mode(&console, KD_GRAPHICS) {
            eprintln!("vportd: could not switch the console to graphics mode");
        }

        let keyboards = Keyboards::open(grab_input);

        Ok(Self {
            drm,
            console,
            connector_id,
            saved,
            fb_id: add.fb_id,
            handle: dumb.handle,
            pitch: dumb.pitch,
            width,
            height,
            map,
            map_len,
            keyboards,
        })
    }
}

impl Backend for KmsBackend {
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn present(&mut self, framebuffer: &Framebuffer) -> io::Result<()> {
        let width = framebuffer.width().min(self.width) as usize;
        let height = framebuffer.height().min(self.height) as usize;
        let stride = framebuffer.width() as usize;
        let pitch = self.pitch as usize;
        let target = unsafe { std::slice::from_raw_parts_mut(self.map as *mut u8, self.map_len) };
        for y in 0..height {
            let source = &framebuffer.pixels()[y * stride..y * stride + width];
            let row = &mut target[y * pitch..y * pitch + width * 4];
            for (index, &pixel) in source.iter().enumerate() {
                row[index * 4..index * 4 + 4].copy_from_slice(&pixel.to_le_bytes());
            }
        }
        Ok(())
    }

    fn needs_polling(&self) -> bool {
        true
    }

    fn poll_events(&mut self, out: &mut Vec<BackendEvent>) {
        if self.keyboards.poll(out) {
            out.push(BackendEvent::Quit);
        }
    }
}

impl Drop for KmsBackend {
    fn drop(&mut self) {
        let fd = self.drm.as_raw_fd();

        let mut restore = self.saved;
        if restore.mode_valid != 0 {
            restore.set_connectors_ptr = &self.connector_id as *const u32 as u64;
            restore.count_connectors = 1;
        } else {
            restore.count_connectors = 0;
            restore.fb_id = 0;
        }
        let _ = drm_ioctl(fd, SET_CRTC, &mut restore);

        let mut fb_id = self.fb_id;
        let _ = drm_ioctl(fd, REMOVE_FB, &mut fb_id);
        unsafe {
            libc::munmap(self.map, self.map_len);
        }
        let mut destroy = DestroyDumb {
            handle: self.handle,
        };
        let _ = drm_ioctl(fd, DESTROY_DUMB, &mut destroy);

        drop_master(fd);
        set_console_mode(&self.console, KD_TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_sizes_match_the_kernel() {
        assert_eq!(size_of::<ModeInfo>(), 68);
        assert_eq!(size_of::<CardRes>(), 64);
        assert_eq!(size_of::<GetConnector>(), 80);
        assert_eq!(size_of::<GetEncoder>(), 20);
        assert_eq!(size_of::<Crtc>(), 104);
        assert_eq!(size_of::<CreateDumb>(), 32);
        assert_eq!(size_of::<MapDumb>(), 16);
        assert_eq!(size_of::<FbCmd>(), 28);
    }

    #[test]
    fn ioctl_numbers_match_the_kernel() {
        assert_eq!(drm_request::<CardRes>(GET_RESOURCES), 0xc040_64a0);
        assert_eq!(drm_request::<Crtc>(SET_CRTC), 0xc068_64a2);
        assert_eq!(drm_request::<GetConnector>(GET_CONNECTOR), 0xc050_64a7);
        assert_eq!(drm_request::<GetEncoder>(GET_ENCODER), 0xc014_64a6);
        assert_eq!(drm_request::<CreateDumb>(CREATE_DUMB), 0xc020_64b2);
        assert_eq!(drm_request::<MapDumb>(MAP_DUMB), 0xc010_64b3);
        assert_eq!(drm_request::<FbCmd>(ADD_FB), 0xc01c_64ae);
    }
}
