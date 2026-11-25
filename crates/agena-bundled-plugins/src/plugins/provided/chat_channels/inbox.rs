use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use agena_plugin_sdk::{
    PluginError, Result,
    host_api::{
        HostClient, HostStorageGetRequest, HostStorageScope, HostStorageSetRequest,
        HostStorageVisibility, current_host_callback_context,
        run_in_isolated_host_callback_context,
    },
    integration::{AdapterBatch, AdapterPollRequest, ExternalEvent},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::config::Connection;

const NAMESPACE: &str = "channel-inbox";
const RETENTION_MS: i64 = 48 * 60 * 60 * 1000;
const MAX_RECORDS: usize = 4096;
const MAX_INBOX_BYTES: usize = 16 * 1024 * 1024;
const MAX_BATCH: u32 = 32;

// Runtime reloads can overlap while private storage stays the same. Serialize
// the read/append/write sequence across plugin generations in this process.
static WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Default, Deserialize, Serialize)]
struct Inbox {
    next_sequence: u64,
    events: Vec<StoredEvent>,
}

#[derive(Deserialize, Serialize)]
struct StoredEvent {
    sequence: u64,
    received_ms: i64,
    event: ExternalEvent,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: u64,
}

fn inbox_key(tenant: &str, account: &str) -> String {
    hex::encode(Sha256::digest(format!("{tenant}/{account}").as_bytes()))
}

async fn read(host: &dyn HostClient, key: &str) -> Result<Inbox> {
    let response = host
        .storage_get(HostStorageGetRequest {
            scope: HostStorageScope::Global,
            visibility: HostStorageVisibility::Private,
            namespace: NAMESPACE.into(),
            key: key.into(),
        })
        .await?;
    response.value.map_or_else(
        || Ok(Inbox::default()),
        |value| {
            serde_json::from_str(&value)
                .map_err(|_| PluginError::internal("chat inbox state is unreadable"))
        },
    )
}

pub(super) async fn append(host: Arc<dyn HostClient>, messages: Vec<ExternalEvent>) -> Result<()> {
    let mut accounts = BTreeMap::<String, Vec<_>>::new();
    for event in messages {
        let identity = &event
            .channel
            .as_ref()
            .ok_or_else(|| PluginError::internal("verified event has no channel identity"))?
            .identity;
        accounts
            .entry(inbox_key(&identity.tenant, &identity.account))
            .or_default()
            .push(event);
    }
    let guard = WRITES.lock().await;
    let callback = current_host_callback_context().unwrap_or_default();
    // Keep the guard through the blocking storage commit, even when the HTTP
    // acknowledgement times out. A replay will encounter the committed IDs.
    tokio::spawn(run_in_isolated_host_callback_context(
        callback,
        async move {
            let _guard = guard;
            for (key, events) in accounts {
                append_account(&*host, key, events).await?;
            }
            Ok(())
        },
    ))
    .await
    .map_err(PluginError::internal)?
}

async fn append_account(
    host: &dyn HostClient,
    key: String,
    messages: Vec<ExternalEvent>,
) -> Result<()> {
    let mut inbox = read(host, &key).await?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    inbox
        .events
        .retain(|item| item.received_ms >= now_ms.saturating_sub(RETENTION_MS));
    let mut ids: HashSet<String> = inbox
        .events
        .iter()
        .map(|item| item.event.id.clone())
        .collect();
    let mut changed = false;
    for event in messages {
        if !ids.insert(event.id.clone()) {
            continue;
        }
        inbox.next_sequence = inbox
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| PluginError::internal("chat inbox sequence exhausted"))?;
        inbox.events.push(StoredEvent {
            sequence: inbox.next_sequence,
            received_ms: now_ms,
            event,
        });
        changed = true;
        if inbox.events.len() > MAX_RECORDS {
            return Err(PluginError::internal(
                "chat inbox has reached its 48-hour capacity",
            ));
        }
    }
    if !changed {
        return Ok(());
    }
    let value =
        serde_json::to_string(&inbox).map_err(|error| PluginError::internal_error(&error))?;
    if value.len() > MAX_INBOX_BYTES {
        return Err(PluginError::internal(
            "chat inbox has reached its byte capacity",
        ));
    }
    host.storage_set(HostStorageSetRequest {
        scope: HostStorageScope::Global,
        visibility: HostStorageVisibility::Private,
        namespace: NAMESPACE.into(),
        key,
        value,
    })
    .await
}

pub(super) async fn poll(
    host: &dyn HostClient,
    connection: &Connection,
    request: &AdapterPollRequest,
) -> Result<AdapterBatch> {
    let cursor: Cursor = if request.cursor.is_null() {
        Cursor::default()
    } else {
        super::decode(request.cursor.clone())?
    };
    let (tenant, account) = connection.identity_parts();
    let inbox = read(host, &inbox_key(tenant, account)).await?;
    let mut after = cursor.after;
    let mut events = Vec::new();
    let limit = request.limit.clamp(1, MAX_BATCH) as usize;
    for item in inbox
        .events
        .into_iter()
        .filter(|item| item.sequence > cursor.after)
    {
        after = item.sequence;
        let event = item.event;
        if event.occurred_at_ms < request.not_before_ms
            || item.received_ms < request.now_ms.saturating_sub(RETENTION_MS)
            || event.channel.as_ref().is_none_or(|channel| {
                !request.channel_kinds.is_empty() && !request.channel_kinds.contains(&channel.kind)
            })
        {
            continue;
        }
        events.push(event);
        if events.len() == limit {
            break;
        }
    }
    Ok(AdapterBatch {
        events,
        cursor: json!({"after": after}),
        retry_after_seconds: None,
    })
}
