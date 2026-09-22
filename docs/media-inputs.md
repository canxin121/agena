# 显式媒体输入、云端文件与剪贴板

本次重写把四种行为分开：本地资源引用、用户直接发送给所选模型、AI 显式调用云端理解、创建可复用的厂商文件。界面能预览图片，不等于模型已经收到图片；本地文件存在，也不等于有权发给任意服务商。

## 用户直接发送

Web 的文件选择、拖放、剪贴板图片/文件和长文本粘贴进入同一个本地草稿队列。默认标记为“将内容发送给所选模型”，面板明确显示所选 Provider；可切换成仅引用。粘贴和预览不会自行调用模型。发送动作捕获当时的模型配置、文本与附件快照。

点击发送后，浏览器文件先通过 Agena 的工作区文件上传接口保存为本地 `.agena/uploads/` 资源；这个步骤不是厂商 Files API。服务端返回内容 SHA-256，Composer activity 将它作为 `provenance.content_hash` 带回。用户已有工作区文件可直接作为引用，只有显式选择 model_input 才准备内容。

运行时检查工作区边界、读取权限、文件内容版本、格式、总量和模型输入能力。真正的 model_input 被转换为一次固定字节快照，再以 ProviderData 绑定选定 Provider/model/adapter 及端点/凭据配置形状的摘要。发送历史时再次检查绑定；换模型、端点或账号不能静默重发先前媒体。没有可确认的模态支持时拒绝，而不是把媒体降级成“图片文件名”后假装看过。

本地引用保持 reference；明确的 reference 即使含 URL、旧 Provider ID 或 Base64，也不会转成视觉输入。它只是资源说明，之后若 AI 要读取/上传，应另外调用有明确权限的工具。历史没有 delivery 标记的内容保留已有兼容路径，不伪造新的上传授权。

## AI 的云端工具

每家服务商新增五个公开入口，总计 15 个：

| OpenAI | Anthropic | Google |
| --- | --- | --- |
| `chatgpt.cloud_image_understanding` | `claude.cloud_image_understanding` | `gemini.cloud_image_understanding` |
| `chatgpt.cloud_document_understanding` | `claude.cloud_document_understanding` | `gemini.cloud_document_understanding` |
| `chatgpt.cloud_file_upload` | `claude.cloud_file_upload` | `gemini.cloud_file_upload` |
| `chatgpt.cloud_file_status` | `claude.cloud_file_status` | `gemini.cloud_file_status` |
| `chatgpt.cloud_file_delete` | `claude.cloud_file_delete` | `gemini.cloud_file_delete` |

图片/文档理解是多模态模型请求，不虚构厂商 `tools` 类型。OpenAI 使用 Responses 图片/文件输入，Anthropic 使用 Messages 内容块，Google 使用 generateContent 内容块。普通分析默认内联输入，不因为“看图”就暗中建立一个远端长期文件。需要复用时，AI 显式先上传，再使用该 Provider 的受管 handle。

### 分析本地图片

```json
{
  "inputs": [{"source":"local","path":"screenshots/error.png","expected_sha256":"读取时确认的SHA256"}],
  "prompt":"解释图中可见的错误；看不清的部分明确说明，不要补猜。",
  "detail":"high"
}
```

`detail` 仅 OpenAI 支持 low/high/auto；其他服务商传非 auto 会明确拒绝，不静默忽略。模型必须在插件配置或本次参数中明确指定。图片输入不需要先调用 `fs.read` 来搬运 Base64。

### 上传、复用、清理

```json
{"path":"reports/analysis.pdf","expected_sha256":"已确认SHA256"}
```

上传返回 `media_<32 hex>` 形式的受管 handle。分析复用：

```json
{
  "inputs":[{"source":"cloud","handle":"上传返回的media_句柄"}],
  "prompt":"概括文档中的结论，并区分证据与推测。"
}
```

状态/删除：

```json
{"handle":"上传返回的media_句柄"}
```

不接受任意原始厂商 file_id。句柄映射由工作区本地的签名记录管理，绑定工作区、会话、厂商和连接；不能跨会话、换账号或改记录后复用。厂商 ID 仅用于审计和已有受管映射，不是调用凭证。

上传前先原子保存 submission_unknown 记录；如果连接中断、调用被取消或响应丢失，同一 call_id 不自动再次上传。状态可能是 ready、processing、expired、deleted、deletion_unconfirmed、submission_unknown 或 unknown/response_invalid；只有实际确认才使用确定状态。

同一进程内，上传、状态与删除使用已有工作区异步门闩串行处理，避免延迟状态结果覆盖确认删除。这不是跨进程分布式事务或远端 exactly-once 保证。远端已接收但无法取得 file_id 的情况需在厂商侧核查，不盲目重试。

OpenAI/Anthropic 默认请求一天过期，可显式设置本实现允许的 1 小时至 30 天；Google 使用厂商自己的生命周期，显式传 expires_in_seconds 会被拒绝，不能伪装成设置成功。分析拒绝已知过期文件。删除回执表明 API 逻辑删除被确认，不声称后端物理副本立即清除。

## 本地文件与 fs.view_image

`fs.view_image` 退役，不再把“本地路径附件”称为模型已视觉检查。旧名称有明确迁移提示，历史展示保留。`fs.read` 的二进制附件模式仍能提供本地资源引用/元信息，但不会编码成隐式模型输入；说明文字明确“未发送媒体内容”。文本读取继续作为普通工具结果参与当前会话，这不等于文本永远不出本机。

云端生成图片和截图工具可以继续产出本地工件；要让 AI 分析这些工件，需显式使用云端理解入口或用户直接发送。

## 剪贴板、队列与恢复

