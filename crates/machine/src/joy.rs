use std::collections::VecDeque;

use crate::irq::{Irq, IRQ_PAD};

/// Host-side DualShock 4 / standard-pad buttons. `true` is pressed.
/// Machine-free of HID: the Debugger fills this from the host device.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HostButtons {
    /// Share → Select.
    pub select: bool,
    /// Options → Start.
    pub start: bool,
    pub up: bool,
    pub right: bool,
    pub down: bool,
    pub left: bool,
    pub l2: bool,
    pub r2: bool,
    pub l1: bool,
    pub r1: bool,
    pub triangle: bool,
    pub circle: bool,
    pub cross: bool,
    pub square: bool,
    /// −1.0 left … +1.0 right
    pub stick_x: f32,
    /// −1.0 down … +1.0 up
    pub stick_y: f32,
}

const STICK_DIGITAL: f32 = 0.5;

/// Active-low digital switches (SPX Standard Digital Pad).
pub fn map_ds4_switches(b: &HostButtons) -> u16 {
    let mut sw = 0xFFFFu16;
    let mut press = |on: bool, bit: u16| {
        if on {
            sw &= !bit;
        }
    };
    press(b.select, 1 << 0);
    press(b.start, 1 << 3);
    press(b.up || b.stick_y >= STICK_DIGITAL, 1 << 4);
    press(b.right || b.stick_x >= STICK_DIGITAL, 1 << 5);
    press(b.down || b.stick_y <= -STICK_DIGITAL, 1 << 6);
    press(b.left || b.stick_x <= -STICK_DIGITAL, 1 << 7);
    press(b.l2, 1 << 8);
    press(b.r2, 1 << 9);
    press(b.l1, 1 << 10);
    press(b.r1, 1 << 11);
    press(b.triangle, 1 << 12);
    press(b.circle, 1 << 13);
    press(b.cross, 1 << 14);
    press(b.square, 1 << 15);
    sw
}

/// SPX: Kernel waits ~100 cycles after TX, then acks old IRQ7, then waits
/// for the new one. /ACK in that window is ignored. Timeout is 100µs
/// (~3387 cycles) from the last SCK. Fire in between.
const ACK_DELAY: u32 = 250;
/// SPX: /ACK LOW duration is circa 100 clock cycles.
const ACK_HOLD: u32 = 100;

const CTRL_SELECT: u16 = 1 << 1;
const CTRL_ACK: u16 = 1 << 4;
const CTRL_RESET: u16 = 1 << 6;
const CTRL_ACK_IRQ: u16 = 1 << 12;
const CTRL_SLOT: u16 = 1 << 13;

/// Controllers and memory cards. Empty ports clock 0xFF into RX with no
/// `/ACK`. Slot 1 may be a standard digital pad (ID `5A41h`).
pub struct Joy {
    mode: u16,
    ctrl: u16,
    baud: u16,
    rx: VecDeque<u8>,
    /// None = disconnected. Some = active-low digital switches.
    slot1: Option<u16>,
    /// 0 = address byte; 1..4 = digital Read; after last, back to 0.
    xfer: u8,
    ack_in: u32,
    ack_low: u32,
    irq_stat: bool,
    last_ctrl: u16,
    last_tx: u8,
    tx_count: u32,
    ack_armed: u32,
}

impl Joy {
    pub fn new() -> Self {
        Self {
            mode: 0,
            ctrl: 0,
            baud: 0x0088,
            rx: VecDeque::with_capacity(8),
            slot1: None,
            xfer: 0,
            ack_in: 0,
            ack_low: 0,
            irq_stat: false,
            last_ctrl: 0,
            last_tx: 0,
            tx_count: 0,
            ack_armed: 0,
        }
    }

    pub fn reset(&mut self) {
        let slot1 = self.slot1;
        let last_ctrl = self.last_ctrl;
        let last_tx = self.last_tx;
        let tx_count = self.tx_count;
        let ack_armed = self.ack_armed;
        *self = Self::new();
        self.slot1 = slot1;
        self.last_ctrl = last_ctrl;
        self.last_tx = last_tx;
        self.tx_count = tx_count;
        self.ack_armed = ack_armed;
    }

    /// Connect a standard digital pad on slot 1, or `None` to unplug.
    pub fn set_slot1(&mut self, switches: Option<u16>) {
        self.slot1 = switches;
        if switches.is_none() {
            self.xfer = 0;
            self.ack_in = 0;
            self.ack_low = 0;
        }
    }

    pub fn last_ctrl(&self) -> u16 {
        self.last_ctrl
    }

    pub fn last_tx(&self) -> u8 {
        self.last_tx
    }

