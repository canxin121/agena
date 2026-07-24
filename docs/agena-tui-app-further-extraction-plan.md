
# `agena-tui-app` 进一步拆分与快速迁移执行计划

> 状态：第一至第三批边界已落地；本轮已完成完整 transcript renderer 与 plugin schema workbench 纯 owner 的真实迁移，剩余为计划明确保留在 app 的 shell/adapter/composer
>
> 规划日期：2026-07-24
>
> 代码基线：以本计划生成时的 `docs/rust-workspace-analysis.md` 为准，报告记录的 Git 基线为 `0a9c9de4b471`
>
> 事实来源：`cargo metadata --format-version 1 --locked`、`scripts/rust-architecture-report.py`、[`docs/rust-workspace-analysis.md`](rust-workspace-analysis.md)、[`docs/agena-app-crate-extraction-plan.md`](agena-app-crate-extraction-plan.md) 以及 `crates/agena-tui-app/src` 的源码静态审查。
>
> 执行模式：一次连续的源码迁移列车。先完成所有 crate、目录移动、路径/API/可见性和 manifest 改写，再第一次运行 `cargo check`；之后按错误类别批量修复，并统一完成 test、Clippy 和架构报告复核。

### 执行记录（2026-07-24）

本计划已经在独立 worktree/branch 中开始执行，当前结果如下；这里记录的是实际落地范围，不把仍由 app 持有的 adapter 或 renderer 误计为已完成：

- 已在 workspace 注册并创建 `agena-tui-transcript`、`agena-tui-plugin-workbench`、`agena-tui-session`、`agena-tui-provider-studio`、`agena-tui-permission-studio` 和 `agena-tui-settings` 六个 library crate；这些 crate 均不依赖 `agena-tui-app`。
- 已通过真实 `git mv` 迁移 transcript viewport/model/navigation/selection、Markdown/math presentation helper、session presentation、plugin/permission/settings presentation；app 侧改为显式 crate 路径。
- `agena-tui-transcript` 已形成独立的 `TranscriptState`、live-update/action/effect 协议、render model、selection/navigation，以及 Markdown/math helper；最近迁移的 `math.rs` 和 `markdown.rs` 已完成依赖、模块注册和 app wiring。
- `agena-tui-session` 已形成 neutral `SessionController`、command/event/live-event/effect 协议，并由 app adapter 映射 session execution refresh；composer 的 pending restore draft 已归入 app 的 `SessionComposerState`。
- plugin、provider、permission、settings 已具备各自的 snapshot/state/action/effect 或 presentation 边界；具体 backend/runtime/persistence 和 route/overlay adapter 仍主要由 app 持有。
- `agena-tui-transcript` 已完成完整 renderer owner 的真实迁移：`renderer.rs`、AST、Markdown/text、tool/operation/request render、diff/auxiliary helper 和 renderer tests 均在新 crate；app 只保留 Runtime/backend/clipboard/pager/route 等 concrete adapter，并通过明确的 crate API 调用 renderer。
- `agena-tui-plugin-workbench` 已完成 schema/model/validation/materialization/row construction/text render/presentation helper 的真实迁移；app 中只保留 `workbench_config.rs`、`workbench_editor.rs`、`workbench_input.rs`、`workbench_navigation.rs` 和 `workbench_render.rs` 五个 concrete adapter。新 crate 的 helper 默认收紧为 crate 内可见，跨 crate 操作集中在可审计的 `api` namespace。
- 已重新生成 `Cargo.lock` 和 `docs/rust-workspace-analysis.md`；报告确认 app 从基线的 141 个 Rust 文件降至 105 个，transcript/plugin 新 owner 分别形成 17/22 个 Rust 文件，且新 feature crate 不依赖 `agena-tui-app`。
- 当前已验证 `cargo metadata --format-version 1 --locked`、格式检查、`git diff --check`、transcript 81 项测试、plugin workbench 7 项测试和 app 111 项测试；随后 workspace `check --all-targets`、`test --all-targets` 和 `clippy --all-targets --all-features` 也全部通过。workspace test 保留一个既有的 macOS linker warning（`__eh_frame section too large`），没有测试失败。
- 使用临时 PTY 驱动器完成了真实 `agena` TUI 的交互 smoke：回应启动阶段的 OSC/CSI terminal protocol、设置 40×120 terminal window，并分别发送 transcript、session、settings、plugin、provider、permission 场景的按键序列；六个场景均以退出码 0 结束。该驱动器未写入仓库文件。

本轮完成的明确边界是完整 transcript renderer 和 plugin schema workbench 纯 owner；composer、route/overlay 组合、Runtime/backend/persistence/platform 调用以及五个 plugin concrete adapter 仍按计划保留在 `agena-tui-app`，不把这些 adapter 误计为已迁出。

## 1. 本轮结论

`agena-tui-app` 确实已经大到需要继续拆分，但拆分方式不能是把 `transcript_view/`、`plugin_workbench/` 或 `provider_studio/` 原封不动地搬进新 crate。当前这些目录虽然名称上是功能域，实际仍有大量 `impl App`、`self.backend`、`self.tx`、`self.overlay` 和 root-level re-export。直接移动只会把 `App` 的耦合换一个目录名，产生一个“看起来独立、实际依赖应用壳”的假 crate。

本轮采用以下顺序：

1. 先在 `agena-tui-app` 内收缩 wildcard/root re-export，明确 feature state、action、effect 和 adapter 的边界。
2. 第一批连续迁移两个相对完整、收益最大的 vertical slice：`agena-tui-transcript` 和 `agena-tui-plugin-workbench`。
3. 第二批迁移 session controller，把 `AppMessage` 中的 session 生命周期、异步请求和 runtime 事件映射变成显式协议。
4. 第三批按 vertical slice 分别迁移 provider、permission、settings；不把它们合成一个新的“大 studio crate”。
5. `composer` 暂不强制拆 crate，等 session contract 稳定后再决定它应进入 session crate、独立成为 composer crate，还是继续由 app shell 持有。

目标依赖方向如下：

```text
agena-tui-app
  ├── agena-tui-transcript
  ├── agena-tui-plugin-workbench
  ├── agena-tui-session
  ├── agena-tui-provider-studio
  ├── agena-tui-permission-studio
  └── agena-tui-settings

feature crates
  ├── agena-tui / agena-tui-components
  ├── agena-api / agena-domain / agena-application（仅限稳定 DTO/契约）
  └── agena-plugin-sdk 等真正的下层契约

agena-tui-app
  ├── agena-tui-backend
  ├── agena-tui-platform
  ├── agena-tui-media
  └── agena-runtime（只在 app adapter 或明确的 session adapter 中使用）
```

所有新 feature crate 都是 library crate，不能依赖 `agena-tui-app`。`agena-tui-app` 仍然负责最终的 route/overlay 组合、应用生命周期、backend/platform adapter 和进程内异步任务编排；新 crate 负责可测试的状态、渲染模型、用户动作和中性的 effect 描述。

