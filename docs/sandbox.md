# 工具沙盒：配置、执行与部署

Agena 使用 `mxc-sdk` 的原生进程沙盒执行模型触发的本地工作。用户继续在现有设置中管理路径、网络、项目与会话权限：`permission.path`、`permission.network`、工具规则、会话权限上限和已保存的审批规则共同决定本次调用的资源边界。

普通用户无需建立另一套沙盒权限设置。审批决定“这次操作是否获得授权”，MXC 将获准的具体资源落实为子进程的文件系统和网络限制。后端无法落实策略时，工具返回错误，执行器不会改为普通宿主进程继续运行。

## 执行流程

1. 工具参数和 hook 转换完成后，执行器按当前项目、会话和来源身份解析实际路径、网络目标以及工具权限。
2. 现有权限流水线处理 `allow`、`deny`、`auto`、`ask`。自动审批和交互审批仍使用原有模型、界面与持久规则。
3. `SandboxPermissionSnapshot` 捕获当前 principal 的路径和网络策略、全部独立权限上限，以及本次调用所属会话可见的持久规则。
4. 宿主将已经获准的资源声明投影成只读目录、读写目录、明确拒绝路径和网络策略。未声明网络效果的调用采用离线策略。
5. 统一 launcher 启动 MXC，再启动真正的命令或 Agena 工具 worker。worker 在普通配置、凭证和插件初始化之前进入专用执行入口。
6. 宿主处理输出、取消、超时和结果发布。文件修改类 worker 先在私有暂存区生成结果，宿主验证修订和目标身份后发布到固定目标。

工具参数不能选择退出沙盒。旧的 `AGENA_SHELL_SANDBOX=disabled` 不再关闭隔离；旧的 `offline`、`required` 取值也不构成另一条执行路径。新实现不读取可通过继承环境关闭沙盒的开关。

## 沿用现有设置

### 项目、全局和会话

项目配置仍位于项目根目录的 `.agena/agena.json`，全局配置仍来自当前 Agena 实例。既有配置加载顺序、环境和启动覆盖、会话有效配置继续生效。

项目会话的相对路径以所属项目解析；有工作目录的 shell 资源声明以实际工作目录解析。全局或 Bot 会话没有隐含项目根，必须提供绝对路径或工具支持的显式绝对 `folder`。一次调用不会因服务默认工作目录不同而读取另一个项目的权限规则。

路径规则继续支持 `<workspace>`、`<cwd>`、`<home>`、`<tmp>`、`<state>` 别名。快照在父进程中捕获这些别名对应的根目录，避免子进程使用私有 HOME/TMP 后改变规则含义。`<state>` 指向当前实例、当前工作区的受管状态目录；它不会扩大到实例全部凭证和其他工作区。

### 默认值与审批模式

当前全局默认值保留原行为：

| 设置 | 默认模式 |
| --- | --- |
| 工作区读取 | `allow` |
| 工作区写入 | `auto` |
| 工作区之外读取、写入 | `auto` |
| 互联网、私有网络、回环网络 | `auto` |
| `<tmp>` 和 `<state>` 对应的默认路径规则 | 读写 `allow`，可在原设置中修改 |

这些默认值属于授权策略。它们不会把整个 HOME、所有项目、共享临时目录或整个磁盘自动挂入每个沙盒。真正的子进程资源集合仍来自本次调用的具体声明、必要运行时文件和进程私有临时目录。

`ask`、`auto` 获准后，仅本次调用所声明的资源进入沙盒。某个目录获准以后，它内部更具体的秘密目录拒绝、只读规则和仍需单独授权的规则继续生效。

### 权限上限与已保存规则

会话权限、子代理的父级限制、非管理员 channel 来源的权限围栏按原有规则组合。所有独立上限相交：后加入的宽松配置不能抹掉前一个上限的拒绝。

持久规则按调用所属的 `global → workspace → session` 作用域解析，每个作用域与动作键只采用最新、未撤销的记录。快照中的持久规则同样遵守原流水线：静态 `deny` 不能被记住的 `allow` 升级；静态 `allow` 不被记住的 `ask`、`auto` 降级；有效的持久 `deny` 仍限制资源。

快照在普通模型调用、工具内委托、会话 hook 委托和既有 operator 执行入口绑定正确作用域。切换会话时会清除旧调用的持久快照，再从新会话加载。已有明确用户/operator 入口继续保留自己的授权语义。

