use std::collections::{HashMap, VecDeque};

use crate::DisplayArea;

const VRAM_W: usize = 1024;
const VRAM_H: usize = 512;
const FIFO_WORDS: usize = 16;

pub struct Gpu {
    vram: Vec<u16>,
    gp0_cmd: Option<u8>,
    gp0_buf: Vec<u32>,
    gpuread: u32,
    stat: u32,
    draw_x1: i32,
    draw_y1: i32,
    draw_x2: i32,
    draw_y2: i32,
    off_x: i32,
    off_y: i32,
    tex_x: u8,
    tex_y: u8,
    tex_page: u32,
    clut_x: u32,
    clut_y: u32,
    dither: bool,
    draw_to_display: bool,
    mask_set: bool,
    mask_check: bool,
    tex_win_mask_x: u32,
    tex_win_mask_y: u32,
    tex_win_off_x: u32,
    tex_win_off_y: u32,
    display_x: u32,
    display_y: u32,
    display_hres: u32,
    display_vres: u32,
    display_enabled: bool,
    dma_dir: u8,
    interlace: bool,
    disp_24: bool,
    gpu_irq: bool,
    range_x1: u32,
    range_x2: u32,
    range_y1: u32,
    range_y2: u32,
    vram_2m: bool,
    tex_cache: HashMap<(u32, u32), u16>,
    clut_cache: Option<(u32, u32, [u16; 16])>,
    transfer: Option<Transfer>,
    pub gp0_count: u64,
    pub gp1_count: u64,
    pub gp0_cmds: Vec<u8>,
    pub gp0_words: Vec<u32>,
    pub gp1_cmds: Vec<u32>,
    odd_frame: bool,
    in_vblank: bool,
    pub frame_n30: u32,
    pub frame_x0: i32,
    pub frame_x1: i32,
    pub frame_y0: i32,
    pub frame_y1: i32,
    pub last_n30: u32,
    pub last_x0: i32,
    pub last_x1: i32,
    pub last_y0: i32,
    pub last_y1: i32,
    pub last_n30_out: u32,
    frame_n30_out: u32,
    pub last_y_bins: [u32; 16],
    frame_y_bins: [u32; 16],
    pub last_hi_y_word: u32,
    /// Occupancy of GP0(30) verts, 512×240 (x wrapped into one buffer).
    pub last_scatter: Vec<u8>,
    frame_scatter: Vec<u8>,
    pub last_long30: u32,
    frame_long30: u32,
    pub last_max_dy: i32,
    frame_max_dy: i32,
    /// Last completed frame: GP0(20h–3Fh) counts by command byte.
    pub last_poly_op: [u32; 32],
    frame_poly_op: [u32; 32],
    /// CRT: last latched field. `display_area` is this, not live VRAM.
    crt: Vec<u16>,
    crt_w: u32,
    crt_h: u32,
    crt_line: u32,
    fifo: VecDeque<u32>,
    draw_busy: u32,
    plot_cost: u32,
    plotting: bool,
    block28: bool,
    scanline: u32,
}

#[allow(dead_code)]
struct Transfer {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    remaining: u32,
    to_vram: bool,
    cur_x: u32,
    cur_y: u32,
}

