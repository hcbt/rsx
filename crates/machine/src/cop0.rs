/// COP0: SR, CAUSE, EPC, and the handful of other R3000A registers.
#[derive(Clone)]
pub struct Cop0 {
    pub sr: u32,
    pub cause: u32,
    pub epc: u32,
    pub badvaddr: u32,
    pub prid: u32,
    bpc: u32,
    bda: u32,
    tar: u32,
    dcic: u32,
    bdam: u32,
    bpcm: u32,
}

#[allow(dead_code)]
impl Cop0 {
    pub fn new() -> Self {
        Self {
            sr: 1 << 22, // BEV on reset
            cause: 0,
            epc: 0,
            badvaddr: 0,
            prid: 2,
            bpc: 0,
            bda: 0,
            tar: 0,
            dcic: 0,
            bdam: 0,
            bpcm: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn bev(&self) -> bool {
        self.sr & (1 << 22) != 0
    }

    pub fn iec(&self) -> bool {
        self.sr & 1 != 0
    }

    pub fn im(&self) -> u32 {
        (self.sr >> 8) & 0xFF
    }

    pub fn isolate_cache(&self) -> bool {
        self.sr & (1 << 16) != 0
    }

    pub fn cu2(&self) -> bool {
        self.sr & (1 << 30) != 0
    }

    pub fn read(&self, rd: u8) -> Result<u32, Cop0Error> {
        match rd {
            3 => Ok(self.bpc),
            5 => Ok(self.bda),
            6 => Ok(self.tar),
            7 => Ok(self.dcic),
            8 => Ok(self.badvaddr),
            9 => Ok(self.bdam),
            11 => Ok(self.bpcm),
            12 => Ok(self.sr),
            13 => Ok(self.cause),
            14 => Ok(self.epc),
            15 => Ok(self.prid),
            16..=31 => Ok(0x20),
            _ => Err(Cop0Error::Reserved),
        }
    }

    pub fn write(&mut self, rd: u8, value: u32) -> Result<(), Cop0Error> {
        match rd {
            3 => self.bpc = value,
            5 => self.bda = value,
            7 => self.dcic = value,
            9 => self.bdam = value,
            11 => self.bpcm = value,
            12 => self.sr = value & 0xF4FF_FFFF,
            13 => {
                // software IRQ bits 8-9 are writable
                self.cause = (self.cause & !0x300) | (value & 0x300);
            }
            16..=31 => {}
            _ => return Err(Cop0Error::Reserved),
        }
        Ok(())
    }

    pub fn rfe(&mut self) {
        let mode = self.sr & 0x3F;
        self.sr = (self.sr & !0xF) | ((mode >> 2) & 0xF);
    }

    pub fn enter_exception(&mut self, epc: u32, code: u8, bd: bool, ce: u8) -> u32 {
        self.epc = epc;
        self.cause = (u32::from(code) << 2) | (u32::from(ce) << 28);
        if bd {
            self.cause |= 1 << 31;
        }
        let mode = self.sr & 0x3F;
        self.sr = (self.sr & !0x3F) | ((mode << 2) & 0x3F);
        if self.bev() {
            0xBFC0_0180
        } else {
            0x8000_0080
        }
    }

    pub fn set_ip_hw(&mut self, pending: bool) {
        if pending {
            self.cause |= 1 << 10;
        } else {
            self.cause &= !(1 << 10);
        }
    }
}

pub enum Cop0Error {
    Reserved,
}

pub const EXC_INT: u8 = 0x00;
pub const EXC_ADEL: u8 = 0x04;
pub const EXC_ADES: u8 = 0x05;
pub const EXC_IBE: u8 = 0x06;
pub const EXC_DBE: u8 = 0x07;
pub const EXC_SYS: u8 = 0x08;
pub const EXC_BP: u8 = 0x09;
pub const EXC_RI: u8 = 0x0A;
pub const EXC_CPU: u8 = 0x0B;
pub const EXC_OVF: u8 = 0x0C;
