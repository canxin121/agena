# 厂商云端工具边界

Agena 的厂商插件只暴露**当前存在、由厂商侧实际执行或托管的数据/模型能力**。本地 Shell、文件编辑、浏览器、记忆、MCP 调用、子任务等能力继续由 Agena 原生工具实现。两条执行路径不互相兜底，也不自动改写。

本仓库没有正式发布版本，也不提供旧工具名、旧回调形状或旧配置的迁移层。工具目录只描述当前契约；未注册名称就是普通未知工具。

## 当前云端工具

### OpenAI

- `chatgpt.cloud_web_search`
- `chatgpt.cloud_file_search`
- `chatgpt.cloud_code_interpreter`
- `chatgpt.cloud_image_generation`
- `chatgpt.cloud_image_edit`
- `chatgpt.cloud_shell`
- `chatgpt.cloud_image_understanding`
- `chatgpt.cloud_document_understanding`
- `chatgpt.cloud_file_upload`
- `chatgpt.cloud_file_status`
- `chatgpt.cloud_file_delete`

### Anthropic

- `claude.cloud_web_search`
- `claude.cloud_web_fetch`
- `claude.cloud_code_execution`
- `claude.cloud_advisor`
- `claude.cloud_image_understanding`
- `claude.cloud_document_understanding`
- `claude.cloud_file_upload`
- `claude.cloud_file_status`
- `claude.cloud_file_delete`

### Google

- `gemini.cloud_code_execution`
- `gemini.cloud_url_context`
- `gemini.cloud_google_search`
- `gemini.cloud_file_search`
- `gemini.cloud_google_maps`
- `gemini.cloud_image_generation`
- `gemini.cloud_image_edit`
- `gemini.cloud_image_understanding`
- `gemini.cloud_document_understanding`
- `gemini.cloud_file_upload`
- `gemini.cloud_file_status`
- `gemini.cloud_file_delete`

共 32 个公共云端工具。公共名称中的 `cloud_` 表示执行位置；发送给厂商 API 的 operation/type 仍使用厂商协议自己的名字。

## 执行边界

厂商插件可以：

- 向配置的厂商 API 发起网络请求；
- 发送用户或模型显式提供、且已通过 Agena 权限检查的输入；
- 接收厂商托管工具结果、来源、usage 和回执；
- 对云端文件执行显式上传、查询和删除；
- 把云端生成的图片保存为 Agena 管理的本地工件。

厂商插件不会：

- 把本地项目目录自动暴露给云端；
- 执行厂商响应里要求客户端执行的 function/computer/local-shell/patch 回调；
- 将云端工具失败后自动改成本地工具；
- 将未知工具名映射到另一个工具；
- 隐式连接 MCP 服务器或调用本地记忆、文件编辑、浏览器；
- 因请求历史里出现客户端 callback/result 而在本机重放它。

若厂商响应包含必须由应用执行的动作，Agena 将其视为执行边界违规并阻止继续，不会执行或重新提交该动作。

## OpenAI hosted shell

`chatgpt.cloud_shell` 只允许 OpenAI 托管容器：

- 默认：`environment.type = "container_auto"`
- 已存在容器：`environment.type = "container_reference"`，并提供 `cntr_...` ID

本地环境、空环境、字符串环境和未知环境类型都在网络请求前拒绝。它不是 `shell.run` 的别名，也不能访问 Agena 工作区，除非内容被显式作为允许的输入发送。

## 代码执行

- OpenAI Code Interpreter 只接受厂商容器语义；
- Anthropic Code Execution 由 Anthropic 服务端执行；
- Gemini Code Execution 由 Google 服务端执行。

这些能力与 Agena 本地 `shell.run` / `shell.write` 完全独立。需要访问本地仓库、终端状态或本机依赖时，使用 Agena 原生工具。

## 图片、文档和云端文件

图片/文档理解和文件生命周期工具遵循 `docs/media-inputs.md`：

- 本地文件先经过工作区边界、权限、大小、格式和哈希校验；
- “本地引用”不会自动变成模型输入；
- “model_input” 或显式云端理解会发送固定内容快照；
- 云端文件使用 Agena 管理 handle，不接受任意裸厂商 file id 作为跨会话凭证；
- 上传、状态、删除都保留明确的远端不确定状态，不假装 exactly-once。

## 请求历史

每家厂商只接受其当前协议允许的历史对象。Agena 在发送前递归检查已知协议 envelope：

- 禁止注入第二套 function/tool declarations；
- 禁止客户端执行结果和 callback 历史；
- 禁止 MCP/tool-search/programmatic-execution 等额外执行路由；
- 检查有深度和节点预算，超出预算时失败关闭。

普通文档内容里恰好出现 `"type":"function_call"` 之类文本不会被误判；检查只遍历已知协议 envelope 字段。

## 执行证据与结果

公共结果会明确记录云端边界，例如：

- `execution_boundary = "vendor_hosted_only"`
- `execution_location = "vendor_cloud"`
- `local_actions_executed = false`
- `client_execution_allowed = false`
- `execution_provider`
- `public_tool_name`
- `response_id` / request id（厂商提供时）
- `usage`
- `response_receipt`
- `execution_evidence`

执行证据区分：

- 已观察到的厂商托管调用；
- 已完成的托管调用；
- 尚未得到结果配对的托管调用；
- 厂商返回的服务端错误；
- 客户端执行边界违规。

原始厂商状态只作为厂商报告事实保存，不能覆盖 Agena 对执行边界和结果的分类。

## 权限与凭据

厂商云端工具仍受 Agena 的工具 allowlist、文件读取/写入、网络和 artifact 权限控制。API key、端点和模型来自当前配置；插件不会读取其他厂商凭据，也不会把凭据写入结果回执。

本地原生工具继续使用各自的权限模型。云端工具不会因为已有本地权限而自动获得本地文件；文件必须作为该次调用的显式输入。

## 当前契约原则

- 工具目录只有当前公共名称。
- 配置只接受当前字段。
- 厂商 callback 不在本地自动执行。
- 云端与本地能力没有隐式 fallback。
- 不维护旧工具名、旧会话工具 identity 或权限名称迁移表。
- 任何不符合当前 schema/协议的 Agena 自有数据都直接拒绝，由开发环境重建或重新创建。

生成的完整工具 schema 与 help 以
`crates/agena-bundled-plugins/generated/tools-reference.md` 为准。
