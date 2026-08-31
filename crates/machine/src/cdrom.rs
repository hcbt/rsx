use std::collections::VecDeque;

use crate::disc::Disc;
use crate::irq::{self, Irq};

/// Inspectable CD-ROM controller state. The Debugger prints this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdromView {
    pub status: u8,
    pub controller: u8,
    pub reading: bool,
    pub motor: bool,
    pub lba: u32,
    pub pending_cycles: Option<u32>,
    pub fifo_bytes: u32,
    pub mode: u8,
    pub last_cmd: u8,
    /// Oldest→newest, 16 slots; unused are 0xFF.
    pub recent: [u8; 16],
}

/// 33.8688 MHz / 75 sectors/s (1×). Mode bit 7 selects 2×.
const CYCLES_PER_SECTOR_1X: u32 = 451_584;
const SEEK_CYCLES: u32 = 451_584;

/// CD-ROM controller: status, GetID, and sector reads.
pub struct Cdrom {
    index: u8,
    status: u8,
    irq_enable: u8,
    irq_flag: u8,
    param: Vec<u8>,
    result: Vec<u8>,
    result_i: usize,
    pending: Option<Pending>,
    disc: Option<Disc>,
    motor: bool,
    loc: (u8, u8, u8),
    lba: u32,
    mode: u8,
    reading: bool,
    fifo: Vec<u8>,
    fifo_i: usize,
    last_cmd: u8,
    last_executed: u8,
    recent: [u8; 16],
    recent_n: u8,
    held_cmd: Option<u8>,
    held_param: Vec<u8>,
    queue: VecDeque<(u8, Vec<u8>)>,
    cmd_delay: Option<(u32, u8, Vec<u8>)>,
    setloc_pending: bool,
    seeking: bool,
    header: [u8; 8],
    header_valid: bool,
    last_lba: u32,
}

enum PendingWhat {
    Irq { irq: u8, result: Vec<u8> },
    SeekDone { then_read: bool },
}

struct Pending {
    cycles: u32,
    what: PendingWhat,
    second: Option<(u32, PendingWhat)>,
}

impl Cdrom {
    pub fn new() -> Self {
        Self {
            index: 0,
            status: 0x18, // param empty, param ready
            irq_enable: 0,
            irq_flag: 0,
            param: Vec::new(),
            result: Vec::new(),
            result_i: 0,
            pending: None,
            disc: None,
            motor: false,
            loc: (0, 0, 0),
            lba: 0,
            mode: 0,
            reading: false,
            fifo: Vec::new(),
            fifo_i: 0,
            last_cmd: 0,
            last_executed: 0,
            recent: [0xFF; 16],
            recent_n: 0,
            held_cmd: None,
            held_param: Vec::new(),
            queue: VecDeque::new(),
            cmd_delay: None,
            setloc_pending: false,
            seeking: false,
            header: [0; 8],
            header_valid: false,
            last_lba: 0,
        }
    }

    pub fn reset(&mut self) {
        let disc = self.disc.take();
        *self = Self::new();
        self.disc = disc;
    }

    pub fn insert(&mut self, disc: Disc) {
        self.disc = Some(disc);
    }

    pub fn tick(&mut self, cycles: u32, irq: &mut Irq) {
        if let Some((delay, _, _)) = self.cmd_delay.as_mut() {
            *delay = delay.saturating_sub(cycles);
        }
        if let Some(p) = self.pending.as_mut() {
            p.cycles = p.cycles.saturating_sub(cycles);
        }
        if self.cmd_delay.as_ref().is_some_and(|(d, _, _)| *d == 0) {
            let (_, irqn, result) = self.cmd_delay.take().unwrap();
            self.push_or_deliver(irqn, result, irq);
        }
        if self.pending.as_ref().is_some_and(|p| p.cycles == 0) {
            let p = self.pending.take().unwrap();
            self.finish_pending(p.what, irq);
            if let Some((delay, what)) = p.second {
                self.pending = Some(Pending {
                    cycles: delay,
                    what,
                    second: None,
                });
            }
        }
    }

