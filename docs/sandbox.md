# 执行保护：原生沙盒与资源预检

Agena 默认使用 `mxc-sdk` 的原生进程沙盒执行模型触发的本地工作和受控网络请求。路径权限放在 `permission.path`，网络权限放在 `permission.network`，工具审批放在 `permission.tools`；Shell、托管子进程和各类工作进程共用当前调用所属身份的生效策略。Agena 作为 MCP 服务接收的调用会强制关闭 MXC，按 `resource_guard` 决定是否启用资源预检；Agena 工具访问外部 MCP 服务则遵循普通会话的沙盒设置。

路径规则的值只接受 `readonly`、`readwrite`、`deny`。它们分别对应 MXC 的只读、读写和拒绝路径。`ask`、`auto` 属于 Agena 的工具审批流程；路径与网络资源权限必须是能够执行的具体模式。

## 开箱即用的权限

没有权限覆盖时，项目会话默认可以读取、创建、修改和删除当前项目中的文件，并允许出站网络连接。MXC 默认开启；普通工具在这些资源边界内直接执行，已有的危险命令拒绝规则继续生效。项目外的业务路径需要在设置中明确授权。

默认的主要配置可以写成：

```json
{
  "sandbox": true,
  "resource_guard": true,
  "permission": {
    "path": {
      "workspace": "readwrite",
      "external": "deny"
    },
    "network": {
      "default": "allow"
    }
  }
}
```

省略这些字段即可使用内置值。全局、项目和会话中已保存的覆盖仍按原有继承规则生效；例如显式的 `readonly`、`deny` 或离线网络不会因升级而改变。Web 和 TUI 的全局设置直接编辑值，并提供恢复默认操作；工作区与会话显示“继承／本层”。缺失的全局字段自动补齐默认值。

## 在 Web 和 TUI 中配置访问权限

Web 的“权限与安全 → 访问权限”集中展示文件、网络和工具策略。宽栏并列显示资源权限和工具权限，窄栏按顺序堆叠。路径、网络、工具名称和终端命令各有一张规则表，提供独立的搜索、新增和批量操作。顶部工具栏集中显示作用域、实际配置对象、保存状态和操作。已有规则直接显示，新增表单和详细说明按需展开。

TUI 从“设置 → 权限”打开统一的访问权限编辑器。首页先展示文件的两个默认项、网络的三个默认项和工具的两个默认项，随后按分组列出规则；每行直接显示配置值及其来源，各分区按实际内容计算列宽，短规则的模式与来源紧跟规则文本。来源操作的鼠标命中区域与显示位置一致；长规则列表不会把其他类别的默认策略推到末尾。

- 文件分区包含默认路径区域和自定义路径规则。
- 网络分区包含继承方式、默认动作、允许和拒绝目标，以及协议、端口等规则内容。
- 工具分区包含默认审批方式、工具名称规则和终端共用的命令规则。

文件、网络和工具共用作用域选择和权限模型：可以选择全局、工作区或当前会话进行修改，也可以只读查看当前生效的权限。Web 可以同时修改多个分区，再通过统一保存栏提交当前作用域的完整草稿；TUI 沿用逐项或逐条规则确认保存，每次提交都会保留其他分区的配置。切换作用域、目标会话或离开页面时会检查尚未保存的修改。已记住审批规则在独立区域按各条规则单独保存，从规则列表返回时会恢复访问权限页的草稿和位置。

默认项将来源与值分开显示：全局直接使用下拉框编辑实际权限值，旁边提供恢复默认按钮；工作区与会话在值旁选择“继承／本层”。继承时直接显示父层来源和实际值，值控件置灰且不可编辑；明确点击“本层”后才能编辑，同时固定当前继承值，即使它与上层相同，也会保存明确覆盖。下层点击“继承”移除对应配置并随上层变化；全局点击恢复默认只删除对应字段或明确范围的配置组。只读生效视图展示合并结果。只读工具类别的 `auto` 表示跟随工具默认策略，界面同时显示实际值与跟随关系，固定为本层时保留 `auto` 的关系。

全局内置规则可直接编辑，规则表不显示来源列；已修改规则提供恢复默认动作。下层继承规则可以通过“在本层设置”建立覆盖。路径和指定工具按单项设置与恢复；网络的出站、入站、宿主回环、允许列表、拒绝列表各自设置与恢复；切换“本层”只固定对应单项或列表，继承列表必须先切换来源才能新增、编辑或删除。所有终端共用整张命令规则表，恢复继承前会确认清除此层的命令覆盖。

TUI 在全局设置行按 `Enter` 直接编辑，按 `Ctrl+O` 发起恢复默认确认；工作区与会话仍用 `Ctrl+O` 打开“继承／本层”选择框，继承值置灰，`Enter`、空格和点击值不会直接创建覆盖。来源操作与 Web 使用相同的单项、独立网络列表和共用命令规则表，未保存的网络表单不会被来源切换覆盖。

旧的文件、网络、工具和沙盒链接会定位到统一访问权限页的对应分区，旧总览链接会打开生效权限作用域。全局和工作区作用域显示服务端当前读写的配置文件路径；当前会话和生效权限的工具栏通过通用会话下拉框选择目标，支持目录搜索和分页。默认目标是最近使用的会话，明确选择后随设置窗口保留；权限读取、保存及生效审批规则均跟随所选会话及其工作区。设置目标独立于正在聊天的会话和最近会话列表。

## 沙盒与资源预检

Web 的“权限与安全 → 访问权限”在作用域工具栏下方首先展示独立的“沙盒与资源预检”卡片。这里和 TUI 的“设置 → 权限”均提供两个独立开关，支持全局默认和工作区覆盖。全局开关直接保存配置值，恢复默认按钮清除该字段；工作区来源选择控制是否继承。卡片标题旁显示服务端当前的执行模式。

```json
{
  "sandbox": true,
  "resource_guard": true
}
```

`sandbox` 和 `resource_guard` 是两个独立的顶层布尔开关，均默认为 `true`。新配置直接使用 `true` 或 `false`，删除当前层字段即可恢复继承。

| 配置             | 策略与行为                                                                                                 |
| ---------------- | ---------------------------------------------------------------------------------------------------------- |
| `sandbox`        | 原生沙盒：通过 MXC 约束 Shell、PTY、托管子进程，以及文件、HTTP、媒体、技能和记忆等工作进程的实际 I/O       |
| `resource_guard` | 资源预检：在未使用 MXC 时，统一检查工具声明的 `reads`、`writes`、`network`，并根据命令分析提示补齐遗漏声明 |

资源预检发生在调用前，依赖工具声明和命令分析。它适合直接执行时做基础检查；原生沙盒还会约束进程随后实际发生的 I/O。

| 本次调用实际使用 MXC | `resource_guard` | 执行方式                           |
| -------------------- | ---------------- | ---------------------------------- |
| 是                   | `true`           | MXC 隔离，资源预检待用             |
| 是                   | `false`          | MXC 隔离，资源预检关闭             |
| 否                   | `true`           | 资源声明校验与命令补充提示一起启用 |
| 否                   | `false`          | 两项预检一起关闭                   |