## 路径设置示例

下面的项目配置把 `input` 设为只读输入，把 `output` 设为读取允许、写入需要审批的输出目录，并拒绝访问 `secrets`。规则同时写目录本身和目录内容，是因为路径规则中的目录字面量只匹配该目录；`/**` 才表达完整子树。

```json
{
  "permission": {
    "path": {
      "workspace": { "read": "deny", "write": "deny" },
      "external": { "read": "deny", "write": "deny" },
      "rules": {
        "<workspace>/input": { "read": "allow", "write": "deny" },
        "<workspace>/input/**": { "read": "allow", "write": "deny" },
        "<workspace>/output": { "read": "allow", "write": "ask" },
        "<workspace>/output/**": { "read": "allow", "write": "ask" },
        "<workspace>/secrets": { "read": "deny", "write": "deny" },
        "<workspace>/secrets/**": { "read": "deny", "write": "deny" }
      }
    },
    "network": {
      "internet": "deny",
      "private": "deny",
      "loopback": "deny"
    }
  }
}
```

项目覆盖仍与原有全局规则合并，例如全局 `<tmp>`、`<state>` 规则；需要调整时在现有设置中修改同名规则。

已有 `input`、`output` 目录时，Linux/macOS 的 shell 调用可以这样声明资源：

```json
{
  "command": "cat ../input/data.txt > result.txt",
  "workdir": "/path/to/project/output",
  "reads": ["../input"],
  "writes": ["."],
  "network": []
}
```

Windows 使用相应 shell 命令和实际路径，路径与网络权限字段保持相同结构。`workdir` 本身需要读取授权；使用输出目录作为工作目录，可以避免为整个项目根申请读取权限。

### 文件系统投影规则

- 只读声明转成只读资源；写入声明转成读写资源。MXC 的读写资源包含读取能力，因此 shell 写入路径会同时经过读取审批。
- 读写父目录中的更具体只读子目录保持只读；读取拒绝路径进入明确拒绝集合，即使该路径也可能由平台运行时基线或 ACL 暴露。
- 目录根的字面量 `allow` 不会自动批准其默认被拒绝的子树。需要目录内容时，使用匹配完整子树的规则并声明相应目录。
- 文件名级负向通配规则，例如 `**/*.key`、`secret?.txt`，不能通过枚举当前文件安全地转换为 MXC 的固定路径策略：枚举会遗漏之后创建的文件。这类无法表达的限制返回 `Unsupported`，不会忽略规则继续启动。
- “拒绝父目录、另行允许其子目录”的重叠在各后端存在差异。不能安全投影的调用会被拒绝，应收窄获准目录或调整为无歧义的子树边界。
- shell 的读写挂载必须已经存在。新建输出文件时声明现有输出目录，或先通过 `fs.write` 创建准确目标；执行器不会为了支持一个不存在的输出文件，自动把它的父目录授予写入权限。
- 共享临时目录根（例如 `/tmp`、`/var/tmp` 及其祖先）不能作为工具资源授权；需要临时文件时，使用自己的具体任务子目录。`agena-sandbox-*`、`agena-tool-worker-*`、`agena-media-worker-*` 是保留的私有目录，其他调用不能通过直接路径或符号链接授权读取。这个检查在宿主复制输入之前执行，防止绕过不同会话的权限边界。

平台运行命令还需要系统库、程序文件和所选工具的安装文件。launcher 只加入必要的运行时路径；工具安装位置如果落入显式拒绝路径，启动会失败。API key、凭证环境变量和用户整套配置目录不会因启动命令而整体继承。

自动加入的 PATH 目录只限系统程序目录。用户安装的程序默认只获得准确可执行文件；识别到的 Python 标准库、虚拟环境子树可作为必要运行时加入。其他依赖须由受信任的运行时配置声明 `runtime_paths`，不会自动开放整个 `~/.local/share`、用户 `lib` 或 `include`。

显式设置 `LD_*`、`DYLD_*`、`GLIBC_TUNABLES`、`GCONV_PATH`、`LIBPATH` 或 `SHLIB_PATH` 会在启动前报错，错误只包含变量名。工作负载的 PATH 等环境值通过私有启动数据传递，外层 launcher 始终使用宿主受信任环境；配置显式提供的普通业务凭据在隔离进程内恢复。

