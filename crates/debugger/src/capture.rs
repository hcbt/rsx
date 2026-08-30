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
        let (frtps, on, osy0, osy1, ovy0, ovy1, otry, otrz, ovx0, ovx1) = machine.gte_frame_obj();
        eprintln!(
            "  frame-rtps={frtps} obj n={on} SY={osy0}..{osy1} VY={ovy0}..{ovy1} VX={ovx0}..{ovx1} TRY={otry} TRZ={otrz}"
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
