//! Host DualShock 4 → Machine slot 1. The Machine never opens HID.
//!
//! On macOS a DualShock 4 over Bluetooth is a Game Controller device, not an
//! IOKit HID gamepad, so gilrs does not see it. Poll `GCController` first.

use gilrs::{Axis, Button, Gamepad, Gilrs};
use rsx_machine::{map_ds4_switches, HostButtons, Machine};

pub struct HostPad {
    gilrs: Option<Gilrs>,
    named: Option<String>,
    note: Option<String>,
}

impl HostPad {
    pub fn open() -> Self {
        match Gilrs::new() {
            Ok(g) => Self {
                gilrs: Some(g),
                named: None,
                note: None,
            },
            Err(e) => {
                eprintln!("host pad: {e}");
                Self {
                    gilrs: None,
                    named: None,
                    note: None,
                }
            }
        }
    }

    /// One-shot line for the I/O log when a host pad appears.
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    /// `None` means no host pad: slot 1 stays disconnected.
    pub fn poll(&mut self) -> Option<u16> {
        if let Some((name, buttons)) = poll_game_controller() {
            self.note_name(&name);
            let sw = map_ds4_switches(&buttons);
            self.note_buttons(sw);
            return Some(sw);
        }
        let gilrs = self.gilrs.as_mut()?;
        while gilrs.next_event().is_some() {}
        let id = gilrs
            .gamepads()
            .find(|(_, p)| p.is_connected())
            .map(|(id, _)| id)?;
        let (name, sw) = {
            let pad = gilrs.gamepad(id);
            (pad.name().to_string(), map_ds4_switches(&from_gilrs(&pad)))
        };
        self.note_name(&name);
        Some(sw)
    }

    fn note_name(&mut self, name: &str) {
        if self.named.as_deref() != Some(name) {
            let line = format!("host pad: {name} → slot 1");
            eprintln!("{line}");
            self.note = Some(line);
            self.named = Some(name.to_string());
        }
    }

    fn note_buttons(&mut self, sw: u16) {
        if sw == 0xFFFF {
            return;
        }
        let line = format!("host pad buttons {sw:04X}");
        if self.note.as_deref() != Some(line.as_str()) {
            eprintln!("{line}");
            self.note = Some(line);
        }
    }
}

/// DualShock 4 on macOS Game Controller: A=Cross, B=Circle, X=Square, Y=Triangle,
/// Menu=Options/Start, Options=Share/Select.
pub fn host_buttons_from_extended(
    a: bool,
    b: bool,
    x: bool,
    y: bool,
    menu: bool,
    options: bool,
    up: bool,
    right: bool,
    down: bool,
    left: bool,
    l1: bool,
    r1: bool,
    l2: bool,
    r2: bool,
    stick_x: f32,
    stick_y: f32,
) -> HostButtons {
    HostButtons {
        select: options,
        start: menu,
        up,
        right,
        down,
        left,
        l2,
        r2,
        l1,
        r1,
        triangle: y,
        circle: b,
        cross: a,
        square: x,
        stick_x,
        stick_y,
    }
}

