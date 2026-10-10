use agena_macros::ToolInput;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use crate::plugins::provided::workflow::{
    SessionRenameToolInput, WorkflowPlugin, WorkflowPluginConfig,
};
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{
    HostClient, HostContextStatusRequest, HostContextStatusResponse,
};
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput,
};

pub(crate) const SESSION_PLUGIN_ID: &str = "agena.session";

pub(crate) struct SessionPlugin {
    inner: WorkflowPlugin,
    host: OnceLock<Arc<dyn HostClient>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("names[]"), non_empty("names[]"), max_items("names", 32))]
#[serde(deny_unknown_fields)]
struct ExecutablesInput {
    /// Curated tool names or aliases. Empty lists installed tools and missing names.
    #[serde(default)]
    names: Vec<String>,
    /// Explicitly run bounded version probes. Requires 1–8 named tools.
    #[serde(default)]
    probe_versions: bool,
    /// Bypass the 15-second availability cache after installing/changing tools.
    #[serde(default)]
    refresh: bool,
}

#[derive(Debug, Clone, Default)]
struct GitFacts {
    branch: Option<String>,
    short_sha: Option<String>,
    dirty: Option<bool>,
}

fn run_git(workspace: &Path, args: &[&str]) -> SdkResult<Option<String>> {
    const MAX_GIT_FACT_BYTES: usize = 4 * 1024 * 1024;
    let mut command = std::process::Command::new("git");
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("GPG_TTY", "")
        .stdin(Stdio::null());
    let output = match agena_process::blocking::output(
        command,
        Duration::from_secs(15),
        MAX_GIT_FACT_BYTES,
    ) {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::FileTooLarge => {
            tracing::warn!(arguments = ?args, "Git inspection output exceeded its budget; facts remain unknown");
            return Ok(None);
        }
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
            return Err(PluginError::timeout_with_public_detail(
                "Git session environment inspection timed out after 15 seconds",
                "Git inspection timed out after 15 seconds.",
            ));
        }
        Err(error) => return Err(PluginError::internal_error(&error)),
    };
    if !output.status.success() {
        tracing::debug!(
            arguments = ?args,
            status = %output.status,
            stderr = %String::from_utf8_lossy(&output.stderr),
            "Git session environment probe exited unsuccessfully"
        );
        return Ok(None);
    }
    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            "decode Git session environment output as UTF-8",
            &error,
        ))
    })?;
    Ok(Some(stdout.trim().to_string()))
}

/// Porcelain v2 uses NUL-separated records and includes branch facts in the
/// same snapshot. Rename source paths are separate records, not headers.
fn parse_git_facts(status: &str) -> Option<GitFacts> {
    let mut facts = GitFacts {
        dirty: Some(false),
        ..GitFacts::default()
    };
    let mut skip_rename_source = false;
    let mut saw_oid = false;
    for record in status.split('\0').filter(|record| !record.is_empty()) {
        if skip_rename_source {
            skip_rename_source = false;
            continue;
        }
        if let Some(oid) = record.strip_prefix("# branch.oid ") {
            saw_oid = true;
            if oid != "(initial)" && oid.len() >= 8 && oid.chars().all(|ch| ch.is_ascii_hexdigit())
            {
                facts.short_sha = Some(oid[..8].to_owned());
            }
        } else if let Some(branch) = record.strip_prefix("# branch.head ") {
            facts.branch = Some(
                if branch == "(detached)" {
                    "HEAD"
                } else {
                    branch
                }
                .to_owned(),
            );
        } else if !record.starts_with("# ") && !record.starts_with("! ") {
            facts.dirty = Some(true);
            skip_rename_source = record.starts_with("2 ");
        }
    }
    (saw_oid && facts.branch.is_some()).then_some(facts)
}

