use crate::bus::Bus;
use crate::cop0::{self, Cop0};
use crate::gte::Gte;

const ICACHE_LINES: usize = 256;

struct ICacheLine {
    tag: u32,
    valid: u8,
    data: [u32; 4],
}

pub struct Cpu {
    gpr: [u32; 32],
    pc: u32,
    next_pc: u32,
    current_pc: u32,
    hi: u32,
    lo: u32,
    cop0: Cop0,
    gte: Gte,
    pending_load: Option<(u8, u32)>,
    last_write: Option<u8>,
    incoming_load: Option<(u8, u32)>,
    branch_delay: bool,
    in_delay: bool,
    icache: Vec<ICacheLine>,
    last_exception: Option<(u8, u32, u32)>,
    pub exception_log: Vec<(u8, u32, u32)>,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            gpr: [0; 32],
            pc: 0xBFC0_0000,
            next_pc: 0xBFC0_0004,
            current_pc: 0xBFC0_0000,
            hi: 0,
            lo: 0,
            cop0: Cop0::new(),
            gte: Gte::new(),
            pending_load: None,
            last_write: None,
            incoming_load: None,
            branch_delay: false,
            in_delay: false,
            last_exception: None,
            exception_log: Vec::new(),
            icache: (0..ICACHE_LINES)
                .map(|_| ICacheLine {
                    tag: 0,
                    valid: 0,
                    data: [0; 4],
                })
                .collect(),
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn pc(&self) -> u32 {
        self.pc
    }

    pub fn gpr(&self, i: u8) -> u32 {
        self.gpr[i as usize]
    }

    pub fn last_exception(&self) -> Option<(u8, u32, u32)> {
        self.last_exception
    }

    pub fn sr(&self) -> u32 {
        self.cop0.sr
    }

    pub fn badvaddr(&self) -> u32 {
        self.cop0.badvaddr
    }

    fn set_gpr(&mut self, i: u8, v: u32) {
        if i != 0 {
            self.gpr[i as usize] = v;
            self.last_write = Some(i);
        }
    }

    fn delay_load(&mut self, rt: u8, value: u32) {
        if rt != 0 {
            self.pending_load = Some((rt, value));
            self.last_write = Some(rt);
        }
    }

    pub fn step(&mut self, bus: &mut Bus) {
        // Sample IRQ after committing delay-slot state. A pending INT on the
        // instruction *after* a taken branch must not fire with BD=0 on the
        // delay slot: that returns into the fall-through and misses the target.
        self.in_delay = self.branch_delay;
        self.branch_delay = false;
        self.current_pc = self.pc;

        self.cop0
            .set_ip_hw(bus.irq().pending_for_cop0());
        let irq = self.cop0.iec()
            && (self.cop0.cause & self.cop0.sr & 0xFF00) != 0;
        if irq && !self.in_delay {
            self.exception(bus, cop0::EXC_INT, 0);
            return;
        }

        if self.current_pc & 3 != 0 {
            self.cop0.badvaddr = self.current_pc;
            self.exception(bus, cop0::EXC_ADEL, 0);
            return;
        }

        let instr = match self.fetch(bus) {
            Some(w) => w,
            None => {
                self.exception(bus, cop0::EXC_IBE, 0);
                return;
            }
        };

        self.pc = self.next_pc;
        self.next_pc = self.next_pc.wrapping_add(4);

        let incoming = self.pending_load.take();
        self.incoming_load = incoming;
        self.last_write = None;
        self.decode_execute(bus, instr);
        if let Some((reg, val)) = incoming {
            if self.last_write != Some(reg) && reg != 0 {
                self.gpr[reg as usize] = val;
            }
        }
        bus.tick(2);
    }

    fn fetch(&mut self, bus: &mut Bus) -> Option<u32> {
        let addr = self.current_pc;
        let cached = (addr >> 29) != 5; // not KSEG1
        let icache_on = bus.cache_ctrl() & (1 << 11) != 0;
        if !cached || self.cop0.isolate_cache() || !icache_on {
            return bus.read32(addr);
        }
        let phys = addr & 0x1FFF_FFFF;
        let line = ((phys >> 4) & 0xFF) as usize;
        let word = ((phys >> 2) & 3) as usize;
        let tag = phys >> 12;
        let c = &mut self.icache[line];
        if c.tag != tag || (c.valid & (1 << word)) == 0 {
            c.tag = tag;
            c.valid = 0;
            let base = addr & !0xF;
            for i in word..4 {
                if let Some(w) = bus.read32(base + (i as u32) * 4) {
                    c.data[i] = w;
                    c.valid |= 1 << i;
                } else {
                    return None;
                }
            }
        }
        Some(self.icache[line].data[word])
    }

