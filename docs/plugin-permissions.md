# 插件权限审计与执行约定

审计覆盖 `bundled_plugin_entries()` 注册的全部 23 个内置插件，以及插件 SDK、宿主回调、stdio/HTTP 传输和会话审批执行链。工具名权限检查统一由 ToolExecutor 执行；插件自己执行的文件和网络操作还必须申请实际资源权限。

## 查询与授权的区别

- `check_path_permission`、`check_network_permission` 只读查询当前会话策略和已保存规则，不调用审批模型、不创建交互，也不构成一次授权。
- `require_path_permission`、`require_network_permission` 在访问资源前执行统一授权：Allow 放行，Deny 返回策略拒绝，Auto 调用审批模型，无法确定时转 Ask，Ask 等待用户明确回复。
- 默认 SDK host 没有审批实现时，Ask 返回 HostUnavailable。不会把 Ask 伪装成 Deny，也不会默许执行。
- 运行时审批要求真实活动中的工具调用及其宿主签发的临时凭证。插件不能通过自行填写 session_id、call_id 或 workspace_root 获得其他会话权限。凭证与插件、调用、工作区和插件代次绑定，调用完成后失效。
- 同一工具调用并发提出的审批串行处理，避免争抢工具的交互槽。已批准的相同访问可在本次调用内复用；网络按主机与端口复用。每次仍先读取当前规则，明确 Deny 优先。AllowOnce 不跨工具调用复用。
- 审批等待暂停执行预算，不暂停取消信号。审批结束后继续计算实际工作耗时；并发等待只计一次。SDK、宿主工具调用、HTTP 传输、抓取和搜索协调器共享这一约定。

## 逐插件结果

