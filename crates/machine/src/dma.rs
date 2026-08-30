use crate::cdrom::Cdrom;
use crate::gpu::Gpu;
use crate::irq::{self, Irq};
use crate::spu::Spu;

/// Extra clocks per 16 words (DRAM hyper-page row load). SPX: ~17 clks / 16 words.

enum Job {
    Block {
        ch: u8,
        addr: u32,
        remaining: u32,
        dir: u8,
        step: i32,
    },
    List {
        addr: u32,
        pkt_left: u32,
        next: u32,
        nodes: u32,
    },
}

pub struct Dma {
    madr: [u32; 7],
    bcr: [u32; 7],
    chcr: [u32; 7],
    dpcr: u32,
    dicr: u32,
    job: Option<Job>,
    pending_ch: Option<usize>,
    credit: u32,
    words_done: u32,
    hyper_pause: bool,
    list_empty: u32,
    list_pkts: u32,
    list_min: u32,
    list_max: u32,
    list_empty_before: u32,
    list_seen_pkt: bool,
    pub last_list_empty: u32,
    pub last_list_pkts: u32,
    pub last_list_min: u32,
    pub last_list_max: u32,
    pub last_list_start: u32,
    pub last_list_start_n: u32,
    pub last_empty_before: u32,
}

impl Dma {
    pub fn new() -> Self {
        Self {
            madr: [0; 7],
            bcr: [0; 7],
            chcr: [0; 7],
            dpcr: 0x0765_4321,
            dicr: 0,
            job: None,
            pending_ch: None,
            credit: 0,
            words_done: 0,
            hyper_pause: false,
            list_empty: 0,
            list_pkts: 0,
            list_min: u32::MAX,
            list_max: 0,
            list_empty_before: 0,
            list_seen_pkt: false,
            last_list_empty: 0,
            last_list_pkts: 0,
            last_list_min: 0,
            last_list_max: 0,
            last_list_start: 0,
            last_list_start_n: 0,
            last_empty_before: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn active(&self) -> bool {
        self.job.is_some()
    }

    pub fn gpu_from_ram(&self) -> bool {
        match self.job {
            Some(Job::List { .. }) => true,
            Some(Job::Block { ch: 2, dir: 1, .. }) => true,
            _ => false,
        }
    }

    pub fn waiting_on_gpu(&self, gpu: &Gpu) -> bool {
        if !gpu.fifo_full() {
            return false;
        }
        match self.job {
            Some(Job::List { pkt_left, .. }) if pkt_left > 0 => true,
            Some(Job::Block {
                ch: 2,
                dir: 1,
                remaining,
                ..
            }) if remaining > 0 => true,
            _ => false,
        }
    }

    pub fn occupies_ram(&self, gpu: &Gpu) -> bool {
        self.job.is_some() && !self.waiting_on_gpu(gpu)
    }

    pub fn burst_cycles(&self) -> u32 {
        match self.job {
            Some(Job::Block { remaining, ch, .. }) if ch != 2 => remaining
                .saturating_mul(Self::clocks_per_word(ch as usize))
                .saturating_add((remaining + 15) / 16)
                .max(1),
            _ => 1,
        }
    }

    pub fn tick(
        &mut self,
        mut cycles: u32,
        ram: &mut [u8],
        gpu: &mut Gpu,
        spu: &mut Spu,
        cdrom: &mut Cdrom,
        irq: &mut Irq,
    ) {
        while cycles > 0 && self.job.is_some() {
            if self.waiting_on_gpu(gpu) {
                cycles -= 1;
                continue;
            }
            if self.hyper_pause {
                self.hyper_pause = false;
                cycles -= 1;
                continue;
            }
            let ch = self.job_ch();
            let rate = Self::clocks_per_word(ch as usize);
            if self.credit + 1 < rate {
                self.credit += 1;
                cycles -= 1;
                continue;
            }
            self.credit = 0;
            self.transfer_one(ram, gpu, spu, cdrom);
            cycles -= 1;
            if self.job.is_none() {
                self.finish(ch as usize, ram, irq);
                if self.job.is_none() {
                    break;
                }
            }
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
        _gpu: &mut Gpu,
        _spu: &mut Spu,
        _cdrom: &mut Cdrom,
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
                        self.start(ch, ram);
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

    fn start(&mut self, ch: usize, ram: &[u8]) {
        if (self.dpcr >> (ch * 4 + 3)) & 1 == 0 {
            self.chcr[ch] &= !(1 << 24);
            return;
        }
        if self.job.is_some() {
            self.pending_ch = Some(ch);
            return;
        }
        self.credit = 0;
        self.words_done = 0;
        self.hyper_pause = false;
        self.job = match ch {
            2 => self.start_gpu(ram),
            3 => Some(Job::Block {
                ch: 3,
                addr: self.madr[3] & 0x1F_FFFF,
                remaining: block_words(self.bcr[3], 1),
                dir: (self.chcr[3] & 1) as u8,
                step: 4,
            }),
            4 => Some(Job::Block {
                ch: 4,
                addr: self.madr[4] & 0x1F_FFFF,
                remaining: block_words(self.bcr[4], 1),
                dir: (self.chcr[4] & 1) as u8,
                step: 4,
            }),
            6 => {
                let mut n = self.bcr[6] & 0xFFFF;
                if n == 0 {
                    n = 0x10000;
                }
                Some(Job::Block {
                    ch: 6,
                    addr: self.madr[6] & 0x1F_FFFF,
                    remaining: n,
                    dir: 0,
                    step: -4,
                })
            }
            _ => None,
        };
        if self.job.is_none() {
            self.chcr[ch] &= !(1 << 24);
        }
    }

    fn start_gpu(&mut self, ram: &[u8]) -> Option<Job> {
        let mode = (self.chcr[2] >> 9) & 3;
        let dir = (self.chcr[2] & 1) as u8;
        let addr = self.madr[2] & 0x1F_FFFF;
        if mode == 2 && dir == 1 {
            self.list_empty = 0;
            self.list_pkts = 0;
            self.list_min = u32::MAX;
            self.list_max = 0;
            self.list_empty_before = 0;
            self.list_seen_pkt = false;
            self.last_list_start = addr;
            self.last_list_start_n = read32(ram, addr) >> 24;
            Some(Job::List {
                addr,
                pkt_left: 0,
                next: 0,
                nodes: 0,
            })
        } else if mode == 1 && dir == 1 {
            Some(Job::Block {
                ch: 2,
                addr,
                remaining: block_words(self.bcr[2], 0x1_0000),
                dir: 1,
                step: 4,
            })
        } else {
            None
        }
    }

    fn clocks_per_word(ch: usize) -> u32 {
        match ch {
            3 => 24,
            4 => 4,
            5 => 20,
            _ => 1,
        }
    }

    fn job_ch(&self) -> u8 {
        match self.job {
            Some(Job::List { .. }) => 2,
            Some(Job::Block { ch, .. }) => ch,
            None => 0,
        }
    }

    fn transfer_one(&mut self, ram: &mut [u8], gpu: &mut Gpu, spu: &mut Spu, cdrom: &mut Cdrom) {
        match self.job {
            Some(Job::List { .. }) => self.list_word(ram, gpu),
            Some(Job::Block { ch: 6, .. }) => self.otc_word(ram),
            Some(Job::Block { ch: 3, .. }) => self.cd_word(ram, cdrom),
            Some(Job::Block { ch: 4, .. }) => self.spu_word(ram, spu),
            Some(Job::Block { ch: 2, .. }) => self.gpu_block_word(ram, gpu),
            _ => self.job = None,
        }
        if self.job.is_some() {
            self.words_done += 1;
            if self.words_done % 16 == 0 {
                self.hyper_pause = true;
            }
        }
    }

    fn list_word(&mut self, ram: &[u8], gpu: &mut Gpu) {
        let Some(Job::List {
            addr,
            pkt_left,
            nodes,
            ..
        }) = &self.job
        else {
            return;
        };
        let addr = *addr;
        let pkt_left = *pkt_left;
        let nodes = *nodes;
        if pkt_left == 0 {
            if addr == 0x00FF_FFFF || addr > 0x1F_FFFF || nodes >= 1_000_000 {
                self.finish_list_stats();
                self.job = None;
                return;
            }
            self.list_min = self.list_min.min(addr);
            self.list_max = self.list_max.max(addr);
            let header = read32(ram, addr);
            let words = header >> 24;
            let next = header & 0x00FF_FFFF;
            if words == 0 {
                self.list_empty += 1;
                if !self.list_seen_pkt {
                    self.list_empty_before += 1;
                }
                if next == 0x00FF_FFFF {
                    self.finish_list_stats();
                    self.job = None;
                    return;
                }
                if let Some(Job::List {
                    addr,
                    nodes,
                    next: n,
                    ..
                }) = self.job.as_mut()
                {
                    *addr = next & 0x1F_FFFF;
                    *nodes += 1;
                    *n = next;
                }
                return;
            }
            self.list_pkts += 1;
            self.list_seen_pkt = true;
            if let Some(Job::List {
                addr,
                pkt_left,
                next: n,
                nodes,
            }) = self.job.as_mut()
            {
                *pkt_left = words;
                *addr = addr.wrapping_add(4) & 0x1F_FFFF;
                *n = next;
                *nodes += 1;
            }
            return;
        }
        gpu.dma_write(read32(ram, addr));
        let ended = {
            let Some(Job::List {
                addr,
                pkt_left,
                next,
                ..
            }) = self.job.as_mut()
            else {
                return;
            };
            *addr = addr.wrapping_add(4) & 0x1F_FFFF;
            *pkt_left -= 1;
            if *pkt_left == 0 {
                if *next == 0x00FF_FFFF {
                    true
                } else {
                    *addr = *next & 0x1F_FFFF;
                    false
                }
            } else {
                false
            }
        };
        if ended {
            self.finish_list_stats();
            self.job = None;
        }
    }

    fn finish_list_stats(&mut self) {
        if self.list_pkts > self.last_list_pkts {
            self.last_list_empty = self.list_empty;
            self.last_list_pkts = self.list_pkts;
            self.last_list_min = self.list_min;
            self.last_list_max = self.list_max;
            self.last_empty_before = self.list_empty_before;
        }
    }

    fn otc_word(&mut self, ram: &mut [u8]) {
        let done = {
            let Some(Job::Block {
                addr,
                remaining,
                step,
                ..
            }) = self.job.as_mut()
            else {
                return;
            };
            let next = if *remaining == 1 {
                0x00FF_FFFF
            } else {
                (*addr as i32).wrapping_add(*step) as u32 & 0x1F_FFFF
            };
            write32(ram, *addr, next);
            *addr = (*addr as i32).wrapping_add(*step) as u32 & 0x1F_FFFF;
            *remaining -= 1;
            *remaining == 0
        };
        if done {
            self.job = None;
        }
    }

    fn cd_word(&mut self, ram: &mut [u8], cdrom: &mut Cdrom) {
        let done = {
            let Some(Job::Block {
                addr, remaining, ..
            }) = self.job.as_mut()
            else {
                return;
            };
            write32(ram, *addr, cdrom.dma_read32());
            *addr = addr.wrapping_add(4) & 0x1F_FFFF;
            *remaining -= 1;
            *remaining == 0
        };
        if done {
            self.job = None;
        }
    }

    fn spu_word(&mut self, ram: &mut [u8], spu: &mut Spu) {
        let done = {
            let Some(Job::Block {
                addr,
                remaining,
                dir,
                ..
            }) = self.job.as_mut()
            else {
                return;
            };
            if *dir == 1 {
                let w = read32(ram, *addr);
                spu.dma_write16(w as u16);
                spu.dma_write16((w >> 16) as u16);
            }
            *addr = addr.wrapping_add(4) & 0x1F_FFFF;
            *remaining -= 1;
            *remaining == 0
        };
        if done {
            self.job = None;
        }
    }

    fn gpu_block_word(&mut self, ram: &[u8], gpu: &mut Gpu) {
        let done = {
            let Some(Job::Block {
                addr, remaining, ..
            }) = self.job.as_mut()
            else {
                return;
            };
            gpu.dma_write(read32(ram, *addr));
            *addr = addr.wrapping_add(4) & 0x1F_FFFF;
            *remaining -= 1;
            *remaining == 0
        };
        if done {
            self.job = None;
        }
    }

    fn finish(&mut self, ch: usize, ram: &[u8], irq: &mut Irq) {
        self.job = None;
        self.hyper_pause = false;
        self.credit = 0;
        self.chcr[ch] &= !(1 << 24);
        if self.dicr & (1 << 23) != 0 && self.dicr & (1 << (16 + ch)) != 0 {
            self.dicr |= 1 << (24 + ch);
        }
        self.update_master(irq);
        if let Some(next) = self.pending_ch.take() {
            if self.chcr[next] & (1 << 24) != 0 {
                self.start(next, ram);
            }
        }
    }

    fn update_master(&mut self, irq: &mut Irq) {
        let flagged = (self.dicr & (1 << 15) != 0)
            || (self.dicr & (1 << 23) != 0 && (self.dicr & 0x7F00_0000) != 0);
        let was_master = self.dicr & (1 << 31) != 0;
        if flagged {
            self.dicr |= 1 << 31;
            // I_STAT.3 is edge-triggered on DICR.31 0→1. A DICR RMW that
            // leaves flags set must not re-assert after the BIOS acked I_STAT.
            if !was_master {
                irq.raise(irq::IRQ_DMA);
            }
        } else {
            self.dicr &= !(1 << 31);
        }
    }
}

fn block_words(bcr: u32, ba_zero: u32) -> u32 {
    let bs = bcr & 0xFFFF;
    let ba = bcr >> 16;
    let bs = if bs == 0 { 0x1_0000 } else { bs };
    let ba = if ba == 0 { ba_zero } else { ba };
    bs.saturating_mul(ba).min(1_000_000)
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
        dma.tick(64, &mut ram, &mut gpu, &mut spu, &mut cdrom, &mut irq);

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
        for _ in 0..50_000 {
            if !dma.active() && gpu.fifo_is_empty() && !gpu.busy() {
                break;
            }
            gpu.tick(1, 0, false);
            dma.tick(1, &mut ram, &mut gpu, &mut spu, &mut cdrom, &mut irq);
        }

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