    fn exception(&mut self, _bus: &mut Bus, code: u8, ce: u8) {
        self.last_exception = Some((code, self.current_pc, self.cop0.cause));
        if code != cop0::EXC_INT
            && code != cop0::EXC_SYS
            && self.exception_log.len() < 24
        {
            self.exception_log
                .push((code, self.current_pc, self.gpr[31]));
        }
        let mut epc = self.current_pc;
        let bd = self.in_delay;
        if bd {
            epc = epc.wrapping_sub(4);
        }
        if code == cop0::EXC_INT {
            let instr = 0; // skip GTE-on-IRQ handled at decode if needed
            let _ = instr;
        }
        let handler = self.cop0.enter_exception(epc, code, bd, ce);
        self.pc = handler;
        self.next_pc = handler.wrapping_add(4);
        self.branch_delay = false;
        self.in_delay = false;
    }

    fn decode_execute(&mut self, bus: &mut Bus, instr: u32) {
        let op = instr >> 26;
        let rs = ((instr >> 21) & 0x1F) as u8;
        let rt = ((instr >> 16) & 0x1F) as u8;
        let rd = ((instr >> 11) & 0x1F) as u8;
        let sa = ((instr >> 6) & 0x1F) as u8;
        let fn_ = instr & 0x3F;
        let imm = instr as u16;
        let simm = imm as i16 as i32 as u32;
        let target = instr & 0x03FF_FFFF;

        match op {
            0x00 => self.special(bus, rs, rt, rd, sa, fn_),
            0x01 => self.bcond(rs, rt, simm),
            0x02 => self.jump(target, false),
            0x03 => self.jump(target, true),
            0x04 => self.branch(self.gpr(rs) == self.gpr(rt), simm, false),
            0x05 => self.branch(self.gpr(rs) != self.gpr(rt), simm, false),
            0x06 => self.branch((self.gpr(rs) as i32) <= 0, simm, false),
            0x07 => self.branch((self.gpr(rs) as i32) > 0, simm, false),
            0x08 => self.add_imm(rt, rs, simm, true),
            0x09 => self.set_gpr(rt, self.gpr(rs).wrapping_add(simm)),
            0x0A => self.set_gpr(
                rt,
                u32::from((self.gpr(rs) as i32) < (simm as i32)),
            ),
            0x0B => self.set_gpr(rt, u32::from(self.gpr(rs) < simm)),
            0x0C => self.set_gpr(rt, self.gpr(rs) & u32::from(imm)),
            0x0D => self.set_gpr(rt, self.gpr(rs) | u32::from(imm)),
            0x0E => self.set_gpr(rt, self.gpr(rs) ^ u32::from(imm)),
            0x0F => self.set_gpr(rt, u32::from(imm) << 16),
            0x10 => self.cop0(bus, rs, rt, rd, instr),
            0x12 => self.cop2(bus, rs, rt, rd, instr),
            0x20 => self.load(bus, rt, self.gpr(rs).wrapping_add(simm), Width::Byte, true),
            0x21 => self.load(bus, rt, self.gpr(rs).wrapping_add(simm), Width::Half, true),
            0x22 => self.lwl(bus, rt, self.gpr(rs).wrapping_add(simm)),
            0x23 => self.load(bus, rt, self.gpr(rs).wrapping_add(simm), Width::Word, true),
            0x24 => self.load(bus, rt, self.gpr(rs).wrapping_add(simm), Width::Byte, false),
            0x25 => self.load(bus, rt, self.gpr(rs).wrapping_add(simm), Width::Half, false),
            0x26 => self.lwr(bus, rt, self.gpr(rs).wrapping_add(simm)),
            0x28 => self.store(bus, self.gpr(rt), self.gpr(rs).wrapping_add(simm), Width::Byte),
            0x29 => self.store(bus, self.gpr(rt), self.gpr(rs).wrapping_add(simm), Width::Half),
            0x2A => self.swl(bus, rt, self.gpr(rs).wrapping_add(simm)),
            0x2B => self.store(bus, self.gpr(rt), self.gpr(rs).wrapping_add(simm), Width::Word),
            0x2E => self.swr(bus, rt, self.gpr(rs).wrapping_add(simm)),
            0x32 => self.lwc2(bus, rt, self.gpr(rs).wrapping_add(simm)),
            0x3A => self.swc2(bus, rt, self.gpr(rs).wrapping_add(simm)),
            _ => self.exception(bus, cop0::EXC_RI, 0),
        }
    }