    fn finish_pending(&mut self, what: PendingWhat, irq: &mut Irq) {
        match what {
            PendingWhat::Irq { irq: irqn, result } => self.push_or_deliver(irqn, result, irq),
            PendingWhat::SeekDone { then_read } => {
                self.seeking = false;
                self.capture_header();
                if then_read {
                    self.reading = true;
                    self.push_or_deliver(1, vec![self.controller_stat()], irq);
                } else {
                    self.push_or_deliver(2, vec![self.controller_stat()], irq);
                }
            }
        }
    }

    fn push_or_deliver(&mut self, irqn: u8, result: Vec<u8>, irq: &mut Irq) {
        if self.irq_flag & 7 != 0 {
            self.queue.push_back((irqn, result));
        } else {
            self.deliver(irqn, result, irq);
        }
    }

    fn deliver(&mut self, irqn: u8, result: Vec<u8>, irq: &mut Irq) {
        self.result = result;
        self.result_i = 0;
        self.irq_flag = (self.irq_flag & !7) | (irqn & 7);
        self.status |= 1 << 5; // result ready
        self.status &= !(1 << 7); // not busy
        if irqn == 1 {
            self.fill_fifo();
        }
        self.update_irq_line(irq);
    }

    fn pop_queue(&mut self, irq: &mut Irq) {
        if let Some((irqn, result)) = self.queue.pop_front() {
            self.deliver(irqn, result, irq);
        }
    }

    fn update_irq_line(&mut self, irq: &mut Irq) {
        irq.set_level(
            irq::IRQ_CDROM,
            self.irq_flag & 7 != 0 && self.irq_flag & self.irq_enable != 0,
        );
    }

    pub fn read8(&mut self, addr: u32) -> u8 {
        match (addr & 3, self.index & 3) {
            (0, _) => {
                self.status = (self.status & !3) | (self.index & 3);
                if self.param.is_empty() {
                    self.status |= 1 << 3;
                } else {
                    self.status &= !(1 << 3);
                }
                self.status |= 1 << 4;
                if self.result_i < self.result.len() {
                    self.status |= 1 << 5;
                }
                if self.fifo_i < self.fifo.len() {
                    self.status |= 1 << 6;
                } else {
                    self.status &= !(1 << 6);
                }
                self.status
            }
            (1, _) => {
                if self.result_i < self.result.len() {
                    let b = self.result[self.result_i];
                    self.result_i += 1;
                    if self.result_i >= self.result.len() {
                        self.status &= !(1 << 5);
                    }
                    b
                } else {
                    0
                }
            }
            (2, _) => self.pop_fifo(),
            (3, 0) | (3, 2) => self.irq_enable | 0xE0,
            (3, 1) | (3, 3) => self.irq_flag | 0xE0,
            _ => 0,
        }
    }

    pub fn write8(&mut self, addr: u32, value: u8, irq: &mut Irq) {
        match (addr & 3, self.index & 3) {
            (0, _) => self.index = value & 3,
            (1, 0) => self.command(value, irq),
            (2, 0) => {
                if self.param.len() < 16 {
                    self.param.push(value);
                }
            }
            (2, 1) => {
                self.irq_enable = value & 0x1F;
                self.update_irq_line(irq);
            }
            (3, 1) => {
                self.irq_flag &= !(value & 0x1F);
                if value & 0x40 != 0 {
                    self.param.clear();
                }
                if self.irq_flag & 7 == 0 {
                    self.result.clear();
                    self.result_i = 0;
                    self.update_irq_line(irq);
                    if let Some(cmd) = self.held_cmd.take() {
                        self.param = std::mem::take(&mut self.held_param);
                        self.execute(cmd, irq);
                    } else {
                        self.pop_queue(irq);
                    }
                } else {
                    self.update_irq_line(irq);
                }
            }
            _ => {}
        }
    }

    fn command(&mut self, cmd: u8, irq: &mut Irq) {
        self.last_cmd = cmd;
        let i = (self.recent_n as usize) % 16;
        self.recent[i] = cmd;
        self.recent_n = self.recent_n.saturating_add(1);
        if self.irq_flag & 7 != 0 {
            if cmd == 0x0A && self.irq_flag & 7 == 2 && self.last_executed == 0x0A {
                self.param.clear();
                return;
            }
            self.held_cmd = Some(cmd);
            self.held_param = std::mem::take(&mut self.param);
            return;
        }
        self.execute(cmd, irq);
    }

