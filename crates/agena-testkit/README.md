# Agena Rust system-test kit

`agena-testkit` 是普通 Rust 测试使用的系统测试库。测试拥有一个隔离的真实
`agena server` 进程、一个本地脚本模型、临时文件和辅助服务。用户消息通过正式
`AgenaClient` 进入产品；模型的文本和工具调用通过真实 HTTP / Responses / SSE
协议返回。提供商适配器、工具执行、权限、插件、运行时配置和持久化使用产品实现。

测试用例仍然是 `#[tokio::test]`，由 `cargo test` 发现和运行。
测试库没有自己的 runner，也不会在测试函数里偷偷调用 Cargo。

## 快速开始

在包含产品 binary 的 `apps/agena` 包中：

```rust
use agena_testkit::{Reply, RequestMatch, TestAgena, TestResult};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_can_chat() -> TestResult {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .run(|case| async move {
            case.model()
                .expect(RequestMatch::user_text("你好"))
                .reply(Reply::text("这是测试自己构造的回答"));

            let user = case.user().new_session().await?;
            let run = user.send("你好").await?;
            let result = run.wait_settled().await?;

            result.assert_succeeded()?;
            result.assert_assistant_text("这是测试自己构造的回答")?;
            Ok(())
        })
        .await
}
```

运行仓库已经提供的完整场景：

```sh
cargo test -p agena-testkit --lib
cargo test-system
cargo test-bundled
```

可以按 Rust 测试名称过滤：

```sh
cargo test -p agena --test system model_tool_call
cargo test -p agena --test system http_plugin
```

场景源码在
[`apps/agena/tests/scenarios`](../../apps/agena/tests/scenarios)。
它们同时作为使用示例和产品回归测试。

`test-system` 和 `test-bundled` 是仓库的 Cargo alias，默认测试并发为 2。
后者覆盖全部正式注册的内置插件，并校验新注册的插件都有行为测试。
可用 `cargo test-bundled fs_edits` 按名称过滤；离线运行写作
`cargo --offline test-bundled`。完整覆盖清单见
[`apps/agena/tests/README.md`](../../apps/agena/tests/README.md)。

### 更短的工具测试

只需要让 AI 调用一次工具时，可以使用 `UserSession::call_tool`：

```rust
use agena_testkit::{TestAgena, TestResult};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reads_a_real_file() -> TestResult {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .seed_file("workspace/example.txt", b"hello from disk".to_vec())
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let result = user
                .call_tool("fs.read", json!({"file_path": "example.txt"}))
                .await?;
            result.assert_succeeded()?;
            result.assert_text_contains("hello from disk")?;
            assert!(result.data()?.to_string().contains("hello from disk"));
            Ok(())
        })
        .await
}
```

这个方法自动构造 AI 的工具调用及读取工具结果后的结束回复，仍然经过真实用户
请求、模型 HTTP 协议、工具分发、权限和持久化。它不修改会话权限。需要权限确认、
用户输入或取消时，使用 `start_tool`。可以直接调用返回值的 `wait_permission()`／
`wait_user_input()`，最后调用 `wait_settled()`；`start_tool_with` 还可传入正式
`RunOptions`。如果工具在生成审批前已失败，这两个交互等待会保留原始错误链，并
追加工具名、call ID 和模型实际收到的 failure kind/code/说明，避免只报“没有审批”。
需要直接操作执行句柄时仍可调用 `run()`。

`ToolResult::assert_succeeded()` 和 `assert_failed()` 检查实际工具 phase。
检查具体失败原因时，使用
`result.assert_failure_kind(agena_testkit::ModelFeedbackKind::PermissionDenied)?`，
或 `result.failure()?` 读取模型实际收到的完整结构化 `Failure`。它们不会把任意
工具失败当作预期的权限拒绝；缺失结构化失败也会报错。
工具失败可以是成功结束的 AI 回合中的预期结果；整个回合失败会由等待方法报错。
`output` 保存下一次模型请求中的真实结果 envelope，`text()` 是模型可见文本；
`data()` 通过正式 HTTP 工具详情及 content API 还原原始结构化 payload。
`raw_output`、`part` 和 `run` 分别提供完整输出结构、持久化工具 part 和精确运行结果。
模型投影可能包含预览提示，只有本来就是完整 JSON 时才使用 `json()`；ID、revision
和 receipt 等断言应使用 `data()`。text-only 工具中的 JSON 文本通常在
`data()["text"]`，需要显式解析。引用内容的自动读取总上限为 4 MiB，更大内容使用
完整客户端的流式读取接口。

配置内置插件可用 `.builtin_plugin("agena.settings", settings)`，它会写入
包含 `enabled: true` 和 `package: {kind: "static"}` 的完整正式配置项。
HTTP／stdio 插件、自定义配置层和非法配置测试仍使用现有原始配置入口。
多工具决策、后台通知和子 Agent 场景继续使用下面的显式模型脚本。

### 多工作区与附件

使用显式工作区创建用户会话，无需重复写 ResolveWorkspace、创建和订阅流程：

```rust
case.seed_file("second-workspace/input.txt", b"second workspace").await?;
let second = case.paths().root.join("second-workspace");
let user = case.user().new_session_in(&second).await?;
```

路径必须是 Agena 进程可见的绝对路径；目录和输入文件仍由用例准备。
`new_session_in_with` 同时接受 `SessionOptions`。这些方法保持产品的默认工作区，
并在新会话发送消息前建立正式订阅；用例应优先使用自己 case 下的目录。

拿到带附件的 `ToolResult` 后，可以直接验证正式媒体传输的字节：

```rust
let attachment = result.attachment(0)?;
let (filename, bytes) = user.download_attachment(attachment).await?;
assert_eq!(bytes, b"expected fixture bytes");
```

