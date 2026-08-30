/// Controllers and memory cards: empty ports. TX still clocks 0xFF into RX.
pub struct Joy {
    mode: u16,
    ctrl: u16,
    baud: u16,
    rx: Option<u8>,
}

impl Joy {
    pub fn new() -> Self {
        Self {
            mode: 0,
            ctrl: 0,
            baud: 0x0088,
            rx: None,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn stat(&self) -> u16 {
        let mut s = 0x0005; // TX ready 1 and 2
        if self.rx.is_some() {
            s |= 1 << 1;
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
        self.rx.take().unwrap_or(0xFF)
    }

    fn write_tx(&mut self, _value: u8) {
        // No device: data line idles high. Store a byte if RX is enabled
        // (slot selected or RXEN). No /ACK, so no IRQ7.
        if self.ctrl & 0x7 != 0 {
            self.rx = Some(0xFF);
        }
    }

    fn write_ctrl(&mut self, value: u16) {
        if value & (1 << 6) != 0 {
            self.reset();
            return;
        }
        // Bits 4 (ACK) and 6 (Reset) are write-only.
        self.ctrl = value & !((1 << 4) | (1 << 6));
    }
}
