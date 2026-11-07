# Agena Plugin Test Harness — 设计 RFC（草案）

> 状态：设计建议，尚未实现。目标是为插件开发者提供可重复、隔离、可检查的插件测试能力，复用生产运行路径，而不是实现一套“假 Agena”。

## 1. 目标与真实性边界

- 验证**实际编译、实际配置、实际加载**的插件，在生产 PluginHost/Runtime 中的可观察行为。
- 覆盖插件 Manifest 所声明的所有对外入口，并允许测试未声明但被插件使用的 HostClient 能力、异步副作用、重启与错误路径。
- 提供可编写、可调试、可在 CI 执行的场景测试、故障注入、执行轨迹和覆盖度报告。
- 默认不访问用户凭证、数据目录、真实模型 API 或真实互联网。可选的真实云服务测试必须显式开启。
- **“覆盖所有功能”不等于自动证明任意插件实现正确。** 框架自动盘点 Manifest 能力和协议契约；插件作者必须定义业务断言；报告分别展示“声明”“执行”“断言”“故障路径”的覆盖情况。

建议采用两个独立的真实性维度：
1. **运行路径真实性**：插件代码、传输、宿主、工具执行器与会话流程尽可能复用生产代码；
2. **外部输入可控性**：仅模拟模型输出、用户决策、时间/网络故障、外部 API、隔离文件与存储内容。

禁止用 `NoopHostClient` 的“无实现返回 HostUnavailable”来模拟完整权限审批，也不能把裸 `PluginHost::invoke_tool` 的成功计为真实会话的端到端权限测试。

## 2. 当前代码中可复用的基础

| 组件 | 已验证的生产入口/约定 | 测试用途 |
| --- | --- | --- |
| `agena-plugin-sdk/src/plugin.rs` | `Plugin`、`InitContext`、工具/命令/服务/Hook | 插件契约 |
| `agena-plugin-sdk/src/manifest.rs` | `PluginManifest`：tools、hooks、hook_handlers、commands、services、skills、surface、settings | 自动枚举声明能力 |
| `agena-plugin-sdk/src/host_api.rs` | `HostClient` 包含权限、配置、存储、密钥、事件、Agent、工具、服务、UI 等接口 | Host API 夹具/观察 |
| `agena-plugin-host/src/loader.rs` | `prepare_entry` + `activate_entry`，真实 manifest 协商、初始化和传输绑定 | 契约与加载测试 |
| `agena-plugin-host/src/host/plugin_host_build.rs` | `PluginHost::new` / `new_with_callback_dispatcher` | 实际 Host 测试 |
| `agena-plugin-host/src/host/plugin_host_core.rs` | 调用、流、Hook、注册表、关闭、架构目录 | 验证真实 Host 交互 |
| `agena-runtime-tools/src/tool/executor/executor_execution.rs` | `ToolExecutor` 工具执行 | 执行链测试 |
| `agena-runtime-session` | 会话、权限审批、模型循环和工具调用 | 全链路用例 |
| `agena-runtime/src/plugin_composition.rs` | Runtime 组合真实 PluginHost | 避免复制装载策略 |
| `scripts/probes/plugin_permissions.py` | 隔离 Server + 本地 Provider + Ask/AllowOnce/Deny/取消检查 | E2E 夹具原型 |
| `scripts/probes/http_experience.py` | 临时数据根、工作区、Mock Provider、HTTP API 和健康检查 | 隔离运行原型 |
| `crates/agena-plugin-marketplace/src/project.rs` | `scaffold_plugin` 生成项目模板 | 自动生成测试 |
| `tools/agena-plugin-dev/src/main.rs` | 独立 `agena-plugin` 开发工具，当前无 test 命令 | 开发者 CLI |
| `.github/workflows/plugin-ci.yml` | 当前主要执行 fmt、clippy、validate | 扩展为运行测试 |

已知协议约束：RPC `PROTOCOL_VERSION = 3`；cdylib ABI v2；Manifest 在 `meta/manifest` 和 `meta/init` 之间必须相同。Hook 和 Tool 的执行环节分布在 Host、ToolExecutor 与 SessionManager，**不能把单层调用当作全链路测试**。WASM 为可选编译特性。当前 Host 代码注明 cdylib 不能获得与 stdio/HTTP 相同的反向回调；该能力差异要在矩阵中明确标注为不支持，不能假绿。

