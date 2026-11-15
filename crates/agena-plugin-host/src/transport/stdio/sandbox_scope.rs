//! Bind callbacks to the process and live requests that received their authority.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::sdk::host_api::HostCallbackContext;
use crate::sdk::{PluginError, PluginErrorKind};

#[derive(Clone, PartialEq, Eq)]
struct BoundScope {
    plugin_id: Option<String>,
    session_id: Option<i64>,
    workspace_root: Option<String>,
    call_id: Option<i64>,
    call_bound: bool,
}

impl BoundScope {
    fn accepts(&self, context: &HostCallbackContext) -> bool {
        self.plugin_id == context.plugin_id
            && self.session_id == context.session_id
            && self.workspace_root == context.workspace_root
            && (!self.call_bound || self.call_id == context.call_id)
            && context.authority_token.is_some()
    }
}

#[derive(Default)]
pub(super) struct CallbackBoundary {
    scope: RwLock<Option<BoundScope>>,
    authorities: Mutex<HashMap<String, usize>>,
}

impl CallbackBoundary {
    pub(super) fn restrict(
        &self,
        context: &HostCallbackContext,
        call_bound: bool,
    ) -> Result<(), PluginError> {
        if context.authority_token.is_none() {
            return Err(denied(
                "scoped plugin process requires an active host authority",
            ));
        }
        let bound = BoundScope {
            plugin_id: context.plugin_id.clone(),
            session_id: context.session_id,
            workspace_root: context.workspace_root.clone(),
            call_id: context.call_id,
            call_bound,
        };
        let mut scope = self
            .scope
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if scope.as_ref().is_some_and(|current| current != &bound) {
            return Err(denied("plugin process callback scope cannot be changed"));
        }
        *scope = Some(bound);
        Ok(())
    }

    pub(super) fn is_restricted(&self) -> bool {
        self.scope
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
    }

    pub(super) fn admit(
        self: &Arc<Self>,
        context: Option<&HostCallbackContext>,
    ) -> Result<CallbackLease, PluginError> {
        if let Some(scope) = self
            .scope
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
        {
            if context.is_none_or(|context| !scope.accepts(context)) {
                return Err(denied(
                    "outbound call does not match the plugin process scope",
                ));
            }
        }
        let token = context.and_then(|context| context.authority_token.clone());
        if let Some(token) = &token {
            *self
                .authorities
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .entry(token.clone())
                .or_default() += 1;
        }
        Ok(CallbackLease {
            boundary: self.clone(),
            token,
        })
    }

    pub(super) fn validate(&self, params: &serde_json::Value) -> Result<(), PluginError> {
        let scope = self.scope.read().unwrap_or_else(|error| error.into_inner());
        let Some(scope) = scope.as_ref() else {
            return Ok(());
        };
        let context = params
            .get("context")
            .cloned()
            .and_then(|value| serde_json::from_value::<HostCallbackContext>(value).ok())
            .ok_or_else(|| denied("scoped plugin callback is missing its host authority"))?;
        if !scope.accepts(&context) {
            return Err(denied("plugin callback belongs to another process scope"));
        }
        let token = context
            .authority_token
            .as_ref()
            .ok_or_else(|| denied("scoped plugin callback is missing its host authority"))?;
        if !self
            .authorities
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(token)
        {
            return Err(denied(
                "plugin callback authority was not issued to an active request in this process",
            ));
        }
        Ok(())
    }
}

pub(super) struct CallbackLease {
    boundary: Arc<CallbackBoundary>,
    token: Option<String>,
}

impl Drop for CallbackLease {
    fn drop(&mut self) {
        let Some(token) = &self.token else {
            return;
        };
        let mut authorities = self
            .boundary
            .authorities
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(count) = authorities.get_mut(token) {
            *count -= 1;
            if *count == 0 {
                authorities.remove(token);
            }
        }
    }
}

fn denied(message: &str) -> PluginError {
    PluginError::from_kind(PluginErrorKind::PolicyDenied, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(token: &str) -> HostCallbackContext {
        HostCallbackContext {
            plugin_id: Some("test.scoped".into()),
            session_id: Some(10),
            call_id: Some(20),
            workspace_root: Some("/workspace".into()),
            authority_token: Some(token.into()),
            ..Default::default()
        }
    }

    #[test]
    fn callbacks_cannot_borrow_another_process_active_authority() {
        let first = Arc::new(CallbackBoundary::default());
        let second = Arc::new(CallbackBoundary::default());
        let context = context("live-token");
        first.restrict(&context, false).unwrap();
        second.restrict(&context, false).unwrap();
        let _request = first.admit(Some(&context)).unwrap();
        let params = serde_json::json!({"context": context});
        assert!(first.validate(&params).is_ok());
        assert!(second.validate(&params).is_err());
    }

    #[test]
    fn callback_authority_ends_with_its_last_request_or_stream_lease() {
        let boundary = Arc::new(CallbackBoundary::default());
        let context = context("live-token");
        boundary.restrict(&context, false).unwrap();
        let first = boundary.admit(Some(&context)).unwrap();
        let second = boundary.admit(Some(&context)).unwrap();
        let params = serde_json::json!({"context": context});
        drop(first);
        assert!(boundary.validate(&params).is_ok());
        drop(second);
        assert!(boundary.validate(&params).is_err());
    }

    #[test]
    fn callback_scope_rejects_session_and_one_call_grant_changes() {
        let boundary = Arc::new(CallbackBoundary::default());
        let context = context("live-token");
        boundary.restrict(&context, true).unwrap();
        let _request = boundary.admit(Some(&context)).unwrap();
        let mut other = context.clone();
        other.session_id = Some(11);
        assert!(boundary.admit(Some(&other)).is_err());
        assert!(
            boundary
                .validate(&serde_json::json!({"context": other}))
                .is_err()
        );
        let mut other = context;
        other.call_id = Some(21);
        assert!(boundary.admit(Some(&other)).is_err());
        assert!(
            boundary
                .validate(&serde_json::json!({"context": other}))
                .is_err()
        );
        assert!(boundary.validate(&serde_json::json!({})).is_err());
    }
}
