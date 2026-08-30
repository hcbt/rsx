//! Emulated SCPH-1001 PlayStation. Host-window-free: load BIOS, step, inspect.

mod bios;
mod bus;
mod cdrom;
mod cop0;
mod cpu;
mod disc;
mod dma;
mod gpu;
mod gte;
mod irq;
mod joy;
mod spu;
mod timers;

pub use bios::BiosError;
pub use disc::DiscError;

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

impl DisplayArea {
    /// 8-bit RGB, row-major, 3 bytes per pixel. The Debugger captures this.
    pub fn to_rgb888(&self) -> Vec<u8> {
        let mut rgb = Vec::with_capacity(self.pixels.len() * 3);
        for p in &self.pixels {
            rgb.push(((p & 0x1F) << 3) as u8);
            rgb.push((((p >> 5) & 0x1F) << 3) as u8);
            rgb.push((((p >> 10) & 0x1F) << 3) as u8);
        }
        rgb
    }
}

/// SCPH-1001 NTSC master crystal. Vblank (263×2160) and SPU 44100 (÷768) derive from this.
pub const CPU_HZ: u64 = 33_868_800;

/// NTSC cycles per scanline (SPX).
pub const CYCLES_PER_LINE: u64 = 2160;

/// NTSC scanlines per frame, including vblank (SPX).
pub const LINES_PER_FRAME: u32 = 263;

/// NTSC vblank rate from the crystal: [`CPU_HZ`] / ([`CYCLES_PER_LINE`] × [`LINES_PER_FRAME`]).
pub fn ntsc_vblank_hz() -> f64 {
    CPU_HZ as f64 / (CYCLES_PER_LINE as f64 * f64::from(LINES_PER_FRAME))
}

