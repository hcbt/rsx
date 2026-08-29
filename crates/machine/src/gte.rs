//! Geometry Transformation Engine (COP2), from PSX-SPX formulas.

#[derive(Clone)]
pub struct Gte {
    data: [u32; 32],
    ctrl: [u32; 32],
}

impl Gte {
    pub fn new() -> Self {
        Self {
            data: [0; 32],
            ctrl: [0; 32],
        }
    }

    pub fn read_data(&self, reg: u8) -> u32 {
        match reg {
            1 | 3 | 5 | 8 | 9 | 10 | 11 => sign16(self.data[reg as usize] as u16) as u32,
            7 | 16 | 17 | 18 | 19 => self.data[reg as usize] & 0xFFFF,
            15 => self.data[14],
            28 | 29 => self.irgb(),
            31 => self.lzcr(),
            r => self.data[r as usize],
        }
    }

    pub fn write_data(&mut self, reg: u8, value: u32) {
        match reg {
            15 => {
                self.data[12] = self.data[13];
                self.data[13] = self.data[14];
                self.data[14] = value;
            }
            28 => {
                let r = (value & 0x1F) * 0x80;
                let g = ((value >> 5) & 0x1F) * 0x80;
                let b = ((value >> 10) & 0x1F) * 0x80;
                self.data[9] = r;
                self.data[10] = g;
                self.data[11] = b;
            }
            29 | 31 => {}
            r => self.data[r as usize] = value,
        }
    }

    pub fn read_control(&self, reg: u8) -> u32 {
        let v = self.ctrl[reg as usize];
        match reg {
            4 | 12 | 20 | 26 | 27 | 29 | 30 => sign16(v as u16) as u32,
            _ => v,
        }
    }

    pub fn write_control(&mut self, reg: u8, value: u32) {
        self.ctrl[reg as usize] = value;
    }

    pub fn command(&mut self, instr: u32) {
        let op = instr & 0x3F;
        let sf = (instr >> 19) & 1;
        let lm = (instr >> 10) & 1 != 0;
        self.ctrl[31] = 0;
        match op {
            0x01 => self.rtps(sf, lm),
            0x06 => self.nclip(),
            0x0C => self.op(sf, lm),
            0x10 => self.dpcs(sf, lm),
            0x11 => self.intpl(sf, lm),
            0x12 => self.mvmva(instr, sf, lm),
            0x13 => self.ncds(sf, lm),
            0x16 => {
                self.ncds(sf, lm);
                self.ncd_vector(1, sf, lm);
                self.ncd_vector(2, sf, lm);
            }
            0x1B => self.nccs(sf, lm),
            0x1E => self.ncs(sf, lm),
            0x20 => {
                self.ncs(sf, lm);
                self.nc_vector(1, sf, lm);
                self.nc_vector(2, sf, lm);
            }
            0x28 => self.sqr(sf),
            0x29 => self.dcpl(sf, lm),
            0x2A => {
                for _ in 0..3 {
                    self.dpct_once(sf, lm);
                }
            }
            0x2D => self.avsz3(),
            0x2E => self.avsz4(),
            0x30 => {
                self.rtps(sf, lm);
                self.rtp_vector(1, sf, lm);
                self.rtp_vector(2, sf, lm);
            }
            0x3D => self.gpf(sf, lm),
            0x3E => self.gpl(sf, lm),
            0x3F => {
                self.nccs(sf, lm);
                self.ncc_vector(1, sf, lm);
                self.ncc_vector(2, sf, lm);
            }
            _ => {}
        }
        let f = self.ctrl[31];
        if f & 0x7F87_E000 != 0 {
            self.ctrl[31] |= 1 << 31;
        }
    }

    fn irgb(&self) -> u32 {
        let sat = |v: u32| {
            let s = (v as i32) / 0x80;
            s.clamp(0, 0x1F) as u32
        };
        sat(self.data[9]) | (sat(self.data[10]) << 5) | (sat(self.data[11]) << 10)
    }

