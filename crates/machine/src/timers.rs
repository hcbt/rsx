use crate::irq::{self, Irq};

pub struct Timers {
    value: [u16; 3],
    mode: [u16; 3],
    target: [u16; 3],
    sysclk8_frac: u32,
}

impl Timers {
    pub fn new() -> Self {
        Self {
            value: [0; 3],
            mode: [0; 3],
            target: [0; 3],
            sysclk8_frac: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn value(&self, i: usize) -> u16 {
        self.value[i]
    }

    pub fn mode(&self, i: usize) -> u16 {
        self.mode[i]
    }

    pub fn tick(&mut self, cycles: u32, irq: &mut Irq) {
        for i in 0..3 {
            let src = (self.mode[i] >> 8) & 3;
            let step = match i {
                2 if src == 2 || src == 3 => {
                    self.sysclk8_frac += cycles;
                    let s = self.sysclk8_frac / 8;
                    self.sysclk8_frac %= 8;
                    s
                }
                2 if src == 0 || src == 1 => cycles,
                _ if src == 0 || src == 2 => cycles,
                _ => 0,
            };
            if step > 0 {
                self.advance(i, step, irq);
            }
        }
    }

    /// One horizontal blanking pulse. Timer 1 can clock from this.
    pub fn hblank(&mut self, irq: &mut Irq) {
        let src = (self.mode[1] >> 8) & 3;
        if src == 1 || src == 3 {
            self.advance(1, 1, irq);
        }
    }

    fn advance(&mut self, i: usize, mut step: u32, irq: &mut Irq) {
        while step > 0 {
            let room = 0xFFFFu32.saturating_sub(u32::from(self.value[i])) + 1;
            let n = step.min(room);
            self.value[i] = self.value[i].wrapping_add(n as u16);
            step -= n;
            if self.value[i] == 0 {
                self.mode[i] |= 1 << 12;
                if self.mode[i] & (1 << 5) != 0 {
                    irq.raise(irq::IRQ_TMR0 + i as u8);
                }
            }
            if self.mode[i] & 8 != 0 && self.value[i] == self.target[i] {
                self.mode[i] |= 1 << 11;
                self.value[i] = 0;
                if self.mode[i] & (1 << 4) != 0 {
                    irq.raise(irq::IRQ_TMR0 + i as u8);
                }
            }
        }
    }

    pub fn read16(&mut self, addr: u32) -> u16 {
        let i = ((addr - 0x1F80_1100) / 0x10) as usize;
        match addr & 0xF {
            0 => self.value[i],
            4 => {
                let m = self.mode[i];
                self.mode[i] &= !0x1800;
                m
            }
            8 => self.target[i],
            _ => 0,
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        let i = ((addr - 0x1F80_1100) / 0x10) as usize;
        match addr & 0xF {
            0 => self.value[i] = value,
            4 => {
                self.mode[i] = value | (1 << 10);
                self.value[i] = 0;
            }
            8 => self.target[i] = value,
            _ => {}
        }
    }
}
