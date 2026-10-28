# Inline Generative UI（正文内嵌富内容）

这项能力让 AI 在**正常 Markdown 回答的指定位置**插入步骤、对比、提示、指标、表格、说明或图表。
不是展示一个独立 App，也不是把工具调用结果画得像卡片就算“内嵌”。

## 实际流程：准备 → 正文引用 → 原位渲染

内置插件是 `agena.inline_ui`。工具目录按需发现，不把所有内容类型的完整 Schema 塞进系统提示。
实际执行名以当前目录为准，以下为 canonical identity：

| 工具 | 行为 |
| --- | --- |
| `agena.inline_ui.guide` | 默认返回简短概览；指定 `topic` 后，只返回该类型的结构和完整示例。 |
| `agena.inline_ui.validate` | 可选校验；不准备引用、不展示 UI。 |
| `agena.inline_ui.prepare` | 保存不可变内容，返回 `reference` 和 `embed`；**工具调用本身不展示这份富内容**。 |

例如模型调用 `prepare`，参数仍是受控的结构化内容：

```json
{
  "document": {
    "version": 1,
    "title": "建议的实施顺序",
    "blocks": [{
      "type": "steps",
      "id": "plan",
      "items": [
        {"label":"P0","title":"统一内容","text":"先让同一份信息在 Web 和终端都能读懂。","icon":"layers"},
        {"label":"P1","title":"验证体验","text":"检查窄屏、复制与历史回放。","icon":"check"}
      ]
    }]
  }
}
```

成功后模型收到简短结果，而不是重复接收整个文档：

```json
{
  "prepared": true,
  "reference": "ui_123_0123456789abcdef0123456789abcdef",
  "embed": "```inline-ui\nui_123_0123456789abcdef0123456789abcdef\n```",
  "message": "Not displayed yet. Paste the entire returned inline-ui fenced block unchanged between Markdown paragraphs..."
}
```

以上 ID 仅为说明。真实 ID 来自该次调用，模型不能猜测、重写或引用其他会话的 ID。
随后 AI 正常输出回答文本：

````markdown
我建议分两个阶段推进：

```inline-ui
ui_123_0123456789abcdef0123456789abcdef
```

第二阶段应重点检查窄屏和历史回放，因为……
````

Web 与 TUI 都在这一行的位置展开内容。前后的普通 Markdown 仍可加粗、写链接和引用来源；
链接的引用定义即使位于另一个富内容块之后，也使用同一回答的解析上下文。
正文可以交错多个引用，不需要分成多条回答或多个工具结果。
**不写正文引用就不会显示这份内容。** 准备工具只保留可检查的普通执行记录，不再重复展示 UI。

## 引用语法与流式行为

使用语言标识为 `inline-ui` 的顶层反引号代码围栏，里面只放一个引用 ID：

````markdown
```inline-ui
ui_<positive-tool-id>_<32-lowercase-hex-nonce>
```
````

`prepare` 返回完整代码块，直接粘贴到正文所需位置即可，不需要再包一层围栏，
更不要把 JSON 文档放进代码块。建议前后留空行；普通文字仍按 Markdown 排版。

只有明确闭合的 `inline-ui` 反引号围栏才会展开；其它语言或无语言的代码块、
引用块、列表、四空格缩进代码、HTML 块，以及更长外层围栏包住的示例都保持代码原文。
同一个块只能有一个合法引用，错误 ID、多个 ID 或 JSON 都不会被当作 UI 执行。
用户消息中的引用代码块也只显示原文。

流式生成尚未闭合的 `inline-ui` 块暂不显示协议代码；生成结束后仍未闭合则保留源码。
不存在、伪造或不属于当前会话的完整引用给出不可用提示，不会静默删除内容或自动调用工具。
不保留双中括号的兼容解析：旧引用即使出现在历史回答中，也只显示原文。

UI 文档先完整校验再准备；正文按普通文本流生成，围栏闭合后在原位展开内容。

## 内容目录与边界

| 类型 | 语义及限制 |
| --- | --- |
| `paragraph` | 富内容内部的连接文字，最多 6,000 个 Unicode 字符。 |
| `callout` | note、tip 或 warning，正文最多 3,000 字符。 |
| `steps` | 1–12 个说明阶段，可选短标签与受控图标；不是任务执行状态。 |
| `comparison` | 2–4 个选项、1–16 个对比维度，每行值数必须一致。 |
| `metrics` | 1–6 个有依据的值，带单位与可选背景。 |
| `table` | 1–8 列、1–40 行，不用来承载无限数据集。 |
| `details` | Web 可折叠；TUI 与复制保留完整内容。 |
| `chart` | line、bar、area、pie；1–128 个分类标签、1–6 个序列。 |

