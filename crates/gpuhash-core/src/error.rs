//! Crate-wide error type.
//!
//! Library code uses `thiserror` for typed, matchable errors. Binaries (`gpuhash-cli`,
//! the Tauri shell) are free to wrap these in `anyhow::Error` for convenience.

use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("invalid hash file: {0}")]
    BadFormat(String),

    #[error("gpu error: {0}")]
    Gpu(String),

    #[error("attack cancelled")]
    Cancelled,

    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
