# 显式媒体输入、云端文件与剪贴板

Agena 把四种行为严格分开：

1. 本地资源引用；
2. 用户把内容显式发送给当前模型；
3. AI 显式调用厂商云端理解工具；
4. AI 显式创建、查询或删除可复用的厂商云端文件。

界面能预览文件，不表示模型已经收到内容；本地文件存在，也不表示它可以被发送给任意服务商。

## 用户直接发送

Web 的文件选择、拖放、剪贴板图片/文件和长文本粘贴进入同一草稿准备队列。默认的 model input 会明确显示目标 Provider；用户也可以只创建本地 reference。

发送前，本地文件会经过：

- 工作区边界检查；
- 读取权限检查；
- 内容哈希和固定字节快照；
- 单文件/总大小限制；
- MIME/格式验证；
- 当前模型模态能力验证；
- 当前 Provider/model/adapter/端点/凭据配置绑定。

真正的 model input 只发送这次固定下来的内容快照。切换 Provider、模型、端点或账号后，先前的 ProviderData 不会自动重发到新目标。

本地 reference 永远只是资源说明。即使引用文本中包含 URL、Provider 名或 Base64，也不会自动提升为视觉输入或云端上传授权。

## AI 的云端理解与文件工具

每家服务商提供五个当前媒体/文件入口：

| OpenAI | Anthropic | Google |
| --- | --- | --- |
| `chatgpt.cloud_image_understanding` | `claude.cloud_image_understanding` | `gemini.cloud_image_understanding` |
| `chatgpt.cloud_document_understanding` | `claude.cloud_document_understanding` | `gemini.cloud_document_understanding` |
| `chatgpt.cloud_file_upload` | `claude.cloud_file_upload` | `gemini.cloud_file_upload` |
| `chatgpt.cloud_file_status` | `claude.cloud_file_status` | `gemini.cloud_file_status` |
| `chatgpt.cloud_file_delete` | `claude.cloud_file_delete` | `gemini.cloud_file_delete` |

图片/文档理解是正常多模态模型请求，不虚构一套本地“视觉检查”状态。OpenAI 使用 Responses 图片/文件输入，Anthropic 使用 Messages 内容块，Google 使用 generateContent 内容块。

普通分析默认使用一次性显式输入；需要复用时先显式上传，再通过 Agena 管理的 handle 使用远端文件。

### 分析本地图片

```json
{
  "inputs": [
    {
      "source": "local",
      "path": "screenshots/error.png",
      "expected_sha256": "读取时确认的SHA256"
    }
  ],
  "prompt": "解释图中可见的错误；看不清的部分明确说明，不要补猜。",
  "detail": "high"
}
```

`detail` 仅 OpenAI 支持 `low/high/auto`；其他服务商传非 `auto` 会明确拒绝，不会静默忽略。

### 分析本地文档

```json
{
  "inputs": [
    {
      "source": "local",
      "path": "reports/analysis.pdf",
      "expected_sha256": "读取时确认的SHA256"
    }
  ],
  "prompt": "概括结论，并区分证据与推测。"
}
```

文档理解接受当前实现支持的 PDF 和有界 UTF-8 文本。Agena 不在本地声称已经对 PDF 做语义解析；实际内容理解由目标厂商完成。

## 云端文件生命周期

上传：

```json
{
  "path": "reports/analysis.pdf",
  "expected_sha256": "已确认SHA256"
}
```

成功后返回 `media_<32 hex>` 形式的 Agena 管理 handle。后续分析使用：

```json
{
  "inputs": [
    {
      "source": "cloud",
      "handle": "media_..."
    }
  ],
  "prompt": "概括文档中的结论。"
}
```

状态与删除都使用同一个 handle：

```json
{"handle":"media_..."}
```

Handle 映射绑定工作区、会话、厂商和连接配置。调用方不能把任意裸厂商 file id 当成跨会话凭据。

