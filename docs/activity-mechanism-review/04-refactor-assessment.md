# 04 — 是否需要彻底重构？评估与路线图

> **历史研究归档：2026-08-07，基线 `acaeaf76`。** 本文保留当时的调查和设计推演；文中的路径、类型、接口与结论属于该基线，不能直接作为当前实现的契约。2026-10-07 整合时保留原文及分支历史，未将这些旧设计直接应用到运行代码。
>
> 当前代码入口：[Activity runtime](../../crates/agena-runtime/src/activity/mod.rs)、[Session store](../../crates/agena-storage/src/store/mod.rs)、[Plugin tool contracts](../../crates/agena-tool/src/lib.rs)。整合记录见 [本地分支整合记录](../research/local-branch-integration-2026-10-07.md)。

> 回答：**不需要“推倒重来”式的彻底重构（full rewrite），但需要一次有边界、分层推进的结构性重构（targeted refactor）**。
> 现有骨架（单表、单写路径、增量 patch、revision 收敛、模型/用户白名单）是对的；问题集中在“表示层重复”“边界断裂”“文件分治”三类，全部可以在不改存储/事件协议的前提下修复。

## 1. 为什么不需要推倒重来

先看哪些已经是好设计（不应破坏）：

1. **单一内容表 + 单写路径**：`agena_content_nodes` + `project_part_content`（`store/mod.rs:1341`）——所有活动/文本的持久化收敛在一个 upsert 上，`node_id` + `owner_kind/owner_id/position` + `revision_seq >=` 守卫。
2. **增量优先的呈现**：`RuntimePresentationEventKind`（`presentation_event.rs:9`）统一 `TranscriptPatch / OperationDetailDelta / ActivityChanged / Refresh`；owner 反查（`store/history.rs:20 conversation_identity_for_message`）让工具活动是增量 patch 而不是全量刷新。
3. **模型/用户分离**：`wire_message.rs:104` 白名单投影 + `normalize_prompt_messages` 剔除无可见 payload 的消息。
4. **收敛逻辑**：`TranscriptSnapshot::apply/merge`（identity+revision 收敛）、`derive_operation_markdown` 三处复用。

这些是 v10/v11 存储重构 + 多次迭代的成果。全量重写会破坏：存储 schema、事件流格式、TUI/Web/API 三个消费者、数百个测试——而收益不确定。

## 2. 结构问题清单（按边界归类，全部有证据）

### 2.1 表示层重复（核心债）

**同一逻辑对象有 3-4 种表示，且手工映射：**

| 对象 | 表示 | 位置 |
| --- | --- | --- |
| 工具操作 | `OperationPart`（runtime） | `runtime-contracts/message/part/tool.rs:1080`（1583 行大文件） |
| 工具操作 | `ActivityPayload::Operation`（domain transcript） | `domain/activity.rs:560` |
| 工具操作 | `OperationPartResource`（API wire） | `api/message_part.rs:165` |
| 工具操作 | `MessagePartDetailResource::Operation`（API detail） | `api/message_part.rs:80` |

映射点分散：`activity_payload`（`store/mod.rs:1169`）、`project_part_detail`（`manager/history.rs:1465`）、`project_operation_part`（`manager/history.rs:1519`）、`From<MessagePartDetailResource>`（`tui-transcript/render_model.rs:153`）、web `canonicalPart`（`chatRenderModel.ts`）。

**呈现推导重复 3-4 份：**

- TUI 快照路径：`snapshot.rs:430 activity_presentation`（18 变体大 match，~170 行）+ `operation_activity_title`（`snapshot.rs:607`）。
- TUI 消息路径：`transcript_text.rs:118 tool_display_label`。
- Web：`chatRenderModel.ts:33 activityTitle / :54 activitySummary`（另一份推导，且引用不存在的 `model_output_text`）。
- API resource 上没有任何 `title()/summary()` 方法——推导逻辑完全在消费端。

**状态映射重复：** `ExecutionStatus → ActivityState/字符串` 至少在 `history.rs:526`（transcript_part_patch）、`store/mod.rs:1427`（project_part_content）、`store/mod.rs:1183`（text 节点）、`snapshot.rs:623 activity_status` 四处各写一遍。

