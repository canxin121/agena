# Agena 测试覆盖盘点（2026-11-07）

以下规模统计保留本轮驱动扩展前的静态快照：**69 个 Rust package**、约 **538,692 行 Rust 源代码**，识别 **1,344 个 Rust 测试声明**。此统计只说明测试入口数量，**不是代码命中率、分支覆盖率、通过率或全面覆盖的证据**。跨 crate 的系统测试会覆盖多个模块；不在此表内的 TypeScript/Web、OS 特定代码、构建脚本应分别评估。

## 已具备的真实产品测试入口

- Chat/Runtime/Session、权限、Hook、工具和插件：apps/agena/tests/scenarios/ 及 bundled/。
- TestClock：冻结/推进调度 UTC 与显式虚拟 sleep/timeout，毫秒边界、调度接纳竞争、满槽位控制和重启确认，见 scenarios/clock.rs；HTTP/清理及全局 Tokio 时钟保留真实时间。
- FaultDriver：正式 kernel.before_commit 门闩、事务中断电、SQLite VFS FULL/READ/WRITE/SYNC/TRUNCATE/OPEN 故障、定向模型连接中断及进程恢复。验证不可见未提交数据、无幽灵记录、提交结果不确定性与恢复，见 scenarios/control.rs、fault_lifecycle.rs 和 storage VFS tests。
- 多协议 ModelDriver：OpenAI Responses/Chat、Anthropic、Gemini、Bedrock Anthropic、Ollama 的统一 Reply/RequestMatch 与真实产品适配器场景；完整 JSON/原生事件和 Bedrock Converse wire-only fixture 由 testkit 协议矩阵测试。
- Git HTTP 工作台：scenarios/git.rs，真实仓库、提交、标签、分页、非法路径及案例独占 bare remote 的 clone/push/fetch/pull。
- Filesystem HTTP 工作台：scenarios/workbench_fs.rs，真实读写/分块/搜索/替换/删除及越界拒绝。
- Plugin marketplace：scenarios/marketplace.rs，CLI 模板、打包/校验、离线安装、依赖回滚和卸载。
- Terminal PTY：scenarios/terminal.rs，真实会话、流式 SSE、输入、停止/恢复/重启。
- StorageLayout、process、Git 诊断：独立真实文件/子进程测试，捕捉 OS 边界。
- TUI settings：配置选择不串值、保存空字符串、过滤、导航和按键行为的直接回归。

## 深度审查与后续矩阵

2026-11-07 后续审查已保存于 `/home/canxin/Git/ai_research/agena/docs/testing/framework-deep-audit.md`。该报告区分已复现缺陷、弱断言、已有测试未执行，以及尚缺的控制能力，并包含源码定位和验收条件。本轮另补跑 Web 303、产品 binary 88、其他 integration 125 项，均通过；这些通过结果不排除新探针揭示的问题。

