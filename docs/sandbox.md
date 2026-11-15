# 工具沙盒：配置、执行与部署

Agena 默认使用 `mxc-sdk` 的原生进程沙盒执行模型触发的本地工作。作为 MCP 服务提供工具时，调用默认关闭 MXC，直接在宿主执行。路径权限放在 `permission.path`，网络权限使用 `permission.network` 的原生 `egress` / `ingress` 结构，工具审批放在 `permission.tools`；启用沙盒时，调用把获准资源直接转换为 MXC 策略。

路径规则的值只接受 `readonly`、`readwrite`、`deny`。它们分别对应 MXC 的只读、读写和拒绝路径。`ask`、`auto` 属于 Agena 的工具审批流程；路径与网络资源权限必须是能够执行的具体模式。

## 关闭沙盒

在启动拥有 Runtime 的服务进程时设置环境变量，即可关闭该进程的 MXC 隔离：

```sh
AGENA_SANDBOX=disabled agena server --workspace /path/to/project
# 后台启动同样继承该设置
AGENA_SANDBOX=disabled agena server start --workspace /path/to/project
```

`false`、`0`、`off` 也表示关闭。未设置时默认启用；`AGENA_SANDBOX=enabled` 明确启用。兼容旧的 `AGENA_SHELL_SANDBOX=disabled`，但 `AGENA_SANDBOX` 的优先级更高。设置在进程启动时捕获，修改后需要重启服务；在工具的 `env` 中填写这些值不会改变宿主的运行模式。

`agena mcp-server` 是常驻服务的 stdio 桥接客户端，工具实际在服务端执行。桥接调用与 HTTP `/mcp` 调用都会在服务端为本次执行关闭沙盒，无须额外设置环境变量。这个标记跟随调用的执行器与宿主回调，不改变同一服务中普通会话的默认沙盒模式，也不会复用另一模式下的 LSP、MCP 子连接或插件进程。

关闭后，shell、PTY、文件工具和辅助子进程直接执行，不要求 MXC 后端或沙盒挂载；命令保留宿主环境及调用方的显式环境设置。浏览器入口也不再因缺少 MXC 浏览器隔离而拒绝启动。工具入口、声明资源和 MCP 工具暴露仍按原授权检查；命令启动后按宿主用户权限访问文件和网络，实际 I/O 不再由 MXC 约束。

## 启用沙盒时的执行流程

1. 保存全局、项目或会话设置时，解析并校验规则。无法表示为受支持资源边界的路径、模式和条件返回具体错误，不写入配置。
2. 工具参数和 hook 转换完成后，执行器按当前项目、会话和来源身份解析实际路径、网络目标以及工具权限。
3. 工具审批处理 `allow`、`deny`、`auto`、`ask`；路径资源使用三种确定模式，网络资源只使用 `allow` / `deny`。审批不能扩大资源策略的允许范围。
4. `SandboxPermissionSnapshot` 捕获当前 principal 的路径和网络策略、独立权限上限及当前调用可见的持久规则。
5. 宿主将本次获准的资源声明转换成具体只读路径、读写路径、拒绝路径和网络策略。没有网络声明的调用采用离线策略。
6. 统一 launcher 启动 MXC，再启动命令或工具 worker。实际后端和主机依赖无法落实策略时返回启动错误，不转成普通宿主进程。
7. 文件修改类 worker 在私有暂存区生成结果，宿主验证修订和目标身份后发布到固定目标。

工具参数不能退出沙盒。隔离模式只由启动服务的受信任环境或 MCP 宿主调用入口选择，模型不能通过工具参数切换它。

## 路径权限

| 配置值 | 授予的权限 | MXC 对应字段 |
| --- | --- | --- |
| `readonly` | 读取文件或目录树，禁止修改 | `readonlyPaths` |
| `readwrite` | 读取和修改；目录内可创建、删除、重命名 | `readwritePaths` |
| `deny` | 禁止读取和写入 | `deniedPaths` |

目录规则覆盖目录及其完整子树，文件规则只作用于具体文件。不再分别填写 `read`、`write`，也不需要同时写目录本身和 `/**`。规则顺序不构成授权优先级，调整 JSON 条目顺序不会扩大资源权限。

### 项目配置示例

项目配置位于项目根目录的 `.agena/agena.json`。下面把 `input` 设为只读、`output` 设为读写，`secrets` 及其余未授权路径沿用默认拒绝；shell 启动需要确认，网络关闭。

```json
{
  "permission": {
    "path": {
      "workspace": "deny",
      "external": "deny",
      "rules": {
        "<workspace>/input": "readonly",
        "<workspace>/output": "readwrite"
      }
    },
    "network": {
      "egress": {
        "default": "deny",
        "allow": [],
        "deny": []
      },
      "ingress": {
        "default": "deny",
        "hostLoopback": "deny"
      }
    },
    "tools": {
      "names": {
        "agena.shell.exec": "ask"
      }
    }
  }
}
```

`workspace: "deny"` 是没有具体规则时的默认拒绝，不会把整个工作区加入 MXC 的 `deniedPaths`。因此它可以与 `input: "readonly"` 同时使用。支持显式路径拒绝的后端还可以增加 `"<workspace>/secrets": "deny"`，用于封锁已有授权根下面的秘密子树；在它下面重新开放子路径会报错。

