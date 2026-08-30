use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

use rsx_machine::{DisplayArea, Machine};

pub fn write_png(area: &DisplayArea, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("capture dir: {e}"))?;
        }
    }
    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(BufWriter::new(file), area.width, area.height);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| format!("png header: {e}"))?;
    writer
        .write_image_data(&area.to_rgb888())
        .map_err(|e| format!("png data: {e}"))?;
    Ok(())
}

pub fn capture_at_vblanks(
    machine: &mut Machine,
    dir: &Path,
    vblanks: &[u64],
) -> Result<Vec<std::path::PathBuf>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("capture dir: {e}"))?;
    let mut written = Vec::new();
    for &n in vblanks {
        machine.run_until_vblank_count(n);
        let area = machine.display_area();
        let path = dir.join(format!("v{n:04}.png"));
        write_png(&area, &path)?;
        let latest = dir.join("latest.png");
        write_png(&area, &latest)?;
        let (dx, dy, dw, dh, on) = machine.display_origin();
        let (ox, oy, x1, y1, x2, y2) = machine.draw_env();
        let (n30, px0, px1, py0, py1, nout) = machine.last_gouraud_tri_stats();
        let (cn, cx0, cx1, cy0, cy1) = machine.gouraud_tri_stats();
        eprintln!(
            "captured {} {}x{} display=({dx},{dy}) {dw}x{dh} on={on} ofs=({ox},{oy}) clip=({x1},{y1})-({x2},{y2}) hash={:016x} last30 n={n30} out={nout} xy=({px0},{py0})-({px1},{py1}) now30 n={cn} xy=({cx0},{cy0})-({cx1},{cy1}) GTE H={:#x} OFX={:#x} OFY={:#x} ZSF3={:#x}",
            path.display(),
            area.width,
            area.height,
            machine.display_area_hash(),
            machine.gte_control(26) & 0xFFFF,
            machine.gte_control(24),
            machine.gte_control(25),
            machine.gte_control(29) & 0xFFFF,
        );
        let (empty, pkts, amin, amax, astart, sn, ebefore) = machine.dma_list_stats();
        eprintln!(
            "  dma2-list empty={empty} pkts={pkts} start={astart:#X} n={sn} empty_before_pkt={ebefore} addr={amin:#X}..{amax:#X}"
        );
        let (hsy, hir2, hn, hsz, hvy, htry, nexp) = machine.gte_hi_sy_trace();
        eprintln!(
            "  gte-hi-sy SY={hsy} IR2={hir2} n={hn:#X} SZ={hsz} VY={hvy} TRY={htry} explode={nexp} TRX={:#X} TRY_reg={:#X} TRZ={:#X} RT22={:#X}",
            machine.gte_control(5) as i32,
            machine.gte_control(6) as i32,
            machine.gte_control(7) as i32,
            machine.gte_control(2) as u16,
        );
        let (ir2lo, ir2hi, vylo, vyhi, htrz, r21, r22, r23) = machine.gte_title_ir2();
        let rt = machine.gte_title_rt();
        eprintln!(
            "  title-rtps IR2={ir2lo}..{ir2hi} VY={vylo}..{vyhi} hi-TRZ={htrz} R2=[{r21},{r22},{r23}]"
        );
        eprintln!(
            "  title-rt [[{},{},{}],[{},{},{}],[{},{},{}]]",
            rt[0], rt[1], rt[2], rt[3], rt[4], rt[5], rt[6], rt[7], rt[8]
        );
        let (frtps, on, osy0, osy1, ovy0, ovy1, otry, otrz, ovx0, ovx1, ovz0, ovz1, fexp) =
            machine.gte_frame_obj();
        eprintln!(
            "  frame-rtps={frtps} obj n={on} SY={osy0}..{osy1} VY={ovy0}..{ovy1} VX={ovx0}..{ovx1} VZ={ovz0}..{ovz1} TRY={otry} TRZ={otrz} explode={fexp}"
        );
        dump_title_ram(machine);
        let scatter = machine.last_gouraud_scatter();
        let scatter_path = dir.join(format!("scatter-v{n:04}.png"));
        write_png(&scatter, &scatter_path)?;
        eprintln!(
            "  scatter {} long30(dy>80)={} max_dy={}",
            scatter_path.display(),
            machine.last_long30(),
            machine.last_max_dy()
        );
        let ops = machine.gte_op_counts();
        let names = [
            (0x01u8, "RTPS"),
            (0x06, "NCLIP"),
            (0x0C, "OP"),
            (0x10, "DPCS"),
            (0x11, "INTPL"),
            (0x12, "MVMVA"),
            (0x13, "NCDS"),
            (0x14, "CDP"),
            (0x16, "NCDT"),
            (0x1B, "NCCS"),
            (0x1C, "CC"),
            (0x1E, "NCS"),
            (0x20, "NCT"),
            (0x28, "SQR"),
            (0x29, "DCPL"),
            (0x2A, "DPCT"),
            (0x2D, "AVSZ3"),
            (0x2E, "AVSZ4"),
            (0x30, "RTPT"),
            (0x3D, "GPF"),
            (0x3E, "GPL"),
            (0x3F, "NCCT"),
        ];
        let hist: Vec<String> = names
            .iter()
            .filter(|(op, _)| ops[*op as usize] > 0)
            .map(|(op, n)| format!("{n}={}", ops[*op as usize]))
            .collect();
        eprintln!("  gte-ops {}", hist.join(" "));
        eprintln!(
            "  ot[0..3] {:08X} {:08X} {:08X} {:08X}",
            machine.ram_word(astart),
            machine.ram_word(astart.wrapping_add(4)),
            machine.ram_word(astart.wrapping_add(8)),
            machine.ram_word(astart.wrapping_add(12)),
        );
        eprintln!(
            "  y-bins/32 {} hi_y_word={:#010X}",
            machine
                .last_y_bins()
                .iter()
                .enumerate()
                .map(|(i, n)| format!("{i}:{n}"))
                .collect::<Vec<_>>()
                .join(" "),
            machine.last_hi_y_word()
        );
        written.push(path);
    }
    Ok(written)
}