    fn lzcr(&self) -> u32 {
        let v = self.data[30];
        if v as i32 >= 0 {
            v.leading_zeros()
        } else {
            v.leading_ones()
        }
    }

    fn i16_pair(v: u32) -> (i32, i32) {
        ((v as i16) as i32, ((v >> 16) as i16) as i32)
    }

    fn vx(&self, n: usize) -> (i32, i32, i32) {
        let (x, y) = Self::i16_pair(self.data[n * 2]);
        let z = self.data[n * 2 + 1] as i16 as i32;
        (x, y, z)
    }

    fn ir_vec(&self) -> (i32, i32, i32) {
        (
            self.data[9] as i16 as i32,
            self.data[10] as i16 as i32,
            self.data[11] as i16 as i32,
        )
    }

    fn rt_el(&self, r: usize, c: usize) -> i32 {
        let idx = r * 3 + c;
        let word = idx / 2;
        let v = self.ctrl[word];
        if idx % 2 == 0 {
            v as i16 as i32
        } else if word == 4 {
            v as i16 as i32
        } else {
            (v >> 16) as i16 as i32
        }
    }

    fn set_ir(&mut self, i: usize, mac: i64, lm: bool) {
        let min = if lm { 0 } else { -0x8000 };
        let mut v = mac;
        if v > 0x7FFF {
            v = 0x7FFF;
            self.flag(24 - (i as u32));
        }
        if v < min {
            v = min;
            self.flag(24 - (i as u32));
        }
        self.data[9 + i] = v as u32;
    }

    fn flag(&mut self, bit: u32) {
        self.ctrl[31] |= 1 << bit;
    }