## 3. 测试分层

| 层级 | 名称 | 真正运行什么 | 模拟什么 | 能声明验证什么 |
| --- | --- | --- | --- | --- |
| L0 | SDK/unit | 开发者插件方法、类型、序列化 | 单测夹具 | 业务逻辑；不能称真实加载 |
| L1 | Host/contract | 实际插件产物 + 真实 `PluginHost::new` + 真实传输协议、注册、初始化与收尾 | 可控 HostClient 后端 | RPC/ABI、Manifest、Schema、服务绑定、基本调用、流、Host Callback |
| L2 | Session/integration | 实际插件 + `ToolExecutor` + `SessionManager` + 生产权限/Hook/Tool 协调链 | 本地模型、外部服务与审批回复 | 一次真实会话中的调用、权限、Hook、模型事件、取消 |
| L3 | Server/E2E | 正式 `agena server`、真实配置、隔离数据目录、HTTP API、插件加载 | 本地脚本化 Provider、外部 API、用户回复 | 启动配置、会话持久化、热重载、恢复、跨进程、日志与诊断 |
| L4 | UI/compat | 可选的 TUI/Web、多个平台/版本/传输 | 可控场景 | 用户可见交互、兼容性、真实云服务（仅显式） |

L1 快速、利于准确定位；L2/L3 是用户所要求的“模拟真实运行”。至少 L2 和 L3 需要直接穿过生产认证和审批入口，不应自制旁路。所有层复用同一份场景声明与断言类型，允许标记场景最低所需层级。

### 为什么单独调用 Host 工具不够

生产中工具的输入经过目录快照、tool.definition、tool.before、输入重校验、工具与资源权限、invoke/stream、输出校验、tool.after、结果持久化与展示。部分步骤不在 `PluginHost::invoke_tool` 内；`ToolExecutor::execute_invocation_detailed` 本身也不是完整的会话审批循环。

因此，L1 的 `call_host_tool` 与 L2 的 `session.call_tool` 必须命名区分，报告亦分开统计。L2 最可靠的最初实现是使用本地 Scripted Provider 生成生产模型协议 `tools_call`，通过实际 Server/Session 接口驱动，而不是调用 Hook RPC 伪造一段流程。

## 4. 建议的代码组织

```text
crates/agena-plugin-testkit/
  src/
    spec.rs              # 测试套件/步骤/断言定义，schema_version
    discovery.rs         # 从 Manifest 推导能力矩阵和待测项目
    fixture.rs           # 独占临时 HOME/数据目录/工作区/种子文件
    sandbox.rs           # OS/container 隔离能力检测和显式安全降级
    host_driver.rs       # L1: 真实 PluginHost 及能力受控的 TestHostClient
    session_driver.rs    # L2: 与运行时共享生产调用入口
    server_driver.rs     # L3: 真实 daemon 生命周期/API
    provider.rs          # OpenAI 兼容、确定性脚本化本地 Provider
    services.rs          # 本地 HTTP/代理/回调服务
    interaction.rs       # Ask/AllowOnce/Deny/UserDeclined、取消、Agent 回答
    fault.rs             # 连接中断、延迟、损坏帧、超时、崩溃、背压
    trace.rs             # 结构化事件记录和关联 ID
    assertions.rs        # 结果/顺序/副作用/无副作用/错误类别
    coverage.rs          # 声明能力 × 执行 × 断言矩阵
    report.rs            # JSON/JUnit/HTML（可逐步引入）
    runner.rs            # 并发调度、超时、清理、重放
  tests/                 # 自举测试：用 echo/multi_tool_plugin_stdio
tools/agena-plugin-dev/  # 新增 test/inspect-test 入口；不复制执行逻辑
crates/agena-cli/        # 可选 'agena plugin test' alias，调用同一个 testkit
crates/agena-plugin-marketplace/src/project.rs
                         # init 自动生成 tests/plugin/*.test.toml
.github/workflows/plugin-ci.yml
                         # 验证、构建、契约、集成测试
```

依赖方向：`testkit -> SDK/Host/Runtime`；Host/SDK **不得反向依赖 testkit**。项目模板只生成文本测试文件，不把 testkit 链接到插件产物。对于为了缩小体积需要独立安装的开发工具，可把重型 Server 驱动放到单独的可选 feature 或 runner 二进制；两种入口使用相同测试规范与结果模型。

