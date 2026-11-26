use agena_domain::ExternalAgentOptionType;

use super::*;

impl AcpClient {
    pub(in crate::session::manager::external_agent) async fn session_options(
        &mut self,
        cwd: &Path,
        target: Option<&str>,
        directories: &[PathBuf],
        cancel: &CancellationToken,
    ) -> Result<Option<Value>, AppError> {
        if let Some(target) = target {
            match self
                .options_for(cwd, Some(target), directories, cancel)
                .await
            {
                result @ Ok(Some(_)) => return result,
                Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                Err(error) => {
                    tracing::debug!(%error, "could not inspect saved external agent options")
                }
                Ok(None) => {}
            }
        }
        self.options_for(cwd, None, directories, cancel).await
    }

    async fn options_for(
        &mut self,
        cwd: &Path,
        target: Option<&str>,
        directories: &[PathBuf],
        cancel: &CancellationToken,
    ) -> Result<Option<Value>, AppError> {
        let caps = self.initialize(cancel).await?;
        if !caps.options || (target.is_some() && !caps.load && !caps.resume && !caps.lifecycle.fork)
        {
            return Ok(None);
        }
        let mut params = json!({"cwd": cwd});
        if let Some(target) = target {
            params["sessionId"] = json!(target);
        }
        if caps.lifecycle.additional_directories && !directories.is_empty() {
            params["additionalDirectories"] = json!(directories);
        }
        policy::apply_to_session(self.agent, &mut params);
        self.call("session/options", params, cancel).await.map(Some)
    }

    pub(super) async fn apply_initial_config_options(
        &mut self,
        cwd: &Path,
        params: &mut Value,
        context: &SessionContext,
        cancel: &CancellationToken,
    ) -> Result<(), AppError> {
        if context.selected_options.is_empty() {
            return Ok(());
        }
        let options = self
            .options_for(
                cwd,
                params["sessionId"].as_str(),
                &context.additional_directories,
                cancel,
            )
            .await;
        let advertised = match options {
            Ok(Some(response)) => config_options(&response),
            Ok(None) => return Ok(()),
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(error) => {
                tracing::debug!(%error, "could not read initial ACP options; using session setters");
                return Ok(());
            }
        };
        // Use the peer's order so changes such as model selection can alter
        // the choices available to later settings. The setup response is the
        // source of truth for which values were actually applied.
        let entries: Vec<_> = advertised
            .iter()
            .filter_map(|option| {
                let selected = context.selected_options.get(&option.kind)?;
                if option.legacy_mode {
                    return None;
                }
                let (kind, value) = match option.value_type {
                    ExternalAgentOptionType::Select => ("id", json!(selected)),
                    ExternalAgentOptionType::Boolean => {
                        ("boolean", json!(selected.parse::<bool>().ok()?))
                    }
                };
                Some(json!({"configId": option.id, "type": kind, "value": value}))
            })
            .collect();
        if !entries.is_empty() {
            params["configOptions"] = Value::Array(entries);
        }
        Ok(())
    }
}
