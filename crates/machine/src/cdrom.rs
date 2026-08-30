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
    recent: [u8; 16],
    recent_n: u8,
}

struct Pending {
    cycles: u32,
    irq: u8,
    result: Vec<u8>,
    second: Option<(u32, u8, Vec<u8>)>,
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
            recent: [0xFF; 16],
            recent_n: 0,
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
        if let Some(p) = self.pending.as_mut() {
            if p.cycles > cycles {
                p.cycles -= cycles;
                return;
            }
            let p = self.pending.take().unwrap();
            self.deliver(p.irq, p.result, irq);
            if let Some((delay, irqn, res)) = p.second {
                self.pending = Some(Pending {
                    cycles: delay,
                    irq: irqn,
                    result: res,
                    second: None,
                });
            }
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
        if self.irq_flag & self.irq_enable != 0 {
            irq.raise(irq::IRQ_CDROM);
        }
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
                if self.irq_flag & self.irq_enable != 0 {
                    irq.raise(irq::IRQ_CDROM);
                }
            }
            (3, 1) => {
                self.irq_flag &= !(value & 0x1F);
                if value & 0x40 != 0 {
                    self.param.clear();
                }
            }
            _ => {}
        }
    }

    fn command(&mut self, cmd: u8, _irq: &mut Irq) {
        self.last_cmd = cmd;
        let i = (self.recent_n as usize) % 16;
        self.recent[i] = cmd;
        self.recent_n = self.recent_n.saturating_add(1);
        self.status |= 1 << 7; // busy
        let (first, second) = match cmd {
            0x01 => (Some((0xC4E1, 3, vec![self.controller_stat()])), None), // Getstat
            0x02 => {
                // Setloc
                if self.param.len() >= 3 {
                    self.loc = (self.param[0], self.param[1], self.param[2]);
                }
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x06 => self.read_n(),
            0x08 => {
                self.reading = false;
                let stat = self.controller_stat();
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((0x00D3_8ACA, 2, vec![stat])),
                )
            }
            0x09 => {
                self.reading = false;
                let stat = self.controller_stat();
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((0x0021_181C, 2, vec![stat])),
                )
            }
            0x0A => {
                if self.disc.is_some() {
                    self.motor = true;
                }
                self.reading = false;
                let stat = self.controller_stat();
                (
                    Some((0x13CCE, 3, vec![stat])),
                    Some((0xC4E1, 2, vec![stat])),
                )
            }
            0x0C => (Some((0xC4E1, 3, vec![self.controller_stat()])), None), // Demute
            0x0E => {
                if let Some(&m) = self.param.first() {
                    self.mode = m;
                }
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x15 => {
                // SeekL
                self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
                let stat = self.controller_stat();
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((451_584, 2, vec![stat])),
                )
            }
            0x19 => {
                let sub = self.param.first().copied().unwrap_or(0);
                if sub == 0x20 {
                    (Some((0xC4E1, 3, vec![0x94, 0x09, 0x19, 0xC0])), None)
                } else {
                    (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
                }
            }
            0x1A => self.get_id(),
            _ => (Some((0xC4E1, 3, vec![self.controller_stat()])), None),
        };
        self.param.clear();
        if let Some((cycles, irqn, result)) = first {
            self.pending = Some(Pending {
                cycles,
                irq: irqn,
                result,
                second,
            });
        }
    }

    fn get_id(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, u8, Vec<u8>)>) {
        let stat = self.controller_stat();
        match self.disc.as_ref() {
            Some(disc) => {
                let mut id = vec![stat, 0x00, 0x20, 0x00];
                id.extend_from_slice(&disc.region);
                (Some((0xC4E1, 3, vec![stat])), Some((0x4A00, 2, id)))
            }
            None => (
                Some((0xC4E1, 3, vec![stat])),
                Some((
                    30_000,
                    5,
                    vec![0x08, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
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
        if self.reading {
            s |= 0x20;
        }
        s
    }

    fn read_n(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, u8, Vec<u8>)>) {
        if self.disc.is_none() {
            return (
                Some((0xC4E1, 3, vec![self.controller_stat()])),
                Some((10_000, 5, vec![0x01])),
            );
        }
        self.motor = true;
        self.reading = true;
        self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
        let stat = self.controller_stat();
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((self.sector_cycles(), 1, vec![stat])),
        )
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
        self.lba += 1;
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
                        irq: 1,
                        result: vec![self.controller_stat()],
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
        cd.param = vec![0x00, 0x02, 0x04];
        cd.command(0x02, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.command(0x06, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        pump(&mut cd, &mut irq, 230_000);
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
        cd.param = vec![0x00, 0x02, 0x04];
        cd.command(0x02, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.command(0x06, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        pump(&mut cd, &mut irq, 230_000);
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
}