    pub fn tx_count(&self) -> u32 {
        self.tx_count
    }

    pub fn ack_armed(&self) -> u32 {
        self.ack_armed
    }

    pub fn tick(&mut self, mut cycles: u32, irq: &mut Irq) {
        if self.ack_in > 0 {
            if cycles < self.ack_in {
                self.ack_in -= cycles;
                return;
            }
            cycles -= self.ack_in;
            self.ack_in = 0;
            self.ack_low = ACK_HOLD;
            self.irq_stat = true;
            if self.ctrl & CTRL_ACK_IRQ != 0 {
                irq.set_level(IRQ_PAD, true);
            }
        }
        if self.ack_low > 0 {
            let low = self.ack_low;
            self.ack_low = self.ack_low.saturating_sub(cycles);
            if low > 0 && self.ack_low == 0 {
                irq.set_level(IRQ_PAD, false);
            }
        }
    }

    pub fn stat(&self) -> u16 {
        let mut s = 0x0005; // TX ready 1 and 2
        if !self.rx.is_empty() {
            s |= 1 << 1;
        }
        if self.ack_low > 0 {
            s |= 1 << 7;
        }
        if self.irq_stat {
            s |= 1 << 9;
        }
        s
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        match addr & 0xF {
            0 => self.read_rx(),
            4 => self.stat() as u8,
            5 => (self.stat() >> 8) as u8,
            8 => self.mode as u8,
            9 => (self.mode >> 8) as u8,
            0xA => self.ctrl as u8,
            0xB => (self.ctrl >> 8) as u8,
            0xE => self.baud as u8,
            0xF => (self.baud >> 8) as u8,
            _ => 0,
        }
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        match addr & 0xF {
            0 => u16::from(self.read_rx()),
            4 => self.stat(),
            8 => self.mode,
            0xA => self.ctrl,
            0xE => self.baud,
            _ => 0,
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        match addr & 0xF {
            0 => self.write_tx(value),
            8 => self.mode = (self.mode & 0xFF00) | u16::from(value),
            9 => self.mode = (self.mode & 0x00FF) | (u16::from(value) << 8),
            0xA => self.write_ctrl((self.ctrl & 0xFF00) | u16::from(value)),
            0xB => self.write_ctrl((self.ctrl & 0x00FF) | (u16::from(value) << 8)),
            0xE => self.baud = (self.baud & 0xFF00) | u16::from(value),
            0xF => self.baud = (self.baud & 0x00FF) | (u16::from(value) << 8),
            _ => {}
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        match addr & 0xF {
            0 => self.write_tx(value as u8),
            8 => self.mode = value,
            0xA => self.write_ctrl(value),
            0xE => self.baud = value,
            _ => {}
        }
    }

    fn read_rx(&mut self) -> u8 {
        self.rx.pop_front().unwrap_or(0xFF)
    }

    fn push_rx(&mut self, b: u8) {
        if self.rx.len() >= 8 {
            self.rx.pop_front();
        }
        self.rx.push_back(b);
    }

    fn slot1_selected(&self) -> bool {
        self.ctrl & CTRL_SELECT != 0 && self.ctrl & CTRL_SLOT == 0
    }

    fn write_tx(&mut self, value: u8) {
        self.last_tx = value;
        self.tx_count = self.tx_count.saturating_add(1);
        if self.ctrl & 0x7 == 0 {
            return;
        }
        let pad = self.slot1.filter(|_| self.slot1_selected());
        let Some(switches) = pad else {
            self.push_rx(0xFF);
            self.xfer = 0;
            return;
        };
        match self.xfer {
            0 => {
                self.push_rx(0xFF);
                if value == 0x01 {
                    self.xfer = 1;
                    self.schedule_ack();
                }
            }
            1 => {
                self.push_rx(0x41);
                self.xfer = 2;
                self.schedule_ack();
            }
            2 => {
                self.push_rx(0x5A);
                self.xfer = 3;
                self.schedule_ack();
            }
            3 => {
                self.push_rx(switches as u8);
                self.xfer = 4;
                self.schedule_ack();
            }
            4 => {
                self.push_rx((switches >> 8) as u8);
                self.xfer = 0;
            }
            _ => {
                self.push_rx(0xFF);
                self.xfer = 0;
            }
        }
    }

    fn schedule_ack(&mut self) {
        self.ack_in = ACK_DELAY;
        self.ack_low = 0;
        self.ack_armed = self.ack_armed.saturating_add(1);
    }

    fn write_ctrl(&mut self, value: u16) {
        self.last_ctrl = value;
        if value & CTRL_RESET != 0 {
            self.reset();
            // Bit 6 is a pulse. The other bits in this write are the new CTRL
            // (PSY-Q PadRead: Reset | TXEN | /JOYn | ACK IRQ in one halfword).
            self.ctrl = value & !(CTRL_ACK | CTRL_RESET);
            return;
        }
        if value & CTRL_ACK != 0 && self.ack_low == 0 {
            self.irq_stat = false;
        }
        let was = self.slot1_selected();
        self.ctrl = value & !(CTRL_ACK | CTRL_RESET);
        if was && !self.slot1_selected() {
            self.xfer = 0;
            self.ack_in = 0;
            self.ack_low = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::irq::{Irq, IRQ_PAD};

    fn select_slot1(joy: &mut Joy) {
        joy.write16(0x1F80_104A, 0x1003);
    }

    #[test]
    fn ctrl_reset_pulse_keeps_txen_and_select() {
        let mut joy = Joy::new();
        let mut irq = Irq::new();
        joy.set_slot1(Some(0xFFFF));
        // Reset | ACK IRQ | TXEN | /JOYn in one write (PSY-Q PadRead).
        joy.write16(0x1F80_104A, 0x1043);
        let rx = tx_rx(&mut joy, 0x01);
        assert_eq!(rx, 0xFF, "High-Z on address byte");
        joy.tick(250, &mut irq);
        assert_ne!(
            irq.read16(0x1F80_1070) & (1 << IRQ_PAD),
            0,
            "Reset pulse must not drop TXEN/select before the 01h /ACK"
        );
    }

    fn tx_rx(joy: &mut Joy, tx: u8) -> u8 {
        joy.write8(0x1F80_1040, tx);
        joy.read8(0x1F80_1040)
    }

    #[test]
    fn disconnected_tx_clocks_ff_and_does_not_raise_irq7() {
        let mut joy = Joy::new();
        let mut irq = Irq::new();
        select_slot1(&mut joy);
        let rx = tx_rx(&mut joy, 0x01);
        joy.tick(200, &mut irq);
        assert_eq!(rx, 0xFF);
        assert_eq!(irq.read16(0x1F80_1070) & (1 << IRQ_PAD), 0);
    }

    #[test]
    fn rx_fifo_holds_a_full_digital_read() {
        let mut joy = Joy::new();
        joy.set_slot1(Some(0xFFFF));
        select_slot1(&mut joy);
        joy.write8(0x1F80_1040, 0x01);
        joy.write8(0x1F80_1040, 0x42);
        joy.write8(0x1F80_1040, 0x00);
        joy.write8(0x1F80_1040, 0x00);
        joy.write8(0x1F80_1040, 0x00);
        let got: [u8; 5] = std::array::from_fn(|_| joy.read8(0x1F80_1040));
        assert_eq!(got, [0xFF, 0x41, 0x5A, 0xFF, 0xFF]);
    }

    #[test]
    fn connected_digital_read_is_41_5a_and_active_low_switches() {
        let mut joy = Joy::new();
        joy.set_slot1(Some(0xFFFF));
        select_slot1(&mut joy);
        let b0 = tx_rx(&mut joy, 0x01);
        let b1 = tx_rx(&mut joy, 0x42);
        let b2 = tx_rx(&mut joy, 0x00);
        let b3 = tx_rx(&mut joy, 0x00);
        let b4 = tx_rx(&mut joy, 0x00);
        assert_eq!([b0, b1, b2, b3, b4], [0xFF, 0x41, 0x5A, 0xFF, 0xFF]);
    }

    #[test]
    fn connected_cross_clears_only_bit14() {
        let mut joy = Joy::new();
        joy.set_slot1(Some(0xFFFF & !(1 << 14)));
        select_slot1(&mut joy);
        let _ = tx_rx(&mut joy, 0x01);
        let _ = tx_rx(&mut joy, 0x42);
        let _ = tx_rx(&mut joy, 0x00);
        let swlo = tx_rx(&mut joy, 0x00);
        let swhi = tx_rx(&mut joy, 0x00);
        let sw = u16::from(swlo) | (u16::from(swhi) << 8);
        assert_eq!(sw & (1 << 14), 0, ">< Cross pressed is active-low bit14");
        assert_eq!(
            sw | (1 << 14),
            0xFFFF,
            "Select/Start/D-pad/shoulders stay released"
        );
    }

    #[test]
    fn ack_does_not_raise_irq7_on_the_tx_cycle() {
        let mut joy = Joy::new();
        let mut irq = Irq::new();
        joy.set_slot1(Some(0xFFFF));
        select_slot1(&mut joy);
        let _ = tx_rx(&mut joy, 0x01);
        assert_eq!(
            irq.read16(0x1F80_1070) & (1 << IRQ_PAD),
            0,
            "I_STAT.7 must not rise in the same cycle as TX"
        );
        joy.tick(100, &mut irq);
        assert_eq!(
            irq.read16(0x1F80_1070) & (1 << IRQ_PAD),
            0,
            "Kernel waits ~100 cycles then acks old IRQ7; /ACK in that window is lost"
        );
        joy.tick(150, &mut irq);
        assert_ne!(
            irq.read16(0x1F80_1070) & (1 << IRQ_PAD),
            0,
            "new IRQ7 after the Kernel wait, still inside the 100µs timeout"
        );
    }

    #[test]
    fn last_digital_read_byte_does_not_ack() {
        let mut joy = Joy::new();
        let mut irq = Irq::new();
        joy.set_slot1(Some(0xFFFF));
        select_slot1(&mut joy);
        let _ = tx_rx(&mut joy, 0x01);
        joy.tick(300, &mut irq);
        irq.write16(0x1F80_1070, !(1 << IRQ_PAD));
        let _ = tx_rx(&mut joy, 0x42);
        joy.tick(300, &mut irq);
        irq.write16(0x1F80_1070, !(1 << IRQ_PAD));
        let _ = tx_rx(&mut joy, 0x00);
        joy.tick(300, &mut irq);
        irq.write16(0x1F80_1070, !(1 << IRQ_PAD));
        let _ = tx_rx(&mut joy, 0x00);
        joy.tick(300, &mut irq);
        irq.write16(0x1F80_1070, !(1 << IRQ_PAD));
        let _ = tx_rx(&mut joy, 0x00);
        joy.tick(300, &mut irq);
        assert_eq!(
            irq.read16(0x1F80_1070) & (1 << IRQ_PAD),
            0,
            "no /ACK after the last digital Read byte"
        );
    }

    #[test]
    fn ds4_face_shoulders_start_select_dpad_map_to_switch_bits() {
        let all = HostButtons {
            select: true,
            start: true,
            up: true,
            right: true,
            down: true,
            left: true,
            l2: true,
            r2: true,
            l1: true,
            r1: true,
            triangle: true,
            circle: true,
            cross: true,
            square: true,
            stick_x: 0.0,
            stick_y: 0.0,
        };
        assert_eq!(
            map_ds4_switches(&all),
            1 << 1 | 1 << 2,
            "L3/R3 stay released on a digital pad; every other switch pressed"
        );
        assert_eq!(map_ds4_switches(&HostButtons::default()), 0xFFFF);

        let mut only = HostButtons::default();
        only.cross = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 14));
        only = HostButtons::default();
        only.circle = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 13));
        only = HostButtons::default();
        only.square = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 15));
        only = HostButtons::default();
        only.triangle = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 12));
        only = HostButtons::default();
        only.start = true; // Options
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 3));
        only = HostButtons::default();
        only.select = true; // Share
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 0));
        only = HostButtons::default();
        only.up = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 4));
        only = HostButtons::default();
        only.right = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 5));
        only = HostButtons::default();
        only.down = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 6));
        only = HostButtons::default();
        only.left = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 7));
        only = HostButtons::default();
        only.l1 = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 10));
        only = HostButtons::default();
        only.r1 = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 11));
        only = HostButtons::default();
        only.l2 = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 8));
        only = HostButtons::default();
        only.r2 = true;
        assert_eq!(map_ds4_switches(&only), 0xFFFF & !(1 << 9));
    }

    #[test]
    fn ds4_left_stick_up_clears_joypad_up() {
        let dpad = HostButtons {
            up: true,
            ..HostButtons::default()
        };
        let stick = HostButtons {
            stick_y: 1.0,
            ..HostButtons::default()
        };
        assert_eq!(map_ds4_switches(&dpad) & (1 << 4), 0);
        assert_eq!(
            map_ds4_switches(&stick) & (1 << 4),
            0,
            "left-stick up is Joypad Up"
        );
        let rest = HostButtons {
            stick_y: 0.2,
            stick_x: -0.2,
            ..HostButtons::default()
        };
        assert_eq!(map_ds4_switches(&rest), 0xFFFF, "resting stick is released");
    }

    #[test]
    fn slot2_stays_disconnected_ff() {
        let mut joy = Joy::new();
        let mut irq = Irq::new();
        joy.set_slot1(Some(0xFFFF));
        joy.write16(0x1F80_104A, 0x1003 | (1 << 13));
        let rx = tx_rx(&mut joy, 0x01);
        joy.tick(200, &mut irq);
        assert_eq!(rx, 0xFF);
        assert_eq!(irq.read16(0x1F80_1070) & (1 << IRQ_PAD), 0);
    }
}
