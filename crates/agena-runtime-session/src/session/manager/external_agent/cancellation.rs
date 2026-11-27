use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Mutex, Weak},
    time::Duration,
};

use agena_domain::{ExternalAgentWorkItem, ExternalAgentWorkKind as Kind};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::*;

const MAX_WORK_ITEMS: usize = 128;
const MAX_CHILDREN: usize = 24;
pub(super) const MAX_REQUESTS: usize = 8;
pub(super) const ASYNC_TASKS_CAPABILITY: &str = "asyncTasks";

pub(in crate::session::manager) type Slot = Mutex<Weak<Registry>>;

pub(in crate::session::manager) struct Registry {
    work: Mutex<Work>,
    requests: mpsc::Sender<Request>,
}

pub(super) struct Connection {
    // The manager's dispatch table stores weak references.
    _dispatch: Arc<SessionDispatch>,
    registry: Arc<Registry>,
    receiver: mpsc::Receiver<Request>,
    air_tasks: bool,
    pub(super) pending: BTreeMap<u64, (Option<Request>, Target)>,
}

pub(super) struct Request {
    id: Uuid,
    pub(super) answer: oneshot::Sender<Result<(), AppError>>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Target {
    kind: Kind,
    session: String,
    id: String,
}

impl Target {
    fn child(session: &str) -> Self {
        Self {
            kind: Kind::Subagent,
            session: session.into(),
            id: session.into(),
        }
    }

    fn item(source: &str, update: &Value, kind: Kind, id_key: &str) -> Option<Self> {
        Some(Self {
            kind,
            session: source.into(),
            id: identifier(&update[id_key])?.into(),
        })
    }

    pub(super) fn method(&self) -> &'static str {
        match self.kind {
            Kind::ToolCall => "tool_call/cancel",
            Kind::Terminal => "terminal/cancel",
            Kind::Subagent => "session/cancel",
            Kind::AsyncTask => "_session/async_task/stop",
        }
    }

    pub(super) fn rpc_id(&self, id: u64) -> Option<u64> {
        (self.kind != Kind::Subagent).then_some(id)
    }

    fn params(&self, root: &str) -> Value {
        match self.kind {
            Kind::ToolCall => json!({"sessionId": self.session, "toolCallId": self.id}),
            Kind::Terminal => json!({"sessionId": self.session, "terminalId": self.id}),
            Kind::Subagent => json!({"sessionId": self.session}),
            // The AIR update is routed to the task's display session. Claude's
            // stop method addresses the root that owns the SDK task registry.
            Kind::AsyncTask => json!({"sessionId": root, "asyncTaskId": self.id}),
        }
    }

    pub(super) fn declined(&self, result: &Value) -> bool {
        self.kind == Kind::AsyncTask && result["stopped"] == false
    }

    pub(super) fn accepted(&self, result: &Value) -> bool {
        if self.kind == Kind::AsyncTask {
            result["stopped"] == true
        } else {
            result.is_object()
        }
    }
}

struct Item {
    title: Option<String>,
    capable: bool,
    active: bool,
    cancel_id: Option<Uuid>,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            title: None,
            capable: false,
            active: true,
            cancel_id: None,
        }
    }
}

impl Item {
    fn update(&mut self, title: Option<&Value>, capable: Option<bool>, active: Option<bool>) {
        if let Some(title) = title {
            self.title = title
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(|text| {
                    // Titles are displayed in the terminal picker as well as Web.
                    text[..text.floor_char_boundary(256)]
                        .chars()
                        .filter(|ch| !ch.is_control())
                        .collect()
                });
        }
        let was_available = self.capable && self.active;
        if let Some(capable) = capable {
            self.capable = capable;
        }
        if let Some(active) = active {
            self.active = active;
        }
        if !self.capable || !self.active {
            self.cancel_id = None;
        } else if !was_available {
            self.cancel_id = Some(Uuid::new_v4());
        }
    }
}

