use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Default, Deserialize)]
struct FileConfig {
    bios: Option<String>,
}

/// CLI `--bios` wins over `rsx.toml`. Either is enough. Missing both is an error.
pub fn resolve_bios(cli: Option<PathBuf>, cwd: &Path) -> Result<PathBuf, String> {
    if let Some(p) = cli {
        return Ok(p);
    }
    let text = fs::read_to_string(cwd.join("rsx.toml"))
        .map_err(|_| "BIOS path required: pass --bios <file> or set bios in rsx.toml".to_string())?;
    let cfg: FileConfig = toml::from_str(&text)
        .map_err(|e| format!("rsx.toml: {e}"))?;
    cfg.bios
        .map(PathBuf::from)
        .ok_or_else(|| "rsx.toml has no bios = \"...\"".to_string())
}

pub fn parse_cli(args: impl IntoIterator<Item = String>) -> Option<PathBuf> {
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--bios" {
            return it.next().map(PathBuf::from);
        }
    }
    None
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
        let p = parse_cli(["rsx".into(), "--bios".into(), "SCPH1001.BIN".into()]);
        assert_eq!(p.unwrap(), PathBuf::from("SCPH1001.BIN"));
    }
}
