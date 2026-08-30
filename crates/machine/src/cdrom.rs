use crate::irq::{self, Irq};

/// No-disc CD-ROM controller: Init / Nop / Getstat / GetID.
pub struct Cdrom {
    index: u8,
    status: u8,
    irq_enable: u8,
    irq_flag: u8,
    param: Vec<u8>,
    result: Vec<u8>,
    result_i: usize,
    pending: Option<Pending>,
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
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
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
            (2, _) => 0,
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
        self.status |= 1 << 7; // busy
        let stat = self.controller_stat();
        let (first, second) = match cmd {
            0x01 => (Some((1000, 3, vec![stat])), None), // Nop
            0x0A => (
                Some((5000, 3, vec![stat])),
                Some((20_000, 2, vec![stat | 2])),
            ), // Init
            0x19 => {
                // Test
                let sub = self.param.first().copied().unwrap_or(0);
                if sub == 0x20 {
                    (Some((1000, 3, vec![0x94, 0x09, 0x19, 0xC0])), None)
                } else {
                    (Some((1000, 3, vec![stat])), None)
                }
            }
            0x1A => {
                // GetID, no disc: INT3 then INT5
                (
                    Some((4000, 3, vec![stat])),
                    Some((30_000, 5, vec![0x08, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00])),
                )
            }
            _ => (Some((2000, 3, vec![stat])), None),
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

    fn controller_stat(&self) -> u8 {
        0 // lid closed, motor off, no disc
    }
}
