# 插件 Web 页面

插件可以在同一个发布包中提供后端代码和原生 Web 客户端模块。安装并启用后，Agena 根据插件声明展示页面入口，加载插件自己的 JavaScript 和 CSS，并通过宿主 SDK 调用插件命令。新增插件页面无需重新构建 Agena Web。

完整示例位于 [Workspace board](../examples/web_dashboard_plugin_stdio/README.md)。它包含独立 stdio 后端、可持久化的任务看板、多文件 ES module、主题与语言适配、响应式布局和清理逻辑。

## 页面声明

在 `PluginManifest.surface.web` 中声明：

~~~json
{
  "api_version": 1,
  "assets": {
    "root": "web/dist"
  },
  "pages": [
    {
      "id": "dashboard",
      "title": "Workspace board",
      "entry": "dashboard.js",
      "styles": ["dashboard.css"],
      "placement": "workspace",
      "scope": "instance",
      "commands": ["list", "update"],
      "settings": false
    }
  ]
}
~~~

`assets.root` 相对插件包根目录；`entry` 和 `styles` 相对资源目录。入口是导出 `mount` 的 ES module。模块可以引用同目录或子目录中的其他模块、图片和字体。所有资源必须位于声明的资源目录内。

页面 ID 在插件内唯一。标题用于页面目录和工作区标签；页面可通过 SDK 更新自己的标签标题。`commands` 声明该页面使用的 Method 命令；命令也必须出现在插件的 `commands` 集合中，并使用 Method target。`settings` 为真时，页面可通过 SDK 读写本插件设置。

`settings` 配置表单与 Web 页面并存：通用设置继续使用现有声明式编辑器，业务页面由插件代码自行实现。

## 插件包根目录和发布

本地开发的插件配置显式设置绝对 `package_root`：

~~~json
{
  "plugins": {
    "list": {
      "demo.web_dashboard": {
        "enabled": true,
        "package_root": "/absolute/path/agena/examples/web_dashboard_plugin_stdio",
        "package": {
          "kind": "stdio",
          "command": "/absolute/path/agena/target/debug/agena-web-dashboard-plugin-stdio",
          "args": [],
          "env": {}
        }
      }
    }
  }
}
~~~

宿主不能根据 `command` 推断包根：stdio 命令可能是 `python3` 或 `node`，实际插件文件位于参数指定的位置。`package_root` 由本地配置或安装器提供，插件 RPC 返回的页面声明只提供包内相对路径。

将构建产物加入现有发布配置：

~~~toml
[release]
entrypoint = "bin/my-plugin"
include = ["web/dist", "README.md", "LICENSE"]
~~~

市场安装器将 archive 解到版本目录，并记录提取后的包根。前端产物与后端一起发布、验证、安装和升级。构建工具可以使用 Vite library mode，也可以像示例一样直接编写标准 ES module；入口不能依赖 Agena 源码中的私有模块路径。Vue/React 项目应把所需运行时包含在自己的产物中。

资源放在 `web/dist` 的哪一层，就使用对应相对引用。例如入口的 `import './model.js'` 会访问同一插件、同一 revision 下的 `model.js`。使用相对资源路径，避免以 `/` 开头的应用根路径。

## 挂载与卸载

客户端入口导出：

~~~ts
export function mount(container: HTMLElement, sdk: AgenaPluginUI) {
  const page = document.createElement('section')
  page.textContent = 'My plugin page'
  container.append(page)

  return {
    dispose() {
      page.remove()
    },
  }
}
~~~

`mount` 可以返回 Promise。宿主拥有路由、容器和挂载过程，插件拥有容器中的组件、订阅和页面状态。Vue 可在容器中 `createApp().mount()`，React 可在容器中 `createRoot().render()`。

切页、关闭标签、禁用或重载插件时，宿主终止页面 SDK 的 `signal` 并调用返回的 `dispose()`。异步挂载晚于卸载完成时，其返回的 handle 也会被清理。插件应在 `dispose` 中释放事件监听、定时器、组件和自己创建的 DOM；在异步调用后检查取消信号，避免更新已卸载界面。

同一模块可能在多个工作区标签中挂载。把页面状态放在 `mount` 内，不要使用模块级的“当前页面”或操作其他标签的 DOM。CSS 使用插件前缀、CSS Modules 或组件 scoped 样式，避免修改宿主全局布局。共享样式由宿主引用计数管理。