| 插件 | 文件、网络或其他副作用 | 权限入口与本轮处理 |
| --- | --- | --- |
| `agena.fs` | 文件读取、目录搜索、文档转换、批量读取、写入和补丁 | read/glob/grep/read_media/apply_patch 由执行器检查；为 stat、document、document_stream、read_many、write、replace 补齐插件内授权。写入的读取、版本检查、创建目录和落盘均在批准后。glob/grep 在下降目录和访问具体匹配项时继续检查子路径规则。 |
| `agena.code` | AST 搜索、语法树、结构化替换 | 搜索与预览要求 Read；应用替换另要求 Write。目录搜索在下降目录和读取匹配语言的文件前逐项授权；拒绝后不进入解析和编辑。 |
| `agena.notebook` | 读取和重写 ipynb | 补齐 Read 和 Write，批准后才打开笔记本。版本校验和格式校验继续保留。 |
| `agena.settings` | 读取全局/工作区配置、合并验证、修改文件及请求重载 | 文件读取检查对应路径；合并验证检查两层 Read；实际编辑检查选中层 Write。dry_run 不申请不发生的写入。读取已加载的 effective 配置属于宿主配置能力。 |
| `agena.commands` | 发现技能/命令、读取资源、安装和移除工作区技能 | 请求时扫描授权现有配置根；资源单独授权具体路径。批准后使用同一个解析出的包，避免再次解析换成别的文件。移除先将别名解析成真实包名，再批准真实目标。安装/移除在创建目录前批准。热重载时获取当前代次的 host。请求按当前调用的工作区扫描；遍历的目录和实际读取的命令文档逐项授权，不使用初始化数据目录替代请求工作区。 |
| `agena.memory` | 工作区记忆文件与内部检索索引 | 显式读写按目录/记录路径申请权限；写入/删除同时授权实际连带修改的 MEMORY.md。目录检索在读每条记录前检查子路径，授权和实际访问均绑定当前调用的工作区。自动召回只查询已有权限，需要审批时跳过，避免后台 hook 发起交互。默认 `<state>` 规则允许当前实例的工作区记忆；用户限制该路径时仍生效。索引由宿主私有存储能力维护。 |
| `agena.web` | 搜索、HTTP 抓取、重定向、robots、缓存读取、浏览器操作、截图与下载 | 所有网络 require 接入统一审批；每个 HTTP 重定向和浏览器 HTTP 请求重新授权。交互浏览器补齐子资源拦截并绕过 service worker/缓存。截图指定用户路径时要求 Write。下载保留调用上下文。拒绝不再被聚合成空搜索或“0 个可读页面”。浏览器旧调用的延迟拒绝不影响新调用。公共地址限制返回明确的策略拒绝，网络批准不会越过这个独立限制。默认兼容常见 TUN 代理的 IPv4/IPv6 Fake-IP DNS，服务端也可声明额外网段，见下文。 |
| `agena.chatgpt` | OpenAI 请求、图片输入、云端媒体及远程文件 | 请求发送前检查实际 provider endpoint；本地图片与媒体先 Read。禁止自动重定向。审批目标移除 query/fragment，避免暴露签名或 API key。 |
| `agena.claude` | Anthropic 请求、云端媒体与文件管理 | 复用相同 endpoint 和本地输入授权；远程文件句柄继续绑定工作区、会话和 provider 连接。 |
| `agena.gemini` | Gemini 请求、图片/媒体、可恢复上传及文件管理 | 相同网络和文件检查；上传初始化、服务端给出的上传地址、查询/删除文件均检查实际请求地址。保留上传地址同源验证。 |
| `agena.mcp` | 资源、提示、工具调用与重连 | 为所有会发送 HTTP 请求的入口补齐 endpoint 授权；缓存工具索引搜索不发生网络。保留高风险 MCP 工具的独立审批检查。HTTP 客户端关闭隐式重定向，避免跳到未批准目标。配置的 stdio 服务器属于管理员安装的进程；它暴露的工具仍经过工具策略。 |
| `agena.shell` | 执行命令、交互 PTY、后台进程、终止进程 | 执行器已有工具/命令策略、声明的文件与网络效果检查和进程归属检查，保持统一执行路径。 |
| `agena.monitor` | 观察进程和接收后续输出 | 执行器已有归属检查；另外补齐宿主 SDK 的 monitor API：启动经过 shell.spawn 权限检查，声明实际效果，读取/列举/停止按工作区与会话过滤，停止经过 shell.stop 检查。 |
| `agena.cron` | 创建、列举、删除调度任务 | 主工具由执行器处理；宿主 SDK 同样检查 cron 工具权限，强制当前会话和工作区归属，不能指定其他 owner。 |
| `agena.lsp` | 本地语言服务与源文件操作 | 主工具已有执行器路径检查；补齐宿主诊断 API 对实际 file URI 的 Read 检查。配置的语言服务进程属于已安装运行时能力。 |
| `agena.session` | 会话信息、工作区 Git 状态、固定 CLI 的版本探测、重命名等 | 使用当前会话宿主能力；新增 Git 状态检查前的工作区 Read 和版本探测前的实际可执行文件 Read。CLI 名称及版本参数来自固定目录，不接受任意命令或参数。 |
| `agena.agent` | 创建子会话、传递消息、等待、取消和继续 | 宿主按活动调用、代理可见性、归属与预算处理；子会话执行实际工具时继续经过权限执行器。 |
| `agena.plan` | 计划的会话私有状态与用户评审 | 宿主作用域存储、版本检查和显式评审交互；不是任意磁盘写入。工作流 hook 复用这些受作用域约束的能力。 |
| `agena.tool_api` | 工具目录、帮助与插件状态 | 元数据查询不执行目标工具；实际 tools_call 继续经过完整工具权限流程。 |
| `agena.interaction` | 请求用户输入、通知 | 使用当前活动工具的持久交互/会话通知能力。提问本身不应再额外弹出“允许提问”的确认。取消或超时不是批准。 |
| `agena.terminal` | TUI 活动状态、展示贡献与通知 | 更新宿主展示状态，不执行终端命令。贡献归属与清理由插件 effect scope 管理。 |
| `agena.report` | 生成结构化发现和展示结果 | 无文件、网络或进程副作用；工具名策略仍适用。 |

