# Agena 系统测试与内置插件测试

这些用例由普通 `cargo test` 运行。被测对象是产品的 `agena server`，AI 回复由
本地脚本模型构造，用户消息通过正式客户端进入产品。插件的工具执行、权限、
Host 回调、配置、持久化和生命周期均走生产实现。

## 运行

在仓库根目录：

```sh
# 每个正式注册的内置插件都有独立行为测试，另有注册清单覆盖检查。
cargo test-bundled

# 只运行某一用例，名称过滤交给 Rust 原生测试框架。
cargo test-bundled fs_edits
cargo test-bundled language_server
cargo test-bundled chatgpt

# 测试库自身、保留的纯契约/故障测试，以及通用产品场景。
cargo test -p agena-testkit --lib
cargo test -p agena-bundled-plugins --lib
cargo test-system

# 精确冻结/推进时间、Cron 边界、任务竞争和重启。
cargo test-clock

# SQLite 提交门闩、磁盘满、读写/sync 故障、进程与模型连接中断。
cargo test-faults
# 旧控制场景过滤入口仍保留。
cargo test-control

# Anthropic、Gemini、Bedrock、Ollama、OpenAI Chat 原生协议。
cargo test-native

# 全部 Rust workspace 库级契约测试（不重复执行系统集成目标）。
cargo test-libs
```

`test-bundled` / `test-system` 是 Cargo alias，默认同时运行两个 Rust 测试用例。
可以直接使用完整命令调整并发：

```sh
cargo test -p agena --test bundled_plugins -- --test-threads=2 --nocapture
```

无需 AI API key、真实账号或远程模型。HTTP 协议依赖由测试里的 Rust 服务提供；
LSP 和 MCP stdio 用例需要 `python3`，它是产品通过正式配置启动的本地协议对端。
Shell 用例使用当前 Unix 环境的 `bash` 和常见基础命令。测试入口始终是 Rust
测试函数，已有产品 binary 由 Cargo 自动构建。

## 不经过 AI 的正式产品入口

`agena-testkit` 不只测试聊天和插件。下面这些也都是普通 Rust integration test，
使用现有的 `cargo test-system` 运行：

| 系统入口 | 推荐 API | 真实场景 |
|---|---|---|
| Git 工作台 | `case.git().post("init", ...)`、`in_directory(...)`、`get_with_query(...)` | [`scenarios/git.rs`](scenarios/git.rs) |
| 文件工作台 REST | `case.fs_workbench().post("write", ...)`、`get_with_query("read-chunk", ...)` | [`scenarios/workbench_fs.rs`](scenarios/workbench_fs.rs) |
| 正式 CLI／插件市场 | `case.run_cli(["plugin", "list-installed"])` | [`scenarios/marketplace.rs`](scenarios/marketplace.rs) |
| 真实 PTY／终端 SSE | `case.terminal().create(case.workspace())`、`input(...)`、`wait_output_contains(...)` | [`scenarios/terminal.rs`](scenarios/terminal.rs) |
| 最终文件副作用 | `case.wait_file_bytes("workspace/file", expected)` | [`scenarios/terminal.rs`](scenarios/terminal.rs) |
| 精确 TestClock／调度竞争 | `.test_clock(initial)`、`advance`、`pause_scheduler`、`wait_observed` | [`scenarios/clock.rs`](scenarios/clock.rs) |
| FaultDriver／SQLite 提交与文件 I/O 故障 | `.kernel_faults()`、`arm`、`disk_full`、`fail_syscall`、`power_cut` | [`scenarios/control.rs`](scenarios/control.rs) |
| 进程与定向模型连接故障 | `kill`、`restart`、`disconnect_model`，预期崩溃生命周期检查 | [`scenarios/fault_lifecycle.rs`](scenarios/fault_lifecycle.rs) |
| 多协议 ModelDriver | `.model_protocol(ModelProtocol::Gemini)` 与通用 `Reply`／`RequestMatch` | [`scenarios/native.rs`](scenarios/native.rs) |

