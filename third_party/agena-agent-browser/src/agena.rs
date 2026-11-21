//! Native browser driver entry points used by Agena.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::native::{actions::DaemonState, browser::BrowserManager};
use crate::session_settings::{MemStore, Running};

pub use crate::native::cdp::chrome::{ChromeProcess, ChromeSandbox};
pub use crate::native::cdp::client::{is_session_detached_error, CdpClient};
pub use crate::native::cdp::types::CdpEvent;

pub async fn connect_cdp(endpoint: &str) -> Result<CdpClient, String> {
    CdpClient::connect_bounded(endpoint).await
}

pub fn launch_chrome(
    executable: &Path,
    timeout: Duration,
    sandbox: ChromeSandbox,
) -> Result<ChromeProcess, String> {
    crate::native::cdp::chrome::launch_agena_chrome(executable, timeout, sandbox)
}

pub struct PageEngine {
    state: DaemonState,
    document_revision: u64,
}

impl PageEngine {
    pub fn new(
        client: Arc<CdpClient>,
        endpoint: String,
        target_id: String,
        session_id: String,
    ) -> Self {
        let mut state = DaemonState::with_store(Running::defaults(), Arc::new(MemStore::default()));
        state.browser = Some(BrowserManager::from_agena_page(
            client, endpoint, target_id, session_id,
        ));
        Self {
            state,
            document_revision: 0,
        }
    }

    pub async fn action(
        &mut self,
        command: &Value,
        iframe_sessions: HashMap<String, String>,
        document_revision: u64,
    ) -> Result<Value, String> {
        if self.document_revision != document_revision {
            self.state.ref_map.invalidate_all_documents();
            self.document_revision = document_revision;
        }
        self.state.iframe_sessions = iframe_sessions;
        self.state.agena_action(command).await
    }
}