Web clipboard 使用浏览器授权的 paste 数据，不后台读取剪贴板。文件在 items/files 同时出现时不重复插入；同名同大小但内容不同的图片不被误去重。短文本与图片混合粘贴保留文字；超过行内展示阈值的文本按 UTF-8 文本附件准备，读取失败回退文字。超过 1 MiB 的粘贴文本在 Web/TUI 都拒绝插入和上传，明确提示并保持剪贴板、当前草稿不变。

队列串行检查名额和总字节预算；清空草稿或切换会话会使旧异步结果失效，新草稿不等待已经取消的旧 Promise。读取/上传准备中，按钮、快捷键和提交函数均不能提前发送。Web 发送失败只在草稿未变化时直接恢复；已有新草稿或切换会话时保留一个显式恢复槽，不覆盖新内容，也不自动重传。恢复槽占用期间阻止下一次发送，用户可回原会话恢复或明确丢弃。此恢复槽是当前页面内存，不承诺浏览器关闭后仍存在。

TUI 的截图、文件路径和长文本粘贴同样进入受限准备流程。待处理媒体计入八个名额；submit、queue 和 steer 都在准备期间拒绝提前提交。草稿 epoch/slot 阻止旧回调插入新会话。内部临时截图文件由拥有者清理，不删除剪贴板指向的用户原文件。PNG 转码前检查尺寸、乘法溢出和像素预算，编码有字节上限。

## 统一限额与格式

- 单输入最多 20 MiB；每次用户消息最多 8 个输入、总计 40 MiB。
- 云端理解的内联二进制总量额外限制为 12 MiB，避免 Base64 和 JSON 膨胀后超出请求预算；更大文件使用显式上传与 handle。
- 图片支持 PNG/JPEG/GIF/WebP，按实际内容检测；最大单边 16384、4000 万像素；在内存限额下实际解码，不仅检查扩展名或头部。
- 直接图片理解只接受图片；文档理解接受 PDF 和有界 UTF-8 文本，不把图片悄悄塞进文档工具。
- 文本附件最多 1 MiB；二进制/格式与扩展名明显不符的文件在发送前失败。PDF 内容不在本地宣称已语义解析，实际有效性仍由厂商处理。
- 用户音频/视频可作为明确 model_input，但必须有当前模型已确认的模态能力和相应适配器支持；本次新增的独立理解工具主要是图片、PDF/文本，不宣称新建了全厂商音视频理解工具。

## 数据保存与可观察状态

用户显式 model_input 的内容快照随会话存储，以便同一路由重放；这不是零留存方案，也没有增加数据库静态加密。摘要/缓存身份只包含内容摘要，不嵌入完整 Base64。ProviderData 不因历史重放被静默搬到另一目标。

云端工具回执区分本地准备、请求已发送、远端文件建立、分析完成、删除确认和持久化失败。媒体文件内容不写进远端映射记录；记录只保留校验信息和签名。API key 不写入回执，resumable URL 不得跨源，向会话 URL 提交字节时不转发凭据；错误日志去掉可能含 token 的请求 URL。

厂商文件遵循各自数据保留规则，不能将 expires_at 或 deleted=true 描述成完全物理擦除。依据：

- https://developers.openai.com/api/reference/resources/files/methods/create
- https://platform.claude.com/docs/en/build-with-claude/files
- https://ai.google.dev/gemini-api/docs/files

## 验证范围

所有新增云端测试使用本机合成 HTTP 服务和假凭据，覆盖实际图片/PDF字节、内联/远端引用、拒绝先于请求、handle归属、签名篡改、过期、取消、幂等重试和删除确认。直接发送测试检查固定内容、路由变化和能力拒绝。剪贴板测试只使用合成字节/文件，不读取真实用户剪贴板。

Web 验证包括纯输入队列/恢复测试、全量 frontend tests、vue-tsc 和生产 build；TUI 验证包括准备状态提交门闩与平台剪贴板边界测试。最终命令和通过数量以 `docs/media-inputs-status.json` 和对应日志为准，不能把编译成功当作真实付费服务认证。
## 本次源码验收结果

> 验证时的 commit/push 状态是历史快照；对应实现现已包含在 `ef00f41f`，当前远端状态以 Git 为准。

最终选定 Rust 库与集成测试 **1266 项通过，0 失败，0 项忽略**；Web 测试 **485 项通过，0 失败**。vue-tsc、Vite 生产构建、11 个核心 crate 的严格 `--all-targets` Clippy、7 个下游运行时/API/MCP/TUI/CLI目标编译、生成契约及目录检查、19 项仓库不变量、格式和 diff 检查全部通过。

当前目录包含 134 个执行工具：32 个厂商云端工具（新增 15 个媒体/文件操作），`fs.view_image` 已退役，原生文本/终端/编辑等能力保留。AI 与用户直接发送两条入口都有独立测试；测试不是只检查附件标签或元信息，而是检查实际发送的字节、目标约束、失败回执和草稿不丢失。

每项 gate 使用同一源码指纹并保存真实退出码、日志哈希和前后指纹一致性。指纹为：

```text
df5e13453c6227c7f8a8d6c6c2b7741515a4e1a7f20bdba5b84d888b19f10c1b
```

机器可读记录：`docs/media-inputs-status.json`。完整命令与日志：`.tmp-artifacts/media-rewrite/final/final2-results.json`。此前失败的编译/回归日志仍保留，不以旧测试结果替代新代码验收。

本次没有 commit/push、重启正在运行的服务、读取真实剪贴板、调用真实付费厂商 API，或开启新的生产权限。部署后才会加载新工具目录及 Web/TUI交互契约；数据保留与平台范围仍按上文边界说明。
