# 渠道接入横向对比：竞品做得比 Agena 好的地方

对比对象（均浅克隆于 `~/Git/ai_research`）：AstrBot、LangBot、CowAgent、kirara-ai、MaiBot、dsh-im、nanoclaw、wechaty/NapCatQQ、letta-code、errbot，以及 Autogen/AgentScope/OpenAI Agents SDK/deer-flow/Codewhale 等 Agent 框架，dify/n8n/coze-studio/FastGPT 等平台。

所有结论都在源码里核对过。本文只列**竞品确实更好**的地方，并标注 Agena 为什么需要它、改动量。

---

## 0. 先说 Agena 确实领先的部分（不要照抄竞品）

- **收件箱去重与游标同事务**：重复事件按 `(binding_id, event_id)` 主键拒绝，首次正文不被后续重放替换，游标推进与事件入库在同一事务（`crates/agena-storage-sqlite/src/integration_repository.rs:17,178`）。
- **发送语义**：`SendDisposition::{Sent,Rejected,Retryable,Uncertain}`，丢失回执标记 `uncertain` 且不盲目重发（`crates/agena-runtime/src/runtime/assistant/integrations.rs:915-930`）。MaiBot 的内存 TTL 去重、nanoclaw 的 `MAX_DELIVERY_ATTEMPTS=3` 都不如它。
- **授权深度**：已验证身份信封、五级角色、每角色独立能力、epoch 撤权。竞品普遍只有"允许名单 + 管理员"。
- **插件运输统一**：同一 ABI 覆盖进程内/cdylib/stdio/HTTP（竞品通常各平台一套代码）。
- **私聊约束严格**：`ChannelKind::Direct` 强制 `exact` 线程 + `per_user` 会话 + 显式参与者，禁止 `allow_unlisted_chat`（`crates/agena-domain/src/channel_access.rs:282-291`）。letta-code 自己承认 Slack DM 的 `pairing` 是"legacy unenforced default"，实际等同 `open`（`letta-code/src/channels/types.ts:511-512`）——Agena 在这点上更严。
- **权限管线 fail-closed 且要求分类器引用规则**：自动批准被拒时必须援引 `AUTO_APPROVAL_BLOCK_RULES` 里的具体规则，否则退回交互询问（`crates/agena-permission/src/pipeline.rs:42-56`）。竞品的自动审批普遍没有这个约束。

---

## 1. 适配器能力声明（最高杠杆）

**竞品做法**

- LangBot 每个适配器带 `manifest.yaml`，声明 `supported_events`（`message.edited`、`message.reaction`、`group.member_joined`、`bot.removed_from_group`、`interaction.callback`）、`supported_apis`（required/optional：`edit_message`、`delete_message`、`send_interaction`…）以及 `platform_specific_apis`（`LangBot/src/langbot/pkg/platform/adapters/telegram/manifest.yaml:48-105`）。
- nanoclaw 用可选方法 + `?.` 探测能力：`setTyping`、`setThreadTitle`、`setSuggestedPrompts`、`subscribe`、`openDM`、`syncConversations`；能力也在注册时声明，离线工具无需实例化即可解析默认值（`nanoclaw/src/channels/adapter.ts:209-328`）。
- AstrBot 在适配器元数据上放 `support_streaming_message`、`support_proactive_message`（`astrbot/core/platform/platform_metadata.py:20-26`）。

**Agena 现状**：`AdapterDescription` 只有 `kind/title/description/config_schema/minimum_poll_seconds/idempotent_send/channel_context_version`（`crates/agena-assistant/src/integrations.rs:38-49`）。宿主无法知道适配器能否编辑、反应、发文件、支持线程。后果可见：`ChannelThreadMode` 是纯宿主策略，从不对照适配器能力（`crates/agena-domain/src/channel_access.rs:90-95`）。

**建议**：给 `AdapterDescription` 加一个有界的 `capabilities` 对象（`text_markdown`、`max_text_bytes`、`supports_edit`、`supports_delete`、`supports_react`、`supports_files`、`supports_buttons`、`supports_threads`、`open_dm`），未声明一律按 false 保守处理。**这是一次性解锁其余所有富能力的开关。**

**成本**：3–5 天。

---

## 2. 群/多资源模型：一个账号服务多个群

**竞品做法**

- AstrBot：`unified_msg_origin = platform:message_type:session_id`，一个 bot 账号天然服务任意多个群（`astrbot/core/conversation_mgr.py:103`）。
- LangBot：一个 bot 实例处理所有群事件，群只是事件属性（`src/langbot/pkg/platform/botmgr.py:254-255,619-625`）。
- nanoclaw：`messaging_group_agents` 是**多对多**——一个群接多个 agent，一个 agent 接多个群，路由时逐 agent 扇出（`src/router.ts:346-420`）。

**Agena 现状**：一个绑定只对应一个 `resource`，`event.resource != self.spec.resource` 直接拒绝（`crates/agena-assistant/src/channel_access.rs:70,115,142`）；每 workspace 绑定上限 64（`crates/agena-storage-sqlite/src/integration_repository.rs:139`）。**服务 100 个群要建 100 个绑定，且撞上限。**

**建议**：把 `spec.resource` 从"唯一资源"改为"资源模式/允许集合"；出站校验改成"在允许集合内"。改动集中在上述三处校验。

**成本**：5–8 天。这是扩展性的硬瓶颈。

---

## 3. 富媒体双向

**竞品做法**

