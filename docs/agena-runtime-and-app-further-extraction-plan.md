# Agena Runtime、TUI App 与 Studio 下一轮一次性拆分执行计划

> 状态：已完成可执行性审查，待执行
>
> 规划日期：2026-07-24
>
> 审查日期：2026-07-24；已针对真实 module 引用、public API 消费方、feature 转发、Cargo.lock 更新、TUI residual owner 和 Studio Git/Axum 耦合完成复核
>
> 代码基线：当前 master / agent/agena-tui-app-further-extraction 的 01051f55216b
>
> 前置计划：[agena-tui-app-further-extraction-plan.md](agena-tui-app-further-extraction-plan.md)
>
> 事实来源：[docs/rust-workspace-analysis.md](rust-workspace-analysis.md)、cargo metadata --format-version 1 --locked、当前 workspace 源码以及前置 TUI 提取计划。
>
> 执行模式：一次连续的源码迁移列车。先一次性创建目标 crate、直接 git mv 文件和目录、连续完成 module/path/API/visibility/manifest 改写；在所有移动和静态收口完成之前不运行 cargo check、cargo test 或 Clippy。第一次编译只在源码所有权和依赖方向已经完整落地之后执行，然后按错误类别批量修复，最后统一完成 workspace 验证。

本计划解决前置计划完成后剩余的三类结构问题：

1. agena-runtime 仍然是一个 104,714 行、307 文件、413 模块的系统级巨型 composition crate；provider、session、plugins、tool 和 config 五个领域仍集中在同一个 crate。
2. agena-tui-app 虽然已经从 60,218 行降到 39,036 行，但 provider/permission/settings/session 的 concrete adapter、route/overlay 组合和异步 runtime 映射仍然集中在 app。
3. agena-studio-server 有 21,212 行，其中 Git/workspace 操作占据主要体量；Git API 目前仍通过 binary package 的 root module 暴露。

本轮的目标不是按目录名制造更多 crate，而是把稳定的契约、纯实现、组合层和外部 adapter 分开，并且让每个新 crate 都能在没有 agena-tui-app 或 agena-runtime 反向依赖的情况下独立编译和测试。

## 0. 审查结论与风险等级

本计划经源码级复核后评定为：**高风险但可执行，满足条件后可以进入一次连续迁移列车**。高风险来自 104k 行 runtime、七个新 crate、稳定 public facade 和 Studio/TUI 同时受影响，而不是来自目标方向不清晰。

执行许可依赖以下前提：

- 必须在独立 branch/worktree 中执行，工作树只包含本轮报告、计划和迁移改动；
- Train 0 的 public facade allowlist、self-alias 分类和新 dependency DAG 三项清单必须真实落盘；
- “一次性”表示所有 ownership 移动完成后才第一次 check，不表示放弃 Git checkpoint、diff review 或中途静态审计；
- Train 1/2/3/4/5/6 可以分别形成可回溯的 checkpoint commit，即使中间 commit 尚不可编译；
- 任一新 crate 需要依赖 parent runtime/app、package graph 出现 cycle、或 contracts 预计超过约 12k 行且开始吸收 implementation 时，必须在第一次 check 前调整边界。

本次审查已修正原草案中的四个关键问题：config/registry 不是纯 config owner；runtime public API 需要显式稳定 facade；Studio Git 是 Axum HTTP vertical slice；新 package graph 写入后需要一次受控 Cargo.lock 更新。执行者不得恢复这些已否决的旧假设。

## 1. 本轮结论与目标

### 1.1 当前量化事实

最新架构报告覆盖：

| 指标 | 当前值 |
| --- | ---: |
| Workspace package | 47 |
| Rust target | 52 |
| 第一方 .rs 文件 | 1,062 |
| Rust 源码行数 | 334,737 |
| 模块解析错误 | 0 |
| 词法结构告警 | 0 |
| agena-runtime | 104,714 行 / 307 文件 / 413 模块 / 862 引用边 |
| agena-tui-app | 39,036 行 / 105 文件 / 140 模块 / 214 引用边 |
| agena-studio-server | 21,212 行 / 61 文件 / 61 模块 / 105 引用边 |

agena-runtime 的主要体量分布如下：

| Runtime 区域 | 行数 | 文件数 | 本轮归属 |
| --- | ---: | ---: | --- |
| provider | 26,256 | 42 | agena-runtime-provider |
| session | 20,863 | 37 | agena-runtime-session |
| plugins | 9,725 | 28 | agena-runtime-plugins |
| tool | 8,129 | 31 | agena-runtime-tools |
| config | 7,670 | 13 | agena-runtime-config |
| runtime composition/root services | 其余 | — | 留在 agena-runtime |

provider 和 session 合计约 47,119 行，占 agena-runtime 的约 45%。但当前静态引用边显示它们不是完全孤立的目录：

- tool -> plugins：32 条；
- session -> message：26 条；
- tool -> message：22 条；
- provider -> error：22 条；
- plugins -> tool：14 条；
- session -> event：12 条；
- session -> db：8 条；
- session -> provider：7 条。

因此本轮必须先建立一个受控的中性协议边界，再移动 implementation。不能把 provider、session、plugins、tool 原样搬走后让新 crate 互相依赖，更不能让任何新 runtime crate 依赖 agena-runtime。

### 1.2 本轮最终目标

完成后，目标结构应接近：

~~~text
apps/agena
  └── agena-runtime                         application entry/composition

agena-runtime
  ├── agena-runtime-contracts               中性跨域 DTO、协议、稳定 value types
  ├── agena-runtime-config                   config loading/registry/resolution
  ├── agena-runtime-provider                 concrete provider adapters/registry/transport
  ├── agena-runtime-tools                    builtin tools/executor/output
  ├── agena-runtime-plugins                  plugin runtime/provided plugins
  ├── agena-runtime-session                  session manager/history/processor
  ├── agena-storage / agena-storage-sqlite   persistence ports/adapters
  ├── agena-provider                         provider-facing contracts
  ├── agena-plugin-host / agena-plugin-sdk   plugin contracts/host
  └── agena-domain                           domain contracts

agena-tui-app
  ├── agena-tui-transcript                   transcript owner
  ├── agena-tui-plugin-workbench             schema/model owner
  ├── agena-tui-session                      session presentation/controller
  ├── agena-tui-provider-studio              provider presentation/state
  ├── agena-tui-permission-studio            permission presentation/state
  ├── agena-tui-settings                     settings presentation/state
  ├── agena-runtime                          runtime adapter/composition only
  ├── agena-tui-backend                      backend adapter
  └── agena-tui-platform/media               platform adapter

agena-studio-server
  └── agena-studio-git                       Git domain/service library
~~~

箭头方向必须保持为：

~~~text
application/binary
       │
       ▼
composition crate (agena-runtime, agena-tui-app, agena-studio-server)
       │
       ├── concrete feature/runtime crates
       │       │
       │       └── contracts/domain/ports
       │
       └── backend/platform/persistence adapters
~~~

### 1.3 本轮不改变的行为

本轮只改变源码所有权、模块路径、Cargo package 边界、可见性和内部 API，不改变：

- provider authentication、model catalog、provider selection、streaming 和 reload 语义；
- session create/submit/continue/compact/rewind/cancel 和 streaming refresh 语义；
- tool permission、tool output、builtin tool、plugin-provided tool 和 workflow 语义；
- plugin loading、RPC、provided plugin、schema lab、shutdown 和 persistence 语义；
- config 文件格式、环境变量、merge precedence、config path 和 reload 语义；
- TUI layout、keymap、文案、route、overlay、clipboard、editor、pager 和 terminal 行为；
- Studio Git API 的 route、request/response JSON、错误语义、Git 操作和安全策略；
- CLI 参数、binary 名称、持久化格式、网络安全边界和 feature 默认值。

任何业务语义变化必须另开任务，不能以“拆分顺便清理”为理由混入本轮。

## 2. 最终 crate 拓扑与边界规则

### 2.1 新增 crate

本轮一次性创建以下七个 library crate。所有 crate 都先创建 manifest 和空 root，再开始任何 git mv。

| 新 crate | Owner | 允许依赖 | 明确禁止 |
| --- | --- | --- | --- |
| agena-runtime-contracts | runtime 跨域稳定 DTO、协议、neutral event/action/error shape | agena-domain、agena-provider、agena-tool、plugin SDK/host 中的稳定 value types，以及必要的 serde/uuid/chrono/schemars derive | App、Backend、Runtime composition、数据库 I/O、route/overlay |
| agena-runtime-config | config raw model、loader、edit、resolution 和 config-facing service contract | contracts、domain、provider/plugin 的稳定配置类型 | concrete provider registry、plugin host、session execution、TUI、runtime composition |
| agena-runtime-provider | runtime provider adapter、auth、transport、registry、wire mapping | contracts、agena-provider、provider-specific crates、config contract | session manager、TUI app、plugin runtime、agena-runtime |
| agena-runtime-tools | builtin tool、tool executor、payload、output、tool registry | contracts、domain、agena-tool、plugin SDK/host ports | plugin lifecycle implementation、session manager、agena-runtime |
| agena-runtime-plugins | plugin runtime、provided plugin、plugin service、plugin lifecycle | contracts、tools contract、agena-plugin-host、agena-plugin-sdk、storage ports | app route、TUI、session manager、agena-runtime |
| agena-runtime-session | session model、history、store、processor、manager、execution protocol | contracts、config、provider、tools、plugins、storage、domain | TUI App、route/overlay、agena-runtime |
| agena-studio-git | Git HTTP vertical slice、Git operations、Git errors、Git request/response mapping | axum、tokio、git2、ignore、serde、path/utility crates | Studio AppState、Studio DB、agena-studio-server |

