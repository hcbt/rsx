/// SPU that answers the BIOS: registers, RAM, DMA. No host audio.
pub struct Spu {
    ram: Vec<u8>,
    regs: [u16; 0x200],
    transfer_addr: u32,
    applied_mode: u16,
    apply_delay: u32,
    transfer_busy: u32,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            ram: vec![0; 512 * 1024],
            regs: [0; 0x200],
            transfer_addr: 0,
            applied_mode: 0,
            apply_delay: 0,
            transfer_busy: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, cycles: u32) {
        if self.apply_delay > 0 {
            if cycles >= self.apply_delay {
                self.apply_delay = 0;
                self.applied_mode = self.spucnt() & 0x3F;
                let mode = (self.applied_mode >> 4) & 3;
                if mode == 1 {
                    // Manual write: busy for a short burst after mode applies.
                    self.transfer_busy = 0x80;
                }
            } else {
                self.apply_delay -= cycles;
            }
        }
        if self.transfer_busy > 0 {
            self.transfer_busy = self.transfer_busy.saturating_sub(cycles);
        }
    }

    fn spucnt(&self) -> u16 {
        self.regs[0xD5]
    }

    pub fn read16(&self, addr: u32) -> u16 {
        let off = ((addr - 0x1F80_1C00) >> 1) as usize;
        if off == 0xD7 {
            return self.stat();
        }
        self.regs.get(off).copied().unwrap_or(0)
    }

    fn stat(&self) -> u16 {
        let mut s = self.applied_mode & 0x3F;
        let mode = (self.applied_mode >> 4) & 3;
        if mode == 2 {
            s |= 1 << 8; // DMA write request
        }
        if mode == 3 {
            s |= 1 << 9; // DMA read request
        }
        if mode == 2 || mode == 3 {
            s |= 1 << 7;
        }
        if self.transfer_busy > 0 {
            s |= 1 << 10;
        }
        s
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        let off = ((addr - 0x1F80_1C00) >> 1) as usize;
        if off == 0xD3 {
            self.transfer_addr = u32::from(value) << 3;
        } else if off == 0xD4 {
            self.dma_write16(value);
        } else if off == 0xD5 {
            self.regs[0xD5] = value;
            // Hardware delays ~300h clocks; apply immediately so wait-loops
            // on SPUSTAT bits 0-5 can complete. Pulse transfer-busy so a
            // following "wait until busy clears" also sees a 1-to-0 edge.
            self.applied_mode = value & 0x3F;
            if (value >> 4) & 3 == 1 {
                self.transfer_busy = 0x80;
            }
            return;
        }
        if off < self.regs.len() {
            self.regs[off] = value;
        }
    }

    pub fn dma_write16(&mut self, value: u16) {
        let a = (self.transfer_addr as usize) & (self.ram.len() - 1) & !1;
        self.ram[a..a + 2].copy_from_slice(&value.to_le_bytes());
        self.transfer_addr = self.transfer_addr.wrapping_add(2);
    }
}
