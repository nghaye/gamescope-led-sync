//! Gamepad input read straight from evdev (`/dev/input/event*`), so the
//! calibration is 100% controller-driven without any input-stack dependency:
//! no new crate, just `libc` and the kernel's `struct input_event`.
//!
//! Devices are filtered with EVIOCGBIT: a device is a gamepad when it
//! reports EV_KEY and at least one of BTN_SOUTH (A) / BTN_DPAD_UP, which
//! covers the Steam Deck's built-in controller and standard USB gamepads
//! (xpad & co. all use the same evdev button codes).
//!
//! Only press edges (value == 1) are reported; key auto-repeat (value == 2)
//! is left to the UI, which implements its own accelerated repeat for the
//! sliders. Stick axes become direction events on threshold crossing.

use std::{
    fs::{File, OpenOptions, read_dir},
    io,
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
};

/// One logical pad action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pad {
    Up,
    Down,
    Left,
    Right,
    /// A button: activate the focused control.
    Confirm,
    /// B button / View: cancel (quit the calibration).
    Cancel,
    /// LB: previous color.
    Prev,
    /// RB: next color.
    Next,
}

// evdev constants (linux/input-event-codes.h).
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const BTN_SOUTH: u16 = 0x130; // A / cross
const BTN_EAST: u16 = 0x131; // B / circle
const BTN_SELECT: u16 = 0x13a; // View / back
const BTN_TL: u16 = 0x136; // LB
const BTN_TR: u16 = 0x137; // RB
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_DOWN: u16 = 0x221;
const BTN_DPAD_LEFT: u16 = 0x222;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

/// Stick deflection that counts as a direction (|value| of the axis).
const STICK_THRESHOLD: i32 = 12_000;

/// sizeof(struct input_event) on 64-bit Linux: timeval (2 x i64) + u16
/// type + u16 code + i32 value.
const INPUT_EVENT_SIZE: usize = 24;

/// Map one key event to a pad action (press edge only).
fn map_key(code: u16) -> Option<Pad> {
    match code {
        BTN_DPAD_UP => Some(Pad::Up),
        BTN_DPAD_DOWN => Some(Pad::Down),
        BTN_DPAD_LEFT => Some(Pad::Left),
        BTN_DPAD_RIGHT => Some(Pad::Right),
        BTN_SOUTH => Some(Pad::Confirm),
        BTN_EAST | BTN_SELECT => Some(Pad::Cancel),
        BTN_TL => Some(Pad::Prev),
        BTN_TR => Some(Pad::Next),
        _ => None,
    }
}

/// Map one absolute-axis event to a direction, emitting only on threshold
/// crossing (prev holds the last value of the axis).
fn map_abs(code: u16, value: i32, prev: &mut i32) -> Option<Pad> {
    let was = *prev;
    *prev = value;
    let inside = |v: i32| v.abs() < STICK_THRESHOLD;
    match code {
        ABS_Y if inside(was) && !inside(value) => Some(if value < 0 {
            Pad::Up
        } else {
            Pad::Down
        }),
        ABS_X if inside(was) && !inside(value) => Some(if value < 0 {
            Pad::Left
        } else {
            Pad::Right
        }),
        _ => None,
    }
}

/// Feed one raw event; `abs_prev` is the per-axis last value ([x, y]).
fn feed_event(ev_type: u16, code: u16, value: i32, abs_prev: &mut [i32; 2]) -> Option<Pad> {
    match ev_type {
        EV_KEY if value == 1 => map_key(code),
        EV_ABS => match code {
            ABS_X => map_abs(code, value, &mut abs_prev[0]),
            ABS_Y => map_abs(code, value, &mut abs_prev[1]),
            _ => None,
        },
        _ => None,
    }
}

/// Parse a raw buffer of input_event structs into pad actions.
fn parse_buffer(buf: &[u8], abs_prev: &mut [i32; 2]) -> Vec<Pad> {
    let mut out = Vec::new();
    for chunk in buf.as_chunks::<INPUT_EVENT_SIZE>().0 {
        let ev_type = u16::from_ne_bytes([chunk[16], chunk[17]]);
        let code = u16::from_ne_bytes([chunk[18], chunk[19]]);
        let value = i32::from_ne_bytes([chunk[20], chunk[21], chunk[22], chunk[23]]);
        if let Some(pad) = feed_event(ev_type, code, value, abs_prev) {
            out.push(pad);
        }
    }
    out
}

/// EVIOCGBIT(ev, len): the ioctl that reads the device's supported-event
/// bitmap for one event type.
fn eviocgbit(ev: u16, len: usize) -> libc::c_ulong {
    const IOC_READ: u64 = 2;
    ((IOC_READ << 30) | ((len as u64) << 16) | (('E' as u64) << 8) | (0x20 + ev as u64))
        as libc::c_ulong
}

fn bit_set(bitmap: &[u8], bit: u16) -> bool {
    let (byte, off) = ((bit / 8) as usize, bit % 8);
    byte < bitmap.len() && bitmap[byte] & (1 << off) != 0
}