`attachments()` 返回完整附件列表。下载会通过当前会话的正式媒体 API 处理本地
路径或 opaque `agena:media:` 引用，不解析内部 token，也不在测试进程直接读文件。
错误会标明会话和附件；服务端继续验证工作区权限与历史归属，其他会话不能借用
opaque 引用。默认在传输期间限制为 4 MiB，使用 `download_attachment_with_limit`
可显式调整，但仍受客户端 100 MiB 硬上限约束；等待沿用 `UserSession::with_timeout`。
外部 URL 和不能通过 session media API 下载的附件来源会明确拒绝。

### 在独立插件 crate 里使用

添加 `agena-testkit`、`tokio` 和插件本来需要的 SDK 开发依赖即可。
`CARGO_BIN_EXE_agena` 只在拥有该 binary 的包的集成测试中由 Cargo 提供；
独立插件包应明确传入已经构建好的**普通 Agena 产品**路径：

产品与 testkit 应使用同一份源码或兼容版本。测试库依赖带 `message_id`、
`attempt_generation` 的执行回执和精确 attempt 查询接口；旧 server 缺少这些
身份信息时会明确失败，不会猜测执行归属。

```rust
let binary = std::env::var_os("AGENA_TEST_BINARY")
    .expect("set AGENA_TEST_BINARY to the built Agena product executable");
TestAgena::builder(binary).run(|case| async move {
    // 注册本地模型脚本、加载插件、通过用户驱动进行断言。
    Ok(())
}).await?;
```

例如先执行 `cargo build -p agena`，再运行：

```sh
AGENA_TEST_BINARY=/absolute/path/to/target/debug/agena cargo test -p my-plugin
```

这里没有另建测试专用 binary；被测进程就是产品发布时使用的入口。

## 一次测试实际走过什么

1. 为本次 case 建立独立的数据目录、工作区、home、临时目录和诊断目录。
2. 启动仅监听 loopback 的脚本模型，写入明确的测试配置文档，执行显式注册的启动前准备。
3. 启动前台产品 `agena server`，等待正式服务身份、健康和 runtime 可用。
4. 使用真实密码登录和公共客户端创建会话，先订阅事件，再发送消息。
5. 产品构建真实模型请求。测试匹配请求并返回自己定义的模型响应。
6. 产品解析响应、调度工具、处理权限和插件，并把工具输出送入下一次模型请求。
7. 测试检查本次执行的结果；case 最后等待关闭并检查未消费脚本和后台任务错误。

默认配置选择 `testkit/openai_responses/fake-model`，只指向该 case 的本地模型，
使用假的内联凭据，关闭公开模型目录和客户端版本自动更新，不继承用户的模型凭据。
不需要真实 AI 服务，也不需要模型 API key。

`raw()`、完整配置替换和原始客户端是刻意提供的扩展入口。它们可以恢复产品默认
行为或指定其他服务；例如完全空配置会保留生产代码默认的后台版本更新设置。
测试库不提供操作系统网络沙箱。自定义模型、OAuth、MCP 等配置应指向自己拥有的
本地 fixture，不能把真实模型服务配置带入测试。

## 自己构造模型工作过程

### 工具调用与真实工具结果

```rust
let model = case.model();
let call = model
    .expect(RequestMatch::user_text("读取文件").declares_function("tools_call"))
    .reply(Reply::tool_call(
        "call_read_1",
        "fs.read",
        serde_json::json!({"file_path": "note.txt"}),
    ));

model
    .expect(RequestMatch::tool_output("call_read_1"))
    .after(&call)
    .check_tool_output_contains("call_read_1", "文件里的真实内容")
    .reply(Reply::text("已经读完"));
```

`Reply::tool_call` 生成真实 Responses `function_call`，外层函数是
`tools_call`，参数是包含工具名和输入的 JSON 字符串。它不会直接调用 Rust
工具实现。输出项目的 ID 和工具 `call_id` 分开构造，后续检查按 `call_id`
查找真实 `function_call_output`，不靠提示词中是否出现某个字符串判断成功。

如果需要更底层的控制，可以使用：

- `Reply::function_call`、`function_calls`、`function_call_raw`。
- `.arguments_chunks(...)`、`.fragment_bytes(...)` 测试分片解析。
- `Reply::approve(...)`、`Reply::block(rule, reason)` 构造审批模型调用。
- `Reply::raw_json`、`raw_body`、`raw_stream` 测试状态码、头、错误和坏流。
- `.usage(Usage::tokens(...))`、`.incomplete(...)`、`.end_turn(...)` 控制响应元数据。

默认协议仍是 OpenAI Responses。通过 `builder.model_protocol(...)` 选择协议后，
`case.model()` 返回的 `ModelDriver`（兼容名称 `ModelHandle`）使用相同的
`Reply::text`、`text_chunks`、`with_thinking_chunks`、`tool_call`、`function_calls`、
`usage` 和暂停／分片 API，按实际请求编码为该协议的 JSON 或流。

`RequestMatch::user_text`、`model`、`streaming`、`declares_function`、
`tool_output` 与 `ModelRequest` 的工具结果读取也理解原生请求结构。
Bedrock、Gemini 等把模型名或流式操作放在 URL 的协议无需自写字符串匹配。

`ModelRequest::raw_uri()` 保留路径及原始 query，`query_pairs()` 保留顺序和重复项，
`query_values(name)` 提供解码后的值。`RequestMatch::query_parameter(name, value)`
要求恰好一个匹配项，适用于 Gemini 的 `alt=sse` 等协议参数。
Query 与 HTTP 认证头可能含凭据：请求匹配可读取实际值，但默认 Debug/错误诊断
只显示参数名、header 名而不显示它们的值。自定义前缀路由使用
`ModelRoute::post_prefix(path).for_protocol(protocol)` 明确声明协议；
选择单一协议后，其他协议的默认 HTTP 路由不会被偷偷放行。

