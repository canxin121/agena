//! Unified, provider-wide model discovery exposed to the calling AI.

use std::sync::{Arc, OnceLock};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const PROVIDER_PLUGIN_ID: &str = "agena.provider";

pub(crate) struct ProviderPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ModelsAction {
    #[default]
    List,
    Get,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("query", "id", "adapter_id", "cursor"),
    max_chars("query", 256),
    max_chars("id", 512),
    max_chars("cursor", 256)
)]
#[serde(deny_unknown_fields)]
struct ModelsInput {
    /// list returns compact model routes; get returns full metadata for id.
    #[serde(default)]
    action: ModelsAction,
    /// Optional case-insensitive text search over IDs and existing metadata.
    /// This is a text filter, not a model-type classifier. list only.
    #[serde(default)]
    query: Option<String>,
    /// Exact model ID for get. Use adapter_id when the ID has multiple routes.
    #[serde(default)]
    id: Option<String>,
    /// Optional adapter within the current provider. Omit to query all enabled adapters.
    #[serde(default)]
    adapter_id: Option<String>,
    /// Maximum list entries per page, 1–200. Defaults to 100.
    #[serde(default = "default_limit")]
    #[schemars(range(min = 1, max = 200))]
    limit: usize,
    /// Opaque next_cursor from list; keep the same query and adapter_id.
    /// If the provider's model list changes, restart without a cursor.
    #[serde(default)]
    cursor: Option<String>,
}

fn default_limit() -> usize {
    100
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena", name = "provider", version = env!("CARGO_PKG_VERSION"),
    summary = "Discover model routes across all enabled adapters of the current provider.",
    translations(
        locale("zh-CN", summary = "查询当前 provider 全部已启用 adapter 的模型及来源。"),
        locale("zh-TW", summary = "查詢目前 provider 全部已啟用 adapter 的模型與來源。")
    )
)]
impl ProviderPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("provider plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        name = "models",
        tags(query, discovery, network),
        summary = "List or inspect model routes across all enabled adapters of the current provider, including discovery sources and failures.",
        help = "Use this to choose a model before calling chatgpt, claude, gemini or other provider tools. The current session identifies the provider; its selected chat model does not restrict the results. action=list returns compact routes across all enabled adapters, with optional query text search, adapter_id, limit (1–200, default 100), and cursor pagination. Continue with next_cursor and the same filters; restart if the list changes. action=get requires an exact id and returns its full metadata; specify adapter_id if the ID occurs on multiple adapters. Provider API pagination is followed automatically. Models include source=provider_discovery or configured; adapters reports discovery status and safe warnings. complete=false means some remote discoveries failed; has_more refers only to this tool's output pages. Configured-only adapters retain that policy. Catalog data enriches returned routes without adding catalog-only models. No model type is inferred for filtering. Reuses the current provider's connection and checks network permission; no model, API key or endpoint input is required. A listed model does not guarantee support for a specific API or account operation.",
        translations(
            locale(
                "zh-CN",
                summary = "列出或查看当前 provider 全部已启用 adapter 的模型，包含来源及查询失败信息。",
                help = "调用厂商工具选择模型前先使用本工具。当前会话仅用于确定 provider，当前选中的聊天模型不会限制返回列表。action=list 返回精简模型路由，可用 query 文本搜索、adapter_id 缩小范围，limit 为 1–200（默认 100）。用 next_cursor 和相同筛选条件继续；列表变化时从第一页重新查询。action=get 需要精确的 id，返回完整元数据；同一 ID 有多个路由时传 adapter_id。工具自动读取远程模型接口的分页。每个模型标明 source（provider_discovery 或 configured）；adapters 提供各 adapter 的查询状态及提示，complete=false 表示部分远程查询失败，has_more 仅表示本工具还有输出页。遵守仅查询配置模型的 adapter 设置。catalog 用于补充已有路由的说明，不追加仅在 catalog 中出现的模型，也不推断模型类型来筛选。复用当前 provider 的连接并检查网络权限，无需传 model、密钥或地址。模型出现在列表中不保证账号支持某项具体 API。"
            ),
            locale(
                "zh-TW",
                summary = "列出或查看目前 provider 全部已啟用 adapter 的模型，包含來源與查詢失敗資訊。",
                help = "呼叫廠商工具選擇模型前先使用本工具。工作階段僅用於確定 provider，選中的聊天模型不限制結果。action=list 回傳精簡路由，支援 query 文字搜尋、adapter_id、limit（1–200，預設 100）及 cursor；使用 next_cursor 與相同條件續查，列表變更時重新開始。action=get 需要 id 並回傳完整資料，同一 ID 有多個路由時指定 adapter_id。自動讀取遠端分頁。source 區分 provider_discovery 與 configured；adapters 提供各 adapter 狀態，complete=false 表示部分遠端查詢失敗，has_more 表示仍有輸出頁。遵守僅使用設定模型的 adapter 政策。catalog 僅補充已有路由的資訊，不追加 catalog 專有模型，也不推斷類型來篩選。沿用目前 provider 連線並檢查網路權限，無需提供 model、金鑰或位址。列表不保證特定 API 或帳號權限。"
            )
        )
    )]
    async fn models(&self, input: ModelsInput) -> SdkResult<ToolInvokeOutput> {
        if !(1..=200).contains(&input.limit) {
            return Err(PluginError::invalid_params("limit must be 1–200"));
        }
        match input.action {
            ModelsAction::List if input.id.is_some() => {
                return Err(PluginError::invalid_params(
                    "id applies only to get; use query for list",
                ));
            }
            ModelsAction::Get if input.id.as_deref().is_none_or(str::is_empty) => {
                return Err(PluginError::invalid_params("id is required for get"));
            }
            ModelsAction::Get if input.query.is_some() || input.cursor.is_some() => {
                return Err(PluginError::invalid_params(
                    "query and cursor apply only to list",
                ));
            }
            _ => {}
        }
        let host = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("provider host is not initialized"))?;
        let catalog = project_models(host.builtin_provider_models().await?, &input)?;
        Ok(ToolInvokeOutput::from_parts(
            "Provider models",
            if matches!(input.action, ModelsAction::Get) {
                "Model details".to_owned()
            } else {
                format!("{} of {} model routes", catalog["count"], catalog["total"])
            },
            serde_json::to_string_pretty(&catalog)
                .map_err(|error| PluginError::internal_error(&error))?,
            Some(catalog),
            Default::default(),
            Vec::new(),
        ))
    }
}