- nanoclaw：入站附件先下载、base64、经"拒绝 symlink 的锚定目录 + 独占创建"落到会话 inbox（`src/session-manager.ts:340-430`、`src/anchored-dir.ts:37-140`、`src/attachment-safety.ts:24-58`）；出站 `OutboundFile { filename, data }` 挂在分段回复的第一段，也支持纯文件消息（`src/channels/adapter.ts:105-116`、`src/channels/chat-sdk-bridge.ts:1121-1155`）。
- letta-code：`OutboundChannelMessage` 支持 `mediaPath/fileName/title`、`richMessage{html|markdown}`、`reaction`、`replyToMessageId`、`parseMode`、`textStyle`，入站有 `attachments[]` 与按需 `downloadAttachment`（`src/channels/types.ts:388-434,236-274`）。
- dsh-im：模型调用 `dsh_im_return_file` **登记**文件，返回"已排队，未发送"，回合结束由渠道上传（`src/channels/shared/semantic/artifact.mjs:643-741`）；投递结果按产物 `sent|rejected|failed|unknown`。

**Agena 现状**：`ExternalEvent.text` 必须非空且 ≤24 KiB，`data` ≤8 KiB（`crates/agena-assistant/src/integrations.rs:87-100`）——**纯图片消息会被直接拒绝**；`ChannelDelivery` 只有 `text`。更关键的是 `event.data` 在 channel 模式**根本不进模型**（`crates/agena-runtime/src/runtime/assistant/integrations.rs:661` 只拼 `event.event.text`），`submit_managed_message` 也只接受纯文本（`crates/agena-runtime-session/src/session/manager/managed.rs:280-306`）。

但底层能力已有：`AttachmentItem/AttachmentKind` 支持图片/音频/PDF 及尺寸/时长/页数（`crates/agena-domain/src/attachment.rs:151-172`），`ComposerNode::Activity` 能承载附件，`fs.write` 已有 `expected_sha256` 乐观并发。**缺的是协议接线，不是存储模型。**

**建议**：`ExternalEvent` 加有界 `attachments[{kind,name,mime,size,bytes|url}]`；`ChannelDelivery` 加产物引用（`{artifact_id,name,media_type,size}`）+ 每产物回执；宿主侧用现有 `private_paths` 目录做落盘，**必须**采用 nanoclaw 的 dirfd/独占创建防 symlink 手法（Agena 现在是 open 后验 metadata，`crates/agena-tool/src/file_io.rs:12-32`，仍有 TOCTOU 窗口）。

**成本**：去程 10–15 天，回程 5–8 天。

---

## 4. 聊天内审批与提问

**竞品做法**

- dsh-im：把审批卡片渲染到聊天里，用户**精确回复"批准"/"拒绝"**（或点按钮）即可；决策绑定原会话 `sessionId + approvalId + actor`，非发起人回复得到 `gone`，超时自动拒绝；卡片状态区分 `SUBMITTED/BUSY/RETRYABLE/GONE`，传输失败时把键盘交还（`src/channels/shared/harness-approval.mjs:38-46,218,435-444`）。提问支持编号选项 + 文字回退 + 群聊要求 @（`harness-question.mjs:11-60`）。
- nanoclaw：`ask_question` → 原生卡片（`Card/Actions/Button`），回调编码 `ncq:<questionId>:<index>` 以适配 Telegram 64 字节限制；点击后**先持久化决策再改卡片**，授权逻辑留在核心、渲染注册表只管展示（`src/channels/ask-question.ts:47-81`、`chat-sdk-bridge.ts:829-878,1002-1050`、`question-render-registry.ts:13-59`）。
- AstrBot：可移植 `Button/ActionRow`（带 `fallback_text`、`action_timeout`），回调令牌持久化在 SQLite，重启后点击仍能解析或明确告知过期（`astrbot/core/message/components.py:153-260`、`platform/button_interaction.py:20-45`）。

**Agena 现状**：明确禁止——渠道里只发"请打开 Agena 会话 #{session} 查看"，并写明"权限不由外部消息文本批准"（`crates/agena-runtime/src/runtime/assistant/integrations.rs:836-842`、`docs/integrations.md:230`）。`ask_user` 返回 `UserInputRequired`（`crates/agena-runtime-tools/src/tool/ask_user.rs:11-14`），渠道侧没有回传通路。**每个审批都把用户踢出聊天。**

值得注意的是 Agena 的数据模型其实更完整：`AskUserToolInput` 支持 1–3 个问题、自定义选项、`auto_resolution_ms`（60s–10min）（`crates/agena-runtime-contracts/src/part/tool.rs:778-798`）；`InteractionView` 已有 `decided_at_ms` 与 `InteractionDecision`。

**建议**：不是"加按钮"，而是补三件事——(a) 入站类型化的 `action` 事件（必须由适配器验证，信任级别等同 `actor_verified`）；(b) 出站 `ask_question/card` 内容变体 + 文本回退；(c) 决策幂等认领 + 迟到/重复点击处理。安全上可照抄 dsh-im 的 `actor` 绑定和 nanoclaw 的"先持久化再改卡片"。

**成本**：10–15 天（依赖第 1 项的能力声明）。

---

## 5. 编辑 / 反应 / 终态投递

**竞品做法**：nanoclaw 把投递定义为操作联合而非纯文本——`{operation:'edit', messageId, text}`、`{operation:'edit', terminalCard}`、`{operation:'reaction', messageId, emoji}`；审批过期时**编辑原卡片**而不是再发一条（`src/channels/chat-sdk-bridge.ts:963-1000`、`src/gateway-approval-coordinator.ts:664-685`）。letta-code 有 `sendRichMessageDraft` 做流式草稿，最终仍发一条持久消息（`src/channels/types.ts:269-274`）。

