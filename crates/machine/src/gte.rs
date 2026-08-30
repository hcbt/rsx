//! Geometry Transformation Engine (COP2), from PSX-SPX formulas.

#[derive(Clone)]
pub struct Gte {
    data: [u32; 32],
    ctrl: [u32; 32],
    pub last_hi_sy: i32,
    pub last_hi_ir2: i32,
    pub last_hi_n: u32,
    pub last_hi_sz: u32,
    pub last_hi_vy: i32,
    pub last_hi_try: i32,
    pub last_hi_trz: i32,
    pub last_hi_r21: i32,
    pub last_hi_r22: i32,
    pub last_hi_r23: i32,
    pub last_hi_rt: [i32; 9],
    pub title_explode: u32,
    pub title_ir2_min: i32,
    pub title_ir2_max: i32,
    pub title_vy_min: i32,
    pub title_vy_max: i32,
    pub op_counts: [u32; 64],
    /// Last completed frame (copied on vblank). Object-like = yaw R with 5/8 Y.
    pub frame_rtps: u32,
    pub frame_obj_n: u32,
    pub frame_obj_sy_min: i32,
    pub frame_obj_sy_max: i32,
    pub frame_obj_vy_min: i32,
    pub frame_obj_vy_max: i32,
    pub frame_obj_try: i32,
    pub frame_obj_trz: i32,
    pub frame_obj_vx_min: i32,
    pub frame_obj_vx_max: i32,
    acc_rtps: u32,
    acc_obj_n: u32,
    acc_obj_sy_min: i32,
    acc_obj_sy_max: i32,
    acc_obj_vy_min: i32,
    acc_obj_vy_max: i32,
    acc_obj_try: i32,
    acc_obj_trz: i32,
    acc_obj_vx_min: i32,
    acc_obj_vx_max: i32,
}

impl Gte {
    pub fn new() -> Self {
        Self {
            data: [0; 32],
            ctrl: [0; 32],
            last_hi_sy: 0,
            last_hi_ir2: 0,
            last_hi_n: 0,
            last_hi_sz: 0,
            last_hi_vy: 0,
            last_hi_try: 0,
            last_hi_trz: 0,
            last_hi_r21: 0,
            last_hi_r22: 0,
            last_hi_r23: 0,
            last_hi_rt: [0; 9],
            title_explode: 0,
            title_ir2_min: i32::MAX,
            title_ir2_max: i32::MIN,
            title_vy_min: i32::MAX,
            title_vy_max: i32::MIN,
            op_counts: [0; 64],
            frame_rtps: 0,
            frame_obj_n: 0,
            frame_obj_sy_min: i32::MAX,
            frame_obj_sy_max: i32::MIN,
            frame_obj_vy_min: i32::MAX,
            frame_obj_vy_max: i32::MIN,
            frame_obj_try: 0,
            frame_obj_trz: 0,
            frame_obj_vx_min: i32::MAX,
            frame_obj_vx_max: i32::MIN,
            acc_rtps: 0,
            acc_obj_n: 0,
            acc_obj_sy_min: i32::MAX,
            acc_obj_sy_max: i32::MIN,
            acc_obj_vy_min: i32::MAX,
            acc_obj_vy_max: i32::MIN,
            acc_obj_try: 0,
            acc_obj_trz: 0,
            acc_obj_vx_min: i32::MAX,
            acc_obj_vx_max: i32::MIN,
        }
    }

    pub fn on_vblank(&mut self) {
        self.frame_rtps = self.acc_rtps;
        self.frame_obj_n = self.acc_obj_n;
        self.frame_obj_sy_min = self.acc_obj_sy_min;
        self.frame_obj_sy_max = self.acc_obj_sy_max;
        self.frame_obj_vy_min = self.acc_obj_vy_min;
        self.frame_obj_vy_max = self.acc_obj_vy_max;
        self.frame_obj_try = self.acc_obj_try;
        self.frame_obj_trz = self.acc_obj_trz;
        self.frame_obj_vx_min = self.acc_obj_vx_min;
        self.frame_obj_vx_max = self.acc_obj_vx_max;
        self.acc_rtps = 0;
        self.acc_obj_n = 0;
        self.acc_obj_sy_min = i32::MAX;
        self.acc_obj_sy_max = i32::MIN;
        self.acc_obj_vy_min = i32::MAX;
        self.acc_obj_vy_max = i32::MIN;
        self.acc_obj_try = 0;
        self.acc_obj_trz = 0;
        self.acc_obj_vx_min = i32::MAX;
        self.acc_obj_vx_max = i32::MIN;
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
            1 | 3 | 5 | 8 | 9 | 10 | 11 => {
                self.data[reg as usize] = sign16(value as u16) as u32;
            }
            7 | 16 | 17 | 18 | 19 => self.data[reg as usize] = value & 0xFFFF,
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
        self.ctrl[reg as usize] = match reg {
            4 | 12 | 20 | 26 | 27 | 29 | 30 => sign16(value as u16) as u32,
            _ => value,
        };
    }