**TUI 双渲染树：** `TranscriptActivityContent`（`render_model.rs:76`）同时承载 `Canonical(&ActivityPayload)` 和富资源（`OperationPartResource` 等），`render_part_node`（`message_render.rs:1346`）对同一类活动走两条渲染路径（`render_activity_canonical` vs `render_tool_execution`），两套 title/error/result 逻辑。

### 2.2 边界断裂（会产生实际 bug 或数据丢失）

1. **Web 读不存在的字段**：`chatRenderModel.ts:102` `payload.model_output_text`（领域 `OperationActivity` 无此字段）→ Web operation 模型输出恒空。
2. **`HumanToolResultResource` 无生产者**：`api/message_part.rs:451` 的 `human` 字段在 master 没有任何赋值点；真正实现只在未合并分支 `worktree-activity-tool-streaming`（16 文件 + 813 行）。
3. **API `MessagePartDetailResource` 缺 `Notice`**：runtime `project_part_detail` 有 `SessionProjectedPartDetail::Notice`（`history.rs:1498`），API 8 变体没有 Notice（`api/message_part.rs:74`），消息投影边界上 notice 会丢失/被吞。
4. **Web 只处理部分类型**：`chatRenderModel.ts` 只特判 resource/operation/error/skill_reference/attachment，notice/hook/text_segment/maintenance 走通用分支。

### 2.3 死代码 / 半成品

| 项 | 证据 |
| --- | --- |
| `SkillExecution/Checklist/Search/FileChanges/NestedTask` payload | 无任何非测试构造点（grep 全仓库） |
| `Custom` payload | 仅测试构造；插件无向 transcript 发自定义活动的通道（`Custom` 空转） |
| `RuntimeActivity::Error` / `ErrorPart` | `ErrorPart {` 全仓库无构造点；错误由 operation/reply 承担 |
| `ActivityNode::new` / `ActivityNode::transition` | 生产代码无调用（runtime 用手写 struct 字面量，`history.rs:449-472`） |
| `ActivityOwner::Session` | 有 schema + 读取 + TUI 渲染，无运行时写者 |
| `ActivityProvenance` | 只有 TUI composer 设置；`parse_activity_row` 硬编码 `Default::default()`，持久化后丢失 |
| `ActivityPayload::Progress` | 只在重试 live 节点出现，不持久化 |

### 2.4 文件分治

- `domain/activity.rs` 1609 行（类型 + 文档 + 测试混排）
- `tui-transcript/snapshot.rs` 2959 行（呈现推导 + 大量测试）
- `manager/history.rs` 1845 行（事件投影 + 快照 + SessionManager 方法 + 工具函数）
- `history/store/mod.rs` 大（8 个 `INSERT INTO agena_content_nodes` + 4 个 UPDATE，列集合各有微差，改列易错位）

## 3. 建议的重构路线图（按价值/风险排序）

### A 阶段：表示层收敛（最高价值，不动存储/协议）

1. **给 `ActivityPayload` 加核心呈现方法（或 trait）**，收敛 title/summary/problem：
   ```rust
   impl ActivityPayload {
       pub fn schema(&self) -> &str;          // "operation" / "notice" / ...
       pub fn title(&self) -> Cow<'_, str>;    // 各变体的标题
       pub fn summary(&self) -> Cow<'_, str>;  // 折叠摘要
       pub fn problem(&self) -> Option<&UserProblem>;
   }
   ```
   然后 `snapshot.rs activity_presentation`、`transcript_text.rs tool_display_label`、web `activityTitle/activitySummary` 都改为消费它（web 可通过 API 序列化该派生字段或同构重实现一次并加契约测试）。
2. **合并 Operation 双形**：给 `OperationActivity` 提供 `from_part(&OperationPart, role)` 构造（内部完成 data 投影 + markdown 派生 + authorization/error 映射），让 `activity_payload` / `parse_activity_row` / `transcript_part_patch` 三处对 operation 的拼装收敛为一处；TUI 的 `Canonical` 与 `Operation` 路径尽量共用同一渲染函数（以 resource 或 payload 二选一为唯一输入）。
3. **集中状态映射**：定义 `impl From<ExecutionStatus> for ActivityState`（域内）并在 store/history/快照复用；`part.status → (state, finished_at)` 收敛为 store 一个 helper。
4. **死代码处置**：删除或实现 SkillExecution/Checklist/Search/FileChanges/NestedTask/Custom/Progress；删除 `RuntimeActivity::Error`（或接上生产端）；`ActivityNode::new/transition` 在 runtime 构造处替换字面量后保留，或删除。

