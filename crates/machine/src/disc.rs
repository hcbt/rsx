use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SECTOR_LEN: usize = 2352;

#[derive(Debug)]
pub enum DiscError {
    Io(io::Error),
    Cue(String),
}

impl std::fmt::Display for DiscError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscError::Io(e) => write!(f, "cannot read Disc: {e}"),
            DiscError::Cue(e) => write!(f, "cue: {e}"),
        }
    }
}

impl std::error::Error for DiscError {}

pub struct Disc {
    data: Vec<u8>,
    pub region: [u8; 4],
}

impl Disc {
    pub fn sector_count(&self) -> u32 {
        (self.data.len() / SECTOR_LEN) as u32
    }

    pub fn sector(&self, lba: u32) -> Option<&[u8]> {
        let off = lba as usize * SECTOR_LEN;
        self.data.get(off..off + SECTOR_LEN)
    }
}

pub fn load_disc(path: &Path) -> Result<Disc, DiscError> {
    let bin_path = if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cue"))
    {
        bin_path_from_cue(path)?
    } else {
        path.to_path_buf()
    };
    let data = fs::read(&bin_path).map_err(DiscError::Io)?;
    if data.len() < SECTOR_LEN {
        return Err(DiscError::Cue("image is shorter than one sector".into()));
    }
    let region = region_from_license(&data);
    Ok(Disc { data, region })
}

fn bin_path_from_cue(cue_path: &Path) -> Result<PathBuf, DiscError> {
    let text = fs::read_to_string(cue_path).map_err(DiscError::Io)?;
    let mut file: Option<String> = None;
    let mut mode_ok = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("FILE ") {
            file = Some(parse_cue_file(rest)?);
        } else if line.contains("MODE2/2352") || line.contains("MODE1/2352") {
            mode_ok = true;
        }
    }
    if !mode_ok {
        return Err(DiscError::Cue(
            "need a MODE2/2352 or MODE1/2352 track".into(),
        ));
    }
    let name = file.ok_or_else(|| DiscError::Cue("no FILE".into()))?;
    let dir = cue_path.parent().unwrap_or_else(|| Path::new("."));
    Ok(dir.join(name))
}

fn parse_cue_file(rest: &str) -> Result<String, DiscError> {
    let rest = rest.trim();
    if let Some(s) = rest.strip_prefix('"') {
        let end = s
            .find('"')
            .ok_or_else(|| DiscError::Cue("unterminated FILE name".into()))?;
        return Ok(s[..end].to_string());
    }
    let name = rest
        .split_whitespace()
        .next()
        .ok_or_else(|| DiscError::Cue("empty FILE".into()))?;
    Ok(name.to_string())
}

fn region_from_license(data: &[u8]) -> [u8; 4] {
    let probe = if data.len() >= SECTOR_LEN * 5 {
        &data[SECTOR_LEN * 4..SECTOR_LEN * 5]
    } else {
        data
    };
    let s = String::from_utf8_lossy(probe);
    if s.contains("Amer") {
        *b"SCEA"
    } else if s.contains("Euro") {
        *b"SCEE"
    } else {
        *b"SCEI"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn mode2_sector(mm: u8, ss: u8, ff: u8, user: &[u8]) -> [u8; SECTOR_LEN] {
        let mut s = [0u8; SECTOR_LEN];
        s[0] = 0;
        s[1..11].fill(0xFF);
        s[11] = 0;
        s[12] = mm;
        s[13] = ss;
        s[14] = ff;
        s[15] = 2;
        let n = user.len().min(2328);
        s[24..24 + n].copy_from_slice(&user[..n]);
        s
    }

    fn write_cue_bin(dir: &Path, license: &[u8]) -> PathBuf {
        let mut bin = Vec::new();
        for i in 0..24u8 {
            let user = if i == 4 { license } else { b"" };
            bin.extend_from_slice(&mode2_sector(0, 2, i, user));
        }
        fs::write(dir.join("game.bin"), &bin).unwrap();
        let cue = dir.join("game.cue");
        let mut f = fs::File::create(&cue).unwrap();
        writeln!(f, "FILE \"game.bin\" BINARY").unwrap();
        writeln!(f, "  TRACK 01 MODE2/2352").unwrap();
        writeln!(f, "    INDEX 01 00:00:00").unwrap();
        cue
    }

    #[test]
    fn cue_mode2_loads_sectors_and_scea() {
        let dir = tempfile::tempdir().unwrap();
        let cue = write_cue_bin(
            dir.path(),
            b"          Licensed  by          Sony Computer Entertainment Amer  ica",
        );
        let disc = load_disc(&cue).unwrap();
        assert_eq!(disc.sector_count(), 24);
        assert_eq!(&disc.region, b"SCEA");
        assert!(disc.sector(4).is_some());
    }

    #[test]
    fn missing_disc_is_an_error() {
        let err = match load_disc(Path::new("/no/such/game.cue")) {
            Err(e) => e,
            Ok(_) => panic!("expected missing Disc to fail"),
        };
        assert!(matches!(err, DiscError::Io(_)));
    }
}