Ollama 原生协议不保留脚本指定的 call ID；使用产品实际生成的 ID（读取器支持
工具内容开头的 `tool_call_id=...` 标记），或使用原生请求自定义 matcher。
其余已支持协议保留工具关联 ID，严格工具结果检查仍拒绝重复和缺失输出。
Gemini/Ollama 工具参数使用完整对象，`arguments_chunks` 会明确拒绝；
需要逐项控制异常或不完整事件时，使用下面的 `ModelEvent` 原生事件接口。

重要：合法 HTTP EOF 不等于模型消息完成。Anthropic/Bedrock Anthropic 要求
`message_stop`，Ollama 要求 `done: true`，Gemini 的流式回复要求候选终态；
只有前缀内容、没有结束事件的回复会使真实模型执行失败，而不是合成成功。
这类回归位于 `scenarios/native.rs`。


| ModelProtocol | 消息与流 | 真实产品适配器 |
| --- | --- | --- |
| `OpenAiResponses` | Responses JSON / SSE | 支持 |
| `OpenAiChat` | Chat Completions JSON / SSE | 支持 |
| `Anthropic` | Messages JSON / SSE | 支持 |
| `Gemini` | GenerateContent JSON / SSE | 支持 |
| `BedrockAnthropic` | Invoke JSON / AWS 二进制 EventStream | 支持，使用本地假凭据签名 |
| `BedrockConverse` | Converse JSON / AWS 二进制 EventStream | 仅独立 HTTP fixture；产品 builder 在启动前明确拒绝 |
| `Ollama` | Chat JSON / NDJSON | 支持 |

```rust
use agena_testkit::{ModelProtocol, Reply, RequestMatch, TestAgena, TestResult};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_gemini_answer() -> TestResult {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .model_protocol(ModelProtocol::Gemini)
        .run(|case| async move {
            case.model()
                .expect(RequestMatch::user_text("hello"))
                .reply(Reply::text_chunks(["wor", "ld"]));
            let result = case.user().new_session().await?
                .send("hello").await?.wait_settled().await?;
            result.assert_succeeded()?;
            result.assert_assistant_text("world")?;
            Ok(())
        }).await
}
```

完整原生消息使用 `protocol.message(json)`；完整原生事件使用
`protocol.events([ModelEvent::json(...), ModelEvent::named(name, json), ...])`。
所有 JSON 字段原样保留，因此可以构造多模态内容、引用、服务端工具、
reasoning、ping、供应商新增字段及故意不完整的流；驱动不自动补终止事件。
`ModelEvent::bedrock(name, payload)` 表示 AWS 外层事件，
`ModelEvent::exception(name, payload)` 生成带 JSON payload 的标准建模异常帧；
`ModelEvent::error(code, message)` 生成只有错误 headers、payload 为空的标准未建模错误帧，
完整保留错误码和说明。Bedrock 的普通 Anthropic 事件会封装为 base64 chunk，
再生成带 prelude/message CRC 的二进制帧。
`Reply::raw_body` / `raw_stream` 仍可用于畸形字节和任意错误响应。

独立 HTTP 测试用 `ScriptedModel::start_with_options(ModelOptions::for_protocol(protocol))`，
不需要启动产品。原有 `.native_protocol(NativeProtocol::...)`、
`NativeProtocol::text/tool` 保持兼容。产品场景见
`apps/agena/tests/scenarios/native.rs`；完整事件与校验和覆盖在 testkit 单元测试中。

### 精确时间与调度竞争

`builder.test_clock(start_utc)` 启用每 case 私有、默认冻结的 `TestClock`；
原有 `.scheduler_clock(...)` 和 `ClockDriver` 名称保持兼容。

```rust
let clock = case.clock()?;
let boundary = clock.pause_scheduler().await?;
assert_eq!(boundary.active_deliveries, 0);

// 到期任务暂不接纳，可先完成删除/取消/配置修改等竞争动作。
clock.advance(chrono::Duration::hours(6)).await?;
// ...通过正式产品 API 提交竞争动作...
clock.resume_scheduler().await?;
clock.wait_observed().await?;
```

`advance` / `advance_to` 同时推进调度 UTC 和虚拟单调时间；
`freeze(at)` / `set(at)` 只调整冻结 UTC，可模拟墙钟回拨而不倒退已注册的时长。
所有变化精确到毫秒：初始值与后续变更都拒绝不支持的精度、负推进和溢出；
多个 `case.clock()` 句柄共享更新锁，不会丢失并发增量。

`sleep`、`sleep_until`、`sleep_until_elapsed` 和 `timeout` 使用这份虚拟时间。
异步计时从 future 被轮询时注册；跨任务测试应先确认等待者已开始，再推进时钟。
`timeout` 在操作结果与期限同时就绪时优先返回操作结果。产品 HTTP、进程退出、
用例总期限与清理期限仍使用真实时间；不会全局暂停 Tokio，也不替换插件进程的系统时钟。

`pause_scheduler` / `resume_scheduler` 等待调度器确认接纳边界；
已接纳的交付可以继续运行，`active_deliveries` 明确报告其数量。
`wait_observed` 通过新 revision 要求一次新的确认，重启前的旧确认不能满足等待。
确认不等于任务交付完成，最终断言仍检查正式 Cron 历史或执行结果。
Scheduler 的任务到期使用受控虚拟 UTC，租约取得、续租和释放使用单独的真实 UTC
租约时间域，以匹配心跳仍按真实单调计时器运行的行为。因而瞬间跳过 6 小时
不会把仍健康的活跃交付当成丢失租约。这个分离保留了真实运行中的租约恢复，
但并不声称模拟了 6 小时内所有真实 heartbeat；需要测试心跳缺失时，应使用明确的
故障/阶段控制，而不是只调整业务时间。回归覆盖了活跃 sink 期间的时间跳跃。

时钟和暂停状态在同一 case 重启后保留。真实场景见 `scenarios/clock.rs`；
运行 `cargo test-clock`，或用 `cargo test-system` 一并运行。

### SQLite 事务与 I/O 故障

`builder.kernel_faults()` 启用真实 SQLite kernel 提交门闩。运行时：

