use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::AuthData;

pub trait AuthStore: Send + Sync {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError>;
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError>;
    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError>;
    fn remove(&self, provider_id: &str) -> Result<(), AppError>;
}

#[derive(Debug, Clone)]
pub struct FileAuthStore {
    path: PathBuf,
}

impl FileAuthStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path() -> PathBuf {
        if let Ok(path) = std::env::var("AGENA_AUTH_FILE") {
            return PathBuf::from(path);
        }

        let mut base = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.push(".agena");
        base.push("auth.json");
        base
    }

    fn read_file(&self) -> Result<AuthFile, AppError> {
        if !self.path.exists() {
            return Ok(AuthFile::default());
        }

        let text = fs::read_to_string(&self.path)?;
        let parsed = serde_json::from_str::<AuthFile>(&text)?;
        Ok(parsed)
    }

    fn write_file(&self, file: &AuthFile) -> Result<(), AppError> {
        if let Some(parent) = self.path.parent() {
            ensure_directory(parent)?;
        }

        let json = serde_json::to_string_pretty(file)?;
        fs::write(&self.path, json.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.path, permissions)?;
        }
        Ok(())
    }
}

impl AuthStore for FileAuthStore {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
        Ok(self.read_file()?.providers)
    }

    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
        Ok(self
            .read_file()?
            .providers
            .remove(normalize_provider_id(provider_id).as_str()))
    }

    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
        let mut data = self.read_file()?;
        data.providers
            .insert(normalize_provider_id(provider_id), auth);
        self.write_file(&data)
    }

    fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        let mut data = self.read_file()?;
        data.providers
            .remove(normalize_provider_id(provider_id).as_str());
        self.write_file(&data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    providers: HashMap<String, AuthData>,
}

fn normalize_provider_id(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

fn ensure_directory(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(path)?;
    Ok(())
}
