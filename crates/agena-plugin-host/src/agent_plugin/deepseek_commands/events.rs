//! Package-scoped lifecycle events. Each retained VM receives only its own
//! session's events and invokes JS listeners under its normal script budget.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

use agena_plugin_sdk::{
    PluginError, PluginErrorKind,
    host_api::{CordisWorkflowSettlement, CordisWorkflowStopReason},
};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::WorkerMessages;

pub(super) const EVENT_CAPACITY: usize = 256;
type Channel = broadcast::Sender<Event>;

#[derive(Clone, Default)]
pub(super) struct SessionEvents(Arc<Mutex<BTreeMap<i64, Weak<Channel>>>>);

impl SessionEvents {
    fn channel(&self, session: i64) -> Arc<Channel> {
        let mut sessions = self.0.lock().unwrap_or_else(|error| error.into_inner());
        sessions.retain(|_, channel| channel.strong_count() > 0);
        if let Some(channel) = sessions.get(&session).and_then(Weak::upgrade) {
            return channel;
        }
        let channel = Arc::new(broadcast::channel(EVENT_CAPACITY).0);
        sessions.insert(session, Arc::downgrade(&channel));
        channel
    }

    pub(super) fn subscribe(&self, session: i64) -> (Arc<Channel>, broadcast::Receiver<Event>) {
        let channel = self.channel(session);
        let receiver = channel.subscribe();
        (channel, receiver)
    }

    pub(super) fn subagent(&self, session: i64, entry_index: usize) -> SubagentEvents {
        SubagentEvents {
            sessions: self.clone(),
            channel: self.channel(session),
            session,
            entry_index,
        }
    }

    pub(super) fn run(&self, session: i64, id: Uuid, meta: &impl serde::Serialize) -> RunEvents {
        RunEvents {
            channel: self.channel(session),
            info: Arc::new(json!({ "id": id, "meta": meta })),
            state: Arc::new(Mutex::new(RunState::default())),
            foreground: None,
        }
    }

    pub(super) fn foreground(
        &self,
        session: i64,
        id: Uuid,
        meta: &impl serde::Serialize,
        entry_index: usize,
        messages: WorkerMessages,
    ) -> RunEvents {
        let mut run = self.run(session, id, meta);
        run.foreground = Some(ForegroundEvents {
            entry_index,
            messages,
        });
        run
    }
}

pub(super) struct SubagentEvents {
    sessions: SessionEvents,
    channel: Arc<Channel>,
    pub session: i64,
    entry_index: usize,
}

impl SubagentEvents {
    pub(super) fn emit_tree(&self, ancestor_sessions: &[i64], name: &'static str, info: Value) {
        let info = Arc::new(info);
        let _ = self.channel.send(Event {
            name,
            info: info.clone(),
            data: None,
            origin: Some(self.entry_index),
        });
        let targets: Vec<Arc<Channel>> = {
            let mut sessions = self
                .sessions
                .0
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            sessions.retain(|_, channel| channel.strong_count() > 0);
            let mut seen = std::collections::BTreeSet::new();
            seen.insert(self.session);
            ancestor_sessions
                .iter()
                .copied()
                .filter(|session| seen.insert(*session))
                .filter_map(|session| sessions.get(&session).and_then(Weak::upgrade))
                .collect()
        };
        for channel in targets {
            let _ = channel.send(Event {
                name,
                info: info.clone(),
                data: None,
                origin: None,
            });
        }
    }
}

#[derive(Clone)]
pub(super) struct Event {
    name: &'static str,
    info: Arc<Value>,
    data: Option<Arc<Value>>,
    origin: Option<usize>,
}

impl Event {
    pub(super) fn originates_from(&self, entry_index: usize) -> bool {
        self.origin == Some(entry_index)
    }

    pub(super) fn wire(&self) -> Value {
        let mut args = vec![self.info.as_ref()];
        args.extend(self.data.as_deref());
        json!({ "name": self.name, "args": args })
    }
}

#[derive(Clone)]
pub(in crate::agent_plugin) struct RunEvents {
    channel: Arc<Channel>,
    info: Arc<Value>,
    state: Arc<Mutex<RunState>>,
    foreground: Option<ForegroundEvents>,
}

#[derive(Clone)]
struct ForegroundEvents {
    entry_index: usize,
    messages: WorkerMessages,
}

#[derive(Default)]
struct RunState {
    agents_started: usize,
    open_agents: BTreeMap<usize, Value>,
    cancellation: Option<String>,
    settled: bool,
}