agena-runtime-contracts 是防止新 crate 产生循环依赖的窄 seam，不是新的“万能 common crate”。它必须保持小而稳定；如果某个类型只被一个 implementation crate 使用，就留在那个 crate，不要为了减少 import 把所有 runtime 类型都搬进去。

### 2.2 agena-runtime 最终保留内容

agena-runtime 仍然是 composition crate，但不再持有五个大型 implementation tree。它保留：

- application service composition；
- runtime bootstrap、shutdown、reload 和 lifecycle；
- Runtime/host client 的组装；
- event bus 的 concrete persistence/forwarding adapter；
- DB、storage、scheduler、MCP、LSP、web 的 composition；
- provider/session/tool/plugin/config crate 的 wiring；
- 对外稳定的 RuntimePresentationEvent 和 application-facing service facade；
- 将各子 crate 的 neutral event/effect 映射到现有 runtime envelope 的 adapter。

它不得重新引入已迁出的：

- src/provider/** 的 concrete provider implementation；
- src/session/** 的 session manager/history/processor implementation；
- src/plugins/** 的 provided plugin implementation；
- src/tool/** 的 builtin tool/executor implementation；
- src/config/** 的 config loader/registry implementation。

### 2.3 agena-runtime-contracts 的最小化规则

跨两个或以上新 crate 使用的类型才考虑放入 contracts。候选类别包括：

- session/execution/plugin/tool/provider 使用的稳定 ID、request context 和 cancellation metadata；
- provider output、tool execution result、plugin invocation result 的 neutral representation；
- session-to-provider、session-to-tool、plugin-to-tool 所需的 action/event shape；
- config resolution 结果中不含 runtime service 的稳定 snapshot；
- 跨 crate 的错误分类和可序列化错误 payload。

以下类型不得进入 contracts：

- App、AppMessage、Route、Overlay、Backend、TerminalRuntime；
- Runtime、RuntimeContext 中包含 concrete service/Arc registry 的结构；
- sea_orm entity、数据库 transaction、storage implementation；
- axum::extract、Axum response、TUI ratatui widget；
- UnboundedSender<AppMessage>、callback closure、全局 singleton；
- 仅为了让编译器通过而搬入的巨大 root re-export。

message tree 当前带有 sea_orm 的 JSON derive、agena-provider usage、agena-tool input 和 plugin attachment value type。它可以作为 runtime value model 进入 contracts，但不得把 database connection、entity、transaction 或查询逻辑一起带入。若某个 derive 只服务持久化，可在保持序列化行为不变的前提下留到后续任务清理；本轮不为了追求“纯”而改变存储格式。

### 2.4 `agena-runtime` 的稳定 public facade

当前至少九个第一方 package、三百余处源码引用通过 `agena_runtime::...` 使用 application-facing API，其中 `agena-application`、`agena-tui-backend`、`agena-api-server` 和 `agena-cli` 是主要消费者。全部强制改成直接依赖 implementation crate 会把 composition 细节泄漏到上层，并造成不必要的 manifest 扩散。

因此本轮允许并要求 `agena-runtime` 保留一份**显式 allowlist facade**。这不是临时 compatibility module，而是 composition crate 的稳定 application API：

- runtime bootstrap/result/application services；
- Runtime event/presentation/event query/publish DTO 和 service trait；
- Runtime config/settings/auth/status/control service trait 和 DTO；
- Session query/request/execution/control/tool-command service trait 和 DTO；
- Plugin runtime service trait 和 public catalog DTO；
- Runtime tool execution service trait和 DTO；
- reload、metrics、tracing 等 application-facing DTO。

允许形式：

~~~rust
pub use agena_runtime_session::{
    SessionQueryService,
    SessionExecutionRequest,
    SessionPresentation,
};
pub use agena_runtime_plugins::PluginRuntimeService;
pub use agena_runtime_config::RuntimeConfigSettingsService;
~~~

禁止 re-export：

- `SessionManager`、`SessionProcessor`、`ProviderRegistry`、`ToolExecutor`；
- config raw/loader internals；
- provided plugin implementation module；
- private adapter、global slot、concrete store 和 runtime builder internals；
- `pub use owner_crate::*` 一类全量 wildcard facade。

上层 consumer 默认继续使用稳定的 `agena_runtime::...` facade；只有真正需要直接组合 implementation 的 crate 才新增 owner crate dependency。最终必须维护一张 facade allowlist，并用 `rg` 确认所有 re-export 都有第一方 consumer 或明确的公共 API 理由。

### 2.5 Error、self-alias 与跨 crate 转换

当前 `AppError` 同时包含 config、provider、HTTP、database、storage、session conflict 和 cancellation；它还通过 `extern crate self as agena_runtime` 引用本 crate 类型。这个 error 不能整体移入 contracts，也不能成为所有新 crate 的共同依赖。

错误归属必须调整为：

```text
agena-runtime-config    -> ConfigError
agena-runtime-provider  -> RuntimeProviderError
agena-runtime-tools     -> ToolError
agena-runtime-plugins   -> PluginRuntimeError
agena-runtime-session   -> SessionError
agena-runtime           -> AppError / facade error adapter
```

implementation crate 返回自己的 typed error；`agena-runtime` composition/facade 在边界统一转换为现有 application-facing error，保持错误分类和用户可见文本。不得把 `reqwest::Error`、`sea_orm::DbErr` 或 concrete provider error塞进 contracts 的万能 enum。

当前 runtime 内部大量使用 `agena_runtime::...` self-alias。移动前除了检查 `crate::`，还必须执行：

~~~bash
rg -n 'extern crate self as agena_runtime|agena_runtime::' crates/agena-runtime/src
~~~

每个命中都要改成新 owner crate 路径、contracts 路径或显式注入的 port；新 runtime feature crate 中不得通过依赖 `agena-runtime` 恢复这些 self-alias 路径。

## 3. TUI app 的收口目标

前置计划已经完成 transcript renderer 和 plugin schema workbench 的 owner 迁移。本轮不新建重复 crate，而是完成已有五个 feature crate 的剩余纯 owner 和 adapter 边界。

### 3.1 agena-tui-provider-studio

当前 package 只有约 124 行，实际 feature 仍分布在 app：

- provider_studio/**：约 2,739 行；
- app_provider_runtime/**：约 1,667 行；
- app_provider_text.rs：约 528 行。

本轮将纯 provider presentation/state/selection/field/catalog projection 移入已有 crate；保留在 app：

- backend load/save；
- auth polling 和 external authentication；
- runtime reload；
- route/overlay mutation；
- AppMessage dispatch；
- terminal/platform side effect。

静态复核后的直接移动优先级：provider_studio/provider_fields.rs、provider_auth/fields.rs、provider_auth/summary.rs 的 app coupling 很低，是首选真实 git mv owner；provider_selection.rs 需要先拆掉少量 app root type。app_provider_runtime/catalog.rs 有大量 backend/tx/route/AppMessage 命中，明确留在 app adapter，不作为整文件移动候选。

### 3.2 agena-tui-permission-studio

纯 rule model、scope editor、validation、summary 和 renderer 移入已有 crate；保留在 app：

- live permission prompt；
- session execution reply；
- backend persistence；
- path browser 的 filesystem adapter；
- route/overlay 和 global flash message。

当前 app 中相关 owner 约为：

- app_permission_helpers/**：约 1,846 行；
- app_permissions/**：约 1,245 行；
- app_permission_display.rs：约 892 行；
- app_permission_studio.rs：约 781 行。

静态复核确认 app_permission_helpers/rules.rs、editor.rs、summary.rs 大部分是纯 helper，可在收紧输入类型后直接移动；app_permissions/**、path browser、overlay handlers 和 app_permission_studio.rs 继续作为 app adapter，不设“必须搬完整目录”的目标。

### 3.3 agena-tui-settings

纯 field schema、choice model、navigation、render model 和 validation 移入已有 crate；保留在 app：

- runtime snapshot/load/save；
- provider/agent concrete query；
- runtime reload；
- backend persistence；
- route/overlay 和 global settings event。

当前 app 中相关 owner 约为：

- app_settings_choices/**：约 1,592 行；
- app_settings_helpers/**：约 1,546 行；
- app_settings.rs：约 300 行级别。

app_settings_helpers/agents.rs、fields.rs、render.rs 的 app coupling 较低，是纯 presentation/helper 的主要移动候选。app_settings_choices/fields.rs、navigation.rs、provider.rs、session.rs 全部是高耦合 impl App，直接读取 backend/i18n/route；它们默认保留为 app adapter，只把能够形成独立输入/输出的 pure projection 拆入 settings crate。

### 3.4 agena-tui-session

将 app 的 session command/event/input/interactive 中纯 controller、presentation snapshot 和 neutral effect 继续归入已有 agena-tui-session；保留在 app：

- runtime request dispatch；
- backend session list/load/refresh；
- composer、route/overlay 组合；
- transcript owner 的组合；
- terminal/editor/clipboard side effect。

当前 app 中主要残留：

- app_session_events/**：约 2,050 行；
- app_session_interactive/**：约 1,417 行；
- app_session_helpers.rs、app_session_input.rs。

复核后确认 app_session_events/dispatch.rs、handlers.rs、interactive.rs、requests.rs 都是高耦合 impl App adapter，直接操作 transcript、route、backend、AppMessage 和 flash。它们不应整文件移动；本轮只允许将新识别出的 neutral reducer/value mapper 抽成独立 module 后移动，四个 adapter 文件本身继续留在 app。agena-tui-session 的完成标准是协议和纯 controller 被消费，不是为了减行数强行搬 adapter。

### 3.5 app shell 最终约束

完成后，agena-tui-app 可以依赖所有 feature crate 和 runtime adapter，但任何 feature crate 都不能依赖 agena-tui-app。Feature crate 公共入口只能接受：

~~~text
snapshot/state + action + neutral event
~~~

并返回：

~~~text
new state + presentation model + effect list
~~~

不得把以下签名暴露到 feature crate 公共 API：

~~~rust
&mut App
&App
Backend
TerminalRuntime
UnboundedSender<AppMessage>
Route
Overlay
~~~

## 4. Runtime 五个 implementation crate 的迁移范围

### 4.1 agena-runtime-config

复核后确认，当前 `src/config` 不是一个纯 config tree：`adapter_models.rs`、`credential_store.rs` 和 `registry/**` 直接创建 ProviderRegistry、provider adapter、AuthStore，并调用 plugin host。因此禁止把整个目录一次 `git mv` 到 config crate。

第一组可以直接移动的 config owner：

~~~text
crates/agena-runtime/src/config_environment.rs
crates/agena-runtime/src/config_error.rs
crates/agena-runtime/src/config_override.rs
crates/agena-runtime/src/config_paths.rs
crates/agena-runtime/src/config_values.rs
crates/agena-runtime/src/runtime_config_settings_service.rs
crates/agena-runtime/src/runtime_configuration_service.rs
~~~

第二组在切断 composition hook 后移动：

~~~text
crates/agena-runtime/src/config/mod.rs                    -> agena-runtime-config/src/config.rs
crates/agena-runtime/src/config/edit.rs                   -> agena-runtime-config/src/config/edit.rs
crates/agena-runtime/src/config/loader.rs                 -> agena-runtime-config/src/config/loader.rs
crates/agena-runtime/src/config/overrides.rs              -> agena-runtime-config/src/config/overrides.rs
crates/agena-runtime/src/config/raw.rs                    -> agena-runtime-config/src/config/raw.rs
crates/agena-runtime/src/config/raw/                      -> agena-runtime-config/src/config/raw/
~~~

`raw.rs` 当前直接调用 bundled plugin entries、MCP config 和 agent permission validator。移动前定义窄的 `ConfigResolutionHooks`/输入值，由 `agena-runtime` composition 提供 bundled plugin config、MCP projection 和 permission validation；config crate 只做 parse/merge/normalize/validate，不反向依赖 plugin/provider/session implementation。

下列文件不是 config owner，随 provider 拆分或留在 composition：

| 当前文件 | 正确 owner |
| --- | --- |
| `config/adapter_models.rs` | `agena-runtime-provider::config_support` |
| `config/credential_store.rs` | `agena-runtime-provider::config_support` |
| `config/registry.rs` 中 provider builder | `agena-runtime-provider::config_support` |
| `config/registry/auth_resolution.rs` | `agena-runtime-provider::config_support` |
| `config/registry/model_listing.rs` | `agena-runtime-provider::config_support` |
| `config/registry/provider_registry.rs` | `agena-runtime-provider::config_support` |
| `config/registry/plugin_host.rs` | `agena-runtime` 的 provider/plugin composition adapter |

保留在 agena-runtime 的 runtime-facing adapter：

~~~text
runtime_composition_config.rs
bootstrap_request.rs
bootstrap_result.rs
composition.rs
config/registry/plugin_host.rs 拆出的 composition adapter
~~~

如果 config_error.rs 中存在只属于 runtime service 的转换函数，则先把纯 error/value 部分随文件移动，再把 runtime adapter 部分留在 agena-runtime；不允许通过 pub use 把整个旧文件重新暴露回 runtime。

目标 API 方向：

~~~rust
pub struct ConfigLoadRequest;
pub struct ConfigResolution;
pub struct ResolvedConfig;
pub trait ConfigResolutionHooks: Send + Sync {}
pub enum ConfigSource;
pub enum ConfigError;

pub fn load_config(request: ConfigLoadRequest) -> Result<ResolvedConfig, ConfigError>;
pub fn resolve_config(input: ConfigResolutionInput) -> Result<ResolvedConfig, ConfigError>;
~~~

config crate 不负责启动 runtime、不创建 provider client/registry、不调用 plugin host、不触发 session execution、不发送 UI event。`RuntimeConfigSettingsService` 和 `RuntimeConfigurationService` 的 trait/DTO 可以由 runtime 显式 re-export，但实现 owner 归 config crate。

### 4.2 agena-runtime-provider

直接移动：

~~~text
crates/agena-runtime/src/provider/
crates/agena-runtime/src/provider_client_versions.rs
crates/agena-runtime/src/provider_model_selection.rs
crates/agena-runtime/src/provider_priorities.rs
crates/agena-runtime/src/provider_sse.rs
~~~

暂时保留在 agena-runtime 的 composition adapter：

~~~text
provider_composition.rs
runtime_authentication_service.rs
runtime_draft_authentication_service.rs
runtime_status_service.rs
~~~

provider crate 负责：

- OpenAI/Anthropic/Gemini/Ollama/Bedrock/GitLab 等 concrete adapter；
- provider auth store、OAuth helper 和 credential mapping；
- model registry、catalog projection 和 model selection primitive；
- request/response wire mapping；
- provider stream、prompt tool transport 和 cancellation；
- provider-level error mapping 和 usage/cost projection。
- 原 `config/adapter_models.rs`、`credential_store.rs` 与 provider registry/auth/model-listing support；

provider crate 不负责：

- session manager 和 history；
- plugin lifecycle；
- TUI provider studio；
- runtime bootstrap；
- app-level config reload；
- app-level event bus。

如果 provider implementation 需要 session/message/tool 类型，优先使用 agena-runtime-contracts 的 neutral type；不得把 crate::session、crate::tool 或 crate::plugins 通过 feature flag 重新引入。

`config/registry/plugin_host.rs` 当前同时依赖 ProviderRegistry 和 plugin provider-list patch。它不得进入 config crate；将其改造成 `agena-runtime` 中的 composition adapter，输入来自 `agena-runtime-provider` 的 registry 和 `agena-runtime-plugins` 的 patch service。这样 provider crate 不依赖 plugin implementation，plugin crate也不依赖 provider implementation。

### 4.3 agena-runtime-tools

直接移动：

~~~text
crates/agena-runtime/src/tool/
crates/agena-runtime/src/tool_output.rs
~~~

保留在 agena-runtime 的 service facade：

~~~text
runtime_tool_execution_service.rs
policy.rs
metrics.rs 中的 runtime composition adapter
~~~

tool crate 负责：

- builtin tool implementation；
- tool definition、payload、input/output 类型；
- tool executor 和 execution hooks；
- tool registry、tool search 和 truncation；
- shell/file/LSP/task/patch/browser 等 tool implementation；
- neutral ToolExecutionResult 和 tool error。

tool 当前依赖 plugins::provided，而 plugin crate 也依赖 tool。迁移时必须先拆掉这个双向实现依赖：

1. tool crate 只依赖 contracts 中的 PluginToolBinding/PluginInvocation 等中性协议；
2. plugin crate 负责把 provided plugin 映射到 tool executor；
3. tool crate 不再直接 import crate::plugins::provided::*；
4. workflow 中需要 tool 的部分通过 ToolRuntimePort 或 contracts API 调用；
5. agena-runtime 在 composition 层把 tool executor 和 plugin host 连接起来。

直接移动 `tool/**` 前还必须处理三个已确认的 mixed seam：

| 当前耦合 | 处理方式 |
| --- | --- |
| `tool/mod.rs` 直接 import/re-export `plugins::provided::*` 并构造 provided plugin | 所有 provided plugin ID/constructor 回归 `agena-runtime-plugins`；tool root 只保留 executor、payload、registry 和 tool-facing API |
| `tool/snapshot.rs` 直接调用 runtime snapshot registry/operations | 在 contracts 定义 `SnapshotRuntimePort`，runtime 实现 adapter 后注入 tools；tool crate 不依赖 runtime snapshot implementation |
| ToolExecutor 直接持有 Agent/SubagentRegistry/permission/monitor concrete type | Agent 和 permission policy value 可进入 contracts；subagent、monitor 等运行能力改成窄 port 或移动到唯一 implementation owner，不能让 tools 反向依赖 session/runtime |

`tool/**` 的目录 `git mv` 仍然一次执行，但以上路径/API 拆分必须在 Train 6 静态收口时完成，且在第一次 check 前确认新 tools crate 中没有 `crate::plugins`、`agena_runtime::Snapshot*` 或 runtime self-alias。

### 4.4 agena-runtime-plugins

直接移动：

~~~text
crates/agena-runtime/src/plugins/
crates/agena-runtime/src/plugin_config.rs
crates/agena-runtime/src/plugin_runtime_service.rs
crates/agena-runtime/src/plugin_shutdown.rs
crates/agena-runtime/src/plugin_slot.rs
~~~

保留在 agena-runtime 的 composition adapter：

~~~text
plugin_composition.rs
memory/mod.rs 中与 composition 绑定的部分
mcp_runtime.rs 中与 runtime composition 绑定的部分
~~~

plugin crate 负责：

- plugin host lifecycle；
- source discovery、provided plugin registration；
- plugin RPC dispatch；
- plugin tool catalog；
- schema lab、workflow、settings、skills、LSP、MCP 等 provided plugin implementation；
- plugin shutdown/slot state；
- plugin config mapping 中不依赖 runtime composition 的部分。

plugin crate 可以依赖 agena-runtime-tools 的 neutral tool port，但不得让 tool crate 反向依赖 plugin implementation。需要 tool 实例时由 composition 层注入 port 或 factory。

`plugins/sources.rs` 当前通过 runtime self-alias 构造 web/memory plugin，并通过 `crate::tool` 间接取得所有 provided plugin constructor。移动后必须：

1. 直接从 `agena-runtime-plugins::provided` 注册 agent/session/workflow/settings/LSP 等插件；
2. 将 `memory/plugin.rs` 和 `web/plugin.rs` 作为 plugin implementation 移入 plugins crate，或由 runtime 以 `ExtraStaticPluginRegistration` 注入；
3. MCP manager、LSP service 等 composition-owned handle 通过输入参数注入；
4. 删除 `agena_runtime::web_plugin_id/new_web_plugin/memory_plugin_id/new_memory_plugin` self-alias；
5. 保留 project-instructions discovery、MCP connection composition 等非 plugin implementation 在其真实 owner。

feature 归属必须同步迁移：

~~~toml
# agena-runtime-plugins/Cargo.toml
[features]
default = ["schema-lab"]
schema-lab = []
plugin-wasm = ["agena-plugin-host/wasm"]
plugin-signing = ["agena-plugin-host/signing"]

# agena-runtime/Cargo.toml
[features]
default = ["schema-lab"]
schema-lab = ["agena-runtime-plugins/schema-lab"]
plugin-wasm = ["agena-runtime-plugins/plugin-wasm"]
plugin-signing = ["agena-runtime-plugins/plugin-signing"]
~~~

默认 feature 和 feature 名称保持不变，防止应用构建行为发生漂移。

### 4.5 agena-runtime-session

直接移动：

~~~text
crates/agena-runtime/src/session/
crates/agena-runtime/src/session_cache.rs
crates/agena-runtime/src/session_cache_policy.rs
crates/agena-runtime/src/session_execution_control.rs
crates/agena-runtime/src/session_execution_service.rs
crates/agena-runtime/src/session_maintenance.rs
crates/agena-runtime/src/session_plugin_command.rs
crates/agena-runtime/src/session_query_service.rs
crates/agena-runtime/src/session_requests.rs
crates/agena-runtime/src/session_tool_execution.rs
~~~

如果某个 root file 同时包含 runtime composition function 和 session implementation，先在原文件内按函数 owner 分成 implementation 与 adapter，再对 implementation 使用真实 git mv；不得把 impl App 或 runtime composition 一起搬入 session crate。

session crate 负责：

- session model、history、store、event rewrite；
- session manager、processor、prompt window；
- run/compact/continue/rewind/cancel 的 neutral command handling；
- session execution state 和 query service；
- session-to-provider/tool/plugin 的 ports；
- neutral session event、checkpoint 和 execution effect。

session crate 不负责：

- database connection 的创建和 migration；
- runtime global event bus 的安装；
- TUI route/overlay/transcript/composer；
- app message channel；
- provider concrete implementation；
- plugin host singleton。

session tree 也不是无例外的整目录 owner：

- `session/manager/history.rs` 当前实现 RuntimeEventQueryService/RuntimeLiveEventSubscription，并构造 RuntimePresentationEvent；将纯 event projection 移入 session 或 contracts，将 facade trait impl 和 runtime envelope adapter 留在 `agena-runtime`；
- `session/mod.rs`、`session/cache.rs`、prompt window 和 manager 多处通过 `agena_runtime::...` self-alias使用 ExecutionRegistry、ContextGovernor、prompt budget、cache 和 path/snapshot helper；分别将 `execution_registry.rs`、`context_budget.rs`、`context_governor.rs`、`compaction_policy.rs`、`prompt_budget.rs`、`prompt_merge.rs` 移入 session owner，path/install/snapshot 能力通过 port 注入；
- session processor 对 event publisher/store 的使用改成 `SessionEventPublisher`/`SessionEventReader` port；runtime event bus 和 concrete store adapter 留在 `agena-runtime`；
- `AppError` 不随 session 移动，session 使用自己的 typed error，runtime facade 做转换。

因此 `session/**` 可以整体 `git mv` 以保留历史，但 Train 6 必须把 history facade adapter 拆回 runtime，并清空所有 `agena_runtime::` self-alias；不得用新 session crate 依赖 `agena-runtime` 解决这些命中。

### 4.6 Root modules 的留存原则

下列 root modules 默认留在 agena-runtime，除非静态分析明确证明它们是某个新 crate 的纯 owner：

~~~text
application_services.rs
bootstrap_*.rs
composition.rs
connect.rs
db/
event/
event_bridge.rs
event_publish_service.rs
event_query_service.rs
runtime/
runtime_authentication_service.rs
runtime_control_service.rs
runtime_draft_authentication_service.rs
runtime_status_service.rs
snapshot*.rs
store.rs
web/
~~~

下列 root modules 必须在本轮重新分类：

~~~text
message/
presentation_event.rs
error.rs
execution_registry.rs
metrics.rs
policy.rs
context_budget.rs
context_governor.rs
compaction_policy.rs
prompt_budget.rs
prompt_merge.rs
model_catalog*/
memory/
~~~

