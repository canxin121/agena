# Agena 测试框架深度审查（2026-11-07）

> **后续状态**：本文件保留修复前的复现实验与问题定位。A01–A09 的对应修复和回归测试已在同日继续实施；请先查看 [修复与验证记录](framework-repair-2026-11-07.md)，不要将下面的“未修复”描述误认为最新工作区状态。更广泛的覆盖缺口仍需分批处理。

本报告审查当前工作树，而非仅审查 Git HEAD。目标是判断框架能否控制目标场景、是否拥有足够的可观察信息、断言能否排除错误行为，以及已有测试是否真正进入执行门禁。仓库已有未提交改动均保留；本轮仅更新审查文档，未修改产品或测试框架实现。下列已复现问题仍待修复。

优先级：P1 为应优先修复的正确性、误报通过或执行门禁问题；P2 为重要覆盖或能力不足；P3 为后续工程增强。‘未发现相关用例’表示在本次所列源码和测试入口中的审查结果，不等于形式化证明整个产品不存在任何间接覆盖。

## 1. 本轮实际验证

| 验证组 | 实际结果 | 证据 |
|---|---|---|
| Web Bun 行为测试 | 303 通过，0 失败，36 个文件 | /tmp/agena-framework-audit-20261107/web-tests.log |
| 产品二进制测试 | 88 通过，0 失败：MCP 30、Preview 51、Terminal 1、Web Push 6 | /tmp/agena-framework-audit-20261107/binary-tests.log |
| 其他 crate 的集成测试 | 19 个目标，125 通过，0 失败 | /tmp/agena-framework-audit-20261107/omitted-integration-tests.log |
| 原生流缺失结束事件 | Anthropic、Gemini、Ollama、Bedrock Anthropic 四个真实产品案例均将部分回复记为 Succeeded | /tmp/agena-model-audit-20261107/native_eof_probe.log |
| ModelDriver 请求/传输能力 | query 丢失、自定义 Gemini 路径语义丢失、错误协议被接受、held 仅阻塞 body 均已复现 | /tmp/agena-model-audit-20261107/query_probe.log；capability_probe.log |
| TestClock 与活跃 lease | 单调度器、首次 sink 已成功，推进 6 小时后再次调用同 delivery_key 的 sink | /tmp/agena-time-fault-audit-20261107/repro.log |
| 生命周期与弱断言 | PTY 残留、未消费 fault、未结束模型回复、终端 echo/replay 均已复现 | /tmp/agena-harness-audit-20261107/probe.log |

本轮补跑的前三组共 516 项通过。这个数字不是覆盖率，也不推翻下面的反例；这些反例恰好说明已通过用例没有排除某些错误行为。上一轮 301 项通过同样只对应当时列出的目标，不能解释为全仓库或全平台都已验证。

执行命令：

```sh
cd /home/canxin/Git/ai_research/agena/packages/agena-web
/home/canxin/.bun/bin/bun test ./tests

cd /home/canxin/Git/ai_research/agena
cargo test -p agena --bin agena --offline --locked -- --test-threads=2
cargo test -p agena-domain -p agena-skills -p agena-mcp-client -p agena-runtime-config -p agena-runtime-contracts -p agena-runtime-tools -p agena-tui-transcript --test '*' --offline --locked -- --test-threads=2
```

独立探针通过现有 testkit 编译产物或公共 Scheduler API 构造场景，没有修改生产源文件。PTY 探针产生的子 session 已按记录身份定向清理，并再次确认 Agena、PTY shell 和其 sleep 进程均不存在；只终止了 fixture 创建并拥有的测试进程，没有终止用户日常运行的服务。

## 2. 已复现的正确性或误报通过问题

### A01 / P1：原生协议在没有结束事件时仍把部分回复记为成功

用已有 ModelEvent/raw stream API 发出合法开头和一段 `unconfirmed-prefix`，刻意不发送协议结束事件，然后让 HTTP body 正常结束。真实 Agena 结果：

```text
Anthropic:       phase=Succeeded; assert_succeeded=true; assert_failed=false
Gemini:          phase=Succeeded; assert_succeeded=true; assert_failed=false
Ollama:          phase=Succeeded; assert_succeeded=true; assert_failed=false
BedrockAnthropic:phase=Succeeded; assert_succeeded=true; assert_failed=false
```