将需要不同权限的数据放在并列的授权目录中。`readonly` 目录下面再设 `readwrite` 子目录，或 `readwrite` 目录下面再设 `readonly` 子目录，当前都会拒绝保存；不同后端的嵌套挂载和授权合并方式不能提供一致边界。允许在已授权根内设置后端支持的明确 `deny`。

路径的项目覆盖仍与全局设置合并，空的路径 `rules` 不会清空已经继承的路径规则。保存时必须验证合并后的生效配置，不能用项目中的宽松项绕过独立的权限上限。

### 默认值和路径别名

| 设置 | 默认模式 |
| --- | --- |
| 工作区路径 | `readonly` |
| 工作区之外的业务路径 | `deny` |
| `<state>` 的具体目录规则 | `readwrite` |
| `<tmp>` 宿主临时目录根 | 无内置规则 |
| 所有平台的出站、入站和宿主回环网络默认动作 | `deny` |

MXC 和工具 worker 所需的任务私有临时目录由宿主通过受信任的 `private_paths` 提供。需要处理宿主临时文件时，显式授权自己的具体任务目录，例如 `"<tmp>/my-task": "readwrite"`。

路径规则保留 `<workspace>`、`<cwd>`、`<home>`、`<tmp>`、`<state>` 别名。路径设置中的 `<cwd>` 规范为所属工作区；工具的相对资源声明仍按本次调用实际工作目录解析。`<state>` 只指当前实例、当前工作区的受管状态目录。

全局或 Bot 会话没有隐含项目根，需要使用绝对路径或工具支持的显式绝对 `folder`。未列出的用户数据不会因为运行程序而被整体加入本次沙盒，实际资源集合仍来自调用声明、运行时依赖和私有临时目录。

### 保存校验与旧配置迁移

- 路径模式必须是上述三个字符串。只写不读、资源模式中的 `ask`/`auto`、不完整的旧 `read`/`write` 对象都会报错。
- 使用具体文件或目录。`**/*.key`、`secret?.txt` 等普通通配模式、未知别名、控制字符和含 `..` 的路径组件不能保存。
- 明确拒绝父目录后重新开放子路径会报错；不能用排序调整绕过拒绝。
- 旧 `workspace`、`external` 中完整的三种确定读写组合可以转换为字符串。
- 旧正向路径规则对象只有在目录本身与同模式 `/**` 规则完整配对时，才会自动合并为一条递归目录规则。未配对的旧正向对象要求用户明确改成字符串，以确认新的路径范围；不根据扩展名猜测文件或目录。旧完整拒绝对象可以转成递归 `deny`，只会收紧访问。
- 旧 `read: allow, write: ask` 不能自动变成 `readwrite`。先把写入审批迁移到合适的工具或任务审批，再明确选择资源权限。
- TUI/Web 显示保存错误并保留可修正的编辑内容。校验失败不更新原有设置。
- 校验当前宿主的已知后端能力。macOS 拒绝非空的细粒度出站规则，Linux 拒绝允许入站或宿主回环的设置。如果所选 Windows 后端不支持 `FileSystemDeny`，明确的路径 `deny` 规则不能保存。默认 `workspace: "deny"`、`external: "deny"` 配合分离的只读/读写根仍可表达最小授权。

路径和主机状态可能在保存后改变；启动前仍需核对真实路径、符号链接、当前权限及后端能力。保存成功表示规则通过当前配置与已知能力校验，不代替在目标系统上启动沙盒。

### 单文件、输出目录与资源声明

已有文件可以单独授予 `readwrite`，不会因此自动开放父目录。创建多个产物时，使用单独的输出目录。

已有 `input`、`output` 目录时，Linux/macOS 的 shell 调用可以这样声明：

```json
{
  "command": "cat ../input/data.txt > result.txt",
  "workdir": "/path/to/project/output",
  "reads": ["../input"],
  "writes": ["."],
  "network": []
}
```

Windows 使用对应 shell 命令和实际路径，资源声明字段保持相同结构。`workdir` 需要读取授权；不要为了启动单文件任务把未授权的整个项目根作为工作目录。

- `reads`、`writes` 是工具本次申请的资源，与配置中已合并为字符串的权限模式不同。
- shell 的写入挂载目标必须已存在。新文件可以放在已授权且已存在的输出目录中，或先通过 `fs.write` 创建准确目标。执行器不会自动把新文件的父目录扩大为读写授权。
- 文件的原子替换可能需要同目录临时文件和父目录操作；精确单文件授权不承诺这些操作都可在普通 shell 内完成。Agena 文件修改工具通过私有暂存区和固定目标提交处理。
- 共享临时目录根，例如 `/tmp`、`/var/tmp` 及其祖先，不能通过允许规则开放为只读或读写；保存时会报错。其他任务的 `agena-sandbox-*`、`agena-tool-worker-*`、`agena-media-worker-*` 保留目录同样不能通过路径或符号链接获得授权。

### 运行时依赖

launcher 另行加入程序和系统库所需的只读路径，以及任务私有临时目录。显式拒绝与必需运行时重叠会使启动失败。API key、凭证环境变量和用户整套配置目录不会因为启动命令而整体继承。

自动加入的 PATH 目录只限系统程序目录。用户安装的程序默认只获得准确可执行文件；识别到的 Python 标准库、虚拟环境子树可以作为运行时加入。其他依赖通过受信任的 `runtime_paths` 声明。