`agena.chatgpt`、`agena.claude`、`agena.gemini` 在每次调用时复用当前会话的 provider 连接，包括认证、协议路径、适配器请求头和网络超时。插件不再提供独立的 API key、base URL、超时或默认模型配置，也不读取插件专用的模型/密钥环境变量。凭证只通过内置插件的宿主回调取得，不开放到外部插件 RPC，不进入工具输出。三组工具均使用简短名称，例如 `chatgpt.web_search`、`claude.web_fetch`、`gemini.image_generation`；云端执行与输入传输在简介和帮助中说明。

连接选择与模型选择分别处理：需要模型的接口默认采用当前会话模型，也可通过单次调用的 `model` 覆盖。当前模型须属于工具对应厂商（OpenAI、Anthropic Claude 或 Google Gemini）且支持所需操作；否则调用者必须显式指定合适的 `model`。模型默认值不会因工具协议不同而改选 provider 内其他模型；文件上传、查询、删除无需模型参数。插件保留各自的 Responses、Messages、Interactions / generateContent 等 API 协议，不做官方地址回退或本地执行回退。当前 provider 或网关必须支持请求的接口；仅支持聊天协议的网关不代表支持 Files 等接口。专用 OAuth 后端若未定义对应通用 API 连接，会明确报错。发送的会话标识可供网关维持亲和性，多账号下的远程文件和容器归属仍由网关负责。

完整接口、资源生命周期、模型发现、实时音频和网关要求见 [Provider API 工具说明](provider-apis.md)。`provider.models` 的远程发现同样检查网络权限；文件下载的显式输出路径经过 Write 检查，已有文件不会被覆盖。

## 宿主和传输补强

stdio 和 HTTP 使用独立的 `host/permission.require_path`、`host/permission.require_network` RPC；所有代理显式转发 enforcing 方法，保留宿主签发的上下文和结构化拒绝结果。只读 check 回调也携带上下文。HTTP 插件请求和回调均关闭自动重定向。

宿主图片 API 在读取本地路径/本地附件前要求 Read。宿主直接创建 MCP 服务时，HTTP 连接要求网络批准，stdio 启动经过 shell 命令权限检查。插件内部的 artifact、receipt、私有 storage 和 secret 服务由宿主作用域隔离，不为每次内部存储操作发起任意文件审批。

会话错误边界保留策略拒绝、用户拒绝及执行器已验证的插件 Failure 分类。策略拒绝不会被标成可重试的内部故障；用户明确拒绝也不会变成一条自动重试指令。爬取和浏览器适配器保留拒绝类型与原始 Failure 标识。

## HTTP / SOCKS 代理

网页搜索、普通 HTTP 抓取、robots、重定向预检和搜索 API 默认读取
`HTTP_PROXY`/`http_proxy`、`HTTPS_PROXY`/`https_proxy`、`ALL_PROXY`/`all_proxy`、
`NO_PROXY`/`no_proxy` 及支持平台的系统代理。服务进程未继承 shell 环境变量时，
也可在 `agena.web` 的 `settings.network.proxy` 中持久配置代理 URL，例如
`http://127.0.0.1:7890` 或 `socks5h://127.0.0.1:7890`；空字符串显式选择直连。
显式配置同样遵守 `NO_PROXY`。支持 HTTP/HTTPS 代理、CONNECT、代理认证及
SOCKS4/4a/5/5h，不会在代理失败后悄悄尝试直连。

授权和连接使用同一份路由快照。HTTP(S)、SOCKS4a、SOCKS5h 由用户配置的可信代理
解析目标域名，本机 DNS 无需成功，也不因本机的 Fake-IP 回答而拒绝请求。
SOCKS4/5 使用本机 DNS；它们和直连一样校验解析结果并绑定连接地址。
`NO_PROXY` 命中的请求走直连校验，不能借代理例外访问真实内网。
每次目标请求和重定向仍执行会话网络权限，代理模式下的私有 IP 字面量和局域网域名仍被拒绝。
远端解析的实际地址由可信代理控制，代理运营者负责其目的地址限制；不能把本机 DNS 检查
当作远端代理的地址保证。代理 URL 和认证信息不会出现在路由 Debug 或连接池标识中。