**Agena 现状**：`ChannelReceipt.remote_id` 有记录但**从未使用**——`IntegrationDelivery` 没有 operation 字段，`ChannelDelivery` 只有 `text`。结果：审批卡片无法标记已解决，长任务无法"先发已开始再更新"，旧卡片永远看起来可点。

**建议**：`ChannelDelivery` 加 `operation`（send/edit/react/delete），保存 `remote_id`（列已存在），适配器新方法放能力开关后。

**成本**：4–6 天（依赖第 4 项）。

---

## 6. 长回复处理：分片而不是截断

**竞品做法**：CowAgent 的 `split_markdown` 在换行处切分，并在下一段**重新打开代码围栏（含 info string）**（`CowAgent/common/markdown_fence.py:69-101`）；Telegram 发送按 4000 切分，HTML 解析失败时降级为纯文本重发（`CowAgent/channel/telegram/telegram_channel.py:659-690`）。dsh-im 有 4500 字符的围栏感知切分 + 代理对安全切片（`src/channels/shared/markdown-chunks.mjs:2-110`）。nanoclaw 用适配器声明的 `maxTextLength`（Discord 2000 / Telegram 4096），并在**转义之后**度量长度（`src/channels/chat-sdk-bridge.ts:396-404,465-532`）。

**Agena 现状**：超过 24000 字节**直接截断**并追加"[full result remains in Agena]"（`crates/agena-runtime/src/runtime/assistant/integrations.rs:19-29,804`）；出站存储层也硬性拒绝 >24576 字节（`crates/agena-storage-sqlite/src/integration_repository.rs:363`）。长回答被腰斩，用户还得去别处看。

**建议**：宿主提供参考分片器（段落→行→词→硬切，围栏感知，代理对安全），配合第 1 项的 `max_text_bytes`。**注意在转义/标记之后度量。**

**成本**：2–4 天（纯文本）。

---

## 7. 触发策略与群上下文

**竞品做法**

- dsh-im：触发是纯函数 `requireMention + freeResponse + replyMode ∈ {never, bot-only, any}`，`bot-only` 要求**可证明的回复对象是机器人**，无法判定时按未触发处理（`src/channels/shared/conversation/trigger-policy.mjs:19-79`）。
- nanoclaw：`engage_mode ∈ {pattern, mention, mention-sticky}`；未触发但 `ignored_message_policy='accumulate'` 的消息仍存入会话上下文，等真正触发时可用（`src/types.ts:134-152`、`src/router.ts:346-372`）。
- dsh-im：群历史环形缓冲按保留天数落 SQLite，在授权门之前记录，注入时作为独立 `<dsh_im_context>` 块、按事件 id 去重、用户原文逐字保留（`src/channels/shared/group-history.mjs:41-56`、`conversation/context-builder.mjs:1-148`）。

**Agena 现状**：`ChannelTrigger` 只有 `DirectOrMention | EveryMessage` 两值（`crates/agena-domain/src/channel_access.rs:96-100`），策略细节只能塞进 `addressed_to_bot` 布尔，宿主无从审计。未触发消息标 `ignored` 丢弃（`crates/agena-storage-sqlite/src/integration_repository.rs:278-281`）——**机器人不知道群里之前聊过什么**。

**建议**：(a) 扩展触发策略为有界结构（mention/free-response/reply-to-bot）；(b) 增加"未触发也入库、但只作为上下文不进模型"的 accumulate 语义；(c) 群历史作为**数据**注入，明确不授予工具权限。

**成本**：触发 3–4 天；群历史 5–7 天。

---

## 8. 消息突发合并与出站节流

**竞品做法**：LangBot 按作用域合并连续消息，1.0–10s 延迟、最多缓存 10 条、合并成一次查询（`src/langbot/pkg/pipeline/aggregator.py:23,245`）；letta-code 有按 key 的尾沿防抖（`src/channels/inbound-debounce.ts:1-60`）。dsh-im 给 Telegram 新消息设 60 秒/群预算，429 重试至多 4 次、超过 60 秒的直接上报，而**编辑/草稿/typing/反应不占预算**（`src/channels/telegram/telegram-api.mjs:10-16,496-535`）；AstrBot 有固定窗口限流（`astrbot/core/pipeline/rate_limit_check/stage.py:15-90`）。

**Agena 现状**：入站只有 `poll_seconds`，出站**没有任何节奏控制**；`ChannelReceipt::Retryable` 的 `retry_after` 没有和每群预算挂钩。用户连发 3–5 条短消息 = 3–5 次独立执行，既贵又碎。

**建议**：按 `(binding, thread, actor)` 做尾沿防抖 + 硬上限；出站按 `(binding, resource)` 做 60 秒窗口预算，编辑/反应不计入。

**成本**：防抖 4–6 天；节流 3–5 天。

---

## 9. 聊天内控制面

**竞品做法**：letta-code 有共享的斜杠命令面 `/help /status /whoami /pause /resume /cancel /new`，命令语义在公共模块、各宿主注入执行器（`src/channels/command-surface.ts:135-170,368-379`）；dsh-im 有 `/new /go /prev /history /compact /reset /stop`（`src/channels/shared/session-nav-command.mjs`、`control-command.mjs:5-104`）；CowAgent 的 `/cancel`、`/steer` 走**绕过会话队列的快速路径**，避免"取消排在要取消的任务后面"（`CowAgent/channel/chat_channel.py:537-592`）。

