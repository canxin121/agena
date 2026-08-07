# Agena Activity 消息机制调查报告

> 分支: `agent/activity-mechanism-review`（基于 `origin/master` @ `acaeaf76`）
> 日期: 本次调查会话

## 0. 调查范围

- 文档: `docs/activity-and-model-visibility.md`、`docs/activity-streaming-refactor.md`、`docs/background-activities.md`
- 核心代码:
  - `agena-domain/src/activity.rs`（1609 行，内容文档 + Activity 类型定义）
  - `agena-runtime-contracts/src/message/part/*`（运行时 Part 层）
  - `agena-runtime-session/src/session/manager/history.rs`（1845 行，实时投影）
  - `agena-runtime-session/src/session/history/store/mod.rs`（持久化投影）
  - `agena-runtime-session/src/session/store/history.rs`（checkpoint 持久化）
  - `agena-runtime-session/src/session/manager/replies/*`、`processor/*`（生产点）
  - `agena-runtime-provider/src/provider/wire_message.rs`（模型侧投影）
  - `agena-tui-transcript/src/{snapshot.rs, renderer/...}`（TUI 呈现）
  - `agena-web-ui/src/agena/pages/chatRenderModel.ts`（Web 呈现）
  - `agena-plugin-sdk/src/activity.rs`（插件后台活动接口）

## 1. 结论摘要（对五个问题的直接回答）

| 问题 | 结论 | 证据/位置 |
| --- | --- | --- |
| 哪些地方产生 activity | 运行时 10+ 处、TUI 乐观输入 3 处、持久化投影 3 类、后台活动 5 类来源 | 见 `01-producers.md` |
| 产生机制是否整体系统完善 | 是。单一内容表、统一写路径、增量 patch、稳定 id + revision 收敛；但前端/API 有未接通的占位和死变体 | 见 `03-lifecycle-organization-issues.md` |
| 代码组织与复用 | 总体良好（单表、单派生入口、apply/merge 收敛），但三套并行 taxonomy 手工映射、大文件、未合并分支、文档漂移 | 见 `03-lifecycle-organization-issues.md` §4 |
| 类型是否完善充足 | 18 个 ActivityPayload 变体偏多，其中 6-7 个在生产代码中没有任何构造点（死变体）；运行时实际只产生 ~10 种 | 见 `02-types-and-visibility.md` §2 |
| AI 服务器 vs 用户呈现分离 | 机制成熟：模型侧按 payload 类型投影、Operation 双轨（model_preview vs human markdown/blocks）、用户专属类型完全不进 provider；但 Web 端引用不存在的 `model_output_text` 字段 | 见 `02-types-and-visibility.md` §3 |
| 创建/删除机制 | 创建=part 带 ActivityId + 单表 upsert；更新=revision 守卫 + O(1) title + 增量 patch；删除=ContentRemoved（重试成功删除 live 节点），durable 错误按设计保留；无面向用户的 transcript 删除 API | 见 `03-lifecycle-organization-issues.md` §2 |

## 2. 报告分卷

- `01-producers.md` — 所有 activity 生产点清单
- `02-types-and-visibility.md` — 类型体系、死变体、AI/用户分离机制
- `03-lifecycle-organization-issues.md` — 创建/更新/删除、代码组织与复用评估、问题清单与建议