Chromium 仍使用其自身的系统网络配置与本机 TUN；上述 HTTP 客户端的显式代理不自动配置浏览器。

## TUN 代理的 Fake-IP DNS

Mihomo/Clash、sing-box、Surge 等代理的 Fake-IP 模式会把公网域名解析为本机 TUN
使用的映射地址。Agena 默认兼容以下常见网段，无需再填写配置：

- IPv4 `198.18.0.0/15`，包括 IPv4-mapped IPv6 表示。
- IPv6 `fc00::/18`。
- IPv6 `fdfe:dcba:9876::/64`。

网页搜索、搜索 API、HTTP 抓取、robots、每个重定向和受管浏览器的请求检查均使用这些默认值。
MCP OAuth 客户端元数据下载、终端 Markdown 远程图片也共享同一份默认地址规则。
终端图片与 MCP 元数据遵守标准 HTTP(S)/SOCKS 代理和 `NO_PROXY`；远端解析的代理无需本机 DNS 成功。
TLS 继续验证原始域名。

如本机代理使用其他 IPv6 ULA 映射网段，可在服务端 `agena.web` 配置中追加规范 CIDR：

```json
{
  "plugins": {
    "list": {
      "agena.web": {
        "package": { "kind": "static" },
        "settings": {
          "network": {
            "fake_ip_ranges": ["fdab:1234:5678::/64"]
          }
        }
      }
    }
  }
}
```

`fake_ip_ranges` 是额外网段；空列表和旧版本保存的单栈列表都不会关闭默认兼容。
最多追加 16 个网段，接受 `198.18.0.0/15`、`fc00::/18` 的子网及 `/64` 或更窄的 IPv6 ULA 子网。
工具参数不能修改这个设置。配置额外网段只影响网页插件；这些网段应由本机 TUN 实际接管。

Fake-IP 兼容仅适用于公网域名的 DNS 回答。直接填写 Fake-IP 地址、localhost、局域网域名、
真实私有地址、回环和链路本地地址仍会被拒绝；IPv4-mapped IPv6 同样按实际 IPv4 判断。
会话的网络 Deny、Ask、Auto 和用户拒绝继续先于连接执行。

直连或本机解析的普通 HTTP、API、HTML 搜索、MCP 元数据和终端图片请求会过滤不允许的地址，
并把连接绑定到剩余地址；双栈 DNS 中不可用的私有回答不再阻断另一条可用路由，也不会进入连接器。
HTML 搜索保留同源重定向限制。Chromium 仍自行解析并连接，因此浏览器预检要求所有 DNS 回答
都属于公网或受支持的 Fake-IP 网段，不对真实私有回答放行。浏览器的系统代理和 TUN 行为见上一节。

## 验证方式

单元测试覆盖 SDK stdio/HTTP 的真实回调序列化、代理转发、只读查询、类型保留、临时权限凭证、文件副作用、AST 预览/应用、笔记本、记忆、provider 目标脱敏、搜索部分成功、浏览器跨调用错误隔离和审批耗时。

真实服务回归使用 `scripts/probes/plugin_permissions.py`，只启动隔离数据库与本地 mock provider、搜索服务，验证 Ask 未回复前没有写入/出站请求、AllowOnce 后才执行、Deny/UserDeclined 不执行、Auto 准许与拒绝、取消后无副作用，还覆盖子路径拒绝、命令别名、记忆索引连带写入、批量抓取的串行审批，以及 AllowOnce 不跨调用泄漏。公共模型目录同步在这个隔离进程中禁用。运行方式：

```sh
cargo build -p agena --bin agena
python3 scripts/probes/plugin_permissions.py \
  --binary target/debug/agena \
  --output target/permission-validation/new-run
```

证据目录必须是新目录；保存每个用例的会话状态、Parts、HTTP 轨迹、服务日志、二进制哈希和汇总结果。

Fake-IP 实机验证使用隔离服务和本地 mock 模型，搜索和抓取实际访问公网。需要本机已经运行
TUN 代理；常见网段无需传参，其他网段可追加 `--fake-ip-range`。脚本不修改用户会话、系统 DNS 或代理配置：