这与强制断开 TCP 不同：HTTP 传输可以干净结束，而应用协议的消息仍未完成。现有断连接测试不能自动覆盖该场景。四个适配器在 EOF 后，只要已经看到部分内容或 usage，就可能合成 Completed。框架已经能表达这个反例，缺的是产品回归与严格的终态判断。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider-adapters/src/provider/anthropic/anthropic_runtime.rs:784`；`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider-adapters/src/provider/amazon_bedrock/bedrock_adapter.rs:1334`；`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider-adapters/src/provider/gemini.rs:498`；`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider-adapters/src/provider/ollama.rs:375`。

官方协议参考：[Claude streaming](https://platform.claude.com/docs/en/build-with-claude/streaming) 的事件序列以 message_stop 结束；[Ollama chat](https://docs.ollama.com/api/chat) 以 done 表示完成，[streaming 示例](https://docs.ollama.com/api/streaming) 最后一条为 done:true。Gemini 的本次结论来自实际输入中无 finishReason 而产品仍合成 Completed 的实验；具体的合法终态集合应按适配器契约逐项定义。

验收：分别覆盖空流、只有开头、文本中途 EOF、工具参数未闭合、已有完整工具参数但无消息结束、缺终态后断连接、正常结束、明确 error/incomplete/安全拦截。保留可见部分输出，但不能把未确认完成标为 Succeeded；不得重复执行已发生的工具副作用。原生回归必须走实际适配器和 durable execution 状态。

### A02 / P1：虚拟推进与真实心跳混用，健康的活跃交付会失去 lease

调度 lease 长度为 90 秒，过期判断和完成提交使用受控墙钟；心跳却每 30 秒真实时间执行。暂停 sink 返回，前进虚拟 6 小时，再让 sink 返回 submitted，会使第一次完成更新被拒绝，之后重新投递。

```text
after_first_sink_success: completed=false, history_count=0, pending_delivery=true
unexpected_second_sink_call: elapsed_real_ms=5011, same_delivery_key=true, attempt=1 -> 2
final: completed=true, history_count=1, sink_invocations=2
```

这是单实例复现。相同 delivery_key 被保留，因此产品 sink 仍可幂等去重；本实验没有证明用户实际收到重复消息。它证明当前 advance 不能被理解为整个产品依序经历了 6 小时的 timer/heartbeat。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-scheduler/src/store.rs:24`、`/home/canxin/Git/ai_research/agena/crates/agena-scheduler/src/store.rs:929`、`/home/canxin/Git/ai_research/agena/crates/agena-scheduler/src/scheduler/worker.rs:290`、`/home/canxin/Git/ai_research/agena/crates/agena-scheduler/src/scheduler/worker.rs:302`。

验收：区分推进业务时间与瞬间跳变墙钟；业务 heartbeat、retry、lease 使用一致的可控时间源。健康交付跨多个虚拟心跳只调用 sink 一次；显式停心跳后才在精确边界失去 lease；旧 owner 不得提交；重投的最终产品副作用恰好一次。短期若仅支持 idle 边界推进，API 应明确验证此条件。

### A03 / P1：杀死 Agena 并 finish 后，产品创建的独立 PTY session 仍可存活

通过 TerminalDriver 创建真实 PTY，运行 `trap '' HUP; printf '%s' $$ > audit-owned-child.pid; sleep 25`。case.crash 和 case.finish 均成功，但 PTY shell 仍处于 S 状态，拥有与 Agena 不同的 session ID。探针随后定向清理并确认所有记录进程已消失。

