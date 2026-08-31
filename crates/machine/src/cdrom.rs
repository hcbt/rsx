use std::collections::VecDeque;

use crate::disc::Disc;
use crate::irq::{self, Irq};

/// One Setloc/ReadN/Pause/Seek as seen by the controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdCmdEvent {
    pub cmd: u8,
    pub loc_lba: u32,
    pub lba: u32,
    pub setloc_pending: bool,
    pub reading: bool,
    pub held: bool,
}

/// Inspectable CD-ROM controller state. The Debugger prints this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdromView {
    pub status: u8,
    pub controller: u8,
    pub reading: bool,
    pub motor: bool,
    pub lba: u32,
    pub loc_lba: u32,
    pub last_lba: u32,
    pub setloc_pending: bool,
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
/// SPX: a handful of sectors; stall rather than drop the locked front.
const SECTOR_BUF: usize = 8;

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
    data_sector: Vec<u8>,
    sector_buf: VecDeque<(u32, Vec<u8>)>,
    pad_byte: u8,
    fifo_loaded: bool,
    filter_file: u8,
    filter_channel: u8,
    want_data: bool,
    playing: bool,
    muted: bool,
    smen: bool,
    shell_open: bool,
    scex_unlocked: bool,
    secret_step: u8,
    vol: [u8; 4],
    vol_applied: [u8; 4],
    sound_map: Vec<u8>,
    sound_map_coding: u8,
    analog: (i16, i16),
    session: u8,
    scex_total: u8,
    scex_ok: u8,
    read_s: bool,
    retries: u8,
    skip: i32,
    play_ready: bool,
    play_sectors: u32,
    session_fail: u8,
    cmd_events: Vec<CdCmdEvent>,
}

enum PendingWhat {
    Irq { irq: u8, result: Vec<u8> },
    SeekDone { then_read: bool },
    PlayTick,
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
            data_sector: Vec::new(),
            sector_buf: VecDeque::new(),
            pad_byte: 0,
            fifo_loaded: false,
            filter_file: 0,
            filter_channel: 0,
            want_data: false,
            playing: false,
            muted: false,
            smen: false,
            shell_open: false,
            scex_unlocked: false,
            secret_step: 0,
            vol: [0x80, 0, 0x80, 0],
            vol_applied: [0x80, 0, 0x80, 0],
            sound_map: Vec::new(),
            sound_map_coding: 0,
            analog: (0, 0),
            session: 1,
            scex_total: 0,
            scex_ok: 0,
            read_s: false,
            retries: 0,
            skip: 0,
            play_ready: false,
            play_sectors: 0,
            session_fail: 0,
            cmd_events: Vec::new(),
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

    pub fn tick(&mut self, mut cycles: u32, irq: &mut Irq) {
        while cycles > 0 {
            let next = self
                .cmd_delay
                .as_ref()
                .map(|(d, _, _)| *d)
                .into_iter()
                .chain(self.pending.as_ref().map(|p| p.cycles))
                .min();
            let Some(next) = next else {
                return;
            };
            if next == 0 {
                if self.cmd_delay.as_ref().is_some_and(|(d, _, _)| *d == 0) {
                    let (_, irqn, result) = self.cmd_delay.take().unwrap();
                    self.push_or_deliver(irqn, result, irq);
                }
                if self.pending.as_ref().is_some_and(|p| p.cycles == 0) {
                    let p = self.pending.take().unwrap();
                    self.finish_pending(p.what, irq);
                    if self.pending.is_none() {
                        if let Some((delay, what)) = p.second {
                            self.pending = Some(Pending {
                                cycles: delay,
                                what,
                                second: None,
                            });
                        }
                    }
                }
                continue;
            }
            if next > cycles {
                if let Some((delay, _, _)) = self.cmd_delay.as_mut() {
                    *delay -= cycles;
                }
                if let Some(p) = self.pending.as_mut() {
                    p.cycles -= cycles;
                }
                return;
            }
            cycles -= next;
            if let Some((delay, _, _)) = self.cmd_delay.as_mut() {
                *delay -= next;
            }
            if let Some(p) = self.pending.as_mut() {
                p.cycles -= next;
            }
            if self.cmd_delay.as_ref().is_some_and(|(d, _, _)| *d == 0) {
                let (_, irqn, result) = self.cmd_delay.take().unwrap();
                self.push_or_deliver(irqn, result, irq);
            }
            if self.pending.as_ref().is_some_and(|p| p.cycles == 0) {
                let p = self.pending.take().unwrap();
                self.finish_pending(p.what, irq);
                if self.pending.is_none() {
                    if let Some((delay, what)) = p.second {
                        self.pending = Some(Pending {
                            cycles: delay,
                            what,
                            second: None,
                        });
                    }
                }
            }
        }
    }

