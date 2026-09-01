//! SPU: 24 ADPCM voices, mix, 44100 Hz stereo. Host playback is the Debugger.

include!("spu_gauss.rs");

const SPU_RAM: usize = 512 * 1024;
const CYCLES_PER_SAMPLE: u32 = 768;
const VOICES: usize = 24;

const FILTER: [[i32; 2]; 5] = [[0, 0], [60, 0], [115, -52], [98, -55], [122, -60]];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone)]
struct Voice {
    current_addr: u32,
    repeat_addr: u32,
    pitch_counter: u32,
    adsr_level: i32,
    adsr_counter: u32,
    phase: Phase,
    hist: [i32; 2],
    gauss: [i16; 4],
    decoded: [i16; 28],
    decode_idx: usize,
    end_after: bool,
    loop_repeat: bool,
    started: bool,
    sweep_level: i32,
    sweep_counter: u32,
}

impl Voice {
    fn new() -> Self {
        Self {
            current_addr: 0,
            repeat_addr: 0,
            pitch_counter: 0,
            adsr_level: 0,
            adsr_counter: 0,
            phase: Phase::Off,
            hist: [0; 2],
            gauss: [0; 4],
            decoded: [0; 28],
            decode_idx: 28,
            end_after: false,
            loop_repeat: false,
            started: false,
            sweep_level: 0,
            sweep_counter: 0,
        }
    }
}

/// SPU that answers the BIOS and mixes 24 voices into host PCM.
pub struct Spu {
    ram: Vec<u8>,
    regs: [u16; 0x200],
    transfer_addr: u32,
    applied_mode: u16,
    apply_delay: u32,
    transfer_busy: u32,
    voices: [Voice; VOICES],
    endx: u32,
    cycle_accum: u32,
    samples: Vec<i16>,
    main_sweep: [i32; 2],
    main_sweep_ctr: [u32; 2],
    irq_pending: bool,
    cd_in: (i16, i16),
    ext_in: (i16, i16),
    reverb_addr: u32,
    capture_i: usize,
}