显式设置 `LD_*`、`DYLD_*`、`GLIBC_TUNABLES`、`GCONV_PATH`、`LIBPATH` 或 `SHLIB_PATH` 会在启动前报错，错误只包含变量名。工作负载环境通过私有启动数据传递，外层 launcher 使用受信任环境。

`private_paths` 表示由宿主创建并持有的 scratch 所有权，本身不授予读取或写入。验证同时覆盖原路径、规范路径别名及 Windows 大小写变体。

## 工具与命令规则

工具审批继续使用 `allow`、`auto`、`ask`、`deny`。命令模式按配置顺序匹配，最后一条匹配规则生效；明确拒绝的工具不能被命令模式重新开放。

命令模式不能使用会被 JSON 编辑器当作数组索引的纯数字键：`0` 到 `4294967294` 的规范十进制写法（无前导零）。这类键会自动重排，保存时返回错误；匹配数字名称的程序及其参数时，可使用 `123 *` 这样的模式。

## 权限上限与持久规则

会话、子代理父级限制和 channel 权限围栏按原作用域组合，独立权限上限取交集。后加入的宽松配置不能抹掉上限中的拒绝。

持久规则按调用所属的 `global → workspace → session` 作用域解析，每个作用域与动作键采用最新且未撤销的记录。明确拒绝不能被记住的批准升级。资源批准之后，本次具体清单仍需通过 MXC 转换。

持久 `NetworkAccess` 只接受 `allow`、`deny` 和数值 IP 目标，例如 `192.0.2.10:443`、`[2001:db8::10]:443`、`udp://192.0.2.10:53` 或 `icmp://192.0.2.10`。裸 IP 匹配该地址的全部协议/端口，允许决定仍受配置范围限制；CIDR 范围放在 `permission.network` 中设置。域名、`ask`/`auto`、含路径或凭据的 URL 无法保存。macOS 的后端不支持这种精确 IP 决定，保存时会明确报错。历史记录继续展示并可撤销或删除；编辑后保存同样执行新校验。

快照按调用所属会话绑定工作区和来源身份。长期进程的权限在创建时固定；改变设置不会给已运行的进程追加挂载或网络权限。

## 网络设置与执行

`permission.network` 直接采用 MXC 的网络模型。Agena 不再启动沙盒 HTTP 代理，不向任务注入代理地址或认证信息。配置只描述数值 IP/CIDR、协议与端口边界，进程通过原生 socket 联网。

### 原生策略示例

```json
{
  "permission": {
    "network": {
      "egress": {
        "default": "deny",
        "allow": [
          {
            "to": [
              {
                "cidr": "10.20.0.0/16",
                "except": ["10.20.10.0/24"]
              },
              {
                "cidr": "fd20:20::/48"
              }
            ],
            "ports": [
              { "protocol": "tcp", "port": 443 },
              { "protocol": "tcp", "port": 8000, "endPort": 8099 }
            ]
          },
          {
            "to": [{ "cidr": "192.0.2.53/32" }],
            "ports": [
              { "protocol": "udp", "port": 53 },
              { "protocol": "tcp", "port": 53 }
            ]
          }
        ],
        "deny": [
          {
            "to": [{ "cidr": "10.20.66.0/24" }]
          },
          {
            "to": [{ "cidr": "10.20.0.0/16" }],
            "ports": [{ "protocol": "tcp", "port": 8080, "endPort": 8089 }]
          }
        ]
      },
      "ingress": {
        "default": "deny",
        "hostLoopback": "deny"
      }
    }
  }
}
```

示例中的 IP 是规则示意，运行时换成实际业务目标与 DNS 服务器。单个 IPv4 地址使用 `/32`，IPv6 地址使用 `/128`。

- 拒绝规则优先于允许规则，与 JSON 或数组的先后顺序无关。本例允许 TCP 8000～8099，再拒绝 TCP 8080～8089。
- 一条规则中的多个目标为“或”，多个协议/端口选择器也为“或”；目标与协议/端口两组条件之间为“且”。
- `except` 只从当前目标选择器中减去子网；其他允许规则仍可能授予这个子网。需要始终禁止的目标应写入 `deny`。
- 省略 `to` 表示所有目标，省略 `ports` 表示所有协议/端口；显式的 `to: []` 或 `ports: []` 无效。
- 协议支持 `tcp`、`udp`、`icmp`、`any`，省略协议按 `any` 处理。端口范围包含两端；端口须为 1～65535，`endPort` 需要同时有 `port` 且不得更小，ICMP 不能指定端口。
- 未提供的字段继承下层配置；明确的 `allow: []`、`deny: []` 替换并清空继承的同名列表。没有任何层设置的默认动作最终为 `deny`。
- 旧的 `internet`、`private`、`loopback`、`rules` 字段、域名规则和网络 `ask` / `auto` 不再接受，不做会扩大权限的隐式迁移。保存时会给出字段级错误。

### 工具网络资源声明

工具的 `network` 声明申请本次调用需要的网络范围，最终策略始终与当前设置及每一层独立权限上限取交集：

| 声明 | 本次申请的范围 |
| --- | --- |
| `[]` | 离线，拒绝外部 IP 网络与宿主回环访问 |
| `["*"]` | 当前配置已经允许的完整原生网络范围；`*` 必须单独使用 |
| `["192.0.2.10:443"]` | 目标 IP 的 TCP 443 |
| `["api.example.com:8443"]` | 启动前解析出的目标 IP 的 TCP 8443 |
| `["https://api.example.com/resource"]` | 目标 IP 的 TCP 443；URL 路径不构成网络隔离条件 |
| `["udp://192.0.2.53:53"]` | 目标 IP 的 UDP 53 |
| `["icmp://192.0.2.10"]` | 目标 IP 的 ICMP |
| `["api.example.com"]` 或 `["192.0.2.10"]` | 该目标内当前策略已经允许的协议和端口，不隐含开放 80、443 |

