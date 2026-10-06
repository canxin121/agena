# Agena Activity 消息机制调查报告

> **历史研究归档：2026-08-07，基线 `acaeaf76`。** 本文保留当时的调查和设计推演；文中的路径、类型、接口与结论属于该基线，不能直接作为当前实现的契约。2026-10-07 整合时保留原文及分支历史，未将这些旧设计直接应用到运行代码。
>
> 当前代码入口：[Activity runtime](../../crates/agena-runtime/src/activity/mod.rs)、[Session store](../../crates/agena-storage/src/store/mod.rs)、[Plugin tool contracts](../../crates/agena-tool/src/lib.rs)。整合记录见 [本地分支整合记录](../research/local-branch-integration-2026-10-07.md)。

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
| **更灵活的 Activity 体系设计** | 工具活动事件流（实时标题/结构化块/摘要）；v2/v3 修订：内容只存一份 raw output，视图是投影/工具自渲染，见 `06-flexible-activity-stream.md` | 见 `06-flexible-activity-stream.md` |
| **彻底重构总设计（推翻重来）** | 统一活动模型 + 统一事件协议 + 统一实时通道 + 有界持久化；v3.1 工具自渲染 + 实时增量渲染（RenderDelta），无渲染函数时 fallback 渲染原始输出；单一事实源只存 raw_output；见 `07-comprehensive-redesign.md` | 见 `07-comprehensive-redesign.md` |
| **插件统一接入契约** | 插件只实现一个 `Tool` trait（执行 + 可选渲染），activity 生命周期/持久化/广播/投影全部由运行时自动接线，插件零 activity 知识；旧插件零改动；见 `08-plugin-contract.md` | 见 `08-plugin-contract.md` |

## 2. 报告分卷

- `01-producers.md` — 所有 activity 生产点清单
- `02-types-and-visibility.md` — 类型体系、死变体、AI/用户分离机制
- `03-lifecycle-organization-issues.md` — 创建/更新/删除、代码组织与复用评估、问题清单与建议
- `04-refactor-assessment.md` — 是否需要彻底重构的评估与分层重构路线图
- `05-refactor-plan.md` — 在“标题与内容不变”约束下的详细重构规划（Golden Invariants、分步实施、验证策略）
- `06-flexible-activity-stream.md` — 更灵活 Activity 体系设计：工具活动事件流与三层内容显式化（v2/v3 修订：单一事实源、工具自渲染）
- `07-comprehensive-redesign.md` — 彻底重构总设计 v3.1：统一事件/模型/存储/渲染、实时增量渲染、工具自渲染 + 原始输出 fallback、接口与工作流程
- `08-plugin-contract.md` — 插件统一接入契约：`Tool` trait 声明即接入、运行时自动接线、缺省完整、旧插件兼容