## 5. 开发者 API

建议统一提供：

```bash
agena-plugin test . --profile contract
agena-plugin test . --profile runtime
agena-plugin test . --profile e2e --output target/plugin-test
agena-plugin test . --case permissions.denied --seed 42
agena-plugin test . --list
agena-plugin test . --coverage --strict
agena-plugin test . --report junit
agena-plugin test . --record
agena-plugin test . --replay target/plugin-test/trace.jsonl
```

`agena plugin test` 可以作为主 CLI 对应别名。命令不应隐式连到正在运行的用户 Agena 实例。

测试格式优先选择 TOML，与现有 `agena-plugin.toml` 项目保持一致；复杂的多步骤测试可由 Rust API 编写。建议一个 `tests/plugin/*.test.toml` 文件对应一个场景组；插件产物可以来源于显式 `--artifact` 或构建配置。测试文件中的相对路径都相对于项目目录解析，禁止静默读取真实 Home 下的配置。

### 声明式用例（目标语法）

```toml
schema_version = 1
id = "example.hello"
profile = "runtime"

[plugin]
id = "example.hello"
kind = "stdio"
artifact = "target/debug/agena-example-hello"

[fixtures]
provider = "scripted"
network = "deny_except_loopback"
workspace = "fresh"

[[cases]]
name = "hello_returns_payload"
tags = ["tool", "happy_path"]

[[cases.steps]]
action = "tool.call"
tool = "example.hello.hello"
input = { name = "Agena" }

[cases.steps.expect]
status = "ok"
payload = { message = "Hello, Agena!" }

[[cases]]
name = "unknown_input_rejected_before_execution"
tags = ["tool", "schema", "negative"]

[[cases.steps]]
action = "tool.call"
tool = "example.hello.hello"
input = { bad = "field" }

[cases.steps.expect]
error_kind = "invalid_params"
side_effects = "none"
```

以上是**拟议 API**，并非当前已支持的文件格式或命令。用例规范应严格校验未知字段、版本、动作和断言。必须区分 `expect.payload`、`expect.error_kind`、`expect.no_effect`、`expect.events` 等，不能以“调用没有抛异常”当作通过。

### Rust 场景接口（拟议）

```rust
let mut h = TestHarness::new()
    .plugin(PluginArtifact::stdio("./target/debug/my-plugin"))
    .profile(Profile::Runtime)
    .isolated_workspace()
    .scripted_provider()
    .build().await?;

let result = h.session().call_tool(
    "acme.todo.create",
    serde_json::json!({"title": "write tests"}),
).await?;

result.assert_payload(serde_json::json!({"created": true}))?;
h.trace().assert_called("acme.todo.create")?;
h.trace().assert_no_external_network()?;
```

通过同一 `ScenarioStep`/`Assertion` 数据模型实现声明式与 Rust API。注意 Trace 仅能证明已观测到的事件，`assert_no_external_network` 需要 OS/容器网络隔离 + 审计支持，否则标为 `inconclusive` 而非 `passed`。

## 6. 夹具与故障注入

**FixtureEnvironment** 每个用例创建唯一的运行目录：
- `HOME`/`XDG_CONFIG_HOME`/`XDG_DATA_HOME`/`AGENA_DATA_DIR`/`TMPDIR`、cwd 和 workspace 均指向隔离空间（平台不同则配置等效变量）。
- 只有白名单环境变量传入插件子进程；不继承真实密钥、代理、Provider 配置或外部用户配置。
- 本地 HTTP Provider 实现实际使用的 provider 协议，提供文本、`tools_call`、流式片段、错误、重试、压缩与取消脚本。
- 支持创建文件、目录、软链接、二进制附件、服务端响应、模拟 secrets/KV、Agent 和用户交互脚本。
- 每个步骤生成事件 `seq`、`case_id`、`plugin_id`、`generation`、`session_id`、`call_id`、`request_id`、`transport`、`method`、时间戳/耗时，以及脱敏后输入/输出摘要。
- 抽象可观察副作用：文件快照/差异、存储键版本、HTTP 请求、注册表增删、通知、日志、活动、子进程数、未完成任务。
- 采用显式 `FaultPoint` 注入：meta/init 失败、stdout 协议污染、RPC 断线、HTTP 5xx、错误顺序/丢帧、slow consumer、Cancel、Host Callback 超时、重载中的挂起调用、子进程崩溃。