## 2. 不变的行为约束

本轮只改变源码所有权、模块路径、Cargo 依赖和内部 API 形状，不改变：

- TUI 的用户可见布局、主题、文案、keymap 和 overlay 行为；
- session 创建、提交、continue、compact、rewind、cancel 及流式刷新语义；
- provider 登录、模型选择、配置保存和 runtime reload 语义；
- plugin 配置校验、编辑、保存、日志和 inspect 语义；
- permission prompt、permission rule 编辑和持久化语义；
- clipboard、外部编辑器、pager、terminal suspend 和图像传输行为；
- CLI 参数、`agena` binary、app-server mode、持久化格式和网络安全边界。

迁移完成后，禁止以“重构顺便清理”为理由改变业务逻辑。任何行为变化都必须另开任务，不能混入本次移动列车。

## 3. 当前 `agena-tui-app` 的量化事实

以 [`rust-workspace-analysis.md`](rust-workspace-analysis.md) 的 `agena-tui-app` package 统计和 `7.41 agena-tui-app::agena_tui_app` 模块邻接表为准：

| 指标 | 当前值 | 说明 |
| --- | ---: | --- |
| Rust 文件 | 141 | 包含生产代码和模块内测试 |
| Rust 行数 | 60,218 | 以报告的源码行统计为准 |
| 模块 | 183 | `lib.rs` root 加子模块和测试模块 |
| 模块声明边 | 182 | 没有解析错误 |
| 模块引用边 | 361 | 静态 token 级近似 |
| 源码 dependency roots | 28 | 说明 crate root 观测到的外部/第一方入口很宽 |
| 第一方 normal dependencies | 12 | `agena-api`、`agena-application`、`agena-domain`、`agena-plugin-host`、`agena-plugin-sdk`、`agena-provider`、`agena-runtime`、`agena-tui`、`agena-tui-backend`、`agena-tui-components`、`agena-tui-media`、`agena-tui-platform` |
| 直接 external dependencies | 23 | 见报告的 `agena-tui-app` dependency 明细 |
| 模块解析错误 | 0 | 当前基线健康 |
| 词法结构告警 | 0 | 当前基线健康 |

这说明当前主要问题不是“有坏模块”，而是一个 crate 同时承担了：

- application shell 和 `App` 生命周期；
- session、transcript、composer 状态；
- Runtime event 到 UI 状态的映射；
- plugin/provider/permission/settings 多套 studio；
- 大段 transcript 和 schema renderer；
- backend、terminal、clipboard、editor/pager 的调用适配；
- 所有 route、overlay、异步消息和全局文案的组合。

### 3.1 共享 namespace 证据

[`crates/agena-tui-app/src/lib.rs`](../crates/agena-tui-app/src/lib.rs) 约第 255 行集中声明几十个功能模块，约第 321 行开始又把多个模块 wildcard 导入到 root namespace：

```rust
use self::app_types::*;
use self::plugin_workbench::*;
use self::provider_studio::provider_auth::*;
use self::provider_studio::provider_fields::*;
use self::provider_studio::provider_selection::*;
use self::transcript_navigation::*;
use self::transcript_selection::*;
```

这会让任意 `impl App` 文件都能通过未限定名称使用别的 feature 的类型。移动文件之前必须先建立显式模块路径，否则迁移过程中会出现大量无法判断 owner 的隐式依赖。

### 3.2 `App` 是当前的 God Object

[`app_types.rs`](../crates/agena-tui-app/src/app_types.rs) 的 `App` 约有 65 类字段，横跨：

- backend/runtime 和 event channel；
- route、overlay stack、flash message 和 UI tick；
- session list、当前 session、session execution；
- transcript、selection、navigation、render cache；
- composer、draft store、queued message；
- provider、plugin、permission、settings studio；
- pending UI/platform actions、mouse、keymap、terminal context。

同文件中的 `AppMessage` 约有 30 个变体，把 session、transcript、provider、usage、permission、plugin 和 status 异步结果混在一个应用级信箱中。`Overlay` 和 `Route` 也在同一个文件中统一注册所有 workflow。

因此新 crate 不能直接接受 `&mut App` 作为公共入口；它必须通过 snapshot/state、action 和 effect 与 app shell 通信。

## 4. 总体边界：State → Action → Effect → Adapter

每个待迁移 vertical slice 都遵循同一个四层模型：

```text
用户输入 / runtime snapshot
          │
          ▼
feature state + feature renderer
          │  Action
          ▼
feature reducer/controller
          │  Effect
          ▼
agena-tui-app adapter
          │  backend / runtime / platform / tx
          ▼
异步结果或中性事件
          │
          └──────────────► feature state::apply(event)
```

边界规则：

- `State` 只保存该 feature 自己的 UI 状态，不保存 `App`、`Backend`、`TerminalRuntime` 或 `UnboundedSender<AppMessage>`。
- `Action` 表示用户意图，不直接执行异步请求，不写 clipboard，不改变全局 route。
- `Effect` 表示 feature 需要 app shell 执行的动作，必须是可枚举、可测试、可记录的值。
- adapter 把 effect 翻译成 backend/runtime/platform 调用，再把结果投影为 feature 能理解的中性 event。
- route、overlay stack、session ID 的全局组合留在 app shell；feature crate 只提供自己的 presentation/state/action。
- 真实跨 crate 使用的 item 才变成 `pub`。`pub(crate)` 不能机械改成 `pub`，需要先判断 owner 和协议方向。

## 5. 目标 crate 拓扑与拆分批次

### 5.1 第一批：两个低风险高收益 feature crate

| 新 crate | 主要 owner | 暂时留在 app 的 adapter | 第一批目标 |
| --- | --- | --- | --- |
| `agena-tui-transcript` | transcript state、navigation、selection、纯 renderer、transcript text/helper | Runtime event 转换、load/refresh、clipboard、pager、external editor、route/overlay mutation | 先让 transcript renderer 不依赖 `App` 和 Runtime envelope |
| `agena-tui-plugin-workbench` | schema、config model、validation、policy builder、纯 display/render | backend 查询/保存、异步执行、flash、route/overlay mutation | 先让 workbench model/render 不依赖 `App` 和 backend |

### 5.2 第二批：session controller

| 新 crate | 主要 owner | 暂时留在 app 的 adapter | 第二批目标 |
| --- | --- | --- | --- |
| `agena-tui-session` | session command/event/effect、session lifecycle reducer、请求协议 | backend/runtime 调用、`tx`、route、composer、transcript owner 的组合 | 把 `AppMessage` 的 session 部分从全局消息枚举中拆出 |

### 5.3 第三批：三个独立 vertical slice

| 新 crate | 纯部分 | 强耦合部分 |
| --- | --- | --- |
| `agena-tui-provider-studio` | provider rows、field model、selection、schema/validation、renderer | backend save/load、auth polling、model catalog refresh、runtime reload、route |
| `agena-tui-permission-studio` | permission rule model、scope editor、rule validation、renderer | live permission prompt、session execution reply、backend persistence、route |
| `agena-tui-settings` | settings field schema、choice model、navigation、renderer | backend snapshot/load/save、runtime reload、provider/agent concrete queries |

