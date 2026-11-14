# 多用户聊天渠道：身份、角色、权限与隔离

聊天接入不再把一个允许名单等同于“这些人都可以使用实例操作者的助手”。每条消息同时经过平台身份验证、渠道范围匹配、本地角色查找、能力边界检查，再进入对应会话。

## 身份与场景

宿主使用以下信息区分安全范围：

```text
已安装的提供方 + 渠道绑定
  → 租户 / 服务器 tenant
  → 机器人账号 account
  → 会话类型 kind
  → 群、私聊或频道 resource
  → 线程 thread
  → 已验证的发送者 actor
```

房间形状只有三种：`direct` 私聊、`group` 多人房间（含 Slack MPIM、Discord 群私聊、可交互频道）、`broadcast` 单向广播（Telegram 频道、WhatsApp newsletter，Bot 可能没有发言权）。形状是**消息属性**，不是账号属性：一个账号同时服务多种房间，因此绑定用 `channel_policy.kinds` 声明允许的形状（留空表示全部）。线程是独立维度，由消息的 `thread` 携带，不计入形状。显示昵称、消息中的“我是管理员”和事件附带的任意 `role` 字段都不作为授权依据。平台管理员身份也不会自动得到 Agena 的实例管理权限。

`resource` 必须是稳定、精确的外部会话 ID。相同消息 ID、用户 ID 或频道名在不同租户/账号中可以存在，但不会共用收件记录、会话或回复路由。内部保留主体名如 `@service`、`@shared` 不能被外部发送者占用。

插件/平台中继必须先验证平台签名和真实身份，再向 Agena 提交签名后的身份信封。共享 HMAC 证明的是“来自这个受信任适配器”；它不能代替适配器对原平台用户的验证。

## 角色不是操作系统权限

渠道内有五种本地角色。角色仅作用于该绑定的租户、会话与线程策略，不跨群、跨频道或跨机器人账号。

| 角色 | 渠道成员管理 | 工具、后台工作、记忆 |
|---|---|---|
| `blocked` 禁用 | 无 | 消息不进入模型，待投递回复也会被拦截 |
| `member` 普通成员 | 只能查看自身身份 | 默认只能聊天；额外能力必须由实例侧明确授予 |
| `operator` 操作成员 | 不能管理别人 | 可以配置为允许某些工具或后台工作，仍受权限上限限制 |
| `administrator` 渠道管理员 | 可管理低于自己的成员 | 不自动获得文件、网络、插件或凭据权限 |
| `owner` 渠道所有者 | 可管理低于自己的所有角色 | 同样不等于实例管理员；能力仍来自明确的角色配置 |

角色之间没有隐含的工具权限继承。给 Operator 配置文件读取，不会自动把文件权限同时授予 Administrator 和 Owner；需要时分别配置。这样可以让群管理员负责成员管理，却不自动获得代码库或服务器访问权。

已有实例认证下的本地 Web/TUI 操作者是配置控制面，可以定义角色能力、转移所有者或恢复无所有者连接。本轮没有把实例控制面的一个管理员令牌转换成多租户 Web 登录系统；不要把该令牌发给外部群管理员。

## 普通聊天、独立上下文与共享上下文

默认 `session_mode: per_conversation`：**一个群聊就是一个会话，一个私聊对话就是一个会话，一个频道就是一个会话**，不同会话的上下文从不合并。`thread_mode: per_thread` 时，不同线程再各自独立；`exact` 时只接受规则里指定的精确线程。同一人在不同平台（例如 Telegram 私聊与飞书私聊）始终是两个会话。

需要"同一会话内每个人各有一份上下文"时，显式选择 `session_mode: per_actor`。

私聊必须使用明确参与者和 `exact`，不允许“任意平台成员可聊”。群和频道默认 `trigger: direct_or_mention`，仅处理私聊或适配器已验证的点名/回复机器人消息；配置成 `every_message` 才处理该范围内每条消息。

**能力属于绑定，不属于角色。** 一个会话的上下文由所有发言者共享，因此 `channel_policy.grant` 是整条绑定唯一的一份能力集合；给不同角色配不同工具，会让低权限成员的消息驱动一个其输出被全群读到的高权限工具。角色仍然决定成员管理权限（`member`/`operator`/`administrator`/`owner`），但不再决定工具。

上下文隔离不改变聊天平台的可见性。回复到群或公开频道，平台中的其他成员仍然可能看见。私聊内容不会自动带入群聊；要让敏感内容只对个人可见，应使用其独立私聊绑定，而不是把“每人独立模型上下文”误当作群消息加密。

## 独立能力授权

绑定的 `channel_policy.grant` 使用 `ChannelGrant`，支持：

| 字段 | 含义 |
|---|---|
| `tools` | 精确的规范工具 ID，例如 `agena.fs.read`；不接受通配符或别名 |
| `mcp_tools` | MCP 服务器到具体远端工具名的白名单；不能直接授权通用网关或连接管理 |
| `private_memory` | 是否允许使用该主体隐私范围中的记忆和助手偏好工具 |
| `source_workspace` | 是否允许工具以明确授权的来源项目为工作目录；默认只用私有范围目录 |
| `network_hosts` | 精确网络主机/端口允许项；默认外网、私网和回环均拒绝 |