## 宿主 SDK

公开 TypeScript 类型位于 `packages/agena-web/src/features/pluginWeb/sdk.ts`。页面收到的 SDK 绑定本次插件实例、页面和运行代次。

| 接口 | 行为 |
| --- | --- |
| `pluginId`、`pageId` | 当前插件实例和页面 ID |
| `signal` | 页面调用与生命周期取消信号 |
| `context.sessionId` | 页面选择的真实会话，或 `null` |
| `context.path` | 当前页面路由下的子路径，供插件内部导航使用 |
| `commands.invoke(id, input)` | 调用页面声明的插件命令，返回 `PluginCommandResult` |
| `settings.read()` | 读取声明允许的本插件配置状态 |
| `settings.update(value)` | 使用宿主现有校验和保存流程更新本插件设置 |
| `navigation.openPage(pageId)` | 打开同一插件声明的另一个页面 |
| `ui.setTitle(title)` | 更新当前工作区标签标题 |
| `ui.notify(message, tone?)` | 显示成功、错误或信息通知 |
| `environment` | 当前 `locale` 和 `theme` |
| `subscribeEnvironment(listener)` | 立即传入当前环境，并订阅变化；返回取消订阅函数 |

调用示例：

~~~js
const result = await sdk.commands.invoke('list', {})
if (sdk.signal.aborted) return
if (result.status !== 'succeeded') {
  throw new Error(result.detail || result.summary)
}
renderTasks(result.output.tasks)
~~~

`PluginCommandResult` 保留 `status`、`summary`、`detail`、`output` 等现有命令语义。页面应显式处理失败结果，业务数据从 `output` 读取。更新接口遇到冲突或失败时，保留用户输入，并提供明确的恢复操作。

环境订阅：

~~~js
const unsubscribe = sdk.subscribeEnvironment(({ theme, locale }) => {
  container.dataset.theme = theme
  container.lang = locale
})

// 在 dispose 中调用 unsubscribe()
~~~

原生页面继承宿主 CSS 变量。颜色值使用当前的 OKLCH 格式，例如 `oklch(var(--foreground))` 和 `oklch(var(--background))`。

## 页面路由与上下文

页面地址为：

~~~text
/plugins/{pluginId}/{pageId}
~~~

路由使用稳定的插件和页面标识；资源 revision 和运行 generation 由当前服务端目录解析。用户可以收藏地址、刷新页面或在多个工作区标签中打开页面。当前插件或页面不可用时，宿主显示恢复入口，不把它当作主站默认聊天页面。

`scope = "instance"` 的页面可独立于聊天会话运行。示例使用普通 Method commands 和插件私有 global KV，因此不需要先创建聊天。

`scope = "session"` 的页面绑定真实会话上下文，适合需要项目目录或会话状态的 Method 命令。后端使用宿主能力时，项目路径和网络权限申请继续走会话权限链。需要项目的操作必须使用实际项目会话，不能构造占位会话 ID。页面命令入口只执行声明的 Method 命令，不接受 Tool target。

## 插件目录与内置管理页

Web `/plugins` 汇合两份独立的权威目录：

- `GET /api/v1/plugins` 列出所有运行时插件（包括 `static`、`stdio`、`cdylib` 等类型）。没有独立 Web 页面的插件依然可被发现，点击后进入现有的插件设置工作台。
- `GET /api/v1/plugins/surface` 的 `catalog.web_pages` 列出插件自己声明的可挂载 Web 模块。一个插件可以有多个页面。

