# Agena 附件链路审查与修复

审查日期：2026-10-28。基线：`d8bf1cd55`。修复分支：`fix/attachments-end-to-end`。

本次在独立 worktree 中检查附件从浏览器/TUI 到上传暂存、消息入队、不可变 Blob、Provider 输出、历史读取、预览和下载的完整路径。未修改 Provider 工具路由选择；未使用生产会话数据库作为测试库；未调用收费图片生成服务。本文区分代码路径确认、自动化回归和真实浏览器组件测试，不将测试替身等同于云端生产验证。

## 1. 数据流与访问边界

普通文件上传保存到 workspace 的私有 uploads 目录。惰性文件引用保持为 WorkspacePath，不擅自读取并发送给模型。显式 model_input 在发送前经过文件授权、类型/大小校验，保存为不可变 Blob；消息只保留 `agena:media:v1:` 引用及校验和。原始上传文件随后移动或改变，不应影响已经冻结的模型输入。

Hosted 生图结果走独立媒体持久化，不把 base64 塞回 512 KiB 控制 JSON。文件属于工作区下的 generated_images/session 目录，工具结果保留路径、大小和 SHA-256。模型输入、附件预览、历史继承是不同权限问题：浏览附件不应依赖当前选择的模型，但仍必须由服务器检查会话可见性；分叉会话继承的是历史记录，不是原会话整个目录的读取权限。

## 2. 已确认的问题及修复

| 编号 | 原问题 | 本次实现 | 主要位置 |
| --- | --- | --- | --- |
| A01 | 冻结成内部媒体引用的图片没有浏览器可用的下载入口 | session media 端点读取不可变 Blob；检查版本、会话成员关系、SHA-256、长度 | runtime-session/kernel/media.rs；application/service/workspaces.rs |
| A02 | 落盘生成图片被 Web 发到不适用的 workbench 原文件入口 | 将附件路径和 opaque URI 交给当前会话媒体端点；图片打开查看器，只有确属当前工作区的文件才打开编辑器，不混用旧会话与当前目录 | web/lib/attachmentLinks.ts；AgenaAttachmentPreview.vue |
| A03 | 排队消息的原图片已转成 Artifact，但编辑校验只允许 WorkspacePath | 接受内部不可变引用进入后续处理，仍由存储检查所属会话和内容身份 | application/session.rs |
| A04 | 中文等 Unicode 文件名被转换为下划线；超长字符名可能超过文件系统字节限制 | 共享 basename 清理器，保留 Unicode，限制 200 UTF-8 字节并保留扩展名；去控制/路径语法和保留设备名 | domain/attachment.rs；上传、下载、模型媒体准备 |
| A05 | HTTP 下载只有 ASCII filename，客户端不识别 filename* | 输出 ASCII fallback 与 UTF-8 filename*；客户端优先解析 UTF-8，保留字面加号并清理路径 | api-server/rest/attachment_response.rs；client/http.rs |
| A06 | 工作区内的合法绝对文件路径也被拒绝下载 | canonicalize 后按路径组件检查工作区包含关系；拒绝前缀碰瓷与越界 | application/service/workspaces.rs |
| A07 | 上传直接写最终文件名，失败/中断可能留下半成品 | 私有临时文件完整写入后原子发布，失败不发布部分内容；最终文件保持私有权限 | upload_workspace_file_in |
| A08 | 预览与下载均为 octet-stream/attachment，缺少安全的内联媒体类型 | 从实际字节识别 MIME；栅格图片、音视频、PDF 可预览；HTML/XML/SVG/未知内容保持下载，增加 nosniff、私有 no-store 与 CSP | attachment_response.rs |
| A09 | 缺少浏览器媒体定位/续传需要的 Range 与正确 HEAD 响应 | 支持单范围、后缀范围、206/416、Content-Range、HEAD；不支持的多范围/无效语法回到完整响应 | attachment_response.rs |
| A10 | 生成图片仅验证 base64，非图片或损坏内容也可落盘 | 检查实际格式、声明 MIME、尺寸、像素及有界解压；扩展名不受 Provider 文件名提示控制 | runtime-tools/project_paths.rs |
| A11 | 生成图片重试可能覆盖已发布文件 | 使用不覆盖的原子创建；已存在时仅允许逐字节一致的幂等验证 | project_paths.rs |
| A12 | 坏 base64、权限拒绝等永久错误被归类为 Storage，可能不断重试 | 永久媒体错误结束处理；只有 Interrupted/WouldBlock I/O 走可重试存储错误 | runtime-session/kernel/processor.rs |
| A13 | 图片从控制预算移出后，待提交媒体缺少独立内存计费；解码大小边界不精确 | 按解码长度/聚合数量检查，保留独立 ReadLease，控制 JSON 原限额不变 | processor.rs |
| A14 | 分叉消息可见，但原会话生成图片路径不可读 | 根据可见工具记录授权精确附件路径，再校验文件长度/SHA；不扩大到原会话目录 | authorized_tool_attachment；session media 服务 |
| A15 | 某些兼容网关在 output_item.added 已返回 completed 图片，唯一图片结果被当成 Started 丢弃 | 有终态和图像结果的 initial item 转为 Completed，普通 in_progress 仍等待终态 | openai_provider_native_tools.rs |
| A16 | 资源 kind 可能继续采用客户端提示而不是已校验内容 | 由 PreparedMedia 的真实类型更新资源 kind/MIME/尺寸 | manager/media/mod.rs |
| A17 | TUI 仅识别 local_path，不识别内部 Blob、Provider artifact 或 composer resource | 补齐引用发现和展示层本地化，内部 URI 直接请求当前会话媒体端点 | tui-app/app_backend/transport.rs |
| A18 | TUI 图像缓存无总量上限、只用路径作键、整表反复拷贝，可能过期复用或混用会话 | 用会话/引用/内容版本作为缓存键，不缓存无版本可变路径；缓存、批次和实际展示副本分别限制 64 MiB；用 Arc 避免整表复制 | transport.rs |