主机名在父进程中解析并转换为具体 IP，再生成原生 MXC 规则。域名声明不提供 HTTP Host、TLS SNI 或 URL 级隔离；同一获准 IP 上的其他服务仍按协议和端口边界处理。

父进程在解析域名前先检查 DNS 权限：当前发现的所有解析器都必须获准使用 UDP/TCP 53，并通过当前设置和全部权限上限检查，不能借域名声明自动开放 DNS。无法证明解析器已获准时，使用具体 IP 声明，或明确申请已经配置的完整范围。使用 `["*"]` 时同样需要在策略中允许实际解析器，或明确允许所需出站范围。Linux 的隔离网络可能使用 MXC 提供的解析器入口，最终地址在编译和启动阶段核对。

已保存的网络批准不能扩大静态策略，保存的拒绝必须落实为原生拒绝规则。无法转换为数值 IP/CIDR 的旧域名拒绝会在联网策略编译时返回错误，要求改为原生规则。

Linux 的私有网络命名空间可以保有沙盒内部回环；这与宿主机 `127.0.0.1` 是不同对象。

### 宿主 HTTP 请求与 DNS 地址校验

宿主 Web 插件的普通抓取、重定向、robots.txt 请求和 HTML 搜索使用同一份调用网络权限。DNS 返回的数值地址必须通过当前原生配置、独立权限上限和持久拒绝检查；只有允许的地址进入 HTTP 客户端的固定解析表。没有获准地址时返回权限错误。

受控下载、provider HTTP/WebSocket 和 MCP 出站访问同样绑定实际连接地址。MCP 对每个 POST、GET、DELETE 请求及 OAuth 的每次跳转先检查原始 URL，再解析并逐个审核数值 IP，将获准地址固定到本次 HTTP 客户端；禁用其代理和自动跳转，避免把目的地址交给另一次 DNS 解析。原始 hostname 继续用于 HTTP Host 与 TLS 校验。

这条校验通过宿主的 `check_resolved_network_permission` 接口执行，保留原调用的身份与权限上限；原生、HTTP 和 stdio 插件回调使用相同接口。

服务自身的 HTTP/SOCKS 路由配置与沙盒权限策略分开。Web 插件不会把已经审核的目标交给远端代理重新解析；无法固定实际目的 IP 的代理 DNS 路由会明确报错。移除的是每次沙盒调用创建的权限代理，服务与 provider 自己的网络配置仍按各自客户端处理。

### 浏览器渲染的当前状态

启用沙盒时，Chrome 服务尚未接入与调用权限一致的 MXC 隔离及固定目的地址的传输，所以 `browser_open`、浏览器动作、浏览器下载和 `render_js: true` 在启动或执行前返回 `HostUnavailable`。抓取及搜索的浏览器重试同样受此入口检查约束，不会启动无沙盒 Chrome。关闭沙盒的调用不受此隔离入口限制。单纯拦截页面请求或预先检查 DNS 不能替代实际连接的隔离。

需要读取页面内容时，可使用普通 HTTP 抓取：

```json
{
  "url": "https://example.com",
  "render_js": false
}
```

页面必须执行 JavaScript 时，启用沙盒的调用会报告当前渲染隔离尚不支持该操作。支持隔离渲染仍需完成浏览器进程、文件资源、控制通道和实际网络连接的统一隔离；MCP 或显式关闭沙盒的调用使用宿主浏览器执行。

### 各平台的原生网络能力

| 平台 | 支持范围与保存校验 |
| --- | --- |
| Linux / Bubblewrap | 支持上述 CIDR、排除子网、协议与端口规则；`ingress.default` 和 `ingress.hostLoopback` 必须为 `deny` |
| Windows / ProcessContainer | 使用原生数值 IP、协议和端口规则；运行时检查当前 Windows 版本与 ProcessContainer/PSEC 能力 |
| macOS / Seatbelt | 支持出站默认动作；非空的 `egress.allow` / `egress.deny` 规则不能保存。允许出站默认动作时，可用 `network: ["*"]` 申请该原生范围；具体目标限制因需要细粒度规则而不能执行 |

Agena 不会为不支持的规则改用代理，也不会降级为不受沙盒限制的进程。

## 已接入的工具和辅助进程

下表描述启用沙盒时的执行边界；MCP 或显式关闭沙盒的调用按前述宿主模式执行。