#[derive(Default)]
struct Work {
    root: Option<String>,
    children: BTreeMap<String, String>,
    items: BTreeMap<Target, Item>,
    background: BTreeMap<String, BTreeSet<Target>>,
}

impl Work {
    fn item(&mut self, target: Target) -> Option<&mut Item> {
        if self.items.len() >= MAX_WORK_ITEMS && !self.items.contains_key(&target) {
            let discard = self
                .items
                .iter()
                .find(|(key, item)| key.kind != Kind::Subagent && (!item.capable || !item.active))
                .map(|(key, _)| key.clone())?;
            self.items.remove(&discard);
        }
        Some(self.items.entry(target).or_default())
    }

    fn observe(&mut self, source: &str, update: &Value) {
        if self.root.as_deref() != Some(source) && !self.children.contains_key(source) {
            return;
        }
        match update["sessionUpdate"].as_str() {
            Some("tool_call" | "tool_call_update") => {
                let Some(item) = Target::item(source, update, Kind::ToolCall, "toolCallId")
                    .and_then(|target| self.item(target))
                else {
                    return;
                };
                let finished = matches!(
                    update["status"].as_str(),
                    Some("completed" | "failed" | "cancelled")
                );
                item.update(
                    update.get("title").or_else(|| update.get("name")),
                    cancel_capability(update),
                    finished.then_some(false),
                );
            }
            Some("terminal_update") => {
                let Some(item) = Target::item(source, update, Kind::Terminal, "terminalId")
                    .and_then(|target| self.item(target))
                else {
                    return;
                };
                item.update(
                    update.get("command"),
                    cancel_capability(update),
                    update["exitStatus"].is_object().then_some(false),
                );
            }
            Some("subagent_update") => self.child(source, update),
            Some("state_update") => self.observe_state(source, update),
            _ => {}
        }
    }

    fn observe_air(&mut self, source: &str, update: &Value) {
        if self.root.as_deref() != Some(source) && !self.children.contains_key(source) {
            return;
        }
        if update["sessionUpdate"] == "subagent_update" {
            self.child(source, update);
            return;
        }
        let Some(target) = Target::item(source, update, Kind::AsyncTask, "asyncTaskId") else {
            return;
        };
        // The adapter trims these two wire IDs before resolving the task.
        if target.id.trim() != target.id
            || self.root.as_deref().is_none_or(|root| root.trim() != root)
        {
            return;
        }
        match update["sessionUpdate"].as_str() {
            Some("async_task_spawned") => {
                if let Some(item) = self.item(target) {
                    item.update(update.get("name"), Some(update["canStop"] == true), None);
                }
            }
            Some("async_task_state_update") => {
                if let Some(item) = self.items.get_mut(&target)
                    && matches!(
                        update["state"].as_str(),
                        Some("completed" | "failed" | "stopped")
                    )
                {
                    item.update(None, None, Some(false));
                }
            }
            _ => {}
        }
    }

    fn child(&mut self, source: &str, update: &Value) {
        let Some(child) = identifier(&update["sessionId"])
            .filter(|child| Some(*child) != self.root.as_deref() && *child != source)
        else {
            return;
        };
        if let Some(parent) = self.children.get(child) {
            if parent != source {
                return;
            }
        } else {
            if self.children.len() >= MAX_CHILDREN {
                let Some(evict) = self
                    .children
                    .keys()
                    .find(|id| {
                        self.items
                            .get(&Target::child(id))
                            .is_none_or(|item| !item.active)
                    })
                    .cloned()
                else {
                    return;
                };
                self.children.remove(&evict);
                self.background.remove(&evict);
                self.items.retain(|target, _| target.session != evict);
            }
            self.children.insert(child.into(), source.into());
        }
        if let Some(item) = self.item(Target::child(child)) {
            item.update(update.get("title"), cancel_capability(update), None);
        }
        if let Some(state) = update.get("state") {
            self.observe_state(child, state);
        }
    }

