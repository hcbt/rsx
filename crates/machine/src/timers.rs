use crate::irq::{self, Irq};

pub struct Timers {
    value: [u16; 3],
    mode: [u16; 3],
    target: [u16; 3],
}

impl Timers {
    pub fn new() -> Self {
        Self {
            value: [0; 3],
            mode: [0; 3],
            target: [0; 3],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, cycles: u32, irq: &mut Irq) {
        for i in 0..3 {
            let src = (self.mode[i] >> 8) & 3;
            let sysclk = match i {
                2 => src == 0 || src == 1,
                _ => src == 0 || src == 2,
            };
            if !sysclk {
                continue;
            }
            let step = if i == 2 && (src == 2 || src == 3) {
                cycles / 8
            } else {
                cycles
            };
            self.advance(i, step as u32, irq);
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
