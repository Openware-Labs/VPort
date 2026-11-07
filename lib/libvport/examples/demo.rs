// VPort — Demo
// SPDX-License-Identifier: MIT
// Copyright 2026-present (C) Openware Labs
// Code Author(s): Openware Labs developers

use libvport::{hid_to_ascii, Connection, Event, GRAB_GRANTED, MOD_SHIFT};

const PORTAL: u32 = 1;
const BUTTON_X: i32 = 20;
const BUTTON_Y: i32 = 20;
const BUTTON_W: i32 = 200;
const BUTTON_H: i32 = 80;

fn main() -> std::io::Result<()> {
    let mut conn = Connection::connect_default()?;
    conn.init_portal(PORTAL, 100, 100, 640, 360)?;
    conn.draw_rect(PORTAL, 0, 0, 640, 360, 0x202020ff)?;
    conn.draw_rect(
        PORTAL,
        BUTTON_X as i16,
        BUTTON_Y as i16,
        BUTTON_W as u16,
        BUTTON_H as u16,
        0x00c853ff,
    )?;
    conn.commit(PORTAL)?;
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
                    }
                    Some(ch) => code.push(ch),
                    None => {}
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
    Ok(())
}