/// Guest cycles that elapse in `nanos` of wall time at [`CPU_HZ`].
pub fn cycles_in_nanos(nanos: u128) -> u64 {
    let n = nanos.saturating_mul(CPU_HZ as u128) / 1_000_000_000;
    u64::try_from(n).unwrap_or(u64::MAX)
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

    pub fn insert_disc(&mut self, path: impl AsRef<Path>) -> Result<(), DiscError> {
        let disc = crate::disc::load_disc(path.as_ref())?;
        self.bus.insert_disc(disc);
        Ok(())
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
            self.cpu.gte_on_vblank();
        }
    }

    pub fn run_until_vblank_count(&mut self, n: u64) {
        while self.bus.vblank_count() < n {
            self.step();
        }
        self.vblank_count = self.bus.vblank_count();
    }

    /// Run until the master crystal is at least `target`. Host realtime converts
    /// wall time through [`cycles_in_nanos`]; this is that deadline.
    pub fn run_until_cycle(&mut self, target: u64) {
        while self.bus.cycles() < target {
            self.step();
        }
        self.vblank_count = self.bus.vblank_count();
    }

    /// Interleaved stereo i16 at 44100 Hz, drained since the last take.
    pub fn take_audio(&mut self) -> Vec<i16> {
        self.bus.take_audio()
    }

    pub fn cycles(&self) -> u64 {
        self.bus.cycles()
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

    pub fn vram_rect(&self, x: u32, y: u32, w: u32, h: u32) -> DisplayArea {
        self.bus.gpu().vram_rect(x, y, w, h)
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

    pub fn lit_bbox(&self) -> Option<(u32, u32, u32, u32)> {
        self.bus.gpu().lit_bbox()
    }

    pub fn gp0_count(&self) -> u64 {
        self.bus.gpu().gp0_count
    }

    pub fn gp1_count(&self) -> u64 {
        self.bus.gpu().gp1_count
    }

    pub fn gp0_cmds(&self) -> &[u8] {
        &self.bus.gpu().gp0_cmds
    }

    pub fn gp0_words(&self) -> &[u32] {
        &self.bus.gpu().gp0_words
    }

    pub fn gp1_cmds(&self) -> &[u32] {
        &self.bus.gpu().gp1_cmds
    }

    pub fn display_origin(&self) -> (u32, u32, u32, u32, bool) {
        self.bus.gpu().display_origin()
    }

    pub fn draw_env(&self) -> (i32, i32, i32, i32, i32, i32) {
        self.bus.gpu().draw_env()
    }

    pub fn last_gouraud_tri_stats(&self) -> (u32, i32, i32, i32, i32, u32) {
        let g = self.bus.gpu();
        (
            g.last_n30,
            g.last_x0,
            g.last_x1,
            g.last_y0,
            g.last_y1,
            g.last_n30_out,
        )
    }

    pub fn gouraud_tri_stats(&self) -> (u32, i32, i32, i32, i32) {
        let g = self.bus.gpu();
        (g.frame_n30, g.frame_x0, g.frame_x1, g.frame_y0, g.frame_y1)
    }

    pub fn gte_control(&self, reg: u8) -> u32 {
        self.cpu.gte_control(reg)
    }

    pub fn gte_hi_sy_trace(&self) -> (i32, i32, u32, u32, i32, i32, u32) {
        self.cpu.gte_hi_sy_trace()
    }

    pub fn gte_title_ir2(&self) -> (i32, i32, i32, i32, i32, i32, i32, i32) {
        self.cpu.gte_title_ir2()
    }

    pub fn gte_op_counts(&self) -> [u32; 64] {
        self.cpu.gte_op_counts()
    }

    pub fn gte_title_rt(&self) -> [i32; 9] {
        self.cpu.gte_title_rt()
    }

    pub fn gte_frame_obj(
        &self,
    ) -> (
        u32,
        u32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        i32,
        u32,
    ) {
        self.cpu.gte_frame_obj()
    }

    pub fn last_y_bins(&self) -> [u32; 16] {
        self.bus.gpu().last_y_bins
    }

    pub fn last_hi_y_word(&self) -> u32 {
        self.bus.gpu().last_hi_y_word
    }

    pub fn last_long30(&self) -> u32 {
        self.bus.gpu().last_long30
    }

    pub fn last_max_dy(&self) -> i32 {
        self.bus.gpu().last_max_dy
    }

    pub fn last_poly_ops(&self) -> [u32; 32] {
        self.bus.gpu().last_poly_op
    }

    /// 512×240 occupancy of last-frame GP0(30) vertices, x wrapped per buffer.
    pub fn last_gouraud_scatter(&self) -> DisplayArea {
        let s = &self.bus.gpu().last_scatter;
        let mut pixels = Vec::with_capacity(512 * 240);
        for &n in s {
            let p = if n == 0 {
                0
            } else if n == 1 {
                0x7FFF // white
            } else {
                0x001F // red: stacked verts
            };
            pixels.push(p);
        }
        DisplayArea {
            width: 512,
            height: 240,
            pixels,
        }
    }

    pub fn dma_list_stats(&self) -> (u32, u32, u32, u32, u32, u32, u32) {
        let d = self.bus.dma();
        (
            d.last_list_empty,
            d.last_list_pkts,
            d.last_list_min,
            d.last_list_max,
            d.last_list_start,
            d.last_list_start_n,
            d.last_empty_before,
        )
    }

    pub fn io_writes(&self) -> u64 {
        self.bus.io_writes
    }

    pub fn last_io(&self) -> u32 {
        self.bus.last_io
    }

    pub fn io_cd(&self) -> u64 {
        self.bus.io_cd
    }

    pub fn io_spu(&self) -> u64 {
        self.bus.io_spu
    }

    pub fn io_irq(&self) -> u64 {
        self.bus.io_irq
    }

    pub fn io_gpu(&self) -> u64 {
        self.bus.io_gpu
    }

    pub fn bios_delay(&self) -> u32 {
        self.bus.memctrl_bios_delay()
    }

    pub fn ram_word(&self, addr: u32) -> u32 {
        self.bus.ram_word(addr)
    }

    pub fn last_exception(&self) -> Option<(u8, u32, u32)> {
        self.cpu.last_exception()
    }

    pub fn trans_y_write(&self) -> Option<(u32, u32, i32)> {
        self.cpu.trans_y_write()
    }

    pub fn trans_y_writes(&self) -> Vec<(u32, u32, u32, i32)> {
        self.cpu.trans_y_writes().to_vec()
    }

    pub fn sr(&self) -> u32 {
        self.cpu.sr()
    }

    pub fn badvaddr(&self) -> u32 {
        self.cpu.badvaddr()
    }

    pub fn exception_log(&self) -> &[(u8, u32, u32)] {
        &self.cpu.exception_log
    }

    pub fn irq_stat(&self) -> u16 {
        self.bus.irq().read16(0x1F80_1070)
    }

    pub fn irq_mask(&self) -> u16 {
        self.bus.irq().read16(0x1F80_1074)
    }

    pub fn timer_value(&self, i: usize) -> u16 {
        self.bus.timers().value(i)
    }

    pub fn timer_mode(&self, i: usize) -> u16 {
        self.bus.timers().mode(i)
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
    fn display_area_to_rgb888_expands_rgb555() {
        let area = DisplayArea {
            width: 1,
            height: 1,
            pixels: vec![0x001F],
        };
        assert_eq!(area.to_rgb888(), vec![0xF8, 0, 0]);
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
    fn addi_overflow_takes_the_ovf_exception() {
        // ADDI of 1 onto 0x7FFFFFFF must trap (EXC_OVF), not leave the dest stale.
        let bios = bios_with_program(&[
            0x3C08_7FFF, // lui t0, 0x7FFF
            0x3508_FFFF, // ori t0, t0, 0xFFFF
            0x2109_0001, // addi t1, t0, 1
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        m.step();
        m.step();
        m.step();
        let (code, pc, _) = m
            .last_exception()
            .expect("ADDI overflow must raise an exception");
        assert_eq!(code, cop0::EXC_OVF, "EXC_OVF");
        assert_eq!(pc, 0xBFC0_0008, "EPC is the ADDI");
        assert_eq!(m.gpr(9), 0, "overflow must not write the dest");
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
    fn bne_skips_fallthrough_after_delay_slot() {
        let bios = bios_with_program(&[
            0x2409_0001, // addiu t1, zero, 1
            0x1520_0002, // bne t1, zero, +2
            0x0000_0000, // nop
            0x240A_00FF, // addiu t2, zero, 0xFF (skipped)
            0x240A_0012, // addiu t2, zero, 0x12 (target)
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        for _ in 0..8 {
            m.step();
        }
        assert_eq!(m.gpr(10), 0x12, "BNE must jump over the fall-through");
    }

    #[test]
    fn disconnected_joy_clocks_ff_into_rx() {
        let bios = bios_with_program(&[
            0x3C08_1F80, // lui t0, 0x1F80
            0x3508_1040, // ori t0, t0, 0x1040
            0x2409_1003, // addiu t1, zero, 0x1003  TXEN | /JOYn
            0xA509_000A, // sh t1, 10(t0)           JOY_CTRL
            0x2409_0001, // addiu t1, zero, 1
            0xA109_0000, // sb t1, 0(t0)            JOY_TX
            0x0000_0000, // nop
            0x950A_0004, // lhu t2, 4(t0)           JOY_STAT
            0x0000_0000, // nop
            0x910B_0000, // lbu t3, 0(t0)           JOY_RX
            0x0000_0000, // nop
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        for _ in 0..16 {
            m.step();
        }
        assert_eq!(
            m.gpr(10) & (1 << 1),
            1 << 1,
            "JOY_STAT bit 1 (RX not empty)"
        );
        assert_eq!(m.gpr(11), 0xFF, "empty port must clock 0xFF into RX");
    }

    #[test]
    fn timer2_sysclk8_counts_across_short_ticks() {
        let bios = bios_with_program(&[
            0x3C08_1F80, // lui t0, 0x1F80
            0x3508_1124, // ori t0, t0, 0x1124  T2_MODE
            0x2409_0200, // addiu t1, zero, 0x200  clock = sysclk/8
            0xA509_0000, // sh t1, 0(t0)
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        for _ in 0..8 {
            m.step();
        }
        let before = m.timer_value(2);
        for _ in 0..40 {
            m.step();
        }
        let after = m.timer_value(2);
        assert!(
            after > before,
            "Timer 2 in sysclk/8 must count (before={before} after={after} mode={:04X})",
            m.timer_mode(2),
        );
    }

    #[test]
    fn dma_irq_does_not_fire_on_the_start_write() {
        let bios = bios_with_program(&[
            0x3C08_1F80, // lui t0, 0x1F80
            0x3508_10F0, // ori t0, t0, 0x10F0  DPCR
            0x8D09_0000, // lw t1, 0(t0)
            0x0000_0000, // nop
            0x3C0A_0800, // lui t2, 0x0800      ch6 enable
            0x012A_4825, // or t1, t1, t2
            0xAD09_0000, // sw t1, 0(t0)
            0x3C09_00C0, // lui t1, 0x00C0      DICR master + ch6 IRQ
            0xAD09_0004, // sw t1, 4(t0)
            0x3C08_1F80, // lui t0, 0x1F80
            0x3508_10E0, // ori t0, t0, 0x10E0  DMA6
            0x2409_0080, // addiu t1, zero, 0x80
            0xAD09_0000, // sw t1, 0(t0)        MADR
            0x2409_0004, // addiu t1, zero, 4
            0xAD09_0004, // sw t1, 4(t0)        BCR
            0x3C09_1100, // lui t1, 0x1100
            0x3529_0002, // ori t1, t1, 2
            0xAD09_0008, // sw t1, 8(t0)        CHCR start
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        for _ in 0..24 {
            m.step();
        }
        for _ in 0..200 {
            m.step();
        }
        assert_eq!(
            m.irq_stat() & (1 << 3),
            1 << 3,
            "IRQ3 after the transfer duration"
        );
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
    fn each_instruction_advances_the_master_clock() {
        let bios = bios_with_program(&[
            0x3C08_0013, // lui $t0, 0x0013
            0x3508_243F, // ori $t0, $t0, 0x243f
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        let c0 = m.cycles();
        m.step();
        let first = m.cycles() - c0;
        assert!(
            (27..=33).contains(&first),
            "uncached BIOS fetch is 27..33 (SPX), got {first}"
        );
        let c1 = m.cycles();
        m.step();
        let second = m.cycles() - c1;
        assert!(
            (27..=33).contains(&second),
            "second BIOS fetch is 27..33, got {second}"
        );
        assert_eq!(CPU_HZ % 768, 0, "SPU 44100 must divide the master crystal");
    }

    #[test]
    fn ram_load_stalls_while_otc_owns_ram() {
        let bios = bios_with_program(&[
            0x3C08_1F80, // lui t0, 0x1F80
            0x3508_10F0, // ori t0, t0, 0x10F0  DPCR
            0x8D09_0000, // lw t1, 0(t0)
            0x0000_0000, // nop
            0x3C0A_0800, // lui t2, 0x0800      ch6 enable
            0x012A_4825, // or t1, t1, t2
            0xAD09_0000, // sw t1, 0(t0)
            0x3C08_1F80, // lui t0, 0x1F80
            0x3508_10E0, // ori t0, t0, 0x10E0  DMA6
            0x2409_0080, // addiu t1, zero, 0x80
            0xAD09_0000, // sw t1, 0(t0)        MADR
            0x2409_0200, // addiu t1, zero, 0x200  512 words
            0xAD09_0004, // sw t1, 4(t0)        BCR
            0x3C09_1100, // lui t1, 0x1100
            0x3529_0002, // ori t1, t1, 2
            0xAD09_0008, // sw t1, 8(t0)        CHCR start
            0x8C0B_0100, // lw t3, 0x100(zero)  RAM — stalls until OTC releases the bus
            0x0000_0000, // nop
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        for _ in 0..16 {
            m.step();
        }
        let c0 = m.cycles();
        m.step();
        let dt = m.cycles() - c0;
        assert!(
            dt > 200,
            "lw from RAM must wait for OTC (512 words); got {dt}"
        );
    }

    #[test]
    fn ram_word_load_from_bios_adds_seven() {
        // lw t0, 0(zero)  — data from RAM, fetch from uncached BIOS.
        let bios = bios_with_program(&[0x8C08_0000]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        let c0 = m.cycles();
        m.step();
        let dt = m.cycles() - c0;
        assert!(
            (34..=40).contains(&dt),
            "BIOS fetch 27..33 plus RAM lw 7, got {dt}"
        );
    }

    #[test]
    fn one_wall_second_is_the_master_crystal() {
        assert_eq!(cycles_in_nanos(1_000_000_000), 33_868_800);
        assert_eq!(cycles_in_nanos(500_000_000), 16_934_400);
        assert_eq!(cycles_in_nanos(0), 0);
    }

    #[test]
    fn ntsc_vblank_is_derived_from_the_crystal() {
        let hz = ntsc_vblank_hz();
        assert!(
            (hz - 59.62).abs() < 0.01,
            "NTSC vblank is CPU_HZ/(2160×263) ≈ 59.62 Hz, got {hz}"
        );
        assert_eq!(CYCLES_PER_LINE * u64::from(LINES_PER_FRAME), 2160 * 263);
    }

    #[test]
    fn run_until_cycle_reaches_the_target() {
        let bios = bios_with_program(&[
            0x3C08_0013, // lui $t0, 0x0013
            0x0000_0000, // nop
        ]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        let c0 = m.cycles();
        m.run_until_cycle(c0 + 100);
        assert!(m.cycles() >= c0 + 100, "must not stop short of the target");
        assert!(
            m.cycles() < c0 + 100 + 64,
            "must not overshoot more than one instruction (got {})",
            m.cycles() - c0
        );
        let pc = m.pc();
        let cycles = m.cycles();
        m.run_until_cycle(cycles);
        assert_eq!(m.cycles(), cycles, "already-there is a no-op");
        assert_eq!(m.pc(), pc);
        m.run_until_cycle(0);
        assert_eq!(m.cycles(), cycles, "must not rewind the crystal");
    }

    #[test]
    fn run_until_cycle_mixes_one_spu_frame_per_768_clocks() {
        let bios = bios_with_program(&[0x0000_0000]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        let _ = m.take_audio();
        let c0 = m.cycles();
        m.run_until_cycle(c0 + 768 * 16);
        let pcm = m.take_audio();
        assert_eq!(
            pcm.len(),
            32,
            "16 SPU frames are 16 stereo pairs (got {} samples after {} cycles)",
            pcm.len(),
            m.cycles() - c0
        );
    }

    #[test]
    fn bios_leaves_reset_and_writes_memctrl_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        if !path.exists() {
            eprintln!("skipping: no local SCPH1001.BIN");
            return;
        }
        let mut m = Machine::from_bios_path(&path).unwrap();
        m.run_until_vblank_count(400);
        assert_ne!(m.pc(), 0xBFC0_0000, "CPU did not leave the reset vector");
        assert_eq!(
            m.bios_delay(),
            0x0013_243F,
            "BIOS did not write the BIOS delay register"
        );
        let display_lit = m
            .display_area()
            .pixels
            .iter()
            .filter(|p| **p & 0x7FFF != 0)
            .count();
        assert!(
            display_lit > 50_000,
            "Intro did not draw into the display area (pc={:08X} display_lit={display_lit} vram_lit={} gp0={} bbox={:?} exc={:?})",
            m.pc(),
            m.vram_lit(),
            m.gp0_count(),
            m.lit_bbox(),
            m.last_exception(),
        );
        assert!(
            m.exception_log().iter().all(|e| e.0 != 9),
            "Intro hit BREAK (exc={:?})",
            m.exception_log()
        );
    }

    #[test]
    fn intro_jingle_is_audible_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        if !path.exists() {
            eprintln!("skipping: no local SCPH1001.BIN");
            return;
        }
        let mut m = Machine::from_bios_path(&path).unwrap();
        m.run_until_vblank_count(250);
        let pcm = m.take_audio();
        let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        let energy: u64 = pcm.iter().map(|s| u64::from(s.unsigned_abs())).sum();
        assert!(
            peak > 512 && energy > 10_000,
            "Intro jingle must mix non-silent PCM (peak={peak} energy={energy} n={})",
            pcm.len()
        );
    }

    #[test]
    fn bios_leaves_joy_wait_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        if !path.exists() {
            eprintln!("skipping: no local SCPH1001.BIN");
            return;
        }
        let mut m = Machine::from_bios_path(&path).unwrap();
        m.run_until_vblank_count(700);
        assert_ne!(
            m.pc() & 0xFFFF,
            0x45D0,
            "BIOS stuck waiting for JOY RX (pc={:08X})",
            m.pc()
        );
        assert_eq!(
            m.irq_stat() & (1 << 3),
            0,
            "DMA IRQ stuck pending (pc={:08X} irq={:04X} gp0={})",
            m.pc(),
            m.irq_stat(),
            m.gp0_count(),
        );
        assert!(
            m.gp0_count() > 5_000,
            "BIOS stopped issuing GPU commands (pc={:08X} gp0={})",
            m.pc(),
            m.gp0_count(),
        );
    }

    #[test]
    fn intro_does_not_present_texpage_before_the_diamond_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        if !path.exists() {
            eprintln!("skipping: no local SCPH1001.BIN");
            return;
        }
        let mut m = Machine::from_bios_path(&path).unwrap();
        m.run_until_vblank_count(250);
        let tex = m.vram_rect(640, 0, 320, 240);
        let tex_lit = tex.pixels.iter().filter(|p| **p & 0x7FFF != 0).count();
        assert!(
            tex_lit > 1_000,
            "Intro textures should already be in VRAM (tex_lit={tex_lit})"
        );
        let (dx, dy, dw, dh, enabled) = m.display_origin();
        let display = m.display_area();
        let display_lit = display.pixels.iter().filter(|p| **p & 0x7FFF != 0).count();
        assert_ne!(
            (display.width, display.height),
            (320, 240),
            "display must not be the 320×240 texpage before the diamond (origin=({dx},{dy}) {dw}×{dh})"
        );
        assert!(
            !enabled,
            "GP1 has not enabled the display yet (origin=({dx},{dy}) {dw}×{dh})"
        );
        assert_eq!(
            display_lit, 0,
            "disabled display must be blank, not the uploaded texpage (display_lit={display_lit} tex_lit={tex_lit})"
        );

        m.run_until_vblank_count(350);
        let diamond = m.display_area();
        assert_eq!((diamond.width, diamond.height), (640, 480));
        let diamond_lit = diamond.pixels.iter().filter(|p| **p & 0x7FFF != 0).count();
        assert!(
            diamond_lit > 50_000,
            "diamond / fade must occupy the GP1 display rectangle (lit={diamond_lit})"
        );
        let w = diamond.width as usize;
        let lower = diamond.pixels[240 * w..]
            .iter()
            .filter(|p| **p & 0x7FFF != 0)
            .count();
        assert!(
            lower > 1_000,
            "480i must latch both fields; lower half must not stay black (lower={lower})"
        );

        m.run_until_vblank_count(550);
        let display = m.display_area();
        assert_eq!((display.width, display.height), (640, 480));
        let fill = display.pixels[0] & 0x7FFF;
        let w = display.width as usize;
        let mut banner = 0usize;
        // SCE text sprites sit at VRAM y=56..104, x=200..440. Display starts at y=2.
        for y in 54..102 {
            for x in 200..440 {
                if display.pixels[y * w + x] & 0x7FFF != fill {
                    banner += 1;
                }
            }
        }
        assert!(
            banner > 1_000,
            "SCE text sprites must be on the display during the diamond (banner={banner} fill={fill:04X})"
        );
    }

    fn write_america_cue(dir: &std::path::Path) -> std::path::PathBuf {
        const SECTOR: usize = 2352;
        let mut bin = vec![0u8; SECTOR * 24];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR * 4 + 24..SECTOR * 4 + 24 + lic.len()].copy_from_slice(lic);
        std::fs::write(dir.join("game.bin"), &bin).unwrap();
        let cue = dir.join("game.cue");
        std::fs::write(
            &cue,
            "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        cue
    }

    #[test]
    fn missing_disc_is_an_error() {
        let bios = bios_with_program(&[0x3C08_0013]);
        let mut m = Machine::from_bios_path(bios.path()).unwrap();
        let err = m.insert_disc("/no/such/game.cue").unwrap_err();
        assert!(matches!(err, DiscError::Io(_)));
    }

    #[test]
    fn licensed_disc_does_not_enter_the_shell_when_present() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        if !path.exists() {
            eprintln!("skipping: no local SCPH1001.BIN");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let cue = write_america_cue(dir.path());
        let mut m = Machine::from_bios_path(&path).unwrap();
        m.insert_disc(&cue).unwrap();
        m.run_until_vblank_count(800);
        assert_ne!(
            m.pc() & 0xFFFFF000,
            0x8003_D000,
            "licensed Disc must not drop into the Shell (pc={:08X})",
            m.pc()
        );
        let area = m.display_area();
        let black = area.pixels.iter().filter(|p| **p & 0x7FFF == 0).count();
        assert!(
            black > 50_000,
            "licensed logo is on black, not the Shell (black={black} pc={:08X} gp0={})",
            m.pc(),
            m.gp0_count()
        );
    }

    #[test]
    fn licensed_logo_holds_while_the_exe_loads_when_present() {
        let bios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        let disc = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms/Crash Bandicoot (USA)/Crash Bandicoot (USA).cue");
        if !bios.exists() || !disc.exists() {
            eprintln!("skipping: no local BIOS or Disc");
            return;
        }
        let mut m = Machine::from_bios_path(&bios).unwrap();
        m.insert_disc(&disc).unwrap();
        m.run_until_vblank_count(1000);
        let (dx, dy, dw, dh, _) = m.display_origin();
        assert_eq!(
            (dw, dh),
            (640, 480),
            "CD 2× must still be on the licensed logo at vblank 1000, not Crash 512×240 (pc={:08X} origin=({dx},{dy}) {dw}×{dh})",
            m.pc()
        );
        let black = m
            .display_area()
            .pixels
            .iter()
            .filter(|p| **p & 0x7FFF == 0)
            .count();
        assert!(
            black > 200_000,
            "licensed screen is still black (black={black} pc={:08X})",
            m.pc()
        );
    }

    #[test]
    fn crash_reaches_512x240_when_present() {
        let bios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        let disc = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms/Crash Bandicoot (USA)/Crash Bandicoot (USA).cue");
        if !bios.exists() || !disc.exists() {
            eprintln!("skipping: no local BIOS or Disc");
            return;
        }
        let mut m = Machine::from_bios_path(&bios).unwrap();
        m.insert_disc(&disc).unwrap();
        m.run_until_vblank_count(2000);
        let (dx, dy, dw, dh, _) = m.display_origin();
        assert_eq!(
            (dw, dh),
            (512, 240),
            "Crash must leave the licensed 640×480 by vblank 2000 (pc={:08X} origin=({dx},{dy}) {dw}×{dh})",
            m.pc()
        );
        let lit = m
            .display_area()
            .pixels
            .iter()
            .filter(|p| **p & 0x7FFF != 0)
            .count();
        assert!(
            lit > 1_000,
            "Crash 512×240 must not stay a blank buffer (lit={lit} pc={:08X})",
            m.pc()
        );
    }

    #[test]
    fn licensed_logo_draws_the_mark_when_present() {
        let bios = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../SCPH1001.BIN");
        let disc = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../roms/Crash Bandicoot (USA)/Crash Bandicoot (USA).cue");
        if !bios.exists() || !disc.exists() {
            eprintln!("skipping: no local BIOS or Disc");
            return;
        }
        let mut m = Machine::from_bios_path(&bios).unwrap();
        m.insert_disc(&disc).unwrap();
        m.run_until_vblank_count(800);
        let area = m.display_area();
        assert_eq!((area.width, area.height), (640, 480));
        let w = area.width as usize;
        let mut lit = 0usize;
        let mut color = 0usize;
        for y in 80..260 {
            for x in 120..520 {
                let p = area.pixels[y * w + x] & 0x7FFF;
                if p == 0 {
                    continue;
                }
                lit += 1;
                let r = p & 0x1F;
                let g = (p >> 5) & 0x1F;
                let b = (p >> 10) & 0x1F;
                if r != g || g != b {
                    color += 1;
                }
            }
        }
        assert!(
            lit > 8_000,
            "3D P/S mark must occupy the area above PlayStation (lit={lit} color={color} pc={:08X})",
            m.pc()
        );
        assert!(
            color > 2_000,
            "P/S mark is red and striped, not grey (lit={lit} color={color})"
        );
    }
}