fn i16s(w: u32) -> (i32, i32) {
    (w as i16 as i32, (w >> 16) as i16 as i32)
}

fn mat9(machine: &Machine, addr: u32) -> [i32; 9] {
    // PSY-Q MATRIX: packed i16 pairs, RT33 in the 5th halfword.
    let w0 = machine.ram_word(addr);
    let w1 = machine.ram_word(addr.wrapping_add(4));
    let w2 = machine.ram_word(addr.wrapping_add(8));
    let w3 = machine.ram_word(addr.wrapping_add(12));
    let w4 = machine.ram_word(addr.wrapping_add(16));
    let (a, b) = i16s(w0);
    let (c, d) = i16s(w1);
    let (e, f) = i16s(w2);
    let (g, h) = i16s(w3);
    let (i, _) = i16s(w4);
    [a, b, c, d, e, f, g, h, i]
}

fn vec3(machine: &Machine, addr: u32) -> (i32, i32, i32) {
    (
        machine.ram_word(addr) as i32,
        machine.ram_word(addr.wrapping_add(4)) as i32,
        machine.ram_word(addr.wrapping_add(8)) as i32,
    )
}

/// NTSC-U (SCUS-94900) BSS from the reconstructed c1 port. Diagnostic only.
fn dump_title_ram(machine: &Machine) {
    let crash = machine.ram_word(0x8005_66B4);
    let (ctx, cty, ctz) = vec3(machine, 0x8005_7864);
    let (crx, cry, crz) = vec3(machine, 0x8005_7870);
    let (cpx, cpy, cpz) = vec3(machine, 0x8005_7888);
    let screen_proj = machine.ram_word(0x8005_78D0);
    let frames = machine.ram_word(0x8006_0E04);
    let ms = mat9(machine, 0x8005_7844);
    let mn = mat9(machine, 0x8005_7824);
    let msc = mat9(machine, 0x8005_77E4);
    eprintln!(
        "  ram crash={crash:#X} cam_trans=({ctx},{cty},{ctz}) cam_rot=({crx},{cry},{crz}) cam_prev=({cpx},{cpy},{cpz}) proj={screen_proj} fe={frames}"
    );
    eprintln!(
        "  ram ms_rot [[{},{},{}],[{},{},{}],[{},{},{}]]",
        ms[0], ms[1], ms[2], ms[3], ms[4], ms[5], ms[6], ms[7], ms[8]
    );
    eprintln!(
        "  ram mn_rot [[{},{},{}],[{},{},{}],[{},{},{}]]",
        mn[0], mn[1], mn[2], mn[3], mn[4], mn[5], mn[6], mn[7], mn[8]
    );
    eprintln!(
        "  ram ms_cam [[{},{},{}],[{},{},{}],[{},{},{}]]",
        msc[0], msc[1], msc[2], msc[3], msc[4], msc[5], msc[6], msc[7], msc[8]
    );
    if crash & 0xFF00_0000 == 0x8000_0000 {
        let (tx, ty, tz) = vec3(machine, crash.wrapping_add(0x80));
        let (rx, ry, rz) = vec3(machine, crash.wrapping_add(0x8C));
        let (sx, sy, sz) = vec3(machine, crash.wrapping_add(0x98));
        let state = machine.ram_word(crash.wrapping_add(0x2C));
        let entity = machine.ram_word(crash.wrapping_add(0x110));
        let anim_frame = machine.ram_word(crash.wrapping_add(0x10C));
        let (vx, vy, vz) = vec3(machine, crash.wrapping_add(0xA4));
        let status_a = machine.ram_word(crash.wrapping_add(0xC8));
        let c1p = machine.ram_word(0x8005_8404);
        let tpf = if c1p & 0xFF00_0000 == 0x8000_0000 {
            machine.ram_word(c1p.wrapping_add(0x84)) as i32
        } else {
            0
        };
        eprintln!(
            "  ram crash.trans=({tx},{ty},{tz}) rot=({rx},{ry},{rz}) scale=({sx},{sy},{sz}) state={state} entity={entity:#X} anim_frame={anim_frame} vel=({vx},{vy},{vz}) status_a={status_a:#X} tpf={tpf}"
        );
        dump_entity(machine, entity);
        let (bx1, by1, bz1) = vec3(machine, crash.wrapping_add(8));
        let (bx2, by2, bz2) = vec3(machine, crash.wrapping_add(0x14));
        let anim_seq = machine.ram_word(crash.wrapping_add(0x108));
        let pc = machine.ram_word(crash.wrapping_add(0xE0));
        let tp = machine.ram_word(crash.wrapping_add(0xE8));
        eprintln!(
            "  ram crash.bound=({bx1},{by1},{bz1})-({bx2},{by2},{bz2}) anim_seq={anim_seq:#X} pc={pc:#X} tp={tp:#X}"
        );
        dump_anim(machine, anim_seq, anim_frame);
        // USA EXE TransformSvtx @ 0x80034684
        eprint!("  ram TransformSvtx");
        for i in 0..12u32 {
            eprint!(" {:08X}", machine.ram_word(0x8003_4684 + i * 4));
        }
        eprintln!();
    }
    let zone = machine.ram_word(0x8005_7914);
    let path = machine.ram_word(0x8005_791C);
    let progress = machine.ram_word(0x8005_7920) as i32;
    let disp = machine.ram_word(0x8006_18B0);
    eprintln!("  ram cur_zone={zone:#X} cur_path={path:#X} progress={progress} disp={disp:#X}");
    dump_entry(machine, zone, "zone");
    dump_zone_rect(machine, zone);
    dump_path(machine, path);
    dump_objects(machine);
}