启用 MXC 时，模型可以省略上述三个资源声明字段；即使传入，也不会用于缩小或扩大沙盒权限。网络是否可用由 `permission.network` 决定，省略工具的 `network` 参数不再意味着离线。工具定义会标明当前执行模式及声明要求。

只使用资源预检：

```json
{
  "sandbox": false,
  "resource_guard": true
}
```

关闭这两种执行保护：

```json
{
  "sandbox": false,
  "resource_guard": false
}
```

两个开关共用 `permission.path`、`permission.network`，不需要另写一套 worker 权限。`permission.tools` 仍由 Agena 处理工具审批；文件发布、HTTP 目标等真实资源授权也继续生效。资源预检检查已声明的路径和出站目标，不能替代 MXC 对进程实际连接、入站或宿主回环访问的约束。MXC 启动失败返回错误；需要直接执行时，在调用前显式关闭沙盒。

### 旧配置兼容

旧的 `"sandbox": { "enabled": true }` 会规范化为 `"sandbox": true`，`false` 同样保留。没有 `enabled` 的旧对象和旧的 `sandbox: null` 不产生当前层覆盖，继续继承上层值。

读取旧配置时，`sandbox.resource_checks` 映射到顶层 `resource_guard`。旧 `sandbox.command_hints` 合并到同一预检策略，不再提供单独开关；只有该旧字段时不会产生新的覆盖值。这样，旧的 `resource_checks: false` 会继续关闭预检，旧的 `resource_checks: true` 会统一启用声明检查和命令提示。

`resource_guard` 与 `sandbox.resource_checks` 同时出现且值不一致时，配置会报冲突；相同值只保留新字段。设置读取结果使用新结构，下一次正常保存时写入规范化的新结构。

### 环境变量与 MCP 调用

启动服务时可以用环境变量强制关闭 MXC：

```sh
AGENA_SANDBOX=disabled agena server --workspace /path/to/project
AGENA_SANDBOX=disabled agena server start --workspace /path/to/project
```

`false`、`0`、`off` 也表示关闭。未设置或设置为 `enabled` 时遵循配置的 `sandbox` 布尔值，不会重新开启已在设置中关闭的沙盒。兼容旧的 `AGENA_SHELL_SANDBOX=disabled`；存在 `AGENA_SANDBOX` 时以它为准。环境在进程启动时捕获，修改后需要重启服务；工具 `env` 中的同名值不能改变运行模式。

`agena mcp-server` 是常驻服务的 stdio 桥接客户端，工具实际在服务端执行。桥接调用与 HTTP `/mcp` 调用在服务端为本次执行关闭沙盒，无须额外环境变量；这类调用按 `resource_guard` 决定是否启用资源预检。这个受信任标记跟随调用执行器与宿主回调，不改变普通会话的模式，也不会复用另一模式下的 LSP、MCP 子连接或插件进程。

关闭后，Shell、PTY、文件工具和辅助子进程直接执行，不要求 MXC 后端或沙盒挂载。命令保留宿主环境及调用方的显式环境设置。

## 启用沙盒时的执行流程

1. 保存全局、项目或会话设置时，校验语法和合并后的策略；启用沙盒时同时编译完整路径边界。无法表示的配置返回具体错误，不写入设置。
2. 工具参数和 hook 转换完成后，按当前项目、会话和来源身份处理工具及命令审批。
3. `SandboxPermissionSnapshot` 捕获生效的路径、网络策略，独立权限上限，以及本次调用可见的持久规则。
4. Shell、PTY 和托管子进程直接从快照生成只读、读写、拒绝路径和完整网络策略，不要求模型重复声明资源。宿主控制的文件 worker 和固定辅助工具继续使用任务实际需要的最小资源范围。
5. 统一 launcher 启动 MXC，再启动命令或 worker。后端和主机依赖无法落实策略时返回启动错误。
6. 文件编辑和 Notebook worker 在私有暂存区生成结果，宿主验证修订和目标身份后发布到固定目标。技能与记忆沿用各自的修订检查、锁和事务，在获准目录内执行；取消技能写入时，工作进程退出后才释放宿主写锁。

工具参数不能切换沙盒。模式由已保存的配置、服务启动环境与受信任 MCP 宿主入口共同决定。

## 路径权限

| 配置值      | 授予的权限                             | MXC 对应字段     |
| ----------- | -------------------------------------- | ---------------- |
| `readonly`  | 读取文件或目录树，禁止修改             | `readonlyPaths`  |
| `readwrite` | 读取和修改；目录内可创建、删除、重命名 | `readwritePaths` |
| `deny`      | 禁止读取和写入                         | `deniedPaths`    |

目录规则覆盖目录及其完整子树，文件规则只作用于具体文件。不再分别填写 `read`、`write`，也不需要同时写目录本身和 `/**`。规则顺序不构成授权优先级，调整 JSON 条目顺序不会扩大资源权限。

### 工作区与外部路径的默认模式

`workspace` 和 `external` 都可以独立选择 `deny`、`readonly`、`readwrite`，九种默认组合都支持。`workspace` 决定所属工作区内的默认权限，`external` 决定工作区之外的默认权限，具体路径规则再细化各个文件和目录。

例如，下面的配置允许修改工作区，工作区之外默认只读，可以直接在全局或工作区设置中保存：

```json
{
  "permission": {
    "path": {
      "workspace": "readwrite",
      "external": "readonly"
    }
  }
}
```

具体规则可以覆盖普通的只读、读写默认值：可写工作区中的 `"<workspace>/reference": "readonly"` 会保护参考资料；进一步设置 `"<workspace>/reference/generated": "readwrite"` 可以允许修改该子目录。只读工作区中单独开放可写输出目录也有效。