    fn observe_state(&mut self, source: &str, state: &Value) {
        if self.children.contains_key(source)
            && let Some(item) = self.items.get_mut(&Target::child(source))
        {
            let active = match state["state"].as_str() {
                Some("idle") => Some(false),
                Some("running" | "requires_action") => Some(true),
                _ => None,
            };
            item.update(None, None, active);
        }
        let next: Option<BTreeSet<_>> = state["background"].as_array().map(|items| {
            items
                .iter()
                .take(MAX_WORK_ITEMS)
                .filter_map(|item| match item["type"].as_str()? {
                    "tool_call" => Target::item(source, item, Kind::ToolCall, "toolCallId"),
                    "terminal" => Target::item(source, item, Kind::Terminal, "terminalId"),
                    _ => None,
                })
                .collect()
        });
        if state["state"] == "idle" {
            let active_background = next.as_ref().or_else(|| self.background.get(source));
            for (target, item) in &mut self.items {
                if target.session == source
                    && !matches!(target.kind, Kind::Subagent | Kind::AsyncTask)
                    && !active_background.is_some_and(|bg| bg.contains(target))
                {
                    item.update(None, None, Some(false));
                }
            }
        }
        if let Some(next) = next {
            if let Some(previous) = self.background.remove(source) {
                for stopped in previous.difference(&next) {
                    if let Some(item) = self.items.get_mut(stopped) {
                        item.update(None, None, Some(false));
                    }
                }
            }
            self.background.insert(source.into(), next);
        }
    }
}

fn identifier(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| id.len() <= 4096 && !id.trim().is_empty() && !id.chars().any(char::is_control))
}

fn cancel_capability(update: &Value) -> Option<bool> {
    update
        .get("capabilities")
        .map(|capability| capability["cancel"].is_object())
}

