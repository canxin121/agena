//! Process and transport faults stay within the owning case and request.

use super::{FaultDriver, KERNEL_BEFORE_COMMIT};
use crate::{StepHandle, TestCase};
use anyhow::{Context, Result, ensure};

impl FaultDriver {
    pub(crate) fn with_case(mut self, case: TestCase) -> Self {
        self.owner = Some(case);
        self
    }

    fn owned_case(&self) -> Result<&TestCase> {
        self.owner
            .as_ref()
            .context("process and model faults require a case.faults() driver")
    }

    /// Kill only this fixture's owned product process tree, preserving its
    /// isolated data directory for a later restart.
    pub async fn kill(&self) -> Result<()> {
        self.owned_case()?.crash().await
    }

    /// Restart the same isolated product and reconnect to its new identity.
    /// Fault state is preserved; clear a consumed fault explicitly when needed.
    pub async fn restart(&self) -> Result<()> {
        self.owned_case()?.restart().await
    }

    /// Cut power to the owned process at an observed, held SQLite transaction.
    /// This is an abrupt process kill, not a host shutdown or a generic SQL error.
    pub async fn power_cut(&self) -> Result<()> {
        let _gate = self.gate.lock().await;
        let state = self.state().await?;
        ensure!(
            state.point == KERNEL_BEFORE_COMMIT
                && state.effect == "hold"
                && state.remaining == 0
                && state.hits > 0
                && !state.released,
            "power_cut requires an observed, unreleased kernel.before_commit Hold gate"
        );
        self.kill().await
    }

    /// Abort the socket for exactly one matched local-model request. The step
    /// must belong to this case; unrelated requests and completed steps are safe.
    pub async fn disconnect_model(&self, step: &StepHandle) -> Result<()> {
        let case = self.owned_case()?;
        ensure!(
            step.belongs_to(&case.model()),
            "model step belongs to another test case"
        );
        step.disconnect().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Reply, RequestMatch, ScriptedModel, TestAgena};
    use serde_json::json;

    #[tokio::test]
    async fn model_fault_cannot_disconnect_a_request_owned_by_another_fixture() -> Result<()> {
        let case = TestAgena::builder(std::env::current_exe()?)
            .kernel_faults()
            .prepare()
            .await?;
        let foreign = ScriptedModel::start().await?;
        let step = foreign
            .handle()
            .expect(RequestMatch::user_text("foreign-case"))
            .reply(Reply::text("still-owned-by-foreign-case"));
        let error = case
            .faults()?
            .disconnect_model(&step)
            .await
            .expect_err("a fault driver accepted another case's model step");
        ensure!(error.to_string().contains("another test case"));
        let body: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/responses", foreign.base_url()))
            .json(&json!({
                "model":"fake-model", "stream":false,
                "input":[{"role":"user","content":"foreign-case"}]
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        ensure!(body["output"][0]["content"][0]["text"] == "still-owned-by-foreign-case");
        step.finished().await?;
        foreign.shutdown().await?;
        case.finish().await
    }
}