文档内部文本字段是纯文本，不编译 HTML、Markdown 模板或代码；普通回答正文仍是 Markdown。
图表的 X 轴是分类序列，不是数值散点图或不等间隔时间轴。`null` 表示缺失，不等于零。
饼图需要单个非负序列且至少有一个正值。演示数据必须明确标注，不得虚构测量或进度。

文档 `version: 1`，1–24 个块，序列化上限 60 KiB，为运行时 64 KiB 的内联结果预算预留包装空间。
块 ID 在文档内唯一，1–80 个 ASCII 字母、数字、下划线或连字符。单个回答最多解析 32 个引用。
所有输入仍经过 Rust 的权威校验和 Web 的防御校验。

## 持久化、权限与回放

准备后的文档只保存在该工具调用的 `RawOutput.payload.inline_ui_prepared`，不另存一套可变 Registry。
引用由持久化工具 ID 和随机后缀组成；服务端验证调用已成功、所属工具正确、保存的后缀完全匹配。
读取使用 `KernelReader.tool(current_session, tool_id)`，沿用会话成员关系授权，不使用全局 ID 查表绕过权限。
分支继承可见历史时使用当前分支的成员关系，不将 origin session 当作访问捷径。

需要修订时再次准备并获得新引用；旧回答保持旧内容。读取、渲染、复制不修改工具结果、不重启服务。
即使历史分页尚未载入原工具消息，仍可通过当前会话的授权引用接口恢复内容：

```text
GET /api/v1/sessions/{session_id}/inline-ui/{reference}
```

接口返回 `Cache-Control: private, no-store`。前端不维护跨会话的全局引用缓存，切换会话时取消旧请求。
真实存储测试覆盖未完成调用、无关会话、错误后缀、伪装工具、重复读取不写入。

## 文本流、TUI 与复制导出

原始正文和 UTF-8 字节游标始终保持不变。`ContentPage` / `ContentTextPage` 可携带可选 `inline_ui`
旁路数据，保存已经过当前会话授权的引用文档。服务端与 Web／TUI 客户端应一起更新，旧版严格解码客户端不支持这个新增传输字段。标记跨网络页面时，服务端用有限的字节回看恢复引用，
不是修改文本长度再伪造游标。未加载原工具记录不影响正文显示。

Web 将正文拆成原样的 Markdown 片段与引用组件。TUI 在读时将引用替换为语义 Markdown：
步骤、对比、提示保持清晰，图表以精确数据表呈现。终端流式缓冲仍按原始字节合并，只有显示投影展开引用。

整条回答复制按正文顺序展开引用，保留前后文字，不把准备工具结果额外复制一次。
原始 SDK `write_part_text` / `write_content_text` 保持返回原始文本；新的 `write_reply_part_text` /
`write_reply_content_text` 用于面向读者的复制和导出。后者收集一份固定文本快照以保持跨页 Markdown
边界，再展开授权引用，不会让网络分块改变代码示例的含义。超大原始输出仍可走原有流式接口。

## 单一协议，不保留历史兼容

唯一的富内容入口是上面定义的 `inline-ui` 引用围栏。双中括号标记、旧 `inline_ui.present`
自动展示、`agena-chart` / `agena-ui` JSON 围栏都不再解析为富内容；对应适配器、工具结果卡片
与复制回退已删除。旧格式按普通文本／代码显示，不执行应用代码，也不自动转换或迁移。

`prepare` 只准备文档，`validate` 只校验，UI 必须由正文中的围栏引用触发。
独立的 Web Preview 和预览应用运行功能没有更改，不属于这套回答协议。

## 验证

```sh
cargo test -p agena-domain -p agena-bundled-plugins -p agena-runtime-tools \
  -p agena-tui-transcript -p agena-application inline_ui --offline
cargo test -p agena-runtime-session --lib session_prompt --offline
cargo check -p agena --all-targets --offline

cd packages/agena-web
bun test ./tests
bun run build
PLAYWRIGHT_MODULE=/path/to/playwright CHROME_BIN=/path/to/chrome bun run test:inline-ui:browser
```

共享样例 `examples/inline-ui/references.json` 覆盖正常引用及普通代码、嵌套围栏、未闭合围栏、引用块、列表、HTML、CRLF、
重复标记和未完成流式标记。真实存储测试检查会话授权、不可变结果和 4 字节分页下的引用恢复。
浏览器检查渲染真实最终回答组件，覆盖正文中两个 UI 与文字交错、准备结果不重复显示、完整回答复制、
代码／用户示例保持原文、历史缺少工具缓存、跨页 SSE、会话切换、浅／深色和 375px 窄屏。
截图及结果保存在忽略提交的 `artifacts/inline-ui/body-*`。

变更工具定义后须重新生成 bundled capability identity snapshot 和 tools reference。
Rust 类型仍是文档 Schema 的唯一来源，前端类型由 `agena-web-types` 生成。
