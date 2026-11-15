# 插件图标与信息

插件通过运行时 `PluginManifest` 声明卡片和详情页的信息。Rust 插件可使用 `#[agena_plugin(...)]`，其他语言的插件在 `meta/manifest` 返回相同字段。

## 元数据字段

| 字段 | 用途 |
| --- | --- |
| `namespace`、`name` | 稳定身份，组成 `namespace.name` 插件 ID |
| `display_name` | 可读名称；省略时显示插件 ID |
| `icon` | 图片 URL 或图片 data URI |
| `version` | 插件版本 |
| `authors` | 作者名称列表 |
| `summary` | 卡片上的简短介绍 |
| `help` | 详情页中的 Markdown 说明 |
| `translations` | 按语言提供 `summary`、`help` 等已有文档字段的翻译 |
| `tags` | 插件功能标签 |

`display_name` 与 `icon` 均可省略。修改显示名称不会改变插件 ID、配置键、工具命名或命令路由。

下面是一份可直接序列化的 manifest：

~~~json
{
  "schema_version": 1,
  "namespace": "demo",
  "name": "board",
  "version": "0.1.0",
  "display_name": "Workspace board",
  "icon": "https://example.com/my-plugin/icon.png",
  "authors": ["canxin121"],
  "summary": "A persistent task board for project work.",
  "help": "Open the board to create, edit and complete tasks.",
  "translations": {
    "zh-CN": {
      "summary": "为项目工作提供持久化任务看板。",
      "help": "打开交互页面后可创建、编辑和完成任务。"
    }
  },
  "tags": ["interactive", "dashboard"]
}
~~~

将示例图片地址替换为插件实际提供的地址。元数据属于插件返回的 manifest；实例配置 `plugins.list` 继续负责启用插件、选择包和填写运行设置。

## 图标来源

`icon` 接受以下形式：

| 形式 | 示例 |
| --- | --- |
| HTTP/HTTPS 图片 | `https://example.com/plugin/icon.svg` |
| 宿主已提供的同源绝对路径 | `/icons/my-plugin.png` |
| Base64 图片 | `data:image/png;base64,...` |
| Base64 SVG | `data:image/svg+xml;base64,...` |
| URI 编码的 SVG | `data:image/svg+xml,%3Csvg%20...%3E...%3C%2Fsvg%3E` |

支持 PNG、JPEG、GIF、WebP、AVIF、SVG 和 ICO。图标源字符串最多为 **256 KiB（262144 字节）**，包含 data URI 的头部与编码内容。二进制图片使用 Base64；SVG 可使用 Base64 或 URI 编码。

包内图片适合在构建时编码为 data URI，这样无需额外图片服务器，也不要求插件声明 `surface.web`。例如，将 SVG 转成可供 Rust 包含的文件：

~~~python
from pathlib import Path
import base64

image = Path("icon.svg").read_bytes()
Path("icon.data-uri").write_text(
    "data:image/svg+xml;base64," + base64.b64encode(image).decode("ascii"),
    encoding="utf-8",
)
~~~

`icon` 表示浏览器可加载的图片来源，不会把本地文件路径或插件包相对路径自动转换为资源 URL。

## Rust 声明

~~~rust
use agena_plugin_sdk::{agena_plugin, ToolTag};

#[agena_plugin(
    namespace = "demo",
    name = "board",
    version = "0.1.0",
    display_name = "Workspace board",
    icon = include_str!("icon.data-uri"),
    authors = ["canxin121"],
    summary = "A persistent task board for project work.",
    help = "Open the board to create, edit and complete tasks.",
    tags(ToolTag::Interactive)
)]
impl BoardPlugin {
    // Declare this plugin's tools, commands and hooks here.
}
~~~

`include_str!` 的路径相对当前 Rust 文件；可将生成的 `icon.data-uri` 放在该文件旁边，或调整为对应的相对路径。`icon` 和 `display_name` 可使用字符串、常量或返回字符串的表达式。手写 `Plugin::manifest()` 时，直接设置 `manifest.icon`、`manifest.display_name`、`manifest.authors` 等字段即可；[Workspace board 示例](../examples/web_dashboard_plugin_stdio/src/main.rs) 展示了 URI 编码的 SVG。

## Web 展示

插件列表接口 `GET /api/v1/plugins` 一次返回图标、名称、作者、版本、标签和简介。卡片使用这些信息，不会为展示作者或图标逐个读取插件详情。详情页继续从 inspect manifest 展示完整说明和作者。

图标按图片加载，不插入 HTML。缺少图标、地址格式不支持或图片加载失败时，显示通用插件图标；插件本身继续运行。允许的来源以外的协议、协议相对 URL、带用户名密码的 URL 和超长数据会回退到默认图标。

内置插件也遵循这份合同：每个插件声明自己的图标与信息，作者统一为 `canxin121`，版本使用所在 Cargo 包的版本。前端不再维护插件 ID 到内置图标的映射表。