私有临时目录的 `private_paths` 表示由宿主创建并持有的 scratch 所有权，本身不授予读取或写入。共享临时目录根，以及其他任务的 `agena-sandbox-`、`agena-tool-worker-`、`agena-media-worker-` 保留目录均不能通过普通资源声明开放；校验同时覆盖原路径、规范路径别名及 Windows 大小写变体。

## 网络设置与执行

`permission.network.internet`、`private`、`loopback` 及 `rules` 继续决定目标授权。模型提供的 `network` 只声明本次调用需要哪些具体主机、IP 或 URL，不能设置策略开关。

### 离线执行

`network: []` 选择离线策略，拒绝进程的外部 IP 网络以及宿主回环访问。需要网络的命令必须声明实际目标，并先通过原网络审批。

Linux 的私有网络命名空间仍可具有沙盒内部回环，这与访问宿主 `127.0.0.1` 是不同对象。不同后端对不可访问文件的表现也不同，例如可能得到权限错误、不可见路径或被遮蔽内容；不能依靠某一种错误文本判断隔离是否成功。

### 带网络的工具调用

当前工具层把网络声明编译为每次调用专用的认证 HTTP 代理策略：

| 声明 | 生效方式 |
| --- | --- |
| `example.com` | 允许审批后的 80、443 端口 |
| `example.com:8443` | 只请求明确的 8443 端口 |
| `https://example.com/resource` | 使用 URL 中确定的主机和端口；路径不变成任意网络开放权限 |
| 空数组 | 离线，无内部 HTTP 出口 |

宿主先解析 DNS，按当前网络快照检查数值地址，再固定代理实际连接的 IP 集合。代理不会在连接时自行重新解析成另一个目的地址。

已批准域名访问公网时，`internet: deny` 配合具体域名 `allow` 可以生效：解析后的公网 IP 继承该域名审批，同时仍受各层显式 IP、CIDR 和适用数值通配规则约束。DNS 从互联网主机转向私有或回环地址，或者从私有网络转向回环时，目标 IP 必须获得最终明确 `allow`；域名审批不能代替另一网络类别的审批。

代理支持 HTTP，以及目标一致的 TLS `CONNECT`。TLS ClientHello 的 SNI 必须与获准目标匹配，ECH 被拒绝。它不提供任意域名的原始 TCP 隧道；SSH、数据库私有协议及不经 HTTP 代理的网络程序不能因为声明了一个域名就任意直连。非标准服务端口需要显式声明。

底层 `agena-process::sandbox::SandboxNetwork::Direct` 提供数值 IP/CIDR、协议和端口策略接口，用于能够准确声明这类资源的受信任调用方。工具层的域名 HTTP 策略不会自动降级成开放网络。

### Windows 的当前网络边界

Windows 原生进程后端支持离线隔离及其能力范围内的数值 IP/CIDR 策略。当前 Agena 的认证 HTTP 代理尚未提供与 AppContainer 身份绑定的 Windows 代理入口，所以 Windows 上的 `Proxy`/`HttpProxy` 模式会明确返回 `Unsupported`。

工具 `network` 声明当前统一使用上述 `HttpProxy` 编译路径；填写一个数值 IP 也不会自动切到低层 `Direct`。Windows 上这类联网工具需要完成身份代理接入后才能使用。执行器不会以允许宿主全部回环、开放任意出口或无沙盒重试来替代该能力。

## 已接入的工具和辅助进程

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
| Playwright Python/Node 辅助进程 | 私有 HOME 和暂存区，只允许当前已归属 CDP 端点的单个 IPv4 回环端口；保留超时、取消及表单内容保护 |

PTY 和其他常驻子进程的内核权限在创建时固定。后续 `shell.write` 即使获准发送新的命令，也只能在这个边界内执行；需要新增目录或网络目标时，应带着新的资源声明创建终端或进程。修改设置不会给已经运行的进程追加挂载或网络权限。

缺失目标程序、必要运行时文件、沙盒后端或联网依赖时，调用会返回具体启动错误。执行器不会转为宿主直跑，也不会通过放宽目录或网络策略重试。

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

MCP 配置位于 `plugins.list["agena.mcp"]`。该插件的 `settings.servers.<name>.process` 沿用 `command`、`args`、`env`、`cwd`，增加 `reads`、`writes`、`network` 资源声明。省略三组声明时，服务仅获得工作目录的读取权限，网络关闭。资源的允许、询问和拒绝继续由既有路径、网络、项目设置、全部 ceiling 以及当前持久规则决定。

