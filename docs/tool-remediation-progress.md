# 工具整改记录：第一批

> 此文件保留第一批结束时的历史状态；最新第二批实现与本地验收结果见 `docs/tool-remediation-batch2.md` 和 `docs/tool-remediation-batch2-status.json`。下文“仍未完成”是当时的记录，不作为当前状态判断。

本记录对应 2026-09-21 的逐工具审计。实现基于 `d4ef9f3c` 加已有工作区改动；保留此前的原生 PTY 实现。没有提交 Git，没有替换或重启当前运行的 Agena/MCP 服务。

## 已落地的具体修复

| 范围 | 修复及验证入口 |
| --- | --- |
| `fs.replace` | `old`/`new` 原样保留；允许仅空白的非空匹配；读取内容与 revision 比较一致；在分配替换结果前检查 16 MiB 限制。真实工具入口验证缩进、中文、CRLF、空搜索和过期版本。 |
| `fs.apply_patch` | 拆出严格解析和完整行匹配；使用 `@@` 上下文、顺序源游标、EOF 和无末尾换行标记；歧义和未知语义在写入前失败；diff 来自实际文件变化；补丁/逆补丁回归覆盖移动、删除恢复和空行。原有多文件预检与回滚保留。 |
| `fs.stat` | 保留请求路径的叶子符号链接；正常和悬空链接均返回链接身份。普通文件通过同一打开句柄计算有界哈希，并检查读取期间的长度/修改时间变化。不是对恶意文件系统竞态的 OS 沙箱保证。 |
| `fs.read_many` | 所有请求路径都有独立状态；单文件失败不丢弃其他结果；预算耗尽明确返回 `not_read_budget`；增加成功/失败计数。 |
| `notebook.edit_cell` | 修复 code/markdown/raw 转换字段；默认清除过期代码输出；保留 metadata/已有有效 ID，为新单元格和需要的缺失 ID 生成唯一值；拒绝重复 ID；源文本按字节限额，结果在序列化过程中受 32 MiB 限额约束，提交前检查结构不变量。 |
| 记忆 | UTF-8 截断不再按任意字节切片；移除先删后写；工具更新已有记录要求 `expected_sha256`；正文/索引在同一进程写锁内预备并提交，索引发布失败时回滚正文；精确匹配索引目标，避免 a.md 误匹配 ba.md；YAML 标量正确引用。读取有界并返回实际文档哈希。 |
| 浏览器 | 不自动读取表单控件值；输入动作不返显输入文本；默认只收集 console/log 事件元信息；每条日志按字节有界；快照与引用绑定原 DOM 节点、快照 ID 和签名，过期引用拒绝；快照截断不拆开 UTF-16 代理对。真实 Chromium 用合成页面验证。 |
| 工件/出站声明 | 截图和下载声明实际写路径；默认截图使用唯一文件名。39 个厂商工具声明配置的网络目标与受管工件写目录；受限策略测试确认这些检查不会因只有描述标签而遗漏。共享厂商 HTTP client 不自动跟随重定向。 |
| `web.search` | 区分真实零结果、部分搜索源失败、全部搜索源失败；全部失败返回错误而不是成功空列表。 |
| `session.environment` | Git 状态缺失/不完整时返回 unknown，而不是 clean；提供 `git_status_known`。 |
| `skills.read_resource` | `content_hash` 对应本次读取的资源正文；技能文档版本单独返回 `skill_content_hash`。 |
| `report.findings` | 拒绝逆序行范围；省略 summary 时生成有效结果摘要，合法报告和空发现不再因为摘要为空而失败。 |

## 调用兼容性变化

`memory.write` 创建新记录仍不需要哈希；覆盖已有记录须先读取 `memory.get` 返回的 `sha256`，并提交 `expected_sha256`。这个限制是有意的陈旧写入保护。

`notebook.edit_cell` 的 `preserve_outputs` 默认值改为 `false`。只有明确需要保留已有代码输出时才传 `true`，结果会标记输出可能过期。类型转换仍会移除不适用于目标类型的字段。

浏览器使用数字 `ref` 时必须携带产生该引用的 `snapshot_id`；CSS selector 方式不需要 ID。一次新快照会使旧快照 ID 失效。页面主世界中的快照对象用于普通 DOM 变化的正确性检查，不是对恶意页面脚本的隔离安全边界。