fn dump_entry(machine: &Machine, addr: u32, name: &str) {
    if addr & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    let magic = machine.ram_word(addr);
    let eid = machine.ram_word(addr.wrapping_add(4));
    let typ = machine.ram_word(addr.wrapping_add(8));
    let nitem = machine.ram_word(addr.wrapping_add(12));
    eprintln!("  ram {name} magic={magic:#X} eid={eid:#X} type={typ} items={nitem}");
}

fn dump_zone_rect(machine: &Machine, zone: u32) {
    if zone & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    let nitem = machine.ram_word(zone.wrapping_add(12));
    if nitem < 2 {
        return;
    }
    let item1 = machine.ram_word(zone.wrapping_add(16 + 4));
    // After NS relocate, items[] are pointers. Fall back to offset-from-entry.
    let rect = if item1 & 0xFF00_0000 == 0x8000_0000 {
        item1
    } else {
        zone.wrapping_add(item1)
    };
    let x = machine.ram_word(rect) as i32;
    let y = machine.ram_word(rect.wrapping_add(4)) as i32;
    let z = machine.ram_word(rect.wrapping_add(8)) as i32;
    let w = machine.ram_word(rect.wrapping_add(12));
    let h = machine.ram_word(rect.wrapping_add(16));
    let d = machine.ram_word(rect.wrapping_add(20));
    eprintln!("  ram zrect@{rect:#X} xyz=({x},{y},{z}) whd=({w},{h},{d})");
}