**Agena 现状**：渠道内只有 `/access` 一套（`crates/agena-runtime/src/runtime/assistant/channel_access.rs:243-297`）。用户无法在聊天里停止执行、开新会话、查看状态。**已有基础设施**：`/access` 已经享受队列优先级（`crates/agena-storage-sqlite/src/integration_repository.rs:268` 的 `CASE WHEN`），扩展到其他命令成本很低。

**建议**：加 `/stop`、`/new`、`/status`、`/help`；`/stop` 必须走快速路径，不能排队。

**成本**：5–8 天。

---

## 10. 未知发送者升级与主动私信

**竞品做法**

- nanoclaw：未接线群里的被 @ 消息不会静默丢弃，而是给 owner 发卡片"连接到已有 agent / 新建 agent / 拒绝"，批准后**重放原事件**；`decline_notify` 是礼貌拒绝 + 一行 owner 通知；并发升级按主键去重（`src/modules/permissions/channel-approval.ts:1-100`、`src/modules/permissions/db/pending-sender-approvals.ts`）。它的 guard 是三态 `ALLOW/DENY/HOLD`，且**grant 只能满足 HOLD，永不满足 DENY**，重放时结构性检查实时重跑（`src/guard/types.ts:20-40`）。
- nanoclaw/letta-code 都有 `openDM(handle)`：把用户解析成私聊通道，区分"handle 就是私聊 ID"（Telegram/WhatsApp）与"需要平台开私聊"（Discord/Slack/Teams）（`nanoclaw/src/channels/adapter.ts:283-298`、`letta-code/src/channels/pairing.ts`）。
- letta-code 的配对流程：未知用户拿到 15 分钟有效的配对码，用 `/channels <ch> pair <code>` 批准（`letta-code/src/channels/pairing.ts:20-40`）。

**Agena 现状**：`ChannelAccessPolicy.validate` 要求 1–128 个显式成员且至少一个未过期 owner（`crates/agena-domain/src/channel_access.rs:261-280`）；`allow_unlisted_chat` 只给"仅聊天"的 member。全新群即使 owner 在场，没枚举就 fail-closed，**且无法从聊天里自救**。宿主只能回复已有事件，**没有主动私信能力**——审批和"你被移除"的通知送不到从未发过消息的用户。

**建议**：(a) 加 `unknown_sender` 策略枚举（`public|request_approval|decline_notify|strict`）+ 待批表 + 重放；(b) 加 `open_dm` 能力与 `(actor, channel)` 缓存。

**成本**：升级 6–8 天；openDM 5–7 天。

---

## 11. 操作者控制面隔离

**竞品做法**：errbot 的安全默认值——**管理员命令默认只能在私聊使用**，群聊里必须显式开 `allowmuc`；ACL 是按命令的 glob 矩阵（用户/房间/私聊/群/参数六个维度），且"拒绝"后检查所以优先（`errbot/errbot/core_plugins/acls.py:41-152`）。dsh-im 把 `/clear` 等命令做宿主侧闸门，未授权直接回"Permission denied"（`nanoclaw/src/command-gate.ts:1-45`）。

**Agena 现状**：`/access`（含 `members/set/revoke`）**在任何群聊里都能执行**，只是文档"建议"敏感操作去本地面板（`docs/channel-access.md:114`）。多人群里执行 `/access members` 会把成员列表公开。

**建议**：管理员命令默认仅私聊；群聊需显式策略开启。**改动极小、收益明确。**

**成本**：1–2 天。

---

## 11b. 操作者向渠道注入消息、typing 指示

**竞品做法**：nanoclaw 的 CLI 管理传输支持 `{text, to, reply_to}`——操作者把消息投进某个渠道，回复重定向回自己的终端；`replyTo` 是**路由层概念，agent 不能设置**（`nanoclaw/src/channels/cli.ts:211-260`、`src/channels/adapter.ts:39-77`）。typing 指示按 4 秒刷新但仅在心跳文件新鲜时发送（`src/modules/typing/index.ts:1-48`）；AstrBot 有 `send_typing/stop_typing`；LangBot 有按适配器的 `processing_indicator`。

**Agena 现状**：宿主只能回复已有事件，**没有操作者向渠道注入消息的通道**（Web/TUI 控制面与渠道是断开的）；也没有 typing 指示，用户发完消息后界面长时间无反馈。

**建议**：`ChannelOutputRoute` 已经能携带目标，加一个"操作者注入"入口即可；typing 放在能力声明后。

**成本**：注入 3–4 天；typing 1–2 天。

---

## 12. 权限模式与建议规则

**竞品做法**：AgentScope 五模式 `DEFAULT/ACCEPT_EDITS/EXPLORE/BYPASS/DONT_ASK`，每个 ASK 附带机器生成的 `suggested_rules`，用户可一键升级为持久规则；`DONT_ASK` 把所有 ASK 转成 DENY，专为无人值守（`src/agentscope/permission/_types.py:22-80`、`_engine.py:161-191,570`）。

**Agena 现状**：`PermissionMode::{Allow,Auto,Ask,Deny}`（`crates/agena-domain/src/permission.rs:7-12`），classifier 失败时 **fail-closed 退回交互询问**（`crates/agena-permission/src/pipeline.rs:28-54`）——在渠道这种无人值守场景，这等于挂起到超时被取消（`crates/agena-runtime/src/runtime/assistant/integrations.rs:747-760`）。没有"建议规则"捷径。

**建议**：加 `DONT_ASK` 语义（ASK→DENY）；ASK 卡片附带建议规则，一键落到现有规则存储。

