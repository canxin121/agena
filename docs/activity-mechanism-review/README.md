# Agena Activity 消息机制调查报告

> 分支: `agent/activity-mechanism-review`（基于 `origin/master` @ `acaeaf76`）

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

## 1. 结论摘要（对六个问题的直接回答）

| 问题 | 结论 | 证据/位置 |
| --- | --- | --- |
| 哪些地方产生 activity | 运行时 10+ 处、TUI 乐观输入 3 处、持久化投影 3 类、后台活动 5 类来源 | 见 `01-producers.md` |
| 产生机制是否整体系统完善 | 是。单一内容表、统一写路径、增量 patch、稳定 id + revision 收敛；但前端/API 有未接通的占位和死变体 | 见 `03-lifecycle-organization-issues.md` |
| 代码组织与复用 | 总体良好（单表、单派生入口、apply/merge 收敛），但三套并行 taxonomy 手工映射、大文件、未合并分支、文档漂移 | 见 `03-lifecycle-organization-issues.md` §4 |
| 类型是否完善充足 | 18 个 ActivityPayload 变体偏多，其中 6-7 个在生产代码中没有任何构造点（死变体）；运行时实际只产生 ~10 种 | 见 `02-types-and-visibility.md` §2 |
| AI 服务器 vs 用户呈现分离 | 机制成熟：模型侧按 payload 类型投影、Operation 双轨（model_preview vs human markdown/blocks）、用户专属类型完全不进 provider；但 Web 端引用不存在的 `model_output_text` 字段 | 见 `02-types-and-visibility.md` §3 |
| 创建/删除机制 | 创建=part 带 ActivityId + 单表 upsert；更新=revision 守卫 + O(1) title + 增量 patch；删除=ContentRemoved（重试成功删除 live 节点），durable 错误按设计保留；无面向用户的 transcript 删除 API | 见 `03-lifecycle-organization-issues.md` §2 |
| **是否需要彻底重构** | **不需要推倒重来**；需要有边界的分层重构（表示层收敛 → 边界接通 → 分治 → 补强），见 `04-refactor-assessment.md` | 见 `04-refactor-assessment.md` |
| **标题/内容不变约束下的重构规划** | 工具调用 activity 的标题/内容由运行时+工具产出、三端只消费不重算；重构须保持 Golden Invariants（标题/摘要/内容字节级不变），见 `05-refactor-plan.md` | 见 `05-refactor-plan.md` |
| **更灵活的 Activity 体系设计** | 工具活动事件流（实时标题/结构化块/摘要）+ 三层内容显式化（AI 原文 / 人类块 / 标签），显式优先、隐式兜底；v2 修订：内容只存一份 canonical，视图是投影，见 `06-flexible-activity-stream.md` | 见 `06-flexible-activity-stream.md` |
| **彻底重构总设计（推翻重来）** | 统一活动模型 + 统一事件协议 + 统一实时通道 + 有界持久化 + 三投影器；**v2 单一事实源**：数据只存一份 canonical（payload/blocks/text），给 AI 与给人看是同一份数据的投影，视图永不落盘；见 `07-comprehensive-redesign.md` | 见 `07-comprehensive-redesign.md` |

## 2. 报告分卷

- `01-producers.md` — 所有 activity 生产点清单
- `02-types-and-visibility.md` — 类型体系、死变体、AI/用户分离机制
- `03-lifecycle-organization-issues.md` — 创建/更新/删除、代码组织与复用评估、问题清单与建议
- `04-refactor-assessment.md` — 是否需要彻底重构的评估与分层重构路线图
- `05-refactor-plan.md` — 在“标题与内容不变”约束下的详细重构规划（Golden Invariants、分步实施、验证策略）
- `06-flexible-activity-stream.md` — 更灵活 Activity 体系设计：工具活动事件流与三层内容显式化（v2 修订：单一事实源）
- `07-comprehensive-redesign.md` — 彻底重构总设计 v2：统一事件/模型/存储/渲染、实时零写库、单一事实源投影、接口与工作流程