use crate::DisplayArea;

const VRAM_W: usize = 1024;
const VRAM_H: usize = 512;

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
    dither: bool,
    draw_to_display: bool,
    mask_set: bool,
    mask_check: bool,
    display_x: u32,
    display_y: u32,
    display_hres: u32,
    display_vres: u32,
    display_enabled: bool,
    dma_dir: u8,
    transfer: Option<Transfer>,
    pub gp0_count: u64,
    pub gp1_count: u64,
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
            dither: false,
            draw_to_display: false,
            mask_set: false,
            mask_check: false,
            display_x: 0,
            display_y: 0,
            display_hres: 320,
            display_vres: 240,
            display_enabled: false,
            dma_dir: 0,
            transfer: None,
            gp0_count: 0,
            gp1_count: 0,
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

    fn update_stat(&mut self) {
        let mut s = 0x1C00_0000; // ready bits 26,27,28
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
        s |= 1 << 13;
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
        s |= u32::from(self.dma_dir) << 29;
        self.stat = s;
    }

    pub fn gp0(&mut self, word: u32) {
        self.gp0_count += 1;
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
                    self.transfer = None;
                }
            }
            return;
        }
        if self.gp0_cmd.is_none() {
            let cmd = (word >> 24) as u8;
            if cmd == 0 || (0x04..=0x1E).contains(&cmd) {
                return;
            }
            self.gp0_cmd = Some(cmd);
            self.gp0_buf.clear();
            self.gp0_buf.push(word);
        } else {
            self.gp0_buf.push(word);
        }
        if self.gp0_ready() {
            self.exec_gp0();
            self.gp0_cmd = None;
            self.gp0_buf.clear();
        }
    }

    fn gp0_len(cmd: u8) -> usize {
        match cmd {
            0x02 => 3,
            0x1F => 1,
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
            0x02 => self.fill(),
            0x20..=0x3F => self.polygon(),
            0x40..=0x5F => self.line(),
            0x60..=0x7F => self.rectangle(),
            0x80 => self.vram_copy(),
            0xA0 => {
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
                    to_vram: true,
                    cur_x: x,
                    cur_y: y,
                });
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
        let cmd = (word >> 24) & 0x3F;
        let p = word & 0xFF_FFFF;
        match cmd {
            0x00 => self.reset(),
            0x01 => {
                self.gp0_buf.clear();
                self.gp0_cmd = None;
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
                self.update_stat();
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
        self.gpuread
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
    }

    fn polygon(&mut self) {
        let cmd = self.gp0_buf[0];
        let gouraud = cmd & (1 << 28) != 0;
        let quad = cmd & (1 << 27) != 0;
        let textured = cmd & (1 << 26) != 0;
        let nvert = if quad { 4 } else { 3 };
        let mut verts = [(0i32, 0i32, 0u32, 0u8, 0u8); 4];
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
            let x = (xy as i16) as i32 + self.off_x;
            let y = ((xy >> 16) as i16) as i32 + self.off_y;
            let mut u = 0u8;
            let mut v = 0u8;
            if textured {
                idx += 1;
                let uv = self.gp0_buf[idx];
                u = uv as u8;
                v = (uv >> 8) as u8;
                if i == 1 {
                    self.tex_page = uv >> 16;
                }
            }
            verts[i] = (x, y, color, u, v);
        }
        self.tri(verts[0], verts[1], verts[2], textured);
        if quad {
            self.tri(verts[1], verts[2], verts[3], textured);
        }
    }

    fn tri(
        &mut self,
        a: (i32, i32, u32, u8, u8),
        b: (i32, i32, u32, u8, u8),
        c: (i32, i32, u32, u8, u8),
        textured: bool,
    ) {
        let minx = a.0.min(b.0).min(c.0).max(self.draw_x1);
        let maxx = a.0.max(b.0).max(c.0).min(self.draw_x2);
        let miny = a.1.min(b.1).min(c.1).max(self.draw_y1);
        let maxy = a.1.max(b.1).max(c.1).min(self.draw_y2);
        let area = orient(a.0, a.1, b.0, b.1, c.0, c.1);
        if area == 0 {
            return;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let w0 = orient(b.0, b.1, c.0, c.1, x, y);
                let w1 = orient(c.0, c.1, a.0, a.1, x, y);
                let w2 = orient(a.0, a.1, b.0, b.1, x, y);
                if (w0 ^ area) < 0 || (w1 ^ area) < 0 || (w2 ^ area) < 0 {
                    continue;
                }
                let sum = w0 + w1 + w2;
                if sum == 0 {
                    continue;
                }
                let color = if textured {
                    let u = (i64::from(w0) * i64::from(a.3)
                        + i64::from(w1) * i64::from(b.3)
                        + i64::from(w2) * i64::from(c.3))
                        / i64::from(sum);
                    let v = (i64::from(w0) * i64::from(a.4)
                        + i64::from(w1) * i64::from(b.4)
                        + i64::from(w2) * i64::from(c.4))
                        / i64::from(sum);
                    self.sample_tex(u as u8, v as u8)
                } else {
                    let r = interp(w0, w1, w2, a.2 & 0xFF, b.2 & 0xFF, c.2 & 0xFF, sum);
                    let g = interp(
                        w0,
                        w1,
                        w2,
                        (a.2 >> 8) & 0xFF,
                        (b.2 >> 8) & 0xFF,
                        (c.2 >> 8) & 0xFF,
                        sum,
                    );
                    let bl = interp(
                        w0,
                        w1,
                        w2,
                        (a.2 >> 16) & 0xFF,
                        (b.2 >> 16) & 0xFF,
                        (c.2 >> 16) & 0xFF,
                        sum,
                    );
                    rgb888_to_555(r as u32, g as u32, bl as u32)
                };
                self.plot(x, y, color);
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
            let x0 = (prev as i16) as i32 + self.off_x;
            let y0 = ((prev >> 16) as i16) as i32 + self.off_y;
            let x1 = (word as i16) as i32 + self.off_x;
            let y1 = ((word >> 16) as i16) as i32 + self.off_y;
            self.draw_line(x0, y0, x1, y1, rgb888_to_555(color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF));
            prev = word;
            i += 1;
        }
        if self.gp0_buf.len() == 3 {
            let a = self.gp0_buf[1];
            let b = self.gp0_buf[2];
            self.draw_line(
                (a as i16) as i32 + self.off_x,
                ((a >> 16) as i16) as i32 + self.off_y,
                (b as i16) as i32 + self.off_x,
                ((b >> 16) as i16) as i32 + self.off_y,
                rgb888_to_555(color & 0xFF, (color >> 8) & 0xFF, (color >> 16) & 0xFF),
            );
        }
    }

    fn draw_line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: u16) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.plot(x0, y0, color);
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
        let x = (xy as i16) as i32 + self.off_x;
        let y = ((xy >> 16) as i16) as i32 + self.off_y;
        let mut idx = 2;
        let (u0, v0) = if textured {
            let uv = self.gp0_buf[idx];
            idx += 1;
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
        for yy in 0..h {
            for xx in 0..w {
                let color = if textured {
                    self.sample_tex(u0.wrapping_add(xx as u8), v0.wrapping_add(yy as u8))
                } else {
                    pix
                };
                self.plot(x + xx, y + yy, color);
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

    fn sample_tex(&self, u: u8, v: u8) -> u16 {
        let tx = (u32::from(self.tex_x) * 64 + u32::from(u)) & 0x3FF;
        let ty = (u32::from(self.tex_y) * 256 + u32::from(v)) & 0x1FF;
        let p = self.read_half(tx, ty);
        if p == 0 {
            0x8000 // skip later? 0 is transparent
        } else {
            p
        }
    }

    fn plot(&mut self, x: i32, y: i32, color: u16) {
        if color == 0 && (color & 0x8000) == 0 {
            // fully transparent texture
        }
        if x < self.draw_x1 || x > self.draw_x2 || y < self.draw_y1 || y > self.draw_y2 {
            return;
        }
        if color == 0 {
            return;
        }
        let mut c = color & 0x7FFF;
        if self.mask_set {
            c |= 0x8000;
        }
        self.write_half(x as u32, y as u32, c);
    }

    fn read_half(&self, x: u32, y: u32) -> u16 {
        let x = (x as usize) & (VRAM_W - 1);
        let y = (y as usize) & (VRAM_H - 1);
        self.vram[y * VRAM_W + x]
    }

    fn write_half(&mut self, x: u32, y: u32, v: u16) {
        let x = (x as usize) & (VRAM_W - 1);
        let y = (y as usize) & (VRAM_H - 1);
        if self.mask_check && self.vram[y * VRAM_W + x] & 0x8000 != 0 {
            return;
        }
        self.vram[y * VRAM_W + x] = v;
    }

    pub fn lit_texels(&self) -> usize {
        self.vram.iter().filter(|p| **p & 0x7FFF != 0).count()
    }

    pub fn display_area(&self) -> DisplayArea {
        let w = self.display_hres.max(1);
        let h = self.display_vres.min(240).max(1);
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                pixels.push(self.read_half(self.display_x + x, self.display_y + y));
            }
        }
        DisplayArea {
            width: w,
            height: h,
            pixels,
        }
    }

    pub fn dma_write(&mut self, word: u32) {
        self.gp0(word);
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
    ((r >> 3) as u16) | (((g >> 3) as u16) << 5) | (((b >> 3) as u16) << 10)
}

fn sign11(v: u32) -> i32 {
    let v = v as i32;
    (v << 21) >> 21
}