permission 的 live prompt 明确留在 session adapter，不能在第三批为了“目录完整”强行搬进 permission crate。provider、permission、settings 也不能合并为一个新的 `agena-tui-studio`，否则只会复制当前 `App` 的大边界。

## 6. 第一批迁移：`agena-tui-transcript`

### 6.1 迁移目标

transcript crate 负责：

- transcript message/part 的 presentation model；
- markdown、code、table、thinking、tool、operation、permission card 的纯渲染；
- transcript navigation、selection、search、collapse/expand、follow-tail；
- render cache、cursor anchor、copy segment 和导出文本构造；
- 可由中性 live update 更新的 transcript state。

候选 owner 总规模约 12.5k 行，其中 renderer 约 10.8k 行；准确文件归属以移动前的 `rg --files` 和静态引用清单为准，不允许凭目录名漏移测试或子模块。

### 6.2 建议迁移清单

第一批可以移动的纯 owner：

```text
crates/agena-tui-app/src/transcript_view.rs
crates/agena-tui-app/src/transcript_view/
crates/agena-tui-app/src/transcript_navigation.rs
crates/agena-tui-app/src/transcript_selection.rs
crates/agena-tui-app/src/app_types/transcript.rs
```

`app_transcript_helpers.rs`、`app_transcript_actions.rs`、`app_transcript_input.rs` 必须先逐函数分类：

- 纯 transcript projection、selection、copy segment、文本格式化的函数随 feature crate 移动；
- clipboard、pager、external editor、backend load/refresh、route/overlay 变更的函数留在 app adapter；
- 既读 transcript 又修改全局 App 的函数拆成“feature pure half + app adapter half”，不能整文件搬走后继续 `impl App`。

`ui_text.rs` 中的 transcript/message/operation 文案可以移动到 feature crate 或拆为 feature text module；仍被多个 feature 共用的通用文案保留在 app shell 的显式 `app_text` 模块中，不能继续通过 root wildcard 隐式共享。

### 6.3 必须先处理的 Runtime 耦合

当前 [`transcript_state.rs`](../crates/agena-tui-app/src/transcript_state.rs) 直接接收 `agena_runtime::RuntimePresentationEvent`，并在约第 248 行处理 Runtime event，在约第 292 行处理 Runtime checkpoint。这部分不能原样迁移。

先定义不暴露 Runtime envelope 的中性类型，例如：

```rust
pub struct TranscriptLiveUpdate {
    pub session_id: i64,
    pub sequence: Option<i64>,
    pub kind: TranscriptLiveUpdateKind,
}

pub enum TranscriptLiveUpdateKind {
    UserMessageAppended { message_id: i64 },
    MessagePartCheckpointed(MessagePartCheckpoint),
    MessagePartDelta(MessagePartDelta),
    AssistantMessageFinished,
    RefreshRequested,
}
```

具体字段以现有 Runtime event 和当前测试需要为准；关键是 `agena-tui-transcript` 不直接依赖 `agena_runtime::RuntimePresentationEvent`、`Backend`、`TerminalRuntime` 或 `App`。

### 6.4 Transcript API 草案

最终 API 不要求一次完全定型，但必须形成以下方向：

```rust
pub struct TranscriptState;
pub struct TranscriptRenderContext;
pub struct TranscriptSnapshot;

pub enum TranscriptAction {
    MoveUp,
    MoveDown,
    Search { query: String },
    ToggleCurrentNode,
    CopyCurrentSelection,
    FollowTail,
}

pub enum TranscriptEffect {
    LoadOlderMessages { session_id: i64, cursor: Option<String> },
    RefreshSession { session_id: i64 },
    CopyText { text: String },
}
```

`CopyText` 可以由 app adapter 转给 platform；feature crate 不调用 clipboard。`LoadOlderMessages` 和 `RefreshSession` 只是 effect，不在 feature crate 内启动 tokio task。

当前 `TranscriptState` 中的 `pending_restore_draft: Option<ComposerDraft>` 不属于 transcript owner，应迁回 session/composer owner；不能为了让移动暂时通过而把 `ComposerDraft` 继续塞进 transcript crate 的公共 API。

### 6.5 Transcript 验收

- transcript crate 能在没有 `agena-tui-app` 的情况下构造 state、输入 action、应用 live update 并渲染 snapshot；
- renderer 测试随 owner 一起移动，测试不通过源码包含访问 app 私有字段；
- app 只保留 Runtime/backend/platform/route adapter；
- `rg` 不再发现 transcript crate 中的 `&mut App`、`crate::App`、`self.backend`、`self.tx`、`TerminalRuntime`、`RuntimePresentationEvent`；
- transcript selection/navigation 的公开 API 不再依赖 app root 的 wildcard import；
- export、copy、pager 等行为仍由 app adapter 调用原有 platform 能力，结果一致。

## 7. 第一批迁移：`agena-tui-plugin-workbench`

### 7.1 迁移目标

plugin workbench 总规模约 11.2k 行。其中约 8.7k 行是 model/schema/validation/render，约 2.5k 行是直接 `impl App` 的 adapter。第一批只搬纯部分，保留 concrete backend adapter 在 app。

纯 owner 候选：

```text
crates/agena-tui-app/src/plugin_workbench.rs
crates/agena-tui-app/src/plugin_workbench/workbench_config_actions.rs
crates/agena-tui-app/src/plugin_workbench/workbench_config_sections/
crates/agena-tui-app/src/plugin_workbench/workbench_config_state.rs
crates/agena-tui-app/src/plugin_workbench/workbench_display.rs
crates/agena-tui-app/src/plugin_workbench/workbench_policy_builder.rs
crates/agena-tui-app/src/plugin_workbench/workbench_render_helpers.rs
crates/agena-tui-app/src/plugin_workbench/workbench_schema_resolution.rs
crates/agena-tui-app/src/plugin_workbench/workbench_schema_util.rs
crates/agena-tui-app/src/plugin_workbench/workbench_schema_validation/
crates/agena-tui-app/src/plugin_workbench/workbench_text_render/
```

以下文件先留在 app，随后逐个把 `impl App` 改成 adapter：

```text
crates/agena-tui-app/src/plugin_workbench/workbench_config.rs
crates/agena-tui-app/src/plugin_workbench/workbench_editor.rs
crates/agena-tui-app/src/plugin_workbench/workbench_input.rs
crates/agena-tui-app/src/plugin_workbench/workbench_navigation.rs
crates/agena-tui-app/src/plugin_workbench/workbench_render.rs
```

### 7.2 已知耦合和拆法

[`workbench_input.rs`](../crates/agena-tui-app/src/plugin_workbench/workbench_input.rs) 直接读取：

