use crate::cdrom::Cdrom;
use crate::gpu::Gpu;
use crate::irq::{self, Irq};
use crate::spu::Spu;

/// Cycles after a transfer before DICR/IRQ3 assert. The BIOS often arms the
/// wait *after* writing CHCR; raising in that same write misses it.
const IRQ_DELAY: u32 = 256;

pub struct Dma {
    madr: [u32; 7],
    bcr: [u32; 7],
    chcr: [u32; 7],
    dpcr: u32,
    dicr: u32,
    irq_delay: u32,
}

impl Dma {
    pub fn new() -> Self {
        Self {
            madr: [0; 7],
            bcr: [0; 7],
            chcr: [0; 7],
            dpcr: 0x0765_4321,
            dicr: 0,
            irq_delay: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, cycles: u32, irq: &mut Irq) {
        if self.irq_delay == 0 {
            return;
        }
        if cycles >= self.irq_delay {
            self.irq_delay = 0;
            self.update_master(irq);
        } else {
            self.irq_delay -= cycles;
        }
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
        cdrom: &mut Cdrom,
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
                        self.run(ch, ram, gpu, spu, cdrom, irq);
                    }
                }
                _ => {}
            }
        } else if addr == 0x1F80_10F0 {
            self.dpcr = value;
        } else if addr == 0x1F80_10F4 {
            // Bits 24-30 are W1C. The BIOS IRQ handler does
            //   (DICR & 00FFFFFFh) | 88000000h
            // which sets bit 31 (read-only master flag) rather than the
            // channel flag it just handled; treat a write with bit 31 set
            // as acknowledging every pending channel.
            let mut ack = (value & 0x7F00_0000) >> 24;
            if value & (1 << 31) != 0 {
                ack = 0x7F;
            }
            let flags = (self.dicr & 0x7F00_0000) >> 24;
            let flags = flags & !ack;
            self.dicr = (value & 0x00FF_FFFF) | (flags << 24);
            self.update_master(irq);
        }
    }

    fn run(
        &mut self,
        ch: usize,
        ram: &mut [u8],
        gpu: &mut Gpu,
        spu: &mut Spu,
        cdrom: &mut Cdrom,
        irq: &mut Irq,
    ) {
        if (self.dpcr >> (ch * 4 + 3)) & 1 == 0 {
            self.chcr[ch] &= !(1 << 24);
            return;
        }
        match ch {
            2 => self.gpu(ram, gpu),
            3 => self.cd(ram, cdrom),
            4 => self.spu(ram, spu),
            6 => self.otc(ram),
            _ => {}
        }
        self.chcr[ch] &= !(1 << 24);
        self.complete(ch);
        self.update_master(irq);
    }

    fn cd(&mut self, ram: &mut [u8], cdrom: &mut Cdrom) {
        let bs = self.bcr[3] & 0xFFFF;
        let ba = self.bcr[3] >> 16;
        let bs = if bs == 0 { 0x1_0000 } else { bs };
        let ba = if ba == 0 { 1 } else { ba };
        let n = bs.saturating_mul(ba).min(1_000_000);
        let mut addr = self.madr[3] & 0x1F_FFFF;
        for _ in 0..n {
            write32(ram, addr, cdrom.dma_read32());
            addr = addr.wrapping_add(4) & 0x1F_FFFF;
        }
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
                if next == 0x00FF_FFFF {
                    break;
                }
                addr = next & 0x1F_FFFF;
            }
        } else if mode == 1 && dir == 1 {
            let bs = self.bcr[2] & 0xFFFF;
            let ba = self.bcr[2] >> 16;
            let bs = if bs == 0 { 0x1_0000 } else { bs };
            let ba = if ba == 0 { 0x1_0000 } else { ba };
            let n = bs.saturating_mul(ba).min(1_000_000);
            let mut addr = self.madr[2] & 0x1F_FFFF;
            for _ in 0..n {
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

    fn complete(&mut self, ch: usize) {
        if self.dicr & (1 << 23) != 0 && self.dicr & (1 << (16 + ch)) != 0 {
            self.dicr |= 1 << (24 + ch);
            self.irq_delay = self.irq_delay.max(IRQ_DELAY);
        }
    }

    fn update_master(&mut self, irq: &mut Irq) {
        let flagged = (self.dicr & (1 << 15) != 0)
            || (self.dicr & (1 << 23) != 0 && (self.dicr & 0x7F00_0000) != 0);
        let was_master = self.dicr & (1 << 31) != 0;
        if flagged && self.irq_delay == 0 {
            self.dicr |= 1 << 31;
            // I_STAT.3 is edge-triggered on DICR.31 0→1. A DICR RMW that
            // leaves flags set must not re-assert after the BIOS acked I_STAT.
            if !was_master {
                irq.raise(irq::IRQ_DMA);
            }
        } else if !flagged {
            self.dicr &= !(1 << 31);
            self.irq_delay = 0;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdrom::Cdrom;
    use crate::gpu::Gpu;
    use crate::irq::Irq;
    use crate::spu::Spu;

    fn xy(x: i32, y: i32) -> u32 {
        (x as u16 as u32) | ((y as u16 as u32) << 16)
    }

    fn poke(ram: &mut [u8], addr: u32, v: u32) {
        write32(ram, addr, v);
    }

    fn peek(ram: &[u8], addr: u32) -> u32 {
        read32(ram, addr)
    }

    #[test]
    fn otc_then_linked_list_delivers_far_then_near() {
        let mut dma = Dma::new();
        let mut ram = vec![0u8; 0x20_0000];
        let mut gpu = Gpu::new();
        let mut spu = Spu::new();
        let mut cdrom = Cdrom::new();
        let mut irq = Irq::new();

        gpu.gp0(0xE3 << 24);
        gpu.gp0(0xE4 << 24 | 1023 | (511 << 10));

        dma.write32(
            0x1F80_10F0,
            0xFFFF_FFFF,
            &mut ram,
            &mut gpu,
            &mut spu,
            &mut cdrom,
            &mut irq,
        );

        // Four-entry OT at 0x1000. MADR = last entry.
        dma.write32(
            0x1F80_10E0,
            0x100C,
            &mut ram,
            &mut gpu,
            &mut spu,
            &mut cdrom,
            &mut irq,
        );
        dma.write32(
            0x1F80_10E4,
            4,
            &mut ram,
            &mut gpu,
            &mut spu,
            &mut cdrom,
            &mut irq,
        );
        dma.write32(
            0x1F80_10E8,
            0x1100_0002,
            &mut ram,
            &mut gpu,
            &mut spu,
            &mut cdrom,
            &mut irq,
        );

        assert_eq!(peek(&ram, 0x1000), 0x00FF_FFFF, "OTC last written is end");
        assert_eq!(peek(&ram, 0x1004), 0x1000);
        assert_eq!(peek(&ram, 0x1008), 0x1004);
        assert_eq!(peek(&ram, 0x100C), 0x1008);

        // Far (OTZ=3) red triangle, then near (OTZ=1) blue overlapping it.
        // Insert prepends: [pkt] = OT[z] | (N<<24); OT[z] = pkt.
        poke(&mut ram, 0x2000, (4 << 24) | 0x1008);
        poke(&mut ram, 0x2004, 0x20 << 24 | 0x0000F8);
        poke(&mut ram, 0x2008, xy(10, 10));
        poke(&mut ram, 0x200C, xy(40, 10));
        poke(&mut ram, 0x2010, xy(10, 40));
        poke(&mut ram, 0x100C, 0x2000);

        poke(&mut ram, 0x2100, (4 << 24) | 0x1000);
        poke(&mut ram, 0x2104, 0x20 << 24 | 0xF80000);
        poke(&mut ram, 0x2108, xy(15, 12));
        poke(&mut ram, 0x210C, xy(35, 12));
        poke(&mut ram, 0x2110, xy(15, 32));
        poke(&mut ram, 0x1004, 0x2100);

        dma.write32(
            0x1F80_10A0,
            0x100C,
            &mut ram,
            &mut gpu,
            &mut spu,
            &mut cdrom,
            &mut irq,
        );
        dma.write32(
            0x1F80_10A8,
            0x0100_0401,
            &mut ram,
            &mut gpu,
            &mut spu,
            &mut cdrom,
            &mut irq,
        );

        let overlap = gpu.vram_rect(16, 13, 8, 8);
        let blue = overlap
            .pixels
            .iter()
            .filter(|p| **p & 0x7FFF == 0x7C00)
            .count();
        let red = overlap
            .pixels
            .iter()
            .filter(|p| **p & 0x7FFF == 0x001F)
            .count();
        assert!(
            blue > 8 && red == 0,
            "far then near: overlap must be the near (blue) primitive (blue={blue} red={red} pix={:04X?})",
            overlap.pixels
        );
    }
}