```rust
use agena_testkit::{FaultEffect, TestAgena, TestResult};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commit_failure_rolls_back() -> TestResult {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .kernel_faults()
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let workspace = user.snapshot().await?.session.workspace_id;
            let faults = case.faults()?;
            faults.arm("kernel.before_commit", FaultEffect::Fail).await?;
            assert!(case.client().await
                .create_session(workspace, "must-not-commit", None).await.is_err());
            assert_eq!(faults.wait_hit().await?.hits, 1);
            faults.clear().await?;
            Ok(())
        }).await
}
```

还可以用 `FaultEffect::Hold` 暂停真正未提交的事务，等待
`faults.wait_hit().await?` 后 `faults.release().await?`；或在门闩上调用
`case.crash().await?`，随后用 `case.restart().await?` 验证 SQLite WAL 恢复。
如需由故障点自行触发真正的 SIGABRT，应**先**调用
`case.arm_expected_kernel_crash().await?`（它也会武装 Crash 门闩），
再触发真实写入，用 `case.reap_expected_kernel_crash().await?` 等待、验证并回收
异常退出，最后 `faults.clear().await?; case.restart().await?`。
这样预期的进程崩溃不会被正常的 unexpected-exit 监视器误判，也不会静默接受
正常退出。示例见 `scenarios/control.rs`。
这些是明确启用的 **debug-build 专用** 故障入口。正常 Release 构建不注册
故障 VFS，也不读取注入控制变量。完整产品回归在 `scenarios/control.rs`
和 `scenarios/fault_lifecycle.rs`，运行 `cargo test-faults` 或 `cargo test-system`。

同一个 `FaultDriver` 还提供以下操作：

| API | 精确作用 |
| --- | --- |
| `kill()` / `restart()` | 杀死或重启当前 case 拥有的产品进程，保留该 case 的隔离数据 |
| `power_cut()` | 只在已命中且未释放的 `kernel.before_commit` Hold 上杀死产品，用真实 WAL 验证未提交事务恢复 |
| `disconnect_model(&step)` | 切断当前 case 中指定请求的真实 HTTP 连接；拒绝其他 case 的 step |
| `disk_full()` | 下一次实际数据库／WAL 写操作返回 `SQLITE_FULL` |
| `fail_syscall(FaultSyscall::...)` | 下一次指定 SQLite VFS 操作返回对应真实错误码 |

`FaultSyscall` 包括 `Read`、`Write`、`Sync`、`Truncate` 和 `Open`。
这些故障位于数据库连接使用的命名 VFS，其他 case 与默认 VFS 不受影响；
它们不会填满宿主磁盘，也不是全进程任意系统调用拦截器。
`wait_hit()` 返回的 `sqlite_code`、`file`、`arm_id` 可证明具体命中的
SQLite 错误码、文件和这一次武装的身份。

```rust
let faults = case.faults()?;
faults.disk_full().await?;
// 通过正式产品 API 发起真实数据库写入并断言它失败。
let hit = faults.wait_hit().await?;
assert_eq!(hit.sqlite_code, Some(13)); // SQLITE_FULL
faults.clear().await?;

faults.fail_syscall(agena_testkit::FaultSyscall::Sync).await?;
// 再发起需要落盘的写入，检查 SQLITE_IOERR_FSYNC 和恢复结果。
```

磁盘满／写失败测试验证事务没有幽灵数据、旧数据保留、重开后可继续写入。
VFS 提交错误可能使产品将 writer 标记为 CommitUnknown 并关闭；这类恢复测试显式
使用 kill/restart 模拟重启，不忽略正常关闭的错误。同步失败可能发生在数据已经
写出之后，提交结果不确定；相应测试验证旧数据、
`integrity_check` 和恢复后的可写性，不把“收到 I/O 错误”解释成“一定没有提交”。

Case 收尾会检查还武装但没有命中的故障（`remaining > 0`）及已命中但未解除的
`kernel.before_commit` Hold。故障测试不能仅 arm 后返回成功。如果故意取消一次
尚未命中的注入，需显式调用 `faults.clear().await?` 解除。`wait_hit()`
继续证明具体 arm 身份和唯一命中，不应省略。

每次注入只消费一次。跨进程更新通过稳定侧文件锁串行化，控制文件本身仍原子替换。
未消费的 arm 和未释放的 Hold 不允许直接覆盖；等待者会拒绝把新 arm 的命中当成旧 arm。
`release()` / `clear()` 解除旧 Hold 后可以立即重新武装；它们不承诺原事务已完成，
仍须等待原请求。预期的 kernel abort 必须先 arm、再观察并 reap，重启和清理会检查
这个顺序；普通 kill 不能被误认成已命中的 kernel abort。

模型连接中断也由事件门闩定位：

```rust
let step = case.model().expect(RequestMatch::user_text("断开此请求"))
    .reply(Reply::text_chunks(["已到达", "尚未到达"]).hold_after_text_chunks(1));
let run = user.send("断开此请求").await?;
run.wait_text_contains("已到达").await?;
case.faults()?.disconnect_model(&step).await?;
step.disconnected().await?;
```

Case 收尾现在区分正常完成与显式验收的断开：
`step.disconnected().await?` 在观察到断开后将其认定为预期。
若测试正是要验证产品 shutdown 主动中断一个仍挂起的模型调用，可以在
`step.observed().await?` 后、case 关闭前调用 `step.expect_disconnection()?`：
收尾不仅允许这次断开，还会验证它**确实发生**；若回复正常完成反而报错。
其他没有声明的模型断开（例如测试体过早返回导致 cleanup 被动取消）会令 case 失败。

这会产生真实传输错误。响应前的错误可经过产品正常重试；
已输出后的处理遵循适配器的正式重放策略，测试必须显式脚本化预期重试或验证失败。
已完成的 step 无法误伤后来复用同一 keep-alive 连接的请求。

