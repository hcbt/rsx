use crate::gpu::Gpu;
use crate::irq::{self, Irq};
use crate::spu::Spu;

pub struct Dma {
    madr: [u32; 7],
    bcr: [u32; 7],
    chcr: [u32; 7],
    dpcr: u32,
    dicr: u32,
}

impl Dma {
    pub fn new() -> Self {
        Self {
            madr: [0; 7],
            bcr: [0; 7],
            chcr: [0; 7],
            dpcr: 0x0765_4321,
            dicr: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn read32(&self, addr: u32) -> u32 {
        let off = addr - 0x1F80_1080;
        if off < 0x70 {
            let ch = (off / 0x10) as usize;
            match off % 0x10 {
                0 => self.madr[ch] & 0x00FF_FFFF,
                4 => self.bcr[ch],
                8 | 0xC => self.chcr[ch],
                _ => 0,
            }
        } else {
            match addr {
                0x1F80_10F0 => self.dpcr,
                0x1F80_10F4 => self.dicr,
                _ => 0,
            }
        }
    }

    pub fn write32(
        &mut self,
        addr: u32,
        value: u32,
        ram: &mut [u8],
        gpu: &mut Gpu,
        spu: &mut Spu,
        irq: &mut Irq,
    ) {
        let off = addr.wrapping_sub(0x1F80_1080);
        if off < 0x70 {
            let ch = (off / 0x10) as usize;
            match off % 0x10 {
                0 => self.madr[ch] = value & 0x00FF_FFFF,
                4 => self.bcr[ch] = value,
                8 | 0xC => {
                    self.chcr[ch] = value;
                    if value & (1 << 24) != 0 {
                        self.run(ch, ram, gpu, spu, irq);
                    }
                }
                _ => {}
            }
        } else if addr == 0x1F80_10F0 {
            self.dpcr = value;
        } else if addr == 0x1F80_10F4 {
            let ack = (value & 0x7F00_0000) >> 24;
            let flags = (self.dicr & 0x7F00_0000) >> 24;
            let flags = flags & !ack;
            self.dicr = (value & 0x00FF_FFFF) | (flags << 24);
            self.update_master(irq);
        }
    }

    fn run(&mut self, ch: usize, ram: &mut [u8], gpu: &mut Gpu, spu: &mut Spu, irq: &mut Irq) {
        if (self.dpcr >> (ch * 4 + 3)) & 1 == 0 {
            self.chcr[ch] &= !(1 << 24);
            return;
        }
        match ch {
            2 => self.gpu(ram, gpu),
            4 => self.spu(ram, spu),
            6 => self.otc(ram),
            _ => {}
        }
        self.chcr[ch] &= !(1 << 24);
        self.complete(ch, irq);
    }

    fn gpu(&mut self, ram: &mut [u8], gpu: &mut Gpu) {
        let mode = (self.chcr[2] >> 9) & 3;
        let dir = self.chcr[2] & 1;
        if mode == 2 && dir == 1 {
            let mut addr = self.madr[2] & 0x1F_FFFF;
            for _ in 0..1_000_000 {
                if addr == 0x00FF_FFFF || addr > 0x1F_FFFF {
                    break;
                }
                let header = read32(ram, addr);
                let words = header >> 24;
                let next = header & 0x00FF_FFFF;
                for i in 0..words {
                    let w = read32(ram, addr.wrapping_add(4 + i * 4) & 0x1F_FFFF);
                    gpu.dma_write(w);
                }
                if next == 0x00FF_FFFF || next & 0x800000 != 0 {
                    break;
                }
                addr = next & 0x1F_FFFF;
            }
        } else if mode == 1 && dir == 1 {
            let bs = self.bcr[2] & 0xFFFF;
            let ba = self.bcr[2] >> 16;
            let mut addr = self.madr[2] & 0x1F_FFFF;
            for _ in 0..(bs * ba) {
                gpu.dma_write(read32(ram, addr));
                addr = addr.wrapping_add(4) & 0x1F_FFFF;
            }
        }
    }

    fn spu(&mut self, ram: &mut [u8], spu: &mut Spu) {
        let bs = self.bcr[4] & 0xFFFF;
        let ba = self.bcr[4] >> 16;
        let dir = self.chcr[4] & 1;
        let mut addr = self.madr[4] & 0x1F_FFFF;
        let n = if bs == 0 { 0x10000 } else { bs } * if ba == 0 { 1 } else { ba };
        for _ in 0..n {
            if dir == 1 {
                spu.dma_write16(read32(ram, addr) as u16);
                spu.dma_write16((read32(ram, addr) >> 16) as u16);
            }
            addr = addr.wrapping_add(4) & 0x1F_FFFF;
        }
    }

    fn otc(&mut self, ram: &mut [u8]) {
        let mut n = self.bcr[6] & 0xFFFF;
        if n == 0 {
            n = 0x10000;
        }
        let mut addr = self.madr[6] & 0x1F_FFFF;
        for i in 0..n {
            let next = if i == n - 1 {
                0x00FF_FFFF
            } else {
                addr.wrapping_sub(4) & 0x1F_FFFF
            };
            write32(ram, addr, next);
            addr = addr.wrapping_sub(4) & 0x1F_FFFF;
        }
    }

    fn complete(&mut self, ch: usize, irq: &mut Irq) {
        if self.dicr & (1 << 23) != 0 && self.dicr & (1 << (16 + ch)) != 0 {
            self.dicr |= 1 << (24 + ch);
            self.update_master(irq);
        }
    }

    fn update_master(&mut self, irq: &mut Irq) {
        let master = (self.dicr & (1 << 15) != 0)
            || (self.dicr & (1 << 23) != 0 && (self.dicr & 0x7F00_0000) != 0);
        if master {
            self.dicr |= 1 << 31;
            irq.raise(irq::IRQ_DMA);
        } else {
            self.dicr &= !(1 << 31);
        }
    }
}

fn read32(ram: &[u8], addr: u32) -> u32 {
    let a = (addr as usize) & (ram.len() - 1) & !3;
    u32::from_le_bytes(ram[a..a + 4].try_into().unwrap())
}

fn write32(ram: &mut [u8], addr: u32, v: u32) {
    let a = (addr as usize) & (ram.len() - 1) & !3;
    ram[a..a + 4].copy_from_slice(&v.to_le_bytes());
}
