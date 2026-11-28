use std::collections::BTreeMap;

use agena_plugin_sdk::{
    HostClient, PluginError, Result,
    host_api::{
        HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
        ImportedPluginAgentState,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const LATEST: &str = "claude.workflow.latest";
pub(super) const MAX_CALLS: usize = 1_000;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default, Deserialize, Serialize)]
struct Journal {
    args: Option<String>,
    calls: Vec<Call>,
}

#[derive(Clone, Copy)]
pub(in crate::agent_plugin) enum ResumeFrom {
    Fresh,
    FreshWithId(uuid::Uuid),
    Latest,
    Run(uuid::Uuid),
}

impl ResumeFrom {
    pub(super) fn from_inputs(resume: bool, run_id: Option<&str>) -> Result<Self> {
        if let Some(id) = run_id {
            return uuid::Uuid::parse_str(id)
                .map(Self::Run)
                .map_err(|_| PluginError::invalid_params("invalid workflow run ID"));
        }
        Ok(if resume { Self::Latest } else { Self::Fresh })
    }
}

#[derive(Deserialize, Serialize)]
struct Call {
    fingerprint: String,
    outcome: Outcome,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Pending { name: String },
    Completed { value: Value },
}

pub(super) enum Replayed {
    Value(Value),
    CompletedAgent(i64),
}

pub(super) struct Checkpoint {
    key: String,
    run_id: uuid::Uuid,
    journal: Journal,
    completed_agents: BTreeMap<usize, i64>,
    next: usize,
}

impl Checkpoint {
    pub(super) async fn load(
        host: &dyn HostClient,
        key: &str,
        args: Option<Value>,
        resume: ResumeFrom,
    ) -> Result<Self> {
        let latest = load_record(host, LATEST, key)
            .await?
            .map(|id| uuid::Uuid::parse_str(&id))
            .transpose()
            .map_err(|_| PluginError::internal("invalid saved workflow run ID"))?;
        let run_id = match resume {
            ResumeFrom::Fresh => uuid::Uuid::new_v4(),
            ResumeFrom::FreshWithId(id) => id,
            ResumeFrom::Latest => latest.ok_or_else(missing_run)?,
            ResumeFrom::Run(id) => id,
        };
        if let Some(latest) = latest.filter(|latest| *latest != run_id) {
            let journal = read_journal(host, key, latest).await?;
            reconcile(host, &journal).await?;
        }
        let mut journal = match resume {
            ResumeFrom::Fresh | ResumeFrom::FreshWithId(_) => Journal::default(),
            _ => read_journal(host, key, run_id).await?,
        };
        let completed_agents = reconcile(host, &journal).await?;
        if let Some(args) = args {
            journal.args = Some(args.to_string());
        }
        if journal
            .args
            .as_ref()
            .is_some_and(|args| args.len() > 64 * 1024)
        {
            return Err(PluginError::invalid_params(
                "workflow arguments exceed 64 KiB",
            ));
        }
        Ok(Self {
            key: key.into(),
            run_id,
            journal,
            completed_agents,
            next: 0,
        })
    }

    pub(super) fn args(&self) -> Option<String> {
        self.journal.args.clone()
    }

    pub(super) fn run_id(&self) -> uuid::Uuid {
        self.run_id
    }

    pub(super) async fn register(&self, host: &dyn HostClient) -> Result<()> {
        self.save(host).await?;
        save_record(
            host,
            LATEST.into(),
            self.key.clone(),
            self.run_id.to_string(),
        )
        .await
    }

    pub(super) fn next(&mut self, fingerprint: &str) -> Result<(usize, Option<Replayed>)> {
        let index = self.next;
        if index == MAX_CALLS {
            return Err(PluginError::invalid_params(
                "a workflow may call agent() at most 1000 times",
            ));
        }
        self.next += 1;
        let replay = self
            .journal
            .calls
            .get(index)
            .filter(|call| call.fingerprint == fingerprint)
            .and_then(|call| match &call.outcome {
                Outcome::Completed { value } => Some(Replayed::Value(value.clone())),
                Outcome::Pending { .. } => self
                    .completed_agents
                    .remove(&index)
                    .map(Replayed::CompletedAgent),
            });
        Ok((index, replay))
    }