impl Gpu {
    pub fn new() -> Self {
        let mut g = Self {
            vram: vec![0; VRAM_W * VRAM_H],
            gp0_cmd: None,
            gp0_buf: Vec::new(),
            gpuread: 0,
            stat: 0x1480_2000,
            draw_x1: 0,
            draw_y1: 0,
            draw_x2: 1023,
            draw_y2: 511,
            off_x: 0,
            off_y: 0,
            tex_x: 0,
            tex_y: 0,
            tex_page: 0,
            clut_x: 0,
            clut_y: 0,
            dither: false,
            draw_to_display: false,
            mask_set: false,
            mask_check: false,
            tex_win_mask_x: 0,
            tex_win_mask_y: 0,
            tex_win_off_x: 0,
            tex_win_off_y: 0,
            display_x: 0,
            display_y: 0,
            display_hres: 256,
            display_vres: 240,
            display_enabled: false,
            dma_dir: 0,
            interlace: false,
            disp_24: false,
            gpu_irq: false,
            range_x1: 0x200,
            range_x2: 0x200 + 256 * 10,
            range_y1: 0x10,
            range_y2: 0x10 + 240,
            vram_2m: false,
            tex_cache: HashMap::new(),
            clut_cache: None,
            transfer: None,
            gp0_count: 0,
            gp1_count: 0,
            gp0_cmds: Vec::new(),
            gp0_words: Vec::new(),
            gp1_cmds: Vec::new(),
            odd_frame: false,
            in_vblank: false,
            frame_n30: 0,
            frame_x0: i32::MAX,
            frame_x1: i32::MIN,
            frame_y0: i32::MAX,
            frame_y1: i32::MIN,
            last_n30: 0,
            last_x0: 0,
            last_x1: 0,
            last_y0: 0,
            last_y1: 0,
            last_n30_out: 0,
            frame_n30_out: 0,
            last_y_bins: [0; 16],
            frame_y_bins: [0; 16],
            last_hi_y_word: 0,
            last_scatter: vec![0; 512 * 240],
            frame_scatter: vec![0; 512 * 240],
            last_long30: 0,
            frame_long30: 0,
            last_max_dy: 0,
            frame_max_dy: 0,
            last_poly_op: [0; 32],
            frame_poly_op: [0; 32],
            crt: Vec::new(),
            crt_w: 0,
            crt_h: 0,
            crt_line: u32::MAX,
            fifo: VecDeque::with_capacity(FIFO_WORDS),
            draw_busy: 0,
            plot_cost: 1,
            plotting: false,
            block28: false,
            scanline: 0,
        };
        g.update_stat();
        g
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn stat(&self) -> u32 {
        self.stat
    }

    pub fn irq_line(&self) -> bool {
        self.gpu_irq
    }

    pub fn display_range(&self) -> (u32, u32, u32, u32) {
        (self.range_x1, self.range_x2, self.range_y1, self.range_y2)
    }

    fn vis_h(&self) -> u32 {
        let range_h = self.range_y2.saturating_sub(self.range_y1);
        if self.display_vres >= 480 {
            self.display_vres.max(1).min(480)
        } else if range_h > 0 {
            range_h.min(480)
        } else {
            self.display_vres.max(1).min(480)
        }
    }

    pub fn fifo_full(&self) -> bool {
        self.fifo.len() >= FIFO_WORDS
    }

    pub fn fifo_is_empty(&self) -> bool {
        self.fifo.is_empty()
    }

    pub fn fifo_space(&self) -> u32 {
        (FIFO_WORDS.saturating_sub(self.fifo.len())) as u32
    }

    pub fn fifo_len(&self) -> usize {
        self.fifo.len()
    }

    pub fn draw_remaining(&self) -> u32 {
        self.draw_busy
    }

    pub fn assembling(&self) -> bool {
        self.gp0_cmd.is_some() || self.transfer.is_some()
    }

    pub fn busy(&self) -> bool {
        self.draw_busy > 0 || self.gp0_cmd.is_some() || self.transfer.is_some()
    }

    fn update_stat(&mut self) {
        let mut s = 0x1400_0000; // ready bits 26 and 28; bit 27 only for VRAM-to-CPU
        let need_params = self.gp0_cmd.is_some();
        let drawing = self.draw_busy > 0;
        if let Some(t) = self.transfer.as_ref() {
            if t.to_vram {
                // SPX / CPU-to-VRAM: still wants GP0 words → GPUSTAT.26 stays set.
            } else {
                s &= !(1 << 26);
                s &= !(1 << 28);
                s |= 1 << 27;
            }
        } else if need_params || drawing {
            s &= !(1 << 26);
        }
        // SPX: bit 28 is the write FIFO empty / DMA-block ready flag.
        // Polygon and line commands clear it from the command word (block28),
        // not only after the last vertex.
        if drawing || self.block28 || !self.fifo.is_empty() || need_params {
            s &= !(1 << 28);
        }
        if !drawing && self.fifo.is_empty() && !need_params && self.transfer.is_none() {
            self.block28 = false;
            s |= 1 << 28;
        }
        s |= u32::from(self.tex_x) & 0xF;
        s |= (u32::from(self.tex_y) & 1) << 4;
        s |= (self.tex_page & 0x1E0) << 0;
        if self.dither {
            s |= 1 << 9;
        }
        if self.draw_to_display {
            s |= 1 << 10;
        }
        if self.mask_set {
            s |= 1 << 11;
        }
        if self.mask_check {
            s |= 1 << 12;
        }
        if !self.interlace || self.odd_frame {
            s |= 1 << 13;
        }
        if !self.in_vblank {
            if self.display_vres >= 480 {
                if self.odd_frame {
                    s |= 1 << 31;
                }
            } else if self.scanline & 1 != 0 {
                s |= 1 << 31;
            }
        }
        match self.dma_dir {
            1 if self.fifo.len() <= FIFO_WORDS / 2 => s |= 1 << 25,
            2 if s & (1 << 28) != 0 => s |= 1 << 25,
            3 if s & (1 << 27) != 0 => s |= 1 << 25,
            _ => {}
        }
        s |= match self.display_hres {
            256 => 0,
            320 => 1 << 17,
            512 => 2 << 17,
            640 => 3 << 17,
            368 => 1 << 16,
            _ => 1 << 17,
        };
        if self.display_vres >= 480 {
            s |= 1 << 19;
        }
        if !self.display_enabled {
            s |= 1 << 23;
        }
        if self.disp_24 {
            s |= 1 << 21;
        }
        if self.gpu_irq {
            s |= 1 << 24;
        }
        s |= u32::from(self.dma_dir) << 29;
        self.stat = s;
    }

    pub fn gp0(&mut self, word: u32) {
        self.gp0_count += 1;
        if self.gp0_words.len() < 96 {
            self.gp0_words.push(word);
        }
        let cmd = (word >> 24) as u8;
        // SPX: GP0(E3h..E5h) do not take FIFO space; they run immediately.
        // A parameter of a command already being assembled is not a command.
        if (0xE3..=0xE5).contains(&cmd) && self.gp0_cmd.is_none() && self.transfer.is_none() {
            let prev_cmd = self.gp0_cmd.take();
            let prev_buf = std::mem::take(&mut self.gp0_buf);
            self.gp0_cmd = Some(cmd);
            self.gp0_buf.push(word);
            self.exec_gp0();
            self.gp0_cmd = prev_cmd;
            self.gp0_buf = prev_buf;
            self.update_stat();
            return;
        }
        // GP0(00h) / 04h–1Eh are NOPs only as command words, not as parameters.
        if self.gp0_cmd.is_none()
            && self.transfer.is_none()
            && self.fifo.is_empty()
            && (cmd == 0 || (0x04..=0x1E).contains(&cmd))
        {
            return;
        }
        while self.fifo.len() >= FIFO_WORDS {
            self.tick(1, 0, false);
        }
        self.fifo.push_back(word);
        self.update_stat();
    }

    fn accept_word(&mut self, word: u32) {
        if self.transfer.as_ref().is_some_and(|t| t.to_vram) {
            let (mut x, mut y, w, base_x) = {
                let t = self.transfer.as_ref().unwrap();
                (t.cur_x, t.cur_y, t.w, t.x)
            };
            self.write_half(x, y, word as u16);
            x += 1;
            if x >= base_x + w {
                x = base_x;
                y += 1;
            }
            self.write_half(x, y, (word >> 16) as u16);
            x += 1;
            if x >= base_x + w {
                x = base_x;
                y += 1;
            }
            if let Some(t) = self.transfer.as_mut() {
                t.cur_x = x;
                t.cur_y = y;
                t.remaining = t.remaining.saturating_sub(1);
                if t.remaining == 0 {
                    self.tex_cache.clear();
                    self.transfer = None;
                    self.update_stat();
                }
            }
            return;
        }
        if self.gp0_cmd.is_none() {
            let cmd = (word >> 24) as u8;
            if self.gp0_cmds.len() < 64 {
                self.gp0_cmds.push(cmd);
            }
            if cmd == 0 || (0x04..=0x1E).contains(&cmd) {
                return;
            }
            self.gp0_cmd = Some(cmd);
            if (0x20..=0x5F).contains(&cmd) {
                self.block28 = true;
            }
            self.gp0_buf.clear();
            self.gp0_buf.push(word);
        } else {
            self.gp0_buf.push(word);
        }
        if self.gp0_ready() {
            let cmd = self.gp0_cmd.unwrap();
            // Fill VRAM is a 16-pixel burst, not fragment raster. Raster
            // primitives take one cycle per written pixel (two if textured).
            let raster = matches!(cmd, 0x20..=0x7F | 0x80);
            self.plotting = raster;
            self.plot_cost = if raster && cmd & 4 != 0 && (0x20..=0x7F).contains(&cmd) {
                2
            } else {
                1
            };
            self.exec_gp0();
            self.plotting = false;
            if raster || cmd == 0x02 {
                self.draw_busy = self.draw_busy.max(1);
                self.block28 = true;
            }
            self.gp0_cmd = None;
            self.gp0_buf.clear();
        }
    }

    fn gp0_len(cmd: u8) -> usize {
        match cmd {
            0x02 => 3,
            0x1F => 1,
            0x01 => 1, // clear texture cache
            0x20..=0x3F => polygon_len(cmd),
            0x40..=0x5F => line_len(cmd),
            0x60..=0x7F => rect_len(cmd),
            0x80 => 4,
            0xA0 => 3, // then data
            0xC0 => 3,
            0xE1..=0xE6 => 1,
            _ => 1,
        }
    }

    fn gp0_ready(&self) -> bool {
        let cmd = self.gp0_cmd.unwrap_or(0);
        if (0x40..=0x5F).contains(&cmd) && cmd & 0x08 != 0 {
            // polyline: terminate
            if let Some(&last) = self.gp0_buf.last() {
                return last & 0xF000_F000 == 0x5000_5000 && self.gp0_buf.len() >= 3;
            }
        }
        self.gp0_buf.len() >= Self::gp0_len(cmd)
    }

    fn exec_gp0(&mut self) {
        let cmd = self.gp0_cmd.unwrap();
        match cmd {
            0x01 => {
                self.tex_cache.clear();
                self.clut_cache = None;
            }
            0x1F => {
                self.gpu_irq = true;
                self.update_stat();
            }
            0x02 => self.fill(),
            0x20..=0x3F => self.polygon(),
            0x40..=0x5F => self.line(),
            0x60..=0x7F => self.rectangle(),
            0x80 => {
                self.tex_cache.clear();
                self.vram_copy();
            }
            0xA0 | 0xC0 => {
                let xy = self.gp0_buf[1];
                let wh = self.gp0_buf[2];
                let x = xy & 0x3FF;
                let y = (xy >> 16) & 0x1FF;
                let w = ((wh.wrapping_sub(1)) & 0x3FF) + 1;
                let h = (((wh >> 16).wrapping_sub(1)) & 0x1FF) + 1;
                let words = (w * h + 1) / 2;
                self.transfer = Some(Transfer {
                    x,
                    y,
                    w,
                    h,
                    remaining: words,
                    to_vram: cmd == 0xA0,
                    cur_x: x,
                    cur_y: y,
                });
                self.update_stat();
            }
            0xE1 => {
                let p = self.gp0_buf[0];
                self.tex_x = (p & 0xF) as u8;
                self.tex_y = ((p >> 4) & 1) as u8;
                self.tex_page = p;
                self.dither = p & (1 << 9) != 0;
                self.draw_to_display = p & (1 << 10) != 0;
                self.update_stat();
            }
            0xE2 => {
                // SPX: UV = (UV AND NOT (mask*8)) OR ((offset AND mask)*8).
                let p = self.gp0_buf[0];
                self.tex_win_mask_x = p & 0x1F;
                self.tex_win_mask_y = (p >> 5) & 0x1F;
                self.tex_win_off_x = (p >> 10) & 0x1F;
                self.tex_win_off_y = (p >> 15) & 0x1F;
            }
            0xE3 => {
                let p = self.gp0_buf[0];
                self.draw_x1 = (p & 0x3FF) as i32;
                self.draw_y1 = ((p >> 10) & 0x1FF) as i32;
            }
            0xE4 => {
                let p = self.gp0_buf[0];
                self.draw_x2 = (p & 0x3FF) as i32;
                self.draw_y2 = ((p >> 10) & 0x1FF) as i32;
            }
            0xE5 => {
                let p = self.gp0_buf[0];
                self.off_x = sign11(p & 0x7FF);
                self.off_y = sign11((p >> 11) & 0x7FF);
            }
            0xE6 => {
                let p = self.gp0_buf[0];
                self.mask_set = p & 1 != 0;
                self.mask_check = p & 2 != 0;
                self.update_stat();
            }
            _ => {}
        }
    }

    pub fn gp1(&mut self, word: u32) {
        self.gp1_count += 1;
        if self.gp1_cmds.len() < 64 {
            self.gp1_cmds.push(word);
        }
        let cmd = (word >> 24) & 0x3F;
        let p = word & 0xFF_FFFF;
        match cmd {
            0x00 => self.reset(),
            0x01 => {
                self.gp0_buf.clear();
                self.gp0_cmd = None;
                self.fifo.clear();
                self.draw_busy = 0;
                self.block28 = false;
            }
            0x02 => {
                self.gpu_irq = false;
                self.update_stat();
            }
            0x03 => {
                self.display_enabled = p & 1 == 0;
                self.update_stat();
            }
            0x04 => {
                self.dma_dir = (p & 3) as u8;
                self.update_stat();
            }
            0x05 => {
                self.display_x = p & 0x3FF;
                self.display_y = (p >> 10) & 0x1FF;
            }
            0x08 => {
                if p & (1 << 6) != 0 {
                    self.display_hres = 368;
                } else {
                    self.display_hres = match p & 3 {
                        0 => 256,
                        1 => 320,
                        2 => 512,
                        _ => 640,
                    };
                }
                self.display_vres = if p & 4 != 0 { 480 } else { 240 };
                self.interlace = p & (1 << 5) != 0;
                self.disp_24 = p & (1 << 4) != 0;
                self.update_stat();
            }
            0x06 => {
                self.range_x1 = p & 0xFFF;
                self.range_x2 = (p >> 12) & 0xFFF;
            }
            0x07 => {
                self.range_y1 = p & 0x3FF;
                self.range_y2 = (p >> 10) & 0x3FF;
            }
            0x09 => {
                self.vram_2m = p & 1 != 0;
            }
            0x10 => {
                self.gpuread = match p & 7 {
                    2 => 0,
                    3 => (self.draw_x1 as u32) | ((self.draw_y1 as u32) << 10),
                    4 => (self.draw_x2 as u32) | ((self.draw_y2 as u32) << 10),
                    5 => (self.off_x as u32 & 0x7FF) | ((self.off_y as u32 & 0x7FF) << 11),
                    7 => 2,
                    _ => self.gpuread,
                };
            }
            _ => {}
        }
    }

    pub fn read_gpuread(&mut self) -> u32 {
        if self.transfer.as_ref().is_some_and(|t| !t.to_vram) {
            let (mut x, mut y, w, base_x) = {
                let t = self.transfer.as_ref().unwrap();
                (t.cur_x, t.cur_y, t.w, t.x)
            };
            let lo = self.read_half(x, y);
            x += 1;
            if x >= base_x + w {
                x = base_x;
                y += 1;
            }
            let hi = self.read_half(x, y);
            x += 1;
            if x >= base_x + w {
                x = base_x;
                y += 1;
            }
            if let Some(t) = self.transfer.as_mut() {
                t.cur_x = x;
                t.cur_y = y;
                t.remaining = t.remaining.saturating_sub(1);
                if t.remaining == 0 {
                    self.transfer = None;
                }
            }
            self.update_stat();
            u32::from(lo) | (u32::from(hi) << 16)
        } else {
            self.gpuread
        }
    }

    pub fn dma_read(&mut self) -> u32 {
        self.read_gpuread()
    }

    fn fill(&mut self) {
        let color24 = self.gp0_buf[0] & 0xFF_FFFF;
        let r = ((color24 & 0xFF) >> 3) as u16;
        let g = (((color24 >> 8) & 0xFF) >> 3) as u16;
        let b = (((color24 >> 16) & 0xFF) >> 3) as u16;
        let pix = r | (g << 5) | (b << 10);
        let xy = self.gp0_buf[1];
        let wh = self.gp0_buf[2];
        let x = (xy & 0x3F0) as i32;
        let y = ((xy >> 16) & 0x1FF) as i32;
        let w = ((((wh & 0x3FF) + 0xF) & !0xF) as i32).min(0x400);
        let h = ((wh >> 16) & 0x1FF) as i32;
        for yy in y..y + h {
            for xx in x..x + w {
                self.write_half(xx as u32, yy as u32, pix);
            }
        }
        // SPX: Fill is done in 16-pixel units, not one fragment per cycle.
        let units = ((w as u32) / 16).max(1).saturating_mul((h as u32).max(1));
        self.draw_busy = self.draw_busy.saturating_add(units);
    }

    fn polygon(&mut self) {
        let cmd = self.gp0_buf[0];
        let gouraud = cmd & (1 << 28) != 0;
        let quad = cmd & (1 << 27) != 0;
        let textured = cmd & (1 << 26) != 0;
        let nvert = if quad { 4 } else { 3 };
        let mut verts = [(0i32, 0i32, 0u32, 0u8, 0u8); 4];
        let mut raw_xy = [0u32; 4];
        let mut idx = 0;
        let color0 = self.gp0_buf[0] & 0xFF_FFFF;
        for i in 0..nvert {
            let color = if i == 0 || !gouraud {
                color0
            } else {
                idx += 1;
                self.gp0_buf[idx] & 0xFF_FFFF
            };
            idx += 1;
            let xy = self.gp0_buf[idx];
            raw_xy[i] = xy;
            let (x, y) = self.vertex_xy(xy);
            let mut u = 0u8;
            let mut v = 0u8;
            if textured {
                idx += 1;
                let uv = self.gp0_buf[idx];
                u = uv as u8;
                v = (uv >> 8) as u8;
                if i == 0 {
                    let clut = uv >> 16;
                    self.clut_x = (clut & 0x3F) << 4;
                    self.clut_y = (clut >> 6) & 0x1FF;
                }
                if i == 1 {
                    self.tex_page = uv >> 16;
                    self.tex_x = (self.tex_page & 0xF) as u8;
                    self.tex_y = ((self.tex_page >> 4) & 1) as u8;
                    self.dither = self.tex_page & (1 << 9) != 0;
                }
            }
            verts[i] = (x, y, color, u, v);
        }
        if (cmd >> 24) as u8 == 0x30 {
            self.frame_n30 += 1;
            let mut miny = i32::MAX;
            let mut maxy = i32::MIN;
            for i in 0..nvert {
                let (x, y) = (verts[i].0, verts[i].1);
                miny = miny.min(y);
                maxy = maxy.max(y);
                self.frame_x0 = self.frame_x0.min(x);
                self.frame_x1 = self.frame_x1.max(x);
                self.frame_y0 = self.frame_y0.min(y);
                self.frame_y1 = self.frame_y1.max(y);
                if x < self.draw_x1 || x > self.draw_x2 || y < self.draw_y1 || y > self.draw_y2 {
                    self.frame_n30_out += 1;
                }
                let bin = (y.clamp(0, 511) as usize) / 32;
                self.frame_y_bins[bin] += 1;
                if y > 300 {
                    self.last_hi_y_word = raw_xy[i];
                }
                let px = ((x % 512 + 512) % 512) as usize;
                let py = y.clamp(0, 239) as usize;
                let si = py * 512 + px;
                self.frame_scatter[si] = self.frame_scatter[si].saturating_add(1);
            }
            let dy = maxy - miny;
            if dy > self.frame_max_dy {
                self.frame_max_dy = dy;
            }
            if dy > 80 {
                self.frame_long30 += 1;
            }
        }
        // SPX GP0 bit24: 0=texture blended (texel×vertex/80h), 1=raw. Not RGB.0.
        let op = ((cmd >> 24) as u8).wrapping_sub(0x20) as usize;
        if op < 32 {
            self.frame_poly_op[op] += 1;
        }
        let blend = textured && cmd & (1 << 24) == 0;
        let semi = cmd & (1 << 25) != 0;
        let dither = self.dither && (gouraud || blend);
        self.tri(verts[0], verts[1], verts[2], textured, blend, semi, dither);
        if quad {
            self.tri(verts[1], verts[2], verts[3], textured, blend, semi, dither);
        }
    }

    fn tri(
        &mut self,
        a: (i32, i32, u32, u8, u8),
        b: (i32, i32, u32, u8, u8),
        c: (i32, i32, u32, u8, u8),
        textured: bool,
        blend: bool,
        semi: bool,
        dither: bool,
    ) {
        let minx = a.0.min(b.0).min(c.0).max(self.draw_x1);
        let maxx = a.0.max(b.0).max(c.0).min(self.draw_x2);
        let miny = a.1.min(b.1).min(c.1).max(self.draw_y1);
        let maxy = a.1.max(b.1).max(c.1).min(self.draw_y2);
        let area = orient(a.0, a.1, b.0, b.1, c.0, c.1);
        if area == 0 {
            return;
        }
        // SPX: polygons exceeding 1023 x or 511 y between vertices are not rendered.
        let dx = a.0.max(b.0).max(c.0) - a.0.min(b.0).min(c.0);
        let dy = a.1.max(b.1).max(c.1) - a.1.min(b.1).min(c.1);
        if dx > 1023 || dy > 511 {
            return;
        }
        // SPX: polygons are displayed up to <excluding> their lower-right
        // coordinates. A pixel on an edge (weight 0) is inside; only the
        // triangle's max vertex x/y are skipped. `(w ^ area) < 0` rejects
        // weight 0 when area < 0, which punches a 1px hole on every shared
        // edge of a clockwise mesh.
        let max_vx = a.0.max(b.0).max(c.0);
        let max_vy = a.1.max(b.1).max(c.1);
        for y in miny..=maxy {
            if y >= max_vy {
                continue;
            }
            for x in minx..=maxx {
                if x >= max_vx {
                    continue;
                }
                let w0 = orient(b.0, b.1, c.0, c.1, x, y);
                let w1 = orient(c.0, c.1, a.0, a.1, x, y);
                let w2 = orient(a.0, a.1, b.0, b.1, x, y);
                if (w0 != 0 && (w0 ^ area) < 0)
                    || (w1 != 0 && (w1 ^ area) < 0)
                    || (w2 != 0 && (w2 ^ area) < 0)
                {
                    continue;
                }
                let sum = w0 + w1 + w2;
                if sum == 0 {
                    continue;
                }
                let vr = interp(w0, w1, w2, a.2 & 0xFF, b.2 & 0xFF, c.2 & 0xFF, sum) as u32;
                let vg = interp(
                    w0,
                    w1,
                    w2,
                    (a.2 >> 8) & 0xFF,
                    (b.2 >> 8) & 0xFF,
                    (c.2 >> 8) & 0xFF,
                    sum,
                ) as u32;
                let vb = interp(
                    w0,
                    w1,
                    w2,
                    (a.2 >> 16) & 0xFF,
                    (b.2 >> 16) & 0xFF,
                    (c.2 >> 16) & 0xFF,
                    sum,
                ) as u32;
                let color = if textured {
                    let u = (i64::from(w0) * i64::from(a.3)
                        + i64::from(w1) * i64::from(b.3)
                        + i64::from(w2) * i64::from(c.3))
                        / i64::from(sum);
                    let v = (i64::from(w0) * i64::from(a.4)
                        + i64::from(w1) * i64::from(b.4)
                        + i64::from(w2) * i64::from(c.4))
                        / i64::from(sum);
                    let tex = self.sample_tex(u as u8, v as u8);
                    if blend {
                        blend_texel_dither(tex, vr, vg, vb, x, y, dither)
                    } else {
                        tex
                    }
                } else {
                    rgb888_to_555_dither(vr, vg, vb, x, y, dither)
                };
                self.plot(x, y, color, textured, semi);
            }
        }
    }

    fn line(&mut self) {
        let mut i = 0;
        let color = self.gp0_buf[0] & 0xFF_FFFF;
        i += 1;
        let mut prev = self.gp0_buf[i];
        i += 1;
        while i < self.gp0_buf.len() {
            let word = self.gp0_buf[i];
            if word & 0xF000_F000 == 0x5000_5000 {
                break;
            }
            let (x0, y0) = self.vertex_xy(prev);
            let (x1, y1) = self.vertex_xy(word);
            self.draw_line(
                x0,
                y0,
                x1,
                y1,
                rgb888_to_555(color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF),
            );
            prev = word;
            i += 1;
        }
        if self.gp0_buf.len() == 3 {
            let a = self.gp0_buf[1];
            let b = self.gp0_buf[2];
            let (ax, ay) = self.vertex_xy(a);
            let (bx, by) = self.vertex_xy(b);
            self.draw_line(
                ax,
                ay,
                bx,
                by,
                rgb888_to_555(color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF),
            );
        }
    }

    fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u16) {
        if (x0 - x1).abs() > 1023 || (y0 - y1).abs() > 511 {
            return;
        }
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.plot(x0, y0, color, false, false);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    fn rectangle(&mut self) {
        let cmd = self.gp0_buf[0];
        let size = (cmd >> 27) & 3;
        let textured = cmd & (1 << 26) != 0;
        let color = self.gp0_buf[0] & 0xFF_FFFF;
        let xy = self.gp0_buf[1];
        let (x, y) = self.vertex_xy(xy);
        let mut idx = 2;
        let (u0, v0) = if textured {
            let uv = self.gp0_buf[idx];
            idx += 1;
            let clut = uv >> 16;
            self.clut_x = (clut & 0x3F) << 4;
            self.clut_y = (clut >> 6) & 0x1FF;
            (uv as u8, (uv >> 8) as u8)
        } else {
            (0, 0)
        };
        let (w, h) = match size {
            1 => (1, 1),
            2 => (8, 8),
            3 => (16, 16),
            _ => {
                let wh = self.gp0_buf[idx];
                ((wh & 0x3FF) as i32, ((wh >> 16) & 0x1FF) as i32)
            }
        };
        let pix = rgb888_to_555(color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF);
        let blend = textured && cmd & (1 << 24) == 0;
        let semi = cmd & (1 << 25) != 0;
        for yy in 0..h {
            for xx in 0..w {
                let color = if textured {
                    let tex = self.sample_tex(u0.wrapping_add(xx as u8), v0.wrapping_add(yy as u8));
                    if blend {
                        blend_texel(tex, color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF)
                    } else {
                        tex
                    }
                } else {
                    pix
                };
                self.plot(x + xx, y + yy, color, textured, semi);
            }
        }
    }

    fn vram_copy(&mut self) {
        let src = self.gp0_buf[1];
        let dst = self.gp0_buf[2];
        let wh = self.gp0_buf[3];
        let sx = src & 0x3FF;
        let sy = (src >> 16) & 0x1FF;
        let dx = dst & 0x3FF;
        let dy = (dst >> 16) & 0x1FF;
        let w = ((wh.wrapping_sub(1)) & 0x3FF) + 1;
        let h = (((wh >> 16).wrapping_sub(1)) & 0x1FF) + 1;
        for y in 0..h {
            for x in 0..w {
                let p = self.read_half((sx + x) & 0x3FF, (sy + y) & 0x1FF);
                self.write_half((dx + x) & 0x3FF, (dy + y) & 0x1FF, p);
            }
        }
    }

    fn sample_tex(&mut self, u: u8, v: u8) -> u16 {
        let page = self.tex_page;
        let tx_base = (page & 0xF) * 64;
        let ty_base = ((page >> 4) & 1) * 256;
        let mode = (page >> 7) & 3;
        // SPX: UV = (UV AND NOT (mask*8)) OR ((offset AND mask)*8)
        let mx = !(self.tex_win_mask_x << 3) & 0xFF;
        let my = !(self.tex_win_mask_y << 3) & 0xFF;
        let ox = ((self.tex_win_off_x & self.tex_win_mask_x) << 3) & 0xFF;
        let oy = ((self.tex_win_off_y & self.tex_win_mask_y) << 3) & 0xFF;
        let uu = (u32::from(u) & mx) | ox;
        let vv = (u32::from(v) & my) | oy;
        match mode {
            0 => {
                let texel = self.cached_half(tx_base + uu / 4, ty_base + vv);
                let index = (texel >> ((uu & 3) * 4)) & 0xF;
                self.cached_clut(u32::from(index))
            }
            1 => {
                let texel = self.cached_half(tx_base + uu / 2, ty_base + vv);
                let index = (texel >> ((uu & 1) * 8)) & 0xFF;
                self.cached_clut(u32::from(index))
            }
            _ => self.read_half(tx_base + uu, ty_base + vv),
        }
    }

    fn vertex_xy(&self, word: u32) -> (i32, i32) {
        let x = trunc11(sign11(word & 0x7FF) + self.off_x);
        let y = trunc11(sign11((word >> 16) & 0x7FF) + self.off_y);
        (x, y)
    }

    fn plot(&mut self, x: i32, y: i32, color: u16, textured: bool, semi: bool) {
        if x < self.draw_x1 || x > self.draw_x2 || y < self.draw_y1 || y > self.draw_y2 {
            return;
        }
        if textured && color & 0x7FFF == 0 {
            return;
        }
        // SPX: textured semi-trans blends only when the texel STP (bit15) is set.
        let do_semi = semi && (!textured || color & 0x8000 != 0);
        let mut c = color & 0x7FFF;
        if do_semi {
            let dst = self.read_half(x as u32, y as u32);
            c = blend_semi(dst, c, (self.tex_page >> 5) & 3);
        }
        if self.mask_set {
            c |= 0x8000;
        }
        self.write_half(x as u32, y as u32, c);
    }

    fn cached_half(&mut self, x: u32, y: u32) -> u16 {
        if let Some(&c) = self.tex_cache.get(&(x, y)) {
            return c;
        }
        let p = self.read_half(x, y);
        self.tex_cache.insert((x, y), p);
        p
    }

    fn cached_clut(&mut self, index: u32) -> u16 {
        let key = (self.clut_x, self.clut_y);
        if let Some((cx, cy, pal)) = self.clut_cache {
            if (cx, cy) == key {
                return pal[index as usize & 15];
            }
        }
        let mut pal = [0u16; 16];
        for i in 0..16 {
            pal[i] = self.read_half(self.clut_x + i as u32, self.clut_y);
        }
        let v = pal[index as usize & 15];
        self.clut_cache = Some((key.0, key.1, pal));
        v
    }

    fn read_half(&self, x: u32, y: u32) -> u16 {
        let x = (x as usize) & (VRAM_W - 1);
        if self.vram_2m && y >= 512 {
            return 0x7FFF;
        }
        let y = (y as usize) & (VRAM_H - 1);
        self.vram[y * VRAM_W + x]
    }

    fn write_half(&mut self, x: u32, y: u32, v: u16) {
        if self.vram_2m && y >= 512 {
            return;
        }
        let x = (x as usize) & (VRAM_W - 1);
        let y = (y as usize) & (VRAM_H - 1);
        if self.mask_check && self.vram[y * VRAM_W + x] & 0x8000 != 0 {
            return;
        }
        if self.plotting {
            self.draw_busy = self.draw_busy.saturating_add(self.plot_cost);
        }
        self.vram[y * VRAM_W + x] = v;
    }

    pub fn lit_texels(&self) -> usize {
        self.vram.iter().filter(|p| **p & 0x7FFF != 0).count()
    }

    pub fn lit_bbox(&self) -> Option<(u32, u32, u32, u32)> {
        let mut minx = 1024u32;
        let mut miny = 512u32;
        let mut maxx = 0u32;
        let mut maxy = 0u32;
        let mut any = false;
        for y in 0..VRAM_H {
            for x in 0..VRAM_W {
                if self.vram[y * VRAM_W + x] & 0x7FFF != 0 {
                    any = true;
                    minx = minx.min(x as u32);
                    miny = miny.min(y as u32);
                    maxx = maxx.max(x as u32);
                    maxy = maxy.max(y as u32);
                }
            }
        }
        any.then_some((minx, miny, maxx, maxy))
    }

    pub fn vram_rect(&self, x: u32, y: u32, w: u32, h: u32) -> DisplayArea {
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for yy in 0..h {
            for xx in 0..w {
                pixels.push(self.read_half(x + xx, y + yy));
            }
        }
        DisplayArea {
            width: w,
            height: h,
            pixels,
            bpp24: self.disp_24,
        }
    }

    pub fn display_origin(&self) -> (u32, u32, u32, u32, bool) {
        (
            self.display_x,
            self.display_y,
            self.display_hres,
            self.display_vres,
            self.display_enabled,
        )
    }

    pub fn draw_env(&self) -> (i32, i32, i32, i32, i32, i32) {
        (
            self.off_x,
            self.off_y,
            self.draw_x1,
            self.draw_y1,
            self.draw_x2,
            self.draw_y2,
        )
    }

    fn read_24(&self, x: u32, y: u32) -> u16 {
        let byte = (y as usize * 2048).wrapping_add(x as usize * 3);
        let mut rgb = [0u8; 3];
        for (i, c) in rgb.iter_mut().enumerate() {
            let off = byte + i;
            let half = self.read_half((off / 2) as u32 % 1024, y);
            *c = if off % 2 == 0 {
                half as u8
            } else {
                (half >> 8) as u8
            };
        }
        rgb888_to_555(u32::from(rgb[0]), u32::from(rgb[1]), u32::from(rgb[2]))
    }

    pub fn display_area(&self) -> DisplayArea {
        let w = self.display_hres.max(1).min(640);
        let h = self.vis_h();
        if !self.display_enabled {
            return DisplayArea {
                width: w,
                height: h,
                pixels: vec![0; (w * h) as usize],
                bpp24: self.disp_24,
            };
        }
        if self.crt_w == w && self.crt_h == h && self.crt.len() == (w * h) as usize {
            return DisplayArea {
                width: w,
                height: h,
                pixels: self.crt.clone(),
                bpp24: self.disp_24,
            };
        }
        DisplayArea {
            width: w,
            height: h,
            pixels: vec![0; (w * h) as usize],
            bpp24: self.disp_24,
        }
    }

    pub fn dma_write(&mut self, word: u32) {
        self.gp0(word);
    }

    fn latch_frame(&mut self) {
        if !self.display_enabled {
            return;
        }
        let w = self.display_hres.max(1).min(640);
        let h = self.vis_h();
        if self.crt_w != w || self.crt_h != h {
            self.crt_w = w;
            self.crt_h = h;
            self.crt.resize((w * h) as usize, 0);
        }
        for y in 0..h {
            let row = (y * w) as usize;
            for x in 0..w {
                self.crt[row + x as usize] = if self.disp_24 {
                    self.read_24(self.display_x + x, self.display_y + y)
                } else {
                    self.read_half(self.display_x + x, self.display_y + y)
                };
            }
        }
        self.crt_line = u32::MAX;
    }

    pub fn tick(&mut self, cycles: u32, line: u32, vblank: bool) {
        let mut left = cycles;
        while left > 0 {
            if self.draw_busy > 0 {
                let n = self.draw_busy.min(left);
                self.draw_busy -= n;
                left -= n;
                if self.draw_busy == 0 {
                    self.block28 = false;
                }
                continue;
            }
            if let Some(w) = self.fifo.pop_front() {
                self.accept_word(w);
                left -= 1;
                continue;
            }
            break;
        }
        if !vblank && self.display_enabled && line != self.crt_line {
            let w = self.display_hres.max(1).min(640);
            let h = self.vis_h();
            if self.crt_w != w || self.crt_h != h {
                self.crt_w = w;
                self.crt_h = h;
                self.crt.resize((w * h) as usize, 0);
            }
            // 480i is latched whole at vblank. Line-by-line even/odd weave of two
            // poses is what the live Display area was showing as combing.
            if h < 480 && line < h {
                let row = (line * w) as usize;
                for x in 0..w {
                    self.crt[row + x as usize] =
                        self.read_half(self.display_x + x, self.display_y + line);
                }
            }
            self.crt_line = line;
        }
        if vblank && !self.in_vblank {
            self.odd_frame = !self.odd_frame;
            self.latch_frame();
            self.last_n30 = self.frame_n30;
            self.last_x0 = self.frame_x0;
            self.last_x1 = self.frame_x1;
            self.last_y0 = self.frame_y0;
            self.last_y1 = self.frame_y1;
            self.last_n30_out = self.frame_n30_out;
            self.last_y_bins = self.frame_y_bins;
            self.last_long30 = self.frame_long30;
            self.last_max_dy = self.frame_max_dy;
            self.last_poly_op = self.frame_poly_op;
            std::mem::swap(&mut self.last_scatter, &mut self.frame_scatter);
            self.frame_scatter.fill(0);
            self.frame_poly_op = [0; 32];
            self.frame_n30 = 0;
            self.frame_n30_out = 0;
            self.frame_long30 = 0;
            self.frame_max_dy = 0;
            self.frame_y_bins = [0; 16];
            self.frame_x0 = i32::MAX;
            self.frame_x1 = i32::MIN;
            self.frame_y0 = i32::MAX;
            self.frame_y1 = i32::MIN;
        }
        self.in_vblank = vblank;
        self.scanline = line;
        self.update_stat();
    }
}

fn polygon_len(cmd: u8) -> usize {
    let gouraud = cmd & 0x10 != 0;
    let quad = cmd & 0x08 != 0;
    let tex = cmd & 0x04 != 0;
    let verts = if quad { 4 } else { 3 };
    1 + verts + if gouraud { verts - 1 } else { 0 } + if tex { verts } else { 0 }
}

fn line_len(cmd: u8) -> usize {
    if cmd & 0x08 != 0 {
        3
    } else if cmd & 0x10 != 0 {
        4
    } else {
        3
    }
}

fn rect_len(cmd: u8) -> usize {
    let size = (cmd >> 3) & 3;
    let tex = cmd & 0x04 != 0;
    2 + if tex { 1 } else { 0 } + if size == 0 { 1 } else { 0 }
}

fn orient(ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32) -> i32 {
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

fn interp(w0: i32, w1: i32, w2: i32, a: u32, b: u32, c: u32, sum: i32) -> i32 {
    ((i64::from(w0) * i64::from(a) + i64::from(w1) * i64::from(b) + i64::from(w2) * i64::from(c))
        / i64::from(sum)) as i32
}

fn rgb888_to_555(r: u32, g: u32, b: u32) -> u16 {
    rgb888_to_555_dither(r, g, b, 0, 0, false)
}

/// SPX dither matrix, applied to 8-bit RGB before the >>3 to 5-bit when the
/// texpage dither bit is on and the poly is Gouraud or texture-blended.
const DITHER: [[i32; 4]; 4] = [
    [-4, 0, -3, 1],
    [2, -2, 3, -1],
    [-3, 1, -4, 0],
    [3, -1, 2, -2],
];

fn rgb888_to_555_dither(r: u32, g: u32, b: u32, x: i32, y: i32, dither: bool) -> u16 {
    let d = if dither {
        DITHER[(y & 3) as usize][(x & 3) as usize]
    } else {
        0
    };
    let ch = |v: u32| -> u16 { ((v as i32 + d).clamp(0, 255) as u32 >> 3) as u16 };
    ch(r) | (ch(g) << 5) | (ch(b) << 10)
}

/// SPX semi-transparency modes from texpage bits 5–6, per 5-bit channel.
fn blend_semi(back: u16, fwd: u16, mode: u32) -> u16 {
    let ch = |p: u16, s: u32| (p >> s) & 0x1F;
    let mix = |b: u16, f: u16| -> u16 {
        match mode & 3 {
            0 => (b + f) / 2,
            1 => (b + f).min(0x1F),
            2 => b.saturating_sub(f),
            _ => (b + f / 4).min(0x1F),
        }
    };
    mix(ch(back, 0), ch(fwd, 0))
        | (mix(ch(back, 5), ch(fwd, 5)) << 5)
        | (mix(ch(back, 10), ch(fwd, 10)) << 10)
}

/// SPX: texture blending is (texel*vertex)/80h per channel; 80h is 1.0, FFh is ~2×.
fn blend_texel(tex: u16, r: u32, g: u32, b: u32) -> u16 {
    let tr = u32::from((tex & 0x1F) << 3);
    let tg = u32::from(((tex >> 5) & 0x1F) << 3);
    let tb = u32::from(((tex >> 10) & 0x1F) << 3);
    let out = rgb888_to_555(
        (tr * r).min(0x7F80) >> 7,
        (tg * g).min(0x7F80) >> 7,
        (tb * b).min(0x7F80) >> 7,
    );
    out | (tex & 0x8000)
}

fn blend_texel_dither(tex: u16, r: u32, g: u32, b: u32, x: i32, y: i32, dither: bool) -> u16 {
    let tr = u32::from((tex & 0x1F) << 3);
    let tg = u32::from(((tex >> 5) & 0x1F) << 3);
    let tb = u32::from(((tex >> 10) & 0x1F) << 3);
    let out = rgb888_to_555_dither(
        (tr * r).min(0x7F80) >> 7,
        (tg * g).min(0x7F80) >> 7,
        (tb * b).min(0x7F80) >> 7,
        x,
        y,
        dither,
    );
    out | (tex & 0x8000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(gpu: &mut Gpu) {
        gpu.tick(4_000_000, 0, false);
    }

    fn upload_clut(gpu: &mut Gpu, x: u32, y: u32, colors: [u16; 16]) {
        gpu.gp0(0xA0 << 24);
        gpu.gp0(x | (y << 16));
        gpu.gp0(16 | (1 << 16));
        for pair in colors.chunks(2) {
            gpu.gp0(u32::from(pair[0]) | (u32::from(pair[1]) << 16));
        }
        settle(gpu);
    }

    fn peek(gpu: &mut Gpu, x: u32, y: u32, w: u32, h: u32) -> DisplayArea {
        settle(gpu);
        gpu.vram_rect(x, y, w, h)
    }

    #[test]
    fn fill_stays_busy_one_cycle_per_pixel() {
        let mut gpu = Gpu::new();
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4_0000_00 | 1023 | (511 << 10));
        gpu.gp0(0x02 << 24);
        gpu.gp0(0);
        gpu.gp0(16 | (16 << 16));
        gpu.tick(3, 0, false);
        assert!(gpu.busy(), "16×16 fill must not finish in the issue cycles");
        let stat = gpu.stat();
        assert_eq!(stat & (1 << 26), 0, "GPUSTAT.26 clear while drawing");
        gpu.tick(16, 0, false);
        assert!(
            !gpu.busy(),
            "Fill VRAM is one cycle per 16-pixel unit (16×16 = 16 cycles)"
        );
    }

    fn scan_field(gpu: &mut Gpu) {
        for line in 0..243 {
            gpu.tick(1, line, false);
        }
        for line in 243..263 {
            gpu.tick(1, line, true);
        }
    }

    #[test]
    fn interlaced_480_latches_both_fields() {
        let mut gpu = Gpu::new();
        gpu.gp1(0x08 << 24 | 7); // 640×480
        gpu.gp1(0x05 << 24); // start (0,0)
        gpu.gp1(0x03 << 24); // display on
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4_0000_00 | 1023 | (511 << 10));
        gpu.gp0(0x02 << 24 | 0x0000F8);
        gpu.gp0(400u32 << 16);
        gpu.gp0(16 | (16 << 16));
        settle(&mut gpu);
        scan_field(&mut gpu);
        let first = gpu.display_area();
        assert_eq!((first.width, first.height), (640, 480));
        scan_field(&mut gpu);
        let pix = gpu.display_area();
        let p = pix.pixels[400 * 640 + 8] & 0x7FFF;
        assert_eq!(
            p, 0x001F,
            "480i must latch VRAM y=400 into the Display area after both fields (got {p:#06X})"
        );
        let top = pix.pixels[0] & 0x7FFF;
        assert_eq!(top, 0, "y=0 was not filled, must stay black");
    }

    #[test]
    fn fifo_holds_sixteen_words() {
        let mut gpu = Gpu::new();
        for i in 0..16 {
            assert!(!gpu.fifo_full(), "fifo filled early at {i}");
            gpu.gp0(0xE1 << 24);
        }
        assert!(gpu.fifo_full());
        assert_eq!(
            gpu.stat() & (1 << 28),
            0,
            "GPUSTAT.28 clear while FIFO is not empty"
        );
    }

    #[test]
    fn draw_offset_does_not_take_fifo_space() {
        let mut gpu = Gpu::new();
        for _ in 0..16 {
            gpu.gp0(0xE1 << 24);
        }
        assert!(gpu.fifo_full());
        gpu.gp0(0xE5 << 24);
        assert!(
            gpu.fifo_full(),
            "E3–E5 must not evict or occupy a FIFO slot"
        );
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4 << 24 | 1023 | (511 << 10));
        assert!(gpu.fifo_full());
        assert_eq!(gpu.draw_env().2, 0);
        assert_eq!(gpu.draw_env().4, 1023);
    }

