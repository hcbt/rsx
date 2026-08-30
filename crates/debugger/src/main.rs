mod config;

use std::path::PathBuf;

use eframe::egui;
use rsx_machine::{DisplayArea, Machine};

struct Debugger {
    machine: Option<Machine>,
    error: Option<String>,
    running: bool,
    log: Vec<String>,
    texture: Option<egui::TextureHandle>,
}

impl Debugger {
    fn new(bios: Result<PathBuf, String>) -> Self {
        let mut d = Self {
            machine: None,
            error: None,
            running: false,
            log: Vec::new(),
            texture: None,
        };
        match bios {
            Ok(p) => d.load(p),
            Err(e) => d.error = Some(e),
        }
        d
    }

    fn load(&mut self, path: PathBuf) {
        match Machine::from_bios_path(&path) {
            Ok(m) => {
                self.log
                    .push(format!("loaded BIOS {}", path.display()));
                self.machine = Some(m);
                self.error = None;
                self.running = true;
            }
            Err(e) => {
                self.machine = None;
                self.error = Some(e.to_string());
            }
        }
    }
}

impl eframe::App for Debugger {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.running {
            if let Some(m) = self.machine.as_mut() {
                let target = m.vblank_count() + 1;
                m.run_until_vblank_count(target);
            }
            ctx.request_repaint();
        }

        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            if ui.button("Run").clicked() {
                self.running = true;
            }
            if ui.button("Pause").clicked() {
                self.running = false;
            }
            if ui.button("Step instruction").clicked() {
                self.running = false;
                if let Some(m) = self.machine.as_mut() {
                    m.step();
                }
            }
            if ui.button("Step frame").clicked() {
                self.running = false;
                if let Some(m) = self.machine.as_mut() {
                    let n = m.vblank_count() + 1;
                    m.run_until_vblank_count(n);
                }
            }
        });

        egui::SidePanel::left("regs").resizable(true).show(ctx, |ui| {
            ui.heading("CPU");
            if let Some(m) = self.machine.as_ref() {
                ui.monospace(format!("PC {:08X}", m.pc()));
                ui.monospace(format!("GPUSTAT {:08X}", m.gpustat()));
                ui.monospace(format!("vblank {}", m.vblank_count()));
                for i in 0..32u8 {
                    ui.monospace(format!("r{i:02} {:08X}", m.gpr(i)));
                }
            } else if let Some(e) = &self.error {
                ui.colored_label(egui::Color32::RED, e);
            }
        });

        egui::TopBottomPanel::bottom("log").resizable(true).show(ctx, |ui| {
            ui.heading("I/O + IRQ log");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for line in &self.log {
                    ui.monospace(line);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Display area");
            if let Some(m) = self.machine.as_ref() {
                let area = m.display_area();
                let image = area_to_color_image(&area);
                let tex = self.texture.get_or_insert_with(|| {
                    ctx.load_texture("display", image.clone(), Default::default())
                });
                tex.set(image, Default::default());
                let avail = ui.available_size();
                let aspect = area.width as f32 / area.height.max(1) as f32;
                let mut w = avail.x;
                let mut h = w / aspect;
                if h > avail.y {
                    h = avail.y;
                    w = h * aspect;
                }
                ui.image((tex.id(), egui::vec2(w.max(1.0), h.max(1.0))));
            }
        });
    }
}

fn area_to_color_image(area: &DisplayArea) -> egui::ColorImage {
    let mut rgb = Vec::with_capacity((area.width * area.height * 3) as usize);
    for p in &area.pixels {
        let r = ((p & 0x1F) << 3) as u8;
        let g = (((p >> 5) & 0x1F) << 3) as u8;
        let b = (((p >> 10) & 0x1F) << 3) as u8;
        rgb.push(r);
        rgb.push(g);
        rgb.push(b);
    }
    egui::ColorImage::from_rgb([area.width as usize, area.height as usize], &rgb)
}

fn main() -> eframe::Result<()> {
    let cli = config::parse_cli(std::env::args());
    let bios = config::resolve_bios(cli, std::path::Path::new("."));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([960.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rsx",
        options,
        Box::new(move |_cc| Ok(Box::new(Debugger::new(bios)))),
    )
}