- `self.backend.config_json_sources()`；
- `self.backend.plugin_statuses()`；
- `self.backend.plugin_inspect()`；
- `self.backend.plugin_logs()`；
- `self.flash_error(...)`。

[`workbench_config.rs`](../crates/agena-tui-app/src/plugin_workbench/workbench_config.rs) 直接调用：

- `self.backend.set_config_setting()`；
- `self.block_on_async(...)`；
- `self.flash_success(...)`、`flash_warning(...) `、`flash_info(...)`。

这些不是 workbench model 的职责。新 crate 的 reducer 只产生 effect，app adapter 再调用 backend，并把成功/失败映射成 `PluginWorkbenchEvent` 或通用 flash effect。

### 7.3 Plugin Workbench API 草案

```rust
pub struct PluginWorkbenchState;
pub struct PluginWorkbenchSnapshot;

pub enum PluginWorkbenchAction {
    SelectPlugin(String),
    MoveSelection(isize),
    OpenConfigPath(Vec<PathSegment>),
    EditValue(String),
    DeleteValue,
    ResetValue,
    Validate,
    ToggleDiff,
}

pub enum PluginWorkbenchEffect {
    LoadPlugins,
    SaveConfig {
        plugin_id: String,
        value: serde_json::Value,
    },
    InspectPlugin { plugin_id: String },
    ReadPluginLogs { plugin_id: String },
}
```

实际 API 应以现有 `PluginWorkbenchOverlay`、schema row、policy builder 和测试需要收敛；不要求把当前所有内部结构都公开。跨 crate 公共面优先使用 snapshot、opaque row、路径段和序列化配置值，避免把 `App`、`Route` 或 backend trait 泄露给 feature crate。

新 crate 禁止出现下列路径或调用：

```text
&mut App
crate::App
crate::Route
crate::app_types
crate::ui_text
self.backend
self.block_on_async(...)
self.flash_success(...)
self.flash_warning(...)
```

### 7.4 Plugin Workbench 验收

- schema materialization、validation、policy builder、config row rendering 和文本渲染测试随 owner 一起移动；
- adapter 只负责 backend 查询/保存、异步调度、flash 和 route/overlay 组合；
- plugin crate 不依赖 `agena-tui-app`，也不通过 `#[path]` 回读旧文件；
- plugin config 的保存、删除、reset、diff、validation 行为保持一致；
- app root 不再用 `use self::plugin_workbench::*` 提供隐式 API，改为显式 `agena_tui_plugin_workbench::...` 或 app adapter 的明确 re-export。

## 8. 第二批迁移：`agena-tui-session`

### 8.1 为什么 session 是第二批

session 是 transcript、composer、permission prompt、runtime event 和 backend 请求的交汇点，耦合比前两块更高。若先移动 session 文件而没有 feature contract，最容易得到一个“带 `App` 参数的 session crate”。因此先完成 transcript/workbench 的 effect 方向，再建立 session controller。

### 8.2 候选移动范围

以当前实际目录为准，候选范围包括：

```text
crates/agena-tui-app/src/app_session_events/
crates/agena-tui-app/src/app_session_interactive/
crates/agena-tui-app/src/app_session_helpers.rs
crates/agena-tui-app/src/app_session_input.rs
crates/agena-tui-app/src/app_command_actions.rs
crates/agena-tui-app/src/app_command_helpers.rs
crates/agena-tui-app/src/commands.rs
```

这些文件不能整目录直接搬。当前 `app_session_events/requests.rs` 和 `handlers.rs` 大量读写 `self.backend`、`self.tx`、`self.transcript`，`app_session_interactive/*` 又直接修改 composer、overlay 和 route。必须先拆为：

1. 纯 command parsing、session selection、request projection、controller state；
2. backend/runtime request adapter；
3. transcript/composer/overlay 的 app composition adapter。

`commands.rs` 的通用 command specification 如果被 app、session、help 和 command palette 多处消费，可以先独立成中性 command model；具体 command 执行仍留在 app/session adapter。

### 8.3 Session 协议

推荐的 reducer/controller 形状：

```rust
pub struct SessionController;

pub enum SessionCommand {
    CreateSession,
    SubmitMessage { draft: ComposerDraft },
    Continue,
    Compact,
    Rewind { message_id: i64 },
    Cancel,
    ReplyPermission { reply: PermissionReply },
    ReplyUserInput { reply: UserInputReply },
}

pub enum SessionEvent {
    SessionsLoaded(/* neutral page */),
    SessionCreated(/* neutral session */),
    MessagesLoaded(/* neutral page */),
    SessionRefreshed(/* neutral update */),
    SessionEventArrived(/* neutral live event */),
    RunCancelled(/* neutral result */),
}

pub enum SessionEffect {
    LoadSessions(/* scope */),
    LoadMessages(/* session + cursor */),
    RefreshSession(/* session + sequence */),
    SubmitMessage(/* session + draft */),
    Continue(/* session */),
    Compact(/* session */),
    Cancel(/* session */),
    SubscribeSessionEvents(/* session */),
}
```

推荐生命周期：

```text
SessionCommand
  -> SessionController::reduce
  -> SessionEffect
  -> app/backend adapter 执行异步请求
  -> SessionEvent
  -> SessionController::apply
```

`SessionEvent` 不应直接暴露 `RuntimePresentationEvent`、backend response 的巨大 envelope 或 `AppMessage`。如果某个 runtime 类型确实是稳定的跨层契约，应先在 `agena-application`/`agena-api` 中确认 owner；否则在 adapter 中转换成 session crate 的中性数据。

### 8.4 `AppMessage` 拆分原则

当前 `AppMessage` 混合 session、transcript、provider、usage、permission、plugin 等结果。迁移时：

- session crate 拥有 session command/event/effect；
- transcript live update 单独进入 transcript crate；
- provider/plugin/permission 的结果进入各自 feature event；
- app shell 可以暂时保留一个 composition-level message，但它只能是明确的 feature event wrapper，不再让每个 feature 共享一个可任意扩展的 God enum；
- 新 feature event 进入 app shell 时必须有明确转换函数，不能靠 root wildcard 或同名 variant 自动拼接。

### 8.5 Session 验收

- controller 的 reduce/apply 可以在无 runtime/backend 的测试中运行；
- backend request 的执行、tokio task、channel send、route/overlay mutation 全留在 app adapter；
- `pending_restore_draft` 由 session/composer owner 管理，不再是 transcript state 的字段；
- cancel、continue、compact、rewind 和 permission/user-input reply 的 effect 顺序保持一致；
- session crate 不依赖 `agena-tui-app`，不接收 `&mut App`；
- app 的 `handle_message` 只做 feature event 分发和 composition，不再承担所有业务 reducer 细节。

## 9. 第三批迁移：provider / permission / settings

第三批必须按 vertical slice 执行，每个 slice 都完成“纯 model/renderer → action/effect → app adapter → 测试”闭环，再开始下一个。不要一次把整个 `provider_studio/`、`app_permissions/`、`app_settings*` 全部移动后再找 owner。

