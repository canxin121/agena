# 工具整改记录：第二批

本批继续在第一批和原生 PTY 的工作区上修改；没有提交 Git、没有重启当前服务、没有启用生产沙箱，也没有调用真实厂商付费 API。验证工件保存在 `.tmp-artifacts/tool-remediation-batch2/`。

## 已实现的改动

| 领域 | 实现 |
| --- | --- |
| 普通后台进程和监控 | 模型入口的 list/logs/stop 与 PTY 一样验证工作区/会话归属；原始 host 管理入口分开。启动重放比对完整启动参数，禁止同 ID 换命令；普通进程最多 64 个运行、256 条保留；会话结束和运行时关闭覆盖所有后端。 |
| Cron | 创建绑定规范工作区和会话；list/update/pause/resume/delete/history 在 owner-scoped 服务中执行。删除按已授权的精确版本 CAS；历史在内存和 SQLite 中保存 owner，删除任务后仍能安全过滤。旧记录缺 workspace owner 时只允许 host 管理，不默认归入当前会话。 |
| 子任务 | 原子容量预留、持久化后才启动；取消/持久化失败回滚 reservation；缓存以 parent + task_id 键控；followup 使用新运行代次和独立条目，旧完成回调不能覆盖新代次；只有主机接受取消后才进入 cancelling。 |
| 计划 | plan_id/revision、同会话版本检查、审批绑定被审核版本；编辑/清空期间的旧审批不能恢复旧计划。免审批要求可信配置 `allow_unreviewed_activation=true`，模型布尔参数本身不是授权。保存后的显示刷新失败仅作为已提交警告。 |
| LSP | 文档同步串行、版本单调递增，取消会恢复暂存的客户端状态。诊断发布携带版本，旧版不能覆盖新版；返回 current/pending/timeout/stale/unversioned/superseded。空列表只有匹配当前版本才是“当前版本无诊断”。 |
| 修改后的验证 | 文件修改完成后尝试有权限、已配置的 LSP 诊断；总等待有上限，超过预算、未配置、需授权或取消都明确标记。验证失败不把已提交修改说成未执行；不会擅自重写文件进行格式化。 |
| 大文件读取 | `fs.read` 改为有界内存范围读取，不再要求整个文件小于 8 MiB；扫描限制 256 MiB/5 秒，文本预览 64 KiB、最多 2000 行，长行明确缩短；返回 next_offset、只在已知时提供 total_lines。 |
| 输出恢复 | 新增 `fs.output_read` 和 `fs.output_search`。长结果自动提供 output_id，纯文本客户端也能看到。按工作区/会话隔离、最多 256 条、总计 64 MiB、单项 8 MiB、一小时 TTL。缓存过期/被淘汰与上游已经丢失的内容不伪装成可恢复。 |
| 项目规则 | 每轮加载允许读取的根目录 AGENTS.override.md / AGENTS.md / AGENA.md / CLAUDE.md；文件读取补充相应子目录。每文档 8 KiB、总计 32 KiB，保留来源、作用域、显示片段哈希和截断；不赋予额外权限。 |
| Settings | 文件读取返回完整持久配置文档的 revision；set/patch/delete 可提交 expected_revision，并在既有写锁内比较。返回 before/after revision；文件保存与 runtime reload 的 queued/request_failed/not_requested 分开。 |
| Skills | get 返回精确文档 revision；update/delete 必须提交 expected_revision；CAS 在实际文件修改锁内执行。删除后的目录清理、修改后的目录刷新失败保留已提交结果，不伪造成功的目录代次。 |
| 已提交副作用 | 中央 post-execution hook 失败不会抹掉已经返回的工具结果；明确保留“可能已提交、不要自动重做”的警告。适用于完成后的后处理，不改变调用前权限检查。 |
| Provider | 区分 HTTP 收到响应、业务 completed/pending/refused/incomplete/failed/unknown 和本地工件持久化；回执/图片保存失败保留响应 ID 和计费用量。多个图像逐项处理，后续坏图像不会隐藏早前保存的图像。 |
| 浏览器归属 | 所有交互入口检查 caller owner；列表不暴露别人的页面；model 的 browser_shutdown 只关闭自己的页面/上下文，不结束共享 Chrome。每 caller 使用独立 Chromium browser context，隔离 cookie 与下载策略。 |
| 浏览器下载 | 使用 download GUID 及真实完成事件，不用文件大小稳定推断；统计在途/部分文件字节；工作线程在调用方取消后继续执行取消、策略恢复与部分文件清理。失败的 cleanup 显式保留，不声称已经删除。 |
| 模型沙箱 | 可选 macOS 离线 OS profile，通过 `/usr/bin/sandbox-exec` 限制实际子进程读取/写入和网络。foreground/background/PTY 都以直接 argv 包装；删除未允许的继承环境变量。没有实现的平台/网络模式在 required 下拒绝，不无隔离降级。 |
| 活目录与构建 | `session.environment` 报告编译进二进制的工具源码指纹、目标平台、实际可见工具目录摘要及 PTY/输出恢复能力；不是从当前 checkout 猜当前服务支持什么。 |
| 批量帮助 | `tools_help` 混合有效/无效工具名时逐项返回，保留成功的契约；单个未知工具仍明确报错，协议入口不能作为执行工具递归调用。 |