    fn special(&mut self, bus: &mut Bus, rs: u8, rt: u8, rd: u8, sa: u8, fn_: u32) {
        match fn_ {
            0x00 => self.set_gpr(rd, self.gpr(rt).wrapping_shl(u32::from(sa))),
            0x02 => self.set_gpr(rd, self.gpr(rt).wrapping_shr(u32::from(sa))),
            0x03 => self.set_gpr(rd, ((self.gpr(rt) as i32) >> sa) as u32),
            0x04 => self.set_gpr(rd, self.gpr(rt).wrapping_shl(self.gpr(rs) & 0x1F)),
            0x06 => self.set_gpr(rd, self.gpr(rt).wrapping_shr(self.gpr(rs) & 0x1F)),
            0x07 => self.set_gpr(
                rd,
                ((self.gpr(rt) as i32) >> (self.gpr(rs) & 0x1F)) as u32,
            ),
            0x08 => self.jr(self.gpr(rs), None),
            0x09 => self.jr(self.gpr(rs), Some(rd)),
            0x0C => self.exception(bus, cop0::EXC_SYS, 0),
            0x0D => self.exception(bus, cop0::EXC_BP, 0),
            0x10 => self.set_gpr(rd, self.hi),
            0x11 => self.hi = self.gpr(rs),
            0x12 => self.set_gpr(rd, self.lo),
            0x13 => self.lo = self.gpr(rs),
            0x18 => {
                let r = (self.gpr(rs) as i32 as i64) * (self.gpr(rt) as i32 as i64);
                self.lo = r as u32;
                self.hi = (r >> 32) as u32;
            }
            0x19 => {
                let r = u64::from(self.gpr(rs)) * u64::from(self.gpr(rt));
                self.lo = r as u32;
                self.hi = (r >> 32) as u32;
            }
            0x1A => {
                let n = self.gpr(rs) as i32;
                let d = self.gpr(rt) as i32;
                if d == 0 {
                    self.hi = n as u32;
                    self.lo = if n >= 0 { 0xFFFF_FFFF } else { 1 };
                } else if n as u32 == 0x8000_0000 && d == -1 {
                    self.lo = 0x8000_0000;
                    self.hi = 0;
                } else {
                    self.lo = (n / d) as u32;
                    self.hi = (n % d) as u32;
                }
            }
            0x1B => {
                let n = self.gpr(rs);
                let d = self.gpr(rt);
                if d == 0 {
                    self.hi = n;
                    self.lo = 0xFFFF_FFFF;
                } else {
                    self.lo = n / d;
                    self.hi = n % d;
                }
            }
            0x20 => self.add_reg(rd, rs, rt, true),
            0x21 => self.set_gpr(rd, self.gpr(rs).wrapping_add(self.gpr(rt))),
            0x22 => self.sub_reg(rd, rs, rt, true),
            0x23 => self.set_gpr(rd, self.gpr(rs).wrapping_sub(self.gpr(rt))),
            0x24 => self.set_gpr(rd, self.gpr(rs) & self.gpr(rt)),
            0x25 => self.set_gpr(rd, self.gpr(rs) | self.gpr(rt)),
            0x26 => self.set_gpr(rd, self.gpr(rs) ^ self.gpr(rt)),
            0x27 => self.set_gpr(rd, !(self.gpr(rs) | self.gpr(rt))),
            0x2A => self.set_gpr(
                rd,
                u32::from((self.gpr(rs) as i32) < (self.gpr(rt) as i32)),
            ),
            0x2B => self.set_gpr(rd, u32::from(self.gpr(rs) < self.gpr(rt))),
            _ => self.exception(bus, cop0::EXC_RI, 0),
        }
    }

    fn add_imm(&mut self, rt: u8, rs: u8, simm: u32, trap: bool) {
        let a = self.gpr(rs) as i32;
        let b = simm as i32;
        match a.checked_add(b) {
            Some(v) => self.set_gpr(rt, v as u32),
            None if trap => {}
            None => self.set_gpr(rt, a.wrapping_add(b) as u32),
        }
    }

