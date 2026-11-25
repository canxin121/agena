use super::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{sync::OnceLock, time::Instant};

const CURSOR_TTL: Duration = Duration::from_secs(1800);
const MAX_CURSORS: usize = 4096;

// A scan position may name inaccessible rows. Keep it server-side so an
// empty page does not disclose those session IDs through its continuation.
struct DiscoveryContinuation {
    scope: Value,
    position: Value,
    created: Instant,
}

fn continuations() -> &'static StdMutex<HashMap<String, DiscoveryContinuation>> {
    static CONTINUATIONS: OnceLock<StdMutex<HashMap<String, DiscoveryContinuation>>> =
        OnceLock::new();
    CONTINUATIONS.get_or_init(Default::default)
}

#[cfg(test)]
pub(super) fn expire_for_test(token: &str) {
    continuations()
        .lock()
        .unwrap()
        .get_mut(token)
        .unwrap()
        .created = Instant::now() - CURSOR_TTL - Duration::from_secs(1);
}

pub(super) fn resume<T: DeserializeOwned>(
    token: &Option<String>,
    scope: &Value,
) -> Result<Option<T>, AppError> {
    let Some(token) = token else { return Ok(None) };
    if token.len() > 128 || !token.starts_with("bd1.") {
        return Err(AppError::Config(
            "invalid discovery cursor; restart without cursor".into(),
        ));
    }
    let registry = continuations().lock().unwrap_or_else(|p| p.into_inner());
    let value = registry.get(token).ok_or_else(|| {
        AppError::Config("stale or invalid discovery cursor; restart without cursor".into())
    })?;
    if value.created.elapsed() > CURSOR_TTL {
        return Err(AppError::Config(
            "expired discovery cursor; restart without cursor".into(),
        ));
    }
    if value.scope != *scope {
        return Err(AppError::Config(
            "cursor belongs to a different action, target, query or filter; restart without cursor"
                .into(),
        ));
    }
    serde_json::from_value(value.position.clone())
        .map(Some)
        .map_err(|_| {
            AppError::Config("invalid discovery cursor position; restart without cursor".into())
        })
}

pub(super) fn continuation(scope: &Value, position: &impl Serialize) -> Result<String, AppError> {
    let position = serde_json::to_value(position)?;
    let mut registry = continuations().lock().unwrap_or_else(|p| p.into_inner());
    registry.retain(|_, value| value.created.elapsed() <= CURSOR_TTL);
    if registry.len() >= MAX_CURSORS
        && let Some(oldest) = registry
            .iter()
            .min_by_key(|(_, value)| value.created)
            .map(|(key, _)| key.clone())
    {
        registry.remove(&oldest);
    }
    let token = format!("bd1.{}", uuid::Uuid::new_v4());
    registry.insert(
        token.clone(),
        DiscoveryContinuation {
            scope: scope.clone(),
            position,
            created: Instant::now(),
        },
    );
    Ok(token)
}