分类规则是：跨两个以上新 crate 使用的纯 DTO/协议进入 agena-runtime-contracts；只属于 implementation 的留在 implementation crate；只负责 wiring 的留在 agena-runtime。分类完成前不能通过 root wildcard 维持旧路径。

## 5. Studio Git 拆分范围

新建 crates/agena-studio-git library crate，直接移动：

~~~text
apps/agena-studio-server/src/git/
apps/agena-studio-server/src/git2_utils.rs
~~~

当前 Git 目录约 11,266 行、32 个文件，包含：

- auth、remote、GPG、LFS；
- status、history、commit、diff、blame；
- branch、tag、worktree、submodule；
- stash、merge、rebase、cherry-pick、revert；
- stage、clean、ignore、rename、delete、reset；
- GitHub push、repository 和 Git operation policy。

第一轮移动方式：

~~~bash
git mv apps/agena-studio-server/src/git \
       crates/agena-studio-git/src/git
git mv apps/agena-studio-server/src/git2_utils.rs \
       crates/agena-studio-git/src/git2_utils.rs
~~~

如果目录移动后 root module 结构更清晰，可以把 git/mod.rs 作为新 crate 的 src/lib.rs 内容基础，但必须保留真实文件 rename 记录，不得复制一份新实现后删除旧目录。

