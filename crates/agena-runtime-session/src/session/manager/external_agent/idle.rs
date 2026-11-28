use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use agena_domain::ResolvedExternalAgent;

use super::*;

const MAX_IDLE_AGENTS: usize = 4;
const IDLE_PEER_TIMEOUT: Duration = Duration::from_secs(300);

pub(in crate::session::manager) type IdlePeers =
    Arc<tokio::sync::Mutex<VecDeque<(SessionId, IdlePeer)>>>;

pub(in crate::session::manager) struct IdlePeer {
    spec_fp: control_plane::AgentSpecFingerprint,
    cwd: PathBuf,
    additional_directories: Vec<PathBuf>,
    stop: CancellationToken,
    reclaim: Arc<AtomicBool>,
    native_close: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<Option<acp::AcpClient>>>,
}

impl IdlePeer {
    async fn close(mut self, native_close: bool) {
        self.reclaim.store(false, Ordering::Release);
        self.native_close.store(native_close, Ordering::Release);
        self.stop.cancel();
        if let Some(task) = self.task.take()
            && let Ok(Some(client)) = task.await
        {
            client.close().await;
        }
    }

    async fn reclaim(mut self) -> Option<acp::AcpClient> {
        self.reclaim.store(true, Ordering::Release);
        self.stop.cancel();
        let task = self.task.take()?;
        task.await.ok().flatten()
    }
}

impl Drop for IdlePeer {
    fn drop(&mut self) {
        self.reclaim.store(false, Ordering::Release);
        self.stop.cancel();
    }
}

impl SessionManager {
    pub(in crate::session::manager) async fn close_idle_external_agent(&self, session: SessionId) {
        self.stop_idle_external_agent(session, false).await;
    }

    pub(in crate::session::manager) async fn close_idle_external_agent_for_management(
        &self,
        session: SessionId,
    ) {
        self.stop_idle_external_agent(session, true).await;
    }

    async fn stop_idle_external_agent(&self, session: SessionId, native_close: bool) {
        let peer = {
            let mut peers = self.external_idle_peers.lock().await;
            let index = peers.iter().position(|(id, _)| *id == session);
            index
                .and_then(|index| peers.remove(index))
                .map(|(_, peer)| peer)
        };
        if let Some(peer) = peer {
            peer.close(native_close).await;
        }
    }

    /// Reclaims the parked warm ACP peer for `session` when its resolved spec,
    /// workspace root, and additional directories still match. Eliminates cold
    /// process spawns on consecutive chat turns (~3.4s -> ~8ms on `agy`).
    pub(super) async fn take_idle_external_agent(
        &self,
        session: SessionId,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        additional_directories: &[PathBuf],
    ) -> Option<acp::AcpClient> {
        let peer = {
            let mut peers = self.external_idle_peers.lock().await;
            let index = peers.iter().position(|(id, _)| *id == session);
            index
                .and_then(|index| peers.remove(index))
                .map(|(_, peer)| peer)
        }?;
        let expected_fp = control_plane::spec_fingerprint(spec);
        if peer.spec_fp != expected_fp
            || peer.cwd != cwd
            || peer.additional_directories != additional_directories
        {
            peer.close(false).await;
            return None;
        }
        peer.reclaim().await
    }

    pub(super) async fn park_external_agent(
        &self,
        mut client: acp::AcpClient,
        spec: &ResolvedExternalAgent,
        cwd: &Path,
        additional_directories: &[PathBuf],
        resume: ExternalAgentResume,
        state: ExternalAgentState,
    ) {
        let session = resume.session;
        let stop = self.shutdown.child_token();
        let monitor_stop = stop.clone();
        let reclaim = Arc::new(AtomicBool::new(false));
        let reclaim_requested = reclaim.clone();
        let native_close = Arc::new(AtomicBool::new(false));
        let close_requested = native_close.clone();
        let kernel = self.session_kernel();
        let input = super::input_requests::Host::new(
            self,
            session,
            client.agent().clone(),
            Some(resume.clone()),
            monitor_stop.clone(),
        );
        let task = self.control_tasks.spawn(async move {
            let exit = match client
                .watch_idle(
                    &kernel,
                    &resume,
                    state,
                    &input,
                    Some(IDLE_PEER_TIMEOUT),
                    &monitor_stop,
                )
                .await
            {
                Ok(exit) => Some(exit),
                Err(error) => {
                    tracing::debug!(session_id = session.get(), %error, "ACP idle stream ended");
                    None
                }
            };
            if exit == Some(acp::IdleExit::Cancelled)
                && reclaim_requested.load(Ordering::Acquire)
                && !close_requested.load(Ordering::Acquire)
                && client.can_reclaim_after_idle()
            {
                return Some(client);
            }
            if close_requested.load(Ordering::Acquire) {
                client
                    .close_native_session_if_supported(&resume.native_id)
                    .await;
            }
            client.close().await;
            None
        });
        let spec_fp = control_plane::spec_fingerprint(spec);
        let mut peers = self.external_idle_peers.lock().await;
        peers.retain(|(id, peer)| {
            *id != session && peer.task.as_ref().is_none_or(|task| !task.is_finished())
        });
        if peers.len() >= MAX_IDLE_AGENTS {
            // Dropping the eldest reader cancels its process and releases its
            // tracker slot. The native conversation is already durable.
            peers.pop_front();
        }
        peers.push_back((
            session,
            IdlePeer {
                spec_fp,
                cwd: cwd.to_path_buf(),
                additional_directories: additional_directories.to_vec(),
                stop,
                reclaim,
                native_close,
                task: Some(task),
            },
        ));
    }
}