新绑定默认没有上述能力。即使显式添加某工具，仍要通过来源会话的权限配置和继承上限、渠道能力上限、路径/网络检查及原生审批；任何一层的拒绝都不能被另一层的 Allow 覆盖。

实例配置、凭据、插件安装、权限管理、任意会话访问、Cron 等管理工具不能直接放进绑定允许集。具名 Bot 的项目管理入口 `agena.bot.sessions` 支持显式授权：绑定关联有效 Bot，并在 `grant.tools` 中配置这个精确 ID 后，聊天入口可以管理普通项目会话。Bot 身份、目标会话规则、当前用户指令、写入幂等和版本检查仍由宿主执行。`agena.bot.connections` 继续使用独立的本地接入管理流程。

MCP 同样需要二次授权：例如 `"mcp_tools":{"docs":["search"]}` 只允许这个服务器的这个工具；工具发现结果也只包含获准的服务器和工具及其输入结构；同服务器其他工具、其他服务器和 MCP 连接配置管理仍会被拒绝。受范围约束的工作和记忆工具由能力开关推导，不让外部用户自行填写一个管理工具名绕过限制。

子代理和会话分叉 继承不可改写的渠道权限身份。工具准备之前、审批检查时及真正执行之前都会重新检查权限；发生撤权后，旧审批或缓存工具列表不能继续授权执行。成员只能继续自己有权访问的会话；原生执行和结果保持所属成员与线程，不能通过其他会话的 ID 获得权限。

旧策略若把 `agena.mcp.tools.call`、连接管理或其他原始 MCP 入口直接写入 `tools`，不会被自动解释为新授权；该能力范围会校验失败，需要实例操作者改为明确的 `mcp_tools` 后重新确认。

MCP 的本层授权限定服务器和工具名，不替远端工具推断每个业务参数的数据权限。应使用用途和资源受限的连接/凭据，或由 MCP 服务端继续验证租户和资源范围；不要把实例管理员的全权连接直接共享给普通成员。

工具插件本身是实例操作者安装的受信任代码。角色允许集和路径/网络权限不等于运行任意不可信插件的操作系统沙箱；不要把能够任意运行宿主命令的工具授予不可信群成员。

## 人格、记忆与数据目录

成员会话不复制实例操作者的原聊天历史和人格配置。默认建立独立的工作区身份、私有目录和记忆根；租户、账号、绑定、会话类型、线程和发送者共同决定隐私范围。

授予 `source_workspace` 只让获准工具访问指定项目，不把实例操作者的记忆库当作渠道用户的记忆。个人记忆仍使用该用户的私有范围。关闭 `private_memory` 后，不允许自动召回或模型写入私人记忆；实例侧显式数据管理仍走原认证控制面。

撤权会停止使用旧会话，不会声称已经删除既有历史、文件或记忆。历史保留用于审计和恢复，应按实例的数据保留策略另行清理。

## 自动报告使用独立服务主体

GitHub 自动执行的报告不借用 Owner 或某个普通成员的私有会话。需要在渠道策略中开启 `allow_automated_reports`，由单独的 `@service` 会话接收系统报告。

`service_grant` 明确控制这个服务主体可以做什么；它不能读取人的私有记忆。自动工作结果可以发布到你授权的渠道目标，但这不自动扩大任何成员的权限，也不能从通知文本里推导出新的授权。

## 管理成员与权限

### Web 和 TUI

Web `/integrations` 中的渠道连接新增“渠道权限 / 管理成员与能力”：可编辑成员角色和到期时间、上下文隔离、线程、触发方式、每人额度与角色能力，查看变更审计。

TUI `/integrations` 中选中连接后按 `a` 打开权限编辑器，修改类型化 JSON，使用 `Ctrl+Enter` 或 `Alt+Enter` 确认。`r` 刷新，`Esc` 放弃未提交修改。两端都使用同一版本号和请求键，网络回执丢失重试不会重复授权。

并发冲突不会自动把旧策略覆盖到新角色配置上。收到冲突后核对最新策略再明确提交。确定被服务端拒绝的输入保留给用户修正；结果不确定的请求保留原请求身份，防止重放产生另一项授权。

### 聊天内的确定性管理命令

```text
/access whoami
/access members
/access set 用户ID member
/access set 用户ID operator
/access set 用户ID administrator
/access revoke 用户ID
```

审计记录区分 `instance_operator` 本地控制面、`channel:用户ID` 渠道成员和 `platform:插件ID` 平台撤权，避免外部用户恰好使用同名 ID 时伪装成实例管理员。

这些命令由宿主直接处理，不经过语言模型决定是否授权。Owner 可以调整 Administrator 等更低角色；Administrator 不能修改自己、其他 Administrator 或 Owner，也不能把任何人提升为自己的同级或更高角色。普通 Member/Operator 无成员管理权限。

