/// SPU that answers the BIOS: registers, RAM, DMA. No host audio.
pub struct Spu {
    ram: Vec<u8>,
    regs: [u16; 0x200],
    transfer_addr: u32,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            ram: vec![0; 512 * 1024],
            regs: [0; 0x200],
            transfer_addr: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn read16(&self, addr: u32) -> u16 {
        let off = ((addr - 0x1F80_1C00) >> 1) as usize;
        if off == 0xD7 {
            // SPUSTAT
            let cnt = self.regs[0xD5];
            return cnt & 0x3F;
        }
        self.regs.get(off).copied().unwrap_or(0)
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        let off = ((addr - 0x1F80_1C00) >> 1) as usize;
        if off == 0xD3 {
            self.transfer_addr = u32::from(value) << 3;
        } else if off == 0xD4 {
            self.dma_write16(value);
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
