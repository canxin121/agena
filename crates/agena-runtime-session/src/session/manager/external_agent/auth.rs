use std::{
    collections::BTreeMap,
    sync::{Mutex, Weak},
};

use agena_domain::{
    ExternalAgentAuth, ExternalAgentAuthAccount, ExternalAgentAuthAction, ExternalAgentAuthFlow,
    ExternalAgentAuthMethod, ExternalAgentAuthStatus, validate_external_agent_terminal_launch,
};

use super::*;

type SharedStatus = Mutex<Option<ExternalAgentAuthStatus>>;
pub(in crate::session::manager) type StatusSlot = Mutex<Weak<SharedStatus>>;

pub(super) struct LiveStatus {
    _dispatch: Arc<SessionDispatch>,
    status: Arc<SharedStatus>,
}

impl LiveStatus {
    pub(super) fn register(manager: &SessionManager, session: SessionId) -> Self {
        let dispatch = manager.session_dispatch(session);
        let status = Arc::new(Mutex::new(None));
        *dispatch
            .external_auth
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Arc::downgrade(&status);
        Self {
            _dispatch: dispatch,
            status,
        }
    }

    pub(super) fn update(&self, status: ExternalAgentAuthStatus) {
        *self
            .status
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(status);
    }
}

impl SessionManager {
    pub(in crate::session::manager) async fn handle_external_agent_live_auth(
        &self,
        id: i64,
    ) -> Result<Option<ExternalAgentAuthStatus>, AppError> {
        let session = session_id(id)?;
        if self.external_agent(session).await?.is_none() {
            return Err(AppError::Config(
                "This session has no external agent.".into(),
            ));
        }
        let status = self
            .session_dispatch(session)
            .external_auth
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .upgrade();
        Ok(status.and_then(|status| {
            status
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }))
    }

    pub(in crate::session::manager) async fn handle_external_agent_auth(
        &self,
        id: i64,
        action: ExternalAgentAuthAction,
    ) -> Result<ExternalAgentAuth, AppError> {
        let session = session_id(id)?;
        self.require_writable_bot_history(id).await?;
        let configuration = read_with_busy_retry(session, "external_agent_auth", || {
            self.kernel().reader.configuration(session)
        })
        .await?;
        let agent = configuration
            .external_agent
            .ok_or_else(|| AppError::Config("This session has no external agent.".into()))?;
        let _permit = self.reserve_execution()?;
        let state = self.state_for_session(session).await?;
        let context = self
            .execution_context_for_execution(session, &state, None)
            .await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let cwd = executor.workspace_root();
        if !cwd.is_absolute() {
            return Err(AppError::Config(
                "External agent sign-in requires a project workspace.".into(),
            ));
        }
        let mut client = acp::AcpClient::spawn_profile(
            agent,
            cwd,
            configuration
                .external_agent_state
                .antigravity_legacy_history,
            &[],
        )?;
        let terminal = matches!(action, ExternalAgentAuthAction::Inspect { terminal: true });
        // Claude advertises its native methods when the client can reproduce
        // the invocation in a terminal. Web only receives methods it can run.
        if terminal || agent == ExternalAgent::ClaudeCode {
            client.enable_terminal_auth();
        }
        let cancel = self.shutdown.child_token();
        let result = async {
            let initialization = client.initialization(&cancel).await?.clone();
            let mut methods: Vec<_> = initialization["authMethods"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| parse_method(entry, terminal))
                .take(32)
                .collect();
            for method in &mut methods {
                if let ExternalAgentAuthFlow::Terminal { args, env } = &mut method.flow {
                    args.extend_from_slice(client.agent_arguments());
                    validate_external_agent_terminal_launch(args, env)
                        .map_err(|error| AppError::Config(error.into()))?;
                }
            }
            let auth = &initialization[if client.uses_v2() {
                "capabilities"
            } else {
                "agentCapabilities"
            }]["auth"];
            let status_supported = client.supports_auth_status();
            let logout_supported = if client.uses_v2() {
                initialization["authMethods"]
                    .as_array()
                    .is_some_and(|methods| !methods.is_empty())
            } else {
                supports(&auth["logout"])
            };
            match action {
                ExternalAgentAuthAction::Inspect { .. } => {}
                ExternalAgentAuthAction::Login { method_id } => {
                    if !methods.iter().any(|method| {
                        method.id == method_id
                            && matches!(method.flow, ExternalAgentAuthFlow::Agent)
                    }) {
                        return Err(AppError::Config(
                            "Choose a sign-in method advertised by the external agent.".into(),
                        ));
                    }
                    let method = if client.uses_v2() {
                        "auth/login"
                    } else {
                        "authenticate"
                    };
                    let host = super::input_requests::Host::new(
                        self,
                        session,
                        agent,
                        None,
                        cancel.clone(),
                    );
                    client
                        .call_with_input(method, json!({"methodId": method_id}), &host, &cancel)
                        .await?;
                }
                ExternalAgentAuthAction::Logout => {
                    if !logout_supported {
                        return Err(AppError::Config(
                            "The external agent did not advertise sign-out.".into(),
                        ));
                    }
                    let method = if client.uses_v2() {
                        "auth/logout"
                    } else {
                        "logout"
                    };
                    client.call(method, json!({}), &cancel).await?;
                }
            }
            let status = if status_supported {
                match client.call("auth/status", json!({}), &cancel).await {
                    Ok(status) => parse_status(&status),
                    Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                    Err(error) => {
                        tracing::debug!(%error, "external agent did not report sign-in status");
                        None
                    }
                }
            } else {
                None
            }
            .or_else(|| client.reported_auth_status().cloned());
            let (state, accounts) = status.map_or((None, Vec::new()), |status| {
                (Some(status.state), status.accounts)
            });
            Ok(ExternalAgentAuth {
                agent,
                cwd: cwd.to_owned(),
                methods,
                logout_supported,
                state,
                accounts,
            })
        }
        .await;
        client.close().await;
        result
    }
}

