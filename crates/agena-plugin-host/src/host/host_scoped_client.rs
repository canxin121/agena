impl ScopedHostClient {
    fn stopped_error(&self) -> PluginError {
        PluginError::from_kind(
            crate::sdk::PluginErrorKind::HostUnavailable,
            format!(
                "plugin callback generation {} for `{}` is stopped or stale",
                self.effect_scope.generation(),
                self.plugin_id
            ),
        )
        .with_plugin(self.plugin_id.clone())
    }

    async fn run_callback<T>(
        &self,
        callback: impl std::future::Future<Output = crate::sdk::Result<T>>,
    ) -> crate::sdk::Result<T> {
        // Admit against the exact generation while publication of a successor
        // is excluded. The lease covers lock waits as well as the host call.
        let _lease = {
            let scopes = super::host_handle::recover_read(
                &self.handle.effect_scopes,
                "plugin effect scope registry",
            );
            if !scopes
                .get(&self.plugin_key)
                .is_some_and(|scope| std::sync::Arc::ptr_eq(scope, &self.effect_scope))
            {
                return Err(self.stopped_error());
            }
            self.effect_scope
                .lease()
                .map_err(|_| self.stopped_error())?
        };
        // Local registry methods read the task-local session just like runtime
        // calls do. Validate and attribute that context before either path can
        // observe it, including calls that never await the inner HostClient.
        let context = self.context()?;
        let stop = self.effect_scope.cancellation_token();
        self.handle
            .run_in_callback_effect_scope(
                self.effect_scope.clone(),
                host_api::run_in_isolated_host_callback_context(context, async {
                    // Dropping the host future on shutdown also releases callbacks
                    // that themselves requested a reload or scope disposal.
                    self.handle
                        .run_in_callback_completion(&self.context()?, async {
                            tokio::select! {
                                biased;
                                _ = stop.cancelled() => Err(self.stopped_error()),
                                result = callback => result,
                            }
                        })
                        .await
                }),
            )
            .await
    }

    fn context(&self) -> crate::sdk::Result<HostCallbackContext> {
        let mut context = host_api::current_host_callback_context().unwrap_or_default();
        context.plugin_id = Some(self.plugin_id.clone());
        self.handle
            .validated_callback_context(Some(self.plugin_id.clone()), Some(context))
    }
}