    /// Persist the launch name before delegating. On restart the host can
    /// reconcile a launched child even if assignment of its ID was interrupted.
    pub(super) async fn start(
        &mut self,
        host: &dyn HostClient,
        index: usize,
        fingerprint: String,
        name: String,
    ) -> Result<()> {
        self.journal.calls.truncate(index);
        self.completed_agents
            .retain(|previous, _| *previous < index);
        self.journal.calls.push(Call {
            fingerprint,
            outcome: Outcome::Pending { name },
        });
        self.save(host).await
    }

    pub(super) async fn complete(
        &mut self,
        host: &dyn HostClient,
        index: usize,
        value: Value,
    ) -> Result<()> {
        self.journal.calls[index].outcome = Outcome::Completed { value };
        self.save(host).await
    }

    pub(super) async fn finish(&mut self, host: &dyn HostClient) -> Result<()> {
        self.journal.calls.truncate(self.next);
        self.save(host).await
    }

    async fn save(&self, host: &dyn HostClient) -> Result<()> {
        let value = serde_json::to_string(&self.journal)
            .map_err(|error| PluginError::internal_error(&error))?;
        if value.len() > MAX_BYTES {
            return Err(PluginError::internal("workflow checkpoint exceeds 4 MiB"));
        }
        save_record(host, run_namespace(self.run_id), self.key.clone(), value).await
    }
}

fn missing_run() -> PluginError {
    PluginError::invalid_params("there is no saved run of this workflow in this session")
}

fn run_namespace(id: uuid::Uuid) -> String {
    format!("claude.workflow.run.{}", id.simple())
}

async fn read_journal(host: &dyn HostClient, key: &str, id: uuid::Uuid) -> Result<Journal> {
    let saved = load_record(host, &run_namespace(id), key)
        .await?
        .ok_or_else(missing_run)?;
    if saved.len() > MAX_BYTES {
        return Err(PluginError::internal("workflow checkpoint is too large"));
    }
    let journal: Journal = serde_json::from_str(&saved)
        .map_err(|_| PluginError::internal("invalid workflow checkpoint"))?;
    if journal.calls.len() > MAX_CALLS {
        return Err(PluginError::internal(
            "workflow checkpoint has too many calls",
        ));
    }
    Ok(journal)
}

async fn reconcile(host: &dyn HostClient, journal: &Journal) -> Result<BTreeMap<usize, i64>> {
    let mut completed = BTreeMap::new();
    for (index, call) in journal.calls.iter().enumerate() {
        if let Outcome::Pending { name } = &call.outcome {
            match host.find_imported_plugin_agent(name.clone()).await? {
                Some(ImportedPluginAgentState::Active) => {
                    return Err(PluginError::invalid_params(
                        "a child from the previous workflow run is still active",
                    ));
                }
                Some(ImportedPluginAgentState::Completed(child)) => {
                    completed.insert(index, child);
                }
                _ => {}
            }
        }
    }
    Ok(completed)
}

async fn load_record(host: &dyn HostClient, namespace: &str, key: &str) -> Result<Option<String>> {
    Ok(host
        .storage_get(HostStorageGetRequest {
            scope: HostStorageScope::Session,
            visibility: HostStorageVisibility::Private,
            namespace: namespace.into(),
            key: key.into(),
        })
        .await?
        .value)
}

async fn save_record(
    host: &dyn HostClient,
    namespace: String,
    key: String,
    value: String,
) -> Result<()> {
    host.storage_set(HostStorageSetRequest {
        scope: HostStorageScope::Session,
        visibility: HostStorageVisibility::Private,
        namespace,
        key,
        value,
    })
    .await
}
