// VPort — Demo
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use libvport::{hid_to_ascii, Connection, Event, GRAB_GRANTED, MOD_SHIFT};

const PORTAL: u32 = 1;
const LOGO: u32 = 1;
const LOGO_HEIGHT: u32 = 110;
const BUTTON_X: i32 = 440;
const BUTTON_Y: i32 = 270;
const BUTTON_W: i32 = 180;
const BUTTON_H: i32 = 70;
const MAX_CODE: usize = 20;

fn render_logo() -> (u16, u16, Vec<u8>) {
    let svg = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/VPort.svg"
    ));
    let options = resvg::usvg::Options::default();
    let tree =
        resvg::usvg::Tree::from_data(svg, &options).expect("assets/VPort.svg is not a valid SVG");
    let size = tree.size();
    let scale = LOGO_HEIGHT as f32 / size.height();
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, LOGO_HEIGHT).expect("invalid logo dimensions");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.data().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    (width as u16, LOGO_HEIGHT as u16, pixels)
}

fn draw_scene(conn: &mut Connection, code: &str) -> std::io::Result<()> {
    conn.draw_rect(PORTAL, 0, 0, 640, 360, 0x202020ff)?;
    conn.draw_image(PORTAL, LOGO, 20, 20)?;
    conn.show_text(PORTAL, 20, 140, 28, 0xb0b0b0ff, "VPORT DEMO")?;
    conn.show_text(PORTAL, 20, 180, 80, 0xffffffff, "INSERT CODE")?;
    conn.show_text(PORTAL, 20, 300, 24, 0x8a8a8aff, &format!("Code: {code}"))?;
    conn.draw_rect(
        PORTAL,
        BUTTON_X as i16,
        BUTTON_Y as i16,
        BUTTON_W as u16,
        BUTTON_H as u16,
        0x00c853ff,
    )?;
    conn.show_text(
        PORTAL,
        (BUTTON_X + 62) as i16,
        (BUTTON_Y + 14) as i16,
        40,
        0x102010ff,
        "OK",
    )?;
    conn.commit(PORTAL)
}

fn main() -> std::io::Result<()> {
    let (logo_width, logo_height, logo_pixels) = render_logo();

    let mut conn = Connection::connect_default()?;
    conn.upload_image(LOGO, logo_width, logo_height, &logo_pixels)?;
    conn.init_portal(PORTAL, 100, 100, 640, 360)?;
    draw_scene(&mut conn, "")?;
    conn.grab_keyboard(PORTAL)?;

    let mut code = String::new();
    loop {
        match conn.wait_event()? {
            Event::GrabStatus(grab) => {
                if grab.status == GRAB_GRANTED {
                    println!("keyboard grab granted");
                } else {
                    println!("keyboard grab refused");
                }
            }
            Event::Key(key) if key.pressed && !key.repeat => {
                let shift = key.modifiers & MOD_SHIFT != 0;
                match hid_to_ascii(key.scancode, shift) {
                    Some('\n') => {
                        println!("code: {code}");
                        code.clear();
                        draw_scene(&mut conn, &code)?;
                    }
                    Some(ch) if code.len() < MAX_CODE => {
                        code.push(ch);
                        draw_scene(&mut conn, &code)?;
                    }
                    _ => {}
                }
            }
            Event::PointerButton(click) if click.pressed => {
                let x = click.x as i32;
                let y = click.y as i32;
                let inside = x >= BUTTON_X
                    && x < BUTTON_X + BUTTON_W
                    && y >= BUTTON_Y
                    && y < BUTTON_Y + BUTTON_H;
                if inside {
                    println!("confirmed");
                    break;
                }
            }
            Event::FocusChanged(focus) => {
                println!("focus {}", if focus.focused { "gained" } else { "lost" });
            }
            _ => {}
        }
    }

    conn.close_portal(PORTAL)?;
    conn.destroy_image(LOGO)?;
    Ok(())
}
