use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
};

use super::*;

const MAX_IDLE_AGENTS: usize = 32;

pub(in crate::session::manager) type IdlePeers =
    Arc<tokio::sync::Mutex<VecDeque<(SessionId, IdlePeer)>>>;

pub(in crate::session::manager) struct IdlePeer {
    stop: CancellationToken,
    native_close: Arc<AtomicBool>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl IdlePeer {
    async fn close(mut self, native_close: bool) {
        self.native_close.store(native_close, Ordering::Release);
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for IdlePeer {
    fn drop(&mut self) {
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

    pub(super) async fn park_external_agent(
        &self,
        mut client: acp::AcpClient,
        resume: ExternalAgentResume,
        state: ExternalAgentState,
    ) {
        let session = resume.session;
        let stop = self.shutdown.child_token();
        let monitor_stop = stop.clone();
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
            if let Err(error) = client
                .watch_idle(&kernel, &resume, state, &input, &monitor_stop)
                .await
            {
                tracing::debug!(session_id = session.get(), %error, "ACP idle stream ended");
            }
            if close_requested.load(Ordering::Acquire) {
                client
                    .close_native_session_if_supported(&resume.native_id)
                    .await;
            }
            client.close().await;
        });
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
                stop,
                native_close,
                task: Some(task),
            },
        ));
    }
}