**成本**：3–5 天。classifier 和拒绝预算（`crates/agena-permission/src/auto/budget.rs`）已经比竞品强。

---

## 13. 插件运行时韧性

**竞品做法**：MaiBot 有按插件熔断（3 次失败 → 开 60s，指数退避至 300s，半开探测）、按组件 RPC 超时元数据、能力作用域的权限令牌、`send|receive|duplex` 路由类型（`src/plugin_runtime/host/circuit_breaker.py:14-80`、`component_timeout.py`、`authorization.py:14-67`）。

**Agena 现状**：`integration_fault` 已有指数退避（30s × 2^n，上限 1h）（`crates/agena-runtime/src/runtime/assistant/integrations.rs:489-492`），服务调用有超时（describe 3s / poll·send 20s）（`crates/agena-plugin-host/src/host/plugin_host_core.rs:213`），但**没有熔断**，也没有抖动——多个绑定同时失败会同步重试。

**建议**：退避加抖动（几行）；加简单熔断。**很便宜。**

**成本**：1–3 天。

---

## 14. 凭据不落地

**竞品做法**：nanoclaw 让 agent **从不持有原始 API key**，外发请求经凭据网关在请求时注入，并执行按 agent 的策略与限流（`nanoclaw/README.md:97`）。

**Agena 现状**：渠道适配器的凭据通过 `env` 直接给插件进程（`docs/integrations.md:65,129`），插件持有明文；插件本身是"使用者安装的受信任代码"（`docs/integrations.md:214`）。文档已明说这不是沙箱。

**建议**：长期做凭据网关（高成本）；短期至少让 `x-agena-secret-reference`（`crates/agena-plugin-sdk/src/settings_contract.rs:22`）走统一的 secret store 注入，而不是裸 env。另外可借鉴 dify 的 API key 资源作用域：key 可绑定到 N 个具体资源，**每次请求实时查作用域而不缓存**，越界的端点直接 403（`dify/api/controllers/service_api/wraps.py:370-395`）。Agena 的 `FileSecretStore` 目前只有 `namespace`（`crates/agena-secret-store/src/lib.rs:75-85`），没有资源级绑定。

**成本**：短期 3–5 天；完整网关数月。

---

## 15. 版本化配置与成本额度

**竞品做法**：dify 每个应用只有一个 `draft`，发布后是单调递增的 `version_number`（`dify/api/models/workflow.py:184-229`）；n8n 有独立 `workflow_history` 表和 `published-version` 概念；LangBot/AstrBot 的 manifest 带 i18n、图标、分类、帮助链接，WebUI 据此**生成表单**（`LangBot/manifest.yaml:1-46`、`astrbot/core/platform/register.py:11-60`）。

**Agena 现状**：(a) 权限策略有 `before/after` 审计（`crates/agena-assistant/src/channel_access.rs:22-23`），但**适配器配置没有历史**；(b) 额度只按事件次数与执行秒数（`crates/agena-assistant/src/integrations.rs:188-190`），**不按 token/成本**——尽管 `SessionCostSummary` 和 `/api/v1/sessions/{id}/cost` 都已存在（`crates/agena-domain/src/session_cost.rs:55`、`crates/agena-api-server/src/lib.rs:520`）；(c) 集成页是**纯 JSON textarea**（`packages/agena-web/src/pages/IntegrationsPage.vue:328`），`config_schema` 只用来生成模板，没渲染成表单（`packages/agena-web/src/features/integrations/model.ts:5`）。

**建议**：(a) 绑定配置存版本快照，支持回滚；(b) 渠道额度接 `SessionCostSummary`，按 binding/actor 聚合；(c) 用已有 `config_schema` + `settings_contract_from_schema` 生成表单，schema 驱动。

**成本**：(a) 5–7 天；(b) 3–5 天；(c) 5–8 天。

---

## 16. 吞吐：轮询与派发的串行瓶颈

**Agena 现状**：每 1 秒 tick 一次，**每 tick 最多轮询 4 个绑定**（`crates/agena-runtime/src/runtime/assistant/integrations.rs:513-515`），且**每个绑定每 tick 只派发 1 个事件**（`:571`）、只发 1 条出站（`:872`）。64 个绑定满配时，每个绑定平均 16 秒才轮询一次；一个群 10 个人同时提问要 10 秒才能全部派发。

**竞品**：nanoclaw 的出站有 in-flight 去重、overdue 停泊、有界并发（`src/delivery.ts:118-150`）；dsh-im 有按会话的 promise 链，保证单会话顺序但**不阻塞其他会话**（`src/channels/shared/conversation-queue.mjs:1-40`）。

**建议**：派发与出站改为每 tick 小批量（如每绑定 8 条）+ 全局并发上限；轮询配额改为按"到期绑定数"动态分配。

**成本**：3–5 天。

---

## 17. 子代理可验证性（跨领域，但渠道自动化会用到）

**竞品做法**（deer-flow）：每次工具调用生成回执 `r1..rN` 并写入消息元数据；父代理核对子报告里的 `[rN]` 引用（返回 `citation_resolved` 而非"已满足"）；验收条件用代码执行（`file:<p> exists`、`tests_passed:<cmd>` 且必须对应**某次真实记录的 bash 执行**，`echo "pytest"` 骗不过）（`deer-flow/backend/packages/harness/deerflow/agents/middlewares/tool_receipt.py:126`、`subagents/acceptance_checks.py:2155`）。

**Agena 现状**：子代理的自述是父代理唯一的证据来源。

**建议**：先做回执戳记 + `tests_passed` 锚定；成本中等但价值高。