显式 `deny` 是不可重新开放的边界。例如 `"<workspace>/secrets": "deny"` 下面再设置允许读取或写入的子路径会拒绝保存。独立权限上限仍与最终规则取交集。

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
      "default": "deny"
    },
    "tools": {
      "names": {
        "agena.shell.exec": "ask"
      }
    }
  }
}
```

`workspace: "deny"` 表示未被具体规则授权的工作区路径默认拒绝，仍可为 `input` 授予读取、为 `output` 授予读写。支持显式路径拒绝的后端还可以设置 `"<workspace>/secrets": "deny"`，用于封锁已有授权范围下面的秘密子树。

路径的项目覆盖仍与全局设置合并，空的路径 `rules` 不会清空已经继承的路径规则。保存时必须验证合并后的生效配置，不能用项目中的宽松项绕过独立的权限上限。

### 终端命令规则

所有终端的命令匹配使用同一张表 `permission.tools.rules."shell.*"`。内置规则允许 `no-op` 和 `routine`，拒绝 `dangerous`。在表内添加 `git push *` 等命令模式时，匹配的最后一条模式决定命令权限。向已打开的终端写入原始输入时，匹配的拒绝或确认规则仍然生效，写入本身也受工具访问策略约束。

读取旧配置时，Agena 会把同一配置层内按终端工具保存的命令表合并成共享表。单张表保留原有模式顺序。不同旧表中的规则冲突时，按拒绝、确认、自动审批、允许的顺序取更严格的决定。下一次保存设置时会写入共享格式。若本层已经声明共享表，则直接采用共享表。

### 默认值和路径别名

| 设置                     | 默认模式    |
| ------------------------ | ----------- |
| 工作区路径               | `readwrite` |
| 工作区之外的业务路径     | `deny`      |
| `<state>` 的具体目录规则 | `readwrite` |
| 沙盒私有临时目录         | `readwrite` |
| `<tmp>` 宿主临时目录根   | 按外部路径默认拒绝 |
| `<workspace>/.agena/agena.json` | `readonly` |
| Git 的 `config`、`config.worktree`、`hooks/` 和 worktree 指针文件 | `readonly` |
| Agena 实例配置、SQLite 数据库及边车文件、凭据和其他工作区状态 | `deny` |
| 所有平台的出站网络       | `allow`     |
| 入站和宿主回环网络       | `deny`      |

launcher 会把宽泛的默认授权范围沿共享临时目录、Linux 内核与控制目录，以及可信 launcher/后端的写保护边界分解为具体挂载。这样可以落实工作区与外部路径的默认模式，同时保留沙盒运行所需的隔离和保护；读写挂载始终限定在已授权范围内。

宽泛默认值只展开当前宿主用户实际可访问的资源；无权读取的兄弟目录和失效链接不会使整个沙盒启动失败。宿主只读父目录中已有的可写子树仍可单独挂载。可信程序的安装目录及其祖先目录保留只读保护，因此不能在这些祖先目录中直接新建、删除或重命名目录项；普通已授权子目录保持自己的访问模式。将 Agena 安装在独立程序目录中；若把可执行文件直接放在任务工作区根目录，该安装目录也会受到写保护。

MXC 把 `TMPDIR`、`TMP`、`TEMP` 指向本次启动的可写临时目录。Linux 的 Bubblewrap 后端还在沙盒内提供私有的 `/tmp`。需要处理普通宿主临时文件时，显式授权自己的具体目录，例如 `"<tmp>/my-task": "readwrite"`。即使该规则与 `external` 的默认模式相同，也会保留这项明确授权，避免在分解默认范围时丢失。

项目设置文件由受信任的设置入口写入。首次启动具有工作区写权限的沙盒时，宿主会准备实体目录 `.agena/` 作为只读挂载点，保护设置文件的创建和替换。已有的可写子目录仍按各自的权限挂载。项目设置和实例私有数据的保护由宿主生成。

Git 的默认保护按实际仓库及 worktree 元数据位置解析。文件工具检查这些路径上的创建和修改，原生沙盒为已存在的控制路径设置只读边界。已授权的索引和引用目录仍可写。需要编辑 Git 配置时，可为具体的 Git 路径保存显式的 `readwrite` 规则。

首次执行时，Agena 会初始化已获准读写的 `<state>` 托管目录；用户自定义的授权文件和目录仍需预先创建。

路径规则保留 `<workspace>`、`<cwd>`、`<home>`、`<tmp>`、`<state>` 别名。路径设置中的 `<cwd>` 规范为所属工作区；回退声明检查中的相对路径按本次调用实际工作目录解析。`<state>` 只指当前实例、当前工作区的受管状态目录。

全局或 Bot 会话没有隐含项目根，`<workspace>`、`<state>` 模板在该作用域内不展开；通过 `external` 默认模式、绝对路径规则或工具支持的显式绝对 `folder` 定义资源范围。Shell 的资源集合来自当前配置的默认模式与具体规则、运行时依赖和私有临时目录。

### 保存校验与旧配置迁移

- 路径模式必须是上述三个字符串。只写不读、资源模式中的 `ask`/`auto`、不完整的旧 `read`/`write` 对象都会报错。
- 使用具体文件或目录。`**/*.key`、`secret?.txt` 等普通通配模式、未知别名、控制字符和含 `..` 的路径组件不能保存。
- 明确拒绝父目录后重新开放子路径会报错；不能用排序调整绕过拒绝。
- 旧 `workspace`、`external` 中完整的三种确定读写组合可以转换为字符串。
- 旧正向路径规则对象只有在目录本身与同模式 `/**` 规则完整配对时，才会自动合并为一条递归目录规则。未配对的旧正向对象要求用户明确改成字符串，以确认新的路径范围；不根据扩展名猜测文件或目录。旧完整拒绝对象可以转成递归 `deny`，只会收紧访问。
- 旧 `read: allow, write: ask` 不能自动变成 `readwrite`。先把写入审批迁移到合适的工具或任务审批，再明确选择资源权限。
- TUI/Web 显示保存错误并保留可修正的编辑内容。校验失败不更新原有设置。
- 保存要求重新加载时，即使文件内容没有变化，也会重试运行时重载。因此，写盘成功但重载失败后，可以直接再次保存相同配置；错误不会被“内容未变化”掩盖。
- 校验当前宿主的已知后端能力。macOS 拒绝无法等价简化的细粒度出站规则，Linux 拒绝允许入站或宿主回环的设置。如果所选 Windows 后端不支持 `FileSystemDeny`，明确的路径 `deny` 规则不能保存。

路径和主机状态可能在保存后改变；启动前仍需核对真实路径、符号链接、当前权限及后端能力。保存成功表示规则通过当前配置与已知能力校验，不代替在目标系统上启动沙盒。

### 单文件与输出目录

已有文件可以单独授予 `readwrite`，不会因此自动开放父目录。创建多个产物时，使用单独的输出目录。

使用前述配置并已创建 `input`、`output` 目录时，Linux/macOS 的 Shell 调用可以直接写为：

```json
{
  "command": "cat ../input/data.txt > result.txt",
  "workdir": "/path/to/project/output"
}
```

Windows 使用对应 Shell 命令和实际路径。`workdir` 本身需要读取授权；未授权的项目根不能作为工作目录。

- 配置中的挂载目标必须存在。新文件可以放在已授权且已存在的输出目录中，或先通过 `fs.write` 创建准确目标；执行器不会自动把父目录扩大为读写授权。
- 文件的原子替换可能需要同目录临时文件和父目录操作。Agena 文件修改工具通过私有暂存区和固定目标提交处理。
- 不能显式整体开放共享临时根（例如 `/tmp`、`/var/tmp`）。授权这些目录的祖先或设置宽泛的 `external` 默认值时，launcher 会排除共享临时目录；可以另行授权自己使用的具体临时子目录。其他任务的 `agena-sandbox-*`、`agena-tool-worker-*`、`agena-media-worker-*`、`agena-native-worker-*` 保留目录继续受保护。
- `external: "readonly"`、`"readwrite"` 均可在启用 MXC 时保存。launcher 按上述隔离边界分解实际挂载；工作区仍按 `workspace` 和具体路径规则执行。

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

持久规则按调用所属的 `global → workspace → session` 作用域解析，每个作用域与动作键采用最新且未撤销的记录。明确拒绝不能被记住的批准升级。这些决定会进入本次配置快照，编译为 MXC 策略。

持久 `NetworkAccess` 只接受 `allow`、`deny` 和数值 IP 目标，例如 `192.0.2.10:443`、`[2001:db8::10]:443`、`udp://192.0.2.10:53` 或 `icmp://192.0.2.10`。裸 IP 匹配该地址的全部协议/端口，允许决定仍受配置范围限制；CIDR 范围放在 `permission.network` 中设置。域名、`ask`/`auto`、含路径或凭据的 URL 无法保存。macOS 的后端不支持这种精确 IP 决定，保存时会明确报错。历史记录继续展示并可撤销或删除；编辑后保存同样执行新校验。

快照按调用所属会话绑定工作区和来源身份。长期进程的权限在创建时固定；改变设置后，新进程使用新策略。已有交互终端的模式或策略不再匹配时，继续输入会返回错误，需关闭并重新打开；输出读取和清理仍可使用。

设置页的全局和工作区编辑器显示该层实际保存的配置；“生效”视图显示所标明会话的合并权限，没有选定会话时显示运行时配置。生效视图会包含继承的内置规则，例如 `<state>: "readwrite"`，所以规则数量可以多于全局编辑器。会话的明确覆盖仍参与合并。

## 网络设置与执行

`permission.network` 使用扁平规则，由 Agena 转换为 MXC 的原生网络策略。配置描述数值 IP/CIDR、协议和端口，进程通过原生 socket 联网，不启动沙盒权限代理。

保存和执行都先校验完整输入，再作严格等价简化：默认允许时删除没有额外作用的 `allow`；默认拒绝且没有允许项时删除冗余的 `deny`；未限制协议和端口的 `to: "*"` 可以折叠为默认允许或默认拒绝，仍保持 `deny` 优先。带协议、端口或网段限制的规则保留这些限制。编辑器保留用户写入的结构，平台能力和原生规则数量限制按实际生效的策略检查。

因此，macOS 也可以保存最终等价于全开或全关的规则写法。例如 `{"allow":[{"to":"*"}]}` 会转换为出站默认允许；确实需要按 IP、协议或端口筛选的策略仍会明确报错。

### 常用配置

完全关闭当前层的网络：

```json
{
  "permission": {
    "network": {
      "version": 2,
      "default": "deny",
      "inbound": "deny",
      "hostLoopback": "deny",
      "allow": [],
      "deny": []
    }
  }
}
```

允许全部出站连接：

```json
{
  "permission": {
    "network": {
      "version": 2,
      "default": "allow",
      "inbound": "deny",
      "hostLoopback": "deny",
      "allow": [],
      "deny": []
    }
  }
}
```

以上两种基础配置适用于 Linux、Windows 和 macOS；入站及宿主回环保持默认拒绝。Linux 和 Windows 可以进一步限制目的地址、协议和端口。例如只允许一个地址的 HTTPS：

```json
{
  "permission": {
    "network": {
      "version": 2,
      "default": "deny",
      "inbound": "deny",
      "hostLoopback": "deny",
      "allow": [
        {
          "to": "192.0.2.10",
          "protocol": "tcp",
          "ports": [443]
        }
      ],
      "deny": []
    }
  }
}
```

新版示例使用 `version: 2`，每个省略字段继续使用基础设置。只允许特定目标时应同时明确设置 `default: "deny"`；完全省略 `network` 则使用整份基础策略，内置策略允许出站。上例的 IP 应替换成实际目标；使用域名的程序还需要获准访问实际 DNS 服务器。macOS 当前后端不能执行细粒度出站列表，保存这类配置会报错。

### 字段与匹配方式

| 字段           | 写法                  | 含义                                                                  |
| -------------- | --------------------- | --------------------------------------------------------------------- |
| `version`      | `2`                   | 使用逐项覆盖格式；无版本对象按旧完整策略兼容                          |
| `default`      | `"deny"` 或 `"allow"` | 没有命中出站规则时的动作；v2 省略时使用基础值，内置允许               |
| `allow`        | 规则数组              | 放行匹配的出站连接；省略时使用基础列表，`[]` 明确清空                 |
| `deny`         | 规则数组              | 拒绝匹配的出站连接，优先于 `allow`；省略时使用基础列表，`[]` 明确清空 |
| `inbound`      | `"deny"` 或 `"allow"` | 入站默认动作；省略时使用基础值，内置拒绝，受平台能力限制              |
| `hostLoopback` | `"deny"` 或 `"allow"` | 宿主回环策略；省略时使用基础值，内置拒绝，受平台能力限制              |

每条 `allow` / `deny` 规则由以下字段组成：

| 字段       | 写法                                                        | 含义                                           |
| ---------- | ----------------------------------------------------------- | ---------------------------------------------- |
| `to`       | `"192.0.2.10"`、`"10.20.0.0/16"`、`"2001:db8::10"` 或 `"*"` | 必填；单个 IP、网段或所有目标                  |
| `protocol` | `"tcp"`、`"udp"`、`"icmp"`、`"any"`                         | 默认 `any`                                     |
| `ports`    | `[443, "8000-8099"]`                                        | 端口或包含两端的范围；省略表示全部端口         |
| `except`   | `["10.20.10.0/24"]`                                         | 从当前目标网段排除子网；省略或空数组表示无排除 |

同一条规则的目标、协议、端口条件必须同时满足。数组中的规则是“或”的关系；拒绝优先，与条目顺序无关。需要放行多个地址或不同协议时，每个地址、协议各写一条规则。

`except` 只影响所在规则。其他允许规则仍可放行该子网；需要始终禁止的范围应放进 `deny`。

### 完整配置示例

```json
{
  "permission": {
    "network": {
      "version": 2,
      "default": "deny",
      "inbound": "deny",
      "hostLoopback": "deny",
      "allow": [
        {
          "to": "10.20.0.0/16",
          "protocol": "tcp",
          "ports": [443, "8000-8099"],
          "except": ["10.20.10.0/24"]
        },
        {
          "to": "fd20:20::/48",
          "protocol": "tcp",
          "ports": [443, "8000-8099"]
        },
        {
          "to": "192.0.2.53",
          "protocol": "udp",
          "ports": [53]
        },
        {
          "to": "192.0.2.53",
          "protocol": "tcp",
          "ports": [53]
        }
      ],
      "deny": [
        {
          "to": "10.20.66.0/24"
        },
        {
          "to": "10.20.0.0/16",
          "protocol": "tcp",
          "ports": ["8080-8089"]
        },
        {
          "to": "*",
          "protocol": "tcp",
          "ports": [25]
        }
      ]
    }
  }
}
```

示例 IP 应换成实际业务目标与 DNS 服务器。IPv4 和 IPv6 单地址可以直接填写，转换时分别补为 `/32` 和 `/128`。网段主机位必须为零，例如 `192.0.2.0/24`，不能写 `192.0.2.7/24`；排除网段同样要求规范写法且属于目标子网。

端口范围为 1～65535。`ports: []`、字符串单端口 `"443"`、倒序范围和 ICMP 端口无效。`to: "*"` 可以省略 `except` 或使用 `[]`，不能设置非空排除子网。域名、URL、未知字段、`null`、资源动作 `ask` / `auto` 都不能保存，错误会指出字段。

### 逐项继承与覆盖

网络权限在 `global → workspace → session` 之间按五个字段分别合并。全局未设置的字段使用程序默认值；工作区与会话未设置的字段使用上层值。

| 当前层内容                            | 生效方式                                                 |
| ------------------------------------- | -------------------------------------------------------- |
| 不含 `network`，或 `{"version":2}`    | 所有网络项使用基础设置                                   |
| `{"version":2,"default":"deny"}`      | 只覆盖出站默认动作；两张规则列表、入站与宿主回环继续继承 |
| `{"version":2,"inbound":"deny"}`      | 只固定入站拒绝                                           |
| `{"version":2,"allow":[…]}`           | 替换允许列表，其他四项继续继承                           |
| `{"version":2,"allow":[]}`            | 本层明确清空允许列表，拒绝列表与默认动作不变             |
| 无版本的 `{}` 或 `{"default":"deny"}` | 兼容旧语义，仍是完整离线策略                             |

删除某个字段恢复该项来源；删除整个 `network` 恢复所有网络项。显式拒绝与空列表都保留在保存结果中。完全离线需要同时拒绝三项默认动作并清空允许列表。

例如上层放行 `192.0.2.10`，当前项目的 v2 允许列表只包含 `198.51.100.20`，项目的允许列表就只有后一条；上层拒绝列表仍然继承。子代理、channel 等独立权限上限仍与最终策略取交集，不属于上述设置覆盖。

### Web 与 TUI 编辑

Web 和 TUI 的全局网络设置直接编辑三个默认动作和两张规则列表，各项分别提供恢复默认操作。工作区与会话各项显示“继承／本层”，切到本层只固定该项的当前值；直接编辑继承规则只复制对应列表。删除最后一条规则保留显式空列表，全局恢复默认或下层恢复继承时，移除对应列表的配置。离线和允许出站预设明确设置完整的五项策略。每条规则可以分别编辑目标、协议、逗号分隔的端口/范围和排除子网。高级选项保留入站、宿主回环和 JSON 编辑。

TUI 使用 `Tab` 切换面板，`Ctrl+N` 新增规则，`Enter` 编辑字段，`Ctrl+S` 保存规则，`Esc` 取消表单，`Ctrl+D` 删除选中的规则。在默认动作或列表标题按 `Ctrl+O`：全局用于恢复默认，下层用于切换来源；列表也可明确清空。未保存表单会先要求处理，防止来源切换或预设覆盖草稿。

界面验证与后端保存校验共同生效。输入有误或当前服务所在平台不支持时，显示具体错误并保留草稿；保存成功后刷新生效配置。读取不到某项的基础值时不能一键固定该值，仍可直接输入明确的本层动作。

### 旧配置升级

启动新版服务时，旧 `egress` / `ingress` 对象和无版本的扁平网络对象会转换为 v2。旧嵌套格式先按原合并方式解析；旧扁平格式补齐三项默认动作与两张列表，保留原先的完整策略效果。旧的 `network: {}` 仍是离线，迁移后五项均为明确的拒绝或空列表。

迁移先校验候选配置，再建立原文备份并原子写入。每个改写的配置文件在原目录保留 `文件名.network-v2-随机值.bak`。失败会保留或恢复原配置，已为 v2 的配置不会重复迁移。

标准设置读取也会在内存中将旧网络块投影为完整 v2，读取本身保留原文件与 revision。编辑旧块的单个字段前先规范化，删除字段即可实际恢复该项来源；首次创建网络叶字段则自动建立 v2 对象。

配置文件中的 `internet`、`private`、`loopback` 和域名规则需要改为明确的 IP/CIDR 策略。公开保存接口接收扁平结构。

### 配置网络与回退声明

启用 MXC 时，Shell 和托管子进程直接使用 `permission.network` 的完整生效策略，并与每一层独立权限上限取交集。无需填写工具的 `network` 参数，也无需使用 `["*"]` 申请已配置的范围。配置为 `{"default":"deny"}` 时离线，配置具体允许规则时可直接访问这些目标。

使用域名的命令需要策略允许实际 DNS 解析器及目标 IP。MXC 执行的是数值 IP、协议和端口边界；同一获准 IP 上的服务仍按这些边界处理。Linux 的私有网络命名空间可以保有沙盒内部回环，它与宿主机 `127.0.0.1` 是不同对象。

已保存的网络批准不能扩大静态策略，保存的拒绝会编译为原生拒绝规则。不能转换成数值 IP/CIDR 的旧域名拒绝会返回具体错误，要求修改或移除该决定。

只有关闭 MXC 且开启 `resource_guard` 时，工具 `network` 数组才作为调用前的资源声明检查，例如 `["192.0.2.10:443"]` 或 `["https://api.example.com"]`。空数组表示没有声明网络操作，不能使宿主直接执行的命令自动离线。

### 受控 HTTP 工作进程与 DNS 地址校验

普通 Web 抓取、robots.txt、HTML/API 搜索、技能归档下载、provider HTTP 和工具侧 HTTP MCP/OAuth 共用受控 HTTP 传输。宿主先授权原始 URL，再解析 DNS，逐个审核数值地址是否符合当前策略、独立权限上限和持久拒绝；没有获准地址时返回权限错误。原始 hostname 继续用于 HTTP Host 和 TLS 证书校验。

启用 MXC 时，每次 HTTP 跳转对应一个工作进程中的单次请求。实际 TCP/TLS 连接、请求发送和响应接收在工作进程内完成，只连接宿主已审核的 IP；worker 禁用自动重定向和代理，也不重新解析目标。请求和响应经进程管道流式传输，保留 multipart、SSE 和有界大文件上传；Agena 不为这些请求创建 HTTP/SOCKS 权限代理服务。

支持重定向的 Web 抓取、robots.txt、技能下载与 OAuth 由宿主审核下一跳后再启动请求。HTML 搜索只跟随已授权同源跳转，并继续固定同一批获准地址；provider 与 MCP 协议请求保留原有的禁止自动跳转策略。Cookie、User-Agent、认证头和超时由调用端明确传入；搜索在复用 Cookie 时仍为每次调用刷新权限与取消状态。响应被丢弃或所属调用结束时回收 HTTP 工作进程，即使没有继续读取 SSE 正文也会停止。

这类受控 HTTP 请求的 DNS 查询由宿主执行，审核后的目的 IP 才进入 worker。Shell 或其他程序自行在沙盒内解析域名时，仍需允许其实际 DNS 服务器。provider 实时语音同样先由宿主授权 URL 与解析结果，再由独立 worker 直接连接固定 IP。

这条校验通过宿主的 `check_resolved_network_permission` 接口执行，保留原调用的身份与权限上限；原生、HTTP 和 stdio 插件回调使用相同接口。

服务自身的 HTTP/SOCKS 路由配置与沙盒权限策略分开。当前 MXC HTTP worker 使用直连；选中的代理路由无法落实固定目的 IP 时明确报错，不会静默忽略代理或改用宿主请求。关闭沙盒时沿用原客户端路由及原有的目标地址授权要求。

### 浏览器交互与页面渲染

`agena.browser` 直接在宿主机启动带原生沙盒的 Chrome，通过本机 CDP 控制浏览器。切换 MXC 的 `sandbox` 设置时，打开的页面和人工接管状态可以继续使用。浏览器导航和受 CDP 拦截的请求按会话网络权限检查。

需要读取页面内容时，可使用普通 HTTP 抓取：

```json
{
  "url": "https://example.com",
  "render_js": false
}
```

`agena.web` 的 `render_js: true` 和搜索的 JavaScript 渲染回退要求直接执行模式。启用 MXC 时，普通 HTTP 抓取通过 MXC HTTP worker 执行。

### 各平台的原生网络能力

| 平台                       | 支持范围与保存校验                                                                            |
| -------------------------- | --------------------------------------------------------------------------------------------- |
| Linux / Bubblewrap         | 支持上述 CIDR、排除子网、协议与端口规则；`inbound` 和 `hostLoopback` 必须为 `deny`            |
| Windows / ProcessContainer | 使用原生数值 IP、协议和端口规则；运行时检查当前 Windows 版本与 ProcessContainer/PSEC 能力     |
| macOS / Seatbelt           | 支持出站默认动作，以及可等价简化为默认动作的规则；仍需按 IP、协议或端口限制目标的策略不能保存 |

Agena 不会为不支持的规则改用代理，也不会降级为不受沙盒限制的进程。

## 已接入的工具和辅助进程

下表描述启用沙盒时的执行边界；通过 Agena 入站 MCP 接口接收的调用、环境强制关闭或设置关闭沙盒的调用，按前述宿主模式与资源预检开关执行。

| 工具或执行入口                                            | 隔离方式与资源边界                                                                                                   |
| --------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `shell.exec`、`shell.spawn`、`shell.watch`                | 同一 MXC launcher；使用配置中的完整路径与网络策略，保留工作目录、超时、前台/后台输出和进程归属                       |
| `shell.open` 与 PTY 后续输入                              | 初始终端使用配置策略；后续输入检查进程归属及策略是否仍匹配，模式或权限改变后需重新打开                               |
| `fs.read`、`fs.glob`、`fs.grep`、`fs.read_many`           | 独立 worker；目录遍历和文件读取同时受获准根、细粒度路径规则与 OS 文件系统策略约束                                    |
| `fs.stat`                                                 | 普通文件在独立 worker 中查询；符号链接通过宿主已授权目标的元数据 broker 保留 `lstat` 语义，不读取链接目标内容        |
| `fs.write`、`fs.replace`、`fs.apply_patch`                | worker 在私有暂存区运行；宿主只向固定、已授权的目标发布验证后的结果                                                  |
| Notebook `edit_cell`                                      | 在文件 worker 内读取与编辑 notebook，保留修订校验；宿主向固定目标发布已验证结果                                      |
| `fs.read_media`                                           | 本地读取、文件格式识别、图片解码和像素限制在离线 worker 内；宿主验证传输大小、哈希和元数据后绑定当前模型 route       |
| LSP 子进程                                                | 根据实际项目根与会话权限构造只读项目策略，使用私有 HOME/cache；不同完整策略不会共用同一个服务进程                    |
| MCP stdio 服务                                            | 配置按需启动；发现、调用、资源、提示与重连都选择当前宿主授权的 scope 和完整策略实例；连接测试使用独立沙盒进程        |
| 外部 stdio 插件                                           | 最小离线 bootstrap 读取插件元数据；实际工具、流式调用、hooks 和 services 按当前宿主授权 scope 与完整策略选择独立进程 |
| 文档转换和提取辅助程序                                    | 外部 Python/文档转换器使用离线 MXC，仅获得已准备的有界输入副本及私有临时资源                                         |
| 固定 CLI 版本探测                                         | 原有工具名单、参数和时间/输出限制继续生效；探测进程也经统一原生沙盒启动                                              |
| `session.environment` 的 Git 探针                         | 使用当前宿主调用授权投影的只读工作区策略和离线进程；Git 信息不可用时返回诊断，不改为宿主 Git 执行                    |
| Web HTTP 抓取、robots.txt、HTML/API 搜索                  | 宿主审核 URL 与 DNS 地址；实际单跳请求及流式响应在 MXC HTTP worker 内，重定向按各工具策略逐跳处理                    |
| 工具侧 HTTP MCP 与 OAuth                                  | POST、GET/SSE、DELETE 及 OAuth 发现、注册、刷新、令牌交换进入 HTTP worker；重定向与重连继续使用当前调用授权          |
| ChatGPT / Claude / Gemini 的本地媒体分析和文件上传        | 文件读取与媒体识别在离线 worker；目标授权后通过 HTTP worker 上传，保留大小、修订、哈希和格式校验                     |
| ChatGPT / Gemini 实时语音                                 | 有界语音 turn 的 WebSocket 连接、输入发送和响应收集在 MXC worker；宿主只准备已授权输入和发布校验后的音频             |
| Skills 目录、资源与安装                                   | 列表、读取、修改、安装预览及安装在离线 worker；目录扫描、归档解包与包校验也在 worker，远程下载使用 HTTP worker       |
| Memory 存储与搜索                                         | 读取、列表、搜索、写入、删除、恢复在离线 worker；保留原有锁、修订和事务日志，派生搜索索引放在任务私有目录            |
| 浏览器渲染和交互                                          | Browser 在宿主启动带原生沙盒的 Chrome，Web JavaScript 渲染要求直接执行模式                                          |

PTY 和其他常驻子进程的内核权限在创建时固定。需要新增目录或网络目标时，先修改 `permission.path`、`permission.network`，再创建新的终端或进程。已有终端的执行模式或资源策略改变后，`shell.write` 会拒绝输入，仍可读取输出或停止进程；修改设置不会给运行中的进程追加挂载或网络权限。

启用沙盒时，缺失目标程序、必要运行时文件、沙盒后端或联网依赖会返回具体启动错误。执行器不会自动转为宿主直跑，也不会通过放宽目录或网络策略重试；直接执行必须在调用前明确选择。

### 文件修改的宿主发布

暂存区只包含本次操作需要的输入副本与候选输出。worker 给出的任意真实路径不会成为新的宿主写入目标。

宿主在准备阶段固定目标映射并持有文件变更锁；发布阶段再次检查原始字节修订、文件类型、目标身份、大小限制和策略。发布使用固定目录能力与相对文件操作，避免通过父目录符号链接替换把已批准路径重定向到另一目录。

文本补丁保持 UTF-8、单文件和事务总量限制。新增、更新、删除通过同一组已固定的目录能力提交和回滚；多文件的确定性验证在首次修改之前完成。移动文件的目标权限来自宿主捕获的真实源文件权限，不由 worker 返回的目标路径或权限关系决定。

文件内容在处理期间被其他程序修改时，提交返回修订冲突，保留外部修改。取消或 worker 失败不进入结果发布。输出目录的创建也是宿主针对已获准目标执行的固定操作，不需要给 worker 整个宿主父目录的写权限。

### 媒体传输

`fs.read_media` 的单文件上限是 20 MiB；文本输入上限是 1 MiB；图片边长最多 16384 像素，总像素最多 4000 万。跨进程传输使用专用 DTO，宿主检查 base64 长度、解码后字节数、SHA-256、文件名、媒体类型和尺寸元数据，随后沿用原有 attachment 与 provider route 绑定。

共享的普通 `PreparedMedia` 序列化仍只包含元数据。媒体 worker 的私有传输字段不会因此进入其他调用的常规 JSON 输出。

ChatGPT、Claude、Gemini 工具读取本地分析输入时使用同一媒体 worker；文件上传先在受限文件 worker 中读取，再在离线媒体 worker 中识别格式。普通文件上传保留 512 MiB 上限，媒体解码沿用 20 MiB 上限；较大的音视频只检查有界容器头，不为上传完整解码。非媒体数据集可以按原文件类型上传，worker 启动或传输失败会返回错误。

实时语音的本地输入先经过路径授权与有界文件读取。独立网络 worker 只接收目标请求头、固定地址、模型/session、文本或 PCM，完成 1～50 秒的 turn 后返回有界音频、转录和事件；宿主检查返回 PCM 与大小限制，再使用原有授权输出流程保存音频。

### 技能与记忆的持久写入

Skills 和 Memory 的工作进程直接操作当前授权的目标目录，保留各自原有的原子替换、修订检查与恢复机制。技能变更由宿主保持变更锁，直到 worker 完成或取消清理结束；记忆读写由 worker 内的文件锁、修订检查和事务日志保护。技能包下载与本地解包分开授权，解包、目录遍历和包校验在离线 worker 中执行；worker 只接收业务数据与必要路径，不启动完整服务或模型。

记忆读写保留文件锁、事务日志、SHA-256、删除标记与恢复逻辑。搜索需要的派生索引在 worker 私有临时目录建立，读操作不因此获得宿主数据库或共享缓存目录的写权限。

### LSP 服务复用

LSP 缓存键包含会话身份、服务器名称、实际根目录和完整 `SandboxPolicy`，使用完整策略相等性。不同会话或不同权限策略不共用语言服务进程；只有已持久允许的项目读取范围才进入会话缓存。仅本次获批的 `ask`/`auto` 根目录以及没有持久会话身份的调用使用临时客户端，最后一个调用句柄释放时回收进程。

语言服务所需的程序和安装目录由受信任配置提供；当前工具接线保持项目只读，HOME、缓存和 `CARGO_TARGET_DIR` 写入进程私有目录。需要在项目内写缓存的语言服务，应配置为使用其私有缓存目录；当前接线不会自动授予项目写权限。

### MCP 服务连接

MCP 配置位于 `plugins.list["agena.mcp"]`。该插件的 `settings.servers.<name>.process` 使用 `command`、`args`、`env`、`cwd` 描述启动方式。启用 MXC 时，服务使用当前生效的 `permission.path`、`permission.network`，并与全部 ceiling 和持久拒绝共同约束；`reads`、`writes`、`network` 声明不会改变此范围，省略声明也不会强制离线。需要确认的启动操作仍由工具审批处理。

以下 stdio 定义使用当前配置授予的路径和网络范围：

```json
{
  "transport": "stdio",
  "process": {
    "command": "node",
    "args": ["server.mjs"],
    "cwd": "/path/to/project"
  }
}
```

服务的 `cwd` 必须是现有目录；显式相对 `cwd` 按服务配置所在工作区解析。启用 MXC 时，`cwd` 需要读取权限，配置中的可写挂载目标须已存在，并同时具有读取权限。

只有关闭 MXC 且开启 `resource_guard` 时，`process.reads`、`process.writes`、`process.network` 才用于调用前的回退声明检查。此时相对资源路径基于服务实际 `cwd`，`<workspace>` 指向当前会话项目；声明检查不会给直接运行的服务建立内核隔离。

加载配置只登记 stdio 定义，管理列表以 `scoped` 表示需要执行作用域。实际使用时，宿主验证当前会话、调用或 operator 能力，选取当前执行模式并生成策略。实例按服务器、配置版本、宿主 scope、工作区、实际工作目录和完整策略隔离；不同会话或更窄策略不会借用已有宽权限进程。实例退出后，下一次合法调用会重新检查当前权限再连接；显式重连也走相同流程，后台不会脱离授权以旧策略创建进程。

当所有资源已由现有策略明确允许时，同一会话可以复用相同策略实例；一次性批准的资源和 operator 调用绑定本次宿主调用的完成信号，调用结束即移除并关闭对应实例。`connections.test` 总是建立独立连接，并在完成后关闭它。缺少可信调用 scope 或所选模式需要的后端能力时直接返回错误。

工具访问 HTTP MCP 时，每次宿主调用创建独立连接管理器，保留该服务器的认证存储、工具过滤、roots 和超时配置。连接、重连、OAuth discovery/registration/refresh 及 token exchange 都使用当前调用的授权上下文和逐请求固定地址，实际请求进入 MXC HTTP worker。流式 HTTP 的 POST、GET/SSE 和 DELETE 使用同一传输与取消边界。清理在建立连接前注册，调用成功、失败或取消后关闭该独立管理器；连接测试也在取消时回收。结束的 OAuth 授权作用域不能退回可信管理客户端。

管理界面的可信连接管理请求沿用管理端传输；模型通过工具访问外部 MCP 服务时使用其所属调用的权限。Agena 自身入站 `/mcp` 和 stdio 桥接仍按前述规则关闭本次 MXC，两种方向的 MCP 入口分别处理。

### 外部 stdio 插件

`plugins.list["<plugin-id>"].package` 在 `kind: "stdio"` 时保留 `reads`、`writes`、`network` 声明，仅在 MXC 关闭且 `resource_guard` 开启时用于回退检查。启用 MXC 时，bootstrap 使用独立空工作目录，只读开放已配置插件包、明确解释器入口及必要运行时，网络关闭；它负责 manifest、初始化和关闭等基础生命周期。会话工具和其他操作使用当前完整配置策略，不通过这些声明缩小或扩大权限。

其他工具、流式工具、hooks 和 services 必须取得当前有效的宿主授权作用域。实例按 scope、工作区、工作目录及完整策略隔离，初始化继承当前会话上下文；动态工具注册进入对应会话 overlay，并随进程 effect scope 清理。子进程回调只能使用该进程当前活跃请求收到的 token，同时匹配其会话、工作区和适用的调用边界。

一次性授权的实例在宿主调用完成后移除并关闭。scope 实例退出后，下一次合法调用会重新求取当前策略再创建进程；不会在后台脱离原授权重新启动。没有可信作用域的非基础生命周期返回明确错误，bootstrap 不承载会话工具调用。

## 进程内插件和浏览器进程

cdylib 插件与 Agena 宿主在同一进程中运行，必须作为受信任代码配置；对子进程的 MXC 启动包装不能隔离已载入的动态库。需要加载不受信任动态库的能力时，应迁移到受限工作进程。

Chrome 沙盒隔离网页进程。Chrome 主进程在宿主机自行建立网络连接。

## 平台要求与当前部署状态

| 平台    | 原生后端                       | 部署要求与当前功能                                                                                                                      |
| ------- | ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------- |
| Linux   | Bubblewrap                     | 需要可用 `bwrap` 和允许创建所需 user/mount/network namespace 的系统策略；联网模式还需 `slirp4netns` 及后端要求的网络工具                |
| macOS   | Seatbelt                       | 使用 MXC 支持的 macOS 版本；MXC 1.1.0 的 macOS 基线为 macOS 15；离线和出站默认动作由本机后端检查，不支持细粒度出站目标规则              |
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

Agena 可执行文件、MXC 后端工具及宿主 PATH 中的程序目录受到写保护，包括符号链接别名。launcher 会在分解宽泛的读写授权时保留这些路径的只读边界；显式要求写入受保护的程序路径会报错。

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

新增工作进程的本地集成检查：

```bash
cargo run -p agena-process --example sandbox_io_smoke -- --files-only
cargo run -p agena-bundled-plugins --example sandbox_local_smoke
```

第一条检查真实文件读取与 MXC 对未授权文件的拒绝。第二条在临时目录检查 TAR 解析、技能安装与修订冲突、记忆读写和只读搜索、Notebook 编辑，以及越权访问失败。测试不会读取用户技能库、记忆库或修改实例配置。

在支持细粒度原生网络规则的宿主上，允许测试连接本机非回环 IPv4 后，运行完整 HTTP 检查：

```bash
cargo run -p agena-process --example sandbox_io_smoke
```

该检查使用临时本机 HTTP 服务，验证固定目标地址、流式上传、重复响应头、SSE、调用取消及关闭网络后不回退宿主执行。`--files-only` 只运行文件部分，不计为网络验证成功。

测试环境允许访问 GitHub、但禁止访问本机服务时，可以单独验证原生 HTTPS 和禁网拒绝：

```bash
cargo run -p agena-process --example sandbox_io_smoke -- --https-only
```

该模式只读取公开的 `https://github.com/robots.txt`，不使用凭证；它检查 TLS、地址固定、完整响应传输和离线拒绝，本机 SSE 时序与连接取消仍由完整本机检查覆盖。

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

| 现象                                              | 原因和处理                                                                                                |
| ------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Linux 创建 user namespace 返回 `EPERM`            | 检查 doctor 的实际 bwrap 路径、AppArmor 限制和发行版专用 profile；修复后重新运行 doctor                   |
| 找不到 `slirp4netns`                              | 先安装当前发行版支持的联网依赖；离线策略与联网策略分开验证                                                |
| 原生网络规则提示后端不支持                        | macOS 只使用出站默认动作或离线；Windows 检查宿主 ProcessContainer/PSEC 能力；不能静默改用代理或无沙盒执行 |
| `configured MXC writable path must already exist` | 在 `permission.path` 配置现有输出目录，或先通过文件工具创建已授权的准确目标；执行器不会自动扩大到父目录   |
| 路径规则 `Unsupported`                            | 按错误指出的路径检查显式 `deny` 下的子授权、符号链接导致的同路径冲突，以及当前后端能力                    |
| `ReadApprovalRequired`                            | 读写挂载会包含读取能力，需要补足该写入目标的读取授权                                                      |
| 域名没有可用的获准 IP                             | 检查实际 DNS 结果是否命中原生允许规则，以及是否被配置或权限上限拒绝；主机名本身不授予访问权限             |
| 补丁修订冲突                                      | 文件在暂存后被修改；重新读取最新版本并准备修改，不覆盖外部变更                                            |

## 源码入口

| 模块                                                                | 职责                                                      |
| ------------------------------------------------------------------- | --------------------------------------------------------- |
| `agena-runtime-contracts/src/permission/sandbox.rs`                 | 可序列化策略、完整权限上限、路径边界和 DNS 地址二次检查   |
| `agena-runtime-tools/src/tool/executor/executor_sandbox.rs`         | 当前 scoped principal 与已保存规则绑定                    |
| `agena-runtime-tools/src/shell_sandbox.rs`                          | Shell 完整配置策略及宿主工具实际资源范围到 MXC 策略的投影 |
| `agena-runtime-tools/src/sandbox_worker.rs`                         | 受限工具 worker、暂存和宿主发布                           |
| `agena-runtime-tools/src/tool/apply_patch.rs`                       | 补丁解析、计划与 worker 内文本修改                        |
| `agena-runtime-tools/src/fs_broker.rs`                              | 固定目录能力、发布前修订校验、原子提交和回滚              |
| `agena-runtime-session/src/session/manager/host/process_sandbox.rs` | 托管子进程的当前作用域策略及关闭沙盒时的回退声明审批      |
| `agena-mcp-client/src/manager/scoped.rs`                            | MCP 定义版本、完整策略与作用域实例生命周期                |
| `agena-plugin-host/src/transport/stdio/scoped.rs`                   | 外部 stdio 插件的 bootstrap 与作用域进程路由              |
| `agena-runtime-tools/src/media_input.rs`                            | 媒体准备与私有 IPC 传输校验                               |
| `agena-process/src/sandbox.rs`                                      | MXC launcher、环境、运行时目录、平台能力和进程生命周期    |
| `agena-process/src/sandbox_http.rs`                                 | 固定目标的单跳 HTTP、流式匿名管道、连接取消和超时         |
| `agena-process/src/sandbox_file.rs`                                 | 上传文件的隔离读取、大小限制与修订校验                    |
| `agena-bundled-plugins/src/worker.rs`                               | 技能、记忆与实时语音的最小请求协议和调用生命周期          |
| `agena-lsp/src/registry.rs`                                         | 按完整权限策略区分语言服务实例                            |