/// Whether the device behind `fd` looks like a gamepad (EV_KEY with a
/// gamepad button).
fn is_gamepad(fd: libc::c_int) -> bool {
    let mut key_bits = [0u8; 96]; // KEY_MAX 0x2ff -> 768 bits
    let req = eviocgbit(EV_KEY, key_bits.len());
    let rc = unsafe { libc::ioctl(fd, req, key_bits.as_mut_ptr()) };
    if rc < 0 {
        return false;
    }
    bit_set(&key_bits, BTN_SOUTH) || bit_set(&key_bits, BTN_DPAD_UP)
}

/// All open gamepad devices.
pub struct Gamepads {
    files: Vec<File>,
    abs_prev: Vec<[i32; 2]>,
}

impl Gamepads {
    /// Open every gamepad under /dev/input. An empty set is not an error:
    /// the UI still works with the keyboard and warns the user.
    pub fn open() -> io::Result<Gamepads> {
        let mut files = Vec::new();
        let entries = read_dir("/dev/input")?;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with("event") {
                continue;
            }
            let Ok(file) = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(entry.path())
            else {
                continue;
            };
            if is_gamepad(file.as_raw_fd()) {
                files.push(file);
            }
        }
        let abs_prev = vec![[0; 2]; files.len()];
        Ok(Gamepads { files, abs_prev })
    }

    pub fn count(&self) -> usize {
        self.files.len()
    }

    /// Drain all pending events from every gamepad (non-blocking).
    pub fn poll(&mut self) -> Vec<Pad> {
        let mut out = Vec::new();
        let mut buf = [0u8; INPUT_EVENT_SIZE * 32];
        for (i, file) in self.files.iter().enumerate() {
            loop {
                let n =
                    unsafe { libc::read(file.as_raw_fd(), buf.as_mut_ptr() as *mut _, buf.len()) };
                if n <= 0 {
                    break; // EAGAIN (non-blocking) or error: nothing more
                }
                out.extend(parse_buffer(&buf[..n as usize], &mut self.abs_prev[i]));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: u16, value: i32) -> [u8; INPUT_EVENT_SIZE] {
        let mut ev = [0u8; INPUT_EVENT_SIZE];
        ev[16..18].copy_from_slice(&EV_KEY.to_ne_bytes());
        ev[18..20].copy_from_slice(&code.to_ne_bytes());
        ev[20..24].copy_from_slice(&value.to_ne_bytes());
        ev
    }

    fn abs(code: u16, value: i32) -> [u8; INPUT_EVENT_SIZE] {
        let mut ev = [0u8; INPUT_EVENT_SIZE];
        ev[16..18].copy_from_slice(&EV_ABS.to_ne_bytes());
        ev[18..20].copy_from_slice(&code.to_ne_bytes());
        ev[20..24].copy_from_slice(&value.to_ne_bytes());
        ev
    }

    #[test]
    fn buttons_map_to_actions() {
        let mut prev = [0; 2];
        assert_eq!(
            feed_event(EV_KEY, BTN_SOUTH, 1, &mut prev),
            Some(Pad::Confirm)
        );
        assert_eq!(
            feed_event(EV_KEY, BTN_EAST, 1, &mut prev),
            Some(Pad::Cancel)
        );
        assert_eq!(feed_event(EV_KEY, BTN_TL, 1, &mut prev), Some(Pad::Prev));
        assert_eq!(feed_event(EV_KEY, BTN_TR, 1, &mut prev), Some(Pad::Next));
        assert_eq!(feed_event(EV_KEY, BTN_DPAD_UP, 1, &mut prev), Some(Pad::Up));
        assert_eq!(
            feed_event(EV_KEY, BTN_DPAD_LEFT, 1, &mut prev),
            Some(Pad::Left)
        );
    }

    #[test]
    fn key_release_and_autorepeat_are_ignored() {
        let mut prev = [0; 2];
        assert_eq!(feed_event(EV_KEY, BTN_SOUTH, 0, &mut prev), None);
        assert_eq!(feed_event(EV_KEY, BTN_SOUTH, 2, &mut prev), None);
    }

    #[test]
    fn stick_emits_only_on_threshold_crossing() {
        let mut prev = [0; 2];
        assert_eq!(feed_event(EV_ABS, ABS_Y, -20_000, &mut prev), Some(Pad::Up));
        assert_eq!(
            feed_event(EV_ABS, ABS_Y, -25_000, &mut prev),
            None,
            "held: no repeat"
        );
        assert_eq!(
            feed_event(EV_ABS, ABS_Y, 0, &mut prev),
            None,
            "return to center: nothing"
        );
        assert_eq!(
            feed_event(EV_ABS, ABS_Y, 20_000, &mut prev),
            Some(Pad::Down)
        );
        assert_eq!(
            feed_event(EV_ABS, ABS_X, 20_000, &mut prev),
            Some(Pad::Right)
        );
        assert_eq!(feed_event(EV_ABS, ABS_X, 5_000, &mut prev), None);
        assert_eq!(
            feed_event(EV_ABS, ABS_X, -20_000, &mut prev),
            Some(Pad::Left)
        );
    }

    #[test]
    fn parse_buffer_reads_a_stream_of_events() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&key(BTN_SOUTH, 1));
        buf.extend_from_slice(&key(BTN_SOUTH, 0));
        buf.extend_from_slice(&abs(ABS_X, 20_000));
        let mut prev = [0; 2];
        assert_eq!(
            parse_buffer(&buf, &mut prev),
            vec![Pad::Confirm, Pad::Right]
        );
    }
}
