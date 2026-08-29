use std::fs;
use std::io;
use std::path::Path;

pub const BIOS_LEN: usize = 512 * 1024;

#[derive(Debug)]
pub enum BiosError {
    Io(io::Error),
    WrongSize { got: usize },
}

impl std::fmt::Display for BiosError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BiosError::Io(e) => write!(f, "cannot read BIOS: {e}"),
            BiosError::WrongSize { got } => {
                write!(f, "BIOS must be {BIOS_LEN} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for BiosError {}

pub fn load_bios(path: &Path) -> Result<Vec<u8>, BiosError> {
    let data = fs::read(path).map_err(BiosError::Io)?;
    if data.len() != BIOS_LEN {
        return Err(BiosError::WrongSize { got: data.len() });
    }
    Ok(data)
}