    pub fn command(&mut self, instr: u32) {
        let op = instr & 0x3F;
        let sf = (instr >> 19) & 1;
        let lm = (instr >> 10) & 1 != 0;
        self.ctrl[31] = 0;
        self.op_counts[op as usize] = self.op_counts[op as usize].saturating_add(1);
        match op {
            0x01 => self.rtps(sf, lm),
            0x06 => self.nclip(),
            0x0C => self.op(sf, lm),
            0x10 => self.dpcs(sf, lm),
            0x11 => self.intpl(sf, lm),
            0x12 => self.mvmva(instr, sf, lm),
            0x13 => self.ncds(sf, lm),
            0x14 => self.cdp(sf, lm),
            0x16 => {
                self.ncds(sf, lm);
                self.ncd_vector(1, sf, lm);
                self.ncd_vector(2, sf, lm);
            }
            0x1B => self.nccs(sf, lm),
            0x1C => self.cc(sf, lm),
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

    /// 44-bit MAC1/2/3 (or 32-bit MAC0): flag A1/A2/A3/A0 then sign-extend.
    fn mac(&mut self, axis: u32, v: i64) -> i64 {
        if axis == 0 {
            if v > i64::from(i32::MAX) || v < i64::from(i32::MIN) {
                self.flag(16);
            }
            (v << 32) >> 32
        } else {
            if v > (1i64 << 43) - 1 || v < -(1i64 << 43) {
                self.flag(31 - axis);
            }
            (v << 20) >> 20
        }
    }

    fn rtp_vector(&mut self, vec: usize, sf: u32, lm: bool) {
        let (vx, vy, vz) = self.vx(vec);
        let tr = [
            i64::from(self.ctrl[5] as i32),
            i64::from(self.ctrl[6] as i32),
            i64::from(self.ctrl[7] as i32),
        ];
        let mut xyz = [0i64; 3];
        for r in 0..3 {
            // SPX/hardware: each partial sum is 44-bit (sign-extended) before
            // the next multiply-add. DuckStation's RTPS matches this chaining.
            let axis = (r as u32) + 1;
            let mut acc = self.mac(
                axis,
                (tr[r] << 12) + i64::from(self.rt_el(r, 0)) * i64::from(vx),
            );
            acc = self.mac(axis, acc + i64::from(self.rt_el(r, 1)) * i64::from(vy));
            xyz[r] = acc + i64::from(self.rt_el(r, 2)) * i64::from(vz);
            self.mac(axis, xyz[r]);
        }
        let shift = sf * 12;
        for i in 0..2 {
            let shifted = xyz[i] >> shift;
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        let mac3 = xyz[2] >> shift;
        self.data[27] = mac3 as u32;
        // FLAG.22 uses MAC3 SAR 12 as if lm=0; stored IR3 uses MAC3 and the
        // actual lm bit (SPX RTPS note).
        self.set_ir(2, xyz[2] >> 12, false);
        let ir3 = if lm {
            mac3.clamp(0, 0x7FFF)
        } else {
            mac3.clamp(-0x8000, 0x7FFF)
        };
        self.data[11] = ir3 as u32;
        let sz_raw = xyz[2] >> 12;
        if !(0..=0xFFFF).contains(&sz_raw) {
            self.flag(18);
        }
        let sz = sz_raw.clamp(0, 0xFFFF) as u32;
        self.data[16] = self.data[17];
        self.data[17] = self.data[18];
        self.data[18] = self.data[19];
        self.data[19] = sz;
        let n = unr_divide(self.ctrl[26] as u16, sz as u16, &mut self.ctrl[31]);
        if self.ctrl[26] as u16 == 0x1F4 && n == 0x1FFFF {
            self.title_explode += 1;
        }
        let ofx = self.ctrl[24] as i32 as i64;
        let ofy = self.ctrl[25] as i32 as i64;
        let ir1 = self.data[9] as i16 as i64;
        let ir2 = self.data[10] as i16 as i64;
        let sx = ((i64::from(n) * ir1) + ofx) >> 16;
        let sy = ((i64::from(n) * ir2) + ofy) >> 16;
        let sx = saturate_sx(sx, &mut self.ctrl[31]);
        let sy = saturate_sy(sy, &mut self.ctrl[31]);
        if self.ctrl[26] as u16 == 0x1F4 {
            self.acc_rtps = self.acc_rtps.saturating_add(1);
            self.title_ir2_min = self.title_ir2_min.min(ir2 as i32);
            self.title_ir2_max = self.title_ir2_max.max(ir2 as i32);
            self.title_vy_min = self.title_vy_min.min(vy);
            self.title_vy_max = self.title_vy_max.max(vy);
            let r12 = self.rt_el(0, 1);
            let r21 = self.rt_el(1, 0);
            let r22 = self.rt_el(1, 1);
            let r23 = self.rt_el(1, 2);
            let r32 = self.rt_el(2, 1);
            // Crash object R is yaw + 5/8 Y + Z negate: off-axis Y terms ≈ 0.
            let object = r12.abs() < 200
                && r21.abs() < 200
                && r23.abs() < 200
                && r32.abs() < 200
                && r22 < -1000;
            if object {
                self.acc_obj_n = self.acc_obj_n.saturating_add(1);
                self.acc_obj_sy_min = self.acc_obj_sy_min.min(sy);
                self.acc_obj_vy_min = self.acc_obj_vy_min.min(vy);
                self.acc_obj_vy_max = self.acc_obj_vy_max.max(vy);
                self.acc_obj_vx_min = self.acc_obj_vx_min.min(vx);
                self.acc_obj_vx_max = self.acc_obj_vx_max.max(vx);
                if sy >= self.acc_obj_sy_max {
                    self.acc_obj_sy_max = sy;
                    self.acc_obj_try = tr[1] as i32;
                    self.acc_obj_trz = tr[2] as i32;
                }
            }
            if sy > self.last_hi_sy {
                let rt = [
                    self.rt_el(0, 0),
                    self.rt_el(0, 1),
                    self.rt_el(0, 2),
                    self.rt_el(1, 0),
                    self.rt_el(1, 1),
                    self.rt_el(1, 2),
                    self.rt_el(2, 0),
                    self.rt_el(2, 1),
                    self.rt_el(2, 2),
                ];
                self.last_hi_sy = sy;
                self.last_hi_ir2 = ir2 as i32;
                self.last_hi_n = n;
                self.last_hi_sz = sz;
                self.last_hi_vy = vy;
                self.last_hi_try = tr[1] as i32;
                self.last_hi_trz = tr[2] as i32;
                self.last_hi_r21 = rt[3];
                self.last_hi_r22 = rt[4];
                self.last_hi_r23 = rt[5];
                self.last_hi_rt = rt;
            }
        }
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
        self.data[24] = self.mac(0, mac0) as u32;
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
        let m = if mx == 3 {
            let r = (self.data[6] & 0xFF) as i32 * 0x10;
            let ir0 = self.data[8] as i16 as i32;
            let rt13 = self.rt_el(0, 2);
            let rt22 = self.rt_el(1, 1);
            [[-r, r, ir0], [rt13, rt13, rt13], [rt22, rt22, rt22]]
        } else {
            [
                [self.mx_el(mx, 0, 0), self.mx_el(mx, 0, 1), self.mx_el(mx, 0, 2)],
                [self.mx_el(mx, 1, 0), self.mx_el(mx, 1, 1), self.mx_el(mx, 1, 2)],
                [self.mx_el(mx, 2, 0), self.mx_el(mx, 2, 1), self.mx_el(mx, 2, 2)],
            ]
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
            2 => (
                self.ctrl[21] as i32 as i64,
                self.ctrl[22] as i32 as i64,
                self.ctrl[23] as i32 as i64,
            ),
            _ => (0, 0, 0),
        };
        let shift = sf * 12;
        for r in 0..3 {
            let t = [tx, ty, tz][r];
            let axis = (r as u32) + 1;
            if cv == 2 {
                // SPX: FC translation is bugged — FLAG uses T+M*Vx, result is
                // only the last two multiply-adds.
                let _flag = self.mac(axis, (t << 12) + i64::from(m[r][0]) * i64::from(vec.0));
                let acc = self.mac(
                    axis,
                    i64::from(m[r][1]) * i64::from(vec.1) + i64::from(m[r][2]) * i64::from(vec.2),
                );
                let shifted = acc >> shift;
                self.data[25 + r] = shifted as u32;
                self.set_ir(r, shifted, lm);
            } else {
                let mut acc = self.mac(axis, (t << 12) + i64::from(m[r][0]) * i64::from(vec.0));
                acc = self.mac(axis, acc + i64::from(m[r][1]) * i64::from(vec.1));
                acc += i64::from(m[r][2]) * i64::from(vec.2);
                self.mac(axis, acc);
                let shifted = acc >> shift;
                self.data[25 + r] = shifted as u32;
                self.set_ir(r, shifted, lm);
            }
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

    fn lcm_ir_bk(&mut self, sf: u32, lm: bool) {
        let ir = self.ir_vec();
        for r in 0..3 {
            let bk = self.ctrl[13 + r] as i32 as i64;
            let axis = (r as u32) + 1;
            let mut acc =
                self.mac(axis, (bk << 12) + i64::from(self.mx_el(2, r, 0)) * i64::from(ir.0));
            acc = self.mac(axis, acc + i64::from(self.mx_el(2, r, 1)) * i64::from(ir.1));
            acc += i64::from(self.mx_el(2, r, 2)) * i64::from(ir.2);
            self.mac(axis, acc);
            let shifted = acc >> (sf * 12);
            self.data[25 + r] = shifted as u32;
            self.set_ir(r, shifted, lm);
        }
    }

    fn cc(&mut self, sf: u32, lm: bool) {
        self.lcm_ir_bk(sf, lm);
        let rgb = self.data[6];
        let r = (rgb & 0xFF) as i64;
        let g = ((rgb >> 8) & 0xFF) as i64;
        let b = ((rgb >> 16) & 0xFF) as i64;
        let ir = self.ir_vec();
        for i in 0..3 {
            let mac = ([r, g, b][i] * i64::from([ir.0, ir.1, ir.2][i])) << 4;
            let shifted = mac >> (sf * 12);
            self.data[25 + i] = shifted as u32;
            self.set_ir(i, shifted, lm);
        }
        self.push_color(lm);
    }

    fn cdp(&mut self, sf: u32, lm: bool) {
        self.lcm_ir_bk(sf, lm);
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
        let mut mac = [
            i64::from(ir.0) << 12,
            i64::from(ir.1) << 12,
            i64::from(ir.2) << 12,
        ];
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

#[cfg(test)]
mod tests {
    use super::*;

    fn identity_rtps(sf: bool) -> Gte {
        let mut g = Gte::new();
        // RT = identity in 3.12 (0x1000 = 1.0).
        g.write_control(0, 0x1000);
        g.write_control(2, 0x1000);
        g.write_control(4, 0x1000);
        g.write_control(7, 5888); // TRZ
        g.write_control(24, 320 << 16); // OFX
        g.write_control(25, 240 << 16); // OFY
        g.write_control(26, 0x200); // H
        g.write_data(0, 0);
        g.write_data(1, 0);
        let cmd = 0x01 | if sf { 1 << 19 } else { 0 };
        g.command(cmd);
        g
    }

    #[test]
    fn rtps_sz3_is_mac3_after_the_12bit_shift() {
        // SPX: IR3 = MAC3 = (TRZ*1000h + …) SAR (sf*12);
        //      SZ3 = MAC3 SAR ((1-sf)*12). With V=0 that is TRZ, not TRZ<<12.
        let g = identity_rtps(true);
        let sz3 = g.read_data(19) & 0xFFFF;
        assert_eq!(
            sz3,
            5888,
            "RTPS sf=1 SZ3 must be TRZ (got {sz3:#X}, FLAG={:08X})",
            g.read_control(31)
        );
        let g0 = identity_rtps(false);
        let sz0 = g0.read_data(19) & 0xFFFF;
        assert_eq!(sz0, 5888, "RTPS sf=0 SZ3 must also be TRZ (got {sz0:#X})");
    }

    #[test]
    fn rtps_screen_y_is_n_times_ir2_plus_ofy() {
        // Identity RT, VY=80, TRZ=H=500, OFY=0, sf=1 → SY ≈ 80 (1:1 at the
        // projection plane). A 2× scale here would put title Crash at Y=422.
        let mut g = Gte::new();
        g.write_control(0, 0x1000);
        g.write_control(2, 0x1000);
        g.write_control(4, 0x1000);
        g.write_control(7, 500);
        g.write_control(24, 0);
        g.write_control(25, 0);
        g.write_control(26, 500);
        g.write_data(0, 80u32 << 16); // VY=80
        g.write_data(1, 0);
        g.command(0x01 | (1 << 19));
        let sy = (g.read_data(14) >> 16) as i16;
        assert_eq!(g.read_data(19) & 0xFFFF, 500, "SZ3");
        assert!(
            (sy - 80).abs() <= 1,
            "RTPS SY must be ~VY when H=SZ (got SY={sy}, FLAG={:08X})",
            g.read_control(31)
        );
    }

    #[test]
    fn perspective_divide_uses_unr_not_integer_reciprocal() {
        let mut flag = 0;
        // SPX: FE3Fh/7F20h UNR-saturates to 1FFFFh without FLAG.17.
        // Naive 32-bit reciprocal is 1FFFEh.
        assert_eq!(unr_divide(0xFE3F, 0x7F20, &mut flag), 0x1FFFF);
        assert_eq!(flag, 0, "UNR sat to 1FFFFh must not set divide overflow");
        flag = 0;
        assert_eq!(unr_divide(0x200, 0x100, &mut flag), 0x1FFFF);
        assert_ne!(flag & (1 << 17), 0, "H >= SZ*2 saturates and sets FLAG.17");
    }

    #[test]
    fn avsz3_otz_is_zsf3_times_sz_sum_shifted_12() {
        let mut g = Gte::new();
        g.write_data(17, 0x0100);
        g.write_data(18, 0x0200);
        g.write_data(19, 0x0300);
        g.write_control(29, 0x1000);
        g.command(0x2D);
        assert_eq!(
            g.read_data(7),
            0x0600,
            "AVSZ3 OTZ = ZSF3*(SZ1+SZ2+SZ3)>>12 (got {:#X})",
            g.read_data(7)
        );
    }

    #[test]
    fn avsz4_otz_is_zsf4_times_sz_sum_shifted_12() {
        let mut g = Gte::new();
        g.write_data(16, 0x0100);
        g.write_data(17, 0x0100);
        g.write_data(18, 0x0100);
        g.write_data(19, 0x0100);
        g.write_control(30, 0x1000);
        g.command(0x2E);
        assert_eq!(
            g.read_data(7),
            0x0400,
            "AVSZ4 OTZ = ZSF4*(SZ0+SZ1+SZ2+SZ3)>>12 (got {:#X})",
            g.read_data(7)
        );
    }

    #[test]
    fn mvmva_cv_none_does_not_add_translation() {
        // PSY-Q rtir12: mx=RT, v=IR, cv=None, sf=1. TR must not leak into IR.
        let mut g = Gte::new();
        g.write_control(0, 0x1000); // R11=1
        g.write_control(2, 0x1000); // R22=1
        g.write_control(4, 0x1000); // R33=1
        g.write_control(5, 100);
        g.write_control(6, 200);
        g.write_control(7, 300);
        g.write_data(9, 0x1000);
        g.write_data(10, 0x1000);
        g.write_data(11, 0x1000);
        // sf=1, mx=0, v=3 (IR), cv=3 (none), op=MVMVA
        g.command(0x01 << 19 | 3 << 15 | 3 << 13 | 0x12);
        assert_eq!(g.read_data(9) as i16, 0x1000, "IR1");
        assert_eq!(g.read_data(10) as i16, 0x1000, "IR2");
        assert_eq!(g.read_data(11) as i16, 0x1000, "IR3");
    }

    #[test]
    fn mvmva_mx3_uses_garbage_matrix() {
        // SPX: mx=3 is -R*10h, +R*10h, IR0 / RT13,RT13,RT13 / RT22,RT22,RT22.
        let mut g = Gte::new();
        g.write_data(6, 0x10); // R=16 → ±0x100
        g.write_data(8, 0x20); // IR0
        g.write_control(1, 0x0003); // RT13=3
        g.write_control(2, 0x0004); // RT22=4
        g.write_data(0, 1 | (1 << 16)); // V0 = (1,1,?)
        g.write_data(1, 1);
        g.command(0x01 << 19 | 3 << 17 | 0x12); // sf=1, mx=3, v=V0, cv=TR=0
        // MAC1 = ((-0x100)*1 + 0x100*1 + 0x20*1) >> 12 = 0x20 >> 12 = 0
        assert_eq!(g.read_data(9) as i16, 0, "IR1 garbage row0");
        // MAC2 = (3+3+3)>>12 = 0
        assert_eq!(g.read_data(10) as i16, 0, "IR2 garbage row1");
        // MAC3 = (4+4+4)>>12 = 0
        assert_eq!(g.read_data(11) as i16, 0, "IR3 garbage row2");
    }

    #[test]
    fn cc_multiplies_lcm_ir_then_rgb() {
        let mut g = Gte::new();
        // LCM = identity 0x1000, BK=0, IR=1.0, RGB=0x80 → FIFO 0x80.
        g.write_control(16, 0x1000);
        g.write_control(18, 0x1000);
        g.write_control(20, 0x1000);
        g.write_data(9, 0x1000);
        g.write_data(10, 0x1000);
        g.write_data(11, 0x1000);
        g.write_data(6, 0x80 | (0x80 << 8) | (0x80 << 16) | (0x30 << 24));
        g.command(0x01 << 19 | 1 << 10 | 0x1C); // sf=1 lm=1 CC
        let rgb = g.read_data(22);
        assert_eq!(rgb & 0xFF, 0x80, "CC R {rgb:#X}");
        assert_eq!((rgb >> 8) & 0xFF, 0x80, "CC G");
        assert_eq!((rgb >> 16) & 0xFF, 0x80, "CC B");
        assert_eq!(rgb >> 24, 0x30, "CC CODE");
    }

    #[test]
    fn rtps_mac_overflow_sets_flag_a3() {
        // TRZ at i32::MAX, R33=VZ=0x7FFF: MAC3 exceeds 43 bits → FLAG.28 and bit 31.
        let mut g = Gte::new();
        g.write_control(4, 0x7FFF);
        g.write_control(7, 0x7FFF_FFFF);
        g.write_data(1, 0x7FFF);
        g.command(0x01 | (1 << 19));
        let f = g.read_control(31);
        assert_ne!(f & (1 << 28), 0, "A3 FLAG.28");
        assert_ne!(f & (1 << 31), 0, "error FLAG.31");
    }

    #[test]
    fn mvmva_rtv0_identity_is_the_vector() {
        // PSY-Q rtv0: sf=1 mx=RT v=V0 cv=None. Identity RT, V=(100,-50,25) → IR=V.
        let mut g = Gte::new();
        g.write_control(0, 0x1000);
        g.write_control(2, 0x1000);
        g.write_control(4, 0x1000);
        g.write_data(0, 100u32 | ((-50i16 as u16 as u32) << 16));
        g.write_data(1, 25);
        g.command(0x01 << 19 | 3 << 13 | 0x12); // sf=1, cv=None, v=V0
        assert_eq!(g.read_data(9) as i16, 100, "IR1");
        assert_eq!(g.read_data(10) as i16, -50, "IR2");
        assert_eq!(g.read_data(11) as i16, 25, "IR3");
    }
}

/// SPX UNR table, 000h..100h: max(0, (40000h/(i+100h)+1)/2 - 101h).
const UNR_TABLE: [u8; 257] = [
    0xFF, 0xFD, 0xFB, 0xF9, 0xF7, 0xF5, 0xF3, 0xF1, 0xEF, 0xEE, 0xEC, 0xEA, 0xE8, 0xE6, 0xE4, 0xE3,
    0xE1, 0xDF, 0xDD, 0xDC, 0xDA, 0xD8, 0xD6, 0xD5, 0xD3, 0xD1, 0xD0, 0xCE, 0xCD, 0xCB, 0xC9, 0xC8,
    0xC6, 0xC5, 0xC3, 0xC1, 0xC0, 0xBE, 0xBD, 0xBB, 0xBA, 0xB8, 0xB7, 0xB5, 0xB4, 0xB2, 0xB1, 0xB0,
    0xAE, 0xAD, 0xAB, 0xAA, 0xA9, 0xA7, 0xA6, 0xA4, 0xA3, 0xA2, 0xA0, 0x9F, 0x9E, 0x9C, 0x9B, 0x9A,
    0x99, 0x97, 0x96, 0x95, 0x94, 0x92, 0x91, 0x90, 0x8F, 0x8D, 0x8C, 0x8B, 0x8A, 0x89, 0x87, 0x86,
    0x85, 0x84, 0x83, 0x82, 0x81, 0x7F, 0x7E, 0x7D, 0x7C, 0x7B, 0x7A, 0x79, 0x78, 0x77, 0x75, 0x74,
    0x73, 0x72, 0x71, 0x70, 0x6F, 0x6E, 0x6D, 0x6C, 0x6B, 0x6A, 0x69, 0x68, 0x67, 0x66, 0x65, 0x64,
    0x63, 0x62, 0x61, 0x60, 0x5F, 0x5E, 0x5D, 0x5D, 0x5C, 0x5B, 0x5A, 0x59, 0x58, 0x57, 0x56, 0x55,
    0x54, 0x53, 0x53, 0x52, 0x51, 0x50, 0x4F, 0x4E, 0x4D, 0x4D, 0x4C, 0x4B, 0x4A, 0x49, 0x48, 0x48,
    0x47, 0x46, 0x45, 0x44, 0x43, 0x43, 0x42, 0x41, 0x40, 0x3F, 0x3F, 0x3E, 0x3D, 0x3C, 0x3C, 0x3B,
    0x3A, 0x39, 0x39, 0x38, 0x37, 0x36, 0x36, 0x35, 0x34, 0x33, 0x33, 0x32, 0x31, 0x31, 0x30, 0x2F,
    0x2E, 0x2E, 0x2D, 0x2C, 0x2C, 0x2B, 0x2A, 0x2A, 0x29, 0x28, 0x28, 0x27, 0x26, 0x26, 0x25, 0x24,
    0x24, 0x23, 0x22, 0x22, 0x21, 0x20, 0x20, 0x1F, 0x1E, 0x1E, 0x1D, 0x1D, 0x1C, 0x1B, 0x1B, 0x1A,
    0x19, 0x19, 0x18, 0x18, 0x17, 0x16, 0x16, 0x15, 0x15, 0x14, 0x14, 0x13, 0x12, 0x12, 0x11, 0x11,
    0x10, 0x0F, 0x0F, 0x0E, 0x0E, 0x0D, 0x0D, 0x0C, 0x0C, 0x0B, 0x0A, 0x0A, 0x09, 0x09, 0x08, 0x08,
    0x07, 0x07, 0x06, 0x06, 0x05, 0x05, 0x04, 0x04, 0x03, 0x03, 0x02, 0x02, 0x01, 0x01, 0x00, 0x00,
    0x00,
];

fn unr_divide(h: u16, sz: u16, flag: &mut u32) -> u32 {
    if u32::from(h) >= u32::from(sz) * 2 {
        *flag |= (1 << 17) | (1 << 31);
        return 0x1FFFF;
    }
    let z = sz.leading_zeros();
    let n = u64::from(h) << z;
    let d = u64::from(sz) << z;
    let u = u64::from(UNR_TABLE[((d - 0x7FC0) >> 7) as usize]) + 0x101;
    let d = (0x200_0080 - d * u) >> 8;
    let d = (0x80 + d * u) >> 8;
    (((n * d) + 0x8000) >> 16).min(0x1FFFF) as u32
}