例如，既有 MCP 连接可以声明输入、输出和准确网络目标：

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

### 外部 stdio 插件

`plugins.list["<plugin-id>"].package` 在 `kind: "stdio"` 时支持 `reads`、`writes`、`network` 声明，默认均为空，授权仍复用现有设置。bootstrap 使用独立空工作目录，只读开放已配置插件包、明确解释器入口及必要运行时，网络关闭；它负责 manifest、初始化和关闭等基础生命周期。

其他工具、流式工具、hooks 和 services 必须取得当前有效的宿主授权作用域。实例按 scope、工作区、工作目录及完整策略隔离，初始化继承当前会话上下文；动态工具注册进入对应会话 overlay，并随进程 effect scope 清理。子进程回调只能使用该进程当前活跃请求收到的 token，同时匹配其会话、工作区和适用的调用边界。

一次性授权的实例在宿主调用完成后移除并关闭。scope 实例退出后，下一次合法调用会重新求取当前策略再创建进程；不会在后台脱离原授权重新启动。没有可信作用域的非基础生命周期返回明确错误，bootstrap 不承载会话工具调用。

## 进程内插件和浏览器进程

cdylib 插件与 Agena 宿主在同一进程中运行，必须作为受信任代码配置；对子进程的 MXC 启动包装不能隔离已载入的动态库。

浏览器层已隔离 Playwright 辅助进程。Agena 自行管理的 Chrome 目前仍使用原有共享服务进程；已经运行的外部浏览器/CDP 会话也保留其原进程权限。接入端继续执行既有目标授权和归属检查。

为这些入口建立相同的内核边界，需要将动态库迁移到受限工作进程，并将浏览器服务改为按会话权限创建独立进程/profile、控制通道与网络策略。

## 平台要求与当前部署状态

| 平台 | 原生后端 | 部署要求与当前功能 |
| --- | --- | --- |
| Linux | Bubblewrap | 需要可用 `bwrap` 和允许创建所需 user/mount/network namespace 的系统策略；联网模式还需 `slirp4netns` 及后端要求的网络工具 |
| macOS | Seatbelt | 使用 MXC 支持的 macOS 版本；MXC 1.1.0 的 macOS 基线为 macOS 15；离线和受限代理能力由本机后端检查 |
| Windows | Process Container/AppContainer | 需要包含相应 Process Container 能力的 Windows 版本；路径拒绝依赖对应文件系统拒绝能力；离线与低层数值 IP 策略按后端能力检查，当前认证 HTTP 代理模式返回不支持 |

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

准备阶段失败时旧服务保持运行；新程序复制到最终路径后必须通过 `--agena-sandbox-check`，否则回滚程序和原运行状态。Linux/macOS 要求离线隔离和认证 HTTP 代理自检都成功；Windows 必须通过离线隔离检查，并单独显示当前 MXC HTTP 代理模式不支持。成功报告保存在 `<install-dir>/sandbox-report.json`。

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

该入口在加载配置、凭证和插件之前执行，输出单行 JSON。`schema_version` 为 `1`，`ok` 与退出码一致，`checks` 分别列出 `offline` 和 `http_proxy` 的 `required`、`status`、`detail`。检查使用私有临时目录和本地 HTTP 测试端点，完成后清理；不连接外部测试网站。AppArmor 按程序路径匹配，诊断时应直接使用最终安装路径的二进制。

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
| Windows 提示 identity-bound AppContainer proxy 不可用 | 当前认证 HTTP 代理未接入该能力；保持离线或使用明确支持的受信任低层数值 IP 策略，不能改成无沙盒重试 |
| `existing writable mount` | 声明现有输出目录，或先用 `fs.write` 创建准确文件；不自动扩大到父目录 |
| 路径规则 `Unsupported` | 检查负向文件名通配、字面量目录与子树默认值、拒绝父目录中的子授权；使用可表达的明确目录边界 |
| `ReadApprovalRequired` | 读写挂载会包含读取能力，需要补足该写入目标的读取授权 |
| 域名没有可用的获准 IP | 检查显式 IP/CIDR 拒绝，以及 DNS 是否指向需要单独允许的私有/回环地址 |
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
| `agena-process/src/sandbox_proxy.rs` | 每次调用的认证网络代理 |
| `agena-lsp/src/registry.rs` | 按完整权限策略区分语言服务实例 |
