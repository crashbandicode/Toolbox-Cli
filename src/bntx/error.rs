use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    BadMagic([u8; 4]),
    UnsupportedVersion(u32),
    UnsupportedFormat(u32),
    Truncated(String),
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::BadMagic(m) => write!(
                f,
                "not a BNTX (magic = {:?})",
                std::str::from_utf8(m).unwrap_or("?")
            ),
            Error::UnsupportedVersion(v) => write!(f, "unsupported BNTX version 0x{v:08x}"),
            Error::UnsupportedFormat(f_) => write!(f, "unknown BNTX surface format 0x{f_:08x}"),
            Error::Truncated(s) => write!(f, "truncated: {s}"),
            Error::Format(s) => write!(f, "format error: {s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
