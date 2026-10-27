# Inline Generative UI（富内容回复）

Inline Generative UI 是回答的一种表达方式，而不是独立应用。AI 用普通文字解释，
在有助于理解时插入步骤、对比、提示、指标、表格、补充说明或图表。
Web 和 TUI 读取相同的语义内容；打开历史回答不会写入配置、启动进程或请求外部数据。

## 给 Agent 的渐进式入口

内置插件 ID 是 `agena.inline_ui`，默认随其他 bundled plugins 注册，可按现有插件设置禁用。
模型看到的简短工具目录引导其按需发现，不把所有内容类型的完整结构塞进系统提示。
实际调用名由当前工具目录决定，下列名称为 canonical identity：

| 工具 | 用途 |
| --- | --- |
| `agena.inline_ui.guide` | 不传 topic 时返回简短概览；传入单一类型时只返回该类型的 Schema 和完整示例。 |
| `agena.inline_ui.validate` | 可选的发布前校验；成功或失败都不产生富内容段落。 |
| `agena.inline_ui.present` | 校验后在当前会话顺序中发布一个不可变的富内容段落。 |

推荐流程：先 `guide({})`，按需 `guide({"topic":"steps"})`，然后调用 `present`。
有疑问时先 `validate`；错误包含具体字段路径。无效文档不会部分发布。

```json
{
  "document": {
    "version": 1,
    "title": "推荐做法",
    "blocks": [
      {
        "type": "paragraph",
        "id": "intro",
        "text": "先统一数据契约，再逐步改进展示。"
      },
      {
        "type": "steps",
        "id": "plan",
        "items": [
          {"label":"P0","title":"定义内容","text":"让同一份内容可用于 Web 与终端。","icon":"layers"},
          {"label":"P1","title":"验证体验","text":"检查窄屏、键盘操作、复制与历史回放。","icon":"check"}
        ]
      }
    ]
  }
}
```

这是工具参数，不应作为 JSON 或特殊 Markdown 围栏直接发给读者。
`present` 的内容已经属于回答；可在前后继续解释，不要在最后又重复一遍。
单个文档支持交错的普通段落与富内容块；多个发布调用的顺序很重要时应串行调用。
这不是任务管理器：步骤标签和图标仅表达讲解结构，不表示某项工作已执行或完成。

## 内容目录

| 类型 | 语义及限制 |
| --- | --- |
| `paragraph` | 连接段落，最多 6,000 个 Unicode 字符。 |
| `callout` | note、tip 或 warning；正文最多 3,000 字符。 |
| `steps` | 1–12 个阶段；可选短标签、受控图标；单项正文最多 3,000 字符。 |
| `comparison` | 2–4 个选项、1–16 个对比维度；每行值数与选项数一致。 |
| `metrics` | 1–6 个有来源的值，含单位和可选背景说明。 |
| `table` | 1–8 列、1–40 行；每行列数一致；不承载无限数据集。 |
| `details` | 补充说明；Web 可折叠，TUI 与复制内容保留完整正文。 |
| `chart` | line、bar、area、pie；1–128 个分类标签、1–6 个序列。 |

图表的 X 轴是分类序列，不是任意数值 X／不等间隔时间轴。空值 `null` 表示缺失，
不能当成零值；饼图只接受单个非负序列且至少有一个正值。TUI 用包含单位的精确数据表
表达图表内容。不要虚构数值；演示数据必须标明为演示，来源与不确定性不能被视觉排版省略。

所有文本字段均为**纯文本**，不是 HTML、Markdown、模板或代码。
文档必须为 `version: 1`，包含 1–24 个块，序列化后不超过 60 KiB。
此上限为运行时的 64 KiB 内联结果预算保留包装与元数据空间，避免历史内容被转存为不再可直接渲染的分页结果。
块 ID 在文档内唯一，由 1–80 个 ASCII 字母、数字、下划线或连字符组成。
标题最多 160 字符；服务端拒绝未知字段、未知版本、终端控制字符以及越界数据。

## Web、TUI 和复制

Web 使用与正文一致的字体、主题和间距，减少不必要的卡片边框。
步骤是有序列表；表格使用真实表格语义；窄屏表格在自己的容器中滚动；说明可用键盘展开。
展示本身不使用 iframe，不执行模型生成的脚本，不访问声明以外的数据源。
图表保留悬停、图例、原始数据和 SVG 导出等局部交互。

富内容不藏在工具折叠区内。原始调用仍可通过“查看原始结果”检查，错误也不会被伪装成成功。
TUI 在工具折叠时同样显示完整讲解，用现有终端排版支持标题、步骤和表格。
复制操作使用语义投影，不从 DOM 抓取布局；共享 fixture 验证 Web 与 Rust 的复制内容完全一致。
终端导出包含折叠说明与图表的完整数据。

生成过程使用正常工具调用的生命周期，待文档完整校验成功后发布。不会把半个 JSON
当成可交互界面，也不声称支持任意 token 级的组件流式渲染。

## 架构与历史兼容

持久化唯一事实源仍是工具 `RawOutput`，`inline_ui` 数据由 Rust 类型定义。
`ViewBlock::InlineUi` 是读时投影，不另存一份可覆盖的 App／Surface 记录。
版本号是内容协议版本，不是一个可变应用版本；历史消息不能因为重新挂载而覆盖新消息。

TypeScript 类型由 `agena-web-types` 从 Rust Schema 生成；Web 入口另做有边界的防御式校验。
后端只为 `agena.inline_ui.present`／正常解析后的 `inline_ui.present` 生成该富内容投影。
`guide` 和 `validate` 的结果保留普通工具展示，不会误发布为回答内容。

旧的 `agena-chart` 和 `agena-ui` 消息继续使用原有只读解析／应用实现以兼容历史；
新系统提示不再要求生成这些围栏。图表展示实现已迁移到 `features/inlineUi`，
历史 chart parser 通过 re-export 使用相同的图表实现。
原有 Interactive App／Web Preview 功能未改造成 Inline Generative UI，二者职责保持分离。

## 开发与验证

```sh
cargo test -p agena-domain -p agena-bundled-plugins -p agena-runtime-tools \
  -p agena-tui-transcript -p agena-web-types inline_ui --offline
cargo run -p agena-web-types --offline > /tmp/agenaState.ts
cmp /tmp/agenaState.ts packages/agena-web/src/generated/agenaState.ts
cargo run -p agena-domain --example inline_ui_markdown --offline > /tmp/inline-ui.md
cmp /tmp/inline-ui.md examples/inline-ui/reply.md

cd packages/agena-web
bun run test:inline-ui
bun run typecheck
bun run build
```

浏览器检查使用独立 Vite 服务、示例数据和真实聊天组件，不启动模型，不连接正在运行的
Agena 服务。安装 Playwright 后运行；也可以通过 `PLAYWRIGHT_MODULE` 指向现有安装：

```sh
PLAYWRIGHT_MODULE=/path/to/playwright CHROME_BIN=/path/to/chrome bun run test:inline-ui:browser
```

覆盖折叠工具、8 类内容、键盘操作、相同块 ID 更新时保留展开状态、浅／深色主题、
375px 窄屏、复制一致性、恶意文本不执行，以及展示时不产生后端请求或写入。
截图与检查记录输出到 `artifacts/inline-ui/`（忽略提交），可用 `INLINE_UI_ARTIFACTS` 指定路径。

新增或调整类型时同时更新 Rust 校验、Web 防御校验、共享样例及复制契约测试，
再重新生成前端类型、bundled capability identity snapshot 与 tools reference。
