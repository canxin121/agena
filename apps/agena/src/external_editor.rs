use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::NamedTempFile;

#[derive(Debug)]
pub enum ExternalEditorError {
    MissingEditor,
    InvalidEditorCommand(String),
    Io(std::io::Error),
    ExitFailure(i32),
}

impl std::fmt::Display for ExternalEditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingEditor => write!(f, "no editor configured; set VISUAL or EDITOR"),
            Self::InvalidEditorCommand(command) => {
                write!(f, "invalid editor command: {command}")
            }
            Self::Io(error) => write!(f, "{error}"),
            Self::ExitFailure(code) => write!(f, "editor exited with status {code}"),
        }
    }
}

impl std::error::Error for ExternalEditorError {}

impl From<std::io::Error> for ExternalEditorError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn editor_command() -> Result<(String, Vec<String>), ExternalEditorError> {
    let editor = env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());

    let parts = shlex::split(editor.as_str())
        .ok_or_else(|| ExternalEditorError::InvalidEditorCommand(editor.clone()))?;
    let (program, args) = parts
        .split_first()
        .ok_or(ExternalEditorError::MissingEditor)?;
    Ok((program.to_string(), args.to_vec()))
}

pub fn edit_text(initial_text: &str) -> Result<String, ExternalEditorError> {
    let (program, args) = editor_command()?;

    let file = NamedTempFile::new()?;
    std::fs::write(file.path(), initial_text)?;
    let path: PathBuf = file.path().to_path_buf();

    let status = Command::new(&program).args(&args).arg(&path).status()?;
    if !status.success() {
        return Err(ExternalEditorError::ExitFailure(
            status.code().unwrap_or(-1),
        ));
    }

    std::fs::read_to_string(file.path()).map_err(ExternalEditorError::Io)
}

pub fn open_path(path: &Path) -> Result<(), ExternalEditorError> {
    let (program, args) = editor_command()?;
    let status = Command::new(&program).args(&args).arg(path).status()?;
    if !status.success() {
        return Err(ExternalEditorError::ExitFailure(
            status.code().unwrap_or(-1),
        ));
    }
    Ok(())
}