正常产品的进程包装本身就使用 Unix ProcessSession，PTY 也建立独立 session；仅杀产品主 session 不能证明这些后代全部回收。当前所有权测试主要验证 Agena leader PID 消失，未覆盖该反例。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-process/src/lib.rs:64`；`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/process.rs:3`；`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/case.rs:1418`；`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/lifecycle/ownership.rs:58`；`/home/canxin/Git/ai_research/agena/apps/agena/src/server/terminal/manager.rs:941`。

验收：跟踪或隔离 case 的实际后代身份，验证普通 child、独立 session、前后台进程组、忽略 HUP/TERM 的子进程在 kill/restart/finish 后均退出。使用 PID 与启动时间等信息防止 PID 重用误杀。Linux 可评估 case 级 cgroup，或 subreaper 结合后代身份跟踪与明确终止策略；其他平台应提供明确的对应机制与原生验证，不能只检查 leader。

### A04 / P1：普通故障从未命中，测试整体仍返回成功

开启 kernel_faults，arm `kernel.before_commit / Fail`，随后不执行任何触发操作，测试体直接返回 Ok。结果为 `run=Ok`，最终 `remaining=1, hits=0, released=false`。

case finish 检查专门的 expected_kernel_crash 和模型脚本，却不验收普通 fault arm。忘记调用 wait_hit，或产品变更后绕开了注入点，故障回归可以退化为正常路径而继续通过。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/fault.rs:106`；`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/fault.rs:173`；`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/case.rs:1597`。

验收：arm 返回带 arm_id 的 expectation；默认要求命中正确次数、正确目标并按 effect 结束。显式 cancel/disarm 或 expect_not_hit 才能允许不命中；清理报告未消费和未释放的注入。检查整个 arm 历史，不能只读取最后一个控制文件状态。

### A05 / P1：未结束的模型回复会被普通清理取消，并满足默认收尾检查

注册 held 回复，仅 `user.send` 和 `step.observed` 后返回，不 release，也不 wait_settled。TestAgena.run 返回 Ok，但随后 step.finished 返回 `response disconnected before its body finished`。产品正常退出码为 0。

根因是 finish 先关闭产品，再验收 model；step terminal 同时接受 finished 和 disconnected。正常取消测试允许 disconnected 是合理的，但框架没有要求调用者明确声明取消。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/case.rs:1642`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/case.rs:1680`；`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:343`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:699`。

验收：正常回复默认要求正常结束；中断场景显式 expect_disconnected/allow_cancelled。跟踪已接受的 run receipt，结束时报告未经 settlement 或取消验收的 run。保留当前 wait_settled 对 execution/generation 的严格判断；不能用粗略 idle 替代它。

### A06 / P1：ModelRequest 丢弃 URL query，无法断言 Gemini 等协议的重要参数

真实 HTTP 对照：`?alt=sse&key=AAA` 和 `?alt=wrong&key=BBB` 都被接受并返回 SSE。除 sequence/step_id 外，两份公开 ModelRequest 的 path、headers、body、preview 完全一致。当前生产 Gemini 会发送正确 alt=sse；这里证明的是测试无法发现未来遗漏或错写参数的回归。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:1079`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:1166`；`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model/request.rs:6`。

验收：保留 raw URI 与有序多值 query，提供 query matcher；测试缺失、错误、重复参数和编码差异，诊断中对敏感参数做适当处理。

### A07 / P2：自定义网关路径丢失协议语义

配置 options.protocol=Gemini，并注册 `/gateway/models/...`，发送 streamGenerateContent。观察结果为 protocol=None、streaming=false、model=None；统一 Reply::text 返回 application/json。fallback 选择了 Gemini 编码，但 ModelRequest 的路由解释仍依赖固定原生路径。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model/request.rs:6`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model/request.rs:11`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model/request.rs:28`；复现记录为 capability_probe.log。

验收：路由匹配结果应携带协议、操作类型和 model 提取规则；不能只在回复编码阶段 fallback。标准路由和带网关前缀路由应通过同一套 JSON/stream/tool request matcher 测试。

### A08 / P2：选择 Anthropic fixture 后，默认仍接受 OpenAI Responses