内置 `agena.telegram` 的 `/plugins/agena.telegram/manage` 页面由 Agena 宿主提供，复用已有插件设置、命令与集成绑定 API。其管理界面不属于第三方发布包，也不要求 Telegram 插件声明 `surface.web`。这类内置管理页必须检查对应插件仍存在于运行时目录，不能在插件卸载或禁用后继续冒用其身份。Telegram 多 Bot 工作台将 Bot ID、管理会话与当前区域保存在深链接中，通过插件命令读写私有 Bot 注册表、独立凭据存储和持久收件队列。聊天发现与授权响应分别展示，多个 Bot 的相同 Chat ID 不合并。Telegram 配置、会话成员与频道权限的操作说明见 [集成说明](integrations.md#内置-telegram-渠道插件)。

## 服务端边界

`GET /api/v1/plugins/surface` 的 `catalog.web_pages` 是当前可用页面目录，包含插件 ID、页面 ID、标题、作用域、入口和样式 URL、revision、generation 及声明的操作。客户端从该目录加载资源，URL 参数不提供任意模块地址。

页面命令经过：

~~~text
POST /api/v1/plugins/{pluginId}/web/{pageId}/commands/{commandId}
~~~

请求携带当前 generation、命令 input 和必要的 session_id。宿主检查页面声明、当前实例和命令输入，再进入已有 Method command 执行链。插件间 `service/invoke` 和插件回调 `plugin-rpc` 保留各自用途。

资源使用：

~~~text
/plugin-assets/{pluginId}/{revision}/{path}
~~~

资源来自已验证的安装包目录，以 revision 固定。静态请求只提供插件声明的构建产物；不存在的 JavaScript/CSS 返回资源错误，不回退到主站 HTML。页面目录、配置与命令 API 使用现有服务器鉴权。

原生模块与 Agena 主页面处于相同的浏览器信任范围，适用于使用者安装的受信任插件。SDK 提供稳定接口和实例绑定；插件 JavaScript 仍具有同源代码的浏览器权限。不要在客户端构建产物中包含 token、服务端配置或秘密。静态构建产物可公开下载，业务数据通过鉴权 API 读取。

## 更新和兼容

页面身份、资源 revision 和运行 generation 分别解决不同问题：

| 标识 | 用途 |
| --- | --- |
| `pluginId + pageId` | 稳定路由和导航 |
| `revision` | 一致的入口、样式和动态 chunk |
| `generation` | 当前插件实例及其调用有效期 |

插件禁用、重载或升级后，旧页面调用失效；页面根据最新目录重新加载或展示不可用状态。只修改本地静态文件时，也应重新加载插件，使宿主建立新的资源快照和 revision。

`api_version = 1` 是客户端页面协议版本。没有 Web 声明的旧插件继续使用原来的工具、命令和配置界面。发布 Web 插件时应使用支持该能力的 Agena 版本；旧宿主的严格 manifest 解析会拒绝未知 Web 字段。

## 验证

从仓库根目录运行示例的后端和前端数据测试：

~~~sh
cargo test --locked -p agena-web-dashboard-plugin-stdio
node --test examples/web_dashboard_plugin_stdio/web/tests/model.test.mjs
~~~

宿主前端的 SDK、挂载与资源生命周期测试位于 `packages/agena-web/tests/pluginWeb.test.ts`，在该包目录运行 `bun run test:plugin-web`。

### 真实浏览器回归

`packages/agena-web/scripts/probe-plugin-web.cjs` 使用真实服务器、已打包的 stdio 插件和 Chrome。安装好仓库 Web 依赖后，从仓库根目录依次执行：

~~~sh
cargo build --locked -p agena -p agena-plugin-dev -p agena-web-dashboard-plugin-stdio --bins
cd packages/agena-web
bun run build
PLAYWRIGHT_MODULE=/absolute/path/to/playwright \
CHROME_BIN=/absolute/path/to/chrome \
bun run test:plugin-web:browser
~~~

`PLAYWRIGHT_MODULE` 指向可被 Node `require` 加载的 Playwright 模块目录，`CHROME_BIN` 指向 Chrome/Chromium 可执行文件。环境中已能解析 `playwright` 且已安装其 Chromium 时，可省略这两个覆盖项。

Probe 自动选择独立端口，创建新的临时 data-dir、工作目录和本地市场索引。它执行真实发布包打包与市场安装，检查资源访问和命令鉴权，然后在 Chrome 登录、打开插件目录和页面、写入任务并刷新深链接。后续步骤覆盖主题和窄窗口布局、键盘关闭编辑器、runtime reload 后的页面重挂载与数据恢复、禁用/启用、卸载以及旧 generation 调用失效。

终端会输出 `Plugin Web probe artifacts: <目录>`。该目录保留 `report.json`、`server.log`、已运行到对应步骤的 `desktop-light.png`、`desktop-dark.png`、`mobile.png`，以及测试使用的 `data/`、`workspace/` 和 `release/`。失败时先查看报告中的检查结果与错误，再结合服务器日志和截图定位。

开发自己的页面时，还应检查多标签同时编辑的冲突恢复、插件后端进程重启后的持久化，以及页面特有的输入和交互。
