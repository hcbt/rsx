/// I_STAT / I_MASK at 1F801070h / 1F801074h.
#[derive(Default)]
pub struct Irq {
    stat: u16,
    mask: u16,
    /// Source levels for edge-triggered bits (CD HINTSTS, etc.).
    level: u16,
}

impl Irq {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn raise(&mut self, bit: u8) {
        self.stat |= 1 << bit;
    }

    /// SPX: I_STAT bits are set only on the source's false→true.
    pub fn set_level(&mut self, bit: u8, high: bool) {
        let m = 1u16 << bit;
        let was = self.level & m != 0;
        if high {
            self.level |= m;
            if !was {
                self.stat |= m;
            }
        } else {
            self.level &= !m;
        }
    }

    pub fn pending_for_cop0(&self) -> bool {
        (self.stat & self.mask) != 0
    }

    pub fn read16(&self, addr: u32) -> u16 {
        match addr & 0xF {
            0x0 => self.stat,
            0x4 => self.mask,
            _ => 0,
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        match addr & 0xF {
            0x0 => self.stat &= value, // write 0 clears
            0x4 => self.mask = value & 0x7FF,
            _ => {}
        }
    }
}

#[allow(dead_code)]
pub const IRQ_VBLANK: u8 = 0;
#[allow(dead_code)]
pub const IRQ_GPU: u8 = 1;
pub const IRQ_CDROM: u8 = 2;
pub const IRQ_DMA: u8 = 3;
pub const IRQ_TMR0: u8 = 4;
pub const IRQ_TMR1: u8 = 5;
pub const IRQ_TMR2: u8 = 6;
pub const IRQ_PAD: u8 = 7;
pub const IRQ_SIO: u8 = 8;
pub const IRQ_SPU: u8 = 9;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i_stat_write_zero_clears_and_write_one_leaves_the_bit() {
        let mut irq = Irq::new();
        irq.raise(IRQ_CDROM);
        irq.raise(IRQ_VBLANK);
        irq.write16(0x1F80_1070, !(1 << IRQ_CDROM));
        assert_eq!(irq.read16(0x1F80_1070) & (1 << IRQ_CDROM), 0);
        assert_ne!(irq.read16(0x1F80_1070) & (1 << IRQ_VBLANK), 0);
        irq.write16(0x1F80_1070, 0xFFFF);
        assert_ne!(
            irq.read16(0x1F80_1070) & (1 << IRQ_VBLANK),
            0,
            "write 1 must not ack"
        );
    }
}