## 新契约与迁移

`fs.output_read` 参数：`output_id`、UTF-8 字节 `offset`、`limit`（1–16000）。`fs.output_search` 参数：`output_id`、字面量 `pattern`、起始字节 `offset` 和匹配上限 `limit`（1–100）。新目录共有 142 个执行工具，不把内部 protocol gateway 数量与执行工具混为一谈。

更新/删除技能要传 `skills.get` 返回的 `revision` 作为 `expected_revision`。Settings 的 `expected_revision` 是兼容性的可选字段，若提供则严格校验；它是完整文件的版本，不是经过脱敏或局部叶值的哈希。计划返回的 revision 同样可用于编辑的陈旧请求检查，批准会自动绑定实际被审阅版本。

`plan.set/phase(request_approval=false)` 只有在可信 plan 设置明确允许时才可激活。既有计划默认兼容读入，但新的免审授权不能由模型自行声明。

`browser_shutdown` 的模型调用语义收紧为 caller-scoped cleanup；管理员全局关闭仍在 host 生命周期控制层。浏览器参考仍须携带 snapshot_id。Chrome context 隔离不等于操作系统沙箱；实际网页脚本仍是不可信内容。

启用 macOS 离线 Shell profile 必须由运行服务的可信环境设置 `AGENA_SHELL_SANDBOX=offline`（或 `required`）；默认未启用。网络型命令在此模式下拒绝，因为没有实现受限域名代理。现有文件/目录的 declared reads/writes 转换为沙箱规则；写新目录时须声明允许的已有父目录。Python/Xcode 等受信系统运行时可读，不默认开放用户 HOME 内容。沙箱限制会导致未声明的工具链/配置/缓存访问失败，这是 fail-closed，不会自动放宽。Linux/Windows required 模式目前明确拒绝。

## 验证设计

测试包含双会话进程/Cron交叉访问、SQLite 删除后历史归属、32 路并发子任务抢占 8 个名额、持久化失败/调用取消回滚、延迟审批时编辑/清空、LSP 慢500ms/乱序/无版本发布、64 MiB 文件的单行读取、隐藏在长输出中间的错误回读、技能/配置过期 revision 拒绝，以及已成功写入后 hook/reload/工件保存失败的事实保留。

真实 Chromium 只运行本机合成页面和本机 HTTP 下载：独立 context cookie、GUID 完成、在途限额、取消清理。沙箱测试要求 Python 真正执行到 socket 调用并捕获 PermissionError；仅在加载阶段失败或出现任意“权限拒绝”字符串不算断网通过。

## 明确边界

本批没有把所有远端 API 的未来/当前协议版本都认证为兼容；已测试的 envelope 字段以外返回 unknown。没有做真实付费工具调用、远端动作的跨进程 exactly-once ledger 或自动交易/发送类副作用重放。

