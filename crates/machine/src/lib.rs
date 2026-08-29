//! Emulated SCPH-1001 PlayStation. Host-window-free: load BIOS, step, inspect.

mod bios;
mod bus;
mod cdrom;
mod cop0;
mod cpu;
mod dma;
mod gpu;
mod gte;
mod irq;
mod spu;
mod timers;

pub use bios::BiosError;

use std::path::Path;

use crate::bios::load_bios;
use crate::bus::Bus;
use crate::cpu::Cpu;

/// Display-area snapshot (RGB555). The Debugger presents this; the Machine does not know about wgpu.
#[derive(Clone, Debug)]
pub struct DisplayArea {
    pub width: u32,
    pub height: u32,
    /// RGB555 pixels, row-major.
    pub pixels: Vec<u16>,
}

pub struct Machine {
    cpu: Cpu,
    bus: Bus,
    vblank_count: u64,
}

impl Machine {
    pub fn from_bios_path(path: impl AsRef<Path>) -> Result<Self, BiosError> {
        let bios = load_bios(path.as_ref())?;
        let mut machine = Self {
            cpu: Cpu::new(),
            bus: Bus::new(bios),
            vblank_count: 0,
        };
        machine.reset();
        Ok(machine)
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.bus.reset();
        self.vblank_count = 0;
    }

    pub fn step(&mut self) {
        let prev_vblanks = self.bus.vblank_count();
        self.cpu.step(&mut self.bus);
        let now = self.bus.vblank_count();
        if now > prev_vblanks {
            self.vblank_count = now;
        }
    }

    pub fn run_until_vblank_count(&mut self, n: u64) {
        while self.bus.vblank_count() < n {
            self.step();
        }
        self.vblank_count = self.bus.vblank_count();
    }

    pub fn pc(&self) -> u32 {
        self.cpu.pc()
    }

    pub fn gpr(&self, index: u8) -> u32 {
        self.cpu.gpr(index)
    }

    pub fn gpustat(&self) -> u32 {
        self.bus.gpu().stat()
    }

    pub fn display_area(&self) -> DisplayArea {
        self.bus.gpu().display_area()
    }

    pub fn display_area_hash(&self) -> u64 {
        hash_pixels(&self.display_area())
    }

    pub fn vblank_count(&self) -> u64 {
        self.bus.vblank_count()
    }

    pub fn vram_lit(&self) -> usize {
        self.bus.gpu().lit_texels()
    }

    pub fn gp0_count(&self) -> u64 {
        self.bus.gpu().gp0_count
    }

    pub fn gp1_count(&self) -> u64 {
        self.bus.gpu().gp1_count
    }

    pub fn io_writes(&self) -> u64 {
        self.bus.io_writes
    }

    pub fn bios_delay(&self) -> u32 {
        self.bus.memctrl_bios_delay()
    }
}

fn hash_pixels(area: &DisplayArea) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in &area.pixels {
        h ^= u64::from(*p);
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= u64::from(area.width);
    h = h.wrapping_mul(0x100000001b3);
    h ^= u64::from(area.height);
    h.wrapping_mul(0x100000001b3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const BIOS_LEN: usize = 512 * 1024;

    fn bios_with_program(words: &[u32]) -> tempfile::NamedTempFile {
        let mut data = vec![0u8; BIOS_LEN];
        for (i, w) in words.iter().enumerate() {
            let off = i * 4;
            data[off..off + 4].copy_from_slice(&w.to_le_bytes());
        }
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&data).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn missing_bios_is_an_error() {
        let err = match Machine::from_bios_path("/no/such/SCPH1001.BIN") {
            Err(e) => e,
            Ok(_) => panic!("expected missing BIOS to fail"),
        };
        assert!(matches!(err, BiosError::Io(_)));
    }

    #[test]
    fn bios_must_be_512_kib() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&[0u8; 16]).unwrap();
        let err = match Machine::from_bios_path(f.path()) {
            Err(e) => e,
            Ok(_) => panic!("expected wrong-size BIOS to fail"),
        };
        assert!(matches!(err, BiosError::WrongSize { .. }));
    }

    #[test]
    fn reset_fetches_from_bios_reset_vector() {
        // lui $t0, 0x0013
        let bios = bios_with_program(&[0x3C08_0013]);
        let m = Machine::from_bios_path(bios.path()).unwrap();
        assert_eq!(m.pc(), 0xBFC0_0000);
    }

    #[test]
    fn lui_loads_upper_immediate() {
        let bios = bios_with_program(&[
            0x3C08_0013, // lui $t0, 0x0013
            0x0000_0000, // nop
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        m.step();
        assert_eq!(m.gpr(8), 0x0013_0000);
        assert_eq!(m.pc(), 0xBFC0_0004);
    }

    #[test]
    fn ori_combines_with_lui() {
        let bios = bios_with_program(&[
            0x3C08_0013, // lui $t0, 0x0013
            0x3508_243F, // ori $t0, $t0, 0x243f
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        m.step();
        m.step();
        assert_eq!(m.gpr(8), 0x0013_243F);
    }

    #[test]
    fn bios_leaves_reset_and_writes_memctrl_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        if !path.exists() {
            eprintln!("skipping: no local SCPH1001.BIN");
            return;
        }
        let mut m = Machine::from_bios_path(&path).unwrap();
        for n in 1..=60 {
            m.run_until_vblank_count(n);
            if n == 1 || n == 10 || n == 30 || n == 60 {
                let lit = m
                    .display_area()
                    .pixels
                    .iter()
                    .filter(|p| **p & 0x7FFF != 0)
                    .count();
                eprintln!(
                    "vblank {n}: pc={:08X} gpustat={:08X} display_lit={lit} vram_lit={} gp0={} gp1={}",
                    m.pc(),
                    m.gpustat(),
                    m.vram_lit(),
                    m.gp0_count(),
                    m.gp1_count(),
                );
                eprintln!("  io_writes={} bios_delay={:08X}", m.io_writes(), m.bios_delay());
            }
        }
        assert_ne!(m.pc(), 0xBFC0_0000, "CPU did not leave the reset vector");
        assert_eq!(
            m.bios_delay(),
            0x0013_243F,
            "BIOS did not write the BIOS delay register"
        );
    }
}