| 工具或执行入口 | 隔离方式与资源边界 |
| --- | --- |
| `shell.exec`、`shell.spawn`、`shell.watch` | 同一 MXC launcher；沿用声明的读取、写入、网络目标、工作目录和超时，保留原有前台/后台输出和进程归属 |
| `shell.open` 与 PTY 后续输入 | 初始终端在同一隔离边界内创建；后续输入继续接受原权限检查，不能扩大已经运行的进程权限 |
| `fs.read`、`fs.glob`、`fs.grep`、`fs.read_many` | 独立 worker；目录遍历和文件读取同时受获准根、细粒度路径规则与 OS 文件系统策略约束 |
| `fs.stat` | 普通文件在独立 worker 中查询；符号链接通过宿主已授权目标的元数据 broker 保留 `lstat` 语义，不读取链接目标内容 |
| `fs.write`、`fs.replace`、`fs.apply_patch` | worker 在私有暂存区运行；宿主只向固定、已授权的目标发布验证后的结果 |
| `code.search_ast`、`code.syntax_tree`、`code.rewrite_ast` | 语法解析在 worker 内；实际改写使用暂存和宿主发布流程 |
| `fs.read_media` | 本地读取、文件格式识别、图片解码和像素限制在离线 worker 内；宿主验证传输大小、哈希和元数据后绑定当前模型 route |
| LSP 子进程 | 根据实际项目根与会话权限构造只读项目策略，使用私有 HOME/cache；不同完整策略不会共用同一个服务进程 |
| MCP stdio 服务 | 配置按需启动；发现、调用、资源、提示与重连都选择当前宿主授权的 scope 和完整策略实例；连接测试使用独立沙盒进程 |
| 外部 stdio 插件 | 最小离线 bootstrap 读取插件元数据；实际工具、流式调用、hooks 和 services 按当前宿主授权 scope 与完整策略选择独立进程 |
| 文档转换和提取辅助程序 | 外部 Python/文档转换器使用离线 MXC，仅获得已准备的有界输入副本及私有临时资源 |
| 固定 CLI 版本探测 | 原有工具名单、参数和时间/输出限制继续生效；探测进程也经统一原生沙盒启动 |
| `session.environment` 的 Git 探针 | 使用当前宿主调用授权投影的只读工作区策略和离线进程；Git 信息不可用时返回诊断，不改为宿主 Git 执行 |
| Web HTTP 抓取与 HTML 搜索 | 宿主逐目标授权，使用当前调用权限检查 DNS 返回 IP，并将允许地址固定到实际客户端；外部代理重新解析目标的路由返回错误 |
| 浏览器渲染和交互 | 当前入口在执行前拒绝；Chrome 尚未接入完整 MXC 边界，辅助进程自身的隔离不足以开放浏览器功能 |

PTY 和其他常驻子进程的内核权限在创建时固定。后续 `shell.write` 即使获准发送新的命令，也只能在这个边界内执行；需要新增目录或网络目标时，应带着新的资源声明创建终端或进程。修改设置不会给已经运行的进程追加挂载或网络权限。

启用沙盒时，缺失目标程序、必要运行时文件、沙盒后端或联网依赖会返回具体启动错误。执行器不会自动转为宿主直跑，也不会通过放宽目录或网络策略重试；直接执行必须在调用前明确选择。

### 文件修改的宿主发布

暂存区只包含本次操作需要的输入副本与候选输出。worker 给出的任意真实路径不会成为新的宿主写入目标。

宿主在准备阶段固定目标映射并持有文件变更锁；发布阶段再次检查原始字节修订、文件类型、目标身份、大小限制和策略。发布使用固定目录能力与相对文件操作，避免通过父目录符号链接替换把已批准路径重定向到另一目录。

文本补丁保持 UTF-8、单文件和事务总量限制。新增、更新、删除通过同一组已固定的目录能力提交和回滚；多文件的确定性验证在首次修改之前完成。移动文件的目标权限来自宿主捕获的真实源文件权限，不由 worker 返回的目标路径或权限关系决定。

文件内容在处理期间被其他程序修改时，提交返回修订冲突，保留外部修改。取消或 worker 失败不进入结果发布。输出目录的创建也是宿主针对已获准目标执行的固定操作，不需要给 worker 整个宿主父目录的写权限。

### 媒体传输

`fs.read_media` 的单文件上限是 20 MiB；文本输入上限是 1 MiB；图片边长最多 16384 像素，总像素最多 4000 万。跨进程传输使用专用 DTO，宿主检查 base64 长度、解码后字节数、SHA-256、文件名、媒体类型和尺寸元数据，随后沿用原有 attachment 与 provider route 绑定。

共享的普通 `PreparedMedia` 序列化仍只包含元数据。媒体 worker 的私有传输字段不会因此进入其他调用的常规 JSON 输出。

用户附件准备、结果附件处理以及 provider 上传工具中其他直接使用共享媒体准备函数的路径，仍需分别迁移到具备适用授权上下文的 worker。此处的进程隔离覆盖表中的 `fs.read_media` 路径。

### LSP 服务复用

LSP 缓存键包含会话身份、服务器名称、实际根目录和完整 `SandboxPolicy`，使用完整策略相等性。不同会话或不同权限策略不共用语言服务进程；只有已持久允许的项目读取范围才进入会话缓存。仅本次获批的 `ask`/`auto` 根目录以及没有持久会话身份的调用使用临时客户端，最后一个调用句柄释放时回收进程。

语言服务所需的程序和安装目录由受信任配置提供；当前工具接线保持项目只读，HOME、缓存和 `CARGO_TARGET_DIR` 写入进程私有目录。需要在项目内写缓存的语言服务，应配置为使用其私有缓存目录；当前接线不会自动授予项目写权限。

### MCP stdio 服务

MCP 配置位于 `plugins.list["agena.mcp"]`。该插件的 `settings.servers.<name>.process` 沿用 `command`、`args`、`env`、`cwd`，增加 `reads`、`writes`、`network` 资源声明。省略三组声明时，服务仅获得工作目录的读取权限，网络关闭。资源范围由路径、原生网络、项目设置、全部 ceiling 和当前持久拒绝共同限制；需要确认的启动操作由工具审批处理。