复核确认 32 个 Git 源文件中有 28 个直接使用 Axum，只有 5 个文件直接读取 Studio AppState。为了保留整棵 Git tree 的真实 rename，本轮将它作为 **HTTP vertical slice** 迁移：Axum handler、request/response DTO 和 Git operation 一起进入 agena-studio-git；不要求先把 28 个混合文件拆成 domain/HTTP 两套文件。

移动后立即处理 5 个 AppState 命中：commit.rs、diff/patch.rs、ops/gh_repo_push.rs、ops/push.rs 和 policy.rs。新 crate 定义窄的 GitPolicyProvider/GitHttpState，只暴露 force-push、no-verify、branch-protection、strict-patch 等 Git 所需动态设置；server 使用现有 settings/AppState 实现或构造该 provider。新 crate 不能 import Studio AppState、StudioDb 或 server error。

最终 server 只负责：

- route registration；
- AppState、auth、settings 和 GitPolicyProvider 注入；
- 将 Git router/handler 挂到现有 auth/CORS/trace middleware；
- 访问日志、限流和 HTTP error response。

agena-studio-git 不得依赖 agena-studio-server，否则只是换目录名而不是拆分。

## 6. 直接移动文件的总执行列车

本节是实际执行顺序。整个 source train 从第一次 git mv 开始，到第 6 列全部完成前，不允许执行编译驱动的验证。

~~~text
Train 0  冻结工作树、生成基线、锁定清单和依赖方向
Train 1  一次创建 contracts/config/provider/tools/plugins/session/studio-git crate
Train 2  直接移动 contracts 中的 neutral owner，先消除 root message/tool 协议耦合
Train 3  直接移动 runtime config/provider/tools/plugins/session 五个 implementation tree
Train 4  直接移动 TUI provider/permission/settings/session 剩余纯 owner
Train 5  直接移动 Studio Git tree 和 git utility
Train 6  连续批量改 module/path/API/visibility/manifest/test/documentation
Train 7  静态收口完成，执行第一次 metadata/fmt/check
Train 8  按错误类别批量修复 affected crates
Train 9  test、Clippy、workspace check/test/clippy、架构报告和 smoke test
~~~