    fn execute(&mut self, cmd: u8, _irq: &mut Irq) {
        self.last_executed = cmd;
        self.status |= 1 << 7; // busy
        let keep_pending =
            self.pending.is_some() && matches!(cmd, 0x01 | 0x02 | 0x0C | 0x0E | 0x10 | 0x19);
        let (first, second) = match cmd {
            0x01 => (Some((0xC4E1, 3, vec![self.controller_stat()])), None),
            0x02 => {
                if self.param.len() >= 3 {
                    self.loc = (self.param[0], self.param[1], self.param[2]);
                }
                self.setloc_pending = true;
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x06 | 0x1B => self.read_n(),
            0x08 => {
                self.drop_int1();
                self.reading = false;
                self.seeking = false;
                self.motor = false;
                let stat = self.controller_stat();
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((
                        0x00D3_8ACA,
                        PendingWhat::Irq {
                            irq: 2,
                            result: vec![stat],
                        },
                    )),
                )
            }
            0x09 => {
                self.drop_int1();
                self.reading = false;
                self.seeking = false;
                let stat = self.controller_stat();
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((
                        0x0021_181C,
                        PendingWhat::Irq {
                            irq: 2,
                            result: vec![stat],
                        },
                    )),
                )
            }
            0x0A => {
                self.mode = 0x20;
                if self.disc.is_some() {
                    self.motor = true;
                }
                self.drop_int1();
                self.reading = false;
                self.seeking = false;
                let stat = self.controller_stat();
                (
                    Some((0x13CCE, 3, vec![stat])),
                    Some((
                        0xC4E1,
                        PendingWhat::Irq {
                            irq: 2,
                            result: vec![stat],
                        },
                    )),
                )
            }
            0x0C => (Some((0xC4E1, 3, vec![self.controller_stat()])), None),
            0x0E => {
                if let Some(&m) = self.param.first() {
                    self.mode = m;
                }
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x10 => self.getloc_l(),
            0x15 => self.seek_l(),
            0x19 => {
                let sub = self.param.first().copied().unwrap_or(0);
                if sub == 0x20 {
                    (Some((0xC4E1, 3, vec![0x94, 0x09, 0x19, 0xC0])), None)
                } else {
                    (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
                }
            }
            0x1A => self.get_id(),
            _ => (Some((0xC4E1, 5, vec![0x11, 0x40])), None),
        };
        self.param.clear();
        if let Some((cycles, irqn, result)) = first {
            if keep_pending {
                self.cmd_delay = Some((cycles, irqn, result));
            } else {
                self.pending = Some(Pending {
                    cycles,
                    what: PendingWhat::Irq { irq: irqn, result },
                    second,
                });
            }
        }
    }

    fn drop_int1(&mut self) {
        self.queue.retain(|(irqn, _)| *irqn != 1);
        if matches!(
            self.pending.as_ref().map(|p| &p.what),
            Some(PendingWhat::Irq { irq: 1, .. }) | Some(PendingWhat::SeekDone { then_read: true })
        ) {
            self.pending = None;
        } else if let Some(p) = self.pending.as_mut() {
            if matches!(
                &p.second,
                Some((_, PendingWhat::Irq { irq: 1, .. }))
                    | Some((_, PendingWhat::SeekDone { then_read: true }))
            ) {
                p.second = None;
            }
        }
    }

    fn get_id(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let stat = self.controller_stat();
        match self.disc.as_ref() {
            Some(disc) => {
                let mut id = vec![stat, 0x00, 0x20, 0x00];
                id.extend_from_slice(&disc.region);
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((0x4A00, PendingWhat::Irq { irq: 2, result: id })),
                )
            }
            None => (
                Some((0xC4E1, 3, vec![stat])),
                Some((
                    30_000,
                    PendingWhat::Irq {
                        irq: 5,
                        result: vec![0x08, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
                    },
                )),
            ),
        }
    }

    pub fn view(&self) -> CdromView {
        let mut recent = [0xFFu8; 16];
        let n = (self.recent_n as usize).min(16);
        let start = if self.recent_n as usize > 16 {
            (self.recent_n as usize) % 16
        } else {
            0
        };
        for i in 0..n {
            recent[i] = self.recent[(start + i) % 16];
        }
        CdromView {
            status: self.status,
            controller: self.controller_stat(),
            reading: self.reading,
            motor: self.motor,
            lba: self.lba,
            pending_cycles: self.pending.as_ref().map(|p| p.cycles),
            fifo_bytes: self.fifo.len().saturating_sub(self.fifo_i) as u32,
            mode: self.mode,
            last_cmd: self.last_cmd,
            recent,
        }
    }

    fn controller_stat(&self) -> u8 {
        let mut s = 0;
        if self.motor {
            s |= 0x02;
        }
        if self.seeking {
            s |= 0x40;
        } else if self.reading {
            s |= 0x20;
        }
        s
    }

    fn getloc_l(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        if self.seeking || !self.header_valid {
            (
                Some((0xC4E1, 5, vec![self.controller_stat() | 1, 0x80])),
                None,
            )
        } else {
            (Some((0xC4E1, 3, self.header.to_vec())), None)
        }
    }

    fn seek_l(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        self.setloc_pending = false;
        self.seeking = true;
        self.reading = false;
        self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
        let stat = self.controller_stat();
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((SEEK_CYCLES, PendingWhat::SeekDone { then_read: false })),
        )
    }