这些 API 只是安全的协议便利层：正常请求仍进正式 Agena HTTP/CLI，非法请求、
未知路由和未来新增参数仍可用 `case.http()` 的原始请求入口测试。`run_cli` 另起
测试自己拥有的产品进程，隔离 HOME、数据目录及继承环境，并有输出大小、超时与
进程树回收限制。插件模板测试为了避免联网解析 SDK，可向产品 CLI 传
`plugin init --skip-lockfile`；真实发布前仍必须生成并提交 `Cargo.lock`。

终端 SSE 不能使用会一直缓存完整响应的 `http.execute()`；使用
`TerminalHandle::wait_output_contains()` 或 `http.open_stream()` 显式处理流和期限。
终端命令通常会回显输入，**回显不代表命令已执行**：必须检查它写出的真实文件
或其他副作用。`wait_file_bytes()` 就是为这个时序提供的有界断言。

截至 2026-11-07 的完整 crate 级静态覆盖盘点与待补范围见
[`docs/testing/coverage-audit.md`](../../../docs/testing/coverage-audit.md)。
并且 `test-system` 内有一个不启动 Server 的普通 Rust 覆盖清单门禁：新增 Rust package
未加入盘点表时，该测试会失败，防止遗漏新增模块。

## 最小工具测试

```rust
use agena_testkit::{TestAgena, TestResult};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_read() -> TestResult {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .seed_file("workspace/example.txt", b"fixture-content".to_vec())
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let result = user
                .call_tool("fs.read", json!({"file_path": "example.txt"}))
                .await?;
            result.assert_succeeded()?;
            result.assert_text_contains("fixture-content")?;
            assert!(result.data()?.to_string().contains("fixture-content"));
            Ok(())
        })
        .await
}
```

`call_tool` 自动注册两次模型交互：AI 请求真实工具调用，随后接收匹配 `call_id`
的工具结果并回复结束标记。它等待精确执行代次完成，并通过正式 HTTP API 获取
该次调用的持久化结果。它保留会话原有权限；需要允许写入、进程或网络操作时，
用例显式设置权限。`bundled/mod.rs::allow_permissions` 是这批本地 fixture 使用的
明确策略，网络只允许 loopback。

需要处理交互时使用 `start_tool`：

```rust
let pending = user.start_tool("interaction.ask", input).await?;
let request = pending.wait_user_input().await?;
request.answer(answers).await?;
let result = pending.wait_settled().await?;
result.assert_succeeded()?;
```

`start_tool_with` 可以传入正式 `RunOptions`。子 Agent、后台通知、定时任务等需要
多个并发模型决策的场景，使用 `case.model()` 的显式脚本和门闩；runtime 用例提供
完整示范。

`pending.wait_permission()`／`wait_user_input()` 会在工具提前失败时补充模型实际
收到的 failure kind/code，原始执行错误仍在错误链中。跨工作区用
`case.user().new_session_in(absolute_path)`；附件字节用
`user.download_attachment(result.attachment(0)?).await?`，它经过正式媒体 API 并
在读取期间限制大小。设置的异步生效用 `case.settings().wait_loaded(...)`；明确
要求新运行快照时用 `reload_and_wait()`，已有插件 reload receipt 则用
`track_reload(task_id, &before_snapshot).wait()`。

### 选择准确的断言对象

| API | 含义 |
|---|---|
| `result.assert_succeeded()` / `assert_failed()` | 真实工具结果的 phase |
| `result.assert_failure_kind(ModelFeedbackKind::PermissionDenied)` | 同时检查失败 phase 和实际模型收到的结构化失败类型 |
| `result.failure()` | 解析完整 `Failure`，用于检查 category、impact、retry 等契约 |
| `result.output` | 下一次 AI 请求实际携带的完整结果 envelope |
| `result.text()` / `assert_text_contains()` | AI 实际看到的模型投影文本 |
| `result.json()` | 仅当模型投影本身是完整 JSON 时解析它 |
| `result.data()` | 通过公开工具详情和 content API 读取的原始 payload |
| `result.raw_output` | 原始输出、资源引用、附件等结构 |
| `result.part` / `result.run` | 本次持久化工具 part 和精确运行结果 |

