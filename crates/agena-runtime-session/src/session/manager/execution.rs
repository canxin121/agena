use super::super::kernel::{KernelRoundOutcome, KernelRoundRequest, KernelRoundRun};
use super::*;
use agena_domain::{PermissionAction, PermissionDecision, Role, ToolInvocation};
use futures_util::{FutureExt, StreamExt, stream};
use serde_json::{Value, json};

impl SessionManager {
    pub(super) fn reserve_execution(&self) -> Result<OwnedSemaphorePermit, AppError> {
        if self.shutdown.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        self.admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Config("the execution queue is full".into()))
    }
    pub(super) fn supervise(
        &self,
        execution: ExecutionView,
        options: SessionRunOptions,
        compact: bool,
        permit: OwnedSemaphorePermit,
    ) -> Arc<RunControl> {
        self.supervise_with_budget(execution, options, compact, permit, None)
    }
    pub(super) fn supervise_with_budget(
        &self,
        execution: ExecutionView,
        options: SessionRunOptions,
        compact: bool,
        permit: OwnedSemaphorePermit,
        budget: Option<AgentBudget>,
    ) -> Arc<RunControl> {
        let state = self.state.load_full();
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(run) = active.get(&execution.id) {
            return run.clone();
        }
        let run = Arc::new(RunControl {
            session: execution.session_id,
            execution: execution.id,
            model: options.model.clone(),
            state: state.clone(),
            cancel: self.shutdown.child_token(),
            phase: StdMutex::new(agena_domain::ExecutionPhase::Starting),
            done: Notify::new(),
            budget,
            failure: Default::default(),
            settled: std::sync::atomic::AtomicBool::new(false),
        });
        active.insert(execution.id, run.clone());
        let dispatch = self.session_dispatch(run.session);
        let predecessor = {
            let mut tail = dispatch.tail.lock().unwrap_or_else(|p| p.into_inner());
            let predecessor = tail.upgrade();
            *tail = Arc::downgrade(&run);
            predecessor
        };
        drop(active);
        let manager = self.clone();
        let result = run.clone();
        tokio::spawn(async move {
            let _permit = permit;
            // Register the predecessor synchronously, before spawning. Tokio
            // task polling order must not decide which queued message starts.
            if let Some(predecessor) = predecessor {
                predecessor.wait_settled().await;
            }
            let _session = dispatch.execution.lock().await;
            let result = if run.cancel.is_cancelled() {
                Err(AppError::Cancelled)
            } else {
                match std::panic::AssertUnwindSafe(manager.drive_execution(
                    run.clone(),
                    options,
                    compact,
                    state,
                ))
                .catch_unwind()
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        run.cancel.cancel();
                        let command = RequestId::new();
                        let _ = persist(|| {
                            manager
                                .kernel()
                                .sessions
                                .cancel(command, run.session, run.execution)
                        })
                        .await;
                        Err(AppError::Internal(
                            "execution supervisor caught a panic; the attempt has been fenced"
                                .into(),
                        ))
                    }
                }
            };
            if let Err(error) = &result {
                *run.failure.lock().unwrap_or_else(|p| p.into_inner()) = Some(error.failure());
                tracing::error!(session_id=run.session.get(),execution_id=%run.execution,%error,"supervised execution settled with an error");
            }
            manager
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&run.execution);
            run.settled
                .store(true, std::sync::atomic::Ordering::Release);
            run.done.notify_waiters();
            manager.schedule_title(run.session);
        });
        result
    }
    pub(super) async fn submit(
        &self,
        request: crate::SessionUserRunRequest,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        let permit = self.reserve_execution()?;
        let manager = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = async {
                let session = session_id(request.run.session_id)?;
                let dispatch = manager.session_dispatch(session);
                // Preparation, commit and supervisor registration form one
                // admission sequence, independently of the running provider.
                let _submission = dispatch.submission.lock().await;
                manager.restore_queued_executions(session).await?;
                let mut options = request.run.options;
                manager.prepare_options(session, &mut options).await?;
                let send_id = request
                    .idempotency_key
                    .as_deref()
                    .map(|key| request_key(&format!("send:{}", session.get()), key))
                    .unwrap_or_default();
                let state = manager.state.load_full();
                let document = manager
                    .submit_hook(session, request.document, &state)
                    .await?;
                let (document, blobs) = manager
                    .prepare_user_media(session, &options.model, document, send_id)
                    .await?;
                manager
                    .update_session_selection(session.get(), options.clone())
                    .await?;
                let receipt = persist(|| {
                    manager.kernel().sessions.send_with_blobs(
                        send_id,
                        session,
                        document.clone(),
                        &blobs,
                    )
                })
                .await
                .map_err(kernel_error)?;
                let result = execution_outcome(&receipt.value);
                manager.supervise(receipt.value, options, false, permit);
                manager.schedule_title(session);
                Ok(result)
            }
            .await;
            let _ = tx.send(result);
        });
        rx.await
            .map_err(|_| AppError::Internal("submission supervisor stopped before commit".into()))?
    }
    async fn submit_hook(
        &self,
        session: SessionId,
        mut document: ComposerDocument,
        state: &ExecutionState,
    ) -> Result<ComposerDocument, AppError> {
        let original = document
            .0
            .iter()
            .filter_map(|node| match node {
                ComposerNode::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if original.is_empty() {
            return Ok(document);
        }
        let updated = state
            .executor
            .plugin_manager()
            .dispatch_user_prompt_submit_cancellable(
                agena_plugin_host::UserPromptSubmitInput {
                    session_id: session.get(),
                    prompt: original.clone(),
                },
                Some(self.shutdown.child_token()),
            )
            .await
            .map_err(|error| {
                AppError::Internal(format!(
                    "prompt blocked by plugin: {}",
                    error.diagnostic_message()
                ))
            })?;
        if updated.prompt != original {
            if updated.prompt.len() > 256 * 1024 {
                return Err(AppError::Config(
                    "rewritten prompt exceeds the input byte budget".into(),
                ));
            }
            let mut placed = false;
            document.0.retain_mut(|node| {
                if let ComposerNode::Text { text } = node {
                    if placed {
                        return false;
                    }
                    *text = updated.prompt.clone();
                    placed = true;
                }
                true
            });
            if !placed {
                document.0.push(ComposerNode::Text {
                    text: updated.prompt,
                });
            }
        }
        Ok(document)
    }

    pub async fn continue_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<Session, AppError> {
        let id = request.session_id;
        self.start_continue_session(request).await?;
        self.get_session(id).await
    }
    pub async fn start_continue_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        self.start_existing(request, false).await
    }
    pub async fn compact_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<Session, AppError> {
        let session = request.session_id;
        self.start_compact_session(request).await?;
        self.wait_session_idle(session).await;
        self.get_session(session).await
    }
    pub async fn start_compact_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        self.start_existing(request, true).await
    }
    async fn start_existing(
        &self,
        request: SessionExecutionRequest,
        compact: bool,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        let permit = self.reserve_execution()?;
        let manager = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = async {
                let session = session_id(request.session_id)?;
                let dispatch = manager.session_dispatch(session);
                let _submission = dispatch.submission.lock().await;
                manager.restore_queued_executions(session).await?;
                let mut options = request.options;
                manager.prepare_options(session, &mut options).await?;
                let message = manager
                    .latest_origin_message(session)
                    .await?
                    .ok_or_else(|| {
                        AppError::Config("this session has no owned message to continue".into())
                    })?;
                let command = RequestId::new();
                let execution = persist(|| {
                    manager
                        .kernel()
                        .sessions
                        .continue_execution(command, session, message)
                })
                .await
                .map_err(kernel_error)?
                .value;
                let outcome = execution_outcome(&execution);
                manager.supervise(execution, options, compact, permit);
                Ok(outcome)
            }
            .await;
            let _ = tx.send(result);
        });
        rx.await.map_err(|_| {
            AppError::Internal("continuation supervisor stopped before commit".into())
        })?
    }
    async fn drive_execution(
        &self,
        control: Arc<RunControl>,
        mut options: SessionRunOptions,
        compact: bool,
        state: Arc<ExecutionState>,
    ) -> Result<(), AppError> {
        let claim = RequestId::new();
        let attempt = match persist(|| {
            self.kernel()
                .executor
                .claim_next(claim, control.session, control.execution)
        })
        .await
        {
            Ok(receipt) => receipt.value,
            // Pausing releases in-memory supervision/admission, but leaves the
            // durable input queued. Explicit resume or recovery registers it
            // again; a paused queue never turns into failed user turns.
            Err(KernelError::QueuePaused | KernelError::QueueOutOfOrder) => return Ok(()),
            Err(error) => return Err(kernel_error(error)),
        };
        let execution = persist(|| {
            self.kernel()
                .reader
                .execution(control.session, control.execution)
        })
        .await
        .map_err(kernel_error)?;
        let message = execution.message_id;
        let mut completed = false;
        // Provider failures already carry a structured failure on their round.
        // Other stops must persist their own notice below, because the round
        // they follow succeeded.
        let mut structured_round_failure = false;
        let result=async {
            if compact {self.perform_compaction(&attempt,&control,&options,&state,None).await?;return Ok(());}
            let mut usage=agena_provider::CompletionUsage::default();
            let mut follow_ups=0usize;let mut stop_follow_ups=0usize;let mut compactions=0usize;let mut proactive_suppressed=false;
            let mut recent_tools=std::collections::VecDeque::<String>::new();
            let max_turns=state.config.max_turns.unwrap_or(DEFAULT_MAX_MODEL_TURNS).max(1);let mut provider_calls=0usize;
            while provider_calls<max_turns {
                if control.cancel.is_cancelled(){return Err(AppError::Cancelled);}
                self.flush_hook_runs(&attempt,control.session,&state).await?;
                self.consume_boundary_deliveries(control.session,control.execution,message).await?;
                control.phase(agena_domain::ExecutionPhase::PreparingModel);
                // Configuration changes are read only at a provider boundary;
                // the request's frozen facts and route then stay fixed.
                self.prepare_options_with_state(control.session,&mut options,&state).await?;
                if let Some(max)=control.budget.as_ref().and_then(|b|b.max_tokens){
                    let remaining=max.saturating_sub(usage.total_tokens());if remaining==0{return Err(AppError::AgentBudgetExceeded("the delegated run reached its token budget".into()));}
                    let output=options.max_output_tokens.unwrap_or(u32::MAX).min(remaining.min(u32::MAX as u64) as u32);options.max_output_tokens=Some(output);crate::prompt_budget::align_output_token_overrides(&mut options.request_override,output);
                }
                let round_id=RequestId::new();
                let round=match persist(||attempt.begin_current_round(round_id)).await{
                    Ok(receipt)=>receipt.value,
                    Err(KernelError::Invalid(message)) if state.config.auto_compaction.enabled&&compactions<4&&(message.contains("prompt exceeds input identity budget")||message.contains("current prompt scan exceeds"))=>{
                        compactions+=1;if self.perform_compaction(&attempt,&control,&options,&state,None).await?{continue;}return Err(AppError::Config(message));
                    },Err(error)=>return Err(kernel_error(error)),
                };
                let facts=round.prompt().await.map_err(kernel_error)?;
                let context=self.execution_context_with_state(control.session,&state).await?;
                let executor=state.executor.for_session_context_async(&context).await.with_cancellation_token(Some(control.cancel.clone()));
                let native=state.providers.native_compaction_enabled(&options.model)?;
                let prompt=self.prompt_hydrator.hydrate_with_executor(&self.kernel().reader,facts,&options.model,native,&executor,context.conversation.as_ref()).await.map_err(kernel_error)?;
                let bindings=if state.providers.agena_tool_mode(&options.model)?.is_disabled(){Vec::new()}else{agena_tool_api::ToolApiRegistry.definitions()};
                let mut system_text=self.assemble_session_system_prompt_with_executor(&executor,options.system.as_deref(),!bindings.is_empty());
                system_text.push_str(&self.agent_roster(control.session).await?);
                let system=Some(system_text);
                let token_estimate=prompt.estimate_tokens(system.as_deref(),&bindings).map_err(kernel_error)?;
                let metadata=state.providers.model_metadata(&options.model).unwrap_or_default();
                let hard_limit=crate::prompt_budget::prompt_token_budget(metadata.limits.context_window_tokens,metadata.limits.max_input_tokens,options.max_output_tokens).map(u64::from);
                let budget=crate::estimate_auto_compaction_limit_tokens(metadata.limits.context_window_tokens,metadata.limits.max_input_tokens,options.max_output_tokens,state.config.auto_compaction.reserved_tokens).unwrap_or_else(||crate::estimate_prompt_tokens_from_chars(state.governor.max_prompt_chars()));
                let policy=compaction_policy_key(&options,&state)?;
                let status=self.kernel().reader.prompt_window_status(control.session).await.map_err(kernel_error)?;
                let disabled=status.policy_key.as_deref()==Some(policy.as_str())&&status.auto_compaction_disabled;
                let hard_exceeded=hard_limit.is_some_and(|limit|token_estimate>limit);
                let proactive=state.config.auto_compaction.enabled&&!disabled&&!proactive_suppressed&&compactions<4&&token_estimate>budget;
                if hard_exceeded||proactive{
                    // The captured round has had no provider effect. End it
                    // before a separate compaction so failed summaries cannot
                    // move its input boundary.
                    let finish=RequestId::new();persist(||round.finish(finish,None,false)).await.map_err(kernel_error)?;
                    if state.config.auto_compaction.enabled&&!disabled&&compactions<4 {
                        compactions+=1;match self.perform_compaction(&attempt,&control,&options,&state,Some(token_estimate)).await{
                            Ok(true)=>continue,Ok(false)=>{},Err(AppError::Cancelled)=>return Err(AppError::Cancelled),Err(error)=>tracing::warn!(%error,"automatic compaction did not produce a usable checkpoint"),
                        }
                    }
                    if hard_exceeded{return Err(AppError::PromptBudgetExceeded{session_id:control.session.get(),estimated_tokens:token_estimate,limit_tokens:hard_limit.unwrap_or_default()});}
                    proactive_suppressed=true;continue;
                }
                let completion=completion_request(&options,system,Vec::new(),bindings);
                let request=KernelRoundRequest::new(round,prompt,completion,Some(control.cancel.clone())).map_err(kernel_error)?;
                let mut run=state.processor.kernel_round(request,self.kernel().reader.clone(),state.providers.clone()).map_err(kernel_error)?;
                provider_calls+=1;
                control.phase(agena_domain::ExecutionPhase::StreamingModel);
                let outcome=self.drive_round(&mut run).await.map_err(kernel_error)?;
                if outcome.cancelled || control.cancel.is_cancelled(){return Err(AppError::Cancelled);}
                if let Some(failure)=outcome.failure {
                    if self.stop_hook_followup(&attempt,&control,&state,&mut stop_follow_ups,Some(failure.user.fallback.clone())).await?{continue;}
                    structured_round_failure=true;
                    return Err(AppError::Internal(failure.user.fallback));
                }
                if let Some(observed)=&outcome.usage {usage.add_assign(observed);}
                if let Some(budget)=&control.budget
                    && (budget.max_tokens.is_some_and(|max|usage.total_tokens()>=max)||budget.max_cost_microusd.is_some_and(|max|usage.effective_cost_usd()*1_000_000.0>=max as f64)){
                        return Err(AppError::AgentBudgetExceeded("the delegated run reached its token or cost budget".into()));
                }
                if !outcome.tools.is_empty(){
                    for tool in &outcome.tools{
                        if tool_call_repeats(&recent_tools,&tool.signature){return Err(AppError::Config("the model repeated the same tool call without progress".into()));}
                        recent_tools.push_back(tool.signature.clone());if recent_tools.len()>32{recent_tools.pop_front();}
                    }
                    follow_ups=0;
                    control.phase(agena_domain::ExecutionPhase::ExecutingTools);
                    let max=state.config.max_concurrent_tools.max(1);
                    let tools=stream::iter(outcome.tools.into_iter().map(|tool|{
                        let executor=executor.clone();let control=control.clone();let state=state.clone();let model=options.model.clone();
                        async move {self.dispatch_tool_api(tool,executor,control,state,model).await}
                    })).buffer_unordered(max).collect::<Vec<_>>().await;
                    for result in tools {result?;}
                    continue;
                }
                if !outcome.follow_up_requested && outcome.finish_reason!=agena_domain::FinishReason::MaxTokens {
                    let conditions=self.stop_conditions(control.session,&state).await?;
                    let stop=RequestId::new();
                    let decision=persist(||attempt.prepare_stop(stop,conditions.clone())).await.map_err(kernel_error)?.value;
                    match decision {
                        agena_storage::kernel::StopDecision::Continue=>continue,
                        agena_storage::kernel::StopDecision::AwaitingBackground=>{},
                        agena_storage::kernel::StopDecision::Ready=>{
                            if self.stop_hook_followup(&attempt,&control,&state,&mut stop_follow_ups,None).await?{continue;}
                            self.flush_hook_runs(&attempt,control.session,&state).await?;
                        },
                    }
                    let finish=RequestId::new();
                    match persist(||attempt.finish(finish,ExecutionPhase::Succeeded)).await {
                        Ok(_)=>{completed=true;return Ok(());},
                        Err(KernelError::PendingDelivery(execution)) if execution==control.execution=>continue,
                        Err(error)=>return Err(kernel_error(error)),
                    }
                }
                follow_ups+=1;if follow_ups>=4{return Err(AppError::Config("the model reached the continuation limit".into()));}
            }
            Err(AppError::Config("the execution reached its configured model turn limit".into()))
        }.await;
        if completed {
            return result;
        }
        let phase = if control.cancel.is_cancelled() || matches!(result, Err(AppError::Cancelled)) {
            ExecutionPhase::Cancelled
        } else if result.is_ok() {
            ExecutionPhase::Succeeded
        } else {
            ExecutionPhase::Failed
        };
        // Keep manager-level failure reasons durable and user-visible. The
        // notice is written while the attempt is still running, then projected
        // as an `error` part on read. Provider-round failures already have a
        // structured failure and are projected from that round instead.
        if phase == ExecutionPhase::Failed
            && !structured_round_failure
            && let Err(error) = &result
        {
            match agena_runtime_contracts::part_content::failure_notice_text(&error.failure()) {
                Ok(text) => {
                    if let Err(error) =
                        persist(|| attempt.notice(RequestId::new(), text.clone())).await
                    {
                        tracing::warn!(
                            session_id = control.session.get(),
                            execution_id = %control.execution,
                            %error,
                            "could not persist the execution failure notice"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    session_id = control.session.get(),
                    execution_id = %control.execution,
                    %error,
                    "could not encode the execution failure notice"
                ),
            }
        }
        let finish = RequestId::new();
        match persist(|| attempt.finish(finish, phase)).await {
            Ok(_) => {}
            Err(KernelError::StaleAttempt | KernelError::Conflict(_)) => {
                let stored = self
                    .kernel()
                    .reader
                    .execution(control.session, control.execution)
                    .await
                    .map_err(kernel_error)?;
                if !stored.phase.is_terminal() {
                    return Err(AppError::Internal(
                        "attempt was fenced without a terminal execution decision".into(),
                    ));
                }
            }
            Err(error) => return Err(kernel_error(error)),
        }
        result
    }
    async fn drive_round(
        &self,
        run: &mut KernelRoundRun,
    ) -> agena_storage::kernel::Result<KernelRoundOutcome> {
        let mut delay = Duration::from_millis(25);
        loop {
            match run.drive().await {
                Ok(value) => return Ok(value.clone()),
                Err(error) if transient(&error) => {
                    tracing::warn!(%error,"provider round awaits persistence; retaining its pending operation");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
                Err(error) => return Err(error),
            }
        }
    }
    pub(super) async fn execute_tool(
        &self,
        tool: ToolHandle,
        invocation: ToolInvocation,
        executor: ToolExecutor,
        control: Arc<RunControl>,
        state: Arc<ExecutionState>,
        model: ModelRef,
    ) -> Result<(), AppError> {
        let key = (control.session.get(), tool.id().get());
        let permit_slot: ToolPermitSlot = Arc::new(tokio::sync::Mutex::new(None));
        self.tools.lock().unwrap_or_else(|p| p.into_inner()).insert(
            key,
            LiveTool {
                tool: tool.clone(),
                control: control.clone(),
                permit: permit_slot.clone(),
                permissions: Default::default(),
            },
        );
        let mut applied_invocation = invocation.clone();
        let runtime = tokio::runtime::Handle::current();
        let manager = self.clone();
        let authorizer = executor.clone();
        let authority_tool = tool.clone();
        let authority_control = control.clone();
        let authority_state = state.clone();
        let executor = executor.with_read_path_authorizer(Arc::new(move |path| {
            let check = authorizer
                .path_permission_check(agena_domain::AccessKind::Read, &path.to_string_lossy());
            runtime.block_on(agena_async::permission_wait::scope(
                Some(key),
                agena_async::permission_wait::waiting(manager.authorize_checks(
                    &authority_tool,
                    vec![check],
                    &authorizer,
                    &authority_control,
                    &authority_state,
                )),
            ))
        }));
        let effect=async {
            let prepared=executor.prepare_invocation(&invocation,key.0,key.1).await?;
            let (invocation,shell)=executor.prepare_shell_invocation(&prepared.invocation,key.0,key.1).await?;
            applied_invocation=invocation.clone();
            self.authorize_tool(&tool,&invocation,&executor,&control,&state).await?;
            if control.cancel.is_cancelled(){return Err(ToolError::Cancelled);}
            let kind=executor.streaming_content_kind(&invocation)?;
            let output=match kind {Some(kind)=>Some(crate::tool::ToolOutputStreams::open(&tool,kind).await.map_err(|e|ToolError::invalid_input(e.to_string()))?),None=>None};
            let permit=tokio::select!{_ = control.cancel.cancelled()=>return Err(ToolError::Cancelled),permit=state.tools.clone().acquire_owned()=>permit.map_err(|_|ToolError::Cancelled)?};
            *permit_slot.lock().await = Some(permit);
            let mut execution=executor.execute_invocation(&invocation,crate::tool::ToolRuntimeContext{
                session_id:Some(key.0),call_id:Some(key.1),prepared_shell_command:shell,launch_provenance:None,
                output,tool:Some(tool.clone()),
            }).await?;
            if let Some(title)=prepared.title_override{execution.view.set_title(title);}execution.view.metadata.extend(prepared.metadata);
            Ok(execution)
        }.await;
        permit_slot.lock().await.take();
        // The effect is now over. Everything below only retries persistence of
        // this exact observed value, never invocation preparation or execution.
        let effect = match effect {
            Err(ToolError::UserInputRequired(input)) => self
                .request_host_user_input(key.0, key.1, *input)
                .await
                .map(super::host::input_execution)
                .map_err(|e| ToolError::invalid_input(e.to_string())),
            other => other,
        };
        let (phase, value, blobs) = match effect {
            Ok(execution) => match self
                .prepare_tool_result(
                    &tool,
                    control.session,
                    &model,
                    &state.providers,
                    &executor,
                    &applied_invocation,
                    &execution,
                )
                .await
            {
                Ok((value, blobs)) => (ToolPhase::Succeeded, value, blobs),
                Err(error) => {
                    let failure = error.failure();
                    (
                        ToolPhase::Failed,
                        json!({"error":agena_failure::UserProblem::from(&failure),"failure":failure,"invocation":applied_invocation,"metadata":{},"capture_error":"the external effect completed, but its result could not be prepared"}),
                        Vec::new(),
                    )
                }
            },
            Err(error) => {
                let cancelled = matches!(error, ToolError::Cancelled);
                let failure = tool_error(error).failure();
                (
                    if cancelled {
                        ToolPhase::Cancelled
                    } else {
                        ToolPhase::Failed
                    },
                    json!({"error":agena_failure::UserProblem::from(&failure),"failure":failure,"invocation":applied_invocation,"metadata":{}}),
                    Vec::new(),
                )
            }
        };
        let result = ToolResult { phase, value };
        let command = RequestId::new();
        let committed = persist(|| tool.complete_with_blobs(command, result.clone(), &blobs)).await;
        self.tools
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
        self.host_sequences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
        match committed {
            Ok(_) => Ok(()),
            Err(KernelError::StaleAttempt | KernelError::Conflict(_))
                if control.cancel.is_cancelled() =>
            {
                Ok(())
            }
            Err(error) => Err(kernel_error(error)),
        }
    }
    pub(super) async fn authorize_tool(
        &self,
        tool: &ToolHandle,
        invocation: &ToolInvocation,
        executor: &ToolExecutor,
        control: &Arc<RunControl>,
        state: &ExecutionState,
    ) -> Result<(), ToolError> {
        let checks = executor
            .collect_permission_checks_for_invocation_in_session(
                invocation,
                Some(control.session.get()),
            )
            .await?;
        self.authorize_checks(tool, checks, executor, control, state)
            .await
    }

    pub(super) async fn authorize_checks(
        &self,
        tool: &ToolHandle,
        checks: Vec<agena_tool::ToolPermissionCheck>,
        executor: &ToolExecutor,
        control: &Arc<RunControl>,
        state: &ExecutionState,
    ) -> Result<(), ToolError> {
        let permissions = self
            .tools
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(control.session.get(), tool.id().get()))
            .map(|live| live.permissions.clone())
            .ok_or(ToolError::Cancelled)?;
        let mut granted = tokio::select! {
            _ = control.cancel.cancelled() => return Err(ToolError::Cancelled),
            granted = permissions.lock() => granted,
        };
        let workspace = self
            .current_workspace_id()
            .await
            .map_err(|e| ToolError::invalid_input(e.to_string()))?;
        for check in checks {
            if control.cancel.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            let action_key = permission_key(&check.action)
                .map_err(|e| ToolError::invalid_input(e.to_string()))?;
            let rules = self
                .kernel_runtime
                .permissions
                .resolve(&action_key, Some(control.session.get()), Some(workspace))
                .await
                .map_err(|e| ToolError::invalid_input(e.to_string()))?;
            let rules = rules
                .into_iter()
                .map(|r| agena_permission::RuleEntry {
                    id: r.id,
                    revision_ms: r.updated_at_ms.or(r.created_at_ms),
                    scope: r.scope,
                    source: r.source,
                    reason: r.reason,
                    operator: r.operator,
                    mode: r.mode,
                })
                .collect::<Vec<_>>();
            let resolution = agena_permission::apply_rules(&check.decision, &rules);
            let decision = resolution.decision.clone();
            let grant_key = match &check.action {
                PermissionAction::NetworkAccess { host, port, .. } => {
                    serde_json::to_string(&("network", host, port))
                        .map_err(|e| ToolError::invalid_input(e.to_string()))?
                }
                _ => action_key.clone(),
            };
            // Re-read policy/rules even for a grant made earlier in this call:
            // a newly effective deny always wins over a remembered grant.
            if !matches!(decision, PermissionDecision::Deny { .. }) && granted.contains(&grant_key)
            {
                continue;
            }
            let automatic = matches!(decision, PermissionDecision::Auto { .. });
            let decision = if matches!(decision, PermissionDecision::Auto { .. }) {
                match tokio::select! {
                    _ = control.cancel.cancelled() => return Err(ToolError::Cancelled),
                    verdict = self.classify_permission(&check.action, &decision, control.session, state, executor) => verdict,
                } {
                    Some(true) => PermissionDecision::Allow,
                    Some(false) => PermissionDecision::Deny {
                        reason: "automatic review rejected this action".into(),
                    },
                    None => PermissionDecision::Ask {
                        reason: "automatic review could not decide this action".into(),
                    },
                }
            } else {
                decision
            };
            match decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Deny { reason } => {
                    let (source, scope, operator, rule_id, revision, authority) =
                        match &resolution.source {
                            agena_domain::PermissionResolutionSource::PersistedRule {
                                source,
                                scope,
                                operator,
                                rule_id,
                                revision_ms,
                                ..
                            } => (
                                Some(source.clone()),
                                Some(*scope),
                                operator.clone(),
                                *rule_id,
                                *revision_ms,
                                agena_domain::PermissionAuthorityKind::PersistedRule,
                            ),
                            _ => (
                                Some("static_policy".into()),
                                None,
                                None,
                                None,
                                None,
                                if automatic {
                                    agena_domain::PermissionAuthorityKind::AutoApprovalModel
                                } else {
                                    agena_domain::PermissionAuthorityKind::StaticPolicy
                                },
                            ),
                        };
                    return Err(ToolError::PolicyDenied(Box::new(
                        agena_domain::PolicyDeniedResult {
                            action: check.action.clone(),
                            related_actions: Vec::new(),
                            denied_actions: vec![check.action.clone()],
                            reason,
                            explanation: resolution.explanation,
                            source,
                            scope,
                            operator,
                            authority,
                            rule_id,
                            rule_revision_ms: revision,
                            trace: resolution.trace,
                        },
                    )));
                }
                PermissionDecision::Ask { reason } | PermissionDecision::Auto { reason } => {
                    if executor.session_relation_kind().is_subagent() {
                        return Err(ToolError::CapabilityUnavailable(
                            super::host::parent_interaction_required(&reason),
                        ));
                    }
                    let command = RequestId::new();
                    let request = InteractionRequest::Permission {
                        action_key: action_key.clone(),
                        description: reason,
                    };
                    let interaction =
                        persist(|| tool.request_interaction(command, request.clone()))
                            .await
                            .map_err(|e| ToolError::invalid_input(e.to_string()))?
                            .value;
                    control.phase(agena_domain::ExecutionPhase::AwaitingInteraction);
                    let decision = self
                        .wait_interaction(control.session, interaction, Some(&control.cancel))
                        .await
                        .map_err(|e| ToolError::invalid_input(e.to_string()))?;
                    control.phase(agena_domain::ExecutionPhase::ExecutingTools);
                    if !matches!(
                        &decision,
                        InteractionDecision::Permission { allowed: true, .. }
                    ) {
                        return Err(ToolError::UserDeclined(Box::new(
                            agena_domain::UserDeclinedResult {
                                request_id: interaction.get().to_string(),
                                action: check.action.clone(),
                                related_actions: Vec::new(),
                                reason: match &decision {
                                    InteractionDecision::Permission { reply, .. } => {
                                        reply.reason.clone()
                                    }
                                    _ => Some("permission request was declined".into()),
                                },
                                persisted_scope: match &decision {
                                    InteractionDecision::Permission { reply, .. }
                                        if matches!(
                                            reply.kind,
                                            agena_domain::PermissionReplyKind::DenyAlways
                                        ) =>
                                    {
                                        Some(
                                            reply
                                                .scope
                                                .unwrap_or(agena_domain::PermissionScope::Session),
                                        )
                                    }
                                    _ => None,
                                },
                            },
                        )));
                    }
                }
            }
            granted.insert(grant_key);
        }
        Ok(())
    }
    async fn stop_hook_followup(
        &self,
        attempt: &AttemptHandle,
        control: &RunControl,
        state: &ExecutionState,
        count: &mut usize,
        error: Option<String>,
    ) -> Result<bool, AppError> {
        if control.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let last = self
            .final_text_for(control.session, Some(control.execution), 64 * 1024)
            .await?;
        let result = state
            .executor
            .plugin_manager()
            .dispatch_agent_stop_cancellable(
                agena_plugin_host::AgentStopInput {
                    session_id: control.session.get(),
                    stop_hook_active: *count > 0,
                    last_assistant_message: last,
                    run_error: error,
                },
                Some(control.cancel.clone()),
            )
            .await;
        let dispatch = match result {
            Ok(dispatch) => dispatch,
            Err(error) => {
                if control.cancel.is_cancelled() {
                    return Err(AppError::Cancelled);
                }
                tracing::warn!(%error,"agent.stop hook failed");
                return Ok(false);
            }
        };
        self.flush_hook_runs(attempt, control.session, state)
            .await?;
        let Some(message) = dispatch.patch.continue_with_message else {
            return Ok(false);
        };
        if *count >= 4 {
            return Err(AppError::Config(
                "plugin stop hooks reached their continuation limit".into(),
            ));
        }
        *count += 1;
        let text = format!(
            "Plugin stop hook follow-up:\n{}",
            crate::prompt_budget::truncate_prompt_text(
                &message,
                256 * 1024,
                " [hook continuation limited]"
            )
        );
        let command = RequestId::new();
        persist(|| attempt.notice(command, text.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(true)
    }
    pub(super) async fn wait_interaction(
        &self,
        session: SessionId,
        interaction: InteractionId,
        cancel: Option<&CancellationToken>,
    ) -> Result<InteractionDecision, AppError> {
        let mut watch = self
            .kernel()
            .reader
            .watch(session, None)
            .map_err(kernel_error)?;
        loop {
            let view = self
                .kernel()
                .reader
                .interaction(session, interaction)
                .await
                .map_err(kernel_error)?;
            if let Some(decision) = view.decision.clone() {
                return Ok(decision);
            }
            let cancellation = async {
                match cancel {
                    Some(cancel) => cancel.cancelled().await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {_ = cancellation=>return Err(AppError::Cancelled),result=watch.recv()=>{
                if result.is_err(){watch=self.kernel().reader.watch(session,None).map_err(kernel_error)?;}
            },_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
    }
    pub(super) async fn wait_session_idle(&self, session: i64) {
        loop {
            let run = self
                .active
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .values()
                .find(|r| r.session.get() == session)
                .cloned();
            let Some(run) = run else {
                return;
            };
            let notified = run.done.notified();
            if !run.settled.load(std::sync::atomic::Ordering::Acquire) {
                notified.await;
            }
        }
    }
    pub(super) async fn prepare_options(
        &self,
        session: SessionId,
        options: &mut SessionRunOptions,
    ) -> Result<(), AppError> {
        let state = self.state.load_full();
        self.prepare_options_with_state(session, options, &state)
            .await
    }
    async fn prepare_options_with_state(
        &self,
        session: SessionId,
        options: &mut SessionRunOptions,
        state: &ExecutionState,
    ) -> Result<(), AppError> {
        let context = self.execution_context_with_state(session, state).await?;
        let selection = state
            .config
            .default_selection
            .overlay_with_cascade(&context.selection);
        if selection.model_ref().ok().flatten().as_ref() == Some(&options.model) {
            if options.thinking_mode.is_none() {
                options.thinking_mode = selection.thinking_mode;
            }
            if options.speed_mode.is_none() {
                options.speed_mode = selection.speed_mode;
            }
            if options.verbosity.is_none() {
                options.verbosity = selection.verbosity;
            }
        }
        self.apply_model_modes_with_state(options, state)?;
        let metadata = state
            .providers
            .model_metadata(&options.model)
            .unwrap_or_default();
        if options.temperature.is_none() {
            options.temperature = metadata.parsed_default_temperature();
        }
        let requested =
            crate::prompt_budget::requested_output_tokens_from_override(&options.request_override)
                .or(options.max_output_tokens);
        let output = crate::session_output_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_output_tokens,
            requested,
        );
        options.max_output_tokens = Some(output);
        crate::prompt_budget::align_output_token_overrides(&mut options.request_override, output);
        Ok(())
    }
    pub(super) fn options_for_selection(
        &self,
        selection: &ExecutionSelection,
    ) -> Result<SessionRunOptions, AppError> {
        let state = self.state.load_full();
        let selection = state
            .config
            .default_selection
            .overlay_with_cascade(selection);
        let model = state
            .providers
            .resolve_default_model_selection(&selection)?
            .ok_or_else(|| {
                AppError::Config("select a provider and model before continuing".into())
            })?;
        Ok(SessionRunOptions {
            model,
            thinking_mode: selection.thinking_mode,
            speed_mode: selection.speed_mode,
            verbosity: selection.verbosity,
            model_supports_parallel_tool_calls: selection.parallel_tool_calls,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        })
    }
}

pub(super) fn execution_outcome(
    execution: &ExecutionView,
) -> crate::SessionExecutionCommandOutcome {
    let turn = agena_domain::TurnId(execution.id.0);
    let reply = agena_domain::AssistantReplyId(execution.id.0);
    crate::SessionExecutionCommandOutcome::accepted(
        execution.session_id.get(),
        execution.id,
        turn,
        reply,
    )
}
pub(super) fn completion_request(
    options: &SessionRunOptions,
    system: Option<String>,
    turns: Vec<agena_provider::CompletionInputRun>,
    bindings: Vec<agena_provider::ToolApiDefinition>,
) -> agena_provider::CompletionRequest {
    agena_provider::CompletionRequest {
        model_supports_parallel_tool_calls: options.model_supports_parallel_tool_calls,
        model: options.model.model_id.clone(),
        system,
        turns,
        tool_api_functions: bindings,
        provider_native_tools: Default::default(),
        disable_tools: false,
        temperature: options.temperature,
        max_output_tokens: options.max_output_tokens,
        prompt_cache_key: None,
        previous_response_id: None,
        prompt_window_generation: None,
        provider_compaction: None,
        stop_sequences: Vec::new(),
        top_p: None,
        top_k: None,
        seed: None,
        thinking: options.thinking.clone(),
        verbosity: options.verbosity.clone(),
        response_format: None,
        responses_api_metadata: None,
        request_override: options.request_override.clone(),
    }
}
pub(super) fn permission_key(action: &PermissionAction) -> Result<String, serde_json::Error> {
    let mut action = action.clone();
    if let PermissionAction::Tool { tool_name, .. } = &mut action
        && let Some(name) =
            agena_domain::ToolPermissionConfig::canonical_command_policy_tool_name(tool_name)
    {
        *tool_name = name.into();
    }
    serde_json::to_string(&action)
}
fn compaction_policy_key(
    options: &SessionRunOptions,
    state: &ExecutionState,
) -> Result<String, AppError> {
    use sha2::{Digest, Sha256};
    let route = state.providers.media_route_binding(&options.model)?;
    let facts = serde_json::to_vec(&(
        crate::COMPACTION_POLICY_VERSION,
        &options.model,
        route,
        options.max_output_tokens,
        &options.thinking_mode,
        &options.speed_mode,
        state.config.auto_compaction.reserved_tokens,
        state.governor.max_prompt_chars(),
    ))?;
    Ok(hex::encode(Sha256::digest(facts)))
}

impl SessionManager {
    pub(super) fn apply_model_mode_requests(
        &self,
        options: &mut SessionRunOptions,
    ) -> Result<(), AppError> {
        let execution = self.state.load_full();
        self.apply_model_modes_with_state(options, &execution)
    }
    pub(super) fn apply_model_modes_with_state(
        &self,
        options: &mut SessionRunOptions,
        execution: &ExecutionState,
    ) -> Result<(), AppError> {
        let provider_registry = &execution.providers;
        let resolved_adapter_id = options.model.adapter_id.clone();

        let mut merged_override = options.request_override.clone();
        merged_override.remove_legacy_parallel_tool_calls_patch();
        let thinking_modes = provider_registry.model_thinking_modes(&options.model)?;
        let default_thinking_mode = || {
            thinking_modes.iter().find_map(|mode| {
                mode.is_default
                    .then(|| mode.selector().map(|selector| selector.into_owned()))
                    .flatten()
            })
        };
        if options.thinking_mode.is_none() {
            options.thinking_mode = default_thinking_mode();
        }
        // A persisted selection can outlive the catalog entry it named: the
        // model was switched, or a provider redefined its modes. The stored
        // mode must not make the session unusable — fall back to the model's
        // default instead. Explicit requests are validated by the application
        // layer before they reach this point.
        if let Some(thinking_mode_name) = options.thinking_mode.as_deref()
            && !thinking_modes
                .iter()
                .any(|mode| mode.selector().as_deref() == Some(thinking_mode_name))
        {
            tracing::warn!(
                model = %options.model,
                thinking_mode = thinking_mode_name,
                "selected think mode is no longer available; using the model default"
            );
            options.thinking_mode = default_thinking_mode();
        }
        if let Some(thinking_mode_name) = options.thinking_mode.as_deref() {
            let thinking_mode = thinking_modes
                .iter()
                .find(|mode| mode.selector().as_deref() == Some(thinking_mode_name))
                .ok_or_else(|| {
                    AppError::Config(format!(
                        "model `{}` has no think mode `{thinking_mode_name}`",
                        options.model
                    ))
                })?;
            options.thinking = thinking_mode.thinking.clone();
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &thinking_mode.request_override,
                &thinking_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        let speed_modes = provider_registry.model_speed_modes(&options.model)?;
        let default_speed_mode = || {
            speed_modes
                .iter()
                .find(|(_, mode)| mode.is_default)
                .map(|(name, _)| name.clone())
        };
        if options.speed_mode.is_none() {
            options.speed_mode = default_speed_mode();
        }
        if let Some(speed_mode_name) = options.speed_mode.as_deref()
            && !speed_modes.contains_key(speed_mode_name)
        {
            tracing::warn!(
                model = %options.model,
                speed_mode = speed_mode_name,
                "selected speed mode is no longer available; using the model default"
            );
            options.speed_mode = default_speed_mode();
        }
        if let Some(speed_mode_name) = options.speed_mode.as_deref() {
            let speed_mode = speed_modes.get(speed_mode_name).ok_or_else(|| {
                AppError::Config(format!(
                    "model `{}` has no speed mode `{speed_mode_name}`",
                    options.model
                ))
            })?;
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &speed_mode.request_override,
                &speed_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        merged_override.remove_legacy_parallel_tool_calls_patch();
        options.model_supports_parallel_tool_calls = Some(
            provider_registry
                .model_metadata(&options.model)
                .unwrap_or_default()
                .supports_parallel_tool_calls_for_model(),
        );
        options.request_override = merged_override;
        Ok(())
    }
}

fn mode_request_override_for_adapter(
    request_override: &agena_domain::ModelSpeedModeRequestOverride,
    adapter_overrides: &std::collections::BTreeMap<
        String,
        agena_domain::ModelSpeedModeRequestOverride,
    >,
    resolved_adapter_id: Option<&agena_domain::AdapterId>,
) -> agena_domain::ModelSpeedModeRequestOverride {
    let mut merged = request_override.clone();
    if let Some(adapter_id) = resolved_adapter_id.map(AsRef::<str>::as_ref)
        && let Some(adapter_override) = adapter_overrides.get(adapter_id)
    {
        merged = merged.merged_with(adapter_override);
    }
    merged
}

impl SessionManager {
    pub async fn compaction_status(&self, id: i64) -> Result<PromptWindowStatus, AppError> {
        let session = session_id(id)?;
        let mut status = (*self
            .kernel()
            .reader
            .prompt_window_status(session)
            .await
            .map_err(kernel_error)?)
        .clone();
        let state = self.state.load_full();
        let context = self.execution_context_with_state(session, &state).await?;
        let mut options = self.options_for_selection(&context.selection)?;
        self.prepare_options_with_state(session, &mut options, &state)
            .await?;
        let policy = compaction_policy_key(&options, &state)?;
        if status.policy_key.as_deref() != Some(policy.as_str()) {
            status.policy_key = Some(policy);
            status.consecutive_compaction_failures = 0;
            status.auto_compaction_disabled = false;
            status.native_compaction_unavailable = false;
            status.last_before_tokens = None;
            status.last_after_tokens = None;
        }
        Ok(status)
    }
    async fn perform_compaction(
        &self,
        attempt: &AttemptHandle,
        control: &RunControl,
        options: &SessionRunOptions,
        state: &ExecutionState,
        before_tokens: Option<u64>,
    ) -> Result<bool, AppError> {
        control.phase(agena_domain::ExecutionPhase::PreparingModel);
        let policy = compaction_policy_key(options, state)?;
        let status = self
            .kernel()
            .reader
            .prompt_window_status(control.session)
            .await
            .map_err(kernel_error)?;
        let native_unavailable = status.policy_key.as_deref() == Some(policy.as_str())
            && status.native_compaction_unavailable;
        let context = self
            .execution_context_with_state(control.session, state)
            .await?;
        let executor = state.executor.for_session_context_async(&context).await;
        let native = state.providers.native_compaction_enabled(&options.model)?;
        let mut system = self.assemble_session_system_prompt_with_executor(
            &executor,
            options.system.as_deref(),
            false,
        );
        system.push_str(&self.agent_roster(control.session).await?);
        let mut completion = completion_request(options, Some(system), Vec::new(), Vec::new());
        let metadata = state
            .providers
            .model_metadata(&options.model)
            .unwrap_or_default();
        let hard_limit = crate::prompt_token_budget(
            metadata.limits.context_window_tokens,
            metadata.limits.max_input_tokens,
            options.max_output_tokens,
        )
        .map(u64::from);
        let mut limit = 128usize;
        let minimum = if status.checkpoint_entry_id.is_some() {
            2
        } else {
            1
        };
        let (round, mut prompt, through, prefix_tokens) = loop {
            let command = RequestId::new();
            let round = persist(|| attempt.begin_compaction(command, limit))
                .await
                .map_err(kernel_error)?
                .value;
            let facts = round.prompt().await.map_err(kernel_error)?;
            let through = facts
                .entries
                .iter()
                .rev()
                .find(|entry| !matches!(entry.entry.payload, EntryPayload::Compaction { .. }))
                .map(|entry| entry.entry.id);
            let Some(through) = through else {
                if status.checkpoint_entry_id.is_none() {
                    return Err(AppError::Config("there is no history to compact".into()));
                }
                // The checkpoint already covers every eligible entry. Retain
                // a visible unchanged outcome without sending another model
                // request or installing a second checkpoint.
                let open = RequestId::new();
                persist(|| round.begin_text(open, false))
                    .await
                    .map_err(kernel_error)?;
                let finish = RequestId::new();
                persist(|| round.finish(finish, None, true))
                    .await
                    .map_err(kernel_error)?;
                let command = RequestId::new();
                persist(|| {
                    attempt.record_compaction(
                        command,
                        policy.clone(),
                        CompactionPolicyOutcome::Unchanged,
                    )
                })
                .await
                .map_err(kernel_error)?;
                return Ok(false);
            };
            let prompt = self
                .prompt_hydrator
                .hydrate_with_executor(
                    &self.kernel().reader,
                    facts,
                    &options.model,
                    native,
                    &executor,
                    context.conversation.as_ref(),
                )
                .await
                .map_err(kernel_error)?;
            let estimate = prompt
                .estimate_tokens(completion.system.as_deref(), &[])
                .map_err(kernel_error)?;
            if hard_limit.is_none_or(|hard| estimate <= hard) {
                break (round, prompt, through, estimate);
            }
            let finish = RequestId::new();
            persist(|| round.finish(finish, None, false))
                .await
                .map_err(kernel_error)?;
            if limit == minimum {
                let command = RequestId::new();
                persist(|| {
                    attempt.record_compaction(
                        command,
                        policy.clone(),
                        CompactionPolicyOutcome::Failed,
                    )
                })
                .await
                .map_err(kernel_error)?;
                return Err(AppError::PromptBudgetExceeded {
                    session_id: control.session.get(),
                    estimated_tokens: estimate,
                    limit_tokens: hard_limit.unwrap_or_default(),
                });
            }
            limit = (limit / 2).max(minimum);
        };
        prompt.install(&mut completion);
        completion.disable_tools = true;
        // Publish an owned compaction part before awaiting the provider. Its
        // empty text stream carries the running lifecycle, and is reused for
        // the eventual checkpoint rather than exposed as an assistant answer.
        let open = RequestId::new();
        let output = persist(|| round.begin_text(open, false))
            .await
            .map_err(kernel_error)?
            .value;
        let effect=async {
            let mut native_context=None;
            if native&&!native_unavailable {
                let future=crate::provider::with_request_cancellation(Some(control.cancel.clone()),state.providers.compact_conversation(&options.model,completion.clone()));
                let output=tokio::select!{_ = control.cancel.cancelled()=>return Err(AppError::Cancelled),result=future=>result};
                match output {
                    Ok(Some(output)) if output.has_checkpoint()=>{
                        let summary=native_portable_summary(&output)?;
                        let context=match output{agena_provider::ProviderCompactionOutput::OpenAiResponses{items}=>agena_provider::ProviderCompactionContext::OpenAiResponses{items}};
                        if let Some(summary)=summary{return Ok(Some((summary,Some(context),None)));}
                        native_context=Some(context);
                    },
                    Ok(Some(_))=>return Ok(None),
                    unavailable=>{
                        if let Err(error)=unavailable{tracing::warn!(%error,"native compaction unavailable; using local summarization");}
                        let command=RequestId::new();persist(||attempt.record_compaction(command,policy.clone(),CompactionPolicyOutcome::NativeUnavailable)).await.map_err(kernel_error)?;
                    },
                }
            }
            completion.system=Some("Summarize the conversation for the next assistant continuation. Preserve user instructions, decisions, implementation state, exact paths and identifiers, unfinished work, and tool outcomes. Omit repetition. Return only the compact checkpoint text. Do not perform work or call tools.".into());
            completion.max_output_tokens=Some(completion.max_output_tokens.unwrap_or(8192).min(8192));
            crate::prompt_budget::align_output_token_overrides(&mut completion.request_override,completion.max_output_tokens.unwrap_or(8192));
            state.processor.apply_chat_params_hook(options.model.provider_id.as_ref(),options.model.model_id.as_ref(),control.session.get(),&mut completion,Some(control.cancel.clone())).await;
            let future=crate::provider::with_request_cancellation(Some(control.cancel.clone()),state.providers.complete(&options.model,completion));
            let response=tokio::select!{_ = control.cancel.cancelled()=>return Err(AppError::Cancelled),result=future=>result?};
            if response.text.trim().is_empty(){return Err(AppError::EmptyResponse);}
            if response.text.len()>1024*1024{return Err(AppError::Config("compaction summary exceeds its byte budget".into()));}
            Ok(Some((response.text,native_context,response.usage)))
        }.await;
        let (summary, compaction, usage) = match effect {
            Ok(Some(value)) => value,
            Ok(None) => {
                let finish = RequestId::new();
                persist(|| round.finish(finish, None, true))
                    .await
                    .map_err(kernel_error)?;
                let command = RequestId::new();
                persist(|| {
                    attempt.record_compaction(
                        command,
                        policy.clone(),
                        CompactionPolicyOutcome::Unchanged,
                    )
                })
                .await
                .map_err(kernel_error)?;
                return Ok(false);
            }
            Err(error) => {
                let finish = RequestId::new();
                let failure = error.failure();
                let mut recorded = super::super::kernel::KernelProviderState::new(
                    options.model.clone(),
                    Value::Null,
                );
                recorded.failure = Some(failure);
                let value = serde_json::to_value(recorded)?;
                persist(|| round.finish(finish, Some(value.clone()), false))
                    .await
                    .map_err(kernel_error)?;
                if !matches!(error, AppError::Cancelled) {
                    let command = RequestId::new();
                    persist(|| {
                        attempt.record_compaction(
                            command,
                            policy.clone(),
                            CompactionPolicyOutcome::Failed,
                        )
                    })
                    .await
                    .map_err(kernel_error)?;
                }
                return Err(error);
            }
        };
        // The provider result is retained from this point. Storage retries
        // below only persist that result, including its exact native replay.
        super::results::append_text_chunks(&output, &summary)
            .await
            .map_err(kernel_error)?;
        let native = compaction.is_some();
        let replacement_tokens = crate::estimate_prompt_tokens_from_chars(
            summary.len().saturating_add(
                compaction
                    .as_ref()
                    .map(|value| serde_json::to_vec(value).map(|v| v.len()))
                    .transpose()?
                    .unwrap_or_default(),
            ),
        );
        let after_tokens = before_tokens.map(|before| {
            before
                .saturating_sub(prefix_tokens)
                .saturating_add(replacement_tokens)
        });
        let mut provider =
            super::super::kernel::KernelProviderState::new(options.model.clone(), Value::Null);
        provider.compaction = compaction;
        provider.usage = usage;
        let provider = serde_json::to_value(provider)?;
        let commit = RequestId::new();
        persist(|| round.commit_compaction(commit, output.id(), through, Some(provider.clone())))
            .await
            .map_err(kernel_error)?;
        let command = RequestId::new();
        persist(|| {
            attempt.record_compaction(
                command,
                policy.clone(),
                CompactionPolicyOutcome::Succeeded {
                    native,
                    before_tokens,
                    after_tokens,
                },
            )
        })
        .await
        .map_err(kernel_error)?;
        Ok(true)
    }
    pub(super) async fn classify_permission(
        &self,
        action: &PermissionAction,
        decision: &PermissionDecision,
        session: SessionId,
        state: &ExecutionState,
        executor: &ToolExecutor,
    ) -> Option<bool> {
        let context = self.execution_context_value(session).await.ok()?;
        let model = if let Some(selection) = &context.effective_permission.approval_model {
            selection.model_ref().ok()?
        } else {
            state
                .providers
                .resolve_default_model_selection(&context.selection)
                .ok()??
        };
        let options = SessionRunOptions {
            model: model.clone(),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            model_supports_parallel_tool_calls: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: Some(0.0),
            max_output_tokens: Some(4096),
        };
        let reason = match decision {
            PermissionDecision::Auto { reason } | PermissionDecision::Ask { reason } => {
                reason.as_str()
            }
            _ => "review this action",
        };
        let action = serde_json::to_string(action).ok()?;
        if action.len() > 64 * 1024 {
            return None;
        }
        // Context is a bounded recent history page; it never loads a session
        // aggregate. User instructions remain present through the checkpoint.
        let history = self.kernel().reader.history(session, None, 64).await.ok()?;
        let mut transcript = String::new();
        for entry in history.entries.iter().rev() {
            match &entry.payload {
                EntryPayload::User { document } => {
                    for node in &document.0 {
                        if let ComposerNode::Text { text } = node {
                            transcript.push_str(text);
                            transcript.push('\n');
                        }
                    }
                }
                EntryPayload::Notice { text } => {
                    transcript.push_str(text);
                    transcript.push('\n');
                }
                _ => {}
            }
        }
        if transcript.len() > 16 * 1024 {
            let mut start = transcript.len() - 16 * 1024;
            while !transcript.is_char_boundary(start) {
                start += 1;
            }
            transcript = transcript[start..].into();
        }
        let context = agena_permission::build_classifier_context_message(Some(&transcript), &[]);
        let action = agena_permission::build_classifier_action_message(&action, reason);
        let mut turns = Vec::new();
        if let Some(context) = context {
            turns.push(agena_provider::CompletionInputRun {
                role: Role::User,
                parts: vec![agena_provider::CompletionInputPart::Text { text: context }],
                provider_state: Default::default(),
            });
        }
        turns.push(agena_provider::CompletionInputRun {
            role: Role::User,
            parts: vec![agena_provider::CompletionInputPart::Text { text: action }],
            provider_state: Default::default(),
        });
        let mut request = completion_request(
            &options,
            Some(agena_permission::auto_approval_system_prompt(
                &executor.path_class_prompt_flags(),
            )),
            turns,
            Vec::new(),
        );
        request.tool_api_functions = agena_permission::auto_approval_decision_tools()
            .into_iter()
            .map(
                |(name, description, input_schema)| agena_provider::ToolApiDefinition {
                    name: name.into(),
                    description: description.into(),
                    input_schema,
                    output_schema: serde_json::json!({}),
                },
            )
            .collect();
        let response = tokio::time::timeout(
            agena_permission::AUTO_APPROVAL_CLASSIFY_TIMEOUT,
            state.providers.complete(&model, request),
        )
        .await
        .ok()?
        .ok()?;
        let verdict = response
            .tool_calls
            .iter()
            .find_map(|call| {
                let agena_provider::CompletionToolCall::Function {
                    name,
                    arguments_json,
                    ..
                } = call;
                agena_permission::ClassifierVerdict::from_tool_call(name, arguments_json)
            })
            .or_else(|| agena_permission::ClassifierVerdict::parse(&response.text))
            .or_else(|| {
                response
                    .reasoning_text
                    .as_deref()
                    .and_then(agena_permission::ClassifierVerdict::parse)
            })?;
        if verdict.allowed {
            Some(true)
        } else if verdict.cites_block_rule() {
            Some(false)
        } else {
            None
        }
    }
}
fn native_portable_summary(
    output: &agena_provider::ProviderCompactionOutput,
) -> Result<Option<String>, AppError> {
    let agena_provider::ProviderCompactionOutput::OpenAiResponses { items } = output;
    let mut text = String::new();
    let mut append = |value: &str| -> Result<(), AppError> {
        if text.len().saturating_add(value.len()).saturating_add(1) > 1024 * 1024 {
            return Err(AppError::Config(
                "native compaction summary exceeds its byte budget".into(),
            ));
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(value);
        Ok(())
    };
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("compaction") {
            if let Some(summary) = item.get("summary").and_then(Value::as_str) {
                append(summary)?;
            }
            if let Some(parts) = item.get("summary").and_then(Value::as_array) {
                for part in parts {
                    if let Some(value) = part.get("text").and_then(Value::as_str) {
                        append(value)?;
                    }
                }
            }
        }
        if matches!(
            item.get("type").and_then(Value::as_str),
            Some("summary" | "summary_text")
        ) && let Some(value) = item.get("text").and_then(Value::as_str)
        {
            append(value)?;
        }
    }
    Ok((!text.trim().is_empty()).then_some(text))
}

fn tool_call_repeats(recent: &std::collections::VecDeque<String>, signature: &str) -> bool {
    recent
        .iter()
        .filter(|seen| seen.as_str() == signature)
        .count()
        >= 4
}