`Reply::block` 的 `rule` 使用正式自动审批分类名称，例如
`Destructive or irreversible damage`；它不是文件权限 pattern。
若传入非法分类，产品会按实际审批逻辑降级处理，fixture 不会替它强制拒绝。

### 匹配、检查与并发

匹配器支持用户文本、模型名、流式标志、工具声明、请求头、JSON pointer，
以及 `RequestMatch::new(label, closure)` 自定义条件。复杂断言使用
`.check(|request| -> anyhow::Result<()> { ... })`。

未匹配请求会报告有界的用户/工具摘要，以及相近脚本中没有满足的条件，例如 model、
stream、header 名或 JSON pointer。长工具目录不会把这些信息挤到日志之外；header
条件不输出期望或实际 header 值。自定义组合条件可以用 `.and_named("purpose", ... )`
提供可定位的名称。

`ModelRequest::require_tool_output(call_id)` 要求恰好一条匹配的工具结果且包含 output。
`check_tool_output_contains` 和 `UserSession::call_tool` 的结果捕获都使用这项严格检查。
原有 `tool_output`／`tool_output_text` 保持 Option 读取语义，适合查看请求；需要证明
工具结果唯一时使用严格方法。

每一步只能消费一次。无匹配、多重匹配、检查失败和用户 matcher/check panic
都会记录为 case 错误，没有“未写脚本就返回成功”的默认分支。

独立会话的脚本按请求内容匹配，没有共享 FIFO 假定。用不同用户标记或
`call_id` 区分并发请求：

- `.after(&step)` 要求前一步 HTTP 响应已经完成或断开。
- `.after_observed(&step)` 只要求前一步请求已经匹配，适用于并发依赖。
- 重试、标题生成、审批模型或其他额外模型请求都需要显式注册脚本。
- `ModelOptions` 控制模型列表、等待时限、请求／响应大小、连接数和日志上限。

`model.requests()` 提供有界诊断快照。大请求的检查仍使用完整的、受
`max_request_bytes` 限制的 body；保存的快照可能带
`body_truncated = true`，此时应查看预览或在 `.check` 中断言。

### 流式、暂停和取消

```rust
let step = case.model()
    .expect(RequestMatch::user_text("流式测试"))
    .reply(Reply::text_chunks(["已经收到的内容", "还没发送的内容"])
        .hold_after_text_chunks(1));

let run = user.send("流式测试").await?;
run.wait_text_contains("已经收到的内容").await?;
run.cancel().await?;
run.wait_settled().await?.assert_cancelled()?;
step.disconnected().await?;
```

`.held()` 在发送响应前等待；`step.release()` 释放暂停。
`.hold_after_text_chunks(n)` 在第 n 个可见文本 delta 后暂停，reasoning、协议事件和
`.fragment_bytes(...)` 的字节分片不会改变这个位置；适合测试用户已经看见部分文字
之后的取消或继续。`.hold_after_chunks(n)` 则按分片后的 HTTP body chunk 计数，
适合验证协议中断。两种计数方式不能同时使用，文本暂停也不能叠加 `.held()`。
文本暂停要求有足够文本的脚本化流式回复，各协议均按可见文本事件计数；
非流式、原始 body 或越界计数会明确报错。
这些控制点由实际请求、响应和连接状态驱动，不靠固定 sleep 猜测时序。

## 用户、执行与交互

`case.user().new_session()` 创建真实工作区中的会话。
`new_session_with(SessionOptions)` 可以设置标题、父会话和自动标题开关；
`attach_session(id)` 用于已存在的会话，特别是重启之后。

`send` 使用 `RunOptions::default()`，没有隐藏的模型、温度、system 或 token
覆盖。需要覆盖时显式调用 `send_with(text, options)` 或
`send_document(document, options)`；原始 `AgenaClient` 仍可用于所有其他操作。

每个 `RunHandle` 保存完整 receipt：
`execution_id`、`message_id`、`attempt_generation`、`turn_id` 和 `reply_id`。
同一个 execution 的续跑可能产生新的 generation，不能只检查 session idle
或者任意一条 assistant 消息。

| 操作 | 保证 |
| --- | --- |
| `step.observed()` | 脚本模型收到并匹配了对应 HTTP 请求 |
| `step.finished()` | 脚本响应 body 已被 HTTP 服务消费完，不表示产品执行结束 |
| `run.wait_terminal()` | 此 execution 的此 generation 已有明确终态 |
| `run.wait_settled()` | 所属 Agena 服务实例中，该 attempt 的 supervisor、后置钩子和直接关联的 settlement 已退出 |
| `run.cancel()` | 以 generation 为条件取消，旧 handle 不取消新 attempt |
| `case.finish()` | 等待 case 拥有的观察任务、产品关闭和辅助服务，并核查脚本 |

`Recovered` 表示该 attempt 属于另一 kernel epoch，或已经标记为 `Interrupted`。
新实例不能凭数据库终态证明旧实例的收尾已经完成；它本身不等于执行失败，
正常重启后查询旧 attempt 也可能得到这个状态。`Superseded` 表示该 execution
已产生后续 generation。`wait_settled` 对两者报错。

attempt settlement 不包括无关定时工作、自动标题、独立后台任务或所有后代
agent 的完成；这些行为需要观察对应任务、事件或持久化结果。

`RunResult` 的文本来自产品正式的文本读取接口，按本次 execution、generation
和所属消息过滤，避免匹配上一轮或重试前的输出。

权限与用户输入均走真实待处理请求：

```rust
let pending = run.wait_permission().await?;
pending.mark_presented().await?;
pending.allow_once().await?;
// 也支持 deny_once、完整 PermissionReplyKind 和 scope。
```

`wait_user_input()` 返回同一种交互驱动，可提交正式答案 map 或取消／超时回复。
驱动使用请求中实际的 session ID，支持父会话上可见的子会话交互。
每次等待有时限；用 `with_timeout` 或 builder 的 case timeout 调整。