模型投影可能带 preview notice 或只保留摘要。ID、revision、receipt、文件 hash
等结构化断言优先使用 `data()`。text-only 工具的原始 JSON 文本通常位于
`data()["text"]`。引用内容会在读取时恢复，便利 API 的总数据上限为 4 MiB；更大
数据可以使用完整客户端的流式读取。

权限测试应同时检查类型和副作用，例如正式审批后调用
`result.assert_failure_kind(agena_testkit::ModelFeedbackKind::UserDeclined)?`，
再断言目标文件不变、服务端没有收到请求。不能用任意失败代替权限生效。

### 配置内置插件

```rust
TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
    .builtin_plugin("agena.settings", json!({
        "reads": {"source": "file", "file_layer": "workspace"},
        "edits": {"file_layer": "workspace", "reload_after_write": false}
    }))
    .workspace_config(json!({"session": {"max_turns": 6}}));
```

`builtin_plugin` 写入完整的正式配置项，包含必填的 `package: {kind: "static"}`。
HTTP／stdio 插件以及故意构造的非法配置，继续使用原始配置入口。需要在产品启动
前准备本地服务、动态 URL 或协议文件时使用 `before_start`；运行中修改设置使用
`case.settings()`，并检查实际生效结果。

## 覆盖清单

需要编写类似测试时，可以从这些真实场景开始：

| 需求 | 示例源码 |
|---|---|
| 跨工作区、附件传输限制、审批/提问与失败诊断 | [`scenarios/tools.rs`](scenarios/tools.rs) |
| 等待实际生效值、精确重载 receipt、失败重载及重启边界 | [`scenarios/settings.rs`](scenarios/settings.rs) |
| AI 可见文本暂停、继续、取消和 UTF-8 分片 | [`scenarios/conversation.rs`](scenarios/conversation.rs) |
| 外部取消测试 future、取消 finish 等待者、产品和 WebSocket 回收 | [`scenarios/lifecycle/ownership.rs`](scenarios/lifecycle/ownership.rs) |
| HTTP 插件 Host 回调、设置与生命周期、候选配置发布前的旧任务 | [`scenarios/plugins.rs`](scenarios/plugins.rs) |

实际注册表包含 24 个内置插件，包括没有 AI 工具的通知插件。`CASES` 将插件 ID
绑定到实际 Rust 测试函数；`every_registered_builtin_has_a_behavior_test` 与产品
`bundled_plugin_entries()` 比较。新增或移除内置插件会要求同步测试清单。

当前 `bundled_plugins` 目标有 74 个测试（73 个行为场景和 1 个注册覆盖检查），
`system` 目标还覆盖测试驱动本身的真实运行场景，包括跨工作区、附件限制、设置
重载、流式文本门闩、外部取消和资源回收。权限、协议和生命周期场景直接作为普通
Rust 测试发现，按名称筛选即可，不需要再维护第二套测试入口。

