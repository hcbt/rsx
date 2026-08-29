/// I_STAT / I_MASK at 1F801070h / 1F801074h.
#[derive(Default)]
pub struct Irq {
    stat: u16,
    mask: u16,
}

impl Irq {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn raise(&mut self, bit: u8) {
        let prev = self.stat;
        self.stat |= 1 << bit;
        let _ = prev;
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