### 9.1 `agena-tui-provider-studio`

候选纯 owner：

```text
crates/agena-tui-app/src/provider_studio/provider_fields.rs
crates/agena-tui-app/src/provider_studio/provider_model_helpers.rs
crates/agena-tui-app/src/provider_studio/provider_selection.rs
crates/agena-tui-app/src/provider_studio/provider_auth/fields.rs
crates/agena-tui-app/src/provider_studio/provider_auth/summary.rs
```

`provider_auth/flow.rs`、`provider_studio.rs` 以及直接调用 backend/runtime 的部分先保留 adapter；auth polling、model catalog refresh、provider save 和 runtime reload 不能被纯 UI crate 持有。

可形成的协议方向：

```rust
pub enum ProviderStudioAction {
    SelectProvider(String),
    EditField { field: ProviderField, value: String },
    Submit,
    RefreshModels,
    StartAuth,
}

pub enum ProviderStudioEffect {
    LoadProviders,
    SaveProvider(/* neutral draft */),
    LoadModels(/* provider */),
    StartAuthentication(/* provider */),
}
```

provider crate 不应直接依赖 `Runtime` 或 `Backend`，也不应持有 route stack。

### 9.2 `agena-tui-permission-studio`

先移动：

```text
permission rule draft / subject / scope model
rule validation
permission rule editor navigation
纯 permission studio renderer
```

可以从 [`app_types/overlays.rs`](../crates/agena-tui-app/src/app_types/overlays.rs) 中拆出 permission rule editor 相关类型，但要先把当前与 `Overlay`、`Route`、live prompt 绑定的字段分开。live permission prompt、`PermissionReply` 发送和 session execution 交互留在 `agena-tui-session` / app adapter。

### 9.3 `agena-tui-settings`

候选纯 owner：

```text
crates/agena-tui-app/src/app_settings.rs
crates/agena-tui-app/src/app_settings_choices/
crates/agena-tui-app/src/app_settings_helpers/
```

但其中直接调用 backend、读取 workspace、读取 runtime snapshot、修改 route 的部分必须留下 adapter。`app_settings_choices/provider/tests.rs` 现有测试如果使用本地 `#[path]`，迁移时改成标准 crate module，不再依靠跨路径源码包含。

settings crate 只产生字段编辑、选择、保存和 reload effect；具体 preference DTO 的 load/save 由 app/backend adapter 执行。

## 10. 可选后续：composer

composer 是 session 的输入端，同时涉及 attachments、clipboard、external editor、draft persistence、prompt history、slash command 和 queue。它当前不适合作为第一批独立 crate。

只有满足以下条件后才评估 `agena-tui-composer`：

- `SessionCommand::SubmitMessage` 已稳定；
- `ComposerDraft` 的 owner 已从 transcript 和 AppMessage 中明确移出；
- clipboard/editor/platform 调用都已经变成 effect；
- queue 不再反向依赖 `app_types::ComposerDraft`；
- composer 的测试可以构造纯 state，不需要完整 `App`。

如果上述条件未满足，继续把 composer 保留在 app shell 是可接受的；“拆得更多”不是本轮的验收目标，“边界真实”才是。

## 11. 快速迁移列车：总执行纪律

本节是本计划的核心操作流程。目的是让重构主要由文件移动和批量路径改写完成，而不是在编译器错误之间来回切换。

### 11.1 Train 0：冻结和盘点

开始移动前完成以下动作：

1. 确认工作树中只有预期的报告改动和本计划改动；已有用户改动不得覆盖。
2. 重新生成一次架构报告，保存当前基线；如果报告输出有变化，先记录原因。
3. 保存每个候选 owner 的文件清单、行数、module declaration 和引用边。
4. 对所有候选文件执行 `rg`，记录 `impl App`、`crate::`、`super::`、`pub(crate)`、wildcard import、backend/runtime/platform 调用。
5. 确认目标 crate 目录不存在；如果目录已经存在且不是本轮新建的空目录，暂停并重新确认 owner，不能覆盖。
6. 确认 workspace 依赖图没有需要反向指向 `agena-tui-app` 的新边。

Train 0 只做静态检查，不修改业务代码。

### 11.2 Train 1：一次创建所有新 crate 骨架

在第一次移动前一次性完成：

- workspace members；
- `[workspace.dependencies]` 中的新 path dependency；
- 每个新 crate 的 `Cargo.toml`；
- `src/lib.rs` 的最小 root；
- lint、edition、license、rust-version 等 workspace 继承；
- 不新增任何不必要的 external dependency。

manifest 先使用目标代码的最小真实 dependency 集合。不能把 `agena-tui-app` 当前 23 个 external dependencies 全量复制给每个 feature crate；应按移动后 `rg` 的 crate roots 分配依赖。若一个依赖仅用于 app adapter，就留在 `agena-tui-app`。

### 11.3 Train 2：按批次连续 `git mv`

推荐顺序：

```text
Train 0  冻结、清单、协议草案
Train 1  创建所有 crate manifest 和空 root
Train 2  git mv transcript 纯 owner
Train 3  git mv plugin workbench 纯 owner
Train 4  git mv /拆分 session controller owner
Train 5  provider / permission / settings vertical slices
Train 6  统一路径、可见性、trait bound、manifest、测试改写
Train 7  静态收口完成后第一次 cargo check
Train 8  批量修复并执行 test / clippy / workspace 验证
```

移动目录时优先使用完整目录：

```bash
git mv crates/agena-tui-app/src/transcript_view \\
       crates/agena-tui-transcript/src/transcript_view
git mv crates/agena-tui-app/src/transcript_view.rs \\
       crates/agena-tui-transcript/src/transcript_view.rs
```

上面的命令只是形式示例；执行前必须确认目标目录已由 Train 1 创建、目标不存在同名内容，并根据最终 root module 布局决定 `mod.rs` 是否需要保留。不要用 shell glob 在未检查的目录上移动。

### 11.4 移动阶段禁止编译驱动

从第一次 `git mv` 开始，到所有源码和 manifest 改写完成前，禁止运行：

- `cargo check`；
- `cargo test`；
- `cargo clippy`；
- integration/E2E test；
- `cargo build`、`cargo run`、benchmark 或依赖扫描；
- 用编译器错误逐条指导移动的循环。

移动阶段允许并应当运行：

```text
rg
find
git status
git diff --check
python3 scripts/rust-architecture-report.py
cargo metadata --format-version 1 --locked
```

`cargo metadata` 只用于确认 workspace/package/manifest graph，不编译 Rust。若 manifest 还未形成可解析状态，先修 manifest 静态结构再运行，不以 `cargo check` 代替。

### 11.5 连续批量改写

所有移动完成后，再按以下顺序批量修改：