| 插件 | 系统场景主要断言 |
|---|---|
| fs | 真实文件读写、hash、替换、patch、检索、过期版本拒绝 |
| shell | 前台读写与退出码、后台日志、会话所有权、停止 |
| code | AST 查询、语法树、预览和实际改写、revision |
| commands | 工作区命令包安装、别名和资源、路径边界、移除 |
| skills | 创建更新、资源 CRUD、revision、启停、删除 |
| memory | 存储搜索、revision、重启持久化、retrieval 设置、真实模型上下文、第二工作区和记录/索引权限 |
| notebook | cell 更新插入删除、输出清理、ID/metadata、错误输入 |
| inline_ui | guide/schema、prepare/read/update、HTTP/SSE、revision、会话边界、错误修复和字段上限 |
| report | finding 的位置、severity、confidence、持久化结构、错误范围 |
| settings | 文件层和生效值、插件默认设置、dry-run、revision、校验、重载、删除 |
| provider | 当前 provider 的配置模型、分页、查询、详情、无效 cursor |
| notifications | 活动状态、完成与提问、无 SSE 观察者、pending 恢复、解决状态、迟到重放、禁用重载、并发状态、重启 |
| session | 实际会话、rename、环境、可执行文件、模型、token 信息 |
| interaction | 通知、实际问题和答案、取消、pending 生命周期 |
| plan | 计划版本、review、阶段、真实工具执行门禁和批准、完成清理 |
| cron | CRUD、暂停恢复、真实定时触发、投递历史、会话隔离 |
| agent | 真实子会话模型请求、消息、等待、followup、取消 |
| monitor | 本地 WebSocket、事件投递、停止和连接回收 |
| chatgpt | Responses、模型目录/重载、原生资源、multipart、receipt、HTTP 错误、WebSocket、SSE、异步及并发计费 |
| claude | Messages、模型选择、原生资源、文件、batch 按模型计费、receipt、HTTP 错误 |
| gemini | Interactions、原生资源、resumable upload/跨 origin 拒绝、WebSocket、batch、receipt、用量归属 |
| web | SearXNG 搜索、筛选去重、部分错误、真实审批拒绝、私有 fetch、browser_open 输入/权限边界 |
| mcp | HTTP/stdio 握手与调用、资源/提示、连接 CRUD、密钥保留/脱敏、读写/进程权限、reload 与子进程退出 |
| lsp | 本地 stdio、hover/definition/references/diagnostics、文档版本、进程回收 |

业务行为以这些系统场景为主。旧的直接插件调用、Fake Host 和 Fake Provider 测试，
在等价真实覆盖落地后删除；迁移对照、退役断言和保留边界见
[`MIGRATION.md`](MIGRATION.md)。纯编解码、脱敏边界以及必须精确注入的内部故障
仍保留聚焦测试。具体网络厂商的兼容性、
Web DOM、真实浏览器推送、终端渲染和其他操作系统仍有对应的专项测试边界。

### HTTP 插件重载覆盖的边界

`scenarios/plugins.rs` 中的候选配置门闩验证：插件配置未变、物理实例复用时，
旧任务在新配置发布前仍能执行工具并完成真实 Host 回调，新配置发布后也能调用。
SDK/Host 协议测试另检验绑定的退休、取消、失败补偿和权限归属。

这项保证针对带有调用归属的请求及其流式工作。插件自行创建、没有调用上下文的
后台回调仍使用最近登记的默认绑定；候选构建可能先切换这个默认值，候选失败后
再回退。它尚不具备与 runtime snapshot 发布原子同步的语义。插件配置变化导致的
远端重新初始化也仍遵循 HTTP 单实例 owner 协议，不能把这组复用测试解释为任意
多个配置版本的远端实例可同时存活。上述场景需要各自的生命周期设计和回归。

## 失败诊断与资源所有权

失败会输出该 case 的诊断目录，其中包含真实配置、产品 stdout/stderr、模型请求
及脚本错误。优先检查首个业务失败，然后检查 cleanup 附带的未消费脚本或资源错误。
`builder.run` 负责等待产品和登记过的服务收尾；使用 `start` 时须显式 `finish`。

测试的所有服务、协议进程、文件都属于独立 case。需要保留成功用例的产物时设置
`keep_artifacts(true)`。未声明的模型请求会使测试失败；后台功能应明确注册它可能
产生的模型工作，不能添加永远成功的通配响应掩盖额外调用。

## 持续集成

`.github/workflows/quality.yml` 的独立 `system-tests` job 在 Linux 上运行测试库、
客户端、插件 Host、SDK HTTP/stdio、运行时会话和保留的内置插件契约测试，然后执行
`cargo --locked test-system` 与 `cargo --locked test-bundled`。这两个 alias 的
并发上限都是 2。配置的 job 总时限为 60 分钟。

测试步骤把 `TMPDIR` 指向 runner 的专用临时目录。失败时上传保留下来的
`agena-testkit-*/diagnostics/**`，保存 7 天，包含产品日志、模型交互和配置诊断；
它们用于定位具体失败，不把整个临时工作区作为构建产物上传。

完整测试库契约和平台边界见
[`agena-testkit/README.md`](../../../crates/agena-testkit/README.md)。