fn dump_path(machine: &Machine, path: u32) {
    if path & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    let parent = machine.ram_word(path.wrapping_add(4));
    let packed = machine.ram_word(path.wrapping_add(0x18));
    let length = (packed & 0xFFFF) as u16;
    let cam_mode = ((packed >> 16) & 0xFFFF) as u16;
    // points[] start after the fixed header: slst+parent+count+4 neighbors (16)
    // + entrance/exit (2) + length (2) + cam_mode (2) + avg (2) + zoom (2)
    // + unk a,b,c (6) + dir xyz (6) = 4+4+4+16+2+2+2+2+2+6+6 = 50, aligned?
    // C: eid 4, parent* 4, count 4, neighbors[4] 16, entrance 1, exit 1,
    // length 2, cam_mode 2, avg 2, zoom 2, unk a,b,c 6, dir 6 → 50, pad to 52?
    // Dump first point as i16 pairs from a few candidate offsets.
    let w0 = machine.ram_word(path);
    let dir = machine.ram_word(path.wrapping_add(0x2C));
    eprintln!(
        "  ram path parent={parent:#X} slst={w0:#X} length={length} cam_mode={cam_mode} dirw={dir:#X}"
    );
    for off in [0x30u32, 0x32, 0x34, 0x38, 0x3C, 0x40] {
        let w = machine.ram_word(path.wrapping_add(off & !3));
        let (lo, hi) = i16s(w);
        eprintln!("    path+{off:#X} {lo},{hi} raw={w:#010X}");
    }
}

fn dump_anim(machine: &Machine, anim: u32, anim_frame: u32) {
    if anim & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    let typ = machine.ram_word(anim);
    let eid = machine.ram_word(anim.wrapping_add(4));
    eprintln!("  ram anim packed={typ:#010X} eid={eid:#X}");
    if eid & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    eprint!("  ram svtx_entry");
    for i in 0..8u32 {
        eprint!(" {:08X}", machine.ram_word(eid.wrapping_add(i * 4)));
    }
    eprintln!();
    let mut entry = eid;
    let mut magic = machine.ram_word(entry);
    // Title stores a pointer to the NSF entry, not the entry itself.
    if magic != 0x100FFFF && magic & 0xFF00_0000 == 0x8000_0000 {
        let inner = magic;
        if machine.ram_word(inner) == 0x100FFFF {
            entry = inner;
            magic = 0x100FFFF;
        }
    }
    let frame_idx = anim_frame >> 8;
    if magic != 0x100FFFF {
        return;
    }
    let nitem = machine.ram_word(entry.wrapping_add(12));
    let etype = machine.ram_word(entry.wrapping_add(8));
    eprintln!("  ram svtx_nsf@{entry:#X} type={etype} items={nitem} frame_idx={frame_idx}");
    if frame_idx >= nitem {
        return;
    }
    let item = machine.ram_word(entry.wrapping_add(16 + frame_idx * 4));
    let frame = if item & 0xFF00_0000 == 0x8000_0000 {
        item
    } else {
        entry.wrapping_add(item)
    };
    eprintln!("  ram svtx frame={frame:#X}");
    if frame & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    // svtx_frame: length, tgeo, x, y, z, bound, col
    let length = machine.ram_word(frame);
    let tgeo = machine.ram_word(frame.wrapping_add(4));
    let fx = machine.ram_word(frame.wrapping_add(8)) as i32;
    let fy = machine.ram_word(frame.wrapping_add(12)) as i32;
    let fz = machine.ram_word(frame.wrapping_add(16)) as i32;
    let colx = machine.ram_word(frame.wrapping_add(0x2C)) as i32;
    let coly = machine.ram_word(frame.wrapping_add(0x30)) as i32;
    let colz = machine.ram_word(frame.wrapping_add(0x34)) as i32;
    eprintln!(
        "  ram svtx_frame len={length} tgeo={tgeo:#X} origin=({fx},{fy},{fz}) col=({colx},{coly},{colz})"
    );
    eprint!("  ram TransformSvtx+0x30");
    for i in 12..24u32 {
        eprint!(" {:08X}", machine.ram_word(0x8003_4684 + i * 4));
    }
    eprintln!();
}