`RunHandle` 的交互等待会核对 execution ID 和 generation。终态后的
`run.post` 若还发起交互，而公共会话投影已经移除了 `active_execution`，
run 级 helper 会明确报错，避免猜测请求归属。此时用
`UserSession::wait_permission()`／`wait_user_input()` 取得会话级的真实请求，
按固定 request ID 和实际 owner session 回复，再用 `wait_settled()` 等待收尾。

## 设置、文件、环境和重启

`TestAgenaBuilder` 提供两种清晰的配置起点：

- 默认使用 `local_model_config` 文档；`configure_global` 在该文档上编辑。
- `raw()` 不注入模型 preset；`global_config` 和 `workspace_config`
  可以直接提供 `ConfigFile::Json`、任意 `Bytes` 或 `Missing`。

`global_config` 是**完整替换**。若还想保留本地模型，请使用
`configure_global`，或在 `configure_global_with` 中根据
`CasePaths`、`ModelHandle` 明确构造配置。库不重新实现 Agena 的合并规则，
由产品处理 defaults、global、workspace、环境变量和受支持的 CLI 覆盖。

其他输入入口：

- `.env(name, value)`、`.env_remove(name)`、`.env_relative(name, path)`。
- `.cli_override("ui.locale=zh-CN")` 和 `.args(...)`。
- `.seed_file("workspace/note.txt", bytes)`、`.cwd(...)`。
- case 内的 `write_config(layer, source)`、`seed_file(...)`。
- `restart()`、`restart_with_env(...)`、`crash()`。
- `expect_start_failure()` 检查产品以非零状态退出的启动失败，并返回进程诊断。
  若要测试服务仍可运行但插件加载失败，使用 `allow_failed_plugins(true)` 后检查正式状态；
  readiness 超时不会被当成一次成功的“预期启动失败”。

CLI 接受的设置名由产品的 `ConfigOverride` 决定。例如 `ui.locale` 有 CLI
映射，`session.max_turns` 应使用文件或 `AGENA_SESSION_MAX_TURNS`。
进程所有权相关参数（后台模式、端口、数据目录等）由 fixture 管理，不能通过
`args` 把测试重定向到用户已有服务。

### 正式设置 API

```rust
use agena_testkit::{EditOptions, SettingsLayer};

let response = case.settings().set(
    SettingsLayer::Workspace,
    "session.max_turns",
    serde_json::json!(8),
    EditOptions {
        dry_run: false,
        validate: true,
        reload: true,
        expected_revision: None,
    },
).await?.require_success()?;

let snapshot = case.config_snapshot().await?;
```

支持 layer GET／SET／DELETE、global PATCH、验证、reload，以及原始响应。
`EditOptions` 暴露 dry-run、校验、revision CAS 和直接 reload 开关。

`config_snapshot()` 同时提供文件层、loaded document、runtime generation 和
服务身份。它会核对前后 generation 与身份；各文件仍各自有 revision，
不是跨文件原子快照。reload 失败时，磁盘文件和已加载 runtime 可以不同，
测试应分别断言。

`loaded` 保留正式 API 的 `{ "config": ..., "meta": ... }` 包装；例如读取
已生效的回合上限使用 `snapshot.loaded.pointer("/config/session/max_turns")`。
文件层响应的实际内容在 `global["value"]`／`workspace["value"]`，revision
仍来自各自的文件响应。

`reload: false` 表示这次请求不直接 reload；独立的产品文件 watcher 仍可能
稍后 reload。因此不要把“若干毫秒内 generation 不变”作为该参数的契约。

`loaded_snapshot()` 与 `wait_loaded()` 返回 `LoadedConfigSnapshot`，只读取已发布的
runtime/loaded config/identity，不解析磁盘文件层。故意写入非法 JSON 测试 reload
失败时，仍能用它们检查最后成功的配置；完整 `config_snapshot()` 保留严格的文件层
读取语义。这些等待的 deadline 包含最初的 identity HTTP 请求。

等待设置生效和等待某个 reload 完成有不同的断言对象，现在可以直接写：

```rust
// 等待公开 resolved-config 中的值生效，失败时显示 pointer、期望值和最后观察值。
let loaded = case.settings()
    .wait_loaded("/config/session/max_turns", serde_json::json!(8)).await?;

// 发起正式 reload，等待精确 task receipt，再读取新的运行快照。
let refreshed = case.settings().reload_and_wait().await?;

// 需要分别安排开始和等待时，保留一个绑定当前 server identity 的 handle。
let reload = case.settings().start_reload().await?;
let snapshot = reload.wait().await?;
```

`reload_and_wait` 处理产品合并并发 reload 请求的情况，并保证相对调用前发布了新
generation。`start_reload` 的 handle 只跟踪它接到的那一个 task；如果产品把请求
合并到已发布 generation 的在途任务，它会等该任务完成。失败和取消会报告正式
task 的 public problem，服务重启后旧 handle 会明确报错。

插件工具已经返回 `reload_task_id` 时，在工具调用前取 `ConfigSnapshot`，然后用
`case.settings().track_reload(task_id, &before).wait().await?`。该方法不会额外发起
reload，也不会把其他任务或任意 generation 变化当作原任务成功。MCP 场景展示了
这个写法。`SettingsDriver::with_timeout` 和 `ReloadHandle::with_timeout` 可以设置
这些等待的期限。

原始 `reload()` 仍返回正式异步响应，供协议和错误测试使用；SET／DELETE 的直接
reload 由产品自己完成。`runtime_status` 的 generation 与任务列表不是跨异步读取
的原子快照，新的等待方法会继续读取，直到精确任务和发布状态同时满足条件。

## 测试自己的插件

需要在 Agena 启动时就加载 HTTP 插件，可以使用异步 `before_start`：它在隔离
路径、配置文件和脚本模型准备好之后执行，拿到的 `TestCase` 可用于
`serve_http`、`model`、`write_config` 和文件准备。先启动本地 SDK router，再把
返回的 URL 写入配置，并用 `expect_plugin("my.echo")` 要求产品启动就绪时插件已运行。

