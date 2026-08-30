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
    let mut writer = enc
        .write_header()
        .map_err(|e| format!("png header: {e}"))?;
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
        let path = dir.join(format!("v{n:04}.png"));
        write_png(&machine.display_area(), &path)?;
        let latest = dir.join("latest.png");
        write_png(&machine.display_area(), &latest)?;
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
