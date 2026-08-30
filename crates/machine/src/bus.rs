use crate::cdrom::Cdrom;
use crate::dma::Dma;
use crate::gpu::Gpu;
use crate::irq::Irq;
use crate::joy::Joy;
use crate::spu::Spu;
use crate::timers::Timers;

const RAM_SIZE: usize = 2 * 1024 * 1024;
const CYCLES_PER_LINE: u64 = 2160;
const LINES_PER_FRAME: u32 = 263;
const VBLANK_START: u32 = 243;

pub struct Bus {
    ram: Vec<u8>,
    bios: Vec<u8>,
    scratch: [u8; 1024],
    memctrl: [u32; 9],
    ram_size: u32,
    cache_ctrl: u32,
    gpu: Gpu,
    spu: Spu,
    cdrom: Cdrom,
    dma: Dma,
    irq: Irq,
    joy: Joy,
    timers: Timers,
    cycles: u64,
    scanline: u32,
    vblanks: u64,
    in_vblank: bool,
    pub io_writes: u64,
    pub last_io: u32,
    pub io_cd: u64,
    pub io_spu: u64,
    pub io_irq: u64,
    pub io_gpu: u64,
}

impl Bus {
    pub fn new(bios: Vec<u8>) -> Self {
        Self {
            ram: vec![0; RAM_SIZE],
            bios,
            scratch: [0; 1024],
            memctrl: [
                0x1F00_0000,
                0x1F80_2000,
                0x0013_243F,
                0x0000_3022,
                0x0013_243F,
                0x2009_31E1,
                0x0002_0843,
                0x0007_0777,
                0x0003_1125,
            ],
            ram_size: 0x0000_0B88,
            cache_ctrl: 0,
            gpu: Gpu::new(),
            spu: Spu::new(),
            cdrom: Cdrom::new(),
            dma: Dma::new(),
            irq: Irq::new(),
            joy: Joy::new(),
            timers: Timers::new(),
            cycles: 0,
            scanline: 0,
            vblanks: 0,
            in_vblank: false,
            io_writes: 0,
            last_io: 0,
            io_cd: 0,
            io_spu: 0,
            io_irq: 0,
            io_gpu: 0,
        }
    }