**成本**：2–3 周。

---

## 18. 事件词汇表：AG-UI 协议

**竞品做法**（CopilotKit，MIT）：AG-UI 把流式事件标准化为 SSE 词汇——run 生命周期（`RUN_STARTED/FINISHED/ERROR`）、文本流（`TEXT_MESSAGE_START/CONTENT/END`）、工具调用（`TOOL_CALL_START/ARGS/END/RESULT`）、共享状态（`STATE_SNAPSHOT/STATE_DELTA`）、富表面（`ACTIVITY_SNAPSHOT`）、自定义中断（`on_interrupt`）。渠道适配器消费同一套事件，因此"写一次、多平台复用"。身份**不放在 `Message.author`**（provider 会校验/清洗），而走 `forwardedProps.channelActor`（`CopilotKit/packages/channels-core/src/run-loop.ts:1-62,192-198`、`identity.ts:43-95`）。

**Agena 现状**：实时流是自己定义的 `Notification` 枚举走 SSE（`crates/agena-api-server/src/sse.rs:1-60`、`crates/agena-api/src/notifications.rs`）。注意两者**模型不同**：agena 的 `Notification` 是**失效通知**（"某个会话变了，带 revisions，请重读"），配合 `Lagged` 让慢客户端重新同步——这对可靠性和断线重连更好；AG-UI 是**内容流**（"这段文本增量是…"），对渠道推送更直接。所以不是简单替换，而是补一层"从已提交状态派生内容增量"的投影。

**建议**：内部把现有 `Notification` 映射到 AG-UI 词汇（`TEXT_MESSAGE_*`/`TOOL_CALL_*`/`STATE_*`），运行请求加 `channelActor`。**按公开规范重新实现，不要抄代码**（AG-UI 是公开协议，MIT）。

**成本**：中等，5–8 天。

---

## 19. 不透明动作 ID 与派发作用域校验

**竞品做法**（CopilotKit）：每个渲染出来的按钮由服务端铸造不透明 id（`ck:…`），绑定 `{channel, conversationKey, platform, threadId, runChainId, initiator}` 并持久化为快照（7 天保留）。点击派发时**从入口事件自身的范围重新推导快照，绝不信任点击载荷**；跨会话/跨平台的展示直接拒绝（`ActionScopeMismatchError`/`ActionContinuationMismatchError`/`ActionExpiredError`）。`awaitChoice` 每会话只允许一个待决选择，默认只有发起人可答（`answerableBy: "initiator"`），支持超时/中止/取代（`CopilotKit/packages/channels-core/src/action-registry.ts:26-83`、`action-store.ts:15-45`、`thread.ts:807-860`）。

**Agena 现状**：`ChannelAuthority` 已经有 `binding_id/epoch/actor/resource/thread`（`crates/agena-domain/src/channel_access.rs:386-401`），`ChannelOutputRoute` 也能携带目标——**缺的只是"按钮 → 谁点、属于哪个会话、重启后怎么办"这一层绑定**。

**建议**：加 `(binding, conversation, nonce)` 动作表 + 派发时对照入口事件范围校验 + TTL 清理。与第 4 项（聊天内审批）配套。

**成本**：小到中等，3–5 天。

---

## 20. 平台中立的消息 IR

**竞品做法**：botpress 定义封闭消息词汇——`text/image/audio/video/file/location/carousel/card/choice/dropdown/bloc`，全部 zod schema（`botpress/packages/sdk/src/message.ts:9-119`）；各平台映射到原生结构（Slack 的 Block Kit section/actions、`choice`→按钮、`dropdown`→`static_select`、`carousel`→扁平化 blocks）。CopilotKit 用 `ChannelNode` IR + 原生逃生口 + `SurfaceCapabilities`（声明 `supportsStreaming`、`supportsModals`、`maxBlocksPerMessage`）（`CopilotKit/packages/channels-ui/src/ir.ts:1-8`、`channels-core/src/platform-adapter.ts:25-45`）。

**Agena 现状**：渠道契约只有 `text: String`。但 agena **已有**丰富的 inline-UI 块词汇（`card`、`table`、`chart`、滑块、动作，见 `docs/inline-ui-runtime.md`），只是没有投影到渠道。

**建议**：先做 `card/choice/image/file/divider` 五类，配合第 1 项的能力协商。**这是"渠道里能放富内容"的具体使能器**，但适配器工作量占大头。

**成本**：五类中小；全量十一类很大。

---

## 21. 通用审计日志（哈希链）

**竞品做法**（LibreChat，MIT）：按租户的链密钥、单调 `seq`、对规范化记录计算 `prevHash`/`hash`、创世哨兵、`{chainKey,seq}` 唯一索引、重复键带抖动重试、`verifyAuditChain` 校验、有界 CSV 导出，记录 actor/target/outcome/severity/请求上下文（IP、UA、session）与 schema 版本（`LibreChat/packages/data-schemas/src/methods/auditLog.ts:170-205,337-370`、`schema/auditLog.ts:88-180`）。

**Agena 现状**：审计**范围很窄**——只有 `agena_channel_access_audit` 覆盖策略写入和平台撤权（`crates/agena-storage-sqlite/src/integration_repository/channel_access.rs:7`、`crates/agena-assistant/src/channel_access.rs:14-24`）。Bot 改名、provider key 变更、插件安装、权限修改**都没有审计**，也没有通用审计表。

**建议**：建一张共享审计表，按 `{scope chain key}` 做 sha256 链，暴露 `/api/v1/audit`（游标分页）+ 校验端点。渠道审计顺带变得防篡改。

