//! Multi-server registry. Maps a file path to the right LSP client and
//! lazily spawns the underlying server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;
use tokio::sync::RwLock;

use crate::client::LspClient;
use crate::error::{LspError, LspResult};
use crate::server_spec::LspServerSpec;
use crate::transport::StdioTransport;

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("no LSP server matches `{0}`")]
    NoServer(String),
}

pub struct LspRegistry {
    workspace_root: PathBuf,
    client_name: String,
    client_version: String,
    servers: RwLock<HashMap<String, LspServerSpec>>,
    spawned: RwLock<HashMap<String, Arc<LspClient>>>,
}

impl LspRegistry {
    pub fn new(
        workspace_root: PathBuf,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self {
            workspace_root,
            client_name: client_name.into(),
            client_version: client_version.into(),
            servers: RwLock::new(HashMap::new()),
            spawned: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, spec: LspServerSpec) {
        let mut g = self.servers.write().await;
        g.insert(spec.name.clone(), spec);
    }

    pub async fn server_names(&self) -> Vec<String> {
        let g = self.servers.read().await;
        g.keys().cloned().collect()
    }

    /// Find the first registered server whose `file_extensions` matches
    /// the given path. Returns `None` if no server claims the file.
    pub async fn server_for_path(&self, path: &Path) -> Option<LspServerSpec> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let g = self.servers.read().await;
        g.values().find(|s| s.handles_extension(&ext)).cloned()
    }

    /// Get an already-spawned client by name, or spawn one and initialize
    /// it. Panics-free: any spawn / initialize failure surfaces as
    /// [`LspError`].
    pub async fn client_for(
        &self,
        server_name: &str,
        hint_dir: &Path,
    ) -> LspResult<Arc<LspClient>> {
        if let Some(client) = self.spawned.read().await.get(server_name).cloned() {
            return Ok(client);
        }
        let spec = {
            let g = self.servers.read().await;
            g.get(server_name)
                .cloned()
                .ok_or_else(|| LspError::UnknownServer(server_name.to_string()))?
        };
        let env: HashMap<String, String> = spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let root_dir = spec.resolve_root(hint_dir, &self.workspace_root);
        let transport =
            StdioTransport::spawn(&spec.name, &spec.command, &spec.args, &env, Some(&root_dir))
                .await?;
        let client = LspClient::new(transport);
        let root_uri = url::Url::from_directory_path(&root_dir)
            .ok()
            .and_then(|u| u.as_str().parse::<lsp_types::Uri>().ok());
        client
            .initialize(
                root_uri,
                &self.client_name,
                &self.client_version,
                spec.initialization_options.clone(),
            )
            .await?;
        let mut spawned = self.spawned.write().await;
        spawned.insert(server_name.to_string(), client.clone());
        Ok(client)
    }

    /// Convenience: spawn / fetch the right server for a file path.
    pub async fn client_for_path(&self, path: &Path) -> LspResult<Arc<LspClient>> {
        let spec = self
            .server_for_path(path)
            .await
            .ok_or_else(|| LspError::UnknownServer(path.display().to_string()))?;
        let parent = path.parent().unwrap_or(&self.workspace_root);
        self.client_for(&spec.name, parent).await
    }

    pub async fn shutdown_all(&self) {
        let mut spawned = self.spawned.write().await;
        for (_, client) in spawned.drain() {
            let _ = client.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, exts: &[&str]) -> LspServerSpec {
        LspServerSpec {
            name: name.to_string(),
            command: "true".to_string(),
            args: vec![],
            env: Default::default(),
            file_extensions: exts.iter().map(|s| s.to_string()).collect(),
            root_markers: vec![],
            initialization_options: None,
        }
    }

    #[tokio::test]
    async fn server_for_path_matches_by_extension() {
        let reg = LspRegistry::new(PathBuf::from("/tmp"), "agena", "test");
        reg.register(spec("rust-analyzer", &["rs"])).await;
        reg.register(spec("typescript-language-server", &["ts", "tsx"]))
            .await;
        let s = reg
            .server_for_path(Path::new("/tmp/lib.rs"))
            .await
            .expect("matched rs");
        assert_eq!(s.name, "rust-analyzer");
        let s = reg
            .server_for_path(Path::new("/tmp/app.tsx"))
            .await
            .expect("matched tsx");
        assert_eq!(s.name, "typescript-language-server");
        assert!(reg.server_for_path(Path::new("/tmp/x.go")).await.is_none());
    }

    #[tokio::test]
    async fn server_names_returns_registered_set() {
        let reg = LspRegistry::new(PathBuf::from("/tmp"), "agena", "test");
        reg.register(spec("a", &[])).await;
        reg.register(spec("b", &["py"])).await;
        let mut names = reg.server_names().await;
        names.sort();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn client_for_path_errors_when_no_server_matches() {
        let reg = LspRegistry::new(PathBuf::from("/tmp"), "agena", "test");
        reg.register(spec("rust-analyzer", &["rs"])).await;
        let outcome = reg.client_for_path(Path::new("/tmp/x.go")).await;
        match outcome {
            Err(LspError::UnknownServer(_)) => {}
            Err(other) => panic!("expected UnknownServer, got {other}"),
            Ok(_) => panic!("expected an error"),
        }
    }
}