配置 Anthropic，发送 POST /v1/responses；未额外写 protocol matcher 时返回正确 Responses SSE，assert_finished 通过。路由判断无条件保留 Responses；回复又可按观测路径自适应。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:1089`。这意味着简单场景中，产品若错误退回 Responses，测试可能仍通过。

验收：单协议 fixture 默认严格限制协议与路由；需要多协议 gateway 时显式启用允许集合。未知 route、错误协议和错误 streaming 模式应失败并产生清楚诊断。

### A09 / P2：终端 contains 可由输入回显和旧 tail 满足

输入 shell no-op `: 'audit-echo-only-F8A19'`，该命令不输出 marker；wait_output_contains 仍因 PTY 回显成功。没有再输入任何内容，第二次等待同 marker 也因旧 tail replay 成功。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/terminal.rs:182`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/terminal.rs:224`。

现有 terminal 场景通过独立文件副作用证明命令执行，这是有效覆盖；但其 SSE marker 也出现在输入里，仍不足以独立证明 stdout 路径。参见 `/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/terminal.rs:23`。

验收：提供订阅就绪、cursor/checkpoint、PTY generation 和命令完成/退出码。输出标记不能原样出现在输入里；旧 replay 和输入 echo 不得满足新命令的输出断言。

## 3. E01 / P1：已有测试没有完整进入 CI

quality.yml 的 rust job 在 Ubuntu/macOS 上执行 fmt/clippy；实际测试 job 仅在 Linux。test-libs 是 `--workspace --lib`，随后只选 SDK transports、一个产品 binary 测试过滤器、system 与 bundled 两个目标。`--all-targets` clippy 只编译检查，不执行测试断言。

| 未纳入常规执行的入口 | 数量 | 说明 |
|---|---:|---|
| 其他 crate 的 integration targets | 30 个目标，161 项 | domain 4、skills 23、mcp-client 11、runtime-config 24、runtime-contracts 3、runtime-tools 1、storage-sqlite 36、tui-transcript 59 |
| 产品 binary tests | 87 项 | 88 项中 CI 仅执行 tmux cleanup 的 1 项 |
| Web Bun tests | 303 项、36 文件 | Web CI 只 build/typecheck/import/i18n/audit，没有 bun test |
| 浏览器探针 | 13 个脚本 | 仓库已有真实 browser probes，未纳入上述 CI |

合计有 551 个现有行为测试入口未被该工作流执行，另有浏览器探针。这个数字指明确遗漏的入口，不表示 551 个独立功能都完全没有间接覆盖。

证据：`/home/canxin/Git/ai_research/agena/.github/workflows/quality.yml:22`、`/home/canxin/Git/ai_research/agena/.github/workflows/quality.yml:72`、`/home/canxin/Git/ai_research/agena/.github/workflows/quality.yml:83`、`/home/canxin/Git/ai_research/agena/.github/workflows/quality.yml:115`；`/home/canxin/Git/ai_research/agena/.cargo/config.toml:27`；`/home/canxin/Git/ai_research/agena/packages/agena-web/package.json:24`。

验收：以 cargo metadata 的实际 test targets 生成执行清单并检查遗漏；完整执行产品 binary tests；Web 明确执行 Bun tests；将浏览器关键路径纳入单独可重复的 job。区分常规、平台、较慢专项门禁，列出的每个门禁都需真实执行，而非只统计声明或编译目标。

## 4. 当前可构造，但断言或测试层次仍不足

### B01 / P2：实时通知正确性被状态轮询兜底掩盖

UserDriver 的订阅事件主要用于唤醒等待，不验证 payload；错误自动重连，每 50ms 又会读取权威 HTTP snapshot。因此普通对话、权限或工具测试成功只能证明最终状态，不证明通知内容、顺序、scope、延迟、断流恢复正常。轮询兜底本身是合理设计。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/user.rs:132`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/user.rs:535`。inline_ui 已有直接 live.recv 断言，不能把它一并说成未覆盖，见 `/home/canxin/Git/ai_research/agena/apps/agena/tests/bundled/core_contracts.rs:166`。

建议独立 Event/StreamDriver：订阅确认后再操作，验证 event identity、scope、序列、重复与 lag/reconnect；专项事件测试禁止 polling fallback，普通状态测试保留兜底。能够暂停消费者和断流，验证恢复时不会漏状态或重复副作用。

### B02 / P2：原生协议测试注入 thinking/usage，但未验收完整产品语义

uniform native round-trip 注入 thinking、并行工具、usage/cache/reasoning token 和 UTF-8 分块；最终主要验收文本和两个工具回传。没有相应断言证明 reasoning part、计量字段、provider metadata 和签名在持久化、下一次请求及 restart 后仍正确。Bedrock 请求只检查 Authorization 前缀和 x-amz-date，不能证明 SigV4 的 canonical request、区域、服务、凭据和签名值正确。

证据：`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/native.rs:210`、`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/native.rs:230`、`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/native.rs:242`。

建议逐字段核验 durable parts、usage 累积、cached/reasoning tokens、stop reason、原生 metadata、工具调用 ID 和第二轮请求原生块；验证 restart 后再回传。不要把 fixture 自己生成/读取同一结构的测试当作真实适配器的完整保真证明。Bedrock 可用独立签名验证器或固定签名向量。

此外，同时返回两个工具结果不证明工具实际并行执行；串行执行也能通过现有断言。要证明并发，需两个可分别等待的工具 gate，要求两者都进入后才释放，并校验每个副作用只发生一次。

原生错误用例主要检查 assert_failed，仍应验证结构化错误类别、retryable、尝试次数、code/message 映射和敏感信息处理。流内 error 与 HTTP 400 不应被同一个‘失败了’断言视为充分等价。

### B03 / P2：故障 callback 单元测试不等于产品恢复测试

FULL/WRITE/SYNC 已有真实 product create_session 失败、kill/restart、再写成功场景。READ/OPEN/TRUNCATE 主要在单连接 VFS 测试中通过 connect、read、显式 checkpoint 触发。尚不足以证明启动退出码、读池补连、cache 是否过时、自动 checkpoint 和 shutdown 失败传播。

证据：`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/control.rs:313`；`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel/test_fault/vfs/tests.rs:193`。

验收：每种操作至少覆盖一条真实产品生命周期或业务路径；多读者、长事务、checkpoint busy、恢复过程中再次失败需要单独场景。SYNC 失败后事务可能已经落盘，允许不确定结果是正确的；需检查跨表原子性、receipt 和幂等重试，不能武断要求数据必不存在。

### B04 / P2：Cron 状态机覆盖仍有限

新增虚拟时间用例主要验证精确到期、删除优先、暂停与重启、submitted history。仓库另有真实时间的 Cron → 模型请求 → system_notification 端到端测试；它使用 Utc::now()+10s，再执行配置更新，存在慢 CI 错过目标秒的时间窗。

证据：`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/clock.rs:60`；`/home/canxin/Git/ai_research/agena/apps/agena/tests/bundled/runtime.rs:355`。

本次未找到对以下状态机分支的充分产品场景：Skip/Reschedule、真实 retry/backoff 与耗尽、活跃交付时 update/pause/delete、旧 owner finish、多实例 lease 抢占、非 UTC 与 DST 缺失/重复小时、闰日、max_age 精确边界、长时间宕机 backlog 公平性。

建议先修时间域语义，然后用虚拟时间驱动现有真正唤醒模型的端到端测试，再扩展上述矩阵；每项同时验证 scheduled_for、attempt、history、delivery_key 与实际副作用次数。


### B05 / P2：fixture 的非流 JSON 矩阵未等价覆盖非流产品适配器

七协议 JSON 测试由 reqwest 直接读取 fixture 响应，生产对话场景则通过 complete_stream_with_tool_drafts；两者不能证明每个 adapter 的 complete() 路径正确。自动权限审查已有非流式用例，但主要是默认 Responses，尚未构成原生适配器矩阵。应补文本、工具、错误、空结果和 usage 的真实 adapter × stream/nonstream 对照。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model/tests/protocols.rs:122`；`/home/canxin/Git/ai_research/agena/crates/agena-runtime-session/src/session/kernel/processor.rs:291`；`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/interaction.rs:50`。