impl Spu {
    pub fn new() -> Self {
        Self {
            ram: vec![0; SPU_RAM],
            regs: [0; 0x200],
            transfer_addr: 0,
            applied_mode: 0,
            apply_delay: 0,
            transfer_busy: 0,
            voices: core::array::from_fn(|_| Voice::new()),
            endx: 0,
            cycle_accum: 0,
            samples: Vec::new(),
            main_sweep: [0; 2],
            main_sweep_ctr: [0; 2],
            irq_pending: false,
            cd_in: (0, 0),
            ext_in: (0, 0),
            reverb_addr: 0,
            capture_i: 0,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn tick(&mut self, cycles: u32) {
        if self.apply_delay > 0 {
            if cycles >= self.apply_delay {
                self.apply_delay = 0;
                self.applied_mode = self.spucnt() & 0x3F;
                let mode = (self.applied_mode >> 4) & 3;
                if mode == 1 {
                    self.transfer_busy = 0x80;
                }
            } else {
                self.apply_delay -= cycles;
            }
        }
        if self.transfer_busy > 0 {
            self.transfer_busy = self.transfer_busy.saturating_sub(cycles);
        }
        self.cycle_accum += cycles;
        while self.cycle_accum >= CYCLES_PER_SAMPLE {
            self.cycle_accum -= CYCLES_PER_SAMPLE;
            self.mix_sample();
        }
    }

    pub fn take_samples(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.samples)
    }

    pub fn ram16(&self, addr: u32) -> u16 {
        let a = (addr as usize) & (SPU_RAM - 1) & !1;
        u16::from_le_bytes([self.ram[a], self.ram[a + 1]])
    }

    pub fn take_irq(&mut self) -> bool {
        let v = self.irq_pending;
        self.irq_pending = false;
        v
    }

    fn spucnt(&self) -> u16 {
        self.regs[0xD5]
    }

    pub fn read16(&self, addr: u32) -> u16 {
        let off = ((addr - 0x1F80_1C00) >> 1) as usize;
        if off == 0xD7 {
            return self.stat();
        }
        if off == 0xCE {
            return self.endx as u16;
        }
        if off == 0xCF {
            return (self.endx >> 16) as u16;
        }
        // Current ADSR volume.
        if off < 24 * 8 && off % 8 == 6 {
            return self.voices[off / 8].adsr_level.clamp(0, 0x7FFF) as u16;
        }
        self.regs.get(off).copied().unwrap_or(0)
    }

    fn stat(&self) -> u16 {
        let mut s = self.applied_mode & 0x3F;
        let mode = (self.applied_mode >> 4) & 3;
        if mode == 2 {
            s |= 1 << 8;
        }
        if mode == 3 {
            s |= 1 << 9;
        }
        if mode == 2 || mode == 3 {
            s |= 1 << 7;
        }
        if self.transfer_busy > 0 {
            s |= 1 << 10;
        }
        if self.irq_pending {
            s |= 1 << 6;
        }
        s
    }

    pub fn write16(&mut self, addr: u32, value: u16) {
        let off = ((addr - 0x1F80_1C00) >> 1) as usize;
        if off == 0xD3 {
            self.transfer_addr = u32::from(value) << 3;
        } else if off == 0xD4 {
            self.dma_write16(value);
        } else if off == 0xD5 {
            self.regs[0xD5] = value;
            self.applied_mode = value & 0x3F;
            if (value >> 4) & 3 == 1 {
                self.transfer_busy = 0x80;
            }
            if value & (1 << 6) == 0 {
                self.irq_pending = false;
            }
            return;
        } else if off == 0xC4 {
            self.key_on(u32::from(value));
        } else if off == 0xC5 {
            self.key_on(u32::from(value) << 16);
        } else if off == 0xC6 {
            self.key_off(u32::from(value));
        } else if off == 0xC7 {
            self.key_off(u32::from(value) << 16);
        }
        if off < self.regs.len() {
            self.regs[off] = value;
        }
        if off < 24 * 8 && off % 8 == 6 {
            let v = &mut self.voices[off / 8];
            v.adsr_level = i32::from(value as i16).clamp(0, 0x7FFF);
        }
        if off < 24 * 8 && off % 8 == 7 {
            self.voices[off / 8].repeat_addr = u32::from(value) << 3;
        }
    }

    pub fn dma_write16(&mut self, value: u16) {
        let a = (self.transfer_addr as usize) & (self.ram.len() - 1) & !1;
        self.ram[a..a + 2].copy_from_slice(&value.to_le_bytes());
        self.hit_irq(self.transfer_addr);
        self.transfer_addr = self.transfer_addr.wrapping_add(2);
    }

    fn key_on(&mut self, bits: u32) {
        for i in 0..VOICES {
            if bits & (1 << i) == 0 {
                continue;
            }
            let start = u32::from(self.regs[i * 8 + 3]) << 3;
            let v = &mut self.voices[i];
            *v = Voice::new();
            v.current_addr = start;
            v.repeat_addr = start;
            v.phase = Phase::Attack;
            v.started = true;
            v.decode_idx = 28;
            self.endx &= !(1 << i);
        }
    }

    fn key_off(&mut self, bits: u32) {
        for i in 0..VOICES {
            if bits & (1 << i) == 0 {
                continue;
            }
            let v = &mut self.voices[i];
            if v.phase != Phase::Off {
                v.phase = Phase::Release;
                v.adsr_counter = 0;
            }
        }
    }

    fn hit_irq(&mut self, addr: u32) {
        if self.spucnt() & (1 << 6) == 0 || self.spucnt() & (1 << 15) == 0 {
            return;
        }
        let irq_addr = u32::from(self.regs[0xD2]) << 3;
        if (addr & !0xF) == (irq_addr & !0xF) {
            self.irq_pending = true;
        }
    }

    pub fn feed_cd(&mut self, l: i16, r: i16) {
        self.cd_in = (l, r);
    }

    pub fn feed_ext(&mut self, l: i16, r: i16) {
        self.ext_in = (l, r);
    }

    fn mix_sample(&mut self) {
        let cnt = self.spucnt();
        let enabled = cnt & (1 << 15) != 0;
        let mute = cnt & (1 << 14) == 0;
        let mut left = 0i32;
        let mut right = 0i32;
        let mut rev_l = 0i32;
        let mut rev_r = 0i32;
        let mut v1 = 0i16;
        let mut v3 = 0i16;
        if enabled {
            let rev_voices = u32::from(self.regs[0xCC]) | (u32::from(self.regs[0xCD]) << 16);
            for i in 0..VOICES {
                let s = self.voice_sample(i);
                if i == 1 {
                    v1 = s.clamp(-0x8000, 0x7FFF) as i16;
                }
                if i == 3 {
                    v3 = s.clamp(-0x8000, 0x7FFF) as i16;
                }
                let vol_l = self.volume(self.regs[i * 8], Some(i));
                let vol_r = self.volume(self.regs[i * 8 + 1], Some(i));
                let vl = (s * vol_l) >> 15;
                let vr = (s * vol_r) >> 15;
                left += vl;
                right += vr;
                if rev_voices & (1 << i) != 0 {
                    rev_l += vl;
                    rev_r += vr;
                }
            }
            if cnt & 1 != 0 {
                let cl = i32::from(self.cd_in.0) * i32::from(self.regs[0xD8] as i16);
                let cr = i32::from(self.cd_in.1) * i32::from(self.regs[0xD9] as i16);
                left += cl >> 15;
                right += cr >> 15;
                if cnt & 4 != 0 {
                    rev_l += cl >> 15;
                    rev_r += cr >> 15;
                }
            }
            if cnt & 2 != 0 {
                let el = i32::from(self.ext_in.0) * i32::from(self.regs[0xDA] as i16);
                let er = i32::from(self.ext_in.1) * i32::from(self.regs[0xDB] as i16);
                left += el >> 15;
                right += er >> 15;
                if cnt & 8 != 0 {
                    rev_l += el >> 15;
                    rev_r += er >> 15;
                }
            }
            let (rl, rr) = self.reverb_mix(rev_l, rev_r, cnt & (1 << 7) != 0);
            left += rl;
            right += rr;
        }
        if mute || !enabled {
            left = 0;
            right = 0;
        } else {
            left = (left * self.volume(self.regs[0xC0], None)) >> 15;
            right = (right * self.volume(self.regs[0xC1], None)) >> 15;
        }
        let ol = left.clamp(-0x8000, 0x7FFF) as i16;
        let or = right.clamp(-0x8000, 0x7FFF) as i16;
        self.samples.push(ol);
        self.samples.push(or);
        let ci = self.capture_i;
        let off = |base: usize| (base + (ci * 2) % 0x400) & (SPU_RAM - 1);
        self.ram[off(0x000)..off(0x000) + 2].copy_from_slice(&self.cd_in.0.to_le_bytes());
        self.ram[off(0x400)..off(0x400) + 2].copy_from_slice(&self.cd_in.1.to_le_bytes());
        self.ram[off(0x800)..off(0x800) + 2].copy_from_slice(&v1.to_le_bytes());
        self.ram[off(0xC00)..off(0xC00) + 2].copy_from_slice(&v3.to_le_bytes());
        self.capture_i = (self.capture_i + 1) % 0x200;
    }

    fn ram_i16(&self, addr: u32) -> i32 {
        i32::from(self.ram16(addr) as i16)
    }

    fn poke_i16(&mut self, addr: u32, v: i32) {
        let a = (addr as usize) & (SPU_RAM - 1) & !1;
        let s = v.clamp(-0x8000, 0x7FFF) as i16;
        self.ram[a..a + 2].copy_from_slice(&s.to_le_bytes());
    }

    fn rev_rel(&self, reg: u16) -> u32 {
        let base = u32::from(self.regs[0xD1]) << 3;
        let area = 0x8_0000u32.saturating_sub(base).max(8);
        let off = (u32::from(reg) << 3).wrapping_add(self.reverb_addr);
        (base.wrapping_add(off % area)) & (SPU_RAM as u32 - 1) & !1
    }

    fn reverb_mix(&mut self, input_l: i32, input_r: i32, write: bool) -> (i32, i32) {
        let mul = |a: i32, b: i32| (a * b) >> 15;
        let viir = i32::from(self.regs[0xE2] as i16);
        let vwall = i32::from(self.regs[0xE7] as i16);
        let vlin = i32::from(self.regs[0xFE] as i16);
        let vrin = i32::from(self.regs[0xFF] as i16);
        let vcomb1 = i32::from(self.regs[0xE3] as i16);
        let vcomb2 = i32::from(self.regs[0xE4] as i16);
        let vcomb3 = i32::from(self.regs[0xE5] as i16);
        let vcomb4 = i32::from(self.regs[0xE6] as i16);
        let vapf1 = i32::from(self.regs[0xE8] as i16);
        let vapf2 = i32::from(self.regs[0xE9] as i16);
        let vlout = i32::from(self.regs[0xC2] as i16);
        let vrout = i32::from(self.regs[0xC3] as i16);
        let mlsame_a = self.regs[0xEA];
        let mrsame_a = self.regs[0xEB];
        let dlsame = self.regs[0xF0];
        let drsame = self.regs[0xF1];
        let mldiff_a = self.regs[0xF2];
        let mrdiff_a = self.regs[0xF3];
        let dldiff = self.regs[0xF8];
        let drdiff = self.regs[0xF9];
        let mlc1 = self.regs[0xEC];
        let mrc1 = self.regs[0xED];
        let mlc2 = self.regs[0xEE];
        let mrc2 = self.regs[0xEF];
        let mlc3 = self.regs[0xF4];
        let mrc3 = self.regs[0xF5];
        let mlc4 = self.regs[0xF6];
        let mrc4 = self.regs[0xF7];
        let mlapf1 = self.regs[0xFA];
        let mrapf1 = self.regs[0xFB];
        let mlapf2 = self.regs[0xFC];
        let mrapf2 = self.regs[0xFD];
        let dapf1 = self.regs[0xE0];
        let dapf2 = self.regs[0xE1];
        let lin = mul(vlin, input_l);
        let rin = mul(vrin, input_r);
        let reflect = |this: &Spu, input: i32, m: u16, d: u16| -> i32 {
            let dst = this.rev_rel(m);
            let src = this.rev_rel(d);
            let prev = this.ram_i16(dst.wrapping_sub(2));
            mul(input + mul(this.ram_i16(src), vwall) - prev, viir) + prev
        };
        let mlsame = reflect(self, lin, mlsame_a, dlsame);
        let mrsame = reflect(self, rin, mrsame_a, drsame);
        let mldiff = reflect(self, lin, mldiff_a, drdiff);
        let mrdiff = reflect(self, rin, mrdiff_a, dldiff);
        if write {
            self.poke_i16(self.rev_rel(mlsame_a), mlsame);
            self.poke_i16(self.rev_rel(mrsame_a), mrsame);
            self.poke_i16(self.rev_rel(mldiff_a), mldiff);
            self.poke_i16(self.rev_rel(mrdiff_a), mrdiff);
        }
        let mut lout = mul(vcomb1, self.ram_i16(self.rev_rel(mlc1)))
            + mul(vcomb2, self.ram_i16(self.rev_rel(mlc2)))
            + mul(vcomb3, self.ram_i16(self.rev_rel(mlc3)))
            + mul(vcomb4, self.ram_i16(self.rev_rel(mlc4)));
        let mut rout = mul(vcomb1, self.ram_i16(self.rev_rel(mrc1)))
            + mul(vcomb2, self.ram_i16(self.rev_rel(mrc2)))
            + mul(vcomb3, self.ram_i16(self.rev_rel(mrc3)))
            + mul(vcomb4, self.ram_i16(self.rev_rel(mrc4)));
        let apf = |this: &mut Spu, mut out: i32, m: u16, d: u16, vapf: i32| -> i32 {
            let dst = this.rev_rel(m);
            let src = this.rev_rel(m.wrapping_sub(d));
            let delayed = this.ram_i16(src);
            out -= mul(vapf, delayed);
            if write {
                this.poke_i16(dst, out);
            }
            mul(out, vapf) + delayed
        };
        lout = apf(self, lout, mlapf1, dapf1, vapf1);
        rout = apf(self, rout, mrapf1, dapf1, vapf1);
        lout = apf(self, lout, mlapf2, dapf2, vapf2);
        rout = apf(self, rout, mrapf2, dapf2, vapf2);
        let base = u32::from(self.regs[0xD1]) << 3;
        self.reverb_addr = self.reverb_addr.wrapping_add(2);
        if self.reverb_addr < base || self.reverb_addr > 0x7FFFE {
            self.reverb_addr = base;
        }
        (mul(lout, vlout), mul(rout, vrout))
    }

    fn voice_sample(&mut self, i: usize) -> i32 {
        if !self.voices[i].started || self.voices[i].phase == Phase::Off {
            return 0;
        }
        let pitch = u32::from(self.regs[i * 8 + 2]).min(0x4000);
        if pitch == 0 {
            return self.apply_adsr(i, i32::from(self.voices[i].gauss[3]));
        }
        self.voices[i].pitch_counter = self.voices[i].pitch_counter.saturating_add(pitch);
        while self.voices[i].pitch_counter >= 0x1000 {
            self.voices[i].pitch_counter -= 0x1000;
            if !self.push_decoded(i) {
                break;
            }
        }
        let i_idx = ((self.voices[i].pitch_counter >> 4) & 0xFF) as usize;
        let g = self.voices[i].gauss;
        let mut out = (i32::from(GAUSS[0xFF - i_idx]) * i32::from(g[0])) >> 15;
        out += (i32::from(GAUSS[0x1FF - i_idx]) * i32::from(g[1])) >> 15;
        out += (i32::from(GAUSS[0x100 + i_idx]) * i32::from(g[2])) >> 15;
        out += (i32::from(GAUSS[i_idx]) * i32::from(g[3])) >> 15;
        self.apply_adsr(i, out)
    }

    fn push_decoded(&mut self, i: usize) -> bool {
        if self.voices[i].decode_idx >= 28 {
            if !self.decode_block(i) {
                return false;
            }
        }
        let s = self.voices[i].decoded[self.voices[i].decode_idx];
        self.voices[i].decode_idx += 1;
        let g = &mut self.voices[i].gauss;
        g[0] = g[1];
        g[1] = g[2];
        g[2] = g[3];
        g[3] = s;
        true
    }

    fn decode_block(&mut self, i: usize) -> bool {
        if self.voices[i].end_after {
            self.endx |= 1 << i;
            if !self.voices[i].loop_repeat {
                self.voices[i].phase = Phase::Release;
                self.voices[i].adsr_level = 0;
                self.voices[i].adsr_counter = 0;
            }
            self.voices[i].current_addr = self.voices[i].repeat_addr;
            self.voices[i].end_after = false;
        }
        let addr = (self.voices[i].current_addr as usize) & (SPU_RAM - 1) & !0xF;
        self.hit_irq(addr as u32);
        let header = self.ram[addr];
        let flags = self.ram[addr + 1];
        let mut shift = header & 0xF;
        if shift > 12 {
            shift = 9;
        }
        let mut filter = (header >> 4) & 7;
        if filter > 4 {
            filter = 4;
        }
        let f0 = FILTER[filter as usize][0];
        let f1 = FILTER[filter as usize][1];
        if flags & 4 != 0 {
            self.voices[i].repeat_addr = addr as u32;
        }
        self.voices[i].end_after = flags & 1 != 0;
        self.voices[i].loop_repeat = flags & 2 != 0;
        let mut hist = self.voices[i].hist;
        for n in 0..14 {
            let byte = self.ram[addr + 2 + n];
            for (k, nib) in [byte & 0xF, byte >> 4].into_iter().enumerate() {
                let mut t = i32::from(nib as i8);
                if t >= 8 {
                    t -= 16;
                }
                t = (t << 12) >> shift;
                t += (hist[0] * f0 + hist[1] * f1 + 32) >> 6;
                t = t.clamp(-0x8000, 0x7FFF);
                self.voices[i].decoded[n * 2 + k] = t as i16;
                hist[1] = hist[0];
                hist[0] = t;
            }
        }
        self.voices[i].hist = hist;
        self.voices[i].decode_idx = 0;
        self.voices[i].current_addr = (addr as u32).wrapping_add(16);
        true
    }

    fn apply_adsr(&mut self, i: usize, sample: i32) -> i32 {
        let adsr_lo = self.regs[i * 8 + 4];
        let adsr_hi = self.regs[i * 8 + 5];
        let sustain_level = (i32::from(adsr_lo & 0xF) + 1) * 0x800;
        let (shift, step, exp, decreasing) = match self.voices[i].phase {
            Phase::Off => return 0,
            Phase::Attack => (
                u32::from((adsr_lo >> 10) & 0x1F),
                u32::from((adsr_lo >> 8) & 3),
                adsr_lo & (1 << 15) != 0,
                false,
            ),
            Phase::Decay => (u32::from((adsr_lo >> 4) & 0xF), 0, true, true),
            Phase::Sustain => (
                u32::from((adsr_hi >> 8) & 0x1F),
                u32::from((adsr_hi >> 6) & 3),
                adsr_hi & (1 << 15) != 0,
                adsr_hi & (1 << 14) != 0,
            ),
            Phase::Release => (u32::from(adsr_hi & 0x1F), 0, adsr_hi & (1 << 5) != 0, true),
        };
        envelope_tick(&mut self.voices[i], shift, step, exp, decreasing);
        match self.voices[i].phase {
            Phase::Attack if self.voices[i].adsr_level >= 0x7FFF => {
                self.voices[i].adsr_level = 0x7FFF;
                self.voices[i].phase = Phase::Decay;
                self.voices[i].adsr_counter = 0;
            }
            Phase::Decay if self.voices[i].adsr_level <= sustain_level => {
                self.voices[i].adsr_level = sustain_level.max(0);
                self.voices[i].phase = Phase::Sustain;
                self.voices[i].adsr_counter = 0;
            }
            Phase::Release if self.voices[i].adsr_level <= 0 => {
                self.voices[i].adsr_level = 0;
                self.voices[i].phase = Phase::Off;
            }
            _ => {}
        }
        (sample * self.voices[i].adsr_level) >> 15
    }
}

impl Spu {
    fn volume(&mut self, reg: u16, voice: Option<usize>) -> i32 {
        if reg & 0x8000 == 0 {
            let v = i32::from((reg as i16) << 1);
            match voice {
                Some(i) => self.voices[i].sweep_level = v,
                None => {}
            }
            v
        } else {
            let exp = reg & (1 << 14) != 0;
            let dec = reg & (1 << 13) != 0;
            let shift = u32::from((reg >> 2) & 0x1F);
            let step = u32::from(reg & 3);
            match voice {
                Some(i) => {
                    tick_level(
                        &mut self.voices[i].sweep_level,
                        &mut self.voices[i].sweep_counter,
                        shift,
                        step,
                        exp,
                        dec,
                    );
                    self.voices[i].sweep_level
                }
                None => {
                    let slot = if reg == self.regs[0xC1] { 1 } else { 0 };
                    tick_level(
                        &mut self.main_sweep[slot],
                        &mut self.main_sweep_ctr[slot],
                        shift,
                        step,
                        exp,
                        dec,
                    );
                    self.main_sweep[slot]
                }
            }
        }
    }
}

fn tick_level(
    level: &mut i32,
    counter: &mut u32,
    shift: u32,
    step_value: u32,
    exponential: bool,
    decreasing: bool,
) {
    let mut dummy = Voice::new();
    dummy.adsr_level = *level;
    dummy.adsr_counter = *counter;
    dummy.phase = if decreasing {
        Phase::Release
    } else {
        Phase::Attack
    };
    envelope_tick(&mut dummy, shift, step_value, exponential, decreasing);
    *level = dummy.adsr_level;
    *counter = dummy.adsr_counter;
}

fn envelope_tick(v: &mut Voice, shift: u32, step_value: u32, exponential: bool, decreasing: bool) {
    if (step_value | (shift << 2)) == 0x7F {
        return;
    }
    let mut adsr_step = 7 - step_value as i32;
    if decreasing {
        adsr_step = !adsr_step;
    }
    let shl = 11u32.saturating_sub(shift);
    adsr_step <<= shl.min(31);
    let mut counter_inc = 0x8000u32 >> shift.saturating_sub(11).min(31);
    if exponential && !decreasing && v.adsr_level > 0x6000 {
        if shift < 10 {
            adsr_step /= 4;
        } else if shift >= 11 {
            counter_inc /= 4;
        } else {
            adsr_step /= 2;
            counter_inc /= 2;
        }
    } else if exponential && decreasing {
        adsr_step = adsr_step * v.adsr_level / 0x8000;
    }
    if counter_inc == 0 {
        counter_inc = 1;
    }
    v.adsr_counter = v.adsr_counter.wrapping_add(counter_inc);
    if v.adsr_counter & 0x8000 == 0 {
        return;
    }
    v.adsr_counter &= 0x7FFF;
    let mut level = v.adsr_level + adsr_step;
    if !decreasing {
        level = level.clamp(-0x8000, 0x7FFF);
    } else {
        level = level.max(0);
    }
    v.adsr_level = level;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent_loop() -> [u8; 16] {
        let mut b = [0u8; 16];
        b[1] = 0b011; // loop end + repeat
        b
    }

    fn tone_block(shift: u8, nibble: u8) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0] = shift & 0xF;
        b[1] = 0b011;
        let packed = nibble | (nibble << 4);
        for i in 2..16 {
            b[i] = packed;
        }
        b
    }

    fn spu_with_tone() -> Spu {
        let mut s = Spu::new();
        s.ram[0x1000..0x1010].copy_from_slice(&tone_block(0, 7));
        s.regs[0xD5] = 0xC000; // enable + unmute
        s.applied_mode = 0;
        // voice 0: vol L/R mid, pitch 1000h, start 1000h/8
        s.regs[0] = 0x3FFF;
        s.regs[1] = 0x3FFF;
        s.regs[2] = 0x1000;
        s.regs[3] = 0x1000 / 8;
        // ADSR: fastest linear attack, sustain full, slow release
        s.regs[4] = 0x80FF; // attack shift 0 step 0, sustain level F
        s.regs[5] = 0x1F00; // release shift 1Fh (slow)
        s.regs[0xC0] = 0x3FFF;
        s.regs[0xC1] = 0x3FFF;
        s.key_on(1);
        s
    }

    #[test]
    fn adpcm_filter0_decodes_constant_nibble() {
        let mut s = Spu::new();
        s.ram[0x1000..0x1010].copy_from_slice(&tone_block(0, 7));
        s.voices[0].current_addr = 0x1000;
        s.voices[0].started = true;
        assert!(s.decode_block(0));
        // nibble 7, shift 0 → 7<<12 = 0x7000
        assert_eq!(s.voices[0].decoded[0], 0x7000);
        assert_eq!(s.voices[0].decoded[27], 0x7000);
    }

    #[test]
    fn seven_hundred_sixty_eight_cycles_emit_one_stereo_frame() {
        let mut s = spu_with_tone();
        assert!(s.take_samples().is_empty());
        s.tick(767);
        assert!(s.take_samples().is_empty());
        s.tick(1);
        let pcm = s.take_samples();
        assert_eq!(pcm.len(), 2, "one stereo pair");
    }

    #[test]
    fn keyed_voice_is_audible_after_attack() {
        let mut s = spu_with_tone();
        s.tick(CYCLES_PER_SAMPLE * 64);
        let pcm = s.take_samples();
        let peak = pcm.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        assert!(
            peak > 1000,
            "keyed ADPCM tone must be audible (peak={peak})"
        );
    }

    #[test]
    fn master_volume_zero_is_silent() {
        let mut s = spu_with_tone();
        s.regs[0xC0] = 0;
        s.regs[0xC1] = 0;
        s.tick(CYCLES_PER_SAMPLE * 64);
        let pcm = s.take_samples();
        assert!(pcm.iter().all(|&x| x == 0), "main volume 0 is silence");
    }

    #[test]
    fn mute_bit_silences_voices() {
        let mut s = spu_with_tone();
        s.regs[0xD5] = 0x8000; // enable, mute
        s.tick(CYCLES_PER_SAMPLE * 32);
        assert!(s.take_samples().iter().all(|&x| x == 0));
    }

    #[test]
    fn loop_end_sets_endx() {
        let mut s = Spu::new();
        s.ram[0x1000..0x1010].copy_from_slice(&silent_loop());
        s.regs[0xD5] = 0xC000;
        s.regs[2] = 0x1000;
        s.regs[3] = 0x1000 / 8;
        s.regs[4] = 0x80FF;
        s.key_on(1);
        // 28 samples at pitch 1000h → one sample per tick; finish the block.
        s.tick(CYCLES_PER_SAMPLE * 40);
        assert_ne!(s.endx & 1, 0, "LOOP-END must set ENDX");
    }

    #[test]
    fn gauss_four_taps_sum_to_headroom() {
        for i in 0..256 {
            let sum = i32::from(GAUSS[0xFF - i])
                + i32::from(GAUSS[0x1FF - i])
                + i32::from(GAUSS[0x100 + i])
                + i32::from(GAUSS[i]);
            assert!((sum - 0x7F80).abs() <= 2, "gauss taps at {i} sum {sum:#X}");
        }
    }

    #[test]
    fn cd_and_ext_input_mix_when_spucnt_bits_set() {
        let mut s = Spu::new();
        s.regs[0xD5] = 0xC000;
        s.regs[0xC0] = 0x3FFF;
        s.regs[0xC1] = 0x3FFF;
        s.regs[0xD8] = 0x3FFF;
        s.regs[0xD9] = 0x3FFF;
        s.feed_cd(0x4000, -0x4000);
        s.tick(CYCLES_PER_SAMPLE);
        assert!(
            s.take_samples().iter().all(|&x| x == 0),
            "CD input is silent while SPUCNT.0 is off"
        );
        s.regs[0xD5] = 0xC001;
        s.feed_cd(0x4000, -0x4000);
        s.tick(CYCLES_PER_SAMPLE);
        let pcm = s.take_samples();
        assert!(pcm[0] > 1000, "SPUCNT.0 mixes CD left ({})", pcm[0]);
        assert!(pcm[1] < -1000, "SPUCNT.0 mixes CD right ({})", pcm[1]);
        s.regs[0xD5] = 0xC002;
        s.regs[0xDA] = 0x3FFF;
        s.regs[0xDB] = 0x3FFF;
        s.feed_ext(-0x2000, 0x2000);
        s.tick(CYCLES_PER_SAMPLE);
        let pcm = s.take_samples();
        assert!(pcm[0] < -500, "SPUCNT.1 mixes external left");
        assert!(pcm[1] > 500, "SPUCNT.1 mixes external right");
    }

    #[test]
    fn reverb_work_area_mix_irq_address_and_capture() {
        let mut s = Spu::new();
        s.regs[0xD5] = 0xC080;
        s.regs[0xC0] = 0x3FFF;
        s.regs[0xC1] = 0x3FFF;
        s.regs[0xC2] = 0x3FFF;
        s.regs[0xC3] = 0x3FFF;
        s.regs[0xD1] = (0x10000 / 8) as u16;
        s.regs[0xE2] = 0x4000;
        s.regs[0xEA] = 0x0020;
        s.regs[0xEB] = 0x0022;
        s.regs[0xF2] = 0x0024;
        s.regs[0xF3] = 0x0026;
        s.regs[0xFA] = 0x0040;
        s.regs[0xFB] = 0x0042;
        s.regs[0xFC] = 0x0060;
        s.regs[0xFD] = 0x0062;
        s.regs[0xFE] = 0x3FFF;
        s.regs[0xFF] = 0x3FFF;
        s.regs[0] = 0x3FFF;
        s.regs[1] = 0x3FFF;
        s.regs[2] = 0x1000;
        s.regs[3] = 0x1000 / 8;
        s.regs[4] = 0x80FF;
        s.regs[5] = 0x1F00;
        s.regs[0xCC] = 1;
        s.ram[0x1000..0x1010].copy_from_slice(&tone_block(0, 7));
        s.key_on(1);
        s.tick(CYCLES_PER_SAMPLE * 64);
        let pcm = s.take_samples();
        let peak = pcm.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
        assert!(
            peak > 100,
            "reverb input voice must be audible (peak={peak})"
        );
        let work = (0..0x200).any(|i| s.ram16(0x10000 + i * 2) != 0);
        assert!(work, "reverb master enable writes the work area");

        let mut s = Spu::new();
        s.regs[0xD5] = 0xC040;
        s.regs[0xD2] = 0x1000 / 8;
        s.ram[0x1000..0x1010].copy_from_slice(&tone_block(0, 7));
        s.regs[2] = 0x1000;
        s.regs[3] = 0x1000 / 8;
        s.regs[4] = 0x80FF;
        s.key_on(1);
        s.tick(CYCLES_PER_SAMPLE * 8);
        assert!(s.take_irq(), "voice read of IRQ address raises IRQ9");

        let mut s = Spu::new();
        s.regs[0xD5] = 0xC000;
        s.regs[0xC0] = 0x3FFF;
        s.regs[0xC1] = 0x3FFF;
        s.ram[0x2000..0x2010].copy_from_slice(&tone_block(0, 7));
        s.regs[1 * 8] = 0x3FFF;
        s.regs[1 * 8 + 1] = 0x3FFF;
        s.regs[1 * 8 + 2] = 0x1000;
        s.regs[1 * 8 + 3] = 0x2000 / 8;
        s.regs[1 * 8 + 4] = 0x80FF;
        s.ram[0x3000..0x3010].copy_from_slice(&tone_block(0, 6));
        s.regs[3 * 8] = 0x3FFF;
        s.regs[3 * 8 + 1] = 0x3FFF;
        s.regs[3 * 8 + 2] = 0x1000;
        s.regs[3 * 8 + 3] = 0x3000 / 8;
        s.regs[3 * 8 + 4] = 0x80FF;
        s.key_on((1 << 1) | (1 << 3));
        s.tick(CYCLES_PER_SAMPLE * 8);
        assert_ne!(s.ram16(0x800), 0, "voice 1 writes capture at 800h");
        assert_ne!(s.ram16(0xC00), 0, "voice 3 writes capture at C00h");
        s.regs[0xD5] = 0xC001;
        s.regs[0xD8] = 0x3FFF;
        s.regs[0xD9] = 0x3FFF;
        s.feed_cd(0x1234, -0x2345);
        s.tick(CYCLES_PER_SAMPLE);
        assert!(
            (0..0x200).any(|i| s.ram16(i * 2) == 0x1234),
            "CD left capture at 000h"
        );
        assert!(
            (0..0x200).any(|i| s.ram16(0x400 + i * 2) == (-0x2345i16) as u16),
            "CD right capture at 400h"
        );
    }
}
