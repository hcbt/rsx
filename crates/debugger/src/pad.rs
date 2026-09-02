//! Host DualShock 4 / HID gamepad → Machine slot 1. The Machine never opens HID.

use gilrs::{Axis, Button, Gamepad, Gilrs};
use rsx_machine::{map_ds4_switches, HostButtons, Machine};

pub struct HostPad {
    gilrs: Option<Gilrs>,
    named: bool,
}

impl HostPad {
    pub fn open() -> Self {
        match Gilrs::new() {
            Ok(g) => Self {
                gilrs: Some(g),
                named: false,
            },
            Err(e) => {
                eprintln!("host pad: {e}");
                Self {
                    gilrs: None,
                    named: false,
                }
            }
        }
    }

    /// `None` means no host pad: slot 1 stays disconnected.
    pub fn poll(&mut self) -> Option<u16> {
        let gilrs = self.gilrs.as_mut()?;
        while gilrs.next_event().is_some() {}
        let id = gilrs
            .gamepads()
            .find(|(_, p)| p.is_connected())
            .map(|(id, _)| id)?;
        let pad = gilrs.gamepad(id);
        if !self.named {
            eprintln!("host pad: {} → slot 1", pad.name());
            self.named = true;
        }
        Some(map_ds4_switches(&from_gilrs(&pad)))
    }
}

pub fn inject(machine: &mut Machine, switches: Option<u16>) {
    machine.set_slot1_pad(switches);
}

fn pressed(pad: &Gamepad<'_>, btn: Button) -> bool {
    pad.is_pressed(btn)
}

fn axis(pad: &Gamepad<'_>, a: Axis) -> f32 {
    pad.axis_data(a).map(|d| d.value()).unwrap_or(0.0)
}

fn from_gilrs(pad: &Gamepad<'_>) -> HostButtons {
    // gilrs LeftStickY: −1 is typically up (SDL / DS4 HID). HostButtons uses +1 up.
    let stick_y = -axis(pad, Axis::LeftStickY);
    let hat_x = axis(pad, Axis::DPadX);
    let hat_y = -axis(pad, Axis::DPadY);
    HostButtons {
        select: pressed(pad, Button::Select),
        start: pressed(pad, Button::Start),
        up: pressed(pad, Button::DPadUp) || hat_y >= 0.5,
        right: pressed(pad, Button::DPadRight) || hat_x >= 0.5,
        down: pressed(pad, Button::DPadDown) || hat_y <= -0.5,
        left: pressed(pad, Button::DPadLeft) || hat_x <= -0.5,
        l2: pressed(pad, Button::LeftTrigger2) || axis(pad, Axis::LeftZ) >= 0.5,
        r2: pressed(pad, Button::RightTrigger2) || axis(pad, Axis::RightZ) >= 0.5,
        l1: pressed(pad, Button::LeftTrigger),
        r1: pressed(pad, Button::RightTrigger),
        triangle: pressed(pad, Button::North),
        circle: pressed(pad, Button::East),
        cross: pressed(pad, Button::South),
        square: pressed(pad, Button::West),
        stick_x: axis(pad, Axis::LeftStickX),
        stick_y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joy_read_bios() -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut data = vec![0u8; 512 * 1024];
        let words = [
            0x3C08_1F80u32,
            0x3508_1040,
            0x2409_1003,
            0xA509_000A,
            0x2409_0001,
            0xA109_0000,
            0x0000_0000,
            0x910A_0000,
            0x0000_0000,
            0x2409_0042,
            0xA109_0000,
            0x0000_0000,
            0x910B_0000,
            0x0000_0000,
            0xA100_0000,
            0x0000_0000,
            0x910C_0000,
            0x0000_0000,
            0xA100_0000,
            0x0000_0000,
            0x910D_0000,
            0x0000_0000,
            0xA100_0000,
            0x0000_0000,
            0x910E_0000,
        ];
        for (i, w) in words.iter().enumerate() {
            data[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&data).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn present_injects_mapped_switches_into_slot1() {
        let f = joy_read_bios();
        let mut m = Machine::from_bios_path(f.path()).unwrap();
        inject(&mut m, None);
        for _ in 0..32 {
            m.step();
        }
        assert_eq!(
            [m.gpr(10), m.gpr(11), m.gpr(12), m.gpr(13), m.gpr(14)],
            [0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            "no host pad: every clock is 0xFF"
        );

        let mut m = Machine::from_bios_path(f.path()).unwrap();
        inject(
            &mut m,
            Some(map_ds4_switches(&HostButtons {
                cross: true,
                ..HostButtons::default()
            })),
        );
        for _ in 0..32 {
            m.step();
        }
        assert_eq!(m.gpr(10), 0xFF, "High-Z");
        assert_eq!(m.gpr(11), 0x41, "idlo");
        assert_eq!(m.gpr(12), 0x5A, "idhi");
        let sw = m.gpr(13) as u16 | (m.gpr(14) as u16) << 8;
        assert_eq!(sw & (1 << 14), 0, "Cross injected on present");
        assert_eq!(sw | (1 << 14), 0xFFFF);
    }
}