### B06 / P2：消息协议 fixture 不能替代 provider 鉴权生命周期测试

fake inline key 和 fake static AWS credentials 跳过了 token 过期、并发刷新、rotation、撤销、持久化失败和重启恢复。MCP server OAuth 已有测试，与模型 provider OAuth 是不同状态机，不能相互替代。

OpenAI OAuth issuer 为固定常量，token 请求走这个地址，HTTP client 使用静态 OnceLock；Copilot 拼接 HTTPS 域名，Google/AWS 还有 SDK credential chain。多数状态机无需真实云账号即可验证，但需要每 case endpoint/transport/credential provider 注入和受控时间。

证据：`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider/src/provider/auth/oauth/shared.rs:19`、同文件第 43 行；`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider/src/provider/auth/oauth/openai.rs:242`；`/home/canxin/Git/ai_research/agena/crates/agena-runtime-provider/src/provider/auth/oauth/copilot.rs:14`。Bedrock 签名时间仍是真实 SystemTime，见 `/home/canxin/Git/ai_research/agena/crates/agena-provider-bedrock-signing/src/lib.rs:46`。

建议专门的 AuthDriver/CredentialFixture，验证多个并发 401 只进行一次受控刷新、旧响应不能覆盖新 token、刷新取消不污染持久状态、重启后仍保留正确 generation。GitLab instance URL 可配置，应按实际接口能力区别对待。