    #[test]
    fn seventeenth_gp0_word_waits_instead_of_dropping() {
        let mut gpu = Gpu::new();
        for _ in 0..16 {
            gpu.gp0(0xE1 << 24);
        }
        assert!(gpu.fifo_full());
        gpu.gp0(0xE1 << 24 | 0xF);
        gpu.tick(32, 0, false);
        assert!(!gpu.busy());
        assert_eq!(gpu.stat() & 0xF, 0xF, "last E1 must still run");
    }

    #[test]
    fn untextured_rect_stays_busy_one_cycle_per_pixel() {
        let mut gpu = Gpu::new();
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4_0000_00 | 1023 | (511 << 10));
        gpu.gp0(0x60 << 24 | 0x00F800);
        gpu.gp0(0);
        gpu.gp0(8 | (8 << 16));
        gpu.tick(3, 0, false);
        assert!(gpu.busy(), "8×8 rect must not finish in the issue cycles");
        gpu.tick(8 * 8, 0, false);
        assert!(!gpu.busy(), "untextured rect is one cycle per pixel");
    }

    #[test]
    fn textured_rect_stays_busy_two_cycles_per_pixel() {
        let mut gpu = Gpu::new();
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4_0000_00 | 1023 | (511 << 10));
        let mut clut = [0u16; 16];
        clut[1] = 0x7FFF;
        upload_clut(&mut gpu, 0, 0, clut);
        gpu.gp0(0xA0 << 24);
        gpu.gp0(64);
        gpu.gp0(2 | (8 << 16));
        for _ in 0..8 {
            gpu.gp0(0x1111_1111);
        }
        settle(&mut gpu);
        gpu.gp0(0xE1 << 24 | 1); // 4-bit page at x=64
        gpu.tick(1, 0, false);
        gpu.gp0(0x64 << 24 | 0x808080);
        gpu.gp0(0);
        gpu.gp0(0); // uv 0,0 clut 0
        gpu.gp0(8 | (8 << 16));
        gpu.tick(4, 0, false);
        assert!(
            gpu.busy(),
            "textured 8×8 must not finish in the issue cycles"
        );
        gpu.tick(8 * 8, 0, false);
        assert!(
            gpu.busy(),
            "textured commands cost two cycles per written pixel"
        );
        gpu.tick(8 * 8 + 8, 0, false);
        assert!(!gpu.busy());
    }

    #[test]
    fn polygon_command_word_clears_gpustat_bit28() {
        let mut gpu = Gpu::new();
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.tick(1, 0, false);
        assert_eq!(
            gpu.stat() & (1 << 28),
            0,
            "SPX: bit 28 clears on the polygon command word, before vertices"
        );
        assert_eq!(
            gpu.stat() & (1 << 26),
            0,
            "GPUSTAT.26 clear while assembling"
        );
    }

    #[test]
    fn textured_quad_samples_4bit_clut() {
        let mut gpu = Gpu::new();
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4_0000_00 | 639 | (479 << 10));
        let mut clut = [0u16; 16];
        clut[1] = 0x7FFF;
        clut[2] = 0x001F;
        upload_clut(&mut gpu, 192, 480, clut);
        // 4-bit page at x=832 (page 0xD): 8×8 texels of index 1 (0x1111 per halfword).
        gpu.gp0(0xA0 << 24);
        gpu.gp0(832);
        gpu.gp0(2 | (8 << 16));
        for _ in 0..8 {
            gpu.gp0(0x1111_1111);
        }

        // GP0(2C) 8×8 sprite at (10,10), UVs covering the two texels.
        gpu.gp0(0x2C80_8080);
        gpu.gp0(10 | (10 << 16));
        gpu.gp0((0x780C << 16) | 0x0000); // clut (192,480), uv 0,0
        gpu.gp0(18 | (10 << 16));
        gpu.gp0((0x000D << 16) | 0x0008); // page 0xD, uv 8,0
        gpu.gp0(10 | (18 << 16));
        gpu.gp0(0x0008_00);
        gpu.gp0(18 | (18 << 16));
        gpu.gp0(0x0008_08);

        let pix = peek(&mut gpu, 10, 10, 8, 8);
        let lit = pix.pixels.iter().filter(|p| **p & 0x7FFF != 0).count();
        assert!(
            lit > 8,
            "4-bit 2C sprite must plot CLUT colors (lit={lit} pixels={:04X?})",
            pix.pixels
        );
        assert!(
            pix.pixels.iter().any(|p| *p & 0x7FFF == 0x7FFF)
                || pix.pixels.iter().any(|p| *p & 0x7FFF == 0x001F),
            "sprite pixels should include CLUT entries 1 or 2 ({:04X?})",
            pix.pixels
        );
    }

    #[test]
    fn textured_quad_covers_bios_sce_sprite() {
        let mut gpu = Gpu::new();
        gpu.gp0(0xE3_0000_00);
        gpu.gp0(0xE4_0000_00 | 639 | (479 << 10));
        let mut clut = [0u16; 16];
        clut[1] = 0x7FFF;
        upload_clut(&mut gpu, 192, 480, clut);
        // Page 0xD at x=832: 60 halfwords × 48 rows of index 1 (240×48 4-bit texels).
        gpu.gp0(0xA0 << 24);
        gpu.gp0(832);
        gpu.gp0(60 | (48 << 16));
        for _ in 0..((60 * 48 + 1) / 2) {
            gpu.gp0(0x1111_1111);
        }
        gpu.gp0(0x2C80_8080);
        gpu.gp0(200 | (56 << 16));
        gpu.gp0((0x780C << 16) | 0x0000);
        gpu.gp0(440 | (56 << 16));
        gpu.gp0((0x000D << 16) | 239);
        gpu.gp0(200 | (104 << 16));
        gpu.gp0(47 << 8);
        gpu.gp0(440 | (104 << 16));
        gpu.gp0(239 | (47 << 8));
        let pix = peek(&mut gpu, 200, 56, 240, 48);
        let lit = pix.pixels.iter().filter(|p| **p & 0x7FFF == 0x7FFF).count();
        assert!(
            lit > 5_000,
            "BIOS-sized 2C sprite must cover the dest rect (lit={lit} / 11520)"
        );
    }

    fn xy(x: i32, y: i32) -> u32 {
        (x as u16 as u32) | ((y as u16 as u32) << 16)
    }

    /// 11-bit X/Y with unused bits 11–15 / 27–31 set (would parse as i16 ≈ −2038).
    fn xy_unused_junk(x: i32, y: i32) -> u32 {
        let x11 = (x as u32) & 0x7FF;
        let y11 = (y as u32) & 0x7FF;
        (x11 | 0xF800) | ((y11 | 0xF800) << 16)
    }

    fn clip(gpu: &mut Gpu, x1: i32, y1: i32, x2: i32, y2: i32) {
        gpu.gp0(0xE3 << 24 | (x1 as u32 & 0x3FF) | ((y1 as u32 & 0x1FF) << 10));
        gpu.gp0(0xE4 << 24 | (x2 as u32 & 0x3FF) | ((y2 as u32 & 0x1FF) << 10));
    }

    fn offset(gpu: &mut Gpu, x: i32, y: i32) {
        gpu.gp0(0xE5 << 24 | (x as u32 & 0x7FF) | ((y as u32 & 0x7FF) << 11));
    }

    fn red_count(pixels: &[u16]) -> usize {
        pixels.iter().filter(|p| *p & 0x7FFF == 0x001F).count()
    }

    fn blue_count(pixels: &[u16]) -> usize {
        pixels.iter().filter(|p| *p & 0x7FFF == 0x7C00).count()
    }

    #[test]
    fn vertex_xy_ignores_unused_bits_that_would_sign_extend_as_i16() {
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        // GP0(20) red triangle at (10,10)-(20,10)-(10,20). Junk bits make i16 X = −2038.
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy_unused_junk(10, 10));
        gpu.gp0(xy_unused_junk(20, 10));
        gpu.gp0(xy_unused_junk(10, 20));
        let pix = peek(&mut gpu, 10, 10, 10, 10);
        let lit = red_count(&pix.pixels);
        assert!(
            lit > 8,
            "signed 11-bit verts must plot at (10,10), not the i16 junk (lit={lit})"
        );
    }

    #[test]
    fn drawing_offset_is_added_then_truncated_to_signed_11bit() {
        // vertex −600 + offset −512 = −1112, trunc11 → 936. Without trunc the
        // pixel is clipped (x < 0).
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, -512, 0);
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(-600, 10));
        gpu.gp0(xy(-580, 10));
        gpu.gp0(xy(-600, 30));
        let pix = peek(&mut gpu, 930, 10, 20, 20);
        let lit = red_count(&pix.pixels);
        assert!(
            lit > 8,
            "offset then trunc11 must land the triangle at x≈936 (lit={lit})"
        );
    }

    #[test]
    fn both_polygon_windings_plot() {
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        // CCW red.
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(20, 10));
        gpu.gp0(xy(40, 10));
        gpu.gp0(xy(20, 30));
        // CW blue over the same triangle.
        gpu.gp0(0x20 << 24 | 0xF80000);
        gpu.gp0(xy(20, 10));
        gpu.gp0(xy(20, 30));
        gpu.gp0(xy(40, 10));
        let pix = peek(&mut gpu, 20, 10, 20, 20);
        let blue = blue_count(&pix.pixels);
        assert!(
            blue > 8,
            "GPU draws both windings; CW overdraw must leave blue (blue={blue})"
        );
    }

    #[test]
    fn quad_splits_as_vertices_1_2_3_then_2_3_4() {
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0x28 << 24 | 0x0000F8);
        gpu.gp0(xy(10, 10));
        gpu.gp0(xy(20, 10));
        gpu.gp0(xy(10, 20));
        gpu.gp0(xy(50, 50));
        let first = peek(&mut gpu, 11, 11, 4, 4);
        let second = peek(&mut gpu, 42, 42, 8, 8);
        let lit_first = red_count(&first.pixels);
        let lit_second = red_count(&second.pixels);
        assert!(
            lit_first > 0,
            "first tri (v0,v1,v2) must fill near (10,10) (lit={lit_first})"
        );
        assert!(
            lit_second > 0,
            "second tri (v1,v2,v3) must fill near v3 (50,50) (lit={lit_second})"
        );
    }

    #[test]
    fn oversized_polygon_is_not_rendered() {
        // SPX: max vertex distance 1023 x, 511 y. A 600-px-tall tri is dropped;
        // a 500-px-tall tri is drawn.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(20, -300));
        gpu.gp0(xy(40, -300));
        gpu.gp0(xy(20, 300));
        assert_eq!(
            red_count(&peek(&mut gpu, 0, 0, 64, 64).pixels),
            0,
            "triangle taller than 511 must not plot"
        );
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(80, -100));
        gpu.gp0(xy(100, -100));
        gpu.gp0(xy(80, 400));
        let lit = red_count(&peek(&mut gpu, 80, 0, 24, 64).pixels);
        assert!(
            lit > 8,
            "triangle within 511 vertical must still plot (lit={lit})"
        );
    }

    #[test]
    fn gouraud_triangle_interpolates_per_vertex_rgb() {
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0x30 << 24 | 0x0000F8); // v0 red
        gpu.gp0(xy(0, 0));
        gpu.gp0(0x0000F8); // v1 red (same)
        gpu.gp0(xy(40, 0));
        gpu.gp0(0xF80000); // v2 blue
        gpu.gp0(xy(0, 40));
        let pix = peek(&mut gpu, 2, 20, 8, 8);
        let mixed = pix.pixels.iter().any(|p| {
            let r = p & 0x1F;
            let b = (p >> 10) & 0x1F;
            r > 2 && b > 2
        });
        assert!(
            mixed,
            "GP0(30h) must interpolate red and blue (got {:04X?})",
            pix.pixels
        );
    }

    #[test]
    fn textured_gouraud_blends_texel_by_vertex_rgb() {
        // SPX: blended textured polys multiply texel by vertex colour / 80h.
        // White CLUT texel * 40h must land near half brightness, not stay 7FFF.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        let mut clut = [0u16; 16];
        clut[1] = 0x7FFF;
        upload_clut(&mut gpu, 0, 0, clut);
        gpu.gp0(0xA0 << 24);
        gpu.gp0(64); // page 1 at x=64
        gpu.gp0(2 | (8 << 16));
        for _ in 0..8 {
            gpu.gp0(0x1111_1111);
        }
        gpu.gp0(0x34 << 24 | 0x00404040);
        gpu.gp0(xy(10, 10));
        gpu.gp0(0x0000); // uv 0,0 clut (0,0)
        gpu.gp0(0x00404040);
        gpu.gp0(xy(26, 10));
        gpu.gp0((1 << 16) | 0x0008); // tpage 1, uv 8,0
        gpu.gp0(0x00404040);
        gpu.gp0(xy(10, 26));
        gpu.gp0(0x0008_00);
        let pix = peek(&mut gpu, 12, 12, 8, 8);
        let half = pix
            .pixels
            .iter()
            .filter(|p| {
                let r = *p & 0x1F;
                let g = (*p >> 5) & 0x1F;
                let b = (*p >> 10) & 0x1F;
                r > 4 && r < 0x1C && g > 4 && g < 0x1C && b > 4 && b < 0x1C
            })
            .count();
        assert!(
            half > 4,
            "GP0(34h) 40h blend must dim a white texel (got {:04X?})",
            pix.pixels
        );
    }

    #[test]
    fn texture_blend_is_command_bit24_not_rgb_lsb() {
        // SPX bit24: 0=blended. RGB.0 of 41h must not switch the poly to raw.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        let mut clut = [0u16; 16];
        clut[1] = 0x7FFF;
        upload_clut(&mut gpu, 0, 0, clut);
        gpu.gp0(0xA0 << 24);
        gpu.gp0(64);
        gpu.gp0(2 | (8 << 16));
        for _ in 0..8 {
            gpu.gp0(0x1111_1111);
        }
        gpu.gp0(0x34 << 24 | 0x00404041);
        gpu.gp0(xy(10, 10));
        gpu.gp0(0x0000);
        gpu.gp0(0x00404041);
        gpu.gp0(xy(26, 10));
        gpu.gp0((1 << 16) | 0x0008);
        gpu.gp0(0x00404041);
        gpu.gp0(xy(10, 26));
        gpu.gp0(0x0008_00);
        let pix = peek(&mut gpu, 12, 12, 8, 8);
        let half = pix
            .pixels
            .iter()
            .filter(|p| {
                let r = *p & 0x1F;
                r > 4 && r < 0x1C
            })
            .count();
        assert!(
            half > 4,
            "GP0(34h) with odd RGB.0 must still blend (got {:04X?})",
            pix.pixels
        );
    }

    #[test]
    fn polygon_excludes_lower_right_vertex_coordinates() {
        // SPX: displayed up to <excluding> lower-right coordinates.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(10, 10));
        gpu.gp0(xy(14, 10));
        gpu.gp0(xy(10, 14));
        let right = red_count(&peek(&mut gpu, 14, 10, 1, 4).pixels);
        let bottom = red_count(&peek(&mut gpu, 10, 14, 4, 1).pixels);
        let interior = red_count(&peek(&mut gpu, 10, 10, 3, 3).pixels);
        assert_eq!(right, 0, "x=max vertex must not plot");
        assert_eq!(bottom, 0, "y=max vertex must not plot");
        assert!(interior > 0, "interior of the triangle must still plot");
    }

    #[test]
    fn quad_split_leaves_no_holes_on_the_shared_diagonal() {
        // SPX: a quad is triangles (v1,v2,v3) and (v2,v3,v4). Lower-right
        // coordinates are excluded, so [2,10)×[2,10) must be solid.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(2, 2));
        gpu.gp0(xy(10, 2));
        gpu.gp0(xy(2, 10));
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(10, 2));
        gpu.gp0(xy(2, 10));
        gpu.gp0(xy(10, 10));
        let pix = peek(&mut gpu, 2, 2, 8, 8);
        let missing = pix.pixels.iter().filter(|p| *p & 0x7FFF == 0).count();
        assert_eq!(
            missing, 0,
            "shared diagonal must not leave holes ({missing} of 64 unfilled)"
        );
    }

    #[test]
    fn clockwise_pair_sharing_a_diagonal_leaves_no_holes() {
        // Both triangles clockwise (area < 0). Crash title keeps negative
        // NCLIP, so the face mesh is this case. SPX only excludes the
        // lower-right vertex coordinates; a pixel on the shared edge is in.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(2, 2));
        gpu.gp0(xy(2, 10));
        gpu.gp0(xy(10, 2));
        gpu.gp0(0x20 << 24 | 0x0000F8);
        gpu.gp0(xy(10, 2));
        gpu.gp0(xy(2, 10));
        gpu.gp0(xy(10, 10));
        let pix = peek(&mut gpu, 2, 2, 8, 8);
        let missing = pix.pixels.iter().filter(|p| *p & 0x7FFF == 0).count();
        assert_eq!(
            missing, 0,
            "clockwise shared diagonal must not leave holes ({missing} of 64 unfilled)"
        );
    }

    #[test]
    fn semi_trans_mode0_averages_back_and_forward() {
        // SPX mode 0: 0.5×B + 0.5×F per 5-bit channel.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0xE1 << 24); // texpage, semi mode 0
        gpu.gp0(0x02 << 24 | 0xF80000); // fill blue
        gpu.gp0(0);
        gpu.gp0(32 | (32 << 16));
        gpu.gp0(0x22 << 24 | 0x0000F8); // semi-trans red tri
        gpu.gp0(xy(2, 2));
        gpu.gp0(xy(20, 2));
        gpu.gp0(xy(2, 20));
        let p = peek(&mut gpu, 4, 4, 1, 1).pixels[0] & 0x7FFF;
        let r = p & 0x1F;
        let b = (p >> 10) & 0x1F;
        assert!(
            r > 8 && r < 24 && b > 8 && b < 24,
            "mode 0 must mix red and blue (got {p:#06X})"
        );
    }

    #[test]
    fn textured_semi_without_stp_stays_opaque() {
        // SPX: textured semi-trans with texel bit15=0 is opaque, not averaged.
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        gpu.gp0(0xE1 << 24);
        gpu.gp0(0x02 << 24 | 0xF80000);
        gpu.gp0(0);
        gpu.gp0(32 | (32 << 16));
        let mut clut = [0u16; 16];
        clut[1] = 0x001F; // red, STP=0
        upload_clut(&mut gpu, 0, 0, clut);
        gpu.gp0(0xA0 << 24);
        gpu.gp0(64);
        gpu.gp0(2 | (8 << 16));
        for _ in 0..8 {
            gpu.gp0(0x1111_1111);
        }
        gpu.gp0(0x26 << 24 | 0x808080); // textured semi tri, blended
        gpu.gp0(xy(2, 2));
        gpu.gp0(0x0000);
        gpu.gp0(xy(18, 2));
        gpu.gp0((1 << 16) | 0x0008);
        gpu.gp0(xy(2, 18));
        gpu.gp0(0x0008_00);
        let p = peek(&mut gpu, 4, 4, 1, 1).pixels[0] & 0x7FFF;
        assert_eq!(
            p, 0x001F,
            "STP=0 texel must stay opaque red, not mix ({p:#06X})"
        );
    }

    #[test]
    fn gouraud_dither_changes_a_pixel() {
        // SPX: dither (texpage bit 9) applies to Gouraud polys.
        let mut without = Gpu::new();
        clip(&mut without, 0, 0, 1023, 511);
        offset(&mut without, 0, 0);
        without.gp0(0xE1 << 24);
        without.gp0(0x30 << 24 | 0x0000F8);
        without.gp0(xy(0, 0));
        without.gp0(0x0000F8);
        without.gp0(xy(8, 0));
        without.gp0(0xF80000);
        without.gp0(xy(0, 8));
        let mut with = Gpu::new();
        clip(&mut with, 0, 0, 1023, 511);
        offset(&mut with, 0, 0);
        with.gp0(0xE1 << 24 | (1 << 9));
        with.gp0(0x30 << 24 | 0x0000F8);
        with.gp0(xy(0, 0));
        with.gp0(0x0000F8);
        with.gp0(xy(8, 0));
        with.gp0(0xF80000);
        with.gp0(xy(0, 8));
        let a = peek(&mut without, 1, 3, 4, 4).pixels;
        let b = peek(&mut with, 1, 3, 4, 4).pixels;
        assert_ne!(a, b, "dither bit must change Gouraud pixels");
    }

    #[test]
    fn texture_window_offset_is_anded_with_mask() {
        // SPX: UV = (UV AND NOT (mask*8)) OR ((offset AND mask)*8).
        // Mask 0 and offset 1 must leave UV unchanged (not OR offset*8).
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        let mut clut = [0u16; 16];
        clut[1] = 0x001F;
        upload_clut(&mut gpu, 0, 480, clut);
        gpu.gp0(0xA0 << 24);
        gpu.gp0(0);
        gpu.gp0(2 | (1 << 16));
        gpu.gp0(0x0000_0001);
        gpu.gp0(0xE2 << 24 | 1u32 << 10); // mask=0, offset_x=1
        gpu.gp0(0xE1 << 24);
        gpu.gp0(0x65 << 24 | 0x808080); // raw textured rect
        gpu.gp0(xy(8, 8));
        gpu.gp0((480u32 << 6) << 16); // uv 0,0 clut y=480
        gpu.gp0(8 | (8 << 16));
        let p = peek(&mut gpu, 8, 8, 1, 1).pixels[0] & 0x7FFF;
        assert_eq!(
            p, 0x001F,
            "E2 offset without mask must not remap UV 0 (got {p:#06X})"
        );
    }

    #[test]
    fn gpustat_25_follows_gp1_04h_dma_direction() {
        let mut gpu = Gpu::new();
        gpu.gp1(0x04 << 24);
        assert_eq!(
            gpu.stat() & (1 << 25),
            0,
            "GP1(04h)=0: GPUSTAT.25 is always 0"
        );
        gpu.gp1(0x04 << 24 | 1);
        assert_ne!(
            gpu.stat() & (1 << 25),
            0,
            "GP1(04h)=1: FIFO half-empty DRQ when FIFO is empty"
        );
        for _ in 0..9 {
            gpu.gp0(0x01 << 24);
        }
        assert_eq!(
            gpu.stat() & (1 << 25),
            0,
            "GP1(04h)=1: DRQ clear when FIFO is more than half full"
        );
        gpu.gp1(0x01 << 24);
        gpu.gp1(0x04 << 24 | 2);
        assert_eq!(
            (gpu.stat() >> 25) & 1,
            (gpu.stat() >> 28) & 1,
            "GP1(04h)=2: GPUSTAT.25 same as FIFO empty (bit 28)"
        );
        gpu.gp1(0x04 << 24 | 3);
        assert_eq!(
            (gpu.stat() >> 25) & 1,
            (gpu.stat() >> 27) & 1,
            "GP1(04h)=3: GPUSTAT.25 same as VRAM-to-CPU ready (bit 27)"
        );
    }

    #[test]
    fn gp0_a0_keeps_gpustat_26_while_it_still_wants_gp0_words() {
        let mut gpu = Gpu::new();
        gpu.gp0(0xA0 << 24);
        gpu.gp0(0);
        gpu.gp0(4 | (4 << 16));
        gpu.tick(16, 0, false);
        assert_ne!(
            gpu.stat() & (1 << 26),
            0,
            "GPUSTAT.26 stays set while GP0(A0) still wants GP0 words"
        );
        gpu.gp0(0);
        gpu.tick(4, 0, false);
        assert_ne!(
            gpu.stat() & (1 << 26),
            0,
            "GPUSTAT.26 stays set after a partial A0 payload"
        );
    }

    #[test]
    fn gpustat_31_toggles_per_scanline_in_240_per_frame_in_480_zero_in_vblank() {
        let mut gpu = Gpu::new();
        gpu.tick(1, 0, false);
        assert_eq!(gpu.stat() >> 31, 0, "240-line even scanline: GPUSTAT.31=0");
        gpu.tick(1, 1, false);
        assert_eq!(gpu.stat() >> 31, 1, "240-line odd scanline: GPUSTAT.31=1");
        gpu.tick(1, 243, true);
        assert_eq!(gpu.stat() >> 31, 0, "GPUSTAT.31 is 0 in vblank");

        let mut gpu = Gpu::new();
        gpu.gp1(0x08 << 24 | 4);
        gpu.tick(1, 0, false);
        assert_eq!(gpu.stat() >> 31, 0, "480-line first field even");
        gpu.tick(1, 1, false);
        assert_eq!(gpu.stat() >> 31, 0, "480-line does not toggle per scanline");
        gpu.tick(1, 243, true);
        assert_eq!(gpu.stat() >> 31, 0, "480-line vblank: GPUSTAT.31=0");
        gpu.tick(1, 0, false);
        assert_eq!(gpu.stat() >> 31, 1, "480-line next field: GPUSTAT.31=1");
    }

    #[test]
    fn gp1_00h_reset_gpustat_is_14802000h() {
        let mut gpu = Gpu::new();
        gpu.gp1(0x08 << 24 | 7);
        gpu.gp1(0x04 << 24 | 2);
        gpu.gp1(0);
        assert_eq!(
            gpu.stat(),
            0x1480_2000,
            "GP1(00h) must restore GPUSTAT 14802000h (got {:08X})",
            gpu.stat()
        );
    }

    #[test]
    fn gpustat_bit13_is_one_when_vertical_interlace_is_off() {
        let mut gpu = Gpu::new();
        assert_ne!(
            gpu.stat() & (1 << 13),
            0,
            "GPUSTAT.13 is 1 when GP1(08h) interlace is off"
        );
        gpu.gp1(0x08 << 24 | (1 << 5) | 4);
        gpu.tick(1, 0, false);
        assert_eq!(
            gpu.stat() & (1 << 13),
            0,
            "GPUSTAT.13 follows the interlace field when GP1(08h).5 is on"
        );
    }

    fn draw_4bpp_dot(gpu: &mut Gpu, clut_y: u32) {
        gpu.gp0(0xE1 << 24);
        gpu.gp0(0x65 << 24 | 0x808080);
        gpu.gp0(xy(8, 8));
        gpu.gp0((clut_y << 6) << 16);
        gpu.gp0(1 | (1 << 16));
        settle(gpu);
    }

    #[test]
    fn gp0_01h_discards_clut_cache_fill_does_not() {
        let mut gpu = Gpu::new();
        clip(&mut gpu, 0, 0, 1023, 511);
        offset(&mut gpu, 0, 0);
        let mut clut = [0u16; 16];
        clut[1] = 0x001F;
        upload_clut(&mut gpu, 0, 480, clut);
        gpu.gp0(0xA0 << 24);
        gpu.gp0(0);
        gpu.gp0(2 | (1 << 16));
        gpu.gp0(0x0000_0001);
        settle(&mut gpu);
        draw_4bpp_dot(&mut gpu, 480);
        assert_eq!(peek(&mut gpu, 8, 8, 1, 1).pixels[0] & 0x7FFF, 0x001F);
        clut[1] = 0x03E0;
        upload_clut(&mut gpu, 0, 480, clut);
        gpu.gp0(0x02 << 24 | 0x0000F8);
        gpu.gp0(400u32 << 16);
        gpu.gp0(16 | (16 << 16));
        settle(&mut gpu);
        draw_4bpp_dot(&mut gpu, 480);
        assert_eq!(
            peek(&mut gpu, 8, 8, 1, 1).pixels[0] & 0x7FFF,
            0x001F,
            "Fill and CLUT COPY leave the CLUT cache; textured draw still samples the old palette"
        );
        gpu.gp0(0x01 << 24);
        settle(&mut gpu);
        draw_4bpp_dot(&mut gpu, 480);
        assert_eq!(
            peek(&mut gpu, 8, 8, 1, 1).pixels[0] & 0x7FFF,
            0x03E0,
            "GP0(01h) discards CLUT cache so the new VRAM palette is sampled"
        );
    }

    #[test]
    fn gp0_1fh_sets_gpustat_24_gp1_02h_clears_it() {
        let mut gpu = Gpu::new();
        gpu.gp0(0x1F << 24);
        settle(&mut gpu);
        assert_ne!(gpu.stat() & (1 << 24), 0, "GP0(1Fh) sets GPUSTAT.24");
        assert!(gpu.irq_line());
        gpu.gp1(0x02 << 24);
        assert_eq!(gpu.stat() & (1 << 24), 0, "GP1(02h) acks GPUSTAT.24");
        assert!(!gpu.irq_line());
    }

    #[test]
    fn gp1_08h_24bpp_and_display_range_and_2mb() {
        let mut gpu = Gpu::new();
        gpu.gp1(0x08 << 24 | (1 << 4) | 1);
        assert_ne!(gpu.stat() & (1 << 21), 0, "GP1(08h) bit4 sets GPUSTAT.21");
        gpu.gp1(0x03 << 24);
        gpu.gp1(0x05 << 24);
        gpu.tick(1, 0, false);
        assert!(
            gpu.display_area().bpp24,
            "Display area is 24-bit when GP1(08h) bit4 is set"
        );
        gpu.gp1(0x06 << 24 | 0x200 | ((0x200 + 320 * 8) << 12));
        gpu.gp1(0x07 << 24 | 10 | (110 << 10));
        assert_eq!(gpu.display_range(), (0x200, 0x200 + 320 * 8, 10, 110));
        gpu.gp1(0x03 << 24);
        for line in 0..120 {
            gpu.tick(1, line, false);
        }
        assert_eq!(
            gpu.display_area().height,
            100,
            "GP1(07h) Y2-Y1 is the Display height"
        );
        gpu.gp1(0x09 << 24 | 1);
        assert_eq!(
            gpu.vram_rect(0, 512, 1, 1).pixels[0],
            0x7FFF,
            "GP1(09h) 2MB: VRAM Y>=200h is open bus 7FFFh"
        );
    }
}

fn sign11(v: u32) -> i32 {
    trunc11(v as i32)
}

fn trunc11(v: i32) -> i32 {
    (v << 21) >> 21
}
