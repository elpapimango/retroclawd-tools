//! `rcchd`: make, unpack, inspect, check and edit CHD files (compressed hunks of data), the work of MAME's `chdman`.
//!
//! The CHD format, its codecs and the writer are the `chd` crate of the Sega family; this crate holds what is specific to the kinds of
//! media a CHD carries (CD images from cue, iso and gdi; hard disks; DVDs; raw data) and the command line.

pub mod cdsrc;
pub mod createcd;
pub mod cue;
pub mod extractcd;
pub mod gdi;
pub mod hdgeom;
pub mod rawio;
pub mod report;
pub mod types;

use std::fmt;

/// An error with a message fit to print.
#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<chd::ChdError> for Error {
    fn from(e: chd::ChdError) -> Self {
        Error(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}