    fn read_n(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        if self.disc.is_none() {
            return (
                Some((0xC4E1, 3, vec![self.controller_stat()])),
                Some((
                    10_000,
                    PendingWhat::Irq {
                        irq: 5,
                        result: vec![0x01],
                    },
                )),
            );
        }
        self.motor = true;
        if self.setloc_pending {
            self.setloc_pending = false;
            self.seeking = true;
            self.reading = false;
            self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
            let stat = self.controller_stat();
            (
                Some((0xC4E1, 3, vec![stat])),
                Some((SEEK_CYCLES, PendingWhat::SeekDone { then_read: true })),
            )
        } else if self.reading {
            (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
        } else {
            self.reading = true;
            self.lba = self.last_lba;
            let stat = self.controller_stat();
            (
                Some((0xC4E1, 3, vec![stat])),
                Some((
                    self.sector_cycles(),
                    PendingWhat::Irq {
                        irq: 1,
                        result: vec![stat],
                    },
                )),
            )
        }
    }

    fn sector_cycles(&self) -> u32 {
        if self.mode & 0x80 != 0 {
            CYCLES_PER_SECTOR_1X / 2
        } else {
            CYCLES_PER_SECTOR_1X
        }
    }

    fn fill_fifo(&mut self) {
        let Some(disc) = self.disc.as_ref() else {
            self.reading = false;
            return;
        };
        if self.lba >= disc.sector_count() {
            self.reading = false;
            return;
        }
        let Some(raw) = disc.sector(self.lba) else {
            self.reading = false;
            return;
        };
        if raw.len() >= 20 {
            self.header.copy_from_slice(&raw[12..20]);
            self.header_valid = true;
        }
        let (start, len) = if self.mode & 0x20 != 0 {
            (12, 0x924)
        } else {
            (24, 0x800)
        };
        let end = (start + len).min(raw.len());
        self.fifo.clear();
        self.fifo.extend_from_slice(&raw[start..end]);
        self.fifo_i = 0;
        self.status |= 1 << 6;
        self.last_lba = self.lba;
        self.lba += 1;
    }

    fn capture_header(&mut self) {
        let Some(disc) = self.disc.as_ref() else {
            return;
        };
        let Some(raw) = disc.sector(self.lba) else {
            return;
        };
        if raw.len() >= 20 {
            self.header.copy_from_slice(&raw[12..20]);
            self.header_valid = true;
        }
        self.last_lba = self.lba;
    }

    fn pop_fifo(&mut self) -> u8 {
        if self.fifo_i < self.fifo.len() {
            let b = self.fifo[self.fifo_i];
            self.fifo_i += 1;
            if self.fifo_i >= self.fifo.len() {
                self.status &= !(1 << 6);
                if self.reading && self.pending.is_none() {
                    self.pending = Some(Pending {
                        cycles: self.sector_cycles(),
                        what: PendingWhat::Irq {
                            irq: 1,
                            result: vec![self.controller_stat()],
                        },
                        second: None,
                    });
                }
            }
            b
        } else {
            0
        }
    }

    pub fn dma_read32(&mut self) -> u32 {
        u32::from(self.pop_fifo())
            | (u32::from(self.pop_fifo()) << 8)
            | (u32::from(self.pop_fifo()) << 16)
            | (u32::from(self.pop_fifo()) << 24)
    }
}

fn bcd(v: u8) -> u32 {
    u32::from((v >> 4) * 10 + (v & 0x0F))
}

fn msf_to_lba(mm: u8, ss: u8, ff: u8) -> u32 {
    let m = bcd(mm);
    let s = bcd(ss);
    let f = bcd(ff);
    (m * 60 + s) * 75 + f - 150
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc::{load_disc, SECTOR_LEN};
    use std::io::Write;
    use std::path::Path;

    fn cue_with_america(dir: &Path) -> std::path::PathBuf {
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR_LEN * 4 + 24..SECTOR_LEN * 4 + 24 + lic.len()].copy_from_slice(lic);
        std::fs::write(dir.join("game.bin"), &bin).unwrap();
        let cue = dir.join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        cue
    }