    fn add_reg(&mut self, rd: u8, rs: u8, rt: u8, trap: bool) {
        let a = self.gpr(rs) as i32;
        let b = self.gpr(rt) as i32;
        match a.checked_add(b) {
            Some(v) => self.set_gpr(rd, v as u32),
            None if trap => { /* overflow: handled by returning without write; caller exceptions */ }
            None => self.set_gpr(rd, a.wrapping_add(b) as u32),
        }
    }

    fn sub_reg(&mut self, rd: u8, rs: u8, rt: u8, trap: bool) {
        let a = self.gpr(rs) as i32;
        let b = self.gpr(rt) as i32;
        match a.checked_sub(b) {
            Some(v) => self.set_gpr(rd, v as u32),
            None if trap => {}
            None => self.set_gpr(rd, a.wrapping_sub(b) as u32),
        }
    }

    fn bcond(&mut self, rs: u8, rt: u8, simm: u32) {
        let s = self.gpr(rs) as i32;
        let link = rt & 0x10 != 0;
        let ge = rt & 1 != 0;
        let take = if ge { s >= 0 } else { s < 0 };
        if link {
            self.set_gpr(31, self.pc.wrapping_add(4));
        }
        self.branch(take, simm, false);
    }

    fn branch(&mut self, take: bool, simm: u32, _link: bool) {
        self.branch_delay = true;
        if take {
            self.next_pc = self.pc.wrapping_add(simm << 2);
        }
    }

    fn jump(&mut self, target: u32, link: bool) {
        self.branch_delay = true;
        if link {
            self.set_gpr(31, self.pc.wrapping_add(4));
        }
        self.next_pc = (self.pc & 0xF000_0000) | (target << 2);
    }

    fn jr(&mut self, dest: u32, link: Option<u8>) {
        self.branch_delay = true;
        if let Some(rd) = link {
            self.set_gpr(rd, self.pc.wrapping_add(4));
        }
        self.next_pc = dest;
    }

    fn cop0(&mut self, bus: &mut Bus, rs: u8, rt: u8, rd: u8, instr: u32) {
        match rs {
            0x00 => match self.cop0.read(rd) {
                Ok(v) => self.delay_load(rt, v),
                Err(_) => self.exception(bus, cop0::EXC_RI, 0),
            },
            0x04 => {
                if self.cop0.write(rd, self.gpr(rt)).is_err() {
                    self.exception(bus, cop0::EXC_RI, 0);
                }
            }
            0x10 => {
                if instr & 0x3F == 0x10 {
                    self.cop0.rfe();
                } else {
                    self.exception(bus, cop0::EXC_RI, 0);
                }
            }
            _ => self.exception(bus, cop0::EXC_RI, 0),
        }
    }

    fn cop2(&mut self, bus: &mut Bus, rs: u8, rt: u8, rd: u8, instr: u32) {
        if !self.cop0.cu2() && (self.cop0.sr & 2) != 0 {
            // user mode without CU2
        }
        if instr & (1 << 25) != 0 {
            self.gte.command(instr);
            return;
        }
        match rs {
            0x00 => self.delay_load(rt, self.gte.read_data(rd)),
            0x02 => self.delay_load(rt, self.gte.read_control(rd)),
            0x04 => self.gte.write_data(rd, self.gpr(rt)),
            0x06 => self.gte.write_control(rd, self.gpr(rt)),
            _ => self.exception(bus, cop0::EXC_RI, 2),
        }
    }

    fn load(&mut self, bus: &mut Bus, rt: u8, addr: u32, width: Width, sign: bool) {
        if self.cop0.isolate_cache() {
            self.delay_load(rt, 0);
            return;
        }
        match width {
            Width::Byte => {
                let v = bus.read8(addr).unwrap_or(0);
                let v = if sign { v as i8 as u32 } else { u32::from(v) };
                self.delay_load(rt, v);
            }
            Width::Half => {
                if addr & 1 != 0 {
                    self.cop0.badvaddr = addr;
                    self.exception(bus, cop0::EXC_ADEL, 0);
                    return;
                }
                let v = bus.read16(addr).unwrap_or(0);
                let v = if sign { v as i16 as u32 } else { u32::from(v) };
                self.delay_load(rt, v);
            }
            Width::Word => {
                if addr & 3 != 0 {
                    self.cop0.badvaddr = addr;
                    self.exception(bus, cop0::EXC_ADEL, 0);
                    return;
                }
                let v = bus.read32(addr).unwrap_or(0);
                self.delay_load(rt, v);
            }
        }
    }