#[async_trait::async_trait]
impl HostClient for ScopedHostClient {
    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value) {
        let _ = self
            .run_callback(async {
                let context = self.context()?;
                let inner = self.handle.inner.read().await.clone();
                host_api::run_in_host_callback_context(context, inner.log(level, message, fields))
                    .await;
                Ok(())
            })
            .await;
    }

    async fn publish_event(&self, env: EventEnvelope) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.publish_event(env)).await
        })
        .await
    }

    async fn publish_activity(&self, activity: BackgroundActivity) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.publish_activity(activity),
            )
            .await
        })
        .await
    }

    async fn register_browser_controller(
        &self,
        controller: std::sync::Arc<dyn crate::sdk::browser::BrowserController>,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.register_browser_controller(controller),
            )
            .await
        })
        .await
    }

    async fn subscribe_events(&self, filter: EventFilter) -> crate::sdk::Result<EventSubscription> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.subscribe_events(filter))
                .await
        })
        .await
    }

    async fn unsubscribe_events(&self, subscription_id: String) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.unsubscribe_events(subscription_id),
            )
            .await
        })
        .await
    }

    async fn read_config(&self, path: Option<String>) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.read_config(path)).await
        })
        .await
    }

    async fn check_path_permission(
        &self,
        request: crate::sdk::host_api::PathPermissionQuery,
    ) -> crate::sdk::Result<crate::sdk::host_api::PermissionQuery> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.check_path_permission(request),
            )
            .await
        })
        .await
    }

    async fn check_network_permission(
        &self,
        target: String,
    ) -> crate::sdk::Result<crate::sdk::host_api::PermissionQuery> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.check_network_permission(target),
            )
            .await
        })
        .await
    }

    async fn check_resolved_network_permission(
        &self,
        original: String,
        resolved: String,
    ) -> crate::sdk::Result<crate::sdk::host_api::PermissionQuery> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.check_resolved_network_permission(original, resolved),
            )
            .await
        })
        .await
    }

    async fn require_path_permission(
        &self,
        request: crate::sdk::host_api::PathPermissionQuery,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                crate::sdk::permission_wait::waiting(inner.require_path_permission(request)),
            )
            .await
        })
        .await
    }

    async fn require_network_permission(&self, target: String) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                crate::sdk::permission_wait::waiting(inner.require_network_permission(target)),
            )
            .await
        })
        .await
    }

    async fn mcp_session_roots(&self) -> crate::sdk::Result<Option<Vec<std::path::PathBuf>>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.mcp_session_roots()).await
        })
        .await
    }

    async fn require_pinned_http_permission(&self, target: String) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                crate::sdk::permission_wait::waiting(inner.require_pinned_http_permission(target)),
            )
            .await
        })
        .await
    }

    async fn request_config_reload(&self) -> crate::sdk::Result<HostConfigReloadRequestResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.request_config_reload())
                .await
        })
        .await
    }

    async fn config_reload_status(
        &self,
        request: HostConfigReloadStatusRequest,
    ) -> crate::sdk::Result<HostConfigReloadStatusResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.config_reload_status(request),
            )
            .await
        })
        .await
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::sdk::Result<ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.invoke_tool(tool, input))
                .await
        })
        .await
    }

    async fn invoke_service(
        &self,
        req: crate::sdk::PluginServiceInvokeInput,
    ) -> crate::sdk::Result<crate::sdk::PluginServiceInvokeOutput> {
        self.run_callback(async {
            self.handle
                .invoke_service_for_plugin(&self.plugin_id, req, Some(self.context()?))
                .await
        })
        .await
    }

    async fn ask_user(&self, req: AskUserRequest) -> crate::sdk::Result<AskUserResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.ask_user(req)).await
        })
        .await
    }

    async fn companion_context(&self) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.companion_context()).await
        })
        .await
    }
    async fn companion_operation(
        &self,
        request: crate::sdk::host_api::CompanionRequest,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.companion_operation(request),
            )
            .await
        })
        .await
    }

    async fn manage_integrations(
        &self,
        request: crate::sdk::host_api::IntegrationRequest,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.manage_integrations(request),
            )
            .await
        })
        .await
    }

    async fn bot_context(&self) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.bot_context()).await
        })
        .await
    }
    async fn bot_operation(
        &self,
        request: crate::sdk::host_api::BotRequest,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.bot_operation(request))
                .await
        })
        .await
    }
    async fn bot_profile_operation(
        &self,
        request: crate::sdk::host_api::BotProfileRequest,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.bot_profile_operation(request),
            )
            .await
        })
        .await
    }
    async fn spawn_agent(&self, req: AgentSpawnRequest) -> crate::sdk::Result<AgentHandle> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.spawn_agent(req)).await
        })
        .await
    }

    async fn message_agent(
        &self,
        req: AgentMessageRequest,
    ) -> crate::sdk::Result<AgentMessageReceipt> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.message_agent(req)).await
        })
        .await
    }

    async fn wait_agent(&self, req: AgentWaitRequest) -> crate::sdk::Result<AgentWaitResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.wait_agent(req)).await
        })
        .await
    }

    async fn start_cordis_workflow(
        &self,
        start: host_api::CordisWorkflowStart,
        run: host_api::CordisWorkflowRunner,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<host_api::CordisWorkflowStarted> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.start_cordis_workflow(start, run, cancellation),
            )
            .await
        })
        .await
    }

    async fn start_cordis_job(
        &self,
        start: host_api::CordisJobStart,
        produce: host_api::CordisJobStarter,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<String> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.start_cordis_job(start, produce, cancellation),
            )
            .await
        })
        .await
    }

    async fn open_cordis_job_registry(
        &self,
        lifetime: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<std::sync::Arc<dyn host_api::CordisJobRegistry>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.open_cordis_job_registry(lifetime),
            ).await
        }).await
    }

    async fn cordis_workflow_job(
        &self,
        request: host_api::CordisWorkflowJobRequest,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.cordis_workflow_job(request, cancellation),
            )
            .await
        })
        .await
    }

    async fn wait_imported_plugin_agent(
        &self,
        child: i64,
        timeout_ms: u64,
    ) -> crate::sdk::Result<crate::sdk::ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.wait_imported_plugin_agent(child, timeout_ms),
            )
            .await
        })
        .await
    }

    async fn start_cordis_subagent(
        &self,
        start: host_api::CordisSubagentStart,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<host_api::CordisSubagentStarted> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.start_cordis_subagent(start, cancellation),
            )
            .await
        })
        .await
    }

    async fn wait_for_cordis_subagent(
        &self,
        wait: host_api::CordisSubagentWait,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.wait_for_cordis_subagent(wait),
            )
            .await
        })
        .await
    }

    async fn cordis_subagent_control(
        &self,
        request: host_api::CordisSubagentControl,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.cordis_subagent_control(request, cancel),
            )
            .await
        })
        .await
    }

    async fn cordis_llm_complete(
        &self,
        request: host_api::CordisLlmRequest,
        cancel: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<host_api::CordisLlmResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.cordis_llm_complete(request, cancel),
            )
            .await
        })
        .await
    }

    async fn evaluate_imported_plugin_prompt(&self, prompt: String) -> crate::sdk::Result<String> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.evaluate_imported_plugin_prompt(prompt),
            )
            .await
        })
        .await
    }

    async fn find_imported_plugin_agent(
        &self,
        name: String,
    ) -> crate::sdk::Result<Option<host_api::ImportedPluginAgentState>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.find_imported_plugin_agent(name),
            )
            .await
        })
        .await
    }

    async fn evaluate_imported_plugin_agent(&self, prompt: String) -> crate::sdk::Result<String> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.evaluate_imported_plugin_agent(prompt),
            )
            .await
        })
        .await
    }

    async fn bind_imported_plugin_background(
        &self,
    ) -> crate::sdk::Result<host_api::ImportedPluginBackground> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.bind_imported_plugin_background(),
            )
            .await
        })
        .await
    }

    async fn submit_imported_plugin_prompt(
        &self,
        request: host_api::ImportedPluginPromptRequest,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            inner.submit_imported_plugin_prompt(request).await
        })
        .await
    }

    async fn imported_mod_tool(
        &self,
        request: host_api::ImportedModToolRequest,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            inner.imported_mod_tool(request).await
        })
        .await
    }

    async fn configure_imported_plugin_files(
        &self,
        paths: Option<Vec<std::path::PathBuf>>,
        _callback_generation: Option<u64>,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.configure_imported_plugin_files(paths, Some(self.effect_scope.generation())),
            )
            .await
        })
        .await
    }

    async fn imported_hook_environment(
        &self,
        _callback_generation: Option<u64>,
    ) -> crate::sdk::Result<Option<host_api::ImportedHookEnvironment>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.imported_hook_environment(Some(self.effect_scope.generation())),
            )
            .await
        })
        .await
    }

    async fn imported_hook_transcripts(
        &self,
        format: host_api::HookTranscriptFormat,
        scope: host_api::HookTranscriptScope,
        _callback_generation: Option<u64>,
    ) -> crate::sdk::Result<host_api::ImportedHookTranscripts> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.imported_hook_transcripts(
                    format,
                    scope,
                    Some(self.effect_scope.generation()),
                ),
            )
            .await
        })
        .await
    }

    async fn imported_claude_stop_work(
        &self,
        parent: bool,
        _callback_generation: Option<u64>,
    ) -> crate::sdk::Result<Option<crate::sdk::ImportedClaudeStopWork>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.imported_claude_stop_work(parent, Some(self.effect_scope.generation())),
            )
            .await
        })
        .await
    }

    async fn visit_ending_session_history(
        &self,
        visit: &mut (dyn FnMut(host_api::SessionHistoryEvent) + Send),
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.visit_ending_session_history(visit),
            )
            .await
        })
        .await
    }

    async fn imported_hook_profile(&self) -> crate::sdk::Result<Option<String>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.imported_hook_profile())
                .await
        })
        .await
    }

    async fn check_imported_task(
        &self,
        input: crate::sdk::ImportedTaskBeforeInput,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.check_imported_task(input),
            )
            .await
        })
        .await
    }

    async fn imported_mcp_server_identity(
        &self,
        id: &str,
    ) -> crate::sdk::Result<Option<host_api::ImportedMcpServerIdentity>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.imported_mcp_server_identity(id),
            )
            .await
        })
        .await
    }

    async fn call_imported_plugin_mcp(
        &self,
        server: String,
        tool: String,
        arguments: serde_json::Value,
    ) -> crate::sdk::Result<crate::sdk::ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.call_imported_plugin_mcp(server, tool, arguments),
            )
            .await
        })
        .await
    }

    async fn followup_agent(&self, req: AgentMessageRequest) -> crate::sdk::Result<AgentHandle> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.followup_agent(req)).await
        })
        .await
    }

    async fn read_imported_plugin_mcp(
        &self,
        server: String,
        uri: String,
    ) -> crate::sdk::Result<crate::sdk::ToolInvokeOutput> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.read_imported_plugin_mcp(server, uri),
            )
            .await
        })
        .await
    }

    async fn deepseek_mcp_manager(
        &self,
        request: host_api::DeepSeekMcpManagerRequest,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.deepseek_mcp_manager(request, cancellation),
            )
            .await
        })
        .await
    }

    async fn observe_deepseek_mcp_manager(
        &self,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<tokio::sync::watch::Receiver<serde_json::Value>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.observe_deepseek_mcp_manager(cancellation),
            )
            .await
        })
        .await
    }

    async fn imported_model_skills(&self) -> crate::sdk::Result<Vec<crate::AgentPluginSkill>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            inner.imported_model_skills().await
        })
        .await
    }

    async fn prepare_managed_mcp_http(&self, server: String) -> crate::sdk::Result<bool> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.prepare_managed_mcp_http(server),
            )
            .await
        })
        .await
    }

    async fn run_imported_skill_shell(
        &self,
        request: crate::sdk::ImportedSkillShellRequest,
    ) -> crate::sdk::Result<crate::sdk::ImportedSkillShellResult> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.run_imported_skill_shell(request),
            )
            .await
        })
        .await
    }

    async fn render_imported_skill(
        &self,
        path: &std::path::Path,
        document: &str,
        arguments: &str,
    ) -> crate::sdk::Result<Option<crate::sdk::ImportedSkillRender>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.render_imported_skill(path, document, arguments),
            )
            .await
        })
        .await
    }

    async fn cancel_agent(
        &self,
        req: AgentTargetRequest,
    ) -> crate::sdk::Result<AgentMessageReceipt> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.cancel_agent(req)).await
        })
        .await
    }

    async fn list_tools(&self) -> crate::sdk::Result<Vec<ToolDescriptor>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.list_tools()).await
        })
        .await
    }

    async fn get_context_status(
        &self,
        req: HostContextStatusRequest,
    ) -> crate::sdk::Result<HostContextStatusResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.get_context_status(req))
                .await
        })
        .await
    }

    async fn get_session(
        &self,
        req: crate::sdk::host_api::HostGetSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostGetSessionResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.get_session(req)).await
        })
        .await
    }

    async fn rename_session(
        &self,
        req: crate::sdk::host_api::HostRenameSessionRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostRenameSessionResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.rename_session(req)).await
        })
        .await
    }

    async fn session_op(
        &self,
        op: crate::sdk::host_api::SessionOp,
    ) -> crate::sdk::Result<crate::sdk::host_api::SessionOpResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.session_op(op)).await
        })
        .await
    }

    async fn session_stream(
        &self,
        req: crate::sdk::host_api::HostSessionStreamRequest,
    ) -> crate::sdk::Result<EventSubscription> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.session_stream(req)).await
        })
        .await
    }

    async fn workspace_query(
        &self,
        req: crate::sdk::host_api::HostWorkspaceQueryRequest,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostWorkspaceQueryResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.workspace_query(req))
                .await
        })
        .await
    }

    async fn builtin_provider_connection(
        &self,
    ) -> crate::sdk::Result<crate::sdk::host_api::BuiltinProviderConnection> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.builtin_provider_connection(),
            )
            .await
        })
        .await
    }

    async fn builtin_provider_models(&self) -> crate::sdk::Result<serde_json::Value> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.builtin_provider_models())
                .await
        })
        .await
    }

    async fn image_execute(
        &self,
        req: HostImageExecuteRequest,
    ) -> crate::sdk::Result<HostImageExecuteResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.image_execute(req)).await
        })
        .await
    }

    async fn register_command(
        &self,
        req: HostCommandRegisterRequest,
    ) -> crate::sdk::Result<HostCommandMutationResponse> {
        self.run_callback(async {
            self.handle
                .command_upsert_for_plugin(&self.plugin_id, req.command)
        })
        .await
    }

    async fn update_command(
        &self,
        req: HostCommandUpdateRequest,
    ) -> crate::sdk::Result<HostCommandMutationResponse> {
        self.run_callback(async {
            self.handle
                .command_upsert_for_plugin(&self.plugin_id, req.command)
        })
        .await
    }

    async fn remove_command(
        &self,
        req: HostCommandRemoveRequest,
    ) -> crate::sdk::Result<HostCommandMutationResponse> {
        self.run_callback(async {
            self.handle
                .command_remove_for_plugin(&self.plugin_id, &req.id)
        })
        .await
    }

    async fn register_tool(
        &self,
        req: HostToolRegisterRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.run_callback(async {
            self.handle
                .tool_upsert_for_plugin(&self.plugin_id, req.tool)
        })
        .await
    }

    async fn update_tool(
        &self,
        req: HostToolUpdateRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.run_callback(async {
            self.handle
                .tool_upsert_for_plugin(&self.plugin_id, req.tool)
        })
        .await
    }

    async fn remove_tool(
        &self,
        req: HostToolRemoveRequest,
    ) -> crate::sdk::Result<HostToolMutationResponse> {
        self.run_callback(async {
            self.handle
                .tool_remove_for_plugin(&self.plugin_id, &req.name, req.by_model_name)
        })
        .await
    }

    async fn list_registered_commands(
        &self,
    ) -> crate::sdk::Result<HostRegisteredCommandListResponse> {
        self.run_callback(async { self.handle.registered_command_list_response() })
            .await
    }

    async fn list_registered_tools(&self) -> crate::sdk::Result<HostRegisteredToolListResponse> {
        self.run_callback(async { self.handle.registered_tool_list_response() })
            .await
    }

    async fn list_plugins(&self) -> crate::sdk::Result<crate::sdk::HostPluginListResponse> {
        self.run_callback(async { Ok(self.handle.plugin_list_response()) })
            .await
    }

    async fn storage_get(
        &self,
        req: HostStorageGetRequest,
    ) -> crate::sdk::Result<HostStorageGetResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_get(req)).await
        })
        .await
    }

    async fn storage_set(&self, req: HostStorageSetRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_set(req)).await
        })
        .await
    }

    async fn storage_compare_exchange(
        &self,
        req: HostStorageSetRequest,
        expected: Option<String>,
    ) -> crate::sdk::Result<bool> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.storage_compare_exchange(req, expected),
            )
            .await
        })
        .await
    }

    async fn storage_delete(&self, req: HostStorageDeleteRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_delete(req)).await
        })
        .await
    }

    async fn imported_plugin_storage_directory(&self) -> crate::sdk::Result<std::path::PathBuf> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.imported_plugin_storage_directory(),
            )
            .await
        })
        .await
    }

    async fn storage_list(
        &self,
        req: HostStorageListRequest,
    ) -> crate::sdk::Result<HostStorageListResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.storage_list(req)).await
        })
        .await
    }

    async fn secret_get(
        &self,
        req: HostSecretGetRequest,
    ) -> crate::sdk::Result<HostSecretGetResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_get(req)).await
        })
        .await
    }

    async fn secret_set(&self, req: HostSecretSetRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_set(req)).await
        })
        .await
    }

    async fn secret_delete(&self, req: HostSecretDeleteRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_delete(req)).await
        })
        .await
    }

    async fn secret_list(&self) -> crate::sdk::Result<HostSecretListResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.secret_list()).await
        })
        .await
    }

    async fn plugin_status_list(&self) -> crate::sdk::Result<HostPluginStatusListResponse> {
        self.run_callback(async { Ok(self.handle.plugin_status_list_response()) })
            .await
    }

    async fn plugin_status_get(
        &self,
        req: HostPluginStatusGetRequest,
    ) -> crate::sdk::Result<HostPluginStatusGetResponse> {
        self.run_callback(async { Ok(self.handle.plugin_status_get_response(&req.plugin_id)) })
            .await
    }

    async fn hook_list(&self) -> crate::sdk::Result<HostHookListResponse> {
        self.run_callback(async { Ok(self.handle.hook_list_response().await) })
            .await
    }

    async fn lsp_list_servers(&self) -> crate::sdk::Result<HostLspListServersResponse> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(self.context()?, inner.lsp_list_servers()).await
        })
        .await
    }

    async fn require_mcp_server_permission(
        &self,
        spec: crate::sdk::host_api::HostMcpServerSpec,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                crate::sdk::permission_wait::waiting(inner.require_mcp_server_permission(spec)),
            )
            .await
        })
        .await
    }

    async fn mcp_elicitation(
        &self,
        server: &str,
        request: crate::sdk::McpElicitation,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<crate::sdk::McpElicitationReply> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.mcp_elicitation(server, request, cancellation),
            )
            .await
        })
        .await
    }

    async fn prepare_process_sandbox(
        &self,
        resources: crate::sdk::host_api::HostProcessResources,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostPreparedProcessSandbox> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                crate::sdk::permission_wait::waiting(inner.prepare_process_sandbox(resources)),
            )
            .await
        })
        .await
    }

    async fn mcp_sampling(
        &self,
        server: &str,
        request: agena_domain::mcp_sampling::McpSamplingRequest,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<Option<agena_domain::mcp_sampling::McpSamplingReply>> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.mcp_sampling(server, request, cancellation),
            )
            .await
        })
        .await
    }

    async fn call_mcp_app_view_tool(
        &self,
        target: uuid::Uuid,
        arguments: serde_json::Map<String, serde_json::Value>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> crate::sdk::Result<agena_domain::mcp_app_tools::McpAppToolResult> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                inner.call_mcp_app_view_tool(target, arguments, cancellation),
            )
            .await
        })
        .await
    }

    async fn prepare_worker_sandbox(
        &self,
        resources: crate::sdk::host_api::HostProcessResources,
    ) -> crate::sdk::Result<crate::sdk::host_api::HostPreparedProcessSandbox> {
        self.run_callback(async {
            let inner = self.handle.inner.read().await.clone();
            host_api::run_in_host_callback_context(
                self.context()?,
                crate::sdk::permission_wait::waiting(inner.prepare_worker_sandbox(resources)),
            )
            .await
        })
        .await
    }

    async fn display_contribute(
        &self,
        req: HostDisplayContributeRequest,
    ) -> crate::sdk::Result<()> {
        self.run_callback(async { self.handle.display_contribute(&self.plugin_id, req) })
            .await
    }

    async fn display_remove(
        &self,
        req: HostDisplayRemoveRequest,
    ) -> crate::sdk::Result<HostDisplayRemoveResponse> {
        self.run_callback(async {
            let removed = self
                .handle
                .display_remove(&self.plugin_id, &req.contribution_id);
            Ok(HostDisplayRemoveResponse { removed })
        })
        .await
    }

    async fn notify(&self, req: PluginNotifyRequest) -> crate::sdk::Result<()> {
        self.run_callback(async {
            self.handle.push_host_notification(&self.plugin_id, req);
            Ok(())
        })
        .await
    }

    async fn ui_theme_register(&self, req: HostThemeRegisterRequest) -> crate::sdk::Result<()> {
        self.run_callback(async { self.handle.theme_register(&self.plugin_id, req) })
            .await
    }

    async fn ui_theme_list(&self) -> crate::sdk::Result<HostThemeListResponse> {
        self.run_callback(async { Ok(self.handle.theme_list_response()) })
            .await
    }

    async fn ui_theme_remove(
        &self,
        req: HostThemeRemoveRequest,
    ) -> crate::sdk::Result<HostThemeRemoveResponse> {
        self.run_callback(async {
            let removed = self.handle.theme_remove(&self.plugin_id, &req.id);
            Ok(HostThemeRemoveResponse { removed })
        })
        .await
    }
}
use super::{
    AgentHandle, AgentMessageReceipt, AgentMessageRequest, AgentSpawnRequest, AgentTargetRequest,
    AgentWaitRequest, AgentWaitResponse, AskUserRequest, AskUserResponse, EventEnvelope,
    EventFilter, EventSubscription, HostCallbackContext, HostClient, HostCommandMutationResponse,
    HostCommandRegisterRequest, HostCommandRemoveRequest, HostCommandUpdateRequest,
    HostConfigReloadRequestResponse, HostConfigReloadStatusRequest, HostConfigReloadStatusResponse,
    HostContextStatusRequest, HostContextStatusResponse, HostDisplayContributeRequest,
    HostDisplayRemoveRequest, HostDisplayRemoveResponse, HostHookListResponse,
    HostImageExecuteRequest, HostImageExecuteResponse, HostLspListServersResponse,
    HostPluginStatusGetRequest, HostPluginStatusGetResponse, HostPluginStatusListResponse,
    HostRegisteredCommandListResponse, HostRegisteredToolListResponse, HostSecretDeleteRequest,
    HostSecretGetRequest, HostSecretGetResponse, HostSecretListResponse, HostSecretSetRequest,
    HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
    HostStorageListRequest, HostStorageListResponse, HostStorageSetRequest, HostThemeListResponse,
    HostThemeRegisterRequest, HostThemeRemoveRequest, HostThemeRemoveResponse,
    HostToolMutationResponse, HostToolRegisterRequest, HostToolRemoveRequest,
    HostToolUpdateRequest, LogLevel, PluginError, PluginNotifyRequest, ScopedHostClient,
    ToolDescriptor, ToolInvokeOutput,
    host_api::{self, BackgroundActivity},
};