### 6.1 Train 0：冻结和盘点

开始前必须执行：

~~~bash
git status --short
git log -1 --oneline
git diff --check
cargo metadata --format-version 1 --locked
python3 scripts/rust-architecture-report.py \
  --output docs/rust-workspace-analysis.md
~~~

记录以下基线：

- 当前 Git commit；
- 当前 workspace package/target 数量；
- agena-runtime、agena-tui-app、agena-studio-server 的文件/行数/模块数；
- 每个目标目录的 rg --files 清单；
- 每个目标 crate 的 direct normal dependency；
- git diff --check 结果。

如果工作树已有用户改动，不能覆盖、重置或自动清理。必须将目标文件与用户改动分开；如果冲突无法安全绕开，暂停 source train，而不是执行 destructive command。

进入 Train 1 前还必须满足四个 go/no-go gate：

- 已生成 `agena-runtime` public facade allowlist，并标明每个 public item 的最终 owner；
- 已对所有 `agena_runtime::` self-alias 命中做 owner/port/adapter 分类；
- 已画出新 package normal dependency DAG，确认不存在 config/provider/plugins/tools/session 回边；
- 已确认七个目标目录不存在，或仅包含本轮刚创建且可审计的空骨架。

任一 gate 未完成时继续静态盘点，不开始第一条 git mv。

### 6.2 Train 1：一次创建所有新 crate 骨架

在第一次移动之前一次性完成根 Cargo.toml 的 workspace members/path dependencies 和七个新 crate 的 manifest/root。

每个 manifest 先采用：

~~~toml
[package]
name = "agena-runtime-contracts"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
# 只放已确认由首批移动文件使用的真实依赖

[lints]
workspace = true
~~~

七个 crate 的 package 名称分别是：

~~~text
agena-runtime-contracts
agena-runtime-config
agena-runtime-provider
agena-runtime-tools
agena-runtime-plugins
agena-runtime-session
agena-studio-git
~~~

src/lib.rs 先只声明目标 root module 和最小 placeholder API；placeholder 不能被 app 或 runtime 用作长期 compatibility facade。所有新 crate 的 Cargo.toml 必须在 manifest 层形成无 cycle 的 package graph。

新增 workspace package 和重新分配 dependencies 会改变 Cargo.lock。Train 0 可以使用现有 lockfile 执行 `--locked` 基线命令；从 Train 1 修改 manifest 开始，到 Train 6 manifest 稳定之前，不要求 `cargo metadata --locked` 成功。Train 6 静态收口时只允许执行一次受控更新：

~~~bash
cargo metadata --format-version 1
git diff -- Cargo.lock
cargo metadata --format-version 1 --locked
~~~

第一条命令负责写入新 path package/dependency graph；必须人工确认 Cargo.lock 只新增本轮 package 和预期依赖边、没有意外 external version 漂移。此后所有 metadata/check/test/clippy 恢复使用 `--locked`。

### 6.3 Train 2：移动 neutral contracts

先处理跨域最深的 message、tool input/output 和 event payload，避免后续五个 crate 同时保留旧的 crate::message 隐式依赖。

执行规则：

1. 用 rg 分类 message、event/client、presentation_event、tool_output 中的类型；
2. 能整体移动的 module directory 直接 git mv；
3. 混合 runtime composition 的文件先使用 apply_patch 按 owner 拆分，再移动纯部分；
4. 将旧 root re-export 改成 contracts 的显式 pub use；
5. contracts 只暴露跨 crate 使用的类型，不暴露旧 runtime root 的全部 namespace；
6. 记录所有由 crate::message 改成 agena_runtime_contracts::message 的调用点；
7. 这一列结束前不运行 cargo check。

当前源码中，message tree 是第一条直接移动候选；它的 provider usage、tool input、attachment 和 metadata 都是多个 implementation crate 共同使用的 runtime value surface。完成 Train 1 的空 root 后，默认执行：

~~~bash
git mv crates/agena-runtime/src/message \
       crates/agena-runtime-contracts/src/message
~~~

移动后将 event/client.rs 中对旧 crate::message 的引用改为 contracts 路径。若 owner 盘点确认某个 message 子文件混入了 database implementation 或 runtime composition，则只先在原文件中拆出该部分，仍然对纯 message owner 执行真实 git mv；不得复制 message tree 来保留旧路径。

contracts 的 API 先以最小公共面为准：

~~~rust
pub mod execution;
pub mod message;
pub mod ports;

pub use agena_domain::{ExecutionId, ExecutionOutcome, ExecutionPhase};
pub use message::{Message, MessagePart, PartContent};
pub use ports::{
    PluginInvocationPort,
    ProviderCompletionPort,
    SessionEventPublisher,
    SnapshotRuntimePort,
    SubagentSpawner,
    ToolExecutionPort,
};
~~~

上面是方向性 API 草案，port 的具体方法和 error associated type 以现有调用点为准。不要在 contracts 中重新发明一套与现有 SessionExecutionRequest、CompletionRequest、ToolInvocation 重复的 DTO；优先复用 agena-domain、agena-provider、agena-tool 和移动后的 message value。也不为了匹配草案而改变业务字段或序列化格式。

### 6.4 Train 3：直接移动 Runtime implementation tree

按以下顺序执行目录和文件移动：

~~~bash
# config tree 先拆 mixed provider/plugin registry，再移动纯 owner
git mv crates/agena-runtime/src/config/adapter_models.rs \
       crates/agena-runtime-provider/src/config_support/adapter_models.rs
git mv crates/agena-runtime/src/config/credential_store.rs \
       crates/agena-runtime-provider/src/config_support/credential_store.rs
git mv crates/agena-runtime/src/config/registry.rs \
       crates/agena-runtime-provider/src/config_support.rs
git mv crates/agena-runtime/src/config/registry/auth_resolution.rs \
       crates/agena-runtime-provider/src/config_support/auth_resolution.rs
git mv crates/agena-runtime/src/config/registry/model_listing.rs \
       crates/agena-runtime-provider/src/config_support/model_listing.rs
git mv crates/agena-runtime/src/config/registry/provider_registry.rs \
       crates/agena-runtime-provider/src/config_support/provider_registry.rs
git mv crates/agena-runtime/src/config/registry/plugin_host.rs \
       crates/agena-runtime/src/provider_registry_composition.rs

git mv crates/agena-runtime/src/config/mod.rs \
       crates/agena-runtime-config/src/config.rs
git mv crates/agena-runtime/src/config/edit.rs \
       crates/agena-runtime-config/src/config/edit.rs
git mv crates/agena-runtime/src/config/loader.rs \
       crates/agena-runtime-config/src/config/loader.rs
git mv crates/agena-runtime/src/config/overrides.rs \
       crates/agena-runtime-config/src/config/overrides.rs
git mv crates/agena-runtime/src/config/raw.rs \
       crates/agena-runtime-config/src/config/raw.rs
git mv crates/agena-runtime/src/config/raw \
       crates/agena-runtime-config/src/config/raw
git mv crates/agena-runtime/src/config_environment.rs \
       crates/agena-runtime-config/src/config_environment.rs
git mv crates/agena-runtime/src/config_error.rs \
       crates/agena-runtime-config/src/config_error.rs
git mv crates/agena-runtime/src/config_override.rs \
       crates/agena-runtime-config/src/config_override.rs
git mv crates/agena-runtime/src/config_paths.rs \
       crates/agena-runtime-config/src/config_paths.rs
git mv crates/agena-runtime/src/config_values.rs \
       crates/agena-runtime-config/src/config_values.rs
git mv crates/agena-runtime/src/runtime_config_settings_service.rs \
       crates/agena-runtime-config/src/runtime_config_settings_service.rs
git mv crates/agena-runtime/src/runtime_configuration_service.rs \
       crates/agena-runtime-config/src/runtime_configuration_service.rs

git mv crates/agena-runtime/src/provider \
       crates/agena-runtime-provider/src/provider
git mv crates/agena-runtime/src/provider_client_versions.rs \
       crates/agena-runtime-provider/src/provider_client_versions.rs
git mv crates/agena-runtime/src/provider_model_selection.rs \
       crates/agena-runtime-provider/src/provider_model_selection.rs
git mv crates/agena-runtime/src/provider_priorities.rs \
       crates/agena-runtime-provider/src/provider_priorities.rs
git mv crates/agena-runtime/src/provider_sse.rs \
       crates/agena-runtime-provider/src/provider_sse.rs

git mv crates/agena-runtime/src/tool \
       crates/agena-runtime-tools/src/tool
git mv crates/agena-runtime/src/tool_output.rs \
       crates/agena-runtime-tools/src/tool_output.rs

git mv crates/agena-runtime/src/plugins \
       crates/agena-runtime-plugins/src/plugins
git mv crates/agena-runtime/src/plugin_config.rs \
       crates/agena-runtime-plugins/src/plugin_config.rs
git mv crates/agena-runtime/src/plugin_runtime_service.rs \
       crates/agena-runtime-plugins/src/plugin_runtime_service.rs