    fn store(&mut self, bus: &mut Bus, value: u32, addr: u32, width: Width) {
        if self.cop0.isolate_cache() {
            self.store_icache(bus.cache_ctrl(), addr, value);
            return;
        }
        match width {
            Width::Byte => bus.write8(addr, value as u8),
            Width::Half => {
                if addr & 1 != 0 {
                    self.cop0.badvaddr = addr;
                    self.exception(bus, cop0::EXC_ADES, 0);
                    return;
                }
                bus.write16(addr, value as u16);
            }
            Width::Word => {
                if addr & 3 != 0 {
                    self.cop0.badvaddr = addr;
                    self.exception(bus, cop0::EXC_ADES, 0);
                    return;
                }
                bus.write32(addr, value);
            }
        }
    }

    fn store_icache(&mut self, bcc: u32, addr: u32, value: u32) {
        let phys = addr & 0x1FFF_FFFF;
        let line = ((phys >> 4) & 0xFF) as usize;
        let word = ((phys >> 2) & 3) as usize;
        let c = &mut self.icache[line];
        if bcc & (1 << 2) != 0 {
            // TAG test mode: low 4 bits of data are per-word valid; code unchanged.
            c.tag = phys >> 12;
            c.valid = (value & 0xF) as u8;
            return;
        }
        c.data[word] = value;
    }

    fn lwl(&mut self, bus: &mut Bus, rt: u8, addr: u32) {
        let aligned = addr & !3;
        let word = bus.read32(aligned).unwrap_or(0);
        let cur = self
            .incoming_load
            .filter(|(r, _)| *r == rt)
            .map(|(_, v)| v)
            .unwrap_or(self.gpr(rt));
        let result = match addr & 3 {
            0 => (cur & 0x00FF_FFFF) | (word << 24),
            1 => (cur & 0x0000_FFFF) | (word << 16),
            2 => (cur & 0x0000_00FF) | (word << 8),
            _ => word,
        };
        self.delay_load(rt, result);
    }

    fn lwr(&mut self, bus: &mut Bus, rt: u8, addr: u32) {
        let aligned = addr & !3;
        let word = bus.read32(aligned).unwrap_or(0);
        let cur = self
            .incoming_load
            .filter(|(r, _)| *r == rt)
            .map(|(_, v)| v)
            .unwrap_or(self.gpr(rt));
        let result = match addr & 3 {
            0 => word,
            1 => (cur & 0xFF00_0000) | (word >> 8),
            2 => (cur & 0xFFFF_0000) | (word >> 16),
            _ => (cur & 0xFFFF_FF00) | (word >> 24),
        };
        self.delay_load(rt, result);
    }

    fn swl(&mut self, bus: &mut Bus, rt: u8, addr: u32) {
        let aligned = addr & !3;
        let mut mem = bus.read32(aligned).unwrap_or(0);
        let v = self.gpr(rt);
        mem = match addr & 3 {
            0 => (mem & 0xFFFF_FF00) | (v >> 24),
            1 => (mem & 0xFFFF_0000) | (v >> 16),
            2 => (mem & 0xFF00_0000) | (v >> 8),
            _ => v,
        };
        bus.write32(aligned, mem);
    }

    fn swr(&mut self, bus: &mut Bus, rt: u8, addr: u32) {
        let aligned = addr & !3;
        let mut mem = bus.read32(aligned).unwrap_or(0);
        let v = self.gpr(rt);
        mem = match addr & 3 {
            0 => v,
            1 => (mem & 0x0000_00FF) | (v << 8),
            2 => (mem & 0x0000_FFFF) | (v << 16),
            _ => (mem & 0x00FF_FFFF) | (v << 24),
        };
        bus.write32(aligned, mem);
    }

    fn lwc2(&mut self, bus: &mut Bus, rt: u8, addr: u32) {
        if addr & 3 != 0 {
            self.cop0.badvaddr = addr;
            self.exception(bus, cop0::EXC_ADEL, 0);
            return;
        }
        let v = bus.read32(addr).unwrap_or(0);
        self.gte.write_data(rt, v);
    }

    fn swc2(&mut self, bus: &mut Bus, rt: u8, addr: u32) {
        if addr & 3 != 0 {
            self.cop0.badvaddr = addr;
            self.exception(bus, cop0::EXC_ADES, 0);
            return;
        }
        bus.write32(addr, self.gte.read_data(rt));
    }
}

enum Width {
    Byte,
    Half,
    Word,
}