fn poll_game_controller() -> Option<(String, HostButtons)> {
    #[cfg(target_os = "macos")]
    {
        macos::poll()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

pub fn inject(machine: &mut Machine, switches: Option<u16>) {
    machine.set_slot1_pad(switches);
}

#[cfg(target_os = "macos")]
mod macos {
    use super::host_buttons_from_extended;
    use objc2_game_controller::{
        GCController, GCControllerButtonInput, GCDevice, GCExtendedGamepad,
    };
    use rsx_machine::HostButtons;

    fn pressed(btn: &impl std::ops::Deref<Target = GCControllerButtonInput>) -> bool {
        // Analog face/shoulders: isPressed or the analog value past the click.
        unsafe { btn.isPressed() || btn.value() >= 0.5 }
    }

    fn name_of(c: &GCController) -> String {
        unsafe { c.vendorName() }
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| Some(unsafe { c.productCategory() }.to_string()))
            .unwrap_or_else(|| "Wireless Controller".into())
    }

    fn from_controller(c: &GCController) -> Option<HostButtons> {
        // capture() latches the live state vector; reading elements without
        // it can stay at released on macOS.
        let snap = unsafe { c.capture() };
        let gp = unsafe { snap.extendedGamepad() }?;
        Some(from_extended(&gp))
    }

    pub fn poll() -> Option<(String, HostButtons)> {
        let mut combined: Option<HostButtons> = None;
        let mut name = String::new();
        // The pad the user just touched, then every other extended pad OR'd
        // so a second "PS4 Controller" entry does not hide the DualShock 4.
        if let Some(c) = unsafe { GCController::current() } {
            if let Some(b) = from_controller(&c) {
                name = name_of(&c);
                combined = Some(b);
            }
        }
        let controllers = unsafe { GCController::controllers() };
        for i in 0..controllers.count() {
            let c = controllers.objectAtIndex(i);
            let Some(b) = from_controller(&c) else {
                continue;
            };
            if name.is_empty() {
                name = name_of(&c);
            }
            combined = Some(match combined {
                Some(a) => merge_host(a, b),
                None => b,
            });
        }
        combined.map(|b| (name, b))
    }

    fn merge_host(a: HostButtons, b: HostButtons) -> HostButtons {
        HostButtons {
            select: a.select || b.select,
            start: a.start || b.start,
            up: a.up || b.up,
            right: a.right || b.right,
            down: a.down || b.down,
            left: a.left || b.left,
            l2: a.l2 || b.l2,
            r2: a.r2 || b.r2,
            l1: a.l1 || b.l1,
            r1: a.r1 || b.r1,
            triangle: a.triangle || b.triangle,
            circle: a.circle || b.circle,
            cross: a.cross || b.cross,
            square: a.square || b.square,
            stick_x: if a.stick_x.abs() >= b.stick_x.abs() {
                a.stick_x
            } else {
                b.stick_x
            },
            stick_y: if a.stick_y.abs() >= b.stick_y.abs() {
                a.stick_y
            } else {
                b.stick_y
            },
        }
    }

    fn from_extended(gp: &GCExtendedGamepad) -> HostButtons {
        let dpad = unsafe { gp.dpad() };
        let stick = unsafe { gp.leftThumbstick() };
        let options = unsafe { gp.buttonOptions() }
            .map(|b| pressed(&b))
            .unwrap_or(false);
        host_buttons_from_extended(
            pressed(&unsafe { gp.buttonA() }),
            pressed(&unsafe { gp.buttonB() }),
            pressed(&unsafe { gp.buttonX() }),
            pressed(&unsafe { gp.buttonY() }),
            pressed(&unsafe { gp.buttonMenu() }),
            options,
            pressed(&unsafe { dpad.up() }),
            pressed(&unsafe { dpad.right() }),
            pressed(&unsafe { dpad.down() }),
            pressed(&unsafe { dpad.left() }),
            pressed(&unsafe { gp.leftShoulder() }),
            pressed(&unsafe { gp.rightShoulder() }),
            pressed(&unsafe { gp.leftTrigger() }),
            pressed(&unsafe { gp.rightTrigger() }),
            unsafe { stick.xAxis().value() },
            unsafe { stick.yAxis().value() },
        )
    }
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
        let wait_rx = |rt: u32| -> [u32; 5] {
            [
                0x240F_0200,
                0x25EF_FFFF,
                0x15E0_FFFE,
                0x0000_0000,
                0x9100_0000 | (rt << 16),
            ]
        };
        let mut words = vec![
            0x3C08_1F80u32,
            0x3508_1040,
            0x2409_000D,
            0xA509_0008,
            0x2409_0088,
            0xA509_000E,
            0x2409_1003,
            0xA509_000A,
            0x2409_0001,
            0xA109_0000,
        ];
        words.extend_from_slice(&wait_rx(10));
        words.extend_from_slice(&[0x2409_0042, 0xA109_0000]);
        words.extend_from_slice(&wait_rx(11));
        words.push(0xA100_0000);
        words.extend_from_slice(&wait_rx(12));
        words.push(0xA100_0000);
        words.extend_from_slice(&wait_rx(13));
        words.push(0xA100_0000);
        words.extend_from_slice(&wait_rx(14));
        for (i, w) in words.iter().enumerate() {
            data[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&data).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn ds4_game_controller_menu_is_start_and_a_is_cross() {
        let b = host_buttons_from_extended(
            true, false, false, false, true, false, false, false, false, false, false, false,
            false, false, 0.0, 0.0,
        );
        let sw = map_ds4_switches(&b);
        assert_eq!(sw & (1 << 14), 0, "buttonA is Cross");
        assert_eq!(sw & (1 << 3), 0, "buttonMenu (Options) is Start");
        assert_eq!(sw | (1 << 14) | (1 << 3), 0xFFFF);
    }

    #[test]
    fn present_injects_mapped_switches_into_slot1() {
        let f = joy_read_bios();
        let mut m = Machine::from_bios_path(f.path()).unwrap();
        inject(&mut m, None);
        m.run_until_cycle(m.cycles() + 500_000);
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
        m.run_until_cycle(m.cycles() + 500_000);
        assert_eq!(m.gpr(10), 0xFF, "High-Z");
        assert_eq!(m.gpr(11), 0x41, "idlo");
        assert_eq!(m.gpr(12), 0x5A, "idhi");
        let sw = m.gpr(13) as u16 | (m.gpr(14) as u16) << 8;
        assert_eq!(sw & (1 << 14), 0, "Cross injected on present");
        assert_eq!(sw | (1 << 14), 0xFFFF);
    }
}