fn dump_entity(machine: &Machine, entity: u32) {
    if entity & 0xFF00_0000 != 0x8000_0000 {
        return;
    }
    let parent = machine.ram_word(entity);
    let w1 = machine.ram_word(entity.wrapping_add(4));
    let spawn_flags = w1 as u16;
    let group = (w1 >> 16) as u16;
    let w2 = machine.ram_word(entity.wrapping_add(8));
    let id = w2 as u16;
    let path_len = (w2 >> 16) as u16;
    let w4 = machine.ram_word(entity.wrapping_add(0x10));
    let typ = (w4 >> 16) as u8;
    let subtype = (w4 >> 24) as u8;
    let (px, py) = i16s(machine.ram_word(entity.wrapping_add(0x14)));
    let (pz, _) = i16s(machine.ram_word(entity.wrapping_add(0x18)));
    eprintln!(
        "  ram entity parent={parent:#X} flags={spawn_flags:#X} group={group} id={id} plen={path_len} type={typ} subtype={subtype} pt0=({px},{py},{pz})"
    );
    if parent & 0xFF00_0000 == 0x8000_0000 {
        let ptype = machine.ram_word(parent.wrapping_add(8));
        eprintln!("  ram entity.parent_zone type={ptype}");
    }
}

fn dump_objects(machine: &Machine) {
    // NTSC-U handles[8] at 0x80060DB8; children at +4 for a handle header.
    let mut left = 24u32;
    for h in 0..8u32 {
        let handle = 0x8006_0DB8u32.wrapping_add(h * 8);
        let child = machine.ram_word(handle.wrapping_add(4));
        walk_obj(machine, child, &mut left);
        if left == 0 {
            break;
        }
    }
}

fn walk_obj(machine: &Machine, mut obj: u32, left: &mut u32) {
    let mut n = 0u32;
    while obj & 0xFF00_0000 == 0x8000_0000 && n < 96 && *left > 0 {
        n += 1;
        *left -= 1;
        let kind = machine.ram_word(obj);
        if kind == 2 {
            walk_obj(machine, machine.ram_word(obj.wrapping_add(4)), left);
        } else {
            let (tx, ty, tz) = vec3(machine, obj.wrapping_add(0x80));
            let (sx, sy, sz) = vec3(machine, obj.wrapping_add(0x98));
            let state = machine.ram_word(obj.wrapping_add(0x2C));
            let status_b = machine.ram_word(obj.wrapping_add(0xCC));
            eprintln!(
                "  ram obj={obj:#X} trans=({tx},{ty},{tz}) scale=({sx},{sy},{sz}) state={state} status_b={status_b:#X}"
            );
            walk_obj(machine, machine.ram_word(obj.wrapping_add(0x6C)), left);
        }
        obj = machine.ram_word(obj.wrapping_add(0x68));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsx_machine::DisplayArea;

    #[test]
    fn write_png_roundtrip_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("one.png");
        let area = DisplayArea {
            width: 2,
            height: 1,
            pixels: vec![0x001F, 0x03E0],
        };
        write_png(&area, &path).unwrap();
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(path.metadata().unwrap().len() > 32);
    }
}