```sh
python3 scripts/probes/web_fake_ip.py \
  --binary target/release/agena \
  --output target/web-validation/fake-ip-new-run
```

验证默认配置与已有显式网段配置下的多引擎真实结果、网页抓取、Internet Deny，以及 IP 字面量和回环地址
在网络审批允许时仍被公共地址策略拒绝。单个搜索引擎的验证码、限流或网络失败可以保留为部分结果诊断。
机器已安装 Chrome/Chromium 时，可追加 `--browser` 验证真实浏览器渲染路径。
追加 `--proxy http://127.0.0.1:7890` 会追加显式代理和环境变量代理两个阶段，验证真实搜索、抓取和拒绝行为。

## 本次验证记录（2026-10-25）

- 27 项权限相关单元测试通过（agena-async、plugin-sdk、plugin-host、runtime-session、runtime-tools、bundled-plugins）。
- 42 项 TUI 单元和集成回归通过。
- 完整 CLI 构建、Rust 格式检查和 git diff --check 通过。
- 隔离服务的 21 个用例全部通过，证据位于 `target/permission-validation/final/`。批量抓取用本地地址验证两次独立审批后仍受公共地址策略约束；不会为了测试开放内网抓取。
- 代理兼容修复的 12 项相关单元测试、严格 Clippy、格式检查和 release 构建通过。
- 同一最终二进制的 25 项权限回归通过，证据位于 `target/permission-validation/proxy-delivery-20261025/`。包含经真实 HTTP 代理访问 `.invalid` 目标的 Ask、AllowOnce、Deny、Auto 拒绝检查；未经批准不会向代理发出请求。
- 同一二进制的 14 项 TUN/显式代理实机用例通过，证据位于 `target/web-validation/proxy-delivery-20261025/`。原始英雄联盟查询在两条路径均返回 15 条结果；普通抓取及 Chrome 渲染返回 HTTP 200；当时版本的默认拒绝、Internet Deny、Fake-IP 字面量和回环地址拒绝均保持有效。

## Fake-IP 默认兼容验证（2026-10-26）

- 公共地址与 Fake-IP 分类 2 项、代理路由 3 项、内置插件 54 项、终端远程图片 3 项、MCP OAuth 23 项单元测试通过；额外复测了显式 `fc00::/18` 配置的加载与网段边界。
- HTTP OAuth 的 8 项集成回归通过，覆盖共享服务密码、授权/撤销/刷新、代理 DNS 与本地目标拒绝。
- 最终 release 二进制的 25 项实机用例通过，证据位于 `target/web-validation/fake-ip-defaults-20261026-final/`。默认、已有显式网段、显式代理和环境变量代理四种模式均返回 15 条真实搜索结果。HTTP 抓取与默认配置下的 Chrome 渲染返回 HTTP 200；Internet Deny、Fake-IP 字面量和回环抓取拒绝均通过。
- 本机实际把 `chatgpt.com` 解析为 `198.18.1.80`。隔离实例验证了真实 ChatGPT 元数据、密码授权、PKCE 换码、MCP 令牌访问与刷新；没有跟随外部回调。
- 隔离实例配置一个返回错误的环境代理，且移除 `NO_PROXY`，本地预览发现和预览转发仍成功，代理没有收到回环目标请求。
- release 构建、修改文件的 Rust 格式检查、Python 语法检查和 `git diff --check` 通过。

## 权限边界

这是 Agena 的协作式权限协议，不是所有插件包的操作系统沙箱。安装的原生、cdylib、stdio、HTTP 插件及 MCP/LSP 进程仍是可信代码；恶意程序自行调用系统接口不能靠 Rust trait 阻止。WASM 没有预打开宿主目录或授予网络。浏览器拦截覆盖 CDP Fetch 支持的 HTTP 请求，不能当成 Chromium 的完整网络沙箱或 DNS 连接绑定。配置的 provider、插件传输、MCP/LSP 启动与技能发现属于已安装运行时能力；会话中由工具选择的资源访问则执行上面的权限流程。
