# 测试框架缺陷修复与验证（2026-11-07）

范围：只修复既有 Rust testkit、调度器、原生模型协议适配器及其直接产品生命周期；**本轮不新增 Web、Git、Marketplace 测试**。所有操作在原来含未提交修改的工作区中增量完成，未进行提交或重置。

这是 [深度审查](framework-deep-audit.md) 的后续状态记录。审查文档中的反例和原始日志保留为修复前证据。

## 已实施的修复

| 审查项 | 修复 | 必须使测试失败的反例 |
|---|---|---|
| A01 原生流缺终态 | Anthropic 与 Bedrock Anthropic 要求 `message_stop`；Gemini 要求候选 `finishReason`；Ollama 要求 `done:true`，否则产生 ProviderError 而非 Completed | HTTP 干净 EOF、只有部分文本、没有应用层结束事件 |
| A02 虚拟时间/租约 | 调度器使用虚拟 UTC 决定业务到期，使用真实 UTC 跟踪与 30s 真实心跳一致的租约；SQLite 与内存存储的 due/deadline 查询均转换租约时间域 | 活跃 sink 阻塞期间向前跳跃六小时，第一笔成功提交不得因为虚拟跳跃被重投 |
| A03 崩溃后 PTY 存活 | `TMUX_TMPDIR` 固定为每个 case 的独立目录，清理始终使用该 case 的专用 socket；Linux 用 `/proc` 父链、PID 与启动时间追踪可见后代 | 进程忽略 HUP、独立 session 中运行 shell/sleep，kill Agena 后不得继续存活；不得访问开发者的全局 tmux |
| A04 未消费 fault | case 清理前检查当前故障 arm 是否还有未消费次数；未释放的 kernel Hold 也算失败。显式 clear 才允许不触发的预期故障 | arm 后直接返回 `Ok`，必须报告未命中 |
| A05 隐式中断 | 正常完成与断开分别验收；需要在 teardown 触发的断开必须预先声明 `expect_disconnection`，且检查最终确实断开 | held reply 在用户测试体结束后被 teardown 取消，不能当做正常完成 |
| A06 Query 丢失 | `ModelRequest::raw_uri/query_pairs/query_values` 保存原始 URL 与多值 query；`RequestMatch::query_parameter` 要求恰好一个匹配项，默认 Debug/错误诊断隐藏 query 和 header 值 | `alt=sse` 写错/重复时，匹配应失败；query 内测试凭据不应泄露到日志 |
| A07 自定义网关丢协议 | `ModelRoute::for_protocol` 为路由显式携带协议；自定义 Gemini 网关仍能解析模型名和流式操作 | 自定义前缀的 Gemini 请求不能被认作非流 JSON |
| A08 默认路由过宽 | 单协议 fixture 不再无条件接受 OpenAI Responses 路由，只有所选协议/显式路由可进入 | Anthropic fixture 收到 `/v1/responses` 必须拒绝 |
| A09 Terminal SSE 弱断言 | `TerminalOutputCheckpoint`、`checkpoint`、`wait_output_after`；`connected.lastSeq` 为可靠游标；历史缺口报错 | 旧输出回放不能被当作新输出，不同 PTY 的 checkpoint 不得混用 |

## 测试入口

```sh
cd ~/Git/ai_research/agena
# 只测 Rust 核心和既有产品协议，不运行 Web/Git 专项
cargo test -p agena-testkit -p agena-scheduler -p agena-storage-sqlite -p agena-runtime-provider-adapters --lib --offline --locked -- --test-threads=2
cargo test -p agena --test system --offline --locked -- --test-threads=1 \
  --skip scenarios::git --skip scenarios::marketplace --skip scenarios::workbench_fs
cargo clippy -p agena -p agena-testkit -p agena-scheduler -p agena-storage-sqlite \
  -p agena-runtime-provider-adapters --all-targets --no-deps --offline --locked -- -D warnings
cargo check -p agena-scheduler -p agena-storage-sqlite --release --offline --locked
```

已有定向验证：四协议缺终态回归与原生协议系统套件 23/23、终端真实场景 2/2、独立 PTY 崩溃 1/1、Gemini query 1/1、tmux 清理契约 1/1。最后一轮四个库 **126 项通过、0 失败**，另有一项刻意忽略的子进程入口；按本轮范围过滤掉 Git、Marketplace 和文件工作台后，实际产品系统测试 **79 项通过、0 失败、13 项过滤**（单线程、710.07 秒）。严格 Clippy 对 Agena 产品、testkit、scheduler、SQLite 存储与 provider adapters 执行 `--all-targets --no-deps -- -D warnings`，**通过（退出码 0）**。Scheduler 与 SQLite 存储的 `--release` 编译检查**通过（退出码 0）**。受影响 27 个 Rust 文件的定向 rustfmt、`git diff --check` 以及文档空白检查均通过。更多输出在 `/tmp/agena-framework-repair-checks/`。

曾有一轮完整系统回归达到 91/92，其中一个 Anthropic 用例在进入模型前读取 session state 时超时；该用例隔离重复三次均通过。这不是四种协议 EOF 语义回归，应作为不确定的环境/并发超时现象另行追踪，不要在未找到原因前声称已根治。

## 仍然不在本批修复范围

- 测试工作流没有全面收录非系统 Rust integration、Web Bun、浏览器探针；本轮没有修改 CI。
- FaultDriver 还不能按 RequestId/transaction/第 N 次 syscall 定向注入；也没有硬件掉电的缓存持久化模拟。
- `TestClock` 的业务时间跳跃不是虚拟执行所有 heartbeat；测试真正的 heartbeat 失效需要显式暂停心跳或租约故障门闩。
- Linux 的 /proc 后代跟踪及私有 tmux socket 不是 cgroup/pid namespace 级隔离。提前 daemonize、macOS/Windows 独立进程的完整回收保证需要其他平台的专项验证。
- 终端输出中的 PTY 输入回显与 shell stdout 都走相同 PTY 字节流。游标解决**旧数据回放**，不能按来源消除命令回显；必须像系统测试那样使用不在输入字面出现的输出标记，以及独立文件副作用断言。
- 非流式 provider 的全部矩阵、应用内通知/EventDriver、SigV4 精确签名、TTL/OAuth 生命周期仍留给后续批次。

**不变的验收原则：** 每一项都同时证明控制点被实际触发、预期结果被独立观察、反例确实会失败，以及清理不会影响其他 case 或用户自己的进程。