**成本**：小到中等，4–6 天。

---

## 22. 策略历史、回滚与版本化

**竞品做法**：n8n 每次保存写不可变 `WorkflowHistory`，加 `WorkflowPublishedVersion` 指针和发布历史；`assertCanPublish` 在有待审时阻止发布。dify 每个应用一个 `draft` + 单调递增 `version_number`。FastGPT 的 `app_versions` 带 `isPublish`/`isAutoSave`/`versionName`。Mattermost 的 ABAC 策略是**带版本号**的文档（`v0.1`…`v0.5`），且允许插件以自己的命名空间拥有策略。

**Agena 现状**：渠道策略已有 `before/after` 审计快照（`crates/agena-assistant/src/channel_access.rs:22-23`），但**没有列表/比较/回滚 API**；`channel_audit` 只读最近 100 行（`crates/agena-storage-sqlite/src/integration_repository/channel_access.rs:79-80`）。适配器配置完全没有历史。`ChannelAccessPolicy` 也没有 schema 版本字段，旧配置的兼容性靠 serde 默认值隐式处理。

**建议**：(a) 策略历史加列表/比较/回滚端点 + UI diff；(b) 给策略加显式 `schema_version`；(c) 适配器配置存快照。

**成本**：(a) 4–6 天；(b) 1–2 天；(c) 5–7 天。

---

## 23. 凭据信封与轮换

**竞品做法**（dify）：IM 凭据用严格的 per-provider Pydantic 模型解析（`extra="forbid"`、`frozen`、`strict`、`hide_input_in_errors`、密钥字段 `repr=False`），再由**租户绑定的** `BoundCredentialCipher` 封装成带版本的 `ChannelEncryptedCredentials` 信封；支持 RSA 与 Azure Key Vault 两种 key provider。n8n 用 AES-256-GCM（HKDF info `n8n-encryption-v1`）加密凭据。

**Agena 现状**：`FileSecretStore` 有 AEAD 和 namespace（`crates/agena-secret-store/src/lib.rs:75-200`），插件设置区分 `sensitive` 与 `secret`。缺的是：**密钥版本化与轮换**、**API 响应里的按 binding 脱敏投影**。

**建议**：secret store 加 `{version, key_id, nonce, ciphertext}` + 轮换命令；硬性规则：GET 只返回脱敏投影。

**成本**：小，3–5 天。

---

## 24. 速率限制的分层与标准响应头

**竞品做法**：n8n 按 IP → 用户 → body key 分层，另加按 API key 的限流（既限单端点也限全端点），输出 `X-RateLimit-Limit/Remaining/Reset`。FastGPT 在渠道路径里按 outLink QPM + team QPM 限流，且**放在去重之后**，避免平台重试重复计数。dify 有 Redis 支撑的按应用**最大活跃请求数**并发限制 + 预留/提交/退还的配额生命周期。

**Agena 现状**：`per_user_daily_limit`（1–1000）+ 按 UTC 日的每用户预算（`crates/agena-domain/src/channel_access.rs:238-247`、`crates/agena-storage-sqlite/src/integration_repository/channel_access.rs:171-175`）——**在"聊天渠道用户额度"这一项上比所有竞品都强**。缺的是：带标准响应头的 IP/登录节流、按 token 的 RPM、并发上限。

**建议**：加标准响应头 + 并发上限；保留现有的每用户日额度。

**成本**：小（响应头）到中等（持久化并发计数）。

---

## 25. 平台验签作为共享库

**竞品做法**：dify 的 Slack 用 `slack_sdk.SignatureVerifier`（单时间戳 + 常数时间比较），飞书用 `HMAC-SHA256(timestamp + nonce + encrypt_key + raw_body)` + base64/AES-CBC 解 `encrypt` 信封，并有 URL 校验例外（`dify/api/core/human_input_v2/im_integration/adapters/slack.py:925-936`、`feishu_lark.py:1493-1527`）。botpress 的 Slack 用 `v0:timestamp:body` + 5 分钟窗口 + `timingSafeEqual`，微信用排序拼接 SHA1。

**Agena 现状**：宿主级 HMAC（`X-Agena-Timestamp` + `X-Agena-Signature`，5 分钟窗口）做得扎实，但**每个平台的具体验签完全在各插件里重复实现**。

**建议**：在 plugin SDK 里提供共享验签模块（Slack/飞书/微信/Teams 变体），配单元测试和官方测试向量。纯函数，成本很低。

**成本**：2–3 天。

---

## 26. 平台原生适配器覆盖

**竞品做法**（dify 是中文 IM 的标杆）：Slack、飞书/Lark、钉钉、企业微信、MS Teams 的生产级适配器，包含严格的凭据模型、**目录能力**（读用户/头像，失败则整体失败不留半条）、**个人文本消息**、**动态卡片**（`assess` → `send_card` → `replace_with_static`，带关联令牌和定位器，保证卡片只更新一次）、卡片事件解码、区分 URL 校验与事件投递的 webhook 处理器。飞书单文件约 2000 行，含 AES 解密。

**Agena 现状**：只有 GitHub 轮询和通用 webhook 聊天桥两个参考适配器。插件模型更干净（宿主管授权、适配器只做标准化），但**没有真实的 Slack/飞书/Telegram 适配器**，模型再好也难以验证。

**建议**：参数化能力接口（`IMDirectory`/`IMMessaging`/`IMDynamicCardMessaging`/`IMCardEventDecoder`/`IMWebhookHandler`/`IMMessagingStream`）值得在 `agena-plugin-sdk` 里镜像；先做一个 Telegram + 一个飞书参考实现。