    fn pump(cd: &mut Cdrom, irq: &mut Irq, cycles: u32) {
        cd.tick(cycles, irq);
    }

    #[test]
    fn getid_without_disc_is_int5() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        cd.irq_enable = 0x1F;
        cd.command(0x1A, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(cd.irq_flag & 7, 3, "INT3");
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        pump(&mut cd, &mut irq, 30_000);
        assert_eq!(cd.irq_flag & 7, 5, "INT5");
        assert_eq!(cd.result[0], 0x08);
    }

    #[test]
    fn getid_with_disc_is_int2_scea() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_with_america(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        cd.motor = true;
        let mut irq = Irq::new();
        cd.irq_enable = 0x1F;
        cd.command(0x1A, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(cd.irq_flag & 7, 3, "INT3");
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        pump(&mut cd, &mut irq, 50_000);
        assert_eq!(cd.irq_flag & 7, 2, "INT2");
        assert_eq!(&cd.result[4..8], b"SCEA");
        assert_eq!(cd.result[2], 0x20);
    }

    #[test]
    fn readn_after_setloc_supplies_user_data() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_with_america(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        cd.motor = true;
        let mut irq = Irq::new();
        cd.irq_enable = 0x1F;
        cd.param = vec![0x80];
        cd.command(0x0E, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        cd.param = vec![0x00, 0x02, 0x04];
        cd.command(0x02, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        cd.command(0x06, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(cd.irq_flag & 7, 1, "INT1");
        let mut bytes = Vec::new();
        for _ in 0..64 {
            bytes.push(cd.pop_fifo());
        }
        let s = String::from_utf8_lossy(&bytes);
        assert!(
            s.contains("Licensed"),
            "ReadN 00:02:04 must yield the license sector ({s:?})"
        );
    }

    #[test]
    fn readn_does_not_replace_fifo_until_drained() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_with_america(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        cd.motor = true;
        let mut irq = Irq::new();
        cd.irq_enable = 0x1F;
        cd.param = vec![0x80];
        cd.command(0x0E, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        cd.param = vec![0x00, 0x02, 0x04];
        cd.command(0x02, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        cd.command(0x06, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        cd.write8(3, 0x1F, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(cd.irq_flag & 7, 1);
        pump(&mut cd, &mut irq, 300_000);
        let mut first = Vec::new();
        for _ in 0..16 {
            first.push(cd.pop_fifo());
        }
        let s = String::from_utf8_lossy(&first);
        assert!(
            s.contains("Licens"),
            "unread sector must stay in the FIFO ({s:?})"
        );
    }

    fn enable(cd: &mut Cdrom, irq: &mut Irq) {
        cd.write8(0, 1, irq);
        cd.write8(2, 0x1F, irq);
    }

    fn send(cd: &mut Cdrom, irq: &mut Irq, cmd: u8, params: &[u8]) {
        cd.write8(0, 0, irq);
        for &p in params {
            cd.write8(2, p, irq);
        }
        cd.write8(1, cmd, irq);
    }

    fn ack_irq(cd: &mut Cdrom, irq: &mut Irq) {
        cd.write8(0, 1, irq);
        cd.write8(3, 0x1F, irq);
    }

    fn hintsts(cd: &mut Cdrom, irq: &mut Irq) -> u8 {
        cd.write8(0, 1, irq);
        cd.read8(3) & 7
    }

    fn result_bytes(cd: &mut Cdrom, n: usize) -> Vec<u8> {
        (0..n).map(|_| cd.read8(1)).collect()
    }

    fn load_licensed() -> (tempfile::TempDir, Cdrom) {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_with_america(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        (dir, cd)
    }

    fn set_2x_and_loc(cd: &mut Cdrom, irq: &mut Irq) {
        send(cd, irq, 0x0E, &[0x80]);
        pump(cd, irq, 0xC4E1);
        ack_irq(cd, irq);
        send(cd, irq, 0x02, &[0x00, 0x02, 0x04]);
        pump(cd, irq, 0xC4E1);
        ack_irq(cd, irq);
    }

    fn drain_data(cd: &mut Cdrom, irq: &mut Irq) {
        loop {
            cd.write8(0, 0, irq);
            if cd.read8(0) & (1 << 6) == 0 {
                break;
            }
            let _ = cd.read8(2);
        }
    }

    #[test]
    fn command_written_while_hintsts_sits_until_ack() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x01, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Getstat INT3");
        send(&mut cd, &mut irq, 0x00, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            3,
            "sitting command must not replace unacked INT3"
        );
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            5,
            "unused command is INT5 after ack"
        );
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x40]);
    }

    #[test]
    fn later_write_replaces_the_sitting_command() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x01, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        send(&mut cd, &mut irq, 0x01, &[]);
        send(&mut cd, &mut irq, 0x00, &[]);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5);
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x40]);
    }

    #[test]
    fn int1_does_not_replace_unacked_int3() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        set_2x_and_loc(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "ReadN INT3");
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            3,
            "INT1 must queue behind unacked INT3"
        );
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0);
        assert_eq!(hintsts(&mut cd, &mut irq), 1, "INT1 after INT3 ack");
    }

    #[test]
    fn pause_drops_queued_int1() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        set_2x_and_loc(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1, "first INT1");
        drain_data(&mut cd, &mut irq);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            1,
            "draining the data FIFO must not ack INT1"
        );
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            1,
            "second INT1 is queued behind the unacked first"
        );
        send(&mut cd, &mut irq, 0x09, &[]);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Pause INT3");
        ack_irq(&mut cd, &mut irq);
        assert_ne!(
            hintsts(&mut cd, &mut irq),
            1,
            "Pause must drop the queued INT1, not deliver it on INT3 ack"
        );
        pump(&mut cd, &mut irq, 0x0021_181C);
        let kind = hintsts(&mut cd, &mut irq);
        assert_ne!(kind, 1, "Pause must drop queued INT1 (got INT{kind})");
        assert_eq!(kind, 2, "Pause INT2");
    }

    #[test]
    fn setloc_is_unprocessed_until_seek_or_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR_LEN * 4 + 24..SECTOR_LEN * 4 + 24 + lic.len()].copy_from_slice(lic);
        bin[SECTOR_LEN * 8 + 24..SECTOR_LEN * 8 + 24 + 4].copy_from_slice(b"NEXT");
        std::fs::write(dir.path().join("game.bin"), &bin).unwrap();
        let cue = dir.path().join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        let disc = load_disc(&cue).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x80]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1);
        let mut first = Vec::new();
        for _ in 0..64 {
            first.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&first).contains("Licensed"),
            "ReadN after Setloc must seek then deliver that sector"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x09, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x0021_181C);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x08]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        let stat = cd.read8(1);
        assert_eq!(
            stat & 0x40,
            0x40,
            "ReadN with pending Setloc INT3 has Seek bit"
        );
        assert_eq!(
            stat & 0x20,
            0,
            "Read bit stays clear until the seek completes"
        );
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_ne!(
            hintsts(&mut cd, &mut irq),
            1,
            "no INT1 until the pending-Setloc seek completes"
        );
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1);
        let mut second = Vec::new();
        for _ in 0..8 {
            second.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&second).contains("NEXT"),
            "seek must land on the new Setloc ({second:?})"
        );
    }

    #[test]
    fn pause_then_readn_without_setloc_redelivers_last_sector() {
        let dir = tempfile::tempdir().unwrap();
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR_LEN * 4 + 24..SECTOR_LEN * 4 + 24 + lic.len()].copy_from_slice(lic);
        bin[SECTOR_LEN * 5 + 24..SECTOR_LEN * 5 + 24 + 6].copy_from_slice(b"SECOND");
        std::fs::write(dir.path().join("game.bin"), &bin).unwrap();
        let cue = dir.path().join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        let disc = load_disc(&cue).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        set_2x_and_loc(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1);
        ack_irq(&mut cd, &mut irq);
        loop {
            if cd.read8(0) & (1 << 6) == 0 {
                break;
            }
            let _ = cd.read8(2);
        }
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1, "second sector INT1");
        let mut last = Vec::new();
        for _ in 0..8 {
            last.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&last).contains("SECOND"),
            "need the sector after Setloc so re-deliver is not a Setloc seek ({last:?})"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x09, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x0021_181C);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        let stat = cd.read8(1);
        assert_eq!(
            stat & 0x20,
            0x20,
            "ReadN without pending Setloc keeps the Read bit"
        );
        assert_eq!(
            stat & 0x40,
            0,
            "ReadN without pending Setloc has no Seek bit"
        );
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1);
        let mut again = Vec::new();
        for _ in 0..8 {
            again.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&again).contains("SECOND"),
            "Pause then ReadN re-delivers the last sector, not Setloc ({again:?})"
        );
    }

    #[test]
    fn reads_is_the_same_int1_stream_as_readn() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        set_2x_and_loc(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x1B, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1, "ReadS INT1");
        let mut bytes = Vec::new();
        for _ in 0..64 {
            bytes.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&bytes).contains("Licensed"),
            "ReadS must yield the same data stream as ReadN"
        );
    }

    #[test]
    fn unused_command_is_int5_11h_40h() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x1F, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5);
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x40]);
    }

    #[test]
    fn init_sets_mode_20h_and_second_init_while_int2_pending_is_dropped() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0A, &[]);
        pump(&mut cd, &mut irq, 0x13CCE);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "Init INT2");
        assert_eq!(cd.view().mode, 0x20, "Init sets mode=20h");
        send(&mut cd, &mut irq, 0x0A, &[]);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x13CCE);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            0,
            "second Init while INT2 pending is dropped"
        );
    }

    #[test]
    fn getlocl_during_seekl_is_int5_80h_then_header_after_int2() {
        let dir = tempfile::tempdir().unwrap();
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let off = SECTOR_LEN * 4;
        bin[off + 12] = 0x00;
        bin[off + 13] = 0x02;
        bin[off + 14] = 0x04;
        bin[off + 15] = 0x02;
        bin[off + 16] = 0x01;
        bin[off + 17] = 0x02;
        bin[off + 18] = 0x64;
        bin[off + 19] = 0x03;
        std::fs::write(dir.path().join("game.bin"), &bin).unwrap();
        let cue = dir.path().join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        let disc = load_disc(&cue).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x15, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x10, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            5,
            "GetlocL during SeekL is INT5"
        );
        let body = result_bytes(&mut cd, 2);
        assert_eq!(body[1], 0x80, "GetlocL during SeekL is INT5(stat,80h)");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "SeekL INT2");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x10, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3);
        assert_eq!(
            result_bytes(&mut cd, 8),
            vec![0x00, 0x02, 0x04, 0x02, 0x01, 0x02, 0x64, 0x03],
            "GetlocL after SeekL INT2 is that sector header, not zeros"
        );
    }

    #[test]
    fn i_stat_cd_is_edge_on_hintsts_not_re_raised_every_tick() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x01, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_ne!(
            irq.read16(0x1F80_1070) & (1 << 2),
            0,
            "I_STAT.CD on HINTSTS"
        );
        irq.write16(0x1F80_1070, !(1 << 2));
        assert_eq!(irq.read16(0x1F80_1070) & (1 << 2), 0, "I_STAT.CD acked");
        pump(&mut cd, &mut irq, 50_000);
        assert_eq!(
            irq.read16(0x1F80_1070) & (1 << 2),
            0,
            "HINTSTS staying high must not re-raise I_STAT.CD"
        );
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "HINTSTS still INT3");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x01, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_ne!(
            irq.read16(0x1F80_1070) & (1 << 2),
            0,
            "new HINTSTS false→true sets I_STAT.CD"
        );
    }
}