多个 `before_start` 按注册顺序各执行一次，重启不会重复运行；注册的 service
继续归 case 所有，活到产品关闭之后。准备阶段有启动超时保护，错误和 panic
也会等待清理。此时产品尚未启动，不能调用 `client()`、`http()` 或 `endpoint()`。

HTTP 插件可以直接在测试进程中运行 SDK router，产品通过真正的插件 HTTP
协议加载它。下面是服务已经启动后，通过真实设置 API 动态加载的写法：

```rust
let fixture = case.serve_http(
    agena_plugin_sdk::drivers::http::router(
        || MyPlugin::default(),
        std::sync::Arc::new(agena_plugin_sdk::NoopHostClient),
    )
).await?;

case.settings().set(
    SettingsLayer::Global,
    "plugins.list.\"my.echo\"",
    serde_json::json!({
        "enabled": true,
        "package": {"kind": "http", "url": fixture.url("/rpc")},
        "settings": {"uppercase": false}
    }),
    EditOptions::default(),
).await?.require_success()?;
```

SDK 的 fallback `NoopHostClient` 不提供假的成功回调。正式 Agena init
会发送真实 callback URL/token，SDK 用真实 HTTP HostClient 替换 fallback。
系统测试中的 Echo 插件会验证 callback 信息存在，并在工具执行时实际
`read_config` 回调产品。

随后让脚本模型调用完整工具名（例如 `my.echo.echo`），并在下一次模型请求中
检查工具结果。测试既能观察模型输出，也能用共享计数器、通道、信号量或临时
文件检查自己插件的 hook、副作用和关闭结果。

[`plugins.rs`](../../apps/agena/tests/scenarios/plugins.rs) 提供完整可执行示例：
before_start 准备服务 → 产品启动时实际加载 → 工具 before／invoke／after → host callback → run.post gate →
设置修改与 reload → 第二次调用 → session.end／shutdown。

stdio 和 cdylib 插件使用同样的正式配置加载已构建产物。测试入口依旧是 Rust
测试，无需为每个场景创建 runner。`case.serve_http` 也可承载 MCP、OAuth、
外部 HTTP API 或其他模型协议的本地 fixture。

`serve_http` 管理监听器、普通 HTTP 连接和升级后的真实 TCP socket。显式
`fixture.shutdown()` 或 case 收尾会断开连接，并等待 socket 释放；handler
持续保留 socket 会使清理失败。

WebSocket 协议优先使用测试库的 extractor，它会在返回升级响应前登记 worker：

```rust
use agena_testkit::FixtureWebSocketUpgrade;
use axum::{Router, extract::ws::Message, routing::get};

let socket_server = case.serve_http(Router::new().route("/ws", get(
    |upgrade: FixtureWebSocketUpgrade| async move {
        upgrade.on_upgrade(|mut socket, cancel| async move {
            let incoming = tokio::select! {
                () = cancel.cancelled() => return Ok(()),
                message = socket.recv() => message,
            };
            let Some(Ok(Message::Text(text))) = incoming else {
                anyhow::bail!("expected a text message");
            };
            anyhow::ensure!(text == "fixture-start");
            socket.send(Message::Text("fixture-reply".into())).await?;
            Ok(())
        })
    }
))).await?;
```

这个 callback 的 `Err`、构造 future 时的 panic 和执行中的 panic 都会使 case 失败。
`fixture.shutdown()` 会关闭登记入口、取消并 join 协议 worker，连尚未完成的握手也
在其所有权内。长循环应观察传入的 cancellation token；不能按期结束的 worker 会
被 abort、join，并明确报告清理失败。需要协议名或帧大小设置时，通过
`upgrade.configure(|ws| ws.protocols(["my-protocol"]))` 配置原生 Axum upgrade。

原始 Axum `WebSocketUpgrade::on_upgrade` 仍可用于底层协议故障测试，其 callback
是 Axum 自行启动的任务，只有 socket 由 `serve_http` 追踪；完整的错误与 join
保证属于 `FixtureWebSocketUpgrade`，或用例自己通过 `spawn_service` 管理的 worker。
现有
[`providers/realtime.rs`](../../apps/agena/tests/bundled/protocols/providers/realtime.rs)
已使用新的 extractor，并验证 setup ACK、完成/失败/超时及音频与用量。

## 资源所有权、失败与诊断

优先使用 `builder.run(...)`。它在正常结果、错误、panic、模型脚本错误和超时
下都进入 awaitable cleanup；返回时同时报告测试体和清理错误。
如果使用 `builder.start()`，必须显式 `case.finish().await`。
Drop 只是紧急兜底，不能代替可验证的异步收尾。

外部取消 `run`、`start` 或准备过程的 future 时，只要 Tokio runtime 仍在运行，
guard 会立即启动 case 自己拥有的清理操作，避免 router/worker 持有 case clone 时
形成资源泄漏。取消调用者本身不能同步等待异步 teardown；持有 case 时可继续
`finish().await` 加入同一清理操作。取消某个 finish 等待者也不会取消真正的清理。

- `case.spawn_task` 注册观察任务，退出时先取消并 join。
- `case.spawn_service` 注册产品运行需要的服务，在产品关闭后再取消并 join。
- `case.serve_http` 自动使用后一种所有权，插件服务可活到 session.end/shutdown。
- 自己已有的 `JoinHandle` 可用 `track_task`／`track_service` 注册。
- `cancellation_token` 和 `service_cancellation_token` 分别对应两个阶段。
- `server_cancellation_token` 对应当前 server epoch，重启会使旧观察者退出。

已有 JoinHandle 的失败也会立即通知 case。注册应发生在 case 活跃阶段，所属阶段
的 cancellation token 触发后必须停止注册新任务。owned worker 不应等待自己的
`case.finish()`，`spawn_task`／`spawn_service` 会对此明确
报错。清理逐个消费完成结果，某个任务挂起不会丢掉其他任务已经返回的错误。

