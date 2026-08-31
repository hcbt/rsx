//! No window: run the Machine and print one measurement line per period.

use std::io::{self, Write};
use std::time::Instant;

use rsx_machine::Machine;

use crate::clock::{self, HostPace};

/// One stdout sample of guest state and host pace.
#[derive(Clone, Debug)]
pub struct Sample {
    pub vblank: u64,
    pub cycles: u64,
    pub pc: u32,
    pub gpustat: u32,
    pub gpu_fifo: usize,
    pub gpu_draw: u32,
    pub gpu_busy: bool,
    pub gp0: u64,
    pub dma_jobs: u8,
    pub dma_chcr: [u32; 7],
    pub cd_reading: bool,
    pub cd_motor: bool,
    pub cd_lba: u32,
    pub cd_pending: Option<u32>,
    pub cd_fifo: u32,
    pub cd_mode: u8,
    pub cd_last: u8,
    pub cd_recent: [u8; 16],
    pub irq_stat: u16,
    pub irq_mask: u16,
    pub wq: usize,
    pub ram_blocked: bool,
    pub display_x: u32,
    pub display_y: u32,
    pub display_w: u32,
    pub display_h: u32,
    pub display_on: bool,
    pub hash: u64,
    pub io_cd: u64,
    pub io_gpu: u64,
    pub t2_value: u16,
    pub t2_mode: u16,
    pub t2_target: u16,
    pub pace: Option<HostPace>,
}

impl Sample {
    pub fn from_machine(machine: &Machine, pace: Option<HostPace>) -> Self {
        let (dx, dy, dw, dh, on) = machine.display_origin();
        let cd = machine.cd_view();
        Self {
            vblank: machine.vblank_count(),
            cycles: machine.cycles(),
            pc: machine.pc(),
            gpustat: machine.gpustat(),
            gpu_fifo: machine.gpu_fifo_len(),
            gpu_draw: machine.gpu_draw_remaining(),
            gpu_busy: machine.gpu_busy(),
            gp0: machine.gp0_count(),
            dma_jobs: machine.dma_job_mask(),
            dma_chcr: [
                machine.dma_chcr(0),
                machine.dma_chcr(1),
                machine.dma_chcr(2),
                machine.dma_chcr(3),
                machine.dma_chcr(4),
                machine.dma_chcr(5),
                machine.dma_chcr(6),
            ],
            cd_reading: cd.reading,
            cd_motor: cd.motor,
            cd_lba: cd.lba,
            cd_pending: cd.pending_cycles,
            cd_fifo: cd.fifo_bytes,
            cd_mode: cd.mode,
            cd_last: cd.last_cmd,
            cd_recent: cd.recent,
            irq_stat: machine.irq_stat(),
            irq_mask: machine.irq_mask(),
            wq: machine.write_queue_len(),
            ram_blocked: machine.ram_blocked(),
            display_x: dx,
            display_y: dy,
            display_w: dw,
            display_h: dh,
            display_on: on,
            hash: machine.display_area_hash(),
            io_cd: machine.io_cd(),
            io_gpu: machine.io_gpu(),
            t2_value: machine.timer_value(2),
            t2_mode: machine.timer_mode(2),
            t2_target: machine.timer_target(2),
            pace,
        }
    }

    pub fn line(&self) -> String {
        let pace = match self.pace {
            Some(p) => p.line(),
            None => "pace —".to_string(),
        };
        let recent: Vec<String> = self
            .cd_recent
            .iter()
            .filter(|&&c| c != 0xFF)
            .map(|c| format!("{c:02X}"))
            .collect();
        let cd = if self.cd_reading {
            format!(
                "read lba={} pend={} fifo={} mode={:02X} last={:02X} [{}]",
                self.cd_lba,
                self.cd_pending.unwrap_or(0),
                self.cd_fifo,
                self.cd_mode,
                self.cd_last,
                recent.join(",")
            )
        } else if self.cd_motor {
            format!(
                "motor lba={} pend={} mode={:02X} last={:02X} [{}]",
                self.cd_lba,
                self.cd_pending.unwrap_or(0),
                self.cd_mode,
                self.cd_last,
                recent.join(",")
            )
        } else {
            format!("idle last={:02X} [{}]", self.cd_last, recent.join(","))
        };
        let mut dma = format!("dma={:02X}", self.dma_jobs);
        for (i, &chcr) in self.dma_chcr.iter().enumerate() {
            if self.dma_jobs & (1 << i) != 0 || chcr & (1 << 24) != 0 {
                dma.push_str(&format!(" chcr{i}={chcr:08X}"));
            }
        }
        format!(
            "vblank={} cycles={} pc={:08X} gpustat={:08X} {pace} gpu fifo={} draw={} busy={} gp0={} {dma} cd={cd} irq={:04X}/{:04X} wq={} ram={} display=({},{}) {}x{} on={} hash={:016x} io_cd={} io_gpu={} t2={:04X}/{:04X}/{:04X}",
            self.vblank,
            self.cycles,
            self.pc,
            self.gpustat,
            self.gpu_fifo,
            self.gpu_draw,
            u8::from(self.gpu_busy),
            self.gp0,
            self.irq_stat,
            self.irq_mask,
            self.wq,
            u8::from(self.ram_blocked),
            self.display_x,
            self.display_y,
            self.display_w,
            self.display_h,
            u8::from(self.display_on),
            self.hash,
            self.io_cd,
            self.io_gpu,
            self.t2_value,
            self.t2_mode,
            self.t2_target,
        )
    }
}