| 优先级 | 领域 | 已确认问题或待验证边界 | 推荐下一步 |
|---|---|---|---|
| P1 | 原生模型终态 | Anthropic、Gemini、Ollama、Bedrock Anthropic 的部分回复在缺终态、干净 HTTP EOF 后仍记为 Succeeded，已复现 | 实际 adapter 的终态/EOF/工具参数矩阵，按协议严格判断完成 |
| P1 | TestClock | 活跃交付跨虚拟 6 小时推进后，lease 失效并再次调用同 delivery_key 的 sink，已复现 | 统一受控业务时间与 heartbeat；区分推进和墙钟跳变 |
| P1 | case 收尾 | 已复现独立 PTY session 在 crash+finish 后存活、未命中 fault 仍通过、未完成 held reply 被 teardown 取消后仍通过 | 子进程身份与 containment、FaultHandle、明确正常/取消预期 |
| P1 | CI 执行门禁 | 30 个 integration targets/161 项、87 个 binary tests、303 个 Web tests 未被当前 quality 工作流执行；另有 13 个 browser probe 脚本 | 生成并验收执行目标清单，接入已有测试与关键 probe |
| P1/P2 | ModelDriver | 已复现 query 丢失；自定义 Gemini 路径丢 streaming/model 语义；所选协议仍无条件放行 Responses | 保留完整 URI、协议化路由和严格默认协议约束 |
| P2 | 实时通知、终端 | 轮询成功不能证明通知正确；terminal contains 可由 echo 和旧 replay 满足，后者已复现 | Event/StreamDriver、cursor/command receipt；保留已有文件副作用断言 |
| P2 | 故障与恢复 | 当前注入按首个 point 消费，不能绑定目标请求；缺提交后至响应前的精确 gate；Read/Open/Truncate 缺完整产品恢复链 | 目标化 fault、after-commit gates、receipt 幂等恢复和 I/O 产品矩阵 |
| P2 | Git / 文件工作台 | 已有真实基本操作、本地 remote 和静态 symlink 拒绝；冲突/历史/patch/LFS/worktree 并发及 TOCTOU 仍不足 | 精确冲突夹具、路径竞争门闩、独立副作用断言 |
| P2 | Marketplace | SHA、缺签名拒绝、依赖回滚、恶意 tar 已测；有效/错误签名、错 key/轮换、升级竞态和解压取消仍不足 | 测试签名密钥、并发安装与取消恢复 |
| P2 | Preview / Web | 已有 51 个 Preview binary tests、相关真实 Chromium/cookie/WS/restart 探针；Web 303 个行为测试 | 纳入 CI，统一 fixture 隔离和失败取证，补 Vue 与真实后端交互 |
| P2 | TUI | reducer/布局/输入与 transcript 已有测试；缺完整产品 PTY 键盘→帧渲染→退出/重连 | TUI 专用 driver，不能用工作台 shell driver 替代 |
| P2 | OAuth / provider | MCP server OAuth 已有刷新/撤销/重启测试；模型 provider auth 是另一套固定端点/凭据链状态机 | 注入每 case endpoint/transport/clock，验证并发刷新与持久化 |
| P2 | 平台与 release | Linux debug 系统测试不能证明 macOS/Windows 原生行为；macOS 当前主要 lint，release check 为编译证据 | OS 原生集成测试及 release/feature 行为矩阵 |
| P3 | 系统性质量门禁 | 静态声明数与清单不代表覆盖率；尚缺分支/diff coverage、定向 mutation、资源趋势预算 | 对关键状态机补覆盖工具、属性/fuzz 和重复运行资源观察 |

## 扩展前 Rust package 静态测试清单（只表示入口）