对外部进程不能假装 Tokio 虚拟时间会控制它们。进程级测试使用真实时钟和有余量的期限；对测试中的可注入 Scheduler/Clock 再使用逻辑时钟。并发断言优先用 happens-before/关联 ID，而非假设所有事件有全局确定顺序。

## 7. 安全隔离是测试正确性的一部分

对插件自身使用 `HostClient::require_*` 只能验证其协作授权的行为；stdio、cdylib、WASM 仍可能有不经 HostClient 的文件或网络访问，不能把 HostClient Deny 当作 OS 沙盒。

- stdio/HTTP：建议在受限子进程或容器运行；容器/namespace 默认断互联网、限制文件挂载，仅允许 fixture loopback 服务。
- cdylib：原生库可能与 Host 共享进程，必须把**整个测试执行器**放到一次性隔离的 worker 进程或容器内；不能在开发者主进程直接加载未知产物。
- WASM：同时检查 wasmtime/WASI 预打开路径、环境、网络能力和 epoch 取消；单独检查 host callback 支持的边界。
- Linux 可接 bubblewrap/container 等强隔离；macOS/Windows 使用能力检测和可靠的平台机制。缺乏有效 OS 隔离时标注为弱隔离并拒绝执行不可信产物（或要求明确 opt-in）。
- CI 使用只读源码、临时可写目录、无凭证、受限网络；默认测试输出必须脱敏。
- 运行完成逐项检查子进程退出、监听端口关闭、旧回调凭证失效、动态注册清理、临时文件清理。

当前 `scripts/probes/http_experience.py` 虽隔离 `AGENA_DATA_DIR`，`clean_environment` 仍可保留父进程 `HOME`。它是可用的回归测试原型，**不是具备完整 OS 隔离的通用不可信插件沙箱**；新框架不能把其隔离强度夸大。

## 8. 自动发现与覆盖矩阵

以 `PluginManifest` 作为**声明能力清单**，载入成功后再以 Host 实际注册表作校验。建议覆盖项：

| 功能组 | 自动生成的基础契约测试 | 作者需要补充的业务/故障测试 |
| --- | --- | --- |
| 元数据与加载 | id/version/协议、重复声明、manifest 稳定、init/shutdown | 初始化副作用、坏设置、重启恢复 |
| 工具 | 所有声明 tools 可发现、有效/无效 schema、结果 schema | 输入等价类、输出、流顺序、无副作用、资源权限 |
| Hook | 所有声明 handler 注册、filter/priority、合法事件 | 真实触发、patch/deny/observe、排序、失败/取消 |
| 命令 | 每条命令注册、参数、执行和结果格式 | 业务错误、命令可见范围、回滚 |
| 服务 | export/import/method/schema、依赖绑定 | 跨插件调用、拒绝、级联失败 |
| Host API | 支持矩阵、鉴权和 scope、参数校验 | KV/secrets、事件、动态工具、Agent、调度等行为 |
| 权限 | tool/path/network deny/ask/allow 基础矩阵 | AllowOnce 不跨调用、权限未批准前零副作用 |
| 状态/配置 | settings 校验、启用/禁用、status/inspect/log | 改配置、reload、旧 generation 资源失效 |
| 传输 | stdio/HTTP/cdylib/WASM 支持和协议握手 | 断链、超时、取消、背压、崩溃重启 |
| UI/媒体 | surface/skills/attachments 声明和序列化 | Web/TUI 显示、附件、通知、主题回归 |

覆盖度**不能只统计测试用例数量**，至少分四列：
1. `declared`：Manifest 暴露了什么；
2. `exercised`：在何种驱动层与何种传输下执行了什么；
3. `asserted`：是否验证了返回值、事件或副作用；
4. `negative/fault`：是否覆盖拒绝、取消、超时或故障。

例：工具 A 仅经 L1 调用成功，L2 的权限行为仍为 `untested`；Hook B 声明但未真实触发，应为 `untested`；当前传输本身不支持的能力是 `unsupported`，不能计入 passed。报告包含 `pass/fail/skip/unsupported/untested/inconclusive`，并指明各状态原因。