/// Unpaced: the host runs as fast as it can. Pace on the line is that throughput
/// versus the crystal, not a wall-locked Debugger.
pub fn run(
    machine: &mut Machine,
    until_vblank: Option<u64>,
    period: u64,
    out: &mut impl Write,
) -> io::Result<()> {
    let period = period.max(1);
    let wall0 = Instant::now();
    let cycles0 = machine.cycles();
    let vblank0 = machine.vblank_count();
    write_sample(machine, wall0, cycles0, vblank0, out)?;
    loop {
        let now = machine.vblank_count();
        if until_vblank.map(|n| now >= n).unwrap_or(false) {
            break;
        }
        let mut target = now.saturating_add(period);
        if let Some(n) = until_vblank {
            target = target.min(n);
        }
        if target <= now {
            break;
        }
        machine.run_until_vblank_count(target);
        let _ = machine.take_audio();
        write_sample(machine, wall0, cycles0, vblank0, out)?;
    }
    Ok(())
}

fn write_sample(
    machine: &Machine,
    wall0: Instant,
    cycles0: u64,
    vblank0: u64,
    out: &mut impl Write,
) -> io::Result<()> {
    let pace = clock::measure(
        machine.cycles().saturating_sub(cycles0),
        machine.vblank_count().saturating_sub(vblank0),
        wall0.elapsed(),
    );
    writeln!(out, "{}", Sample::from_machine(machine, pace).line())?;
    let log = machine.exception_log();
    if !log.is_empty() {
        write!(out, "  exc")?;
        for (code, pc, ra) in log {
            write!(out, " {code:02X}@{pc:08X} ra={ra:08X}")?;
        }
        writeln!(
            out,
            " last={:?} badvaddr={:08X} sr={:08X}",
            machine.last_exception(),
            machine.badvaddr(),
            machine.sr()
        )?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsx_machine::CPU_HZ;
    use std::time::Duration;

    fn idle_sample() -> Sample {
        Sample {
            vblank: 60,
            cycles: CPU_HZ,
            pc: 0x8001_0000,
            gpustat: 0x1C00_0000,
            gpu_fifo: 0,
            gpu_draw: 0,
            gpu_busy: false,
            gp0: 12,
            dma_jobs: 0,
            dma_chcr: [0; 7],
            cd_reading: true,
            cd_motor: true,
            cd_lba: 150,
            cd_pending: Some(451_584),
            cd_fifo: 0,
            cd_mode: 0x80,
            cd_last: 0x06,
            cd_recent: {
                let mut r = [0xFFu8; 16];
                r[0] = 0x0E;
                r[1] = 0x02;
                r[2] = 0x06;
                r
            },
            irq_stat: 0x0004,
            irq_mask: 0x0007,
            wq: 0,
            ram_blocked: false,
            display_x: 0,
            display_y: 2,
            display_w: 640,
            display_h: 480,
            display_on: true,
            hash: 0xBA52_1443_3916_99A1,
            io_cd: 40,
            io_gpu: 9,
            t2_value: 0,
            t2_mode: 0,
            t2_target: 0,
            pace: clock::measure(CPU_HZ, 60, Duration::from_secs(1)),
        }
    }

    #[test]
    fn sample_line_names_clock_guest_and_cd() {
        let line = idle_sample().line();
        assert!(line.contains("vblank=60"), "{line}");
        assert!(line.contains("pc=80010000"), "{line}");
        assert!(line.contains("clock="), "{line}");
        assert!(line.contains("fps="), "{line}");
        assert!(line.contains("cd=read lba=150"), "{line}");
        assert!(line.contains("last=06"), "{line}");
        assert!(line.contains("display=(0,2) 640x480 on=1"), "{line}");
        assert!(line.contains("gpu fifo=0 draw=0 busy=0"), "{line}");
        assert!(line.contains("dma=00"), "{line}");
    }

    #[test]
    fn sample_line_prints_busy_dma_chcr() {
        let mut s = idle_sample();
        s.dma_jobs = 1 << 2;
        s.dma_chcr[2] = 0x0100_0401;
        let line = s.line();
        assert!(line.contains("dma=04"), "{line}");
        assert!(line.contains("chcr2=01000401"), "{line}");
    }

    #[test]
    fn run_prints_a_sample_each_period_until_vblank() {
        let mut data = vec![0u8; 512 * 1024];
        data[0..4].copy_from_slice(&0x3C08_0013u32.to_le_bytes());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bios.bin");
        std::fs::write(&path, data).unwrap();
        let mut machine = Machine::from_bios_path(&path).unwrap();
        let mut out = Vec::new();
        run(&mut machine, Some(1), 1, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.len() >= 2, "expected start + vblank 1, got {lines:?}");
        assert!(lines[0].contains("vblank=0"), "{}", lines[0]);
        assert!(lines.last().unwrap().contains("vblank=1"), "{text}");
        assert!(machine.vblank_count() >= 1);
    }
}