fn project_models(mut catalog: Value, input: &ModelsInput) -> SdkResult<Value> {
    if let Some(adapter_id) = &input.adapter_id {
        let known = catalog["adapters"].as_array().is_some_and(|adapters| {
            adapters
                .iter()
                .any(|adapter| adapter["adapter_id"].as_str() == Some(adapter_id.as_str()))
        });
        if !known {
            return Err(PluginError::invalid_params(
                "adapter_id is not an enabled adapter of the current provider",
            ));
        }
    }
    let mut models = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .map(std::mem::take)
        .ok_or_else(|| PluginError::internal("provider discovery returned no model list"))?;
    let query = input.query.as_deref().unwrap_or_default().to_lowercase();
    models.retain(|model| {
        input
            .adapter_id
            .as_ref()
            .is_none_or(|id| model["adapter_id"].as_str() == Some(id.as_str()))
            && (query.is_empty() || model.to_string().to_lowercase().contains(&query))
    });
    models.sort_by(|a, b| {
        (a["adapter_id"].as_str(), a["id"].as_str())
            .cmp(&(b["adapter_id"].as_str(), b["id"].as_str()))
    });
    if matches!(input.action, ModelsAction::Get) {
        let id = input
            .id
            .as_deref()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| PluginError::invalid_params("id is required for get"))?;
        let matching = models
            .into_iter()
            .filter(|model| model["id"].as_str() == Some(id))
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(PluginError::invalid_params(
                "model ID has multiple routes; specify adapter_id",
            ));
        }
        catalog
            .as_object_mut()
            .expect("discovery is an object")
            .remove("models");
        catalog["model"] = matching.into_iter().next().ok_or_else(|| {
            PluginError::invalid_params(
                "model ID was not found in this provider's discovered or configured routes",
            )
        })?;
        return Ok(catalog);
    }
    let identity = serde_json::to_vec(&json!([
        catalog["provider_id"],
        input.adapter_id,
        query,
        models
    ]))
    .map_err(|error| PluginError::internal_error(&error))?;
    let fingerprint = hex::encode(Sha256::digest(identity));
    let offset = if let Some(cursor) = &input.cursor {
        let (snapshot, offset) = cursor
            .split_once(':')
            .ok_or_else(|| PluginError::invalid_params("invalid model cursor"))?;
        if snapshot != fingerprint {
            return Err(PluginError::invalid_params(
                "model list or filters changed; restart without cursor",
            ));
        }
        offset
            .parse::<usize>()
            .map_err(|_| PluginError::invalid_params("invalid model cursor offset"))?
    } else {
        0
    };
    if offset > models.len() {
        return Err(PluginError::invalid_params(
            "model cursor is beyond the result list",
        ));
    }
    let end = offset.saturating_add(input.limit).min(models.len());
    catalog["total"] = json!(models.len());
    catalog["count"] = json!(end - offset);
    catalog["has_more"] = json!(end < models.len());
    catalog["next_cursor"] = if end < models.len() {
        json!(format!("{fingerprint}:{end}"))
    } else {
        Value::Null
    };
    catalog["models"] = json!(
        models[offset..end]
            .iter()
            .map(|model| json!({
                "id":model["id"], "adapter_id":model["adapter_id"],
                "display_name":model["display_name"], "source":model["source"],
                "description":model["metadata"]["description"],
            }))
            .collect::<Vec<_>>()
    );
    Ok(catalog)
}