| package | src 行数 | src 测试 | tests/ 测试 |
|---|---:|---:|---:|
| `crates/agena-tui-app` | 69,440 | 170 | 0 |
| `crates/agena-bundled-plugins` | 43,020 | 32 | 0 |
| `crates/agena-runtime-tools` | 25,342 | 41 | 1 |
| `apps/agena` | 23,982 | 88 | 143 |
| `crates/agena-plugin-host` | 22,617 | 41 | 0 |
| `crates/agena-runtime-session` | 21,876 | 88 | 0 |
| `crates/agena-tui-transcript` | 21,008 | 41 | 59 |
| `crates/agena-plugin-sdk` | 20,465 | 78 | 0 |
| `crates/agena-storage-sqlite` | 18,791 | 13 | 36 |
| `crates/agena-application` | 17,579 | 36 | 0 |
| `crates/agena-runtime-provider-adapters` | 16,623 | 15 | 0 |
| `crates/agena-runtime` | 15,168 | 9 | 0 |
| `crates/agena-macro-core` | 15,055 | 3 | 0 |
| `crates/agena-provider` | 13,826 | 7 | 0 |
| `crates/agena-git-http` | 13,551 | 2 | 0 |
| `crates/agena-runtime-provider` | 13,483 | 21 | 0 |
| `crates/agena-domain` | 11,018 | 17 | 4 |
| `crates/agena-tui` | 10,888 | 9 | 0 |
| `crates/agena-tui-plugin-workbench` | 10,440 | 6 | 0 |
| `crates/agena-tui-components` | 10,330 | 9 | 0 |
| `crates/agena-api-server` | 10,059 | 10 | 0 |
| `crates/agena-tool` | 9,780 | 16 | 0 |
| `crates/agena-testkit` | 10,376 | 50 | 0 |
| `crates/agena-cli` | 7,516 | 17 | 0 |
| `crates/agena-storage` | 7,353 | 2 | 0 |
| `crates/agena-runtime-config` | 6,842 | 8 | 24 |
| `crates/agena-client` | 6,454 | 26 | 0 |
| `crates/agena-api` | 6,034 | 23 | 0 |
| `crates/agena-web` | 5,975 | 7 | 0 |
| `crates/agena-runtime-contracts` | 4,942 | 7 | 3 |
| `crates/agena-tui-media` | 4,878 | 29 | 0 |
| `crates/agena-tui-session` | 4,400 | 26 | 0 |
| `crates/agena-mcp-client` | 4,336 | 4 | 11 |
| `crates/agena-plugin-marketplace` | 4,219 | 4 | 0 |
| `crates/agena-tui-platform` | 3,784 | 10 | 0 |
| `crates/agena-skills` | 3,176 | 9 | 23 |
| `crates/agena-scheduler` | 2,983 | 1 | 0 |
| `crates/agena-process` | 2,527 | 4 | 0 |
| `crates/agena-plugin-contracts` | 2,285 | 3 | 0 |
| `crates/agena-lsp` | 1,705 | 0 | 0 |
| `crates/agena-runtime-plugins` | 1,637 | 12 | 0 |
| `crates/agena-tui-settings` | 1,441 | 4 | 0 |
| `crates/agena-failure` | 1,085 | 0 | 0 |
| `crates/agena-permission` | 1,060 | 0 | 0 |
| `crates/agena-mcp-server` | 926 | 3 | 0 |
| `crates/agena-prompt-layers` | 821 | 12 | 0 |
| `crates/agena-tool-api` | 800 | 12 | 0 |
| `tools/agena-web-types` | 742 | 2 | 0 |
| `crates/agena-storage-layout` | 622 | 5 | 0 |
| `crates/agena-tui-permission-studio` | 612 | 0 | 0 |
| `crates/agena-notification` | 573 | 0 | 0 |
| `crates/agena-rollout` | 551 | 0 | 0 |
| `crates/agena-runtime-notifications` | 550 | 0 | 0 |
| `crates/agena-secret-store` | 424 | 2 | 0 |
| `crates/agena-async` | 422 | 6 | 0 |
| `crates/agena-memory-index` | 384 | 0 | 0 |
| `tools/agena-plugin-dev` | 371 | 0 | 0 |
| `crates/agena-tui-provider-studio` | 270 | 0 | 0 |
| `crates/agena-provider-bedrock-streaming` | 261 | 0 | 0 |
| `examples/multi_tool_plugin_stdio` | 154 | 0 | 0 |
| `examples/echo_plugin` | 141 | 0 | 0 |
| `crates/agena-marketplace-server` | 131 | 0 | 0 |
| `crates/agena-stdio-codec` | 123 | 0 | 0 |
| `crates/agena-provider-bedrock-signing` | 113 | 0 | 0 |
| `crates/agena-runtime-session-core` | 109 | 0 | 0 |
| `crates/agena-provider-bedrock-auth` | 104 | 0 | 0 |
| `examples/echo_plugin_stdio` | 54 | 0 | 0 |
| `crates/agena-provider-google-auth` | 45 | 0 | 0 |
| `crates/agena-macros` | 40 | 0 | 0 |

## 如何复验

```sh
cargo --offline test-system
cargo --offline test-bundled
cargo test --offline -p agena-testkit -p agena-git-http -p agena-process -p agena-storage-layout --lib
```

所有用例仍是 Rust 原生 test，使用正式 Agena binary 与可控本地模拟模型。上述命令覆盖的目标不等于整个 workspace 的所有目标、Web/Bun 或全部平台，也不能将测试声明数当成实际执行通过数。