计划/子任务的准入与版本门闩目前是同一服务进程内的保证；不是多副本分布式事务。配置和技能版本检查依托既有文件锁；崩溃一致性仍受实际文件系统与存储实现约束。

输出资源是有配额的进程内恢复缓存，不是永久对象存储，不能找回 PTY/上游捕获层已经丢弃的数据。下载限额是事件和周期计量，不是严格内核磁盘配额，采样期间可能短暂超出阈值。

LSP 服务不报告版本时不自动将空结果说成验证成功；没有强制对所有语言执行自动格式化。新增 project guidance 在文件读取时加载更深层级；直接通过 shell 写入一个从未读取的子目录，不等于已自动获得该目录全部指南。

本批没有替换本次连接正在使用的旧服务。构建/目录验收通过后，是否部署、如何滚动重启和跨平台启用策略必须是独立、可观察的运维步骤。

## 本地验收收尾结果

第二批原有的两个展示失败和严格 lint 阻塞已关闭。本轮没有降低断言要求：为 `fs.output_read` / `fs.output_search` 补齐专用视图、完成标题、继续读取位置和捕获损失提示；重组后台启动参数，移动生产 helper，并修正此前未编译的 Settings 文本版本测试。

另修复两处真实浏览器生命周期/时序问题：根 CDP 连接现在被持续保留，避免新的开页调用断开旧连接并销毁 `disposeOnDetach` 上下文；下载处理区分“收到完成事件”和“最终文件可见”，容忍 `.crdownload` 重命名造成的短暂 `NotFound`，但仍拒绝其他 I/O 错误、超限文件和符号链接，继续受原始取消/超时约束。

| 验收项 | 实际结果 |
| --- | --- |
| 九个核心 crate 的库与集成测试 | **551 通过，0 失败，0 忽略**，共 18 个测试套件 |
| 所有执行工具的展示 | 8 项展示测试通过，包含针对 142 个执行工具的遍历断言 |
| 真实 Chromium | 主测试通过；同一已验证二进制再独立复跑 3 轮，全部通过。每轮含 4 次小文件下载、超限拒绝和取消清理，并检查独立上下文与根连接复用 |
| 严格 Clippy | 九个核心 crate，`--all-targets -- -D warnings`，退出码 0 |
| 下游编译 | Runtime、Session、MCP Server、TUI App、Application、API Server 的 `--all-targets` 均通过 |
| 生成目录与文档 | 能力身份快照 2 项、工具参考 1 项通过，均与本次编译得到的契约一致；本轮没有新增工具参数契约 |
| 仓库不变量 | 19 项全部通过 |
| 格式与差异 | `cargo fmt --all -- --check`、`git diff --check` 均通过 |

每个 gate 都有独立日志、真实子进程退出码和前后源码指纹检查。本次验证指纹为：

```text
0ee722085476dda1157c7ba2f88e2ddef89159562149c7bcfb4fe46d1cb780c8
```

机器可读状态在 `docs/tool-remediation-batch2-status.json`。原始运行结果在 `.tmp-artifacts/tool-remediation-closeout/final2-results.json`，回归日志为 `final2-tests.log`，浏览器复跑证据为 `browser-repeats.json`。此前失败的验证记录保留，没有用局部成功覆盖或拼接成全绿。

这些结果关闭的是本次选定的本地源码验收范围。没有重启生产服务、开启生产沙箱、提交 Git，或执行真实厂商付费调用；跨平台运行、分布式事务和生产部署的边界仍按上文保留，不计入本次通过结果。

## 验收后的源码提交

实现、回归测试、依赖锁文件和生成工具契约已统一提交为：

```text
eee87a68b6412cc55a681d5562a1761f834d1143
feat(tools): add interactive terminals and harden tool execution
```

提交位于仓库实际主分支 `master`。本文件前面的“未提交”描述保留了执行验收时的历史状态；最新机器可读状态通过 `source_commit` 绑定上述已验证实现。文档单独提交，不改变测试所对应的源码指纹。Git 提交不代表重启服务、开启生产沙箱或完成真实厂商 API 认证。