fn git_facts(workspace: &Path) -> SdkResult<Option<GitFacts>> {
    Ok(
        run_git(workspace, &["status", "--porcelain=v2", "--branch", "-z"])?
            .as_deref()
            .and_then(parse_git_facts),
    )
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "session",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Inspect and manage the current runtime session and its environment, model, and token state.",
    translations(
        locale("zh-CN", summary = "查看并管理当前运行会话及其环境、模型和令牌状态。"),
        locale("zh-TW", summary = "檢視並管理目前執行階段的工作階段、環境、模型與 Token 狀態。"),
        locale("ja-JP", summary = "現在の実行セッションと、その環境・モデル・トークンの状態を確認および管理します。"),
        locale("ko-KR", summary = "현재 런타임 세션과 환경, 모델, 토큰 상태를 확인하고 관리합니다."),
        locale("fr-FR", summary = "Consulter et gérer la session d’exécution actuelle, son environnement, son modèle et l’état des jetons."),
        locale("de-DE", summary = "Die aktuelle Laufzeitsitzung sowie Umgebung, Modell und Tokenstatus prüfen und verwalten."),
        locale("es-ES", summary = "Consulta y gestiona la sesión de ejecución actual, su entorno, modelo y estado de tokens."),
        locale("hi-IN", summary = "मौजूदा रनटाइम सत्र, उसके परिवेश, मॉडल और टोकन स्थिति की जाँच और प्रबंधन करें।"),
        locale("ar-SA", summary = "افحص جلسة التشغيل الحالية وبيئتها ونموذجها وحالة الرموز وأدرها."),
        locale("pt-BR", summary = "Consulte e gerencie a sessão de execução atual, seu ambiente, modelo e estado de tokens.")
    ),
)]
impl SessionPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
            host: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(Arc::clone(&host))
            .map_err(|_| PluginError::internal("session plugin initialized more than once"))?;
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Inspect the current session metadata.",
        translations(
            locale("zh-CN", summary = "查看当前会话信息。"),
            locale("zh-TW", summary = "檢視目前工作階段資訊。"),
            locale("ja-JP", summary = "現在のセッション情報を確認します。"),
            locale("ko-KR", summary = "현재 세션 정보를 확인합니다."),
            locale(
                "fr-FR",
                summary = "Consulter les informations de la session actuelle."
            ),
            locale("de-DE", summary = "Aktuelle Sitzungsinformationen anzeigen."),
            locale("es-ES", summary = "Consulta la información de la sesión actual."),
            locale("hi-IN", summary = "मौजूदा सत्र की जानकारी देखें।"),
            locale("ar-SA", summary = "اعرض معلومات الجلسة الحالية."),
            locale("pt-BR", summary = "Consulte as informações da sessão atual.")
        )
    )]
    async fn get(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_get_session().await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Inspect the runtime workspace, git state, shell, platform, and available host CLIs.",
        translations(
            locale(
                "zh-CN",
                summary = "查看运行时工作区、Git 状态、Shell、平台和可用的主机 CLI。"
            ),
            locale(
                "zh-TW",
                summary = "檢視執行階段工作區、Git 狀態、Shell、平台與可用的主機 CLI。"
            ),
            locale(
                "ja-JP",
                summary = "実行環境のワークスペース、Git の状態、シェル、プラットフォーム、利用可能なホスト CLI を確認します。"
            ),
            locale(
                "ko-KR",
                summary = "런타임 작업 공간, Git 상태, 셸, 플랫폼 및 사용 가능한 호스트 CLI를 확인합니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter l’espace de travail, l’état Git, le shell, la plateforme et les CLI disponibles sur l’hôte."
            ),
            locale(
                "de-DE",
                summary = "Laufzeit-Arbeitsbereich, Git-Status, Shell, Plattform und verfügbare Host-CLIs anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Consulta el espacio de trabajo, el estado de Git, el shell, la plataforma y las CLI disponibles en el host."
            ),
            locale(
                "hi-IN",
                summary = "रनटाइम वर्कस्पेस, Git स्थिति, शेल, प्लेटफ़ॉर्म और उपलब्ध होस्ट CLI देखें।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض مساحة العمل وحالة Git والصدفة والمنصة وأدوات CLI المتاحة على المضيف."
            ),
            locale(
                "pt-BR",
                summary = "Consulte o espaço de trabalho, o estado do Git, o shell, a plataforma e as CLIs disponíveis no host."
            )
        )
    )]
    async fn environment(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let mut lines = vec![format!("Working directory: {workspace_root}")];
        let mut git_branch = None::<String>;
        let mut git_short_sha = None::<String>;
        let mut git_dirty = None;
        let git_workspace = workspace_root.clone();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "acquire a session environment worker",
                    &error,
                ))
            })?;
        let facts = tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            git_facts(Path::new(&git_workspace))
        })
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "Git session environment inspection task failed",
                &error,
            ))
        })?;
        let mut git_error = None;
        let facts = match facts {
            Ok(facts) => facts,
            Err(error) => {
                let detail = agena_failure::diagnostic::format_error_chain_with_context(
                    "Git environment facts are unavailable",
                    &error,
                );
                lines.push(detail.clone());
                git_error = Some(detail);
                None
            }
        };
        if let Some(facts) = facts {
            git_branch = facts.branch;
            git_short_sha = facts.short_sha;
            git_dirty = facts.dirty;
            if let (Some(branch), Some(short_sha)) =
                (git_branch.as_deref(), git_short_sha.as_deref())
            {
                let dirty = match git_dirty {
                    Some(true) => " (dirty)",
                    Some(false) => " (clean)",
                    None => " (status unknown)",
                };
                lines.push(format!("Git: {branch} @ {short_sha}{dirty}"));
            } else if let Some(branch) = git_branch.as_deref() {
                lines.push(format!("Git branch: {branch}"));
            }
        }
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                if cfg!(windows) {
                    "powershell".to_string()
                } else {
                    "/bin/bash".to_string()
                }
            });
        lines.push(format!("Shell: {shell}"));
        lines.push(format!(
            "OS: {} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
        let cli_inventory =
            agena_runtime_tools::cli_tools::discover(Path::new(&workspace_root), false)
                .await
                .map_err(|error| PluginError::internal_error(&error))?;
        lines.push(format!(
            "Available host CLIs: {}. Detailed usage: session.executables.",
            cli_inventory.available_names().join(", ")
        ));
        let catalog = match self.inner.host() {
            Ok(host) => match host.list_tools().await {
                Ok(mut tools) => {
                    tools.sort_by(|a, b| a.name.cmp(&b.name));
                    let bytes = serde_json::to_vec(&tools)
                        .map_err(|error| PluginError::internal_error(&error))?;
                    Some(
                        serde_json::json!({"status":"available","count":tools.len(),"sha256":hex::encode(Sha256::digest(&bytes)),
                        "interactive_shell":tools.iter().any(|tool|tool.name.ends_with("shell.write")),"output_recovery":tools.iter().any(|tool|tool.name.ends_with("fs.read"))}),
                    )
                }
                Err(_) => None,
            },
            Err(_) => None,
        };
        let build = serde_json::json!({"package_version":env!("CARGO_PKG_VERSION"),"target":env!("AGENA_TOOL_BUILD_TARGET"),"source_fingerprint":env!("AGENA_TOOL_SOURCE_FINGERPRINT"),"source_scope":env!("AGENA_TOOL_SOURCE_SCOPE")});
        lines.push(format!(
            "Compiled tool-runtime source: {} ({})",
            env!("AGENA_TOOL_SOURCE_FINGERPRINT"),
            env!("AGENA_TOOL_BUILD_TARGET")
        ));
        lines.push(format!(
            "Visible tool catalogue: {}",
            catalog
                .as_ref()
                .map(|catalog| catalog.to_string())
                .unwrap_or_else(
                    || "unavailable; capabilities were not inferred from the checkout".into()
                )
        ));
        let payload = serde_json::json!({
            "tool_runtime_build": build,
            "tool_catalog": catalog,
            "cli_tools": cli_inventory.compact(),
            "workspace_root": workspace_root,
            "git_branch": git_branch,
            "git_short_sha": git_short_sha,
            "git_dirty": git_dirty,
            "git_status_known": git_dirty.is_some(),
            "git_error": git_error,
            "shell": shell,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        });
        Ok(ToolInvokeOutput::from_parts(
            "session environment",
            "environment facts",
            lines.join("\n"),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Inspect installed modern CLIs, their task-specific usage, and optional versions.",
        translations(
            locale("zh-CN", summary = "查看已安装的现代 CLI、各自用途及可选版本。"),
            locale("zh-TW", summary = "檢視已安裝的現代 CLI、各自用途與可選版本。"),
            locale(
                "ja-JP",
                summary = "インストール済みの主要 CLI、その用途、必要に応じてバージョンを確認します。"
            ),
            locale(
                "ko-KR",
                summary = "설치된 주요 CLI와 용도, 선택적 버전 정보를 확인합니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter les CLI modernes installées, leur usage et, en option, leur version."
            ),
            locale(
                "de-DE",
                summary = "Installierte moderne CLIs, ihre Verwendung und optional ihre Version anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Consulta las CLI modernas instaladas, su uso y, si se solicita, su versión."
            ),
            locale(
                "hi-IN",
                summary = "इंस्टॉल किए गए आधुनिक CLI, उनके उपयोग और वैकल्पिक संस्करण देखें।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض أدوات CLI الحديثة المثبّتة واستخداماتها وإصداراتها عند الطلب."
            ),
            locale(
                "pt-BR",
                summary = "Consulte as CLIs modernas instaladas, seus usos e, se solicitado, as versões."
            )
        ),
        help = "Resolves tools from the Agena server PATH and workspace, including fd/fdfind and bat/batcat aliases. Does not install tools or read interactive shell startup files. Omit names for installed tools and a compact missing list; pass names to inspect specific tools. probe_versions runs bounded version commands only when 1–8 names are supplied. refresh bypasses the 15-second availability cache. Presence does not establish plugin/model dependencies or authorize execution."
    )]
    async fn executables(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ExecutablesInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if input.probe_versions && (input.names.is_empty() || input.names.len() > 8) {
            return Err(PluginError::invalid_params(
                "probe_versions requires 1–8 names",
            ));
        }
        let names = input.names.iter().map(|name| {
            agena_tool::cli_catalog::find(name).map(|spec| spec.name)
                .ok_or_else(|| PluginError::invalid_params(format!("Unknown executable capability '{name}'; omit names to inspect the catalog")))
        }).collect::<SdkResult<std::collections::BTreeSet<_>>>()?;
        let mut inventory = agena_runtime_tools::cli_tools::discover(
            Path::new(context.workspace_root),
            input.refresh,
        )
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
        let missing = inventory
            .tools
            .iter()
            .filter(|tool| {
                !tool.available && (names.is_empty() || names.contains(tool.name.as_str()))
            })
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        inventory.tools.retain(|tool| {
            if names.is_empty() {
                tool.available
            } else {
                names.contains(tool.name.as_str())
            }
        });
        if input.probe_versions {
            futures_util::future::join_all(inventory.tools.iter_mut().map(|tool| {
                agena_runtime_tools::cli_tools::probe_version(
                    tool,
                    Path::new(context.workspace_root),
                )
            }))
            .await;
        }
        let available = inventory.tools.iter().filter(|tool| tool.available).count();
        let mut lines = inventory
            .tools
            .iter()
            .map(|tool| {
                format!(
                    "{}: {}{} — {}\n{}",
                    tool.name,
                    tool.executable
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "unavailable".into()),
                    tool.version
                        .as_ref()
                        .map(|version| format!(" ({version})"))
                        .unwrap_or_default(),
                    tool.purpose,
                    tool.guidance
                )
            })
            .collect::<Vec<_>>();
        for tool in &inventory.tools {
            if let Some(error) = &tool.probe_error {
                lines.push(format!("{}: {error}", tool.name));
            }
        }
        if !missing.is_empty() {
            lines.push(format!("Unavailable: {}", missing.join(", ")));
        }
        Ok(ToolInvokeOutput::from_parts(
            "host executables",
            format!("{available} available"),
            lines.join("\n\n"),
            Some(
                serde_json::json!({"tools": inventory.tools, "missing": missing,
                "checked_at_unix_ms": inventory.checked_at_unix_ms, "cache_age_ms": inventory.cache_age_ms}),
            ),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    async fn execution_snapshot(
        &self,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<HostContextStatusResponse> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("session plugin invoked before init"))?
            .get_context_status(HostContextStatusRequest {
                session_id: Some(context.session_id),
            })
            .await
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Inspect the current session model identity, runtime modes, and model token limits.",
        translations(
            locale("zh-CN", summary = "查看当前会话的模型身份、运行模式和模型令牌上限。"),
            locale(
                "zh-TW",
                summary = "檢視目前工作階段的模型身分、執行模式與模型 Token 上限。"
            ),
            locale(
                "ja-JP",
                summary = "現在のセッションで使うモデル、実行モード、トークン上限を確認します。"
            ),
            locale(
                "ko-KR",
                summary = "현재 세션의 모델, 실행 모드 및 토큰 한도를 확인합니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter le modèle de la session, ses modes d’exécution et ses limites de jetons."
            ),
            locale(
                "de-DE",
                summary = "Sitzungsmodell, Laufzeitmodi und Tokenlimits des Modells anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Consulta el modelo de la sesión, sus modos de ejecución y los límites de tokens."
            ),
            locale("hi-IN", summary = "मौजूदा सत्र का मॉडल, रनटाइम मोड और टोकन सीमा देखें।"),
            locale(
                "ar-SA",
                summary = "اعرض نموذج الجلسة الحالية وأوضاع التشغيل وحدود الرموز."
            ),
            locale(
                "pt-BR",
                summary = "Consulte o modelo da sessão, os modos de execução e os limites de tokens."
            )
        )
    )]
    async fn model(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let status = self.execution_snapshot(context).await?;
        let model_identity = match (
            status.model_provider_id.as_deref(),
            status.model_adapter_id.as_deref(),
            status.model_id.as_deref(),
        ) {
            (Some(provider), Some(adapter), Some(model)) => {
                format!("{provider}/{adapter}/{model}")
            }
            (Some(provider), None, Some(model)) => format!("{provider}/{model}"),
            _ => "unknown".to_string(),
        };
        let mut model_detail = vec![format!("Model: {model_identity}")];
        if let Some(thinking) = status.thinking_mode.as_deref() {
            model_detail.push(format!("thinking: {thinking}"));
        }
        if let Some(speed) = status.speed_mode.as_deref() {
            model_detail.push(format!("speed: {speed}"));
        }
        if let Some(verbosity) = status.verbosity.as_deref() {
            model_detail.push(format!("verbosity: {verbosity}"));
        }
        let payload = serde_json::json!({
            "session_id": status.session_id,
            "model_provider_id": status.model_provider_id,
            "model_adapter_id": status.model_adapter_id,
            "model_id": status.model_id,
            "thinking_mode": status.thinking_mode,
            "speed_mode": status.speed_mode,
            "verbosity": status.verbosity,
            "model_context_window_tokens": status.model_context_window_tokens,
            "model_max_input_tokens": status.model_max_input_tokens,
            "model_max_output_tokens": status.model_max_output_tokens,
        });
        Ok(ToolInvokeOutput::from_parts(
            "session model",
            model_identity,
            model_detail.join("; "),
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(query, discovery, read_only),
        summary = "Inspect current and projected token use, effective limits, and remaining session budget.",
        translations(
            locale(
                "zh-CN",
                summary = "查看当前和预计的令牌用量、有效上限及会话剩余预算。"
            ),
            locale(
                "zh-TW",
                summary = "檢視目前與預估的 Token 用量、有效上限及工作階段剩餘預算。"
            ),
            locale(
                "ja-JP",
                summary = "現在と予測のトークン使用量、有効な上限、セッションの残り予算を確認します。"
            ),
            locale(
                "ko-KR",
                summary = "현재 및 예상 토큰 사용량, 적용 한도와 남은 세션 예산을 확인합니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter l’usage actuel et prévu des jetons, les limites effectives et le budget restant."
            ),
            locale(
                "de-DE",
                summary = "Aktuellen und prognostizierten Tokenverbrauch, gültige Limits und verbleibendes Sitzungsbudget anzeigen."
            ),
            locale(
                "es-ES",
                summary = "Consulta el uso actual y previsto de tokens, los límites efectivos y el presupuesto restante."
            ),
            locale(
                "hi-IN",
                summary = "मौजूदा और अनुमानित टोकन उपयोग, लागू सीमाएँ और बचा हुआ सत्र बजट देखें।"
            ),
            locale(
                "ar-SA",
                summary = "اعرض استهلاك الرموز الحالي والمتوقع والحدود الفعلية والميزانية المتبقية للجلسة."
            ),
            locale(
                "pt-BR",
                summary = "Consulte o uso atual e previsto de tokens, os limites efetivos e o orçamento restante da sessão."
            )
        )
    )]
    async fn tokens(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let status = self.execution_snapshot(context).await?;
        let ratio = status
            .limit_tokens
            .and_then(|limit| (limit > 0).then_some(status.current_tokens as f64 / limit as f64));
        let payload = serde_json::json!({
            "session_id": status.session_id,
            "current_tokens": status.current_tokens,
            "input_limit_tokens": status.input_limit_tokens,
            "measured_prompt_tokens": status.measured_prompt_tokens,
            "projected_tokens": status.projected_tokens,
            "limit_tokens": status.limit_tokens,
            "remaining_tokens": status.remaining_tokens,
            "usage_ratio": ratio,
            "reserved_tokens": status.reserved_tokens,
        });
        let text = format!(
            "Tokens: {} used; measured {}; projected {}; limit {}; remaining {}; reserved {}.",
            status.current_tokens,
            status
                .measured_prompt_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .projected_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .limit_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .remaining_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status.reserved_tokens,
        );
        Ok(ToolInvokeOutput::from_parts(
            "session tokens",
            status.remaining_tokens.map_or_else(
                || format!("{} tokens used", status.current_tokens),
                |remaining| format!("{} used · {remaining} remaining", status.current_tokens),
            ),
            text,
            Some(payload),
            std::collections::BTreeMap::from([
                (
                    "current_tokens".to_string(),
                    status.current_tokens.to_string(),
                ),
                (
                    "remaining_tokens".to_string(),
                    status
                        .remaining_tokens
                        .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
                ),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        tags(mutate),
        summary = "Rename the current session.",
        translations(
            locale("zh-CN", summary = "重命名当前会话。"),
            locale("zh-TW", summary = "重新命名目前工作階段。"),
            locale("ja-JP", summary = "現在のセッション名を変更します。"),
            locale("ko-KR", summary = "현재 세션의 이름을 바꿉니다."),
            locale("fr-FR", summary = "Renommer la session actuelle."),
            locale("de-DE", summary = "Die aktuelle Sitzung umbenennen."),
            locale("es-ES", summary = "Cambia el nombre de la sesión actual."),
            locale("hi-IN", summary = "मौजूदा सत्र का नाम बदलें।"),
            locale("ar-SA", summary = "غيّر اسم الجلسة الحالية."),
            locale("pt-BR", summary = "Renomeie a sessão atual.")
        )
    )]
    async fn rename(&self, input: &SessionRenameToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_rename_session(input).await
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::SessionPlugin;

    #[test]
    fn manifest_contains_split_session_tools() {
        let manifest = SessionPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "session");
        assert_eq!(
            tool_names,
            [
                "get",
                "environment",
                "executables",
                "model",
                "tokens",
                "rename"
            ]
        );
    }
}

#[cfg(test)]
mod audit_fact_tests {
    #[tokio::test]
    async fn executable_query_resolves_aliases_without_version_processes() {
        let directory = tempfile::tempdir().unwrap();
        let context = agena_plugin_host::sdk::ToolInvokeContext {
            tool_name: "executables",
            session_id: 41,
            call_id: 1,
            workspace_root: directory.path().to_str().unwrap(),
        };
        let input = super::ExecutablesInput {
            names: vec!["fdfind".into(), "fd".into()],
            ..Default::default()
        };
        let payload = super::SessionPlugin::new()
            .executables(&context, &input)
            .await
            .unwrap()
            .payload
            .unwrap();
        assert_eq!(payload["tools"].as_array().unwrap().len(), 1);
        assert_eq!(payload["tools"][0]["name"], "fd");
        assert!(payload["tools"][0].get("version").is_none());
        for invalid in [
            super::ExecutablesInput {
                probe_versions: true,
                ..Default::default()
            },
            super::ExecutablesInput {
                names: vec!["not-a-curated-command".into()],
                ..Default::default()
            },
        ] {
            assert!(
                super::SessionPlugin::new()
                    .executables(&context, &invalid)
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn actual_git_snapshot_handles_unborn_and_untracked_workspaces() {
        let directory = tempfile::tempdir().unwrap();
        let result = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        let clean = super::git_facts(directory.path()).unwrap().unwrap();
        assert_eq!(clean.dirty, Some(false));
        assert!(clean.short_sha.is_none());
        std::fs::write(directory.path().join("untracked file.txt"), "content").unwrap();
        assert_eq!(
            super::git_facts(directory.path()).unwrap().unwrap().dirty,
            Some(true)
        );
    }

    #[test]
    fn porcelain_snapshot_preserves_unknown_unborn_and_rename_facts() {
        assert!(super::parse_git_facts("").is_none());
        let unborn =
            super::parse_git_facts("# branch.oid (initial)\0# branch.head main\0").unwrap();
        assert_eq!(unborn.dirty, Some(false));
        assert!(unborn.short_sha.is_none());
        let dirty = super::parse_git_facts(
            "# branch.oid 0123456789abcdef\0# branch.head main\0? new file\0",
        )
        .unwrap();
        assert_eq!(dirty.short_sha.as_deref(), Some("01234567"));
        assert_eq!(dirty.dirty, Some(true));
        let renamed = super::parse_git_facts("# branch.oid 0123456789abcdef\0# branch.head (detached)\x002 R. fields\0# branch.oid deadbeefdeadbeef\0").unwrap();
        assert_eq!(renamed.short_sha.as_deref(), Some("01234567"));
        assert_eq!(renamed.branch.as_deref(), Some("HEAD"));
    }

    #[tokio::test]
    async fn nongit_workspace_has_explicit_unknown_git_status() {
        let directory = tempfile::tempdir().unwrap();
        let context = agena_plugin_host::sdk::ToolInvokeContext {
            tool_name: "environment",
            session_id: 41,
            call_id: 1,
            workspace_root: directory.path().to_str().unwrap(),
        };
        let output = super::SessionPlugin::new()
            .environment(&context)
            .await
            .unwrap()
            .payload
            .unwrap();
        assert!(output["git_dirty"].is_null());
        assert_eq!(output["git_status_known"], false);
    }
}