impl Registry {
    fn work(&self) -> std::sync::MutexGuard<'_, Work> {
        self.work.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Connection {
    pub(super) fn register(turn: &ExternalTurn, air_tasks: bool) -> Self {
        let dispatch = turn.manager.session_dispatch(turn.control.session);
        let (requests, receiver) = mpsc::channel(MAX_REQUESTS);
        let registry = Arc::new(Registry {
            work: Mutex::default(),
            requests,
        });
        *dispatch
            .external_work
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(&registry);
        Self {
            _dispatch: dispatch,
            registry,
            receiver,
            air_tasks,
            pending: BTreeMap::new(),
        }
    }

    pub(super) fn set_root(&self, id: Option<&str>) {
        let mut work = self.registry.work();
        if work.root.as_deref() != id {
            *work = Work {
                root: id.map(str::to_owned),
                ..Default::default()
            };
        }
    }

    pub(super) fn observe(&self, source: &str, update: &Value) {
        let mut work = self.registry.work();
        if self.air_tasks {
            work.observe_air(source, update);
        } else {
            work.observe(source, update);
        }
    }

    pub(super) fn claim(&self, request: &Request) -> Option<(Target, Value)> {
        let mut work = self.registry.work();
        let Work { root, items, .. } = &mut *work;
        let root = root.as_deref()?;
        let (target, item) = items
            .iter_mut()
            .find(|(_, item)| item.cancel_id == Some(request.id))?;
        item.cancel_id = None;
        Some((target.clone(), target.params(root)))
    }

    pub(super) fn reoffer(&self, target: &Target) {
        if let Some(item) = self.registry.work().items.get_mut(target)
            && item.capable
            && item.active
            && item.cancel_id.is_none()
        {
            item.cancel_id = Some(Uuid::new_v4());
        }
    }
}

pub(super) enum Event {
    Cancel(Request),
    Abandoned(u64),
}

pub(super) async fn receive(connection: &mut Option<Connection>) -> Option<Event> {
    let Some(connection) = connection else {
        return std::future::pending().await;
    };
    let pending = &mut connection.pending;
    tokio::select! {
        biased;
        id = std::future::poll_fn(|cx| {
            pending.iter_mut().find_map(|(&id, (request, _))| {
                request.as_mut()?.answer.poll_closed(cx).is_ready().then_some(id)
            }).map_or(std::task::Poll::Pending, std::task::Poll::Ready)
        }) => Some(Event::Abandoned(id)),
        request = connection.receiver.recv() => request.map(Event::Cancel),
    }
}

pub(super) fn unavailable() -> AppError {
    AppError::Config(
        "This external agent work item is no longer available to cancel. Refresh the work list."
            .into(),
    )
}

pub(super) fn busy() -> AppError {
    AppError::Config("The external agent is already receiving cancellation requests.".into())
}

impl SessionManager {
    pub(in crate::session::manager) async fn handle_external_agent_work(
        &self,
        id: i64,
        cancel_id: Option<Uuid>,
    ) -> Result<Vec<ExternalAgentWorkItem>, AppError> {
        let session = session_id(id)?;
        if let Some(cancel_id) = cancel_id {
            self.require_writable_bot_history(id).await?;
            let registry = self.native_work_registry(session).ok_or_else(unavailable)?;
            let (answer, response) = oneshot::channel();
            registry
                .requests
                .try_send(Request {
                    id: cancel_id,
                    answer,
                })
                .map_err(|error| match error {
                    mpsc::error::TrySendError::Full(_) => busy(),
                    mpsc::error::TrySendError::Closed(_) => unavailable(),
                })?;
            tokio::time::timeout(Duration::from_secs(10), response)
                .await
                .map_err(|_| {
                    AppError::Config("The external agent did not acknowledge cancellation.".into())
                })?
                .map_err(|_| unavailable())??;
        } else if self.external_agent(session).await?.is_none() {
            return Err(AppError::Config(
                "This session has no external agent.".into(),
            ));
        }
        Ok(self
            .native_work_registry(session)
            .map(|registry| {
                registry
                    .work()
                    .items
                    .iter()
                    .filter_map(|(target, item)| {
                        Some(ExternalAgentWorkItem {
                            cancel_id: item.cancel_id?,
                            kind: target.kind,
                            session_id: target.session.clone(),
                            target_id: target.id.clone(),
                            title: item.title.clone(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn native_work_registry(&self, session: SessionId) -> Option<Arc<Registry>> {
        self.session_dispatch(session)
            .external_work
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .upgrade()
            .filter(|registry| !registry.requests.is_closed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_evicts_inactive_child_beyond_max_children() {
        let mut work = Work {
            root: Some("root".into()),
            ..Default::default()
        };
        for i in 0..MAX_CHILDREN {
            let id = format!("child-{i}");
            work.observe(
                "root",
                &json!({
                    "sessionUpdate": "subagent_update",
                    "sessionId": id,
                    "title": format!("Worker {i}"),
                    "capabilities": {"cancel": {}},
                    "state": {"state": if i == 0 { "idle" } else { "running" }}
                }),
            );
        }
        assert_eq!(work.children.len(), MAX_CHILDREN);
        // A 25th subagent should evict idle child-0 rather than being dropped.
        work.observe(
            "root",
            &json!({
                "sessionUpdate": "subagent_update",
                "sessionId": "child-24",
                "title": "Worker 24",
                "capabilities": {"cancel": {}},
                "state": {"state": "running"}
            }),
        );
        assert!(work.children.contains_key("child-24"));
        assert!(!work.children.contains_key("child-0"));
    }

    #[test]
    fn idle_state_without_background_still_clears_finished_foreground() {
        let mut work = Work {
            root: Some("root".into()),
            ..Default::default()
        };
        work.observe(
            "root",
            &json!({
                "sessionUpdate": "tool_call",
                "toolCallId": "tool-1",
                "title": "Run check",
                "capabilities": {"cancel": {}}
            }),
        );
        assert_eq!(work.items.len(), 1);
        assert!(work.items.values().all(|item| item.active && item.cancel_id.is_some()));
        work.observe(
            "root",
            &json!({
                "sessionUpdate": "state_update",
                "state": "idle"
            }),
        );
        assert!(work.items.values().all(|item| !item.active && item.cancel_id.is_none()));
    }
}