Linux/Windows 在原生后端能力可用且设置允许目标 IP 时，可以这样声明输入、输出和网络目标；macOS 可选择离线或已获准的完整原生出站范围：

```json
{
  "transport": "stdio",
  "process": {
    "command": "node",
    "args": ["server.mjs"],
    "cwd": "/path/to/project",
    "reads": ["input"],
    "writes": ["output"],
    "network": ["api.example.com:443"]
  }
}
```

相对资源路径基于服务的实际 `cwd`；`<workspace>` 指向当前会话项目。显式相对 `cwd` 延续原有服务配置所在工作区的解析规则。写入目录须已存在，并同时经过读取授权。

加载配置只登记 stdio 定义，管理列表以 `scoped` 表示需要执行作用域。实际使用时，宿主验证当前会话、调用或 operator 能力，完成资源审批并生成策略。实例按服务器、配置版本、宿主 scope、工作区、实际工作目录和完整策略隔离；不同会话或更窄策略不会借用已有宽权限进程。实例退出后，下一次合法调用会重新检查当前权限再连接；显式重连也走相同流程，后台不会脱离授权以旧策略创建进程。

当所有资源已由现有策略明确允许时，同一会话可以复用相同策略实例；一次性批准的资源和 operator 调用绑定本次宿主调用的完成信号，调用结束即移除并关闭对应实例。`connections.test` 总是建立独立受限连接并关闭它。缺少可信调用 scope 或后端能力时直接返回错误。

工具访问 HTTP MCP 时，每次宿主调用创建独立连接管理器，保留该服务器的认证存储、工具过滤、roots 和超时配置。连接、重连、OAuth discovery/registration/refresh 及 token exchange 都使用当前调用的授权上下文和逐请求固定地址。清理在建立连接前注册，调用成功、失败或取消后关闭该独立管理器；连接测试也在取消时回收。结束的 OAuth 授权作用域不能退回可信管理客户端。

### 外部 stdio 插件

`plugins.list["<plugin-id>"].package` 在 `kind: "stdio"` 时支持 `reads`、`writes`、`network` 声明，默认均为空，授权仍复用现有设置。bootstrap 使用独立空工作目录，只读开放已配置插件包、明确解释器入口及必要运行时，网络关闭；它负责 manifest、初始化和关闭等基础生命周期。

其他工具、流式工具、hooks 和 services 必须取得当前有效的宿主授权作用域。实例按 scope、工作区、工作目录及完整策略隔离，初始化继承当前会话上下文；动态工具注册进入对应会话 overlay，并随进程 effect scope 清理。子进程回调只能使用该进程当前活跃请求收到的 token，同时匹配其会话、工作区和适用的调用边界。

一次性授权的实例在宿主调用完成后移除并关闭。scope 实例退出后，下一次合法调用会重新求取当前策略再创建进程；不会在后台脱离原授权重新启动。没有可信作用域的非基础生命周期返回明确错误，bootstrap 不承载会话工具调用。

## 进程内插件和浏览器进程

cdylib 插件与 Agena 宿主在同一进程中运行，必须作为受信任代码配置；对子进程的 MXC 启动包装不能隔离已载入的动态库。

Playwright Python/Node 辅助进程已有独立的私有 HOME、暂存区和受限 CDP 通道；Chrome 主进程尚未获得同样的资源边界，因此启用沙盒时，浏览器功能通过入口检查拒绝执行。已运行的外部浏览器不会被视为具备当前会话的 MXC 隔离。

后续浏览器服务需要按会话权限创建独立进程/profile，统一约束文件、控制通道与实际网络连接。需要加载不受信任动态库的能力时，同样应迁移到受限工作进程。

## 平台要求与当前部署状态

| 平台 | 原生后端 | 部署要求与当前功能 |
| --- | --- | --- |
| Linux | Bubblewrap | 需要可用 `bwrap` 和允许创建所需 user/mount/network namespace 的系统策略；联网模式还需 `slirp4netns` 及后端要求的网络工具 |
| macOS | Seatbelt | 使用 MXC 支持的 macOS 版本；MXC 1.1.0 的 macOS 基线为 macOS 15；离线和出站默认动作由本机后端检查，不支持细粒度出站目标规则 |
| Windows | Process Container/AppContainer | 需要包含相应 Process Container 能力的 Windows 版本；路径拒绝依赖对应文件系统拒绝能力；离线与原生数值 IP/协议/端口策略按本机后端能力检查 |

部署版本、Windows 能力标志和各后端差异以 MXC 上游文档与本机探针结果为准：