上传前会先保存本地 submission 状态。若远端请求超时、连接中断或响应丢失，同一调用不会盲目重复上传。可能出现的状态包括：

- `ready`
- `processing`
- `expired`
- `deleted`
- `deletion_unconfirmed`
- `submission_unknown`
- `unknown` / `response_invalid`

只有得到明确远端证据时才记录确定状态。Agena 不把“调用返回了”描述成 exactly-once，也不把 API 逻辑删除描述成厂商后端立即物理擦除。

OpenAI/Anthropic 当前上传路径允许本实现支持的显式过期时间范围；Google 使用厂商自己的生命周期。当某家厂商不支持某个参数时，Agena 会在请求前拒绝，不会假装参数生效。

## 本地资源与视觉分析

`fs.read` 的二进制附件模式可以返回本地资源引用和元信息，但不会把二进制自动编码成模型视觉输入。

若 AI 需要理解一个本地图片或文档，应显式调用对应 Provider 的 `cloud_image_understanding` / `cloud_document_understanding` 工具。若用户希望把媒体作为本轮消息直接交给模型，应使用 model input 路径。

云端生成图片、浏览器截图和其他工具可以继续产出本地 artifact；“产出了 artifact”与“模型已经分析 artifact”是两个独立事实。

## 剪贴板与草稿准备

Web clipboard 只处理浏览器 paste 事件提供的数据，不后台读取剪贴板。

当前约束：

- 同一文件不会因为 items/files 两种浏览器表示重复插入；
- 同名同大小但内容不同的文件不按名字误去重；
- 图片和短文本混合粘贴保留文字；
- 长文本按照有界 UTF-8 文本附件准备；
- 超过 1 MiB 的粘贴文本拒绝插入和上传；
- 文件读取、转码或上传失败不会覆盖已经产生的新草稿；
- 准备中的媒体会阻止 submit/queue/steer 提前发送；
- 清空草稿或切换会话会使旧异步准备结果失效。

TUI 的截图、文件路径和长文本粘贴进入相同的受限准备模型。内部临时截图由 Agena 清理；用户原文件不会因为草稿被清空而删除。

## 限额与格式

当前统一边界：

- 单输入最多 20 MiB；
- 每次用户消息最多 8 个输入；
- 每次用户消息媒体总量最多 40 MiB；
- 云端理解的内联二进制总量额外限制为 12 MiB；
- 文本附件最多 1 MiB；
- 图片支持 PNG/JPEG/GIF/WebP；
- 图片最大单边 16384，最大 4000 万像素；
- 二进制内容与声明格式明显不一致时在发送前失败。

更大的可复用内容应使用显式云端上传 + handle，而不是通过 Base64 扩大普通请求。

## 持久化与隐私边界

用户显式 model input 的内容快照随当前会话持久化，用于同一绑定目标的重放。这不是零留存方案，也没有额外增加数据库静态加密。

本地摘要、缓存 identity 和云端映射记录只保存必要的哈希、归属、Provider/connection 绑定和远端状态，不保存 API key。Resumable URL 等敏感传输信息不作为普通回执暴露。

厂商文件最终遵循各自的数据保留规则。Agena 的 `expires_at` 或 `deleted=true` 只描述当前能确认的 API 状态。

参考：

- OpenAI Files API
- Anthropic Files API
- Google Gemini Files API

## 当前契约原则

- reference 与 model input 是不同类型，不能隐式互转；
- 本地资源不会自动发送给 Provider；
- 云端文件 handle 只能在其绑定范围内使用；
- 云端分析不会暗中创建长期远端文件；
- 不接受旧媒体 wrapper、旧 delivery shape 或旧 Provider identity 的迁移路径；
- Agena 自有持久化数据必须匹配当前 schema，否则直接拒绝并要求重新创建；
- 测试使用合成字节和本地 fixture，不需要真实剪贴板或付费 Provider 请求。

完整厂商云端工具边界见 `docs/provider-hosted-tools.md`。