impl RunEvents {
    fn emit(&self, name: &'static str, data: Option<Value>) {
        let event = Event {
            name,
            info: self.info.clone(),
            data: data.map(Arc::new),
            origin: self
                .foreground
                .as_ref()
                .map(|foreground| foreground.entry_index),
        };
        // The initiating VM emits start synchronously and end before resolving
        // the result. Intermediate events reach it over the live host bridge.
        if let Some(foreground) = &self.foreground
            && !matches!(name, "workflow/start" | "workflow/end")
        {
            foreground.messages.workflow(event.clone());
        }
        let _ = self.channel.send(event);
    }

    pub(in crate::agent_plugin) fn start(&self) {
        self.emit("workflow/start", None);
    }

    pub(in crate::agent_plugin) fn start_foreground(&self) -> ForegroundRun {
        self.start();
        ForegroundRun(Some(self.clone()))
    }

    pub(super) fn record_cancellation(&self, reason: &str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.settled && state.cancellation.is_none() {
            state.cancellation = Some(
                PluginError::from_kind(
                    PluginErrorKind::Cancelled,
                    format_args!("workflow cancelled: {reason}"),
                )
                .failure
                .user
                .fallback,
            );
        }
    }

    pub(in crate::agent_plugin) fn narration(&self, kind: &str, text: &str) {
        let name = match kind {
            "phase" => "workflow/phase",
            "log" => "workflow/log",
            _ => return,
        };
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.settled {
            self.emit(name, Some(Value::String(text.to_owned())));
        }
    }

    pub(in crate::agent_plugin) fn agent(
        &self,
        seq: usize,
        label: &str,
        phase: Option<&str>,
        child: i64,
    ) -> AgentEvents {
        let mut data = json!({
            "seq": seq,
            "label": &label[..label.floor_char_boundary(label.len().min(4096))],
            "childId": child.to_string(),
        });
        if let Some(phase) = phase {
            data["phase"] =
                Value::String(phase[..phase.floor_char_boundary(phase.len().min(4096))].to_owned());
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !state.settled {
            state.agents_started += 1;
            state.open_agents.insert(seq, data.clone());
            self.emit("workflow/agent-start", Some(data));
        }
        AgentEvents {
            run: self.clone(),
            seq,
        }
    }

    fn agent_end(&self, seq: usize, outcome: &'static str) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(data) = state.open_agents.remove(&seq) {
            self.emit_agent_end(data, outcome);
        }
    }

    fn emit_agent_end(&self, mut data: Value, outcome: &'static str) {
        data["outcome"] = outcome.into();
        self.emit("workflow/agent-end", Some(data));
    }

    pub(in crate::agent_plugin) fn end(&self, settlement: CordisWorkflowSettlement) -> Value {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.settled = true;
        // Pair any child promises still held by QuickJS before publishing the
        // terminal run event.
        for (_, data) in std::mem::take(&mut state.open_agents) {
            self.emit_agent_end(data, "cancelled");
        }
        let mut data = json!({
            "stopReason": settlement.reason,
            "agentsStarted": settlement.agents_started.unwrap_or(state.agents_started),
        });
        let error = match settlement.reason {
            CordisWorkflowStopReason::Cancelled => state.cancellation.take().or(settlement.error),
            _ => settlement.error,
        };
        if let Some(error) = error {
            data["error"] = error.into();
        }
        self.emit("workflow/end", Some(data.clone()));
        data
    }
}

/// Close the observer's run even if the invocation drops its host future.
pub(in crate::agent_plugin) struct ForegroundRun(Option<RunEvents>);

impl ForegroundRun {
    pub(in crate::agent_plugin) fn finish(mut self, settlement: CordisWorkflowSettlement) -> Value {
        self.0
            .take()
            .expect("foreground run is active")
            .end(settlement)
    }
}

impl Drop for ForegroundRun {
    fn drop(&mut self) {
        if let Some(events) = self.0.take() {
            events.end(CordisWorkflowSettlement {
                reason: CordisWorkflowStopReason::Cancelled,
                error: Some("workflow cancelled".into()),
                agents_started: None,
            });
        }
    }
}

/// Once a child was observed, dropping its JS promise still pairs its event.
pub(in crate::agent_plugin) struct AgentEvents {
    run: RunEvents,
    seq: usize,
}

impl AgentEvents {
    pub(in crate::agent_plugin) fn finish(self, outcome: &'static str) {
        self.run.agent_end(self.seq, outcome);
    }
}

impl Drop for AgentEvents {
    fn drop(&mut self) {
        self.run.agent_end(self.seq, "cancelled");
    }
}