## 5. 需要扩展控制或观测能力的场景

| 能力域 | 当前边界 | 有价值的扩展与验收 |
|---|---|---|
| 故障目标精度 | 同一 case 控制文件一次只 arm 一个 point，首个匹配操作消费；file 是命中后记录 | request/transaction/batch/file/offset/第 N 次调用过滤；两个并发请求只指定者失败 |
| 提交后崩溃 | 精确 gate 在 COMMIT 之前，不能卡在 commit 成功到投影/receipt/HTTP 响应之间 | after_commit、before_projection、before_response gate；同 RequestId 重试必须复用结果和副作用 |
| 磁盘与 syscall | SQLite MAIN_DB、MAIN_JOURNAL、WAL 的 Open/Read/Write/Sync/Truncate；debug hooks | 持续 ENOSPC、重复 EIO、短写、撕裂写、丢写、sync 返回成功但不持久化、锁/SHM/目录同步；产品配置/附件/解压写失败 |
| 产品时间 | TestClock 控制 scheduler UTC 和显式虚拟 sleep/timeout；其他许多生产 timer 仍真实 | 可控业务 monotonic clock、provider retry、OAuth TTL、web push retry；真实测试 watchdog 继续保留 |
| 确定性竞争 | pause_scheduler 只停止新接纳，已开始交付继续；同 deadline 的 sleeper 仍由 Tokio 排程 | timer 注册/排空、advance_to_next、claim/sink/persist 命名 gate、可回放 trace；稳定执行 A→B 和 B→A |
| 网络故障阶段 | ModelDriver 是 loopback HTTP/1；held 只延迟 body，响应头已发送 | accept/header/body 分阶段 gate、半关闭、带宽/背压、连接池复用；TLS/DNS/HTTP2 专用 fixture |
| 协议完整覆盖 | 7 种 wire 编码；6 个产品适配器；Bedrock Converse 为 wire-only | 按 vendor 维护真实适配器能力矩阵；JSON、流、工具、多模态、错误、取消逐项验证 |
| 长期资源 | 有界日志和登记 task 的清理，但无通用 RSS/FD/thread/child/queue 预算 | warmup 后循环 reload/cancel/reconnect 的净增长预算；区分 fixture 和 product 资源 |
| 权限环境 | case root 在 tempdir，而产品默认放行 <tmp>/** | 受控非临时目录场景或明确权限环境配置；保持真实生产规则，测试继承与优先级 |
| 平台/构建 | 实际系统测试为 Linux debug；macOS 主要 fmt/clippy；Windows 无对应执行 | OS 原生 Job Object/PTY/文件锁/路径测试；release 生命周期和 feature 矩阵 |
| 失败取证 | case 会保留根目录，但 CI 上传只取 diagnostics/** | 带清单和预算保存必要 DB/WAL、配置、fault/clock 状态、脚本和进程身份，保证失败可复现 |

关键定位：

- 故障消费：`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel/test_fault.rs:101`、`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel/test_fault.rs:129`。
- 提交窗口：`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel.rs:1805`、`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel.rs:1816`。
- VFS 范围：`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel/test_fault/vfs.rs:139`、`/home/canxin/Git/ai_research/agena/crates/agena-storage-sqlite/src/kernel/test_fault/vfs.rs:169`。
- HTTP/1 fixture：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:760`、`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/model.rs:812`；held header/body 差异已在 capability_probe.log 实证。
- 真实业务 timer：`/home/canxin/Git/ai_research/agena/crates/agena-runtime-session/src/session/manager/execution.rs:623`、`/home/canxin/Git/ai_research/agena/crates/agena-runtime-session/src/session/manager/execution.rs:746`；OAuth 见 `/home/canxin/Git/ai_research/agena/apps/agena/src/server/mcp.rs:2469`，Web Push 见 `/home/canxin/Git/ai_research/agena/apps/agena/src/server/web_push.rs:165`。
- 权限：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/case.rs:402`；`/home/canxin/Git/ai_research/agena/crates/agena-domain/src/permission_config.rs:125`；已有明确路径覆盖规则见 `/home/canxin/Git/ai_research/agena/crates/agena-testkit/README.md:741`。
- 诊断：`/home/canxin/Git/ai_research/agena/crates/agena-testkit/src/case.rs:1724`；`/home/canxin/Git/ai_research/agena/.github/workflows/quality.yml:97`。

补充：当前 kernel Hold 有真实 20 秒上限，clock ack 默认真实 8 秒。case 总 timeout 不会自动调整这些预算。较长故障步骤需要明确的 waiting/released/timed_out/aborted 状态；旧的 hits/released marker 不能永远证明 worker 仍停在 gate。源码风险判断尚未另做超时 probe。

ModelDriver 的简洁流门闩也有边界：当前是一个 hold 点和 sticky release，不能直接编排‘发 A→等动作 1→发 B→等动作 2’；request body 会先完整读取再发布 observed，不能用同一 API 精确中断上传阶段。fragment_bytes 能制造碎片化条件，但 TCP 可以合并，不能据此证明 parser 经历了每个预定字节边界；chunks_sent 也只证明已送入 fixture 内部 channel，不代表产品解析或持久化完成。multipart、二进制上传和 WebSocket 音频需使用通用 fixture；现有 provider/realtime 专项应继续保留。

当前 SIGKILL 加真实 SQLite VFS 故障不等于硬件断电模型：OS 页缓存、磁盘缓存和返回成功但实际未持久化的写入尚未模拟。硬件级 crash consistency 应使用独立 durable-image VFS 或隔离环境。

## 6. 产品领域的下一层测试矩阵

| 领域 | 已有有效覆盖 | 仍应补齐 |
|---|---|---|
| Git 工作台 | 初始化、分支/提交、状态分页、路径拒绝、本地 bare remote clone/push/fetch/pull | merge/rebase/cherry-pick 冲突与 abort/continue、stash/undo、patch、submodule/LFS、worktree 并发、认证/凭据失败 |
| 文件工作台 | 静态 symlink 拒绝、读写/搜索/替换/删除、原子写 | 检查后路径被替换的 TOCTOU、并发编辑冲突、取消期间原子发布、watcher 丢事件和资源耗尽 |
| Marketplace | SHA 校验、缺签名时 require-signature 拒绝、依赖回滚、恶意 tar/外部链接/超大声明条目 | 真实有效签名/错误签名/错 key/轮换，升级与卸载竞态，解压取消，缓存污染和恢复 |
| Preview / 浏览器 | 51 个 binary tests、13 个浏览器探针中的相关脚本，含真实 proxy/cookie/WS/revocation | 将现有探针纳入 CI；统一隔离、失败取证、产品版本选择；Vue 页面真实交互与后端恢复联动 |
| TUI | reducer、布局、转录、输入归一化；59 个 transcript integration tests | 完整产品 PTY 键盘→协议→帧渲染、resize/theme/IME、退出/重连；TerminalDriver 只驱动工作台 shell，不等于 TuiDriver |
| OAuth/MCP | binary 内已有刷新轮换、撤销、重启等测试，另有本地 MCP/browser probe | 纳入 CI；冻结 TTL/竞争窗口、多客户端并发 refresh、stream 恢复、真实 provider 特有 auth 流 |
| 负载/鲁棒性 | 若干手工构造的并发/饱和场景 | queue 上限、公平性、慢消费者、长期资源趋势、状态机随机操作、parser fuzz/属性测试 |

TOCTOU 结论来自路径先检查后使用的结构，尚未在本轮运行实际越界竞争。证据：`/home/canxin/Git/ai_research/agena/apps/agena/src/server/fs/fs_core.rs:178` 和 `/home/canxin/Git/ai_research/agena/apps/agena/src/server/fs/file_write.rs:288`。静态 symlink 测试存在，并不能代替竞争测试。

Marketplace 的缺签名拒绝已经测试，证据：`/home/canxin/Git/ai_research/agena/apps/agena/tests/scenarios/marketplace.rs:303`；实际 Ed25519 验证入口为 `/home/canxin/Git/ai_research/agena/crates/agena-plugin-marketplace/src/installer.rs:30`。不能继续沿用‘缺签名完全没测’的旧表述。

Preview 浏览器 probe 已有真实 Chromium、cookie、WebSocket、stop/restart 和 SIGTERM 清理断言，见 `/home/canxin/Git/ai_research/agena/packages/agena-web/scripts/probe-workspace-preview.cjs:1`、`/home/canxin/Git/ai_research/agena/packages/agena-web/scripts/probe-workspace-preview.cjs:1111`。其说明也明确 tiny workbench shell 不是 Vue component rendering。此处应解决执行与统一，而非从零重写同类 probe。

## 7. 优先实施顺序与完成标准

1. **先关闭已复现的误判入口。** 把 A01–A06 和 PTY/echo 对照场景转成持久回归；修复原生 EOF、时钟/lease、独立子 session 清理、未命中 fault、非预期取消和 query 丢失。每项应有修复前失败、修复后通过的证据，正常完成/明确取消仍通过。
2. **补全执行门禁。** 执行所有列出的 integration/bin/Web 测试，并加入目标清单校验；接入已有关键浏览器 probe。避免以 clippy、静态测试数或文档库存替代执行。
3. **补齐高风险产品状态机。** 提交后响应丢失与幂等恢复、Cron retry/misfire/lease/DST、READ/OPEN/TRUNCATE 产品恢复、多协议终态/usage/metadata/多模态与工具取消。
4. **扩展控制和观察 API。** FaultHandle、阶段门闩、Event/StreamDriver、Terminal cursor/command receipt、资源快照；然后完善网关协议路由和传输阶段控制。
5. **增加平台与系统性验证。** release/feature/OS 矩阵、经过校准的负载测试、定向 mutation/属性/fuzz、代码覆盖与 diff coverage。覆盖率用于发现未执行代码，不能证明断言强度。

建议每条测试同时写清四件事：操纵了哪个控制点；用何种独立证据观察结果；哪一种错误实现必须令测试失败；最后如何证明资源已回收。仅增加 happy-path 数量不能解决本轮发现的误报通过。

## 8. 应保留的正确能力与结论边界

- wait_settled 已绑定具体 execution/generation；不要退回简单 idle 判断。
- VFS 故障发生在真实 SQLite 回调并返回 extended code；默认 VFS 和不同 case 的隔离已覆盖。
- fault 控制文件已有稳定 sidecar OS lock；上一轮修复过的 arm 覆盖竞争不再作为本轮缺陷。
- 非法时钟文件不会静默切回真实时间；scheduler ack 明确只代表 admission boundary，不能要求它证明 delivery 完成。
- committed WAL 未 checkpoint 后的崩溃恢复已有存储测试；当前缺口是产品 receipt/事件/副作用链路的精确 crash window。
- ModelDriver 的 message/events/raw API 可以表达扩展与畸形消息，不能把‘没有 typed helper’一律称为不能测；Bedrock Converse 的 wire-only 状态是明确边界。
- 本轮没有运行完整浏览器探针、跨平台执行、release 行为套件、LLVM 分支覆盖或长时间负载；未给出这些方面的通过结论。

## 9. 复现资料索引

- 生命周期、echo/replay、普通 fault 未命中、held run：`/tmp/agena-harness-audit-20261107/harness_probe.rs` 与 `probe.log`。
- query 丢失：`/tmp/agena-model-audit-20261107/query_probe.rs` 与 `query_probe.log`。
- 自定义路径、错协议、header/body gate：`/tmp/agena-model-audit-20261107/capability_probe.rs` 与 `capability_probe.log`。
- 四协议 EOF：`/tmp/agena-model-audit-20261107/native_eof_probe.rs` 与 `native_eof_probe.log`；保留的案例目录为 `/tmp/agena-testkit-eUoHMa`、`/tmp/agena-testkit-Y0gZNN`、`/tmp/agena-testkit-OIWJYQ`、`/tmp/agena-testkit-nBhvGL`。
- scheduler lease：`/tmp/agena-time-fault-audit-20261107/src/main.rs`、`Cargo.toml` 与 `repro.log`。
- 本轮补跑日志：`/tmp/agena-framework-audit-20261107/`。

以上 /tmp 内容是当前审查环境的诊断材料；核心步骤、结果、源码定位和验收条件已保存在本报告中。