1. module root：`mod foo;`、`pub mod foo;`、`mod.rs` 与目录 root；
2. crate path：`crate::...`、`super::...`、`self::...`；
3. cross-crate import：`use agena_tui_...::...`；
4. `pub(crate)`、`pub(super)`、private item 的 owner 可见性；
5. `impl App` adapter 与 effect/event 转换；
6. trait bound、associated type、error type 和 serde derive；
7. unit test、test helper、fixture、`#[path]`；
8. Cargo dependencies、dev-dependencies、feature、workspace dependency；
9. 删除旧 root wildcard import、旧 module declaration 和未使用的 bridge。

路径改写可以由多个精确的 `rg` 结果驱动，但不得用无边界的全仓库替换覆盖 unrelated crate。每一批替换后用 `git diff --check` 和 `rg` 检查，不运行编译。

## 12. Source train 前的冻结和保护检查

### 12.1 工作树检查

```bash
git status --short
git diff -- docs/rust-workspace-analysis.md docs/agena-app-crate-extraction-plan.md
git diff --check
```

`agena-app-crate-extraction-plan.md` 本轮不得被改写。报告文件的既有更新如果来自本轮重新生成，应保留；如果发现无关差异，先停止并确认。

### 12.2 目标目录检查

```bash
test ! -e crates/agena-tui-transcript
test ! -e crates/agena-tui-plugin-workbench
test ! -e crates/agena-tui-session
test ! -e crates/agena-tui-provider-studio
test ! -e crates/agena-tui-permission-studio
test ! -e crates/agena-tui-settings
```

如果某个目录已存在，不能直接删除、覆盖或复用；先确认它是否是已有用户工作、未完成实验或别的 crate。

### 12.3 旧耦合清单

在移动前对候选文件建立清单，至少检查：

```text
impl App
&mut App
crate::App
crate::Route
crate::Overlay
self.backend
self.tx
self.transcript
self.composer
self.overlay
self.current_route
self.block_on_async
self.flash_
RuntimePresentationEvent
use self::*
pub(crate)
pub(super)
#[path =
include!(
```

这些命中不代表都要立刻删除，但每一个命中都必须在“纯 owner、feature adapter、app composition”三者中做出归属决定。

## 13. Manifest 和 workspace 修改顺序

### 13.1 Workspace

在根 [`Cargo.toml`](../Cargo.toml) 中：

1. 将新 crate 加入 workspace members；
2. 在 `[workspace.dependencies]` 加入 path dependency；
3. 保持 `default-members = ["apps/agena"]` 不变；
4. 不引入新的第一方循环；
5. 不添加与现有 lockfile 不同版本的 external dependency；
6. 保持 workspace lint、edition、license、rust-version 继承方式一致。

### 13.2 New crate manifest

每个新 crate 先拥有一个可解析的标准 manifest：

```toml
[package]
name = "agena-tui-transcript"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
# 只列出移动后源码真正需要的契约和 presentation dependencies

[lints]
workspace = true
```

其余 crate 使用相同结构。不要先加 `agena-tui-app` 作为 dependency；如果 feature crate 只有依赖 app 才能编译，说明边界还没有拆好。

### 13.3 旧 crate manifest 收缩

所有源码移动和路径改写完成后，按源码实际 crate roots 删除 `agena-tui-app/Cargo.toml` 中不再使用的依赖。不要在第一轮错误修复期间为了省事保留一份“全量依赖垃圾桶”；否则架构报告无法显示真实边界。

dev-dependencies 也要随测试 owner 移动。测试专用依赖可以留在 feature crate 的 dev-dependencies，不能把 production dependency 为了 test helper 错误地放进 normal dependencies。

## 14. `git mv` 文件移动清单

下面是执行时的 owner 清单，不是允许不经检查的机械脚本。每一项移动前必须用 `rg` 重新确认没有隐藏的 app adapter。

### 14.1 Transcript

```text
crates/agena-tui-app/src/transcript_view.rs
crates/agena-tui-app/src/transcript_view/
crates/agena-tui-app/src/transcript_navigation.rs
crates/agena-tui-app/src/transcript_selection.rs
crates/agena-tui-app/src/app_types/transcript.rs
```

按函数拆分：

```text
app_transcript_helpers.rs       -> pure transcript helpers / app adapter
app_transcript_actions.rs       -> transcript actions / platform + route adapter
app_transcript_input.rs         -> transcript action mapping / App input adapter
ui_text.rs                      -> transcript text / shared app text
```

### 14.2 Plugin workbench

```text
crates/agena-tui-app/src/plugin_workbench.rs
crates/agena-tui-app/src/plugin_workbench/workbench_config_actions.rs
crates/agena-tui-app/src/plugin_workbench/workbench_config_sections/
crates/agena-tui-app/src/plugin_workbench/workbench_config_state.rs
crates/agena-tui-app/src/plugin_workbench/workbench_display.rs
crates/agena-tui-app/src/plugin_workbench/workbench_policy_builder.rs
crates/agena-tui-app/src/plugin_workbench/workbench_render_helpers.rs
crates/agena-tui-app/src/plugin_workbench/workbench_schema_resolution.rs
crates/agena-tui-app/src/plugin_workbench/workbench_schema_util.rs
crates/agena-tui-app/src/plugin_workbench/workbench_schema_validation/
crates/agena-tui-app/src/plugin_workbench/workbench_text_render/
```

保留并改造成 app adapter：

```text
workbench_config.rs
workbench_editor.rs
workbench_input.rs
workbench_navigation.rs
workbench_render.rs
```

### 14.3 Session

```text
crates/agena-tui-app/src/app_session_events/
crates/agena-tui-app/src/app_session_interactive/
crates/agena-tui-app/src/app_session_helpers.rs
crates/agena-tui-app/src/app_session_input.rs
crates/agena-tui-app/src/app_command_actions.rs
crates/agena-tui-app/src/app_command_helpers.rs
crates/agena-tui-app/src/commands.rs
```

这里的“移动”必须伴随拆分；不能把包含 `impl App` 的目录整体放入 `agena-tui-session`。

### 14.4 Provider / permission / settings

```text
provider_studio/ 的纯 fields / selection / model helper / renderer
app_permissions/ 的 pure rule editor / validation / renderer
app_settings.rs
app_settings_choices/
app_settings_helpers/ 的 pure field / navigation / render 部分
```

每个目录中直接访问 backend、runtime、channel、route、overlay 或 platform 的文件/函数继续留在 app adapter，随后通过 effect contract 逐个替换。

## 15. 路径改写和可见性规则

### 15.1 路径规则

移动后禁止留下以下旧路径作为兼容层：

```text
crate::transcript_view::...
crate::plugin_workbench::...
crate::provider_studio::...
crate::app_types::...
```

生产代码应该使用明确的 crate path，例如：

```rust
use agena_tui_transcript::{TranscriptAction, TranscriptState};
use agena_tui_plugin_workbench::PluginWorkbenchSnapshot;
```

如果 app shell 为了组合需要 re-export，必须是少量、带注释、可审计的显式 API；不能恢复 `use self::feature::*`。

