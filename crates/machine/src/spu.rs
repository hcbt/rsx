//! SPU: 24 ADPCM voices, mix, 44100 Hz stereo. Host playback is the Debugger.

include!("spu_gauss.rs");

const SPU_RAM: usize = 512 * 1024;
const CYCLES_PER_SAMPLE: u32 = 768;
const VOICES: usize = 24;
const SAMPLE_CAP: usize = 44_100 * 2 * 120;

const FILTER: [[i32; 2]; 5] = [
    [0, 0],
    [60, 0],
    [115, -52],
    [98, -55],
    [122, -60],
];

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
    irq_pending: bool,
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
            irq_pending: false,
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

    fn mix_sample(&mut self) {
        let cnt = self.spucnt();
        let enabled = cnt & (1 << 15) != 0;
        let mute = cnt & (1 << 14) == 0;
        let mut left = 0i32;
        let mut right = 0i32;
        if enabled {
            for i in 0..VOICES {
                let s = self.voice_sample(i);
                let vol_l = voice_volume(self.regs[i * 8]);
                let vol_r = voice_volume(self.regs[i * 8 + 1]);
                left += (s * vol_l) >> 15;
                right += (s * vol_r) >> 15;
            }
        }
        if mute || !enabled {
            left = 0;
            right = 0;
        } else {
            left = (left * voice_volume(self.regs[0xC0])) >> 15;
            right = (right * voice_volume(self.regs[0xC1])) >> 15;
        }
        if self.samples.len() >= SAMPLE_CAP {
            let drop = self.samples.len() - SAMPLE_CAP + 2;
            self.samples.drain(..drop);
        }
        self.samples.push(left.clamp(-0x8000, 0x7FFF) as i16);
        self.samples.push(right.clamp(-0x8000, 0x7FFF) as i16);
    }

    fn voice_sample(&mut self, i: usize) -> i32 {
        if !self.voices[i].started {
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

fn voice_volume(reg: u16) -> i32 {
    if reg & 0x8000 != 0 {
        // Sweep not yet: hold mid volume so wet-looking writes still sound.
        0x7FFF
    } else {
        i32::from((reg as i16) << 1)
    }
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
            assert!(
                (sum - 0x7F80).abs() <= 2,
                "gauss taps at {i} sum {sum:#X}"
            );
        }
    }
}