fn supports(value: &Value) -> bool {
    value.is_object() || value == true
}

fn parse_method(entry: &Value, terminal: bool) -> Option<ExternalAgentAuthMethod> {
    let raw_id = entry.get("methodId").unwrap_or(&entry["id"]);
    let id = text(raw_id, 1024)?;
    // An ID is passed back to the agent verbatim.
    if raw_id.as_str()? != id {
        return None;
    }
    let flow = match entry["type"].as_str() {
        None | Some("agent") => ExternalAgentAuthFlow::Agent,
        Some("terminal") if terminal => {
            let args: Vec<String> = match entry["args"].as_array() {
                Some(values) => values
                    .iter()
                    .map(|arg| arg.as_str().map(str::to_owned))
                    .collect::<Option<_>>()?,
                None if entry["args"].is_null() => Vec::new(),
                _ => return None,
            };
            let env = terminal_environment(&entry["env"])?;
            validate_external_agent_terminal_launch(&args, &env).ok()?;
            ExternalAgentAuthFlow::Terminal { args, env }
        }
        _ => return None,
    };
    Some(ExternalAgentAuthMethod {
        id,
        name: text(&entry["name"], 256)?,
        description: text(&entry["description"], 1024),
        flow,
    })
}

fn terminal_environment(value: &Value) -> Option<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    let mut insert = |name: &str, value: &Value| -> Option<()> {
        if env
            .insert(name.to_owned(), value.as_str()?.to_owned())
            .is_some()
        {
            return None;
        }
        Some(())
    };
    match value {
        Value::Null => {}
        Value::Object(values) => {
            for (name, value) in values {
                insert(name, value)?;
            }
        }
        Value::Array(values) => {
            for variable in values {
                insert(variable["name"].as_str()?, &variable["value"])?;
            }
        }
        _ => return None,
    }
    Some(env)
}

pub(super) fn parse_status(value: &Value) -> Option<ExternalAgentAuthStatus> {
    let state = match value["state"].as_str()? {
        state @ ("authenticated" | "unauthenticated" | "not_required") => state,
        _ => "unknown",
    };
    Some(ExternalAgentAuthStatus {
        state: state.to_owned(),
        accounts: value["accounts"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(parse_account)
            .take(32)
            .collect(),
    })
}

fn parse_account(entry: &Value) -> Option<ExternalAgentAuthAccount> {
    Some(ExternalAgentAuthAccount {
        label: text(&entry["label"], 256)?,
        active: entry["active"] == true,
        description: text(&entry["description"], 512),
        name: text(&entry["name"], 256),
        email: text(&entry["email"], 320),
        organization: text(&entry["organization"], 256),
    })
}

fn text(value: &Value, max: usize) -> Option<String> {
    let value = value.as_str()?.trim();
    let value: String = value[..value.floor_char_boundary(max)]
        .chars()
        .filter(|ch| !ch.is_control())
        .collect();
    (!value.is_empty()).then_some(value)
}