### 15.2 可见性规则

按以下顺序处理可见性：

1. feature crate 内部仍然 private 的 item 不改；
2. feature crate 内部跨 module 使用的 item 用 `pub(crate)`；
3. app adapter 使用的 item 才提升为 `pub`；
4. workspace 外部没有使用的 helper 不 re-export；
5. 类型出现在公共函数签名中时，先移动类型 owner 或使用中性 DTO，再决定是否公开；
6. 绝不为了消除 E0603 一次性把整棵模块树改成 `pub`。

`pub(super)` 在跨 crate 后通常不再成立，应回到 owner module 的 private helper 或转为 feature crate 内 `pub(crate)`；不要直接机械替换为 `pub`。

### 15.3 Trait 和 error 规则

- Backend trait 不进入纯 renderer/model crate；
- effect 不携带具体 `Backend` trait object；
- feature crate 的错误类型描述 feature 层失败，adapter 再映射 backend/runtime error；
- 不为了复用 app 的 `UiResult` 而让所有新 crate 依赖 app 的错误 alias；
- serde/clone/debug 等 derive 只在协议确实需要时保留，避免为跨 crate 暴露内部缓存结构。

## 16. 第一次编译前的静态收口清单

以下清单全部完成后，才允许第一次 `cargo check`。

### 16.1 文件和 module

- [x] 所有目标文件通过 `git mv` 移动，`git status` 显示 rename 而不是删除后重建；
- [x] 每个新 crate 有唯一 root module；
- [x] 没有同一源码 owner 的第二份复制；
- [x] 没有 `include!`、跨 crate `#[path]`、symlink、hard link 或源码镜像；
- [x] 所有 `mod`、目录和 `mod.rs` 路径都能由文件树解释；
- [x] 旧 crate 中没有孤立的旧 module declaration；
- [x] 新 crate 内没有指回 `agena-tui-app/src` 的路径。

### 16.2 路径和 namespace

- [x] `rg` 搜索旧 `crate::feature` 路径，结果只剩允许的历史文档或注释；
- [x] `lib.rs` 的 wildcard import 已删除或收缩到明确的内部边界；
- [x] `crate::App`、`&mut App` 只出现在 app adapter；
- [x] feature crate 没有 `self.backend`、`self.tx`、`self.overlay`、`self.current_route`；
- [x] feature crate 没有直接使用 `RuntimePresentationEvent`；
- [x] 所有 cross-crate import 的 crate name、module name、re-export 都已批量改写；
- [x] `pub(crate)` / `pub(super)` 的每个提升都有 owner 依据。

### 16.3 协议和依赖方向

- [x] 每个 `Action`、`Event`、`Effect` 都有明确 owner；
- [x] effect 没有隐含的 `&mut App` 或 callback closure；
- [x] app adapter 有把 effect 转给 backend/platform/runtime 的明确入口；
- [x] runtime event 已在 app adapter 转为中性 feature event；
- [x] 新 crate 不依赖 `agena-tui-app`；
- [x] Cargo package 图仍无第一方 cycle；
- [x] feature crate 只声明真实需要的 normal dependency；
- [x] test-only dependency 没有混进 production dependency。

### 16.4 测试和文档

- [x] 单元测试跟随 owner 移动；
- [x] `#[path]` 测试已改为标准 module；
- [x] fixture、test helper、snapshot 路径已改写；
- [x] 不再通过 compatibility facade 访问旧私有实现；
- [x] 本计划和架构报告是唯一需要同步更新的重构文档；
- [x] `agena-app-crate-extraction-plan.md` 保持不变。

### 16.5 静态命令

```bash
git status --short
git diff --check
rg -n 'include!\\s*\\(|#\\s*\\[path\\s*=|agena-tui-app/src|crate::(transcript_view|plugin_workbench|provider_studio|app_types)' crates Cargo.toml apps
rg -n '&mut App|crate::App|self\\.backend|self\\.tx|self\\.overlay|self\\.current_route|RuntimePresentationEvent' crates/agena-tui-transcript crates/agena-tui-plugin-workbench crates/agena-tui-session crates/agena-tui-provider-studio crates/agena-tui-permission-studio crates/agena-tui-settings
cargo metadata --format-version 1 --locked
python3 scripts/rust-architecture-report.py --output docs/rust-workspace-analysis.md
```

最后一条报告命令是静态分析，但它会更新报告文件；执行后要确认差异只反映本次结构变更，并再次运行 `git diff --check`。

## 17. 第一次 check/test/clippy 的执行顺序

静态收口完成后，才开始编译验证。不要同时启动多个会争抢 `target` 锁的 Cargo 命令。

### 17.1 先验证 manifest 和受影响 crate

```bash
cargo fmt --all -- --check
cargo metadata --format-version 1 --locked
cargo check -p agena-tui-transcript --all-targets --locked
cargo check -p agena-tui-plugin-workbench --all-targets --locked
cargo check -p agena-tui-session --all-targets --locked
cargo check -p agena-tui-provider-studio --all-targets --locked
cargo check -p agena-tui-permission-studio --all-targets --locked
cargo check -p agena-tui-settings --all-targets --locked
cargo check -p agena-tui-app --all-targets --locked
```

如果某个新 crate 尚未在本批次实现，不要执行不存在的 package 命令；应先完成该 crate 的 manifest/root 或调整计划中的批次边界。

### 17.2 再运行受影响 crate 的 test

```bash
cargo test -p agena-tui-transcript --all-targets --locked
cargo test -p agena-tui-plugin-workbench --all-targets --locked
cargo test -p agena-tui-session --all-targets --locked
cargo test -p agena-tui-provider-studio --all-targets --locked
cargo test -p agena-tui-permission-studio --all-targets --locked
cargo test -p agena-tui-settings --all-targets --locked
cargo test -p agena-tui-app --all-targets --locked
```

### 17.3 再执行 Clippy 和 workspace 验证

```bash
cargo clippy -p agena-tui-app --all-targets --all-features --locked
cargo clippy -p agena-tui-transcript --all-targets --all-features --locked
cargo clippy -p agena-tui-plugin-workbench --all-targets --all-features --locked

cargo check --workspace --all-targets --locked
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked
```

如果 workspace 规模较大，可以先完成 affected package 的 check/test/clippy，再执行全 workspace 三连；不能因此省略最终 workspace 验证。

## 18. 编译错误的批量修复策略

第一次 check 的价值是产生完整错误集，而不是指导下一次文件移动。处理方式：

### 18.1 第一组：路径和 module

集中处理：

- E0432/E0433 unresolved import/module；
- `crate::`、`super::`、`self::` 残留；
- root module、目录 module 和 re-export 错位；
- 旧 crate name 或 hyphen/underscore 名称错误。

一次性修完后重新执行 affected `cargo check`，不要在同一轮夹杂业务 API 改造。

### 18.2 第二组：可见性和 owner

集中处理：