命令的回复仍发到原会话/线程；在多人群内执行成员列表命令会公开列出的信息，敏感管理建议使用本地管理面板。实例权限、工具能力和所有权交接不通过群内自由文本修改。

聊天或任务额度用完后，已授权成员仍能调用确定性的 `/access` 控制命令；不会因模型额度耗尽而阻止管理员撤权。

## 平台退群、撤销和成员到期

适配器应把平台已验证的移除/退群事件标准化为 `membership.revoked`，使用与消息相同的签名通道，设置 `actor_verified: true`、`is_member: false`。这个事件只能减少权限，不能凭平台元数据新增角色或提升权限。

宿主先处理同批次中的撤权，再处理普通消息。成员角色改为 Blocked，授权代次增加，变更写入审计；旧会话、执行中的模型、子代理、会话执行和待发送结果失效。平台重复推送使用同一事件身份，不会重复消耗预算或恢复被撤销权限。移除最后一位有效 Owner 时，连接进入需实例操作者处理的故障状态。

成员到期不依赖下一条消息：后台检查会取消过期主体的活动执行，新输入和待发送结果也重新检查成员状态。失效后的旧政策回执重试不会重建权限，也不能误停后续新授权会话。

实际平台成员状态由适配器负责验证与提供。宿主无法凭空获知尚未收到的外部退群事件；平台插件应及时订阅/同步成员移除，使用本地到期时间限制长期授权。在收到有效撤权后才可准确声称该外部变化已经在 Agena 生效。已经交给外部网关的发送请求无法撤回，尚未发送的旧结果会拦截。

## 配置示例

以下字段放入渠道绑定的 `channel_policy`，替换真实 ID 后核对。它适合“普通成员聊天、Runner 分析项目、管理员管理成员”的群场景：

```json
{
  "identity": {"tenant": "tenant-123", "account": "bot-456"},
  "kinds": ["group"],
  "session_mode": "per_conversation",
  "thread_mode": "per_thread",
  "trigger": "direct_or_mention",
  "members": [
    {"actor": "owner-user", "role": "owner", "expires_at_ms": null},
    {"actor": "group-admin", "role": "administrator", "expires_at_ms": null},
    {"actor": "trusted-runner", "role": "operator", "expires_at_ms": null},
    {"actor": "ordinary-user", "role": "member", "expires_at_ms": null}
  ],
  "allow_unlisted_chat": false,
  "grant": {
    "tools": ["agena.fs.read"],
    "mcp_tools": {},
    "private_memory": false,
    "source_workspace": true,
    "network_hosts": []
  },
  "per_user_daily_limit": 50,
  "allow_automated_reports": false,
  "service_grant": {
    "tools": [], "mcp_tools": {}, "private_memory": false,
    "source_workspace": false, "network_hosts": []
  }
}
```

渠道绑定的旧 `actors` 列表应为空，身份权限由 `channel_policy.members` 管理。旧连接缺少策略时拒绝收发和使用旧结果，不把过去允许名单自动升级成新的管理员授权。

对应消息必须有以下身份信封，签名覆盖整个消息 JSON：

```json
{
  "channel": {
    "identity": {"tenant": "tenant-123", "account": "bot-456"},
    "kind": "group",
    "actor_verified": true,
    "is_member": true,
    "addressed_to_bot": true,
    "verified_at_ms": 0
  }
}
```

`verified_at_ms` 应替换为实际验证时间。正常输入要求证据新鲜，不能长期重用旧成员证明。`membership.revoked` 使用相同身份信封，但 `is_member` 必须是 `false`，并使用稳定且与普通消息不冲突的事件 ID。

## 实现与验证入口

| 层次 | 位置 |
|---|---|
| 身份、场景、角色与能力策略 | `crates/agena-domain/src/channel_access.rs` |
| 入站检查、不可变权限身份和出站范围 | `crates/agena-assistant/src/channel_access.rs` |
| 事务策略修改、审计、额度、平台撤权 | `crates/agena-storage-sqlite/src/integration_repository/channel_access.rs` |
| 私有会话、实时授权验证、确定性管理命令 | `crates/agena-runtime/src/runtime/assistant/channel_access.rs` |
| 工具能力与多个权限上限取交集 | `agena-runtime-tools` 和 `agena-runtime-contracts` |
| Web 权限表单 | `packages/agena-web/src/features/integrations/ChannelAccessPanel.vue` |
| TUI 权限编辑器 | `crates/agena-tui-app/src/app_integrations.rs` |

```sh
cargo test -p agena-domain channel_access --lib --offline --locked
cargo test -p agena-storage-sqlite --test channel_access --offline --locked
cargo test -p agena --test system scenarios::channel_access --offline --locked \
  -- --test-threads=2 --nocapture
cargo test -p agena --test system channel_access_policy_editor_and_conflict \
  --offline --locked -- --ignored --test-threads=2 --nocapture
python3 examples/integration-plugins/test_adapters.py -v
```

系统用例使用真实 Agena、独立 stdio 适配插件、签名 Webhook、SQLite 和本地回复网关。模型为脚本化测试提供方，用于稳定触发越权调用等路径；权限判定由宿主执行，不依赖模型自觉拒绝。