权限测试需要留意产品自带的 `<tmp>/**` 放行规则。fixture 工作区位于临时目录，
仅把宽泛的 workspace 模式设成 `ask` 或 `auto` 不一定会触发审阅；应像现有
工具场景那样，为测试目标的绝对路径配置明确的 `path.rules` 条目，再观察待审批
状态和文件副作用。这仍使用生产权限匹配规则。

没有注册到 case 的任意 `tokio::spawn` 不会凭空变成被管理的任务；插件自己
创建的后台任务仍需要在插件 shutdown 中停止和等待。

默认成功后在最后一个 `TestCase` clone 释放时删除临时目录；`finish()` 返回后仍可
持有 case 检查文件。失败保留配置、stdout/stderr、有界模型请求和错误
诊断。可用 `keep_artifacts(true)` 或 `retain_artifacts()` 主动保留。
脚本未消费、未声明的模型调用、后台 fixture 错误、关闭超时和意外进程退出
都使测试失败。

`crash()` 是对 fixture 拥有的产品进程进行故障注入。Unix process group
本身不能覆盖独立 session 的后代；测试库现在强制为每个 case 设置单独的
`TMUX_TMPDIR` 和私有 tmux Unix socket，并在 crash/final finish 时清理属于该
socket 的 tmux server、pane 与子进程，不会连接或杀死用户已有的全局 tmux。
重启仍可保留同一 case 的 tmux 状态。Linux 还会基于 /proc 父进程链、PID +
进程启动时间采集并回收其他仍属于 Agena 的直接可见后代；当清理并非正常退出
所需的额外杀进程时会留下清理错误。它是有界的进程所有权防线，不是 cgroup
或进程命名空间：抢在观测前 daemonize、macOS 的脱离父进程等情况仍需专门
平台隔离才能提供强保证。Windows 继续使用 Job Object 的既有行为。

当前前台进程的正常关闭使用 Unix 信号；Windows Job Object 能强制回收，但
不能据此证明插件优雅关闭，因此 fixture 会把这种收尾报告为 forced。Windows
上的优雅生命周期测试需要补充产品可观察的正常关闭入口。

Runtime 的 awaitable drain 覆盖已登记的 worker、session、控制回调、活动会话的
session.end、当前／已退休且完成初始化的 PluginHost 和 kernel。初始化尚未返回的
内部回滚，以及既有 monitor／terminal 的操作系统任务停止，仍不能由这个 barrier
证明所有外部后代均已退出；需要对应的专项 fixture 和资源断言。

HTTP SDK 为最近 256 个已成功清理的显式 owner 保留幂等关闭记录。若某个旧
PluginHost 跨越超过这个保留窗口后才退役，关闭会明确报告 owner 已失效；未知
owner 和被淘汰的记录都不会被当作成功，真实的 shutdown 错误也不会被覆盖。

## 覆盖范围

### 无模型场景：Git、文件工作台、插件 CLI 与 PTY

`TestCase` 还允许直接驱动产品的非聊天入口：

```rust
let git = case.git().in_directory("workspace/project")?;
git.post("init", &serde_json::json!({"defaultBranch":"main"})).await?
    .require_success()?;

let fs = case.fs_workbench();
fs.post("write", &serde_json::json!({"path":"note.txt", "content":"hello"}))
    .await?.require_success()?;

let output = case.run_cli(["plugin", "list-installed"]).await?;
output.require_success()?;

let terminal = case.terminal().create(case.workspace()).await?;
terminal.input(b"printf 'ready' > from-shell.txt\n".to_vec()).await?;
case.wait_file_bytes("workspace/from-shell.txt", b"ready").await?;
terminal.delete().await?;
```

`GitDriver`／`FsDriver` 管理该 case 的目录和正式 HTTP 认证，原始 `request()`
支持测试未封装的动作。`run_cli` 使用正式 Agena binary 和清理规则，不会调用
额外的测试 runner；它返回状态码、标准输出与标准错误，允许检查正常失败。
`terminal` 持有真实 PTY，`wait_output_contains` 按流逐帧读取 SSE，但会包含
服务器重放的历史尾部，因此不应单独证明**本次**命令执行。要验证新输出，
操作前调用 `let checkpoint = terminal.checkpoint().await?`，发送输入后使用
`terminal.wait_output_after(&checkpoint, "期望的新输出").await?`；新 API 只接受
该 checkpoint 之后更大的 SSE `seq`，遇到历史缺口立即失败，不会把过期输出
当成新输出。不同终端的 checkpoint 不能混用；stop/start 后需重新 checkpoint。
**PTY 仍会回显输入**，所以期望文本不要原样出现在输入命令中。例如输入
`printf 'proof-%s\\n' 'complete'`，等待 `proof-complete`，另检查独立文件副作用。
`case.http().open_stream()` 是其他 SSE 类端点的基础能力。
`wait_file_bytes` 会等待确切内容及长度，避免把模型文本或 PTY 命令回显当作
磁盘操作完成。更多可复制的测试见 `apps/agena/tests/scenarios/`。


这一层覆盖的是**产品 Server 与 Runtime 的真实业务链路**。公共客户端和
原始 HTTP 驱动允许测试未包装的新接口、非法请求和状态码；配置文档入口允许
测试未知字段、缺失文件、凭据路径、插件声明和启动失败。新增功能通常增加
场景或本地 fixture 即可，不要求给测试库新增一个专门方法。

它不自动证明 Web DOM、浏览器 localStorage、TUI 键盘／终端渲染、真实模型
服务的兼容性、真实 OAuth 网站或操作系统级别的全进程沙箱。这些边界需要相应
的 UI／协议／平台测试。内置场景也不是全部设置组合的穷举；库提供真实输入和
观察入口，具体产品约束仍要写明确的行为断言。