- E0603 private item；
- `pub(super)` 跨 crate 无效；
- feature crate 公共签名暴露了 app-private 类型；
- 测试 helper owner 不在正确 crate。

修复原则是移动类型 owner 或缩小 API，不是普遍加 `pub`。

### 18.3 第三组：协议、trait 和类型

集中处理：

- effect/event 的字段类型不稳定；
- lifetime、trait bound、associated type；
- `serde`、`Clone`、`Debug`、`Send`、`Sync` 约束；
- Runtime DTO 到 neutral DTO 的转换遗漏。

这一组应优先修协议定义，再修调用方；不要在每个调用点做局部类型强转。

### 18.4 第四组：feature 和 manifest

集中处理：

- missing dependency；
- dev dependency 误放或反之；
- feature flag 不在 owner crate；
- lockfile/target-specific dependency；
- `cfg(test)` 与 production module 不一致。

### 18.5 第五组：测试和行为回归

最后处理：

- fixture/import path；
- 测试 helper visibility；
- snapshot/render expectation；
- async test runtime；
- app integration test 对新 event/effect 的适配。

每组修完后都运行同一组 affected check/test，错误类别清空后再进入下一组。若发现新的架构边界问题，回到 API contract 修复，不建立临时 compatibility facade。

## 19. 最终架构报告和 diff 验收

所有代码验证通过后重新生成报告：

```bash
python3 scripts/rust-architecture-report.py \\
  --output docs/rust-workspace-analysis.md
git diff --check
git status --short
```

需要从新报告确认：

- 新 package 被 Cargo 识别，且没有第一方依赖环；
- `agena-tui-app` 文件数和行数显著下降，减少来自真实 owner 迁出而非删除源码；
- `agena-tui-transcript`、`agena-tui-plugin-workbench`、`agena-tui-session` 的模块树不再回指 app；
- 新 crate 的源码 dependency roots 与 manifest dependencies 基本一致；
- app shell 仍然能看到 backend/platform/runtime，但 feature crate 不反向看到 app；
- 模块解析错误和词法结构告警保持为 0；
- 报告中不出现意外的未触达生产 `.rs` 文件；
- 旧 `agena-tui-app` root wildcard 边显著收缩；
- 测试模块仍然挂在正确的 owner 下。

报告不只是最终统计文件，也是判断这次拆分是否真实的证据。若新 crate 仍依赖 app，或 app 只是通过 wrapper 暴露原有大模块，本轮不算完成。

## 20. 最终验收标准

### 20.1 结构验收

- [x] `agena-tui-app` 不再承载完整 transcript renderer 和完整 plugin schema workbench；
- [x] 第一批至少形成 `agena-tui-transcript`、`agena-tui-plugin-workbench` 两个真实 library crate；
- [x] session controller 有独立的 command/event/effect 协议；
- [x] provider、permission、settings 的后续边界已按 vertical slice 记录并可独立执行；
- [x] 新 crate 不依赖 `agena-tui-app`；
- [x] 没有 compatibility facade、alias crate、symlink、hard link、源码复制或 `include!`。

### 20.2 代码验收

- [x] `git diff` 显示真实 rename/move；
- [x] 所有移动文件的测试一起移动；
- [x] `App` 只保留 shell state、composition、adapter 和生命周期；
- [x] feature crate 的公共 API 由 state/action/effect/snapshot 组成，而不是 `&mut App`；
- [x] Runtime/backend/platform 调用集中在 app adapter；
- [x] root wildcard re-export 已消除或收缩到可审计的少量 API；
- [x] 可见性没有为了过编译而无差别公开。

### 20.3 验证验收

- [x] `cargo fmt --all -- --check` 通过；
- [x] `cargo metadata --format-version 1 --locked` 通过；
- [x] 所有 affected package 的 `cargo check --all-targets --locked` 通过；
- [x] 所有 affected package 的 `cargo test --all-targets --locked` 通过；
- [x] affected package Clippy 通过；
- [x] workspace check/test/clippy 通过；
- [x] `python3 scripts/rust-architecture-report.py` 成功；
- [x] `git diff --check` 通过；
- [x] 交互式 transcript、session、plugin、provider、permission、settings 流程完成最小手工 smoke test。

## 21. 明确禁止的“快速”方式

以下方式虽然可能短期减少编译错误，但不属于本计划的快速重构：

- 把整个 `transcript_view` 或 `plugin_workbench` 目录搬走，仍让所有文件 `impl App`；
- 新 crate 依赖 `agena-tui-app`，或让 app 反向提供 feature crate 的所有内部类型；
- 使用 `include!`、跨 crate `#[path]`、symlink、hard link 或复制源码保持旧路径；
- 建立一个 compatibility module，继续通过 `crate::old_path::*` 暴露所有旧实现；
- 建立 alias/wrapper crate 只为了让旧 import 继续编译；
- 为消灭 E0603 将整个 module tree、所有 struct field 和 helper 机械改成 `pub`；
- 在移动阶段反复 `cargo check`，按单条错误临时修改，最后留下半成品 adapter；
- 把所有当前 dependencies 原样复制到每个新 crate；
- 把 provider、permission、settings、plugin、session 全部合成一个新的“studio/common”大 crate；
- 为了让测试通过删除原有测试、降低断言、绕过真实 effect 或屏蔽 Clippy；
- 把 `RuntimePresentationEvent`、`Backend`、`TerminalRuntime` 直接暴露为每个 feature crate 的公共核心 API；
- 让 transcript state 继续拥有 composer draft，或让 session controller 继续依赖 transcript 的内部缓存。

## 22. 推荐的实际执行顺序摘要

```text
1. 重新生成 rust-workspace-analysis.md，确认基线和工作树
2. 清理 lib.rs wildcard，标记 feature owner 和 app adapter
3. 定义 TranscriptLiveUpdate / TranscriptAction / TranscriptEffect
4. 定义 PluginWorkbenchSnapshot / Action / Effect
5. 一次创建六个新 crate 的 manifest/root（不引入 app 反向依赖）
6. git mv transcript 纯 owner
7. git mv plugin workbench 纯 owner
8. 按 rg 清单批量改所有 crate:: / super:: / pub(crate) / test path
9. 静态检查旧路径、旧 owner、manifest 和依赖方向
10. 第一次 cargo check：先 affected crate，再 agena-tui-app
11. 按路径 → visibility → protocol/type → feature → test 分组修复
12. 完成 session controller 和 AppMessage session 分割
13. 连续完成 provider / permission / settings vertical slices
14. 再决定 composer 是否独立
15. cargo fmt/check/test/clippy 全量验证
16. 重新生成 rust-workspace-analysis.md，检查真实 package/module 边界
```

本计划的完成标准不是“所有目录都搬完”，而是 `agena-tui-app` 的职责被压缩为真实的应用壳，feature crate 可以在没有 `App` 的情况下单独测试，并且整个迁移过程仍然通过一次连续的移动、批量改写和最后统一验证快速完成。