git mv crates/agena-runtime/src/plugin_shutdown.rs \
       crates/agena-runtime-plugins/src/plugin_shutdown.rs
git mv crates/agena-runtime/src/plugin_slot.rs \
       crates/agena-runtime-plugins/src/plugin_slot.rs

git mv crates/agena-runtime/src/session \
       crates/agena-runtime-session/src/session
git mv crates/agena-runtime/src/session_cache.rs \
       crates/agena-runtime-session/src/session_cache.rs
git mv crates/agena-runtime/src/session_cache_policy.rs \
       crates/agena-runtime-session/src/session_cache_policy.rs
git mv crates/agena-runtime/src/session_execution_control.rs \
       crates/agena-runtime-session/src/session_execution_control.rs
git mv crates/agena-runtime/src/session_execution_service.rs \
       crates/agena-runtime-session/src/session_execution_service.rs
git mv crates/agena-runtime/src/session_maintenance.rs \
       crates/agena-runtime-session/src/session_maintenance.rs
git mv crates/agena-runtime/src/session_plugin_command.rs \
       crates/agena-runtime-session/src/session_plugin_command.rs
git mv crates/agena-runtime/src/session_query_service.rs \
       crates/agena-runtime-session/src/session_query_service.rs
git mv crates/agena-runtime/src/session_requests.rs \
       crates/agena-runtime-session/src/session_requests.rs
git mv crates/agena-runtime/src/session_tool_execution.rs \
       crates/agena-runtime-session/src/session_tool_execution.rs

# session-owned support，不再通过 agena_runtime self-alias 访问
git mv crates/agena-runtime/src/execution_registry.rs \
       crates/agena-runtime-session/src/execution_registry.rs
git mv crates/agena-runtime/src/context_budget.rs \
       crates/agena-runtime-session/src/context_budget.rs
git mv crates/agena-runtime/src/context_governor.rs \
       crates/agena-runtime-session/src/context_governor.rs
git mv crates/agena-runtime/src/compaction_policy.rs \
       crates/agena-runtime-session/src/compaction_policy.rs
git mv crates/agena-runtime/src/prompt_budget.rs \
       crates/agena-runtime-session/src/prompt_budget.rs
git mv crates/agena-runtime/src/prompt_merge.rs \
       crates/agena-runtime-session/src/prompt_merge.rs
~~~

这些命令是目标布局的基线，不是允许无检查执行的 shell glob。`config/registry.rs` 移动前先删掉 `mod plugin_host` 并改成 provider-only root；原 `config/mod.rs` 移动为 config.rs 前同步删除 adapter_models/credential_store/registry declaration。每个目录移动前必须确认目标 crate 已创建、目标目录为空且没有用户文件。移动后必须立即检查 git status --short，确认 Git 识别为 rename，而不是删除后重新创建。

### 6.5 Train 4：连续完成 TUI adapter owner

本轮不创建新的 TUI crate。直接将以下纯 owner 从 agena-tui-app 移入已有 crate：

