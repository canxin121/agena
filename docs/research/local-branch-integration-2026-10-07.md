# 本地分支整合记录（2026-10-07）

整合起点为 `master` / `origin/master` 的 `592d67f5`。盘点共 103 个本地分支：除主分支外，85 个已经是主分支祖先，17 个尚未在提交历史上合并。对于这 17 个分支，结合补丁等价、最终 tree、后续重写提交和当前代码逐项判断，避免重新应用已经被主分支覆盖的旧代码。

## 实际合入的生产工作

`feat/shell-fidelity-and-windows-input` 中的 Shell 探测、启动快照、命令启动契约和 Windows ConPTY 输入规范化此前确实未合入。原开发工作区还保留了前后端性能工作；提交前将这些工作分别保存为：

| 提交 | 工作 |
| --- | --- |
| `d7ab5070` | 后端窄查询、批量设置读取、二进制附件上传和性能索引 |
| `b066857a` | 可见 pane 的读取／订阅生命周期、会话投影与渲染、缓存预算、编辑器和首屏资源优化 |
| `b4a9f1f2` | 原性能修复记录与基准数据 |

随后通过合并提交 `909ab0a3` 将整个工作分支整合到主分支，保留原提交历史。该分支中的 `eae4b96f`（消息结束时才推进会话列表排序时间）已经有主分支等价实现，整合保留该行为。

六处冲突涉及设置、文件变更投影、资源版本、附件上传、SQLite engine 和会话 facade。整合保留主分支较新的每个 session 独立缓存锁、part ID 索引、版本校验／CAS、写队列、取消后的 streaming flush 所有权和有界 JSON codec，并把新增功能接入这些边界：

- 新设置接口保留批量叶节点读取，同步设置工作使用有界 worker。
- 二进制附件与旧 Base64 接口共用上传校验、权限检查、大小预算和文件系统 worker；大附件 buffer 复制也使用有界 worker。
- 文件变更 facts 的计算／投影和新增 SQLite／scheduler 查询的 JSON 解码使用有界 worker；缓存淘汰的 payload 在 registry 锁外释放。
- controls 和子任务状态的窄读取继续在 worker 内重建 session；子任务日志投影也有独立的有界 worker。
- Shell 探测、启动文件 metadata 和快照解析移出 Tokio worker；每个 Shell 使用异步 capture gate，整个进程最多同时捕获两个 Shell 启动快照。
- GC 采用最多 2048 IDs 的主键窗口，最多八个短事务，每个最多删除 256 项；保留引用／leaf 校验，事务间 yield。

## 保留的历史研究

| 原分支 | 内容与处理 |
| --- | --- |
| `agent/activity-mechanism-review` | 九份 Activity 调查／设计文档，原始基线 `acaeaf76`；保留原文并为每份文档增加归档说明和当前代码入口 |
| `docs/notification-display-map` | 两份通知显示盘点／重构文档，原始基线 `4eaebcfd`；同样标明历史语义和当前代码入口 |

这些资料来自 2026-08-07，包含已经退役的路径和类型。保留资料及原分支历史，不将旧设计当作当前架构契约。入口：[Activity 研究](../activity-mechanism-review/README.md)、[通知显示盘点](../notification-and-status-display.md)、[通知重构方案](../notification-refactor-plan.md)。

## 不需要重新合入的历史或实验分支

| 分支 | 判断依据 |
| --- | --- |
| `research/tool-alternatives`、`backup/local-transcript-pre-rebase-36f34330`、`agent/align-vim-word-motions`、`worktree-agent-a0a90e1fafd1b76a4`、`fix/activity-folding-stale-notices`、`fix/response-body-retry-plan-autorun` | 分支独有历史的补丁已与主分支等价 |
| `ci/universal-release-matrix`、`backup/master-before-squash-0090c17d` | 最终 tree 与主分支中的 `04348a24` 完全相同；独有提交属于 squash 前的旧历史 |
| `feat/git-safety-workflow` | 主分支 `fb7332c9` 已重做实现；剩余差异主要为旧测试写法 |
| `worktree-agent-a8be35d3cc302a17c` | 主分支 `53828e09` 已实现并扩充验证 |
| `backup/unified-plugin-surface-pre-rebase-c512ddd9` | 六个补丁等价；其余旧 CI／依赖工作对应主分支 `95a3d006`、`92982219` 等后续实现，其中旧 workflow 已退役 |
| `probe/cache-hardening-smoke` | 曾临时替换常规 CI；主分支已有独立 `.github/workflows/sccache-smoke.yml`，包含缓存命中率阈值 |
| `probe/sccache-wrapper-path` | 临时 CI 路径诊断；主分支已有独立配置 action，无需覆盖当前 workflow |
| `probe/release-cgu-16` | 将 release 的 `codegen-units` 从 1 改为 16 的旧构建实验；没有本轮收益证据，保留实验分支，不改生产构建参数 |

原分支和 worktree 记录均保留，没有因整合删除历史或清理目录。

## 本次验证范围

生产整合后的代码已完成以下编译检查：

- `bun run build`：import 规则、settings i18n、Vue／TypeScript 类型检查和 Vite 生产构建通过。
- `cargo check --offline --locked --workspace --features agena-plugin-host/wasm,agena-plugin-host/signing --release --target aarch64-apple-darwin`。
- 相同 release／target 下的默认 `agena` 应用、HTTP-only API、SSE-only API 编译。
- 七个相关 crate 的现有测试 target 通过 `cargo check --tests` 编译；未执行测试。旧缓存用例已适配当前每个 session 的缓存接口和版本校验。
- `git diff --check` 及暂存区检查通过，冲突标记全部移除。

按用户要求，本次没有新增测试，也没有运行测试或基准。原性能文档中的测试数量及基准结果来自原工作分支，不能作为此次主分支整合后重跑的结果。本次仅整合和推送，部署版本不在本次操作范围内。