## 3. 保持的容量与协议约束

- 上传：50 MiB；普通文件下载：100 MiB。上限不等于模型可接收大小。
- 显式模型附件：20 MiB/项、40 MiB/批、最多 8 项；文本和图像另有内容约束。
- 当前 session generated-image 展示和 Blob 下载：20 MiB；Hosted 图片还受聚合媒体预算和原控制数据预算限制。
- 生成图片验证保留 16384 像素边长/40 megapixel 上限，并在解压时限制内存。
- `Range` 仅实现单范围。多范围或无法解析的范围按完整响应处理，不假装返回 multipart。没有强验证器时不应用 If-Range。
- 不因为文件名是 png 就把 HTML 作为可执行的同源预览；安全的下载响应保留原始字节而不是重编码图像。

## 4. 验证方法

### Rust

使用 worktree 自己的 Cargo target 和隔离 AGENA_DATA_DIR；不复用主工作区 target，避免不同代码版本共享 fingerprint 导致误导性编译结果。

```sh
export CARGO_TARGET_DIR="$PWD/target"
export CARGO_INCREMENTAL=0
export AGENA_DATA_DIR="$PWD/target/attachment-audit/test-state"
cargo test -p agena-domain -p agena-runtime-tools -p agena-runtime-session \
  -p agena-runtime-provider-adapters -p agena-client -p agena-application \
  -p agena-api-server -p agena-tui-app --lib --offline --locked -j 6 -- --test-threads=4
cargo clippy -p agena-domain -p agena-runtime-tools -p agena-runtime-session \
  -p agena-runtime-provider-adapters -p agena-client -p agena-application \
  -p agena-api-server -p agena-tui-app --all-targets --offline --locked -j 6 -- -D warnings
```

新增用例覆盖真实 SQLite Blob 的未发布拒绝、当前会话允许、无关会话拒绝、fork 继承、伪造大小/校验和/版本拒绝；生成文件继承必须命中可见工具精确附件；文件系统越界/符号链接/前缀混淆；Unicode/长度/设备名/控制字符；上传原子发布、权限、字节与哈希；HTTP Range/HEAD/文件名与被伪装的 MIME；合法/损坏图像；幂等发布；永久错误不重试；TUI 引用类型和缓存隔离。

### Web 与真实 Chromium

```sh
cd packages/agena-web
bun test tests
bun run typecheck
PLAYWRIGHT_MODULE=/path/to/playwright-core CHROME_BIN=/usr/bin/google-chrome \
  node scripts/probe-attachments.cjs
```

浏览器探针启动仅绑定 loopback 的独立 Vite fixture，挂载真实 Vue 附件组件，通过本地确定性 HTTP 媒体响应验证：生成图/不可变图都加载成功；打开按钮使用正确会话地址；文件真实下载并保留中文名；下载字节与 PNG fixture 完全一致；不可用附件无预览、无下载、无请求；页面没有 JavaScript 错误。浏览器测试没有借用生产后端或云服务。

结果日志、浏览器 JSON 报告与截图存放在 `target/attachment-audit/`。

### 最终执行结果

| Rust crate | 通过 | 失败 | ignored |
| --- | ---: | ---: | ---: |
| `agena-api-server` | 6 | 0 | 0 |
| `agena-application` | 13 | 0 | 0 |
| `agena-client` | 5 | 0 | 1 |
| `agena-domain` | 2 | 0 | 0 |
| `agena-runtime-provider-adapters` | 10 | 0 | 0 |
| `agena-runtime-session` | 43 | 0 | 0 |
| `agena-runtime-tools` | 17 | 0 | 0 |
| `agena-tui-app` | 61 | 0 | 0 |
| 合计 | **157** | **0** | **1** |

`ignored` 项是项目既有的 `proxy_environment_child`，其声明说明由代理测试在隔离进程中调用，不是本次附件测试被跳过。

Web：**37 项通过，0 失败**；Vue/TypeScript 类型检查通过；Vite 生产构建通过。真实 Chromium 探针 **10 项检查通过，页面 JavaScript 错误为 0**；测试得到的中文文件名及下载字节均正确。

质量检查：上述 8 个 crate 的 `cargo clippy --all-targets -- -D warnings` 通过；`cargo fmt --all --check` 与 `git diff --check` 通过。

最终日志：`verified-rust-tests.log`、`verified-web.log`、`clippy.log`；浏览器记录：`browser/report.json`、`browser/attachments.png`，均位于 `target/attachment-audit/`。这些是隔离验证产物，不进入源码提交。


## 5. 审查边界与未夸大的保证

这些测试验证了此分支的关键附件链路和明确边界，不是“所有可能输入都不存在问题”的证明。没有重放生产会话 43 的收费云端请求，没有接入所有第三方 Provider，没有在每一种真实音视频编解码器和操作系统上播放媒体。

普通下载仍沿用既有的有界整文件读取，新增 Range 响应会无拷贝切片响应体，但这不等于磁盘/Blob 流式下载。历史版本已丢失的图像 bytes 不会凭空恢复。本次不做未经引用追踪的暂存文件按年龄批量删除；文件引用仍可能有效，贸然回收会造成数据丢失。已有生产历史未被迁移或重写。

本次保持工具路由、Provider 选择和权限策略来源不变，仅修复附件传输、保存、校验、可见性与展示链路。独立分支保留待 review，未合并主分支。