    pub fn reset(&mut self) {
        self.ram.fill(0);
        self.scratch = [0; 1024];
        self.gpu.reset();
        self.spu.reset();
        self.cdrom.reset();
        self.dma.reset();
        self.irq.reset();
        self.joy.reset();
        self.timers.reset();
        self.cycles = 0;
        self.scanline = 0;
        self.vblanks = 0;
        self.in_vblank = false;
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    pub fn dma(&self) -> &crate::dma::Dma {
        &self.dma
    }

    pub fn insert_disc(&mut self, disc: crate::disc::Disc) {
        self.cdrom.insert(disc);
    }

    pub fn irq(&self) -> &Irq {
        &self.irq
    }

    pub fn timers(&self) -> &Timers {
        &self.timers
    }

    pub fn vblank_count(&self) -> u64 {
        self.vblanks
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn take_audio(&mut self) -> Vec<i16> {
        self.spu.take_samples()
    }

    pub fn memctrl_bios_delay(&self) -> u32 {
        self.memctrl[4]
    }

    pub fn cache_ctrl(&self) -> u32 {
        self.cache_ctrl
    }

    fn note_io(&mut self, p: u32) {
        match p {
            0x1F80_1800..=0x1F80_1803 => {
                self.io_cd += 1;
                self.io_writes += 1;
                self.last_io = p;
            }
            0x1F80_1C00..=0x1F80_1FFF => {
                self.io_spu += 1;
                self.io_writes += 1;
                self.last_io = p;
            }
            0x1F80_1070..=0x1F80_1076 => {
                self.io_irq += 1;
                self.io_writes += 1;
                self.last_io = p;
            }
            0x1F80_1810..=0x1F80_1814 => {
                self.io_gpu += 1;
                self.io_writes += 1;
                self.last_io = p;
            }
            0x1F80_1000..=0x1F80_1FFF | 0xFFFE_0130 => {
                self.io_writes += 1;
                self.last_io = p;
            }
            _ => {}
        }
    }

    pub fn ram_word(&self, addr: u32) -> u32 {
        let p = addr & 0x1FFF_FFFF;
        if p >= 0x1FC0_0000 {
            let off = (p - 0x1FC0_0000) as usize;
            return u32::from_le_bytes(
                self.bios
                    .get(off..off + 4)
                    .and_then(|s| s.try_into().ok())
                    .unwrap_or([0; 4]),
            );
        }
        let a = (p as usize) & (RAM_SIZE - 1) & !3;
        u32::from_le_bytes(self.ram[a..a + 4].try_into().unwrap())
    }

    pub fn tick(&mut self, cycles: u32) {
        self.cycles += u64::from(cycles);
        self.timers.tick(cycles, &mut self.irq);
        self.cdrom.tick(cycles, &mut self.irq);
        self.spu.tick(cycles);
        if self.spu.take_irq() {
            self.irq.raise(crate::irq::IRQ_SPU);
        }
        self.dma.tick(cycles, &mut self.irq);
        let line = ((self.cycles / CYCLES_PER_LINE) as u32) % LINES_PER_FRAME;
        if line != self.scanline {
            self.scanline = line;
            self.timers.hblank(&mut self.irq);
            let vblank = line >= VBLANK_START;
            if vblank && !self.in_vblank {
                self.vblanks += 1;
                self.irq.raise(crate::irq::IRQ_VBLANK);
            }
            self.in_vblank = vblank;
        }
        self.gpu.tick(self.in_vblank);
    }

    pub fn read8(&mut self, addr: u32) -> Option<u8> {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => Some(self.ram[(p as usize) & (RAM_SIZE - 1)]),
            0x1F80_0000..=0x1F80_03FF => Some(self.scratch[(p - 0x1F80_0000) as usize]),
            0x1F80_1040..=0x1F80_104F => Some(self.joy.read8(p)),
            0x1F80_1800..=0x1F80_1803 => Some(self.cdrom.read8(p)),
            0x1FC0_0000..=0x1FFF_FFFF => {
                let off = (p - 0x1FC0_0000) as usize;
                Some(*self.bios.get(off % self.bios.len()).unwrap_or(&0xFF))
            }
            _ => self.read32(addr).map(|v| {
                let shift = (p & 3) * 8;
                (v >> shift) as u8
            }),
        }
    }

    pub fn read16(&mut self, addr: u32) -> Option<u16> {
        let p = phys(addr);
        match p {
            0x1F80_1C00..=0x1F80_1FFF => Some(self.spu.read16(p)),
            0x1F80_1070..=0x1F80_1076 => Some(self.irq.read16(p)),
            0x1F80_1100..=0x1F80_112F => Some(self.timers.read16(p)),
            0x1F80_1040..=0x1F80_104F => Some(self.joy.read16(p)),
            0x1F80_1054 => Some(0x0005),
            _ => self.read32(addr & !3).map(|v| {
                if p & 2 != 0 {
                    (v >> 16) as u16
                } else {
                    v as u16
                }
            }),
        }
    }

    pub fn read32(&mut self, addr: u32) -> Option<u32> {
        let p = phys(addr) & !3;
        Some(match p {
            0x0000_0000..=0x007F_FFFF => {
                let a = (p as usize) & (RAM_SIZE - 1);
                u32::from_le_bytes(self.ram[a..a + 4].try_into().unwrap())
            }
            0x1F80_0000..=0x1F80_03FF => {
                let a = (p - 0x1F80_0000) as usize;
                u32::from_le_bytes(self.scratch[a..a + 4].try_into().ok()?)
            }
            0x1F80_1000..=0x1F80_1020 => self.memctrl[((p - 0x1F80_1000) / 4) as usize],
            0x1F80_1060 => self.ram_size,
            0x1F80_1070 | 0x1F80_1074 => u32::from(self.irq.read16(p)),
            0x1F80_1080..=0x1F80_10FC => self.dma.read32(p),
            0x1F80_1100..=0x1F80_1128 => u32::from(self.timers.read16(p)),
            0x1F80_1810 => self.gpu.read_gpuread(),
            0x1F80_1814 => self.gpu.stat(),
            0x1F80_1C00..=0x1F80_1FFF => u32::from(self.spu.read16(p)),
            0x1FC0_0000..=0x1FFF_FFFF => {
                let off = ((p - 0x1FC0_0000) as usize) % self.bios.len();
                u32::from_le_bytes(self.bios[off..off + 4].try_into().unwrap_or([0; 4]))
            }
            0xFFFE_0130 => self.cache_ctrl,
            0x1F80_1040 => u32::from(self.joy.read16(p)),
            0x1F80_1044 => u32::from(self.joy.stat()),
            0x1F80_1048 => {
                u32::from(self.joy.read16(p)) | (u32::from(self.joy.read16(p + 2)) << 16)
            }
            0x1F80_1050..=0x1F80_105C => 0x0000_0005,
            _ => 0xFFFF_FFFF, // open bus-ish
        })
    }

    pub fn write8(&mut self, addr: u32, value: u8) {
        let p = phys(addr);
        match p {
            0x0000_0000..=0x007F_FFFF => {
                self.ram[(p as usize) & (RAM_SIZE - 1)] = value;
            }
            0x1F80_0000..=0x1F80_03FF => {
                self.scratch[(p - 0x1F80_0000) as usize] = value;
            }
            0x1F80_1040..=0x1F80_104F => self.joy.write8(p, value),
            0x1F80_1800..=0x1F80_1803 => {
                self.note_io(p);
                self.cdrom.write8(p, value, &mut self.irq);
            }
            0x1F80_2041 => {} // POST
            _ => {
                let shift = (p & 3) * 8;
                if let Some(cur) = self.read32(addr & !3) {
                    self.write32(
                        addr & !3,
                        (cur & !(0xFF << shift)) | (u32::from(value) << shift),
                    );
                }
            }
        }
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        let p = phys(addr);
        self.note_io(p);
        match p {
            0x1F80_1C00..=0x1F80_1FFF => self.spu.write16(p, value),
            0x1F80_1040..=0x1F80_104F => self.joy.write16(p, value),
            0x1F80_1070..=0x1F80_1076 => self.irq.write16(p, value),
            0x1F80_1100..=0x1F80_112F => self.timers.write16(p, value),
            _ => {
                let cur = self.read32(addr & !3).unwrap_or(0);
                let v = if p & 2 != 0 {
                    (cur & 0x0000_FFFF) | (u32::from(value) << 16)
                } else {
                    (cur & 0xFFFF_0000) | u32::from(value)
                };
                self.write32(addr & !3, v);
            }
        }
    }

    pub fn write32(&mut self, addr: u32, value: u32) {
        let p = phys(addr) & !3;
        self.note_io(p);
        match p {
            0x0000_0000..=0x007F_FFFF => {
                let a = (p as usize) & (RAM_SIZE - 1);
                self.ram[a..a + 4].copy_from_slice(&value.to_le_bytes());
            }
            0x1F80_0000..=0x1F80_03FF => {
                let a = (p - 0x1F80_0000) as usize;
                if a + 4 <= self.scratch.len() {
                    self.scratch[a..a + 4].copy_from_slice(&value.to_le_bytes());
                }
            }
            0x1F80_1000..=0x1F80_1020 => {
                self.memctrl[((p - 0x1F80_1000) / 4) as usize] = value;
            }
            0x1F80_1060 => self.ram_size = value,
            0x1F80_1070 | 0x1F80_1074 => self.irq.write16(p, value as u16),
            0x1F80_1080..=0x1F80_10FC => {
                self.dma.write32(
                    p,
                    value,
                    &mut self.ram,
                    &mut self.gpu,
                    &mut self.spu,
                    &mut self.cdrom,
                    &mut self.irq,
                );
            }
            0x1F80_1100..=0x1F80_1128 => self.timers.write16(p, value as u16),
            0x1F80_1040 => self.joy.write16(p, value as u16),
            0x1F80_1048 => {
                self.joy.write16(p, value as u16);
                self.joy.write16(p + 2, (value >> 16) as u16);
            }
            0x1F80_1810 => self.gpu.gp0(value),
            0x1F80_1814 => self.gpu.gp1(value),
            0x1F80_1C00..=0x1F80_1FFF => {
                self.spu.write16(p, value as u16);
                self.spu.write16(p + 2, (value >> 16) as u16);
            }
            0xFFFE_0130 => self.cache_ctrl = value,
            _ => {}
        }
    }
}

fn phys(addr: u32) -> u32 {
    match addr >> 29 {
        0 => addr,               // KUSEG
        4 => addr & 0x1FFF_FFFF, // KSEG0
        5 => addr & 0x1FFF_FFFF, // KSEG1
        _ => addr,               // KSEG2 / cache control
    }
}
