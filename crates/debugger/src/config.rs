use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, PartialEq, Eq)]
pub struct Cli {
    pub bios: Option<PathBuf>,
    pub disc: Option<PathBuf>,
    /// Headless: run to each vblank and capture the Display area.
    pub capture_at: Vec<u64>,
    pub capture_dir: Option<PathBuf>,
    /// No window: run the Machine and print measurements on stdout.
    pub headless: bool,
    /// Stop after this many vblanks. None means until SIGINT.
    pub until_vblank: Option<u64>,
    /// Print one sample every this many vblanks (default 60 ≈ one NTSC second).
    pub period: u64,
}

impl Default for Cli {
    fn default() -> Self {
        Self {
            bios: None,
            disc: None,
            capture_at: Vec::new(),
            capture_dir: None,
            headless: false,
            until_vblank: None,
            period: 60,
        }
    }
}

#[derive(Default, Deserialize)]
struct FileConfig {
    bios: Option<String>,
    disc: Option<String>,
}

fn load_toml(cwd: &Path) -> Result<FileConfig, String> {
    let text = fs::read_to_string(cwd.join("rsx.toml")).map_err(|_| {
        "BIOS path required: pass --bios <file> or set bios in rsx.toml".to_string()
    })?;
    toml::from_str(&text).map_err(|e| format!("rsx.toml: {e}"))
}

/// CLI `--bios` wins over `rsx.toml`. Either is enough. Missing both is an error.
pub fn resolve_bios(cli: Option<PathBuf>, cwd: &Path) -> Result<PathBuf, String> {
    if let Some(p) = cli {
        return Ok(p);
    }
    load_toml(cwd)?
        .bios
        .map(PathBuf::from)
        .ok_or_else(|| "rsx.toml has no bios = \"...\"".to_string())
}

/// CLI `--disc` wins over `rsx.toml`. Missing both means the drive is empty.
pub fn resolve_disc(cli: Option<PathBuf>, cwd: &Path) -> Result<Option<PathBuf>, String> {
    if let Some(p) = cli {
        return Ok(Some(p));
    }
    match fs::read_to_string(cwd.join("rsx.toml")) {
        Ok(text) => {
            let cfg: FileConfig = toml::from_str(&text).map_err(|e| format!("rsx.toml: {e}"))?;
            Ok(cfg.disc.map(PathBuf::from))
        }
        Err(_) => Ok(None),
    }
}

pub fn parse_cli(args: impl IntoIterator<Item = String>) -> Cli {
    let mut cli = Cli::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--bios" {
            cli.bios = it.next().map(PathBuf::from);
        } else if a == "--disc" {
            cli.disc = it.next().map(PathBuf::from);
        } else if a == "--capture-at" {
            if let Some(v) = it.next() {
                cli.capture_at = parse_vblank_list(&v);
            }
        } else if a == "--capture-dir" {
            cli.capture_dir = it.next().map(PathBuf::from);
        } else if a == "--headless" {
            cli.headless = true;
        } else if a == "--until-vblank" {
            if let Some(v) = it.next() {
                cli.until_vblank = v.parse().ok();
            }
        } else if a == "--period" {
            if let Some(v) = it.next() {
                if let Ok(n) = v.parse::<u64>() {
                    if n > 0 {
                        cli.period = n;
                    }
                }
            }
        }
    }
    cli
}

fn parse_vblank_list(s: &str) -> Vec<u64> {
    let mut v: Vec<u64> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub fn capture_dir(cli: Option<PathBuf>) -> PathBuf {
    cli.unwrap_or_else(|| PathBuf::from("target/rsx-display"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_wins_over_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("rsx.toml"), "bios = \"from-toml.bin\"\n").unwrap();
        let got = resolve_bios(Some(PathBuf::from("from-cli.bin")), dir.path()).unwrap();
        assert_eq!(got, PathBuf::from("from-cli.bin"));
    }

    #[test]
    fn toml_used_when_no_cli() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("rsx.toml"), "bios = \"from-toml.bin\"\n").unwrap();
        let got = resolve_bios(None, dir.path()).unwrap();
        assert_eq!(got, PathBuf::from("from-toml.bin"));
    }

    #[test]
    fn missing_both_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(resolve_bios(None, dir.path()).is_err());
    }

    #[test]
    fn parse_cli_bios_flag() {
        let c = parse_cli(["rsx".into(), "--bios".into(), "SCPH1001.BIN".into()]);
        assert_eq!(c.bios.unwrap(), PathBuf::from("SCPH1001.BIN"));
        assert!(c.disc.is_none());
    }

    #[test]
    fn parse_cli_disc_flag() {
        let c = parse_cli([
            "rsx".into(),
            "--bios".into(),
            "SCPH1001.BIN".into(),
            "--disc".into(),
            "game.cue".into(),
        ]);
        assert_eq!(c.bios.unwrap(), PathBuf::from("SCPH1001.BIN"));
        assert_eq!(c.disc.unwrap(), PathBuf::from("game.cue"));
    }

    #[test]
    fn disc_cli_wins_over_toml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("rsx.toml"),
            "bios = \"b.bin\"\ndisc = \"from-toml.cue\"\n",
        )
        .unwrap();
        let got = resolve_disc(Some(PathBuf::from("from-cli.cue")), dir.path()).unwrap();
        assert_eq!(got, Some(PathBuf::from("from-cli.cue")));
    }

    #[test]
    fn disc_toml_used_when_no_cli() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("rsx.toml"),
            "bios = \"b.bin\"\ndisc = \"from-toml.cue\"\n",
        )
        .unwrap();
        let got = resolve_disc(None, dir.path()).unwrap();
        assert_eq!(got, Some(PathBuf::from("from-toml.cue")));
    }

    #[test]
    fn parse_cli_capture_at() {
        let c = parse_cli([
            "rsx".into(),
            "--capture-at".into(),
            "500,150,150,450".into(),
            "--capture-dir".into(),
            "shots".into(),
        ]);
        assert_eq!(c.capture_at, vec![150, 450, 500]);
        assert_eq!(c.capture_dir.unwrap(), PathBuf::from("shots"));
        assert!(!c.headless);
    }

    #[test]
    fn parse_cli_headless_defaults_period() {
        let c = parse_cli(["rsx".into(), "--headless".into()]);
        assert!(c.headless);
        assert_eq!(c.until_vblank, None);
        assert_eq!(c.period, 60);
    }

    #[test]
    fn parse_cli_headless_until_and_period() {
        let c = parse_cli([
            "rsx".into(),
            "--headless".into(),
            "--until-vblank".into(),
            "1200".into(),
            "--period".into(),
            "30".into(),
        ]);
        assert!(c.headless);
        assert_eq!(c.until_vblank, Some(1200));
        assert_eq!(c.period, 30);
    }

    #[test]
    fn missing_disc_is_an_empty_drive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("rsx.toml"), "bios = \"b.bin\"\n").unwrap();
        let got = resolve_disc(None, dir.path()).unwrap();
        assert_eq!(got, None);
    }
}