    fn finish_pending(&mut self, what: PendingWhat, irq: &mut Irq) {
        match what {
            PendingWhat::Irq { irq: irqn, result } => {
                if irqn == 1 && self.reading && !self.playing {
                    self.try_read_sector(irq);
                } else {
                    self.push_or_deliver(irqn, result, irq);
                }
            }
            PendingWhat::SeekDone { then_read } => {
                self.seeking = false;
                self.capture_header();
                if then_read {
                    self.reading = true;
                    self.try_read_sector(irq);
                } else {
                    self.push_or_deliver(2, vec![self.controller_stat()], irq);
                }
            }
            PendingWhat::PlayTick => self.play_tick(irq),
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
        if self.smen && irqn == 3 {
            self.irq_flag |= 0x10;
            self.smen = false;
        }
        if irqn == 5 && self.session_fail > 0 {
            self.session_fail -= 1;
            if self.session_fail > 0 {
                self.queue.push_back((5, vec![0x06, 0x40]));
            }
        }
        self.status |= 1 << 5; // result ready
        self.status &= !(1 << 7); // not busy
        if irqn == 1 && !self.playing {
            if let Some((sec_lba, data)) = self.sector_buf.pop_front() {
                self.last_lba = sec_lba;
                self.data_sector = data;
                self.sync_pad_byte();
            } else {
                self.capture_data_sector();
            }
            if self.want_data {
                self.load_data_fifo();
            }
            self.arm_next_int1();
        }
        self.update_irq_line(irq);
    }

    fn fifo_busy(&self) -> bool {
        self.fifo_loaded && self.fifo_i < self.fifo.len()
    }

    fn arm_next_int1(&mut self) {
        if self.reading && !self.playing && self.pending.is_none() {
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

    fn clear_sector_buf(&mut self) {
        self.sector_buf.clear();
        self.fifo.clear();
        self.fifo_i = 0;
        self.fifo_loaded = false;
        self.status &= !(1 << 6);
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
            (1, 1) => {
                if self.sound_map.len() < 0x900 {
                    self.sound_map.push(value);
                    if self.sound_map.len() == 0x900 {
                        let (l, r) = decode_xa(&self.sound_map, self.sound_map_coding);
                        self.analog = self.apply_vol(l, r);
                    }
                }
            }
            (1, 2) => self.sound_map_coding = value,
            (2, 0) => {
                if self.param.len() < 16 {
                    self.param.push(value);
                }
            }
            (2, 2) => self.vol[0] = value,
            (3, 2) => self.vol[1] = value,
            (1, 3) => self.vol[2] = value,
            (2, 3) => self.vol[3] = value,
            (2, 1) => {
                self.irq_enable = value & 0x1F;
                self.update_irq_line(irq);
            }
            (3, 0) => {
                if value & 0x20 != 0 {
                    self.smen = true;
                }
                self.want_data = value & 0x80 != 0;
                if self.want_data {
                    self.load_data_fifo();
                } else {
                    self.fifo.clear();
                    self.fifo_i = 0;
                    self.fifo_loaded = false;
                    self.status &= !(1 << 6);
                }
            }
            (3, 3) => {
                if value & 0x20 != 0 {
                    self.vol_applied = self.vol;
                }
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

    fn loc_lba_now(&self) -> u32 {
        msf_to_lba(self.loc.0, self.loc.1, self.loc.2)
    }

    fn log_cmd(&mut self, cmd: u8, held: bool) {
        if !matches!(cmd, 0x02 | 0x06 | 0x09 | 0x15 | 0x16 | 0x1B) {
            return;
        }
        if self.cmd_events.len() >= 64 {
            self.cmd_events.remove(0);
        }
        self.cmd_events.push(CdCmdEvent {
            cmd,
            loc_lba: self.loc_lba_now(),
            lba: self.lba,
            setloc_pending: self.setloc_pending,
            reading: self.reading,
            held,
        });
    }

    pub fn cmd_events(&self) -> &[CdCmdEvent] {
        &self.cmd_events
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
            // SPX: Setloc is unprocessed until Seek/Read/Play, but the loc
            // registers latch when the command is written. A later ReadN that
            // replaces the sitting command still seeks to that loc.
            if cmd == 0x02 && self.param.len() >= 3 {
                self.loc = (self.param[0], self.param[1], self.param[2]);
                self.setloc_pending = true;
            }
            self.log_cmd(cmd, true);
            self.held_cmd = Some(cmd);
            self.held_param = std::mem::take(&mut self.param);
            return;
        }
        self.execute(cmd, irq);
        self.log_cmd(cmd, false);
    }

    fn execute(&mut self, cmd: u8, _irq: &mut Irq) {
        self.last_executed = cmd;
        self.status |= 1 << 7; // busy
        let keep_pending = self.pending.is_some()
            && matches!(
                cmd,
                0x01 | 0x02 | 0x0B | 0x0C | 0x0D | 0x0E | 0x0F | 0x10 | 0x11 | 0x13 | 0x14 | 0x19
            );
        if cmd == 0x01 && !self.param.is_empty() {
            self.param.clear();
            self.pending = Some(Pending {
                cycles: 0xC4E1,
                what: PendingWhat::Irq {
                    irq: 5,
                    result: vec![self.controller_stat() | 1, 0x20],
                },
                second: None,
            });
            return;
        }
        let (first, second) = match cmd {
            0x01 => (Some((0xC4E1, 3, vec![self.controller_stat()])), None),
            0x02 => {
                if self.param.len() >= 3 {
                    self.loc = (self.param[0], self.param[1], self.param[2]);
                }
                self.setloc_pending = true;
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x03 => self.play(),
            0x04 => self.forward_back(true),
            0x05 => self.forward_back(false),
            0x06 => self.read_n(false),
            0x1B => self.read_n(true),
            0x07 => {
                if self.motor {
                    (
                        Some((0xC4E1, 5, vec![self.controller_stat() | 1, 0x20])),
                        None,
                    )
                } else {
                    self.motor = true;
                    let stat = self.controller_stat();
                    (
                        Some((0xC4E1, 3, vec![stat])),
                        Some((
                            0xC4E1,
                            PendingWhat::Irq {
                                irq: 2,
                                result: vec![stat],
                            },
                        )),
                    )
                }
            }
            0x08 => {
                self.drop_int1();
                self.reading = false;
                self.seeking = false;
                self.playing = false;
                self.clear_sector_buf();
                self.skip = 0;
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
                self.playing = false;
                self.clear_sector_buf();
                self.skip = 0;
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
            0x0B => {
                self.muted = true;
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x0C => {
                self.muted = false;
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x0D => {
                if self.param.len() >= 2 {
                    self.filter_file = self.param[0];
                    self.filter_channel = self.param[1];
                }
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x0E => {
                if let Some(&m) = self.param.first() {
                    self.mode = m;
                }
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x0F => (
                Some((
                    0xC4E1,
                    3,
                    vec![
                        self.controller_stat(),
                        self.mode,
                        0,
                        self.filter_file,
                        self.filter_channel,
                    ],
                )),
                None,
            ),
            0x10 => self.getloc_l(),
            0x11 => self.getloc_p(),
            0x12 => self.set_session(),
            0x13 => self.get_tn(),
            0x14 => self.get_td(),
            0x15 => self.seek_l(),
            0x16 => self.seek_p(),
            0x19 => self.test_cmd(),
            0x1A => self.get_id(),
            0x1C => self.reset_cmd(),
            0x1D => self.get_q(),
            0x1E => self.read_toc(),
            0x1F => {
                // SCPH-1001: unsupported Video CD; SPX leaves the parameter FIFO.
                (Some((0xC4E1, 5, vec![0x11, 0x40])), None)
            }
            0x50..=0x57 => self.secret(cmd),
            _ => (Some((0xC4E1, 5, vec![0x11, 0x40])), None),
        };
        if !matches!(cmd, 0x1F) {
            self.param.clear();
        }
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
            Some(PendingWhat::Irq { irq: 1, .. })
                | Some(PendingWhat::SeekDone { then_read: true })
                | Some(PendingWhat::PlayTick)
        ) {
            self.pending = None;
        } else if let Some(p) = self.pending.as_mut() {
            if matches!(
                &p.second,
                Some((_, PendingWhat::Irq { irq: 1, .. }))
                    | Some((_, PendingWhat::SeekDone { then_read: true }))
                    | Some((_, PendingWhat::PlayTick))
            ) {
                p.second = None;
            }
        }
    }

    fn get_id(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        if self.shell_open {
            return (Some((0xC4E1, 5, vec![0x11, 0x80])), None);
        }
        let stat = self.controller_stat();
        match self.disc.as_ref() {
            Some(disc) if disc.licensed || self.scex_unlocked => {
                let mut id = vec![stat, 0x00, 0x20, 0x00];
                id.extend_from_slice(&disc.region);
                (
                    Some((0xC4E1, 3, vec![stat])),
                    Some((0x4A00, PendingWhat::Irq { irq: 2, result: id })),
                )
            }
            Some(_) => (
                Some((0xC4E1, 3, vec![stat])),
                Some((
                    0x4A00,
                    PendingWhat::Irq {
                        irq: 5,
                        result: vec![0x0A, 0x80, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00],
                    },
                )),
            ),
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
            loc_lba: self.loc_lba_now(),
            last_lba: self.last_lba,
            setloc_pending: self.setloc_pending,
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
        if self.shell_open {
            s |= 0x10;
        }
        if self.playing && !self.seeking {
            s |= 0x80;
        } else if self.seeking {
            s |= 0x40;
        } else if self.reading {
            s |= 0x20;
        }
        s
    }

    pub fn open_shell(&mut self, irq: &mut Irq) {
        self.shell_open = true;
        self.playing = false;
        self.reading = false;
        self.push_or_deliver(5, vec![self.controller_stat() | 1, 0x80], irq);
    }

    pub fn close_shell(&mut self) {
        self.shell_open = false;
    }

    pub fn take_analog(&mut self) -> (i16, i16) {
        let s = self.analog;
        if !self.playing && self.sound_map.len() < 0x900 {
            self.analog = (0, 0);
        }
        s
    }

    fn getloc_p(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let lba = if self.seeking {
            msf_to_lba(self.loc.0, self.loc.1, self.loc.2)
        } else {
            self.last_lba
        };
        let abs = frames_to_msf(lba + 150);
        let rel = frames_to_msf(lba);
        (
            Some((
                0xC4E1,
                3,
                vec![0x01, 0x01, rel.0, rel.1, rel.2, abs.0, abs.1, abs.2],
            )),
            None,
        )
    }

    fn get_tn(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let (first, last) = self
            .disc
            .as_ref()
            .and_then(|d| d.tracks.first().zip(d.tracks.last()))
            .map(|(a, b)| (to_bcd(u32::from(a.number)), to_bcd(u32::from(b.number))))
            .unwrap_or((0x01, 0x01));
        (
            Some((0xC4E1, 3, vec![self.controller_stat(), first, last])),
            None,
        )
    }

    fn get_td(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let want = bcd(self.param.first().copied().unwrap_or(0));
        let Some(disc) = self.disc.as_ref() else {
            return (
                Some((0xC4E1, 5, vec![self.controller_stat() | 1, 0x80])),
                None,
            );
        };
        if want == 0 {
            let abs = frames_to_msf(disc.sector_count() + 150);
            return (
                Some((0xC4E1, 3, vec![self.controller_stat(), abs.0, abs.1])),
                None,
            );
        }
        match disc.tracks.iter().find(|t| u32::from(t.number) == want) {
            Some(t) => {
                let abs = frames_to_msf(t.start_lba + 150);
                (
                    Some((0xC4E1, 3, vec![self.controller_stat(), abs.0, abs.1])),
                    None,
                )
            }
            None => (
                Some((0xC4E1, 5, vec![self.controller_stat() | 1, 0x10])),
                None,
            ),
        }
    }

    fn play(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        self.motor = true;
        self.playing = true;
        self.reading = false;
        if self.setloc_pending {
            self.setloc_pending = false;
            self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
            self.last_lba = self.lba;
        }
        if let Some(&tr) = self.param.first() {
            if let Some(t) = self
                .disc
                .as_ref()
                .and_then(|d| d.tracks.iter().find(|x| x.number == bcd(tr) as u8))
            {
                self.lba = t.start_lba;
                self.last_lba = self.lba;
            }
        }
        self.play_ready = false;
        self.skip = 0;
        self.play_sectors = 0;
        self.feed_cdda();
        let stat = self.controller_stat();
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((self.sector_cycles(), PendingWhat::PlayTick)),
        )
    }

    fn forward_back(
        &mut self,
        fwd: bool,
    ) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        if !self.playing || !self.play_ready {
            return (
                Some((0xC4E1, 5, vec![self.controller_stat() | 1, 0x80])),
                None,
            );
        }
        let step = 75i32;
        self.skip = if fwd {
            self.skip.max(0) + step
        } else {
            self.skip.min(0) - step
        };
        self.lba = self.lba.saturating_add_signed(self.skip);
        self.last_lba = self.lba;
        self.feed_cdda();
        (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
    }

    fn report_bytes(&self) -> Vec<u8> {
        let abs = frames_to_msf(self.last_lba + 150);
        vec![
            self.controller_stat(),
            0x01,
            0x01,
            abs.0,
            abs.1,
            abs.2,
            0,
            0,
        ]
    }

    fn set_session(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let s = self.param.first().copied().unwrap_or(0);
        if s == 0 {
            return (Some((0xC4E1, 5, vec![0x03, 0x10])), None);
        }
        if s > 1 {
            self.session_fail = 2;
            let stat = self.controller_stat();
            return (
                Some((0xC4E1, 3, vec![stat])),
                Some((
                    SEEK_CYCLES,
                    PendingWhat::Irq {
                        irq: 5,
                        result: vec![0x06, 0x40],
                    },
                )),
            );
        }
        self.session = s;
        let stat = self.controller_stat();
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((
                SEEK_CYCLES,
                PendingWhat::Irq {
                    irq: 2,
                    result: vec![stat],
                },
            )),
        )
    }

    fn seek_p(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        self.setloc_pending = false;
        self.seeking = true;
        self.reading = false;
        self.playing = false;
        self.clear_sector_buf();
        self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
        let stat = self.controller_stat();
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((SEEK_CYCLES, PendingWhat::SeekDone { then_read: false })),
        )
    }

    fn reset_cmd(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        self.mode = 0x20;
        self.reading = false;
        self.playing = false;
        self.seeking = false;
        self.scex_unlocked = false;
        let stat = self.controller_stat();
        (Some((0xC4E1, 3, vec![stat])), None)
    }

    fn get_q(&self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let stat = self.controller_stat();
        let point = self.param.get(1).copied().unwrap_or(0x01);
        let start = self
            .disc
            .as_ref()
            .and_then(|d| {
                d.tracks
                    .iter()
                    .find(|t| to_bcd(u32::from(t.number)) == point)
            })
            .map(|t| t.start_lba)
            .unwrap_or(0);
        let abs = frames_to_msf(start + 150);
        let sub = vec![
            0x41, 0x00, point, 0x00, 0x00, 0x00, 0x00, abs.0, abs.1, abs.2, 0,
        ];
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((
                SEEK_CYCLES,
                PendingWhat::Irq {
                    irq: 2,
                    result: sub,
                },
            )),
        )
    }

    fn read_toc(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let stat = self.controller_stat();
        (
            Some((0x13CCE, 3, vec![stat])),
            Some((
                0x20_0000,
                PendingWhat::Irq {
                    irq: 2,
                    result: vec![stat],
                },
            )),
        )
    }

    fn test_cmd(&mut self) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        match self.param.first().copied().unwrap_or(0) {
            0x20 => (Some((0xC4E1, 3, vec![0x94, 0x09, 0x19, 0xC0])), None),
            0x21 => {
                let mut f = 0u8;
                if self.shell_open {
                    f |= 2;
                }
                (Some((0xC4E1, 3, vec![f])), None)
            }
            0x22 => (Some((0xC4E1, 3, b"for U/C".to_vec())), None),
            0x23 => (
                Some((0xC4E1, 3, b"CXD2940Q/CXD1817Q/CXD2545Q/CXD1782BR".to_vec())),
                None,
            ),
            0x24 => (
                Some((0xC4E1, 3, b"CXD2940Q/CXD1817Q/CXD2545Q/CXD2510Q".to_vec())),
                None,
            ),
            0x25 => (
                Some((0xC4E1, 3, b"CXD2940Q/CXD1817Q/CXD1815Q/CXD1199BQ".to_vec())),
                None,
            ),
            0x04 => {
                self.motor = true;
                self.scex_total = 0;
                self.scex_ok = 0;
                if self.disc.as_ref().is_some_and(|d| d.licensed) {
                    self.scex_total = 1;
                    self.scex_ok = 1;
                }
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x05 => (Some((0xC4E1, 3, vec![self.scex_total, self.scex_ok])), None),
            0x00 => {
                self.motor = true;
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            0x03 => {
                self.motor = false;
                (Some((0xC4E1, 3, vec![self.controller_stat()])), None)
            }
            _ => (Some((0xC4E1, 5, vec![0x11, 0x10])), None),
        }
    }

    fn secret(&mut self, cmd: u8) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        let expect: &[&[u8]] = &[
            b"",
            b"Licensed by",
            b"Sony",
            b"Computer",
            b"Entertainment",
            b"of America",
            b"",
        ];
        let idx = (cmd - 0x50) as usize;
        if cmd == 0x57 {
            self.scex_unlocked = false;
            self.secret_step = 0;
        } else if idx < expect.len()
            && (expect[idx].is_empty() || self.param.starts_with(expect[idx]))
            && self.secret_step == idx as u8
        {
            self.secret_step = idx as u8 + 1;
            if cmd == 0x56 {
                self.scex_unlocked = true;
            }
        } else {
            self.scex_unlocked = false;
            self.secret_step = 0;
        }
        (Some((0xC4E1, 5, vec![0x11, 0x40])), None)
    }

    fn feed_cdda(&mut self) {
        if self.muted || !self.audio_here() {
            self.analog = (0, 0);
            return;
        }
        let Some(raw) = self.disc.as_ref().and_then(|d| d.sector(self.lba)) else {
            self.analog = (0, 0);
            return;
        };
        let l = i16::from_le_bytes([raw[0], raw[1]]);
        let r = i16::from_le_bytes([raw[2], raw[3]]);
        self.analog = self.apply_vol(l, r);
    }

    fn xa_to_spu(&self) -> bool {
        if self.mode & 0x40 == 0 {
            return false;
        }
        let Some(raw) = self.disc.as_ref().and_then(|d| d.sector(self.lba)) else {
            return false;
        };
        if raw.len() < 24 || raw[15] != 2 {
            return false;
        }
        let sm = raw[18];
        if sm & 0x44 != 0x44 {
            return false;
        }
        if self.mode & 8 != 0 && (raw[16] != self.filter_file || raw[17] != self.filter_channel) {
            return false;
        }
        true
    }

    fn feed_xa(&mut self) {
        if self.muted {
            self.analog = (0, 0);
            return;
        }
        let Some(raw) = self.disc.as_ref().and_then(|d| d.sector(self.lba)) else {
            self.analog = (0, 0);
            return;
        };
        if raw.len() < 24 + 0x80 {
            self.analog = (0, 0);
            return;
        }
        let coding = raw.get(19).copied().unwrap_or(0);
        let (l, r) = decode_xa(&raw[24..], coding);
        self.analog = self.apply_vol(l, r);
    }

    fn apply_vol(&self, l: i16, r: i16) -> (i16, i16) {
        // SPX: ATV0 L→L, ATV1 L→R, ATV2 R→R (1F801801h.Index3), ATV3 R→L (1F801802h.Index3).
        let [ll, lr, rr, rl] = self.vol_applied.map(|v| i32::from(v));
        let l = i32::from(l);
        let r = i32::from(r);
        let ol = ((l * ll + r * rl) >> 7).clamp(-0x8000, 0x7FFF) as i16;
        let or = ((l * lr + r * rr) >> 7).clamp(-0x8000, 0x7FFF) as i16;
        (ol, or)
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
        self.clear_sector_buf();
        self.lba = msf_to_lba(self.loc.0, self.loc.1, self.loc.2);
        let stat = self.controller_stat();
        (
            Some((0xC4E1, 3, vec![stat])),
            Some((SEEK_CYCLES, PendingWhat::SeekDone { then_read: false })),
        )
    }

    fn read_n(&mut self, read_s: bool) -> (Option<(u32, u8, Vec<u8>)>, Option<(u32, PendingWhat)>) {
        self.read_s = read_s;
        self.retries = if read_s { 0 } else { 3 };
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
            self.clear_sector_buf();
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
            self.clear_sector_buf();
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

    fn sector_bad(&self) -> bool {
        self.disc
            .as_ref()
            .and_then(|d| d.sector(self.lba))
            .is_some_and(|raw| raw.len() > 15 && raw[15] == 0xFF)
    }

    fn audio_here(&self) -> bool {
        self.disc.as_ref().is_some_and(|d| {
            d.tracks
                .iter()
                .rev()
                .find(|t| t.start_lba <= self.lba)
                .is_some_and(|t| t.audio)
        })
    }

    fn try_read_sector(&mut self, irq: &mut Irq) {
        if self.xa_to_spu() {
            self.feed_xa();
            self.lba = self.lba.saturating_add(1);
            if self.reading {
                self.pending = Some(Pending {
                    cycles: self.sector_cycles(),
                    what: PendingWhat::Irq {
                        irq: 1,
                        result: vec![self.controller_stat()],
                    },
                    second: None,
                });
            }
            return;
        }
        if self.sector_bad() {
            if self.read_s || self.retries == 0 {
                self.reading = false;
                self.push_or_deliver(5, vec![self.controller_stat() | 1, 0x40], irq);
            } else {
                self.retries -= 1;
                self.pending = Some(Pending {
                    cycles: self.sector_cycles(),
                    what: PendingWhat::Irq {
                        irq: 1,
                        result: vec![self.controller_stat()],
                    },
                    second: None,
                });
            }
            return;
        }
        // Locked FIFO: buffer at 1×/2×, no extra INT1. INT3 still queues.
        if self.fifo_busy() && self.irq_flag & 7 == 0 && self.sector_buf.len() < SECTOR_BUF {
            self.extra_capture();
            self.arm_next_int1();
            return;
        }
        if self.fifo_busy() || self.irq_flag & 7 == 1 {
            self.arm_next_int1();
            return;
        }
        self.push_or_deliver(1, vec![self.controller_stat()], irq);
    }

    fn play_tick(&mut self, irq: &mut Irq) {
        self.play_ready = true;
        let step = if self.skip != 0 { self.skip } else { 1 };
        let next = self.lba as i64 + i64::from(step);
        let count = self
            .disc
            .as_ref()
            .map(|d| i64::from(d.sector_count()))
            .unwrap_or(0);
        if next >= count {
            self.playing = false;
            self.motor = false;
            self.skip = 0;
            self.push_or_deliver(4, vec![self.controller_stat()], irq);
            return;
        }
        if next < 0 {
            self.lba = 0;
            self.skip = 0;
        } else {
            self.lba = next as u32;
        }
        self.last_lba = self.lba;
        self.play_sectors = self.play_sectors.saturating_add(1);
        self.feed_cdda();
        if self.mode & 4 != 0 && self.play_sectors % 10 == 0 {
            self.push_or_deliver(1, self.report_bytes(), irq);
        }
        self.pending = Some(Pending {
            cycles: self.sector_cycles(),
            what: PendingWhat::PlayTick,
            second: None,
        });
    }

    fn take_disc_sector(&mut self) -> Option<(u32, Vec<u8>)> {
        let disc = self.disc.as_ref()?;
        if self.lba >= disc.sector_count() {
            self.reading = false;
            return None;
        }
        let raw = disc.sector(self.lba)?;
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
        let data = raw[start..end].to_vec();
        let sec_lba = self.lba;
        self.lba += 1;
        Some((sec_lba, data))
    }

    fn sync_pad_byte(&mut self) {
        let pad_i = if self.mode & 0x20 != 0 {
            0x924 - 4
        } else {
            0x800 - 8
        };
        self.pad_byte = self.data_sector.get(pad_i).copied().unwrap_or(0);
    }

    fn capture_data_sector(&mut self) {
        let Some((sec_lba, data)) = self.take_disc_sector() else {
            self.data_sector.clear();
            return;
        };
        self.last_lba = sec_lba;
        self.data_sector = data;
        self.sync_pad_byte();
    }

    fn extra_capture(&mut self) {
        if let Some(pair) = self.take_disc_sector() {
            self.sector_buf.push_back(pair);
        }
    }

    fn load_data_fifo(&mut self) {
        self.fifo.clear();
        self.fifo.extend_from_slice(&self.data_sector);
        self.fifo_i = 0;
        self.fifo_loaded = !self.fifo.is_empty();
        if self.fifo_loaded {
            self.status |= 1 << 6;
        } else {
            self.status &= !(1 << 6);
        }
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
                if self.reading && !self.playing {
                    if self.sector_buf.is_empty() {
                        // Empty ring: next INT1 is a full sector from drain.
                        self.pending = Some(Pending {
                            cycles: self.sector_cycles(),
                            what: PendingWhat::Irq {
                                irq: 1,
                                result: vec![self.controller_stat()],
                            },
                            second: None,
                        });
                    } else if self.pending.is_none() {
                        self.arm_next_int1();
                    }
                }
            }
            b
        } else if self.fifo_loaded {
            self.pad_byte
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

    /// SPX DRQSTS: data FIFO has a sector (BFRD loaded), including pad bytes
    /// after the 800h/924h payload. Empty when Want Data is clear.
    pub fn drq(&self) -> bool {
        self.fifo_loaded
    }

    #[cfg(test)]
    pub fn test_fill_fifo(&mut self, bytes: &[u8]) {
        self.data_sector = bytes.to_vec();
        self.sector_buf.clear();
        self.want_data = true;
        self.load_data_fifo();
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

fn to_bcd(v: u32) -> u8 {
    ((((v / 10) % 10) << 4) | (v % 10)) as u8
}

fn frames_to_msf(frames: u32) -> (u8, u8, u8) {
    (
        to_bcd(frames / 75 / 60),
        to_bcd((frames / 75) % 60),
        to_bcd(frames % 75),
    )
}

fn decode_xa(buf: &[u8], coding: u8) -> (i16, i16) {
    if buf.len() < 128 {
        return (0, 0);
    }
    let src = &buf[..128];
    let stereo = coding & 1 != 0;
    let eight = coding & 0x10 != 0;
    let mut old_l = 0i32;
    let mut older_l = 0i32;
    let mut old_r = 0i32;
    let mut older_r = 0i32;
    let l = xa_block(src, 0, 0, &mut old_l, &mut older_l, eight);
    if stereo {
        let r = xa_block(src, 0, 1, &mut old_r, &mut older_r, eight);
        (l, r)
    } else {
        (l, l)
    }
}

fn xa_block(
    src: &[u8],
    blk: usize,
    nibble: usize,
    old: &mut i32,
    older: &mut i32,
    eight: bool,
) -> i16 {
    const POS: [i32; 4] = [0, 60, 115, 98];
    const NEG: [i32; 4] = [0, 0, -52, -55];
    let header = src.get(4 + blk * 2 + nibble).copied().unwrap_or(0);
    let mut sh = u32::from(header & 0xF);
    if sh > 12 {
        sh = 9;
    }
    let filter = ((header >> 4) & 3) as usize;
    let f0 = POS[filter];
    let f1 = NEG[filter];
    let expand: u32 = if eight { 8 } else { 12 };
    let mut first = 0i16;
    for j in 0..28 {
        let t = if eight {
            let off = 16 + j * 4;
            let word = u32::from_le_bytes([
                src.get(off).copied().unwrap_or(0),
                src.get(off + 1).copied().unwrap_or(0),
                src.get(off + 2).copied().unwrap_or(0),
                src.get(off + 3).copied().unwrap_or(0),
            ]);
            let b = ((word >> (nibble * 8)) & 0xFF) as i8 as i32;
            b << expand.saturating_sub(sh)
        } else {
            let byte = src.get(16 + blk + j * 4).copied().unwrap_or(0);
            let nib = (byte >> (nibble * 4)) & 0xF;
            let t = if nib & 8 != 0 {
                i32::from(nib) - 16
            } else {
                i32::from(nib)
            };
            t << expand.saturating_sub(sh)
        };
        let s = t + (*old * f0 + *older * f1 + 32) / 64;
        let s = s.clamp(-0x8000, 0x7FFF);
        if j == 0 {
            first = s as i16;
        }
        *older = *old;
        *old = s;
    }
    first
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
        want_data(&mut cd, &mut irq);
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
        want_data(&mut cd, &mut irq);
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

    #[test]
    fn readn_fifo_stays_locked_while_dma_and_sector_clock_buffers_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR_LEN * 4 + 24..SECTOR_LEN * 4 + 24 + lic.len()].copy_from_slice(lic);
        bin[SECTOR_LEN * 5 + 24..SECTOR_LEN * 5 + 24 + 8].copy_from_slice(b"NEXTSECT");
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
        ack_irq(&mut cd, &mut irq);
        want_data(&mut cd, &mut irq);
        let mut head = Vec::new();
        for _ in 0..16 {
            head.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&head).contains("Licens"),
            "DMA started on the first sector ({head:?})"
        );
        pump(&mut cd, &mut irq, 300_000);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            0,
            "no extra INT1 while the FIFO is still being read (DMA in progress)"
        );
        let mut more = Vec::new();
        for _ in 0..16 {
            more.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&more).contains("ed  by"),
            "locked FIFO must not be replaced mid-DMA ({more:?})"
        );
        assert!(
            !String::from_utf8_lossy(&more).contains("NEXTSECT"),
            "must not skip to the next sector while DMA holds the FIFO ({more:?})"
        );
        for _ in 32..0x800 {
            let _ = cd.read8(2);
        }
        // Less than a fresh 2× sector (225792) from drain, but enough for the
        // leftover sector-clock that ran during the locked DMA.
        pump(&mut cd, &mut irq, 200_000);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            1,
            "SPX: sector clock buffered the next sector; INT1 after the locked FIFO drains"
        );
        want_data(&mut cd, &mut irq);
        let mut second = Vec::new();
        for _ in 0..8 {
            second.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&second).contains("NEXTSECT"),
            "buffered sector is the next one ({second:?})"
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

    fn want_data(cd: &mut Cdrom, irq: &mut Irq) {
        cd.write8(0, 0, irq);
        cd.write8(3, 0x80, irq);
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
        want_data(&mut cd, &mut irq);
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
        irq.write16(0x1F80_1070, !(1 << 2));
        pump(&mut cd, &mut irq, 0x0021_181C);
        let kind = hintsts(&mut cd, &mut irq);
        assert_ne!(kind, 1, "Pause must drop queued INT1 (got INT{kind})");
        assert_eq!(kind, 2, "Pause INT2");
        assert_ne!(
            irq.read16(0x1F80_1070) & (1 << 2),
            0,
            "SPX: HINTSTS 0→INT2 after Pause INT3 ack must edge I_STAT.CD"
        );
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
        want_data(&mut cd, &mut irq);
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
        want_data(&mut cd, &mut irq);
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
    fn setloc_while_hintsts_still_latches_loc_for_a_replacing_readn() {
        let dir = tempfile::tempdir().unwrap();
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR_LEN * 4 + 24..SECTOR_LEN * 4 + 24 + lic.len()].copy_from_slice(lic);
        bin[SECTOR_LEN * 5 + 24..SECTOR_LEN * 5 + 24 + 4].copy_from_slice(b"SEQ!");
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
        want_data(&mut cd, &mut irq);
        drain_data(&mut cd, &mut irq);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x09, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x0021_181C);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "Pause INT2");
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x08]);
        send(&mut cd, &mut irq, 0x06, &[]);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1);
        want_data(&mut cd, &mut irq);
        let mut body = Vec::new();
        for _ in 0..8 {
            body.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&body).contains("NEXT"),
            "Setloc while HINTSTS must still latch loc so replacing ReadN seeks (got {body:?})"
        );
        assert!(
            !String::from_utf8_lossy(&body).contains("SEQ!"),
            "must not continue sequentially after the old loc"
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
        want_data(&mut cd, &mut irq);
        drain_data(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1, "second sector INT1");
        want_data(&mut cd, &mut irq);
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
        want_data(&mut cd, &mut irq);
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
        want_data(&mut cd, &mut irq);
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

    #[test]
    fn int1_data_loads_only_when_want_data_is_set() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        set_2x_and_loc(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 1);
        cd.write8(0, 0, &mut irq);
        assert_eq!(
            cd.read8(0) & (1 << 6),
            0,
            "without Want Data the data FIFO stays empty"
        );
        want_data(&mut cd, &mut irq);
        cd.write8(0, 0, &mut irq);
        assert_ne!(cd.read8(0) & (1 << 6), 0, "BFRD loads the data FIFO");
        let mut bytes = Vec::new();
        for _ in 0..32 {
            bytes.push(cd.read8(2));
        }
        assert!(
            String::from_utf8_lossy(&bytes).contains("Licens"),
            "Want Data must load the INT1 sector"
        );
    }

    #[test]
    fn motor_on_mute_setfilter_getparam_play_use_command_summary() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x07, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "MotorOn INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "MotorOn INT2");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x07, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            5,
            "MotorOn while spinning is INT5"
        );
        assert_eq!(result_bytes(&mut cd, 2)[1], 0x20);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0B, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Mute INT3");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0D, &[0x01, 0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Setfilter INT3");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x80]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0F, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Getparam INT3");
        let gp = result_bytes(&mut cd, 5);
        assert_eq!(&gp[1..], &[0x80, 0x00, 0x01, 0x02]);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x03, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Play INT3");
    }

    #[test]
    fn getlocp_gettn_gettd_and_response_fifo_ack() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x15, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x11, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "GetlocP INT3");
        assert_eq!(
            result_bytes(&mut cd, 8),
            vec![0x01, 0x01, 0x00, 0x00, 0x04, 0x00, 0x02, 0x04]
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x13, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "GetTN INT3");
        let _ = cd.read8(1);
        ack_irq(&mut cd, &mut irq);
        assert_eq!(
            cd.read8(1),
            0,
            "IRQ ack must empty unread response bytes (GetTN is 3 bytes)"
        );
        send(&mut cd, &mut irq, 0x13, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        let tn = result_bytes(&mut cd, 3);
        assert_eq!(&tn[1..], &[0x01, 0x01], "single-track disc first=last=01h");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x14, &[0x01]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "GetTD INT3");
        assert_eq!(&result_bytes(&mut cd, 3)[1..], &[0x00, 0x02]);
    }

    #[test]
    fn data_fifo_repeats_spx_padding_byte_past_800h() {
        let dir = tempfile::tempdir().unwrap();
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        bin[SECTOR_LEN * 4 + 24 + 0x7F8] = 0xA5;
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
        cd.write8(0, 0, &mut irq);
        assert_eq!(
            cd.read8(2),
            0,
            "without Want Data the data port is 0, not the pad byte"
        );
        want_data(&mut cd, &mut irq);
        for _ in 0..0x800 {
            let _ = cd.read8(2);
        }
        assert_eq!(
            cd.read8(2),
            0xA5,
            "past 800h the data FIFO repeats the byte at 800h-8"
        );
        cd.write8(0, 0, &mut irq);
        cd.write8(3, 0, &mut irq);
        assert_eq!(
            cd.read8(2),
            0,
            "clearing BFRD must not keep returning the pad byte"
        );
    }

    fn cue_two_tracks(dir: &Path) -> std::path::PathBuf {
        let mut bin = vec![0u8; SECTOR_LEN * 200];
        let lic = b"          Licensed  by          Sony Computer Entertainment Amer  ica";
        bin[SECTOR_LEN * 4 + 24..SECTOR_LEN * 4 + 24 + lic.len()].copy_from_slice(lic);
        for i in 0..588 {
            let off = SECTOR_LEN * 150 + i * 4;
            bin[off..off + 2].copy_from_slice(&0x1000u16.to_le_bytes());
            bin[off + 2..off + 4].copy_from_slice(&0xE000u16.to_le_bytes());
        }
        std::fs::write(dir.join("game.bin"), &bin).unwrap();
        let cue = dir.join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        writeln!(f, "  TRACK 02 AUDIO").unwrap();
        writeln!(f, "    INDEX 01 00:02:00").unwrap();
        cue
    }

    fn cue_unlicensed(dir: &Path) -> std::path::PathBuf {
        let bin = vec![0u8; SECTOR_LEN * 24];
        std::fs::write(dir.join("game.bin"), &bin).unwrap();
        let cue = dir.join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        cue
    }

    fn cue_xa_and_bad(dir: &Path) -> std::path::PathBuf {
        let mut bin = vec![0u8; SECTOR_LEN * 24];
        let off = SECTOR_LEN * 4;
        bin[off] = 0x00;
        for b in bin.iter_mut().take(off + 11).skip(off + 1) {
            *b = 0xFF;
        }
        bin[off + 11] = 0x00;
        bin[off + 12] = 0x00;
        bin[off + 13] = 0x02;
        bin[off + 14] = 0x04;
        bin[off + 15] = 0x02;
        bin[off + 16] = 0x01;
        bin[off + 17] = 0x01;
        bin[off + 18] = 0x44;
        bin[off + 19] = 0x00;
        bin[off + 24] = 0x0C;
        for i in 0..112 {
            bin[off + 16 + 16 + i] = 0x77;
        }
        let bad = SECTOR_LEN * 5;
        bin[bad] = 0x00;
        for b in bin.iter_mut().take(bad + 11).skip(bad + 1) {
            *b = 0xFF;
        }
        bin[bad + 11] = 0x00;
        bin[bad + 15] = 0xFF;
        std::fs::write(dir.join("game.bin"), &bin).unwrap();
        let cue = dir.join("game.cue");
        let mut f = std::fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        cue
    }

    #[test]
    fn forward_backward_without_play_are_int5_stat_80h() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x04, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "Forward while idle is INT5");
        assert_eq!(result_bytes(&mut cd, 2)[1], 0x80);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x05, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "Backward while idle is INT5");
        assert_eq!(result_bytes(&mut cd, 2)[1], 0x80);
    }

    #[test]
    fn forward_while_playing_is_int3_and_skips_along_the_disc() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_two_tracks(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0C, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x03, &[0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Play INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        send(&mut cd, &mut irq, 0x11, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        let before = result_bytes(&mut cd, 8);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x04, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            3,
            "Forward while Playing is INT3"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x11, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        let after = result_bytes(&mut cd, 8);
        assert_ne!(
            &after[5..8],
            &before[5..8],
            "Forward must skip along the disc (before {before:?} after {after:?})"
        );
    }

    #[test]
    fn play_with_report_repeats_int1() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_two_tracks(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x03, &[0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Play INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584 * 12);
        assert_eq!(hintsts(&mut cd, &mut irq), 1, "Play+Report INT1");
        let r = result_bytes(&mut cd, 8);
        assert_eq!(r.len(), 8, "report is eight bytes");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584 * 12);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            1,
            "Play+Report INT1 repeats, not a one-shot"
        );
    }

    #[test]
    fn setsession_seekp_reset_getq_readtoc_match_spx() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x12, &[0x00]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "SetSession 00h is INT5");
        assert_eq!(result_bytes(&mut cd, 2), vec![0x03, 0x10]);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x12, &[0x01]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "SetSession 01h INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "SetSession 01h INT2");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x12, &[0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "SetSession 02h INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "bad session INT5");
        assert_eq!(result_bytes(&mut cd, 2)[1], 0x40);
        ack_irq(&mut cd, &mut irq);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            5,
            "SetSession 02h on a single-session disc is twice INT5(06h,40h)"
        );
        assert_eq!(result_bytes(&mut cd, 2), vec![0x06, 0x40]);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x10]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x16, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "SeekP INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "SeekP INT2");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x11, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            &result_bytes(&mut cd, 8)[5..8],
            &[0x00, 0x02, 0x10],
            "SeekP uses Setloc MM:SS:FF"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x1C, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "Reset INT3");
        assert_eq!(cd.view().mode, 0x20, "Reset sets mode=20h");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x400000);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            0,
            "Reset has no second INT; software waits 400000h cycles"
        );
        send(&mut cd, &mut irq, 0x1D, &[0x01, 0x01]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "GetQ INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "GetQ INT2");
        let q = result_bytes(&mut cd, 11);
        assert_eq!(q.len(), 11, "GetQ is 10 SubQ bytes plus peak LSB");
        assert_eq!(q[2], 0x01, "POINT=01h");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x1E, &[]);
        pump(&mut cd, &mut irq, 0x13CCE);
        assert_eq!(hintsts(&mut cd, &mut irq), 3, "ReadTOC INT3");
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x20_0000);
        assert_eq!(hintsts(&mut cd, &mut irq), 2, "ReadTOC INT2");
    }

    #[test]
    fn secret_unlock_and_video_cd_are_int5_11h_40h() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x50, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5);
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x40]);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x51, b"Licensed by");
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x40]);
        ack_irq(&mut cd, &mut irq);
        cd.write8(0, 0, &mut irq);
        cd.write8(2, 0xAB, &mut irq);
        cd.write8(1, 0x1F, &mut irq);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5);
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x40]);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x01, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            5,
            "1Fh must leave the param FIFO so leftover 0xAB makes Getstat INT5"
        );
    }

    #[test]
    fn test_19h_version_switches_region_chipset_scex() {
        let (_dir, mut cd) = load_licensed();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0x20]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(result_bytes(&mut cd, 4), vec![0x94, 0x09, 0x19, 0xC0]);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0x21]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3);
        assert_eq!(result_bytes(&mut cd, 1)[0] & 2, 0, "door closed");
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0x22]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(result_bytes(&mut cd, 7), b"for U/C".to_vec());
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0x23]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert!(
            result_bytes(&mut cd, 16)
                .windows(7)
                .any(|w| w == b"CXD2940" || w == b"CXD1817" || w == b"CXD2545"),
            "19h,23h is a chipset string"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0x05]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            result_bytes(&mut cd, 2),
            vec![0x01, 0x01],
            "licensed disc SCEx counters"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x19, &[0xFF]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "unknown Test 19h is INT5");
        assert_eq!(result_bytes(&mut cd, 2), vec![0x11, 0x10]);
    }

    #[test]
    fn gettn_gettd_use_disc_toc() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_two_tracks(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x13, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            &result_bytes(&mut cd, 3)[1..],
            &[0x01, 0x02],
            "GetTN first/last from TOC"
        );
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x14, &[0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            &result_bytes(&mut cd, 3)[1..],
            &[0x00, 0x04],
            "GetTD(2) is MM:SS of INDEX 01 00:02:00 plus 00:02 pregap"
        );
    }

    #[test]
    fn default_atv_keeps_cdda_stereo() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_two_tracks(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0C, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x03, &[0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(
            cd.take_analog(),
            (0x1000, -0x2000),
            "default ATV is L→L and R→R, not (L+R, 0)"
        );
    }

    #[test]
    fn smen_volume_sound_map_xa_filter_cdda() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_xa_and_bad(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        cd.write8(0, 0, &mut irq);
        cd.write8(3, 0x20, &mut irq);
        send(&mut cd, &mut irq, 0x01, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        cd.write8(0, 1, &mut irq);
        let flags = cd.read8(3);
        assert_eq!(flags & 0x17, 0x13, "SMEN + INT3 is INT13h (got {flags:#x})");
        ack_irq(&mut cd, &mut irq);

        let dir2 = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_two_tracks(dir2.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0C, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        cd.write8(0, 2, &mut irq);
        cd.write8(2, 0, &mut irq);
        cd.write8(3, 0, &mut irq);
        cd.write8(0, 3, &mut irq);
        cd.write8(1, 0, &mut irq);
        cd.write8(2, 0, &mut irq);
        cd.write8(3, 0x20, &mut irq);
        send(&mut cd, &mut irq, 0x03, &[0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        let muted = cd.take_analog();
        assert_eq!(muted, (0, 0), "ATV 0 applied is silent CD-DA");

        let mut cd = Cdrom::new();
        enable(&mut cd, &mut irq);
        cd.write8(0, 2, &mut irq);
        cd.write8(1, 0x01, &mut irq);
        cd.write8(0, 1, &mut irq);
        for _ in 0..18 {
            for _ in 0..16 {
                cd.write8(1, 0, &mut irq);
            }
            for _ in 0..112 {
                cd.write8(1, 0x07, &mut irq);
            }
        }
        let sm = cd.take_analog();
        assert_eq!(
            sm,
            (0x7000, 0),
            "Sound Map decodes 4-bit XA-ADPCM (nibble 7, shift 0) not raw PCM of the header bytes"
        );

        let dir3 = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_xa_and_bad(dir3.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0C, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x40]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        assert_eq!(
            hintsts(&mut cd, &mut irq) & 7,
            0,
            "XA-ADPCM (Setmode.6, MODE2 audio+realtime) must not raise CPU INT1"
        );
        let xa = cd.take_analog();
        assert_ne!(xa, (0, 0), "XA-ADPCM goes to analog/SPU");
        send(&mut cd, &mut irq, 0x09, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 0x0021_181C);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0D, &[0x02, 0x02]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x48]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x04]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 2_000_000);
        let _ = cd.take_analog();
        assert_eq!(
            cd.take_analog(),
            (0, 0),
            "Setmode filter drops non-matching XA file/channel"
        );
    }

    #[test]
    fn readn_retries_bad_sector_reads_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_xa_and_bad(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x80]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x05]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x15, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x06, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 225_792);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            0,
            "ReadN retries a bad sector instead of INT1/INT5 on the first attempt"
        );
        pump(&mut cd, &mut irq, 225_792 * 4);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "ReadN gives up with INT5");
        ack_irq(&mut cd, &mut irq);

        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_xa_and_bad(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x0E, &[0x80]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x02, &[0x00, 0x02, 0x05]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x15, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 451_584);
        ack_irq(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x1B, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 225_792);
        assert_eq!(
            hintsts(&mut cd, &mut irq),
            5,
            "ReadS does not retry: INT5 after one sector time"
        );
    }

    #[test]
    fn shell_open_int5_and_unlicensed_getid() {
        let mut cd = Cdrom::new();
        let mut irq = Irq::new();
        enable(&mut cd, &mut irq);
        cd.open_shell(&mut irq);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "shell-open INT5");
        assert_eq!(result_bytes(&mut cd, 2)[1], 0x80);
        ack_irq(&mut cd, &mut irq);
        cd.close_shell();

        let dir = tempfile::tempdir().unwrap();
        let disc = load_disc(&cue_unlicensed(dir.path())).unwrap();
        let mut cd = Cdrom::new();
        cd.insert(disc);
        enable(&mut cd, &mut irq);
        send(&mut cd, &mut irq, 0x1A, &[]);
        pump(&mut cd, &mut irq, 0xC4E1);
        assert_eq!(hintsts(&mut cd, &mut irq), 3);
        ack_irq(&mut cd, &mut irq);
        pump(&mut cd, &mut irq, 50_000);
        assert_eq!(hintsts(&mut cd, &mut irq), 5, "unlicensed GetID INT5");
        let id = result_bytes(&mut cd, 8);
        assert_eq!(id[0], 0x0A);
        assert_eq!(id[1], 0x80);
    }
}