默认允许逐步补齐；`--coverage --strict` 在仓库/CI 可配置覆盖策略，未覆盖的已声明入口可以令 CI 失败。避免让“自动输入生成成功”伪装业务语义覆盖。

## 9. 必测不变量与回归用例

- Manifest 在 `meta/manifest` 与 `meta/init` 中一致；协议不符不启动，未成功初始化不会残留注册。
- 输入 schema 无效时必须在调用插件前被拒绝；Hook 修改输入后重新校验。
- 输出 schema 无效时报告**可能已经发生副作用**，不得盲目重试。
- Deny/用户拒绝前没有实际文件/网络副作用；AllowOnce 只在当前调用有效。
- Host callback 的授权上下文必须匹配插件、调用、会话、工作区和代次；伪造/过期凭证拒绝。
- `tool.before` 拒绝不得执行工具；observer 失败不能改写已完成行为。
- 流式 chunk 顺序完整、终态唯一，取消和背压可中断；未确认结果不应静默当成功。
- 停机/重载等待或撤销在途工作；旧 generation 不再注册工具/Hook/主题/事件。
- stdio 崩溃/重启策略有效；日志错误有可辨别分类；超时后资源清理完成。
- storage/secrets 的命名空间隔离，敏感值不写入测试报告或日志。
- 二次 HostClient 调用与嵌套服务遵守原调用 lifetime；父调用结束后不可继续借凭证访问 Host。
- 模拟 Provider 产生的 `tools_call` 走真实会话与持久化路径。

优先用 `examples/echo_plugin_stdio`、`examples/multi_tool_plugin_stdio`、`host/contract_tests.rs`、`host/permission_tests.rs` 作为 testkit 的自举对照，检验脚手架与现有测试结论一致。

## 10. 推荐实施顺序与验收

**阶段 A：打通真实 Host 的最小可用链**
- 新建 `agena-plugin-testkit` 的 spec、fixture、host_driver、trace、coverage。
- L1 支持 stdio 实际二进制加载/初始化/工具/流/关闭；错误 schema 和脚本化 HostClient。
- 开发者 CLI `agena-plugin test .` + 标准 JSON/JUnit 结果；plugin init 生成一条真场景用例。
- 自举：echo 正向、无效参数、流式输出、进程失败、退出清理全部通过。

**阶段 B：兑现真实会话运行**
- 引入 production Session/Tool 执行路径、脚本化本地 Provider、真实权限审批事件。
- 实现 Hook、Ask/AllowOnce/Deny/取消、真实 tool.before/after、config 和会话重放。
- 对照 `plugin_permissions.py`，相同业务断言由新 testkit 能复现。

**阶段 C：扩展完整插件能力**
- 服务导出/导入、Host API 存储/密钥/Agent、动态注册、热重载、崩溃恢复与故障注入。
- HTTP 回调端点鉴权、cdylib worker、WASM 可选传输及能力差异矩阵。
- 完整系统测试、结构化 trace 与安全报告。

**阶段 D：生态与持续集成**
- 项目初始化模板 + `.github/workflows/plugin-ci.yml` 自动运行 L1/L2；L3 nightly 或发布前。
- 附件、UI、跨平台、升级兼容、变异/模糊测试与真实云服务 opt-in。
- 为插件发布增加测试报告/覆盖矩阵门槛，明确标注平台限制。

### 最小可交付验收标准

从 `agena plugin init` 创建的示例工程出发，只执行统一的 `plugin test` 命令，应能：
1. 构建并加载实际产物，验证 Manifest/Init/Shutdown；
2. 在临时工作区完成正向、非法输入、错误输出、stream、取消、权限拒绝测试；
3. 在真实会话中看见 Hook、审批与工具结果（不是测试器直接假造其结果）；
4. 断言拒绝时没有文件/网络副作用，记录完整可复现的相关事件；
5. 输出能明确区分通过、失败、未测试和当前传输不支持的能力报告；
6. 测试结束不残留进程、端口、可用凭证或数据污染，且不读取真实用户配置。

**首要架构决策：复用真实生产 Host/Session，而把 TestHarness 的可替换边界放在“外部世界”与驱动层。** 这是避免脚手架将来和 Agena 真实行为发生漂移的关键。