原始页面日志正文默认关闭，可用 `browser.capture_console_text=true` 显式开启；开启后页面日志可能含敏感数据。控件值省略并不等于对任意网页内容的通用秘密检测。

厂商工具即使是查询，也会保存响应回执，因此现在需要受管工件目录的写权限。网络/路径声明进入现有权限管线；它们不构成操作系统级沙箱。厂商接口版本、远端业务状态和收费操作去重仍需后续专门验证。

## 测试与复现

`crates/agena-bundled-plugins/tests/tool_correctness.rs` 从真实 PluginHost/ToolExecutor JSON 入口执行操作并检查文件字节与状态，不以 mock 成功回执替代文件验证。首轮未修复实现运行 10 项测试，其中 8 项失败，作为基线保留。

`crates/agena-bundled-plugins/tests/tool_effect_permissions.rs` 使用真实权限检查入口，在工具本身允许、网络或写权限拒绝时验证所有 39 个厂商工具及浏览器工件。它不读取 API key，不调用付费服务，也不把“权限检查通过”说成 OS 级阻断测试。

内置浏览器测试使用实际 Chromium 的空白合成页面。厂商重定向测试使用本机 HTTP fixture 与假凭据。记忆入口测试在隔离 HOME 的子测试进程中执行，避免接触真实用户记录。最早测试产生的两份已确认合成记录已移出用户存储，保存在忽略的测试工件目录。

本轮日志目录：`.tmp-artifacts/tool-remediation-20260921/`。主要命令：

```sh
cargo test --offline --locked -p agena-bundled-plugins --test tool_correctness --test tool_effect_permissions
cargo test --offline --locked -p agena-runtime-tools --lib
cargo test --offline --locked -p agena-storage --lib
cargo test --offline --locked -p agena-bundled-plugins --lib
cargo test --offline --locked -p agena-bundled-plugins --test docs_reference --test human_rendering --test capability_manifest --test plan_display
cargo check --offline --locked --all-targets -p agena-runtime -p agena-runtime-session -p agena-mcp-server -p agena-tui-app
cargo clippy --offline --locked --all-targets -p agena-storage -p agena-runtime-tools -p agena-bundled-plugins -p agena-runtime-contracts -- -D warnings
cargo fmt --all -- --check
git diff --check
```

## 仍未完成，不作关闭处理

这批修复不是对 140 个工具或 19 个工作包的全部验收。普通进程和 Cron 的统一归属、子任务原子准入、计划版本化审批、统一副作用回执、操作系统沙箱、流式大文件读取/完整输出回读、版本化 LSP、目录指令加载、技能与配置 CAS，以及厂商远端动作账本仍保留在审计台账。

记忆正文/索引回滚目前是单进程内的事务补偿，没有宣称跨进程协调或断电崩溃原子性。浏览器节点引用修复没有替代整个 browser context 的调用主体隔离，也没有完成下载在途限额及全子资源/重定向网络治理。

本次没有执行真实厂商 API 的兼容性认证，没有做 Windows 实机运行认证，也没有更换正在服务本次连接的旧进程。运行新构建并刷新工具目录后，新契约才会进入实际连接。

## 本批最终验收结果

最终选定测试集 **347 项通过、0 失败**：内置插件 96、能力清单 2、文档一致性 1、工具渲染 6、计划界面 9、真实工具入口 18、权限集成 4、运行时契约 27、运行时工具 128、存储 56。

严格 Clippy（`-D warnings`）、运行时/会话/MCP/前端的 `--all-targets` 编译、`cargo fmt --check` 和 `git diff --check` 均通过。工具参考与能力身份快照都已重新生成并经测试比对。

真实 Chromium 测试包含嵌套角色容器下的 password、textarea 和 contenteditable 合成数据；快照与动作结果不返回这些值。实际测试执行标记保存在 `all-bundled-final.log`。记忆哈希、浏览器快照 ID 和 ref 标签均进入纯文本工具结果，避免只有 payload 客户端才能继续交互。

这些是本轮所选验证范围内的通过结果，不代表完整工作区测试、全部厂商 API、所有平台或剩余审计工作包都已认证。