    fn rtp_vector(&mut self, vec: usize, sf: u32, lm: bool) {
        let (vx, vy, vz) = self.vx(vec);
        let trx = self.ctrl[5] as i32 as i64;
        let try_ = self.ctrl[6] as i32 as i64;
        let trz = self.ctrl[7] as i32 as i64;
        let mut mac = [0i64; 3];
        for r in 0..3 {
            let tr = [trx, try_, trz][r] << 12;
            let m = tr
                + i64::from(self.rt_el(r, 0)) * i64::from(vx)
                + i64::from(self.rt_el(r, 1)) * i64::from(vy)
                + i64::from(self.rt_el(r, 2)) * i64::from(vz);
            mac[r] = m;
        }
        for i in 0..3 {
            let shifted = mac[i] >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        let sz = (mac[2] >> ((1 - sf) * 12)).clamp(0, 0xFFFF) as u32;
        self.data[16] = self.data[17];
        self.data[17] = self.data[18];
        self.data[18] = self.data[19];
        self.data[19] = sz;
        if mac[2] >> ((1 - sf) * 12) > 0xFFFF {
            self.flag(18);
        }
        let n = unr_divide(self.ctrl[26] as u16, sz as u16, &mut self.ctrl[31]);
        let ofx = self.ctrl[24] as i32 as i64;
        let ofy = self.ctrl[25] as i32 as i64;
        let ir1 = self.data[9] as i16 as i64;
        let ir2 = self.data[10] as i16 as i64;
        let sx = ((i64::from(n) * ir1) + ofx) >> 16;
        let sy = ((i64::from(n) * ir2) + ofy) >> 16;
        let sx = saturate_sx(sx, &mut self.ctrl[31]);
        let sy = saturate_sy(sy, &mut self.ctrl[31]);
        self.data[12] = self.data[13];
        self.data[13] = self.data[14];
        self.data[14] = ((sy as u32) << 16) | (sx as u16 as u32);
        let dqa = self.ctrl[27] as i16 as i64;
        let dqb = self.ctrl[28] as i32 as i64;
        let mac0 = i64::from(n) * dqa + dqb;
        self.data[24] = mac0 as u32;
        let ir0 = (mac0 >> 12).clamp(0, 0x1000);
        self.data[8] = ir0 as u32;
    }

    fn rtps(&mut self, sf: u32, lm: bool) {
        self.rtp_vector(0, sf, lm);
    }

    fn nclip(&mut self) {
        let sx = |i: usize| self.data[12 + i] as i16 as i64;
        let sy = |i: usize| (self.data[12 + i] >> 16) as i16 as i64;
        let mac0 = sx(0) * sy(1) + sx(1) * sy(2) + sx(2) * sy(0)
            - sx(0) * sy(2)
            - sx(1) * sy(0)
            - sx(2) * sy(1);
        self.data[24] = mac0 as u32;
    }

    fn avsz3(&mut self) {
        let zsf3 = self.ctrl[29] as i16 as i64;
        let sum = i64::from(self.data[17] as u16)
            + i64::from(self.data[18] as u16)
            + i64::from(self.data[19] as u16);
        let mac0 = zsf3 * sum;
        self.data[24] = mac0 as u32;
        let otz = (mac0 >> 12).clamp(0, 0xFFFF);
        self.data[7] = otz as u32;
    }

    fn avsz4(&mut self) {
        let zsf4 = self.ctrl[30] as i16 as i64;
        let sum = i64::from(self.data[16] as u16)
            + i64::from(self.data[17] as u16)
            + i64::from(self.data[18] as u16)
            + i64::from(self.data[19] as u16);
        let mac0 = zsf4 * sum;
        self.data[24] = mac0 as u32;
        let otz = (mac0 >> 12).clamp(0, 0xFFFF);
        self.data[7] = otz as u32;
    }

    fn sqr(&mut self, sf: u32) {
        for i in 0..3 {
            let ir = self.data[9 + i] as i16 as i64;
            let mac = (ir * ir) >> (sf * 12);
            self.data[25 + i] = mac as u32;
            self.set_ir(i, mac, false);
        }
    }

    fn op(&mut self, sf: u32, lm: bool) {
        let d1 = self.ctrl[0] as i16 as i64;
        let d2 = (self.ctrl[2] >> 16) as i16 as i64;
        let d3 = self.ctrl[4] as i16 as i64;
        let ir1 = self.data[9] as i16 as i64;
        let ir2 = self.data[10] as i16 as i64;
        let ir3 = self.data[11] as i16 as i64;
        let mac1 = (ir3 * d2 - ir2 * d3) >> (sf * 12);
        let mac2 = (ir1 * d3 - ir3 * d1) >> (sf * 12);
        let mac3 = (ir2 * d1 - ir1 * d2) >> (sf * 12);
        self.data[25] = mac1 as u32;
        self.data[26] = mac2 as u32;
        self.data[27] = mac3 as u32;
        self.set_ir(0, mac1, lm);
        self.set_ir(1, mac2, lm);
        self.set_ir(2, mac3, lm);
    }

    fn mvmva(&mut self, instr: u32, sf: u32, lm: bool) {
        let mx = ((instr >> 17) & 3) as usize;
        let v = ((instr >> 15) & 3) as usize;
        let cv = ((instr >> 13) & 3) as usize;
        let vec = match v {
            0..=2 => self.vx(v),
            _ => self.ir_vec(),
        };
        let (tx, ty, tz) = match cv {
            0 => (
                self.ctrl[5] as i32 as i64,
                self.ctrl[6] as i32 as i64,
                self.ctrl[7] as i32 as i64,
            ),
            1 => (
                self.ctrl[13] as i32 as i64,
                self.ctrl[14] as i32 as i64,
                self.ctrl[15] as i32 as i64,
            ),
            _ => (0, 0, 0),
        };
        for r in 0..3 {
            let tr = [tx, ty, tz][r] << 12;
            let m = tr
                + i64::from(self.mx_el(mx, r, 0)) * i64::from(vec.0)
                + i64::from(self.mx_el(mx, r, 1)) * i64::from(vec.1)
                + i64::from(self.mx_el(mx, r, 2)) * i64::from(vec.2);
            let shifted = m >> (sf * 12);
            self.data[25 + r] = shifted as u32;
            self.set_ir(r, shifted, lm);
        }
    }

    fn mx_el(&self, mx: usize, r: usize, c: usize) -> i32 {
        let base = match mx {
            0 => 0,
            1 => 8,
            2 => 16,
            _ => 0,
        };
        let idx = r * 3 + c;
        let word = base + idx / 2;
        let v = self.ctrl[word];
        if idx == 8 {
            v as i16 as i32
        } else if idx % 2 == 0 {
            v as i16 as i32
        } else {
            (v >> 16) as i16 as i32
        }
    }

    fn ncs(&mut self, sf: u32, lm: bool) {
        self.nc_vector(0, sf, lm);
    }

    fn nc_vector(&mut self, vec: usize, sf: u32, lm: bool) {
        let v = self.vx(vec);
        for r in 0..3 {
            let m = i64::from(self.mx_el(1, r, 0)) * i64::from(v.0)
                + i64::from(self.mx_el(1, r, 1)) * i64::from(v.1)
                + i64::from(self.mx_el(1, r, 2)) * i64::from(v.2);
            let shifted = m >> (sf * 12);
            self.data[25 + r] = shifted as u32;
            self.set_ir(r, shifted, lm);
        }
        let ir = self.ir_vec();
        for r in 0..3 {
            let bk = self.ctrl[13 + r] as i32 as i64;
            let m = (bk << 12)
                + i64::from(self.mx_el(2, r, 0)) * i64::from(ir.0)
                + i64::from(self.mx_el(2, r, 1)) * i64::from(ir.1)
                + i64::from(self.mx_el(2, r, 2)) * i64::from(ir.2);
            let shifted = m >> (sf * 12);
            self.data[25 + r] = shifted as u32;
            self.set_ir(r, shifted, lm);
        }
        self.push_color(lm);
    }

    fn ncds(&mut self, sf: u32, lm: bool) {
        self.ncd_vector(0, sf, lm);
    }

    fn ncd_vector(&mut self, vec: usize, sf: u32, lm: bool) {
        self.nc_vector(vec, sf, lm);
        let rgb = self.data[6];
        let r = (rgb & 0xFF) as i64;
        let g = ((rgb >> 8) & 0xFF) as i64;
        let b = ((rgb >> 16) & 0xFF) as i64;
        let ir = self.ir_vec();
        let mut mac = [
            (r * i64::from(ir.0)) << 4,
            (g * i64::from(ir.1)) << 4,
            (b * i64::from(ir.2)) << 4,
        ];
        self.depth_cue(&mut mac, sf);
        for i in 0..3 {
            let shifted = mac[i] >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn nccs(&mut self, sf: u32, lm: bool) {
        self.ncc_vector(0, sf, lm);
    }

    fn ncc_vector(&mut self, vec: usize, sf: u32, lm: bool) {
        self.nc_vector(vec, sf, lm);
        let rgb = self.data[6];
        let r = (rgb & 0xFF) as i64;
        let g = ((rgb >> 8) & 0xFF) as i64;
        let b = ((rgb >> 16) & 0xFF) as i64;
        let ir = self.ir_vec();
        for i in 0..3 {
            let c = [r, g, b][i];
            let mac = (c * i64::from([ir.0, ir.1, ir.2][i])) << 4;
            let shifted = mac >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn depth_cue(&mut self, mac: &mut [i64; 3], sf: u32) {
        let ir0 = self.data[8] as i16 as i64;
        for i in 0..3 {
            let fc = self.ctrl[21 + i] as i32 as i64;
            let tmp = ((fc << 12) - mac[i]) >> (sf * 12);
            let tmp = tmp.clamp(-0x8000, 0x7FFF);
            mac[i] += tmp * ir0;
        }
    }

    fn dpcs(&mut self, sf: u32, lm: bool) {
        let rgb = self.data[6];
        let mut mac = [
            ((rgb & 0xFF) as i64) << 16,
            (((rgb >> 8) & 0xFF) as i64) << 16,
            (((rgb >> 16) & 0xFF) as i64) << 16,
        ];
        self.depth_cue(&mut mac, sf);
        for i in 0..3 {
            let shifted = mac[i] >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn dpct_once(&mut self, sf: u32, lm: bool) {
        let rgb = self.data[20];
        let mut mac = [
            ((rgb & 0xFF) as i64) << 16,
            (((rgb >> 8) & 0xFF) as i64) << 16,
            (((rgb >> 16) & 0xFF) as i64) << 16,
        ];
        self.depth_cue(&mut mac, sf);
        for i in 0..3 {
            let shifted = mac[i] >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn intpl(&mut self, sf: u32, lm: bool) {
        let ir = self.ir_vec();
        let mut mac = [i64::from(ir.0) << 12, i64::from(ir.1) << 12, i64::from(ir.2) << 12];
        self.depth_cue(&mut mac, sf);
        for i in 0..3 {
            let shifted = mac[i] >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn dcpl(&mut self, sf: u32, lm: bool) {
        let rgb = self.data[6];
        let ir = self.ir_vec();
        let mut mac = [
            ((rgb & 0xFF) as i64 * i64::from(ir.0)) << 4,
            (((rgb >> 8) & 0xFF) as i64 * i64::from(ir.1)) << 4,
            (((rgb >> 16) & 0xFF) as i64 * i64::from(ir.2)) << 4,
        ];
        self.depth_cue(&mut mac, sf);
        for i in 0..3 {
            let shifted = mac[i] >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn gpf(&mut self, sf: u32, lm: bool) {
        let ir0 = self.data[8] as i16 as i64;
        for i in 0..3 {
            let ir = self.data[9 + i] as i16 as i64;
            let mac = (ir * ir0) >> (sf * 12);
            self.data[25 + i] = mac as u32;
            self.set_ir(i, mac, lm);
        }
        self.push_color(lm);
    }

    fn gpl(&mut self, sf: u32, lm: bool) {
        let ir0 = self.data[8] as i16 as i64;
        for i in 0..3 {
            let ir = self.data[9 + i] as i16 as i64;
            let mac_old = (self.data[25 + i] as i32 as i64) << (sf * 12);
            let mac = (ir * ir0 + mac_old) >> (sf * 12);
            self.data[25 + i] = mac as u32;
            self.set_ir(i, mac, lm);
        }
        self.push_color(lm);
    }

    fn push_color(&mut self, _lm: bool) {
        let sat8 = |mac: i32| {
            let v = mac / 16;
            v.clamp(0, 0xFF) as u32
        };
        let r = sat8(self.data[25] as i32);
        let g = sat8(self.data[26] as i32);
        let b = sat8(self.data[27] as i32);
        let code = (self.data[6] >> 24) & 0xFF;
        let rgb = r | (g << 8) | (b << 16) | (code << 24);
        self.data[20] = self.data[21];
        self.data[21] = self.data[22];
        self.data[22] = rgb;
    }
}

fn sign16(v: u16) -> i32 {
    v as i16 as i32
}

fn saturate_sx(v: i64, flag: &mut u32) -> i32 {
    if v < -0x400 {
        *flag |= 1 << 14;
        -0x400
    } else if v > 0x3FF {
        *flag |= 1 << 14;
        0x3FF
    } else {
        v as i32
    }
}

fn saturate_sy(v: i64, flag: &mut u32) -> i32 {
    if v < -0x400 {
        *flag |= 1 << 13;
        -0x400
    } else if v > 0x3FF {
        *flag |= 1 << 13;
        0x3FF
    } else {
        v as i32
    }
}

fn unr_divide(h: u16, sz: u16, flag: &mut u32) -> u32 {
    if sz == 0 || u32::from(h) >= u32::from(sz) * 2 {
        *flag |= (1 << 17) | (1 << 31);
        return 0x1FFFF;
    }
    // Accurate enough for Intro: integer divide with SPX saturation.
    let n = ((u32::from(h) << 16) + u32::from(sz) / 2) / u32::from(sz);
    n.min(0x1FFFF)
}