- [MXC SDK 与运行方式](https://github.com/microsoft/mxc/blob/main/src/mxc-sdk/README.md)
- [MXC 策略结构与能力](https://github.com/microsoft/mxc/blob/main/docs/schema.md)
- [Linux Bubblewrap 后端](https://github.com/microsoft/mxc/blob/main/docs/backends/bwrap/bubblewrap-backend.md)
- [macOS Seatbelt 后端](https://github.com/microsoft/mxc/blob/main/docs/backends/seatbelt/seatbelt-backend.md)
- [Windows 文件系统策略](https://github.com/microsoft/mxc/blob/main/docs/backends/process-container/filesystem.md)
- [Windows 网络策略](https://github.com/microsoft/mxc/blob/main/docs/backends/process-container/networking.md)

### 通过安装器自动准备

通常直接运行安装命令即可补齐依赖、配置系统权限并验证真实沙盒：

```bash
bash ./scripts/agena/install.sh reinstall --source --host 0.0.0.0
```

默认 `--setup-mode auto` 会先准备源码构建所需工具，再补齐原生沙盒运行依赖。Linux 会按最终安装路径生成专用 AppArmor userns profile、准备 `nf_conntrack` 和联网工具；系统步骤在需要时通过 `sudo` 请求密码。macOS 原生服务会在安装阶段通过稳定的 `Agena.app` 身份集中检查/申请辅助功能、屏幕录制，并打开完全磁盘访问设置。Windows 源码安装会准备相应工具链，并在需要时请求 UAC 授权。已有路径、网络和工具审批设置继续保留。

准备阶段失败时旧服务保持运行；新程序复制到最终路径后必须通过 `--agena-sandbox-check`，否则回滚程序和原运行状态。各平台必须通过离线隔离检查；Linux/Windows 还必须通过原生网络自检。macOS 将细粒度原生网络检查标为非必需的 `unsupported`，同时保留可用的出站默认动作能力。成功报告保存在 `<install-dir>/sandbox-report.json`。

Linux 安装器按 `/etc/os-release` 的发行版家族选择 `apt-get/apt`、`dnf/dnf5/yum`、`pacman`、`zypper` 或 `apk`，分别处理开发库、网络工具及 AppArmor 的包名。支持通过 `sudo`、`doas` 或交互式 `su` 完成系统步骤；systemd/OpenRC 的模块开机加载也分别处理。Alpine 使用 `--source` 构建原生 musl 版本，或通过 `--archive` 提供原生包；当前官方 Linux 发布包为 glibc。各包管理器分支的离线测试覆盖调用和配置行为，最终可用性由目标宿主的真实隔离探针决定。

只修复系统准备并重新检查已安装程序，不重装或重启服务：

```bash
bash ./scripts/agena/install.sh permissions
# 仅检查，不安装依赖或修改系统配置：
bash ./scripts/agena/install.sh permissions --setup-mode check
```

Windows 对应 `install.ps1 -Action Permissions`；只检查时增加 `-SetupMode check`。完整选项、各平台权限流程见 [安装文档](installation.md#automatic-setup-and-sandbox-verification)。

开发机上的真实检测也用于确认失败是否能被准确报告：Ubuntu 24.04.3 的现有安装在尚未执行系统准备时，离线检查返回 `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`，联网检查返回 `unshare` 写入 `uid_map` 被拒绝；该机启用了 AppArmor userns 限制且缺少 `slirp4netns`。这些失败会使安装自检返回非零退出码，待安装器完成权限准备后重新确认。

### Ubuntu 24.04 的手动管理员配置

Ubuntu 24.04 对无专用 AppArmor profile 的非特权程序限制 user namespace 内的 capabilities；发行版说明推荐给具体应用添加含 `userns,` 的 profile。[Ubuntu 24.04 发布说明](https://discourse.ubuntu.com/t/ubuntu-24-04-lts-noble-numbat-release-notes/39890) 给出了 `flags=(unconfined)` 应用 profile 的模板。

先安装运行依赖：

```bash
sudo apt update
sudo apt install bubblewrap slirp4netns iproute2 iptables util-linux kmod apparmor-profiles apparmor-utils
```

Agena 可执行文件、MXC 后端工具及宿主 PATH 中的程序目录必须安装在任务可写目录之外；启动前会拒绝覆盖这些受信任路径的读写授权，包括符号链接别名。

为实际运行的 Agena 可执行文件建立精确路径 profile。下面以安装路径 `/opt/agena/bin/agena` 为例，保存为 `/etc/apparmor.d/agena-userns`；部署到其他位置时替换这一条绝对路径：

```text
abi <abi/4.0>,
include <tunables/global>

"/opt/agena/bin/agena" flags=(unconfined) {
  userns,
  include if exists <local/agena-userns>
}
```

加载这一个 profile，并检查状态：

```bash
sudo apparmor_parser -r /etc/apparmor.d/agena-userns
sudo aa-status
```

profile 的目录和单文件加载方法见 [Ubuntu Server AppArmor 文档](https://ubuntu.com/server/docs/how-to/security/apparmor/)。配置后从匹配的可执行文件路径启动新进程。测试用 `sandbox_doctor`、`sandbox_smoke` 是独立二进制；测试这些程序时，需用各自实际绝对路径配置对应 profile，不能把 Agena 主程序的匹配规则当成测试程序的授权。

[Ubuntu 的 apparmor-profiles 文件清单](https://packages.ubuntu.com/noble-updates/all/apparmor-profiles/filelist) 也包含 `bwrap-userns-restrict` 和 `unshare-userns-restrict` 候选 profile。它们位于 `/usr/share/apparmor/extra-profiles/`，应先审阅与已有规则的重叠及子进程能力限制。MXC 联网模式使用 `unshare --user --map-current-user --keep-caps` 启动网络 supervisor；仅配置 `bwrap` 不能替代对联网启动链的验证。

随后分别验证离线沙盒和带获准网络目标的实际工具调用。若仍返回 `EPERM`，管理员可结合 `sudo journalctl -k` 中的 AppArmor 拒绝记录核对命中的 profile、可执行文件路径和具体操作。这些步骤保留系统级 user namespace 限制，通过应用专用规则提供所需能力。

## 诊断和验证

使用已安装的主程序运行与安装器相同的真实自检：

```bash
agena --agena-sandbox-check
```

该入口在加载配置、凭证和插件之前执行，输出单行 JSON。`schema_version` 为 `1`，`ok` 与退出码一致，`checks` 分别列出 `offline` 和 `native_network` 的 `required`、`status`、`detail`。Linux/Windows 的原生网络检查使用本机非回环 IPv4 上的两个临时 TCP 端口：一个允许端口必须返回预期数据，另一个同时位于 allow 和 deny 的端口必须被拒绝，并检查任务没有代理环境变量。检查完成后清理私有临时目录和监听端点，不访问外部网站。AppArmor 按程序路径匹配，诊断时应直接使用最终安装路径的二进制。

在仓库根目录运行诊断示例：

```bash
cargo run -p agena-process --example sandbox_doctor
```

doctor 列出 SDK 发现的后端，检查 Linux 所需程序和 namespace/AppArmor 状态，再实际启动一个离线隔离进程。程序存在或后端被列出，只说明能够发现该能力；最终启动探针的成功结果才表示当前宿主可以启动沙盒。

修复宿主部署后运行真实原生 smoke：

```bash
cargo run -p agena-process --example sandbox_smoke
```

smoke 检查输入可读、输入不可写、输出可写、秘密目录不可读取、外部路径不可读取、宿主回环不可访问、环境清理、私有 HOME、超时和子孙进程回收。后端不可用时，该测试会直接失败，不能跳过为成功。

针对实现的回归与编译检查：

```bash
cargo test -p agena-runtime-contracts permission::sandbox --lib
cargo test -p agena-runtime-tools --lib
cargo test -p agena-process sandbox --lib
cargo test -p agena-lsp --lib
cargo test -p agena-mcp-client manager::scoped --lib
cargo test -p agena-runtime-config stdio_resource_declarations --lib
cargo test -p agena-plugin-host transport::stdio --lib
cargo check -p agena --bin agena
```

MCP 的真实 stdio 协议集成测试还需要 Python 3、可工作的 MXC 后端及已构建的 Agena 主程序；测试 harness 本身不提供 launcher 自举入口：

```bash
AGENA_TEST_SANDBOX_LAUNCHER=/absolute/path/to/agena cargo test -p agena-mcp-client --test connections
```

依赖已经缓存时可附加 `--offline`。上述命令分别覆盖权限快照、DNS 地址限制、工具 worker、暂存发布、媒体传输、底层策略、LSP 复用和调用接线；真实 OS 隔离结果仍由 doctor/smoke 给出。

### 常见失败与处理

| 现象 | 原因和处理 |
| --- | --- |
| Linux 创建 user namespace 返回 `EPERM` | 检查 doctor 的实际 bwrap 路径、AppArmor 限制和发行版专用 profile；修复后重新运行 doctor |
| 找不到 `slirp4netns` | 先安装当前发行版支持的联网依赖；离线策略与联网策略分开验证 |
| 原生网络规则提示后端不支持 | macOS 只使用出站默认动作或离线；Windows 检查宿主 ProcessContainer/PSEC 能力；不能静默改用代理或无沙盒执行 |
| `existing writable mount` | 声明现有输出目录，或先用 `fs.write` 创建准确文件；不自动扩大到父目录 |
| 路径规则 `Unsupported` | 检查负向文件名通配、字面量目录与子树默认值、拒绝父目录中的子授权；使用可表达的明确目录边界 |
| `ReadApprovalRequired` | 读写挂载会包含读取能力，需要补足该写入目标的读取授权 |
| 域名没有可用的获准 IP | 检查实际 DNS 结果是否命中原生允许规则，以及是否被配置或权限上限拒绝；主机名本身不授予访问权限 |
| 补丁修订冲突 | 文件在暂存后被修改；重新读取最新版本并准备修改，不覆盖外部变更 |

## 源码入口

| 模块 | 职责 |
| --- | --- |
| `agena-runtime-contracts/src/permission/sandbox.rs` | 可序列化策略、完整权限上限、路径边界和 DNS 地址二次检查 |
| `agena-runtime-tools/src/tool/executor/executor_sandbox.rs` | 当前 scoped principal 与已保存规则绑定 |
| `agena-runtime-tools/src/shell_sandbox.rs` | shell 和工具路径/网络声明到执行策略的投影 |
| `agena-runtime-tools/src/sandbox_worker.rs` | 受限工具 worker、暂存和宿主发布 |
| `agena-runtime-tools/src/tool/apply_patch.rs` | 补丁解析、计划与 worker 内文本修改 |
| `agena-runtime-tools/src/fs_broker.rs` | 固定目录能力、发布前修订校验、原子提交和回滚 |
| `agena-runtime-session/src/session/manager/host/process_sandbox.rs` | native 进程声明的现有审批与当前 scope 策略 |
| `agena-mcp-client/src/manager/scoped.rs` | MCP 定义版本、完整策略与作用域实例生命周期 |
| `agena-plugin-host/src/transport/stdio/scoped.rs` | 外部 stdio 插件的 bootstrap 与作用域进程路由 |
| `agena-runtime-tools/src/media_input.rs` | 媒体准备与私有 IPC 传输校验 |
| `agena-process/src/sandbox.rs` | MXC launcher、环境、运行时目录、平台能力和进程生命周期 |
| `agena-lsp/src/registry.rs` | 按完整权限策略区分语言服务实例 |