| 现有 app 路径 | 目标 crate | 保留在 app 的部分 |
| --- | --- | --- |
| provider_studio/provider_fields.rs、provider_auth/fields.rs、provider_auth/summary.rs | agena-tui-provider-studio | auth flow、backend、runtime reload、route；provider_selection 先去 App type |
| app_provider_runtime/** | 不整文件移动 | 全部保留为 catalog refresh、backend/runtime/AppMessage adapter，只抽 pure projection |
| app_permission_helpers/rules.rs、editor.rs、summary.rs，必要时 navigation.rs | agena-tui-permission-studio | live prompt、persistence、path browser、overlay |
| app_settings_helpers/agents.rs、fields.rs、render.rs | agena-tui-settings | app_settings_choices/** 全部保留为 backend/i18n/route adapter |
| app_session_events/** | 不整文件移动 | 四个文件均为 App/transcript/backend/route adapter，只抽新形成的 neutral mapper |
| app_session_interactive/** | 默认不整文件移动 | overlay、terminal、settings/global route；仅移动可独立测试的 pure effect builder |

移动前逐文件处理 impl App：

- 纯函数、纯 state、pure projection 直接移动；
- 既读 feature state 又改 app global state 的函数拆成 pure half 和 adapter half；
- self.backend、self.tx、self.overlay、self.current_route 一律留在 app；
- 测试跟随真实 owner 移动；
- 不建立旧路径 facade。

### 6.6 Train 5：直接移动 Studio Git

执行：

~~~bash
git mv apps/agena-studio-server/src/git \
       crates/agena-studio-git/src/git
git mv apps/agena-studio-server/src/git2_utils.rs \
       crates/agena-studio-git/src/git2_utils.rs
~~~

随后在新 crate 内将 5 个 AppState 命中改成 GitPolicyProvider/GitHttpState，并把 app.rs 中的 route 注册从 crate::git::... 改为 agena_studio_git 的公开 handler/router。request/response JSON、StatusCode、错误 body 和 route path 保持不变；Axum 依赖随 vertical slice 迁入新 crate。

## 7. 移动阶段的连续改写纪律

### 7.1 第一次 git mv 后禁止的命令

从第一次移动开始，直到 Train 6 静态收口完成，禁止运行：

~~~text
cargo check
cargo test
cargo clippy
cargo build
cargo run
cargo bench
E2E/integration test
~~~

禁止编译驱动移动的原因是：本轮涉及七个新 crate、五个 runtime implementation tree、四个已有 TUI feature owner 和一个 Studio library。如果每移动一个目录就 check，会把错误拆成大量不完整的局部反馈，导致临时 re-export、错误 visibility 和错误依赖方向混入代码。

### 7.2 移动阶段允许的静态命令

允许并应当使用：

~~~bash
rg
find
git status --short
git diff --check
git diff --name-status
cargo metadata --format-version 1 --locked
python3 scripts/rust-architecture-report.py --output docs/rust-workspace-analysis.md
~~~

cargo metadata 不编译 Rust；如果 manifest 暂时无法解析，先修 workspace member、package name、path dependency 和 TOML 结构，不以 cargo check 代替 manifest 静态检查。修改 workspace package graph 后，只有在按 6.2 完成一次不带 `--locked` 的受控 Cargo.lock 更新后，才运行这里的 `--locked` metadata 和 Python 报告；不得把预期的 stale-lock 错误误判成源码失败。

### 7.3 批量改写顺序

所有目标文件移动完成后，再统一按以下顺序改写：

1. 新 crate root 的 mod/pub mod/pub use；
2. 旧 runtime/app root 的 module declaration；
3. crate::、super::、self:: 路径；
4. cross-crate use agena_runtime_*::...；
5. root wildcard import 和旧 re-export；
6. pub(crate)、pub(super)、pub(in crate::...)；
7. trait bound、associated type、error type、serde derive；
8. app adapter 的 event/effect mapping；
9. tests、fixtures、#[path]、snapshot helper；
10. Cargo normal/dev/build dependency 和 feature；
11. 删除空 module、旧 facade 和没有调用者的 bridge。

每一批改写后只执行：

~~~bash
git diff --check
git diff --name-status
rg -n 'crate::(provider|session|plugins|tool|config)|crate::(transcript_view|provider_studio|plugin_workbench)' crates apps
~~~

不要使用无边界的全仓库字符串替换；每个替换都应以明确的旧路径和目标 package 为范围。

## 8. Manifest 修改顺序与依赖收缩

### 8.1 Workspace manifest

在根 Cargo.toml 中一次完成：

1. 添加七个 workspace member；
2. 添加七个 [workspace.dependencies] path entry；
3. 保持 default-members = ["apps/agena"] 不变；
4. 不增加第一方 cycle；
5. 不引入新的 external version；
6. 保持 workspace edition/license/MSRV/lints 继承。

### 8.2 新 crate manifest 的依赖方向

目标依赖约束：

~~~text
agena-runtime-contracts
  └── agena-domain + agena-provider/agena-tool/plugin SDK-host 的稳定 value types + derives

agena-runtime-config
  └── contracts + agena-domain + agena-provider + plugin contracts

agena-runtime-provider
  └── contracts + config contracts + agena-provider + provider adapters

agena-runtime-tools
  └── contracts + agena-domain + agena-tool + plugin/tool ports

agena-runtime-plugins
  └── contracts + runtime-tools ports + agena-plugin-host/sdk + storage ports

agena-runtime-session
  └── contracts + config + provider + tools + plugins + storage + domain

agena-runtime
  └── contracts + config + provider + tools + plugins + session + composition adapters

agena-studio-git
  └── axum + tokio + git2 + ignore + serde/path utilities

agena-studio-server
  └── agena-studio-git + runtime + API/application + HTTP/server dependencies
~~~

硬规则：

- agena-runtime-* 不得依赖 agena-runtime；
- agena-runtime-tools 不得依赖 agena-runtime-plugins；
- agena-runtime-provider 不得依赖 agena-runtime-session；
- agena-runtime-config 不得启动 provider/session/plugin；
- agena-studio-git 不得依赖 agena-studio-server；
- 任意新 crate 不得依赖 agena-tui-app；
- app adapter 可以依赖 feature crate，但 feature crate 不得反向依赖 app。

### 8.3 旧 runtime manifest 收缩

所有移动和路径改写完成后，按源码真实 roots 从 crates/agena-runtime/Cargo.toml 删除：

- 仅被 agena-runtime-provider 使用的 provider-specific dependency；
- 仅被 agena-runtime-tools 使用的 tool-specific dependency；
- 仅被 agena-runtime-plugins 使用的 plugin-specific dependency；
- 仅被 agena-runtime-config 使用的 config-specific dependency；
- 仅被 agena-runtime-session 使用的 session-specific dependency。

不能为了先过编译而把全部旧依赖继续留在 runtime；那会掩盖新 crate 的真实边界，也会让架构报告无法验证拆分结果。

同理，agena-tui-app/Cargo.toml 中已经迁出的 presentation-only dependency 必须在静态收口后删除；agena-studio-server/Cargo.toml 中仅 Git implementation 使用的依赖必须转移到 agena-studio-git。

## 9. 第一次编译前的静态收口清单

以下项目全部完成后，才允许第一次 cargo check。

### 9.1 文件和 Git rename

- [ ] 七个新 crate 的目录和 manifest 已创建；
- [ ] 所有目标目录已通过真实 git mv 移动；
- [ ] git diff --name-status 显示 rename，而不是复制后删除；
- [ ] 没有 symlink、hard link、include! 或跨 crate #[path]；
- [ ] 测试、fixture、snapshot helper 跟随 owner 移动；
- [ ] git status --short 没有意外删除或未跟踪的重复源码。

### 9.2 Module/path/namespace

- [ ] 新 crate lib.rs 的 module tree 与文件布局一致；
- [ ] 旧 agena-runtime/src/lib.rs 不再声明已移动的大目录；
- [ ] 旧 app root 不再重复声明已迁出的纯 feature module；
- [ ] crate::provider/session/plugins/tool/config 旧路径仅存在于应保留的 runtime adapter；
- [ ] 旧 root wildcard import 已删除或收缩为少数明确 re-export；
- [ ] #[path]、include!、旧 package path 和兼容 facade 已清零。

### 9.3 Ownership/visibility

- [ ] 每个移动类型都有唯一 owner；
- [ ] 新 crate 公共 API 不暴露 app/runtime composition 类型；
- [ ] 所有 pub(crate) 提升都有明确跨 crate consumer；
- [ ] pub(in ...) 已改成新 crate 内有效的最小 visibility；
- [ ] 没有为了编译将整个 module tree 机械改为 pub；
- [ ] error、event、snapshot、effect 的 owner 和转换方向明确。

### 9.4 依赖和协议

- [ ] agena-runtime-contracts 只包含稳定中性类型；
- [ ] tool/plugins 双向依赖已通过 port/protocol 消除；
- [ ] provider/session 不再互相引用 implementation module；
- [ ] 新 runtime crate 不依赖 agena-runtime；
- [ ] 新 TUI crate 不依赖 agena-tui-app；
- [ ] package dependency graph 无第一方 cycle；
- [ ] 每个 manifest 的 normal/dev/build dependency 与源码实际 roots 对齐。

### 9.5 静态命令

~~~bash
git status --short
git diff --name-status
git diff --check

rg -n 'include!\s*\(|#\s*\[path\s*=|agena-runtime/src|agena-tui-app/src' crates apps Cargo.toml
rg -n 'crate::(provider|session|plugins|tool|config)' crates/agena-runtime/src
rg -n 'agena_runtime::|use agena_runtime' \
  crates/agena-runtime-contracts \
  crates/agena-runtime-config \
  crates/agena-runtime-provider \
  crates/agena-runtime-tools \
  crates/agena-runtime-plugins \
  crates/agena-runtime-session
rg -n 'agena-runtime\s*=.*workspace|path\s*=.*agena-runtime' \
  crates/agena-runtime-contracts/Cargo.toml \
  crates/agena-runtime-config/Cargo.toml \
  crates/agena-runtime-provider/Cargo.toml \
  crates/agena-runtime-tools/Cargo.toml \
  crates/agena-runtime-plugins/Cargo.toml \
  crates/agena-runtime-session/Cargo.toml
rg -n '&mut App|crate::App|self\.backend|self\.tx|self\.overlay|RuntimePresentationEvent' \
  crates/agena-runtime-contracts \
  crates/agena-runtime-config \
  crates/agena-runtime-provider \
  crates/agena-runtime-tools \
  crates/agena-runtime-plugins \
  crates/agena-runtime-session \
  crates/agena-tui-transcript \
  crates/agena-tui-plugin-workbench \
  crates/agena-tui-session \
  crates/agena-tui-provider-studio \
  crates/agena-tui-permission-studio \
  crates/agena-tui-settings

cargo metadata --format-version 1
git diff -- Cargo.lock
cargo metadata --format-version 1 --locked
python3 scripts/rust-architecture-report.py --output docs/rust-workspace-analysis.md
git diff --check
~~~

报告命令允许在这里运行，因为它是静态扫描；但它会更新报告文件，必须确认差异只来自本次 package/module 结构变化。

## 10. 第一次 check/test/clippy 的执行顺序

静态收口完成后开始编译验证。所有 Cargo 命令必须串行执行，避免多个命令争抢 target lock。第一次 check 的目的，是一次性收集完整的路径、visibility、trait 和 dependency 错误，不再用编译错误指导下一轮文件移动。

### 10.1 先执行格式和 metadata

~~~bash
cargo fmt --all -- --check
cargo metadata --format-version 1 --locked
~~~

如果格式检查失败，先统一运行 cargo fmt --all 并检查 diff；不得在格式错误中混入业务改写。

### 10.2 先 check 新 crate

~~~bash
cargo check -p agena-runtime-contracts --all-targets --locked
cargo check -p agena-runtime-config --all-targets --locked
cargo check -p agena-runtime-provider --all-targets --locked
cargo check -p agena-runtime-tools --all-targets --locked
cargo check -p agena-runtime-plugins --all-targets --locked
cargo check -p agena-runtime-plugins --all-targets --no-default-features --locked
cargo check -p agena-runtime-plugins --all-targets --all-features --locked
cargo check -p agena-runtime-session --all-targets --locked
cargo check -p agena-studio-git --all-targets --locked
~~~

### 10.3 再 check composition/app

~~~bash
cargo check -p agena-runtime --all-targets --locked
cargo check -p agena-runtime --all-targets --no-default-features --locked
cargo check -p agena-runtime --all-targets --all-features --locked
cargo check -p agena-application --all-targets --locked
cargo check -p agena-api-server --all-targets --locked
cargo check -p agena-cli --all-targets --locked
cargo check -p agena-tui-backend --all-targets --locked
cargo check -p agena-tui-transcript --all-targets --locked
cargo check -p agena-tui-plugin-workbench --all-targets --locked
cargo check -p agena-tui-session --all-targets --locked
cargo check -p agena-tui-provider-studio --all-targets --locked
cargo check -p agena-tui-permission-studio --all-targets --locked
cargo check -p agena-tui-settings --all-targets --locked
cargo check -p agena-tui-app --all-targets --locked
cargo check -p agena-studio-server --all-targets --locked
cargo check -p agena --all-targets --locked
cargo check -p agena-e2e --all-targets --locked
~~~

### 10.4 再执行 affected test

~~~bash
cargo test -p agena-runtime-contracts --all-targets --locked
cargo test -p agena-runtime-config --all-targets --locked
cargo test -p agena-runtime-provider --all-targets --locked
cargo test -p agena-runtime-tools --all-targets --locked
cargo test -p agena-runtime-plugins --all-targets --locked
cargo test -p agena-runtime-plugins --all-targets --all-features --locked
cargo test -p agena-runtime-session --all-targets --locked
cargo test -p agena-studio-git --all-targets --locked

cargo test -p agena-runtime --all-targets --locked
cargo test -p agena-application --all-targets --locked
cargo test -p agena-api-server --all-targets --locked
cargo test -p agena-cli --all-targets --locked
cargo test -p agena-tui-backend --all-targets --locked
cargo test -p agena-tui-transcript --all-targets --locked
cargo test -p agena-tui-plugin-workbench --all-targets --locked
cargo test -p agena-tui-session --all-targets --locked
cargo test -p agena-tui-provider-studio --all-targets --locked
cargo test -p agena-tui-permission-studio --all-targets --locked
cargo test -p agena-tui-settings --all-targets --locked
cargo test -p agena-tui-app --all-targets --locked
cargo test -p agena-studio-server --all-targets --locked
cargo test -p agena --all-targets --locked
~~~

### 10.5 再执行 Clippy 和 workspace 三连

~~~bash
cargo clippy -p agena-runtime-contracts --all-targets --all-features --locked
cargo clippy -p agena-runtime-config --all-targets --all-features --locked
cargo clippy -p agena-runtime-provider --all-targets --all-features --locked
cargo clippy -p agena-runtime-tools --all-targets --all-features --locked
cargo clippy -p agena-runtime-plugins --all-targets --all-features --locked
cargo clippy -p agena-runtime-session --all-targets --all-features --locked
cargo clippy -p agena-studio-git --all-targets --all-features --locked
cargo clippy -p agena-runtime --all-targets --all-features --locked
cargo clippy -p agena-tui-app --all-targets --all-features --locked
cargo clippy -p agena-studio-server --all-targets --all-features --locked

cargo check --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked
~~~

workspace test 中如果出现既有 macOS linker warning，需要记录 warning 文本和退出码；不能把 warning 当成 test failure，也不能为了消除 warning 改变本轮 ownership。

## 11. 编译错误的批量修复策略

第一次 check 完成后保存完整输出，再按错误类别处理。每一组错误修完后只重新执行受影响的 check，确认该组清空后再进入下一组。

### 11.1 第一组：workspace/package/module

处理：

- workspace member、path dependency、package name；
- missing module、duplicate module、wrong mod.rs；
- crate::/super::/self:: 残留；
- hyphen/underscore crate name；
- binary/library target 入口错误。

原则：先修 manifest 和 module tree，不改业务 API，不增加 compatibility facade。

### 11.2 第二组：跨 crate import 和 visibility

处理：

- E0432/E0433 unresolved import；
- E0603 private item；
- pub(super)、pub(in ...) 在新 crate 中失效；
- test helper owner 错位；
- re-export 层级错误。

原则：移动类型 owner 或建立最小 API，不通过全局 pub 解决。

### 11.3 第三组：contracts 和 trait

处理：

- neutral DTO 字段缺失或错误；
- associated type/lifetime/trait bound；
- Send/Sync、Clone、Debug、serde derive；
- provider/session/tool/plugin effect/event 的方向错误；
- Result error 类型不一致。

原则：先修 contracts 定义和 adapter mapping，再修调用方；不要在每个调用点增加局部类型转换。

### 11.4 第四组：feature/manifest/dependency

处理：

- missing dependency；
- production dependency 与 dev-dependency 错位；
- feature flag owner 错误；
- target-specific dependency；
- runtime 旧依赖未收缩；
- new crate 误引入 agena-runtime 或 agena-tui-app。

### 11.5 第五组：测试和行为回归

处理：

- fixture/import path；
- async test runtime；
- snapshot/render expectation；
- TUI app adapter test；
- Studio Git request/response test；
- runtime provider/session/plugin/tool integration test。

不能删除测试、降低断言、屏蔽 Clippy 或用 fake effect 绕过真实 adapter。

## 12. 架构报告和最终验收

所有 affected check/test/clippy 通过后，重新生成报告：

~~~bash
python3 scripts/rust-architecture-report.py \
  --output docs/rust-workspace-analysis.md
git diff --check
git status --short
~~~

### 12.1 结构验收

- [ ] agena-runtime-contracts 是小型中性协议 crate，没有 App/runtime composition；
- [ ] agena-runtime-provider、agena-runtime-session、agena-runtime-config、agena-runtime-tools、agena-runtime-plugins 均被 Cargo 识别；
- [ ] 新 runtime crate 没有反向依赖 agena-runtime；
- [ ] agena-studio-git 不依赖 agena-studio-server；
- [ ] agena-tui-* feature crate 不依赖 agena-tui-app；
- [ ] workspace normal dependency graph 没有第一方 cycle；
- [ ] agena-runtime 的大 implementation tree 已迁出，保留 composition 和 adapter；
- [ ] agena-tui-app 的 feature 纯 owner 已迁出，app 保留 shell/adapter；
- [ ] Git tree 不再由 agena-studio-server 直接拥有。

### 12.2 文件和源码验收

- [ ] Git diff 显示真实 rename/move；
- [ ] 没有源码复制、symlink、hard link、include! 或跨 crate #[path]；
- [ ] 移动后的测试仍然位于真实 owner；
- [ ] runtime root wildcard 显著收缩；
- [ ] feature crate 公共 API 没有 &mut App、Backend、Runtime 或 route/overlay；
- [ ] tool/plugins 双向 implementation dependency 已消除；
- [ ] provider/session 只通过 contracts/ports 跨域；
- [ ] `agena-runtime` public facade 只包含审计过的 allowlist re-export，所有上层 consumer 路径保持稳定；
- [ ] 新 runtime feature crate 中没有 `agena_runtime::` self-alias 或 parent dependency；
- [ ] 未出现新的生产 .rs 未触达文件；
- [ ] crates/agena-runtime/src/permission/runtime.rs 已确认是合法模块、迁移 owner 或明确遗留文件，不能保持未解释状态。

### 12.3 编译和行为验收

- [ ] cargo fmt --all -- --check 通过；
- [ ] cargo metadata --format-version 1 --locked 通过；
- [ ] 所有新 crate cargo check --all-targets --locked 通过；
- [ ] agena-runtime、agena-tui-app、agena-studio-server check 通过；
- [ ] 所有 affected crate test 通过；
- [ ] workspace check/test/clippy 通过；
- [ ] transcript、session、provider、permission、settings、plugin workbench TUI smoke 通过；
- [ ] provider authentication/model selection/streaming smoke 通过；
- [ ] plugin load/RPC/provided tool smoke 通过；
- [ ] Studio Git status/diff/commit/branch/remote/worktree 至少各完成一条真实 API smoke；
- [ ] Python 架构报告生成成功且模块解析/词法告警为 0；
- [ ] git diff --check 通过。

### 12.4 量化验收目标

重新生成的架构报告应满足以下目标。它们用于发现“文件搬了但职责没搬”的情况，不允许为了达标删除测试或压缩代码：

| Package | 当前值 | 目标上界/趋势 |
| --- | ---: | --- |
| agena-runtime | 104,714 行 / 307 文件 | 不高于约 45,000 行 / 180 文件，且五个大 implementation tree 已消失 |
| agena-tui-app | 39,036 行 / 105 文件 | 下降到约 36,000 行或更低；下降必须来自真实纯 owner 迁移 |
| agena-studio-server | 21,212 行 / 61 文件 | 约 12,000 行或更低，Git tree 不再由 server package 持有 |
| agena-runtime-contracts | 新 crate | 原则上不高于约 12,000 行；依赖面不含 runtime composition 或 database I/O |
| agena-runtime-provider | 新 crate | provider implementation 集中，且不依赖 session/plugins/runtime parent |
| agena-runtime-tools / plugins | 新 crate | package graph 单向，不存在 tool/plugins cycle |
| agena-runtime-session | 新 crate | session implementation 集中，runtime facade adapter 不混入 |

如果某一数值超过目标但 ownership 和依赖方向正确，应在执行记录中解释；数字不是失败的唯一依据。以下情况无论行数如何都判定失败：新 crate 依赖 parent runtime/app、出现第一方 cycle、runtime 通过全量 re-export 继续暴露内部实现、或 Studio Git 仍引用 server AppState。

## 13. 明确禁止的伪快速方式

以下方式不属于本计划：

- 让 agena-runtime-provider、agena-runtime-session 或任何新 crate 依赖 agena-runtime；
- 把整个 runtime/src 搬到一个新的 agena-runtime-core，只换 package 名称而不改变 ownership；
- 把 provider/session/plugins/tool/config 全部合并到一个新的“大 runtime common crate”；
- 保留旧 module 作为 compatibility facade 并把所有旧实现 re-export 出去；
- 使用 include!、#[path]、symlink、hard link 或复制源码；
- 为通过 E0603 将所有内部 item 机械改为 pub；
- 在第一次 git mv 后用 cargo check 单条错误驱动下一次移动；
- 为了省事把旧 crate 的全量 dependencies 复制到所有新 manifest；
- 删除原有测试、降低断言、绕过真实 runtime/plugin/tool effect；
- 通过更改序列化字段、route、错误文案或 keymap 来“顺便修复”行为；
- 让 feature crate 直接持有 App、Backend、RuntimePresentationEvent 或 UnboundedSender<AppMessage>。

## 14. 实际执行顺序摘要

~~~text
1. 读取本计划和前置 TUI 计划，确认工作树与当前报告基线
2. cargo metadata + Python 架构报告，锁定 package/file/module 清单
3. 盘点所有 crate::/super::/pub(crate)/impl App/Runtime coupling
4. 一次创建七个新 crate manifest、workspace member、path dependency 和空 root
5. 直接移动 contracts neutral owner
6. 直接移动 runtime config tree 和 config root files
7. 直接移动 runtime provider tree 和 provider root support files
8. 直接移动 runtime tool tree 和 tool output
9. 直接移动 runtime plugin tree 和 plugin root service files
10. 直接移动 runtime session tree 和 session root service files
11. 连续移动 TUI provider/permission/settings/session 的纯 owner
12. 直接移动 Studio Git tree 和 git2 utility
13. 一次性批量改 module/path/API/visibility/test/manifest
14. 静态清理旧 root module、wildcard、facade、错误依赖和未触达文件
15. 运行 git diff --check、cargo metadata、Python report
16. 第一次 cargo check：先新 crate，再 runtime/app/studio
17. 按 module → visibility → contracts/trait → manifest → test 分类修复
18. 运行 affected test 和 Clippy
19. 运行 workspace check/test/clippy
20. 重新生成架构报告，完成 TUI/runtime/Studio smoke 和最终验收
~~~

本计划的完成标准不是“目录都搬到了新 crate”，而是：

~~~text
agena-runtime       = composition + adapters
runtime feature     = independent implementation + neutral contracts
agena-tui-app       = application shell + backend/platform/runtime adapters
agena-studio-server = HTTP/application shell + Studio Git adapter
~~~

整个过程必须保持真实 git mv、连续源码改写、一次静态收口、最后统一编译验证的节奏，使这次拆分可以快速完成，同时不留下只换目录名的假边界。
