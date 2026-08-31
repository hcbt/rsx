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

    pub fn target(&self, i: usize) -> u16 {
        self.target[i]
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
            let v = u32::from(self.value[i]);
            let tgt = u32::from(self.target[i]);
            let to_ovf = 0x1_0000 - v;
            let to_tgt = if tgt > v { tgt - v } else { 0x1_0000 - v + tgt };
            let n = step.min(to_ovf).min(to_tgt).max(1);
            self.value[i] = self.value[i].wrapping_add(n as u16);
            step -= n;
            if self.value[i] == 0 {
                self.mode[i] |= 1 << 12;
                if self.mode[i] & (1 << 5) != 0 {
                    irq.set_level(irq::IRQ_TMR0 + i as u8, true);
                    irq.set_level(irq::IRQ_TMR0 + i as u8, false);
                }
            }
            // SPX: bit 4 IRQs when the counter equals target, whether or not
            // bit 3 (reset-to-0) is set. A large tick must not skip the compare.
            if self.value[i] == self.target[i] {
                self.mode[i] |= 1 << 11;
                if self.mode[i] & (1 << 4) != 0 {
                    irq.set_level(irq::IRQ_TMR0 + i as u8, true);
                    irq.set_level(irq::IRQ_TMR0 + i as u8, false);
                }
                if self.mode[i] & 8 != 0 {
                    self.value[i] = 0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::irq::Irq;

    const T2_VAL: u32 = 0x1F80_1120;
    const T2_MODE: u32 = 0x1F80_1124;
    const T2_TGT: u32 = 0x1F80_1128;

    fn t2_pending(irq: &Irq) -> bool {
        irq.read16(0x1F80_1070) & (1 << 6) != 0
    }

    #[test]
    fn irq_on_target_does_not_need_reset() {
        let mut t = Timers::new();
        let mut irq = Irq::new();
        t.write16(T2_TGT, 0x1000);
        // bit 4 = IRQ on target, bits 9-8 = 2 = sysclk/8. No bit 3 reset.
        t.write16(T2_MODE, (1 << 4) | (2 << 8));
        t.tick(0x1000 * 8, &mut irq);
        assert!(
            t2_pending(&irq),
            "Timer 2 must IRQ at target 0x1000 without reset (val={:04X} mode={:04X})",
            t.value(2),
            t.mode(2)
        );
    }

    #[test]
    fn irq_on_target_survives_a_tick_that_would_skip_it() {
        let mut t = Timers::new();
        let mut irq = Irq::new();
        t.write16(T2_TGT, 0x1000);
        // PSY-Q SetRCnt(CNT2, 0x1000, INTR): reset + IRQ on target + repeat + /8.
        t.write16(T2_MODE, 0x0258);
        t.write16(T2_VAL, 0x0F00);
        // 2160 CPU cycles = 270 sysclk/8 ticks; 0x0F00+270 skips 0x1000 if we add in one go.
        t.tick(2160, &mut irq);
        assert!(
            t2_pending(&irq),
            "Timer 2 must IRQ when a large tick crosses the target (val={:04X})",
            t.value(2)
        );
    }

    #[test]
    fn timer_i_stat_is_edge_not_re_raised_every_tick() {
        let mut t = Timers::new();
        let mut irq = Irq::new();
        t.write16(T2_TGT, 0x1000);
        t.write16(T2_MODE, (1 << 4) | (2 << 8));
        t.tick(0x1000 * 8, &mut irq);
        assert!(t2_pending(&irq), "first target sets I_STAT.TMR2");
        irq.write16(0x1F80_1070, !(1 << 6));
        assert!(!t2_pending(&irq), "I_STAT.TMR2 acked");
        t.tick(8, &mut irq);
        assert!(
            !t2_pending(&irq),
            "timer I_STAT must not re-raise every tick while the IRQ line stays high"
        );
    }
}
