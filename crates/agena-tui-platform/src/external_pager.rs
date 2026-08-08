//! Launching an external pager.

use std::{env, io, process::Command};

use tempfile::NamedTempFile;

#[derive(Debug)]
/// Error launching an external pager.
pub enum ExternalPagerError {
    InvalidPagerCommand(String),
    Io(std::io::Error),
    ExitFailure(i32),
    NoPagerAvailable,
}

impl std::fmt::Display for ExternalPagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPagerCommand(command) => write!(f, "invalid pager command: {command}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::ExitFailure(code) => write!(f, "pager exited with status {code}"),
            Self::NoPagerAvailable => write!(f, "no pager available; set PAGER or install less"),
        }
    }
}

impl std::error::Error for ExternalPagerError {}

impl From<std::io::Error> for ExternalPagerError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn configured_pager() -> Result<Option<(String, Vec<String>)>, ExternalPagerError> {
    let Some(pager) = env::var("PAGER")
        .ok()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(None);
    };

    let parts = shlex::split(pager.as_str())
        .ok_or_else(|| ExternalPagerError::InvalidPagerCommand(pager.clone()))?;
    let (program, args) = parts
        .split_first()
        .ok_or_else(|| ExternalPagerError::InvalidPagerCommand(pager.clone()))?;
    Ok(Some((program.to_string(), args.to_vec())))
}

fn run_pager(
    program: &str,
    args: &[String],
    path: &std::path::Path,
) -> Result<bool, ExternalPagerError> {
    let status = match Command::new(program).args(args).arg(path).status() {
        Ok(status) => status,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(ExternalPagerError::Io(error)),
    };

    if status.success() {
        Ok(true)
    } else {
        Err(ExternalPagerError::ExitFailure(status.code().unwrap_or(-1)))
    }
}

pub fn page_text(text: &str) -> Result<(), ExternalPagerError> {
    let file = NamedTempFile::new()?;
    std::fs::write(file.path(), text)?;

    if let Some((program, args)) = configured_pager()? {
        run_pager(program.as_str(), args.as_slice(), file.path())?;
        return Ok(());
    }

    if run_pager("less", &[String::from("-R")], file.path())? {
        return Ok(());
    }
    if run_pager("more", &[], file.path())? {
        return Ok(());
    }

    Err(ExternalPagerError::NoPagerAvailable)
}