### B 阶段：边界接通（修 P1 bug）

5. 合并 `worktree-activity-tool-streaming`（human blocks 流式），接通 `HumanToolResultResource` 生产端。
6. 修 web `model_output_text`：改为读 `markdown`/`data`，或由快照/API 投影给出 operation 的 model_output 字段（与 TUI 对齐）。
7. API `MessagePartDetailResource` 补 `Notice` 变体（并同步 render_model/web）。

### C 阶段：结构分治（可维护性）

8. 拆 `activity.rs`：按变体分组子模块（`activity/{resource,operation,interaction,error,notice,custom}.rs`），类型定义各自独立，顶层 re-export。
9. 收敛 SQL：8 个 INSERT → 1 个 `upsert_content_node(db, owner, node_type, actor, payload_json, state, position, revision_seq, ...)` 内部函数，减少列错位风险。
10. 拆 `manager/history.rs`：事件投影（presentation_event）与快照查询（transcript_snapshot）分成两个模块。

### D 阶段：机制补强（完整性）

11. 决定 `Custom` 去留：若产品需要插件产出 transcript 活动，加 `HostClient::publish_transcript_activity`（schema+presentation 已就绪）；否则删除。
12. 决定 `ActivityOwner::Session` 去留：加写者（系统级 notice、会话摘要）或移除存储/渲染分支。
13. 加枚举覆盖测试：每个 `ActivityPayload` 变体至少一个“生产 → 持久化 → 快照 → 渲染”测试，防止死变体回归。
14. 错误三表示（`ErrorActivity` / `OperationActivity.error` / `reply.failure`）生命周期文档化，评估是否可合并。

## 4. Rust 最佳实践核对表

| 实践 | 现状 | 建议 |
| --- | --- | --- |
| 新类型（uuid id） | ✅ `ActivityId/TurnId/ReplyId/ToolCallId` 均为 newtype | 保持 |
| serde tag 统一 | ✅ `#[serde(tag="activity_type"/"type")]` snake_case | 保持 |
| 大 enum 变体 | ⚠️ `#[allow(clippy::large_enum_variant)]` 用于协议型 enum 合理（有注释说明）；渲染型 `TranscriptActivityContent` 可 Box | 协议型保留 allow；渲染型按需 Box |
| `From`/`TryFrom` 转换 | ❌ 转换散落为自由函数（`activity_payload`/`project_part_detail`），无 `From` 语义 | 收敛为 `impl From<&OperationPart> for OperationActivity`、`impl TryFrom<MessagePart> for ActivityPayload` |
| enum 固有方法 | ❌ `ActivityPayload` 无 `impl`，行为全在消费端 match | 加 `schema()/title()/summary()/problem()` |
| 构造 helper 使用 | ❌ `ActivityNode::new` 存在但生产代码用手写字面量 | 统一走构造 helper，避免 revision/lifecycle 错配 |
| 状态机类型化 | ⚠️ `ActivityState`/`ExecutionStatus` 映射散落 | 集中为 `From` 实现 |
| 密封 vs 开放 | `ActivityPayload` 是 sealed enum，新增类型需改所有 match | 若插件通道落地，`Custom{schema,data,presentation}` 是扩展点，保持 sealed + Custom |
| 文件规模 | ❌ 1600-3000 行单文件 | 按变体/职责拆分 |

## 5. 结论

- **不做**：全量重写。现有架构根基正确，重写风险 >> 收益。
- **做**：A 阶段（表示层收敛 + 死代码处置）是最大收益/最低风险，建议最先做；B 阶段修 3 个已知断链；C/D 阶段按需推进。
- **验收标准**：每个 `ActivityPayload` 变体有唯一 title/summary 来源；TUI 与 Web 渲染同一份派生结果；`HumanToolResultResource` 有生产者；Notice 能穿过 API 边界；无死变体；大文件拆分后行为不变（测试全绿）。