**成本**：每个平台都不小，但参考实现能验证整条设计。

---

**Agena 现状**：子代理的自述是父代理唯一的证据来源。

**建议**：先做回执戳记 + `tests_passed` 锚定；成本中等但价值高。

**成本**：2–3 周。

---

## 建议的落地顺序

**第一批（低成本、立刻生效）**
1. **`/access` 仅私聊**（第 11 项）——一天的安全改进。
2. **退避加抖动 + 熔断**（第 13 项）——几小时。
3. **分片代替截断**（第 6 项）——2–4 天。
4. **平台验签共享库**（第 25 项）——2–3 天。

**第二批（结构性瓶颈）**
5. **能力声明**（第 1 项）——一次性解锁后续所有富能力。
6. **多资源绑定**（第 2 项）+ **吞吐修复**（第 16 项）。
7. **出站节流**（第 8 项下半）+ **突发合并**（第 8 项上半）。

**第三批（聊天体验）**
8. **触发策略 + 群上下文**（第 7 项）。
9. **聊天内控制面 `/stop /new /status`**（第 9 项）。
10. **typing 指示 + 操作者注入**（第 11b 项）。

**第四批（最大改动）**
11. **富媒体**（第 3 项）+ **消息 IR**（第 20 项）。
12. **聊天内审批**（第 4/5 项）+ **动作 ID 作用域**（第 19 项）。

**第五批（运维与长期）**
13. **通用哈希链审计**（第 21 项）+ **策略历史/回滚**（第 22 项）。
14. **凭据信封与轮换**（第 23 项）。
15. **AG-UI 事件词汇**（第 18 项）。
16. 其余（未知发送者升级、成本额度、平台原生适配器、子代理回执）按需。

---

## 明确不要抄的

- **MaiBot 内存 TTL 去重**：重启会重放，Agena 的持久去重更好。
- **CowAgent 的每渠道重复业务逻辑**：每个 channel 重写 mention 解析和白名单，正是 Agena 用统一 ABI 避开的反模式。
- **AutoGen 的 `AgentTool`/`TeamTool`**：官方要求关闭并行工具调用（"agents cannot run concurrently as they maintain internal state"），弱于 Agena 的会话隔离子代理。
- **AstrBot 基类的 `send_streaming`**：基类是空实现，真正流式只在少数适配器；不要把它当作普遍能力。
- **coze-studio 的权限模型**：`SpaceUser.role_type` 存在但授权检查根本不看它——`checkReadPermission`/`checkWritePermission` 都退化成 `operatorID == CreatorID`（`coze-studio/backend/domain/permission/authz_checker.go:102-124`），且没有邀请流程。它的草稿/发布是真的，权限不是。
- **Rasa 的渠道层**：只是薄的 `InputChannel`/`OutputChannel` 子类，没有持久收件箱、没有幂等、没有卡片/动作模型。唯一可借鉴的是 `conversation_granularity: sender|channel|thread`，而 Agena 的 `ChannelSessionMode + ChannelThreadMode` 表达得更好。
- **FastGPT 的渠道凭据存储**：`app` 文档里直接存飞书 `appSecret`、钉钉 `clientSecret`、企微密钥明文，没有 AEAD。Agena 的 secret ref 更好。
- **dify 的 RBAC 实现**：`RBAC_ENABLED` 会短路整个检查，启用时又走外部企业服务（`dify/api/controllers/common/rbac/checks.py:36-43`、`services/enterprise/base.py:160-225`）。开源仓库里只有契约和装饰器，不是自洽的引擎。它的角色/权限**目录**还有参考价值。
- **n8n 的配额模型**：是商业许可驱动的（项目数、角色），不是用量。
- **测试文件数量**：LangBot ~792、CowAgent ~677、AstrBot ~264 个测试文件，多为平台集成夹具；Agena 自己的 `examples/integration-plugins/test_adapters.py`（验签/重放/分页/幂等回执）比多数竞品的渠道测试更有针对性。

---

## 许可证约束（抄代码前必看）

| 仓库 | 许可证 | 对 Agena 的约束 |
|---|---|---|
| dify | 修改版 Apache-2.0 | **未经书面授权不得用其源码运营多租户环境**；不得移除 LOGO。多租户模型只能当设计参考。 |
| FastGPT | 修改版 Apache-2.0 | **不得运营与 FastGPT 类似的多租户 SaaS**。同上。 |
| coze-studio | Apache-2.0（干净） | 可复用代码，但权限模型本身没价值。 |
| n8n | Sustainable Use License（非 OSI） | 仅限内部/非商业用途；`.ee.` 文件是企业版。模式可重实现，代码不可。 |
| Chatwoot | MIT + `enterprise/` 商业许可 | Pundit/inbox 是 MIT；审计日志和自定义角色是企业版。 |
| Rocket.Chat | MIT + `ee/`/`apps/meteor/ee/` | 授权核心和 Apps Engine 是 MIT；ABAC 和市场门控是 EE。 |
| Mattermost | AGPL-3.0 / 商业 | **抄设计，别抄代码**。AGPL 是真实约束。 |
| LibreChat | MIT | 完全可复用（租户隔离插件、哈希链审计）。 |
| lobehub | LobeHub Community License | 商业衍生分发需授权；schema 可作参考。 |
| CopilotKit | MIT | 完全可复用；AG-UI 是公开协议。 |
| botpress | MIT | 可复用（消息 IR 直接参考）。 |
| Rasa | Apache-2.0 | 可复用，但价值低。 |
