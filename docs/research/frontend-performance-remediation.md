# 前端性能修复记录

初次记录：2026-10-06；最近补充：2026-10-07。修复基线：`eae4b96f255e216113ba9b6c0ffea86f3df20d4b`。第 1–11 节保留第一轮结果，第 12 节记录继续检查后新增的修复与验证。

本次针对「会话消息多、侧栏会话多、应用内会话标签页／分屏多时明显卡顿」实施修复。这里的窗口指应用内 pane；同一分屏组的后台标签页与另一个分屏组中仍可见的 pane 需要分别处理。

原性能工作分支已通过前端生产构建、完整前端测试及受影响后端 crate 的检查和测试；这些是记录时的验证结果。主分支整合阶段按用户要求只做编译检查，不重新执行测试或基准。CPU 基准显示多个热点的重复工作显著减少。原记录环境无法取得可用的应用内浏览器，因此没有实际页面的 FPS、输入延迟、布局耗时或长期内存曲线，不能据此承诺任意数据规模下都没有卡顿。

## 1. 会话投影、消息更新与滚动

| 问题及规模影响                                                           | 已实现的修复                                                                                                  | 主要代码                                                                                             |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| 连续 assistant 消息每加入一段都重新合并、排序，长回复的成本接近二次增长  | 先合并整组，再排序一次                                                                                        | [transcriptProjection.ts](../../packages/agena-web/src/pages/chat/transcriptProjection.ts)           |
| 每个 text part 向后查找工具，多个片段接工具时反复扫描                    | 逆向一次扫描，记录后续工具信息以确定最终回答                                                                  | 同上                                                                                                 |
| 尾部流式更新重新投影不变历史，产生新 block／part 对象并扩大 Vue 更新范围 | reply group 使用独立 computed；part 使用 WeakMap 缓存，保留未变对象身份                                       | 同上                                                                                                 |
| 工具组每次更新都拼接用于复制的全文                                       | copyText 改为访问时计算                                                                                       | 同上                                                                                                 |
| 摘要提取的正则在长空白行上反复回溯                                       | 首个有效行使用 `/\S[^\r\n]*/`，保留正文并避免反复从空白起点尝试                                               | 同上                                                                                                 |
| pane 依赖整份 messagesBySessionId，其他会话更新也可能使当前会话重新计算  | 增加按 pane／session 隔离的中间 computed                                                                      | [chat.ts](../../packages/agena-web/src/stores/chat.ts)                                               |
| 滚动每帧读取所有用户消息的几何位置                                       | 使用有序消息锚点的二分查找，只读取少量节点的矩形                                                              | [useChatScrollNav.ts](../../packages/agena-web/src/pages/chat/useChatScrollNav.ts)                   |
| 多分屏展示相同消息时 DOM id 冲突，导航可能跳到别的 pane                  | 消息 DOM id 加入 windowId，导航使用相同规则                                                                   | [MessageItem.vue](../../packages/agena-web/src/components/chat/MessageItem.vue)、useChatScrollNav.ts |
| 远离视口的消息仍持续产生布局、绘制与富内容工作                           | 消息行使用 `content-visibility: auto` 和 intrinsic size；富内容接入共享 near-viewport observer                | MessageItem.vue、[useNearViewport.ts](../../packages/agena-web/src/composables/useNearViewport.ts)   |
| 切换 session 后残留旧 activity visibility 状态                           | session 切换时清理                                                                                            | [MessageList.vue](../../packages/agena-web/src/components/chat/MessageList.vue)                      |
| 隐藏标签页卸载消息子树后，切回时阅读位置丢失，后台也可能触发历史读取     | 隐藏前保存消息 key、偏移、scrollTop 和底部跟随状态；暂停 rAF、ResizeObserver 与自动读取；显示后恢复锚点或底部 | [usePinnedScroll.ts](../../packages/agena-web/src/composables/chat/usePinnedScroll.ts)               |

投影缓存减少的是未变历史的重新转换与子组件更新。外层仍需要遍历消息以组织 reply groups；不应将它描述为每次尾部更新都与历史数量完全无关。

滚动恢复保存的是 key 和数值，避免后台标签页持有已卸载 DOM。不同 session 使用正常首次落底流程。若后台期间 transcript 被 LRU 淘汰，旧历史需要重新加载；现有实现不保证跨缓存淘汰仍精确恢复原阅读位置。

## 2. 多标签页／分屏的可见性与后台工作

新增 [workspacePaneContext.ts](../../packages/agena-web/src/app/workspace/workspacePaneContext.ts) 的 `isVisible`，与键盘焦点独立。每个分屏组的 active pane 都可见，未获得焦点的可见分屏仍能更新；同组后台标签页暂停展示相关的工作。

| 对象                               | 隐藏时的处理                                                                                        | 显示时的处理                            |
| ---------------------------------- | --------------------------------------------------------------------------------------------------- | --------------------------------------- |
| 会话消息                           | renderBlocks 返回空数组，释放 group 投影缓存并卸载消息子树；按可见 session 维护 retain／revalidator | 恢复当前 session 的读取与消息展示       |
| 工具详情、活动日志、Plan、资源面板 | 直接读取、定时刷新和恢复入口统一检查 pane 与 document 可见性                                        | 恢复必要读取，避免隐藏 watcher 仍发请求 |
| Files                              | 关闭 filesystem stream，清 overflow timer，暂停 revalidator                                         | 恢复页面读取和订阅                      |
| Git                                | abort 仓库／watch 读取，关闭 watch；隐藏时暂停自动 fetch                                            | 重建 controller，重新加载并恢复 watch   |
| Terminal                           | 关闭 UI-state／output streams，清 reconnect timers                                                  | 按 cursor 续读并 resize                 |
| 模型配置                           | 隐藏时暂停配置 revalidator                                                                          | 显示后恢复必要刷新                      |
| 滚动                               | 取消自动跟随 rAF，断开跟随 ResizeObserver，阻止自动历史读取                                         | 按保存状态恢复                          |

主要入口：[WorkspaceEditorGroupPane.vue](../../packages/agena-web/src/layout/WorkspaceEditorGroupPane.vue)、[WorkspacePaneView.vue](../../packages/agena-web/src/layout/WorkspacePaneView.vue)、[ChatPage.vue](../../packages/agena-web/src/pages/ChatPage.vue)、[useChatRenderBlocks.ts](../../packages/agena-web/src/pages/chat/useChatRenderBlocks.ts)、[AgenaOperationPart.vue](../../packages/agena-web/src/components/chat/AgenaOperationPart.vue)、[useVisibleResource.ts](../../packages/agena-web/src/pages/chat/useVisibleResource.ts)、[useSessionPlan.ts](../../packages/agena-web/src/pages/chat/useSessionPlan.ts)、[FilesPage.vue](../../packages/agena-web/src/pages/FilesPage.vue)、[GitPage.vue](../../packages/agena-web/src/pages/GitPage.vue)、[TerminalPage.vue](../../packages/agena-web/src/pages/TerminalPage.vue)。

另有两项 pane 隔离修复：sessionAction 绑定 session／window，由匹配且 focused 的 pane 消费；Composer fullscreen 根标记以 owners Set 管理，多个 pane 不会在某一方退出时错误清除其他方的状态。Composer 布局读取也合并到一个 rAF。

暂停页面订阅不等同于停止服务端任务。全局运行状态仍需更新；用户重新显示 pane 时应取得当前任务状态。

## 3. 搜索、Vim 与 DOM 文本模型

主要代码：[useChatTranscriptVim.ts](../../packages/agena-web/src/pages/chat/useChatTranscriptVim.ts)、[reactiveKeySet.ts](../../packages/agena-web/src/lib/reactiveKeySet.ts)、[transcriptSearchHighlights.ts](../../packages/agena-web/src/pages/chat/transcriptSearchHighlights.ts)。

1. 搜索结果使用 shallowRef。每行是否选中／命中使用 reactive Set 的按 key `has` 依赖，替代每行扫描整个结果列表，也避免某个 key 的变化使所有行一起重新计算。
2. 搜索输入使用 120 ms debounce；跳转与关闭时 flush，避免用户确认跳转时仍使用上一轮结果。
3. 优先使用 CSS Highlights，兼容路径保留 DOM marks。每个 pane 的名称和样式独立，卸载时清理。
4. 普通装饰高亮只为 near-viewport 行生成，预算为 2048 个 ranges；当前命中额外保留。全文搜索索引、匹配数量和跳转结果不按装饰预算截断。
5. 有序 text segments 与 matches 单次合并，避免为每个命中重新过滤所有文本节点。
6. DOM 文本投影使用 WeakMap 缓存；part 变更只使受影响投影失效。
7. class、style、hidden、aria-hidden、open、媒体 load、resize 等可影响可见文本／布局的变化会使投影失效。祖先布局变化清理其后代投影；行选择／搜索外观和 chrome 样式不触发无意义的文本重建。near-viewport 标记变化只刷新装饰高亮。
8. session 切换清文本缓存；隐藏 pane 的消息子树卸载后不继续保留其 DOM 模型。

这部分仍需要实际 DOM 文本和几何信息。首次进入 Vim／首次构建完整搜索模型，以及大范围布局失效后的重建，仍随消息数量增长。本次没有改为完全不依赖 DOM 的全文模型。

## 4. Markdown、代码、Mermaid 与 Monaco

| 原热点                                            | 修复后的行为                                                                                                | 主要代码                                                                                                                                                                                                               |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 流式 Markdown 在渲染线程反复同步解析              | 流式内容或超过 8192 个字符串字符的内容交给一个共享 Worker；小静态内容保留同步路径                           | [Markdown.vue](../../packages/agena-web/src/components/Markdown.vue)、[markdownAsync.ts](../../packages/agena-web/src/lib/markdownAsync.ts)、[markdown.worker.ts](../../packages/agena-web/src/lib/markdown.worker.ts) |
| 多个 pane／旧流式版本堆积解析工作                 | Worker 一次运行一个 parse；abort 移除待执行正文；version 与 AbortController 防止旧结果覆盖                  | 同上                                                                                                                                                                                                                   |
| 远处／隐藏的 Markdown、Mermaid 和媒体观察仍工作   | near viewport 才开始富渲染；隐藏时停止解析与相关观察；远处／Worker 错误时提供完整 escaped plain source      | Markdown.vue、[useNearViewport.ts](../../packages/agena-web/src/composables/useNearViewport.ts)                                                                                                                        |
| 自动语言检测对大代码尝试多个 grammar              | 已注册显式语言超过 16384 个字符、自动检测或未注册语言超过 2048 个字符时使用完整转义源码；超过单行预算也降级 | [highlight.ts](../../packages/agena-web/src/lib/highlight.ts)                                                                                                                                                          |
| 长行检测的定长正则产生重复尝试                    | 改为线性字符扫描                                                                                            | 同上                                                                                                                                                                                                                   |
| 同一小段代码反复高亮；缓存无限增长                | 高亮 LRU 最多 256 项，输入 key 与输出合计最多 512 Ki 个字符                                                 | 同上                                                                                                                                                                                                                   |
| 折叠代码块仍高亮全文                              | 折叠时只高亮 preview，展开／复制使用完整内容                                                                | [CodeBlock.vue](../../packages/agena-web/src/components/ui/CodeBlock.vue)、[markdown.ts](../../packages/agena-web/src/lib/markdown.ts)                                                                                 |
| 相同文件多分屏共享 model 时，一方释放使另一方失效 | model 使用共享 leases，最后一个使用者释放才 dispose；外部 model 不由组件 dispose                            | [utils.ts](../../packages/agena-web/src/lib/monaco-editor/utils.ts)、[Editor.ts](../../packages/agena-web/src/lib/monaco-editor/Editor.ts)                                                                             |
| 未命名编辑器 URI 冲突与 view state 无限累计       | useId 为未命名编辑器构造独立 URI；每个编辑器 view state 最多保留 64 项                                      | [MonacoCodeEditor.vue](../../packages/agena-web/src/components/MonacoCodeEditor.vue)、Editor.ts                                                                                                                        |

大代码降级保留完整文字与复制内容，但会减少语法着色。远处 Markdown 的纯文本展示与接近视口后的富渲染可能有高度变化，真实页面仍需验证其对滚动锚点的影响。

取消一个已经进入 Worker 的同步 parse 会丢弃结果，但不会抢占该次 parse。Worker 当前没有空闲自动 terminate；它只创建一个共享实例，也会保留自身模块与高亮缓存。没有 Worker 的环境仍保留同步兼容路径。

共享 near-viewport observer 记录实际 observedElement；ref 替换时重新设置可见状态，scope 在 post watcher 执行前结束也会 unobserve 旧元素，避免长期持有失效节点。

## 5. 侧栏、附件、输入与共享读取

### 5.1 侧栏

主要代码：[directorySessionStore.ts](../../packages/agena-web/src/stores/directorySessionStore.ts)、[expandedTree.ts](../../packages/agena-web/src/features/sessions/model/expandedTree.ts)、[ChatSidebar.vue](../../packages/agena-web/src/layout/ChatSidebar.vue)、[SessionRow.vue](../../packages/agena-web/src/layout/chatSidebar/components/SessionRow.vue)。

- known rows 使用 computed，减少重复组合与扫描。
- hydration／locate 共用最多 4 个读取的 limiter，避免大量会话定位同时发请求。
- 去掉 dedupe key 中的全量 preferences stringify；保留未知 expanded ancestor 的处理语义。
- child page 等缓存有上限；ancestor expand 按 directory 合并刷新。
- expanded tree 最多 4 个并发读取，最后迭代 flatten 一次，避免递归每层复制子数组。输出顺序不依赖 HTTP 请求完成顺序。
- 4000 层深树、并发上限、abort 和顺序均有回归覆盖。
- SessionRow 使用 `content-visibility`；操作控件仅 near viewport、行聚焦、重命名或菜单打开时挂载。

这里限制了并发与重复工作，没有截断用户完整展开的树。侧栏未改成真正虚拟列表；全部展开的行数和总读取量仍可能很大。

### 5.2 附件与 Composer

主要代码：[attachmentIngestion.ts](../../packages/agena-web/src/pages/chat/attachmentIngestion.ts)、[useChatAttachments.ts](../../packages/agena-web/src/pages/chat/useChatAttachments.ts)、[blobUrlRegistry.ts](../../packages/agena-web/src/lib/blobUrlRegistry.ts)、[Composer.vue](../../packages/agena-web/src/components/chat/Composer.vue)、[composerDomSelection.ts](../../packages/agena-web/src/pages/chat/composerDomSelection.ts)。

- 新附件使用 Blob／object URL 做本地预览，并支持 raw binary 上传，减少 Base64 字符串、JSON 编码与冗余副本。
- 跨 pane 与已保存草稿共用 128 MiB 本地附件 staging 预算。超额拒绝新加入的附件，保留已有未发送草稿。
- URL 生命周期包含当前／保存／发送中／失败／optimistic 消息／图片查看器引用。discard、unmount 和延迟返回的取消结果均释放不再使用的 URL。
- 同名同大小的附件再以 SHA-256 内容判重；hash 等待期间旧附件删除，不会误删新附件。
- 上传服务保留 legacy Base64 JSON 入口和相同内容哈希／workspace staging 语义；文件系统与哈希工作放到 blocking pool。
- selection offset 单次遍历同时计算两端，移除 `cloneContents`；缓存 selection endpoints。
- `notifyInput` 只读一次 segments，reportedText 保留到 nextTick，减少输入反馈中的重复 DOM 读取。
- Prompt history 最多 200 项、256 Ki 个字符；多个 pane 共享同一 storage 状态，不重复解析未变化的内容。

附件预算衡量的是登记 Blob 的字节数，不是浏览器进程总内存。图片解码、请求体、临时哈希 buffer 与 DOM 内存仍需实际测量。

### 5.3 重复 catalog 读取

[modelSelectionCatalog.ts](../../packages/agena-web/src/pages/chat/modelSelectionCatalog.ts) 按 auth scope／配置 generation 共用请求，普通读取复用 30 秒；已经加载的 picker 显式 reload 只复用短暂的 250 ms 窗口。[pluginCommandRead.ts](../../packages/agena-web/src/lib/pluginCommandRead.ts) 也按 scope 共用 30 秒请求，打开 palette 才读取，失败后允许重试。

## 6. SSE、活动日志与事件应用

主要代码：[sseFrames.ts](../../packages/agena-web/src/lib/sseFrames.ts)、[sse.ts](../../packages/agena-web/src/lib/sse.ts)、[activity.ts](../../packages/agena-web/src/types/activity.ts)、[sessionActivity.ts](../../packages/agena-web/src/stores/sessionActivity.ts)。

1. SSE delimiter 改为增量扫描，避免每个 chunk 都对不断增长的未完成 frame 重新 replace／split；支持跨 chunk 的 CRLF 与 UTF-8 字符。
2. 对可合并的完整 snapshots 合并排队更新，而非保留每一个中间状态；新 snapshot 替换旧字段集合，避免遗漏字段被旧值残留。
3. 合并后的 snapshot 保留在其实际到达位置；signals、removal、recovery 通知作为顺序屏障。较旧 revision 不覆盖较新排队状态。
4. 应用事件每批以约 8 ms 为让出预算，JSON frame 处理循环也让出执行；排队达到 1024 slots 时施加 backpressure。
5. EOF、错误和下一次 onOpen 前分批 drain 旧队列，避免断流时丢掉最后一批更新。
6. drain 中用户回调 close 后重新检查 closed，不再触发 onError／其他后续回调；已关闭对象不再 schedule flush。
7. 全局 busy 查询使用 `active_only=true`，避免前端为运行状态下载所有已完成操作。
8. 活动日志合并保留未变化 line identity；UI 日志限制为 200 行／128 KiB。UTF-8 尾部保留从末尾扫描预算范围，避免先编码整段超大日志。

8 ms 是批次让出目标，不能抢占单次 `JSON.parse` 或一次很重的 onEvent。1024 是排队 slots 的 backpressure 门槛，不是 SSE 字节数上限。单个极大的 frame 仍需要缓冲、解析与内存；这部分没有改为流式 JSON parser。

## 7. 后端查询、缓存、流式写入与维护

仅修改前端不足以消除多个 pane 同时读取大历史的成本。本次也修复了相关 API／存储路径。

### 7.1 按需要读取数据库

主要代码：[engine.rs](../../crates/agena-storage-sqlite/src/engine.rs)、[engine.rs（trait）](../../crates/agena-storage/src/store/engine.rs)、[facade.rs](../../crates/agena-storage/src/store/facade.rs)、[history.rs](../../crates/agena-runtime-session/src/session/manager/history.rs)、[runs.rs](../../crates/agena-runtime-session/src/session/manager/runs.rs)、[sessions.rs](../../crates/agena-runtime-session/src/session/manager/sessions.rs)。

| 读取用途                       | 修复                                                                                    |
| ------------------------------ | --------------------------------------------------------------------------------------- |
| 用户消息 ordinal               | 批量查询，IDs 每 512 个一批；避免每个消息单独扫描历史                                   |
| 会话文件变更                   | SQL 先筛相关 tool parts，再 decode，避免完整加载所有文本／工具                          |
| 子任务日志                     | 按 run cursor 读取分页；Web 正文合计限制 128 KiB、单个 run 限制 64 KiB，保留 UTF-8 尾部 |
| controls／待处理交互           | 只读 own pending／in_progress tool calls                                                |
| cost summary                   | 只读取 run markers；保留 fork membership 的统计语义                                     |
| active background operations   | session／active／kind 在 SQL 中筛选                                                     |
| scheduler owner 与 active 查询 | 将 owner_session_id／active 条件下推到 SQL，避免先加载再逐条 decode                     |
| task reconcile                 | 先读取 metadata；需要处理 terminal 状态时才读全文                                       |

普通日志 reader 仍可读取完整正文。Web 的展示预算没有改写数据库中的完整历史，也没有删除原始日志。

### 7.2 缓存与锁

- 主分支整合后，MemoryLayer 的全局 registry 仅保存每个 session 的 Arc handle；payload 使用每个 session 独立的锁和 part ID 索引。深 clone、part merge 和 streaming overlay 均在有界 worker 内执行，保留主分支的版本校验／CAS 与取消后 flush 所有权，避免并发旧 snapshot 覆盖新结果。
- buffered pure text append 原地追加，减少每个 token 复制已有文本的成本。
- API revision 计算的哈希工作移出全局 mutex；使用 BTreeMap prefix range 避免无关 resource 扫描。
- comparison memos 使用 LRU；最多 8192 个 resource revisions，逐项淘汰并保留 revision floor，避免达到容量时清空全部热缓存。
- baseline 淘汰前 materialize 对保留 list revision 的影响；lagged recovery 仍保持必要的 reset 语义。
- 文件变更 facts 使用 Arc<str> 与预计算 hash，共享 apply_patch diff。
- AppState 内 file projection cache 最多 32 个 sessions／估算 64 MiB；按 revision token 验证，竞争下不会缓存旧 read；summary／截断 detail 不改动缓存 full facts。

对应代码：[facade.rs](../../crates/agena-storage/src/store/facade.rs)、[part_update.rs](../../crates/agena-storage/src/store/part_update.rs)、[revisions.rs](../../crates/agena-api-server/src/revisions.rs)、[file_changes.rs](../../crates/agena-api-server/src/rest/file_changes.rs)。

### 7.3 版本与维护正确性

- committed part revision 单调递增，物理 updated_at 不倒退；buffered logical time 按 +1 推进。同毫秒更新、时钟倒退和 revision 上限有回归测试，避免优化后更新被前端判为旧状态。
- maintenance 等待本轮完成，不再叠加并发任务。
- GC 每轮探测最多 2048 IDs 的主键窗口，没有候选时不取得写事务；leaf-first、外键保护、cursor 轮转，每轮最多 8 个短事务、每个最多删除 256 项，事务间主动 yield，避免一次扫描／删除完整大表或长期占用写队列。

对应代码：[engine.rs](../../crates/agena-storage-sqlite/src/engine.rs)、[builder.rs](../../crates/agena-runtime/src/runtime/builder.rs)。

session_usage 的全文 prompt estimate，以及 lease renewal 的单项事务，本次保留现有运行语义。没有用近似用量或减少 lease 更新来换取界面速度。

### 7.4 现有数据库兼容

主数据库和 scheduler 原本对非空数据库做严格 schema 校验。直接把新索引加入必需对象会导致原有合法数据库无法启动，因此性能索引拆为独立 `PERFORMANCE_INDEXES`。

- tables、原必需 indexes、triggers 继续严格校验。
- 已存在的 performance index 定义必须一致。
- 仅在 durable validation 成功后，以单事务追加缺失的非 unique 派生索引。
- 不改 columns、rows、revision 或更新时间；不兼容数据库不会先被追加索引。
- scheduler JSON expression index 使用 `WHERE json_valid(job_json)`，并保留 invalid-json partial index；追加索引不会使原有 malformed stored row 丢失。

代码：[主 schema](../../crates/agena-storage-sqlite/src/schema.rs)、[validation.rs](../../crates/agena-storage-sqlite/src/schema/validation.rs)、[scheduler schema](../../crates/agena-scheduler/src/schema.rs)。规则已同步到 [development.md](../development.md)。兼容与拒绝路径均在临时／内存数据库测试，未对用户真实数据库执行操作。

第一次打开缺少性能索引的大数据库，SQLite 建索引仍会产生一次性启动工作。本次没有对真实大型数据库测量这段耗时。

## 8. 资源预算与行为取舍

字符串字符数使用 JavaScript `string.length` 的单位；它与 UTF-8 字节数不同。带「估算」的缓存预算不是进程内存的精确测量。

| 资源                                    | 当前预算／策略                            | 超预算行为                                                                                                 |
| --------------------------------------- | ----------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| 前端 transcript cache                   | 24 sessions／估算 32 MiB                  | 淘汰不受保护的旧 transcript；保护可见、selected 和 HTTP in-flight sessions，所以保护集合过大时可以超出预算 |
| 本地附件 staging                        | 跨 pane／saved drafts 共用 128 MiB        | 拒绝新附件，保留已有草稿；释放不再引用的 object URL                                                        |
| Prompt history                          | 200 entries／256 Ki 个字符                | 只保留预算内条目                                                                                           |
| 显式已注册语言高亮                      | 16384 个字符以内，另有单行预算            | 完整 escaped plain source                                                                                  |
| 自动／未注册语言高亮                    | 2048 个字符以内                           | 完整 escaped plain source                                                                                  |
| 高亮缓存                                | 256 entries／key 与输出合计 512 Ki 个字符 | 淘汰最旧缓存项                                                                                             |
| Markdown parse                          | 一个共享 Worker、一次一个 parse           | abort 移除待执行旧版本；已开始任务的旧结果丢弃                                                             |
| 普通搜索装饰                            | near viewport，2048 ranges                | 保留完整搜索结果与当前命中，减少其他装饰                                                                   |
| SSE                                     | 约 8 ms 批次、1024 slots backpressure     | 分批应用／让出执行，无按数量丢弃通知的策略                                                                 |
| UI 活动日志                             | 200 lines／128 KiB                        | 保留尾部                                                                                                   |
| Web 子任务日志正文                      | 合计 128 KiB／run 64 KiB                  | 保留 UTF-8 尾部；普通 reader 的完整内容仍可用                                                              |
| 侧栏 hydration／locate、expanded tree   | 各相应读取路径最多 4 并发                 | 排队；总展开树没有硬截断                                                                                   |
| Monaco view states                      | 每个编辑器 64 entries                     | 淘汰最旧状态                                                                                               |
| API file projection cache               | 32 sessions／估算 64 MiB                  | 淘汰旧 facts；不修改 full facts 内容                                                                       |
| API resource revisions／comparison memo | 对应缓存最多 8192 entries                 | 逐项淘汰，保留必要 floor 与 retained list 语义                                                             |
| GC                                      | 一轮 2048 IDs 窗口、最多 8 层             | 后续轮次继续推进                                                                                           |

transcript 淘汰会清消息、hydration、history cursors、observations、revalidators、retry timers 和 part→sessions 反向 membership。Composer 与附件草稿另存，不随 transcript 淘汰。缓存大小与策略代码见 [transcriptCache.ts](../../packages/agena-web/src/stores/chat/transcriptCache.ts) 和 [chat.ts](../../packages/agena-web/src/stores/chat.ts)。

## 9. CPU 基准

原始结果：[frontend-performance-benchmark.json](./frontend-performance-benchmark.json)。脚本：[benchmark-chat-performance.ts](../../packages/agena-web/scripts/benchmark-chat-performance.ts)。

环境：Apple M5、darwin arm64、Bun 1.4.0。Git 基线代码与当前代码在同一进程运行；每项 3 次 warmup、9 次采样，取中位数。结果采集时间为 `2026-10-06T14:50:54.426Z`。

| 场景                                            | 修复前 ms | 修复后 ms |
| ----------------------------------------------- | --------: | --------: |
| 连续 assistant，2000 messages                   |    99.234 |     0.444 |
| 4000 text parts 后接工具的回答分类              |    59.792 |     0.542 |
| 6000 messages，仅尾消息流式更新                 |    10.413 |     3.541 |
| 2000 rows／4000 matches，搜索行 membership      |   203.533 |     0.058 |
| 2 MiB SSE frame，4 KiB chunks 的 delimiter 扫描 |    31.264 |     1.755 |
| 92000 字符 JavaScript 的展示转换                |    16.852 |     0.052 |
| 120 万字符活动日志，保留 128 KiB 尾部           |     0.937 |     0.107 |

这是独立 CPU 热点测量，不包括浏览器 DOM、layout、paint、网络和 JSON parsing；搜索 membership 项也不包括构建完整 DOM 搜索索引。大代码项采用超预算后停止语法着色的策略，不能解读成同等高亮工作加速了 322 倍。这些数字不是整应用 FPS 或用户交互延迟。

从 `packages/agena-web` 目录运行：

```sh
bun scripts/benchmark-chat-performance.ts --output=../../docs/research/frontend-performance-benchmark.json
```

脚本当前以执行时 Git HEAD 作为基线。此次结果已记录原 commit；若 HEAD 后续包含修复，再运行不能继续称为与本次修复前的比较。

## 10. 验证结果

| 检查                            | 结果                                                                         |
| ------------------------------- | ---------------------------------------------------------------------------- |
| `bun run build`                 | 通过 imports、settings i18n、vue-tsc 和 Vite 生产构建；本次 Vite 阶段 4.37 s |
| `bun test`                      | 675 pass、0 fail、1575 expect；202 files，16.98 s                            |
| 受影响 7 crate 的 `cargo check` | 通过                                                                         |
| 同一组 `cargo test --lib`       | 650 pass、0 fail                                                             |
| `git diff --check`              | 通过                                                                         |

Rust 测试按 crate：agena-api-server 69、agena-application 46、agena-runtime 125、agena-runtime-session 219、agena-scheduler 42、agena-storage 68、agena-storage-sqlite 81。SQLite suite 内调用子进程复跑的单项测试不重复计入 650。

前端命令在 `packages/agena-web` 执行，Rust 命令在仓库根目录执行：

```sh
bun run build
bun test

cargo check -p agena-storage-sqlite -p agena-storage -p agena-api-server \
  -p agena-runtime-session -p agena-runtime -p agena-application \
  -p agena-scheduler

cargo test -p agena-storage-sqlite -p agena-storage -p agena-api-server \
  -p agena-runtime-session -p agena-runtime -p agena-application \
  -p agena-scheduler --lib

git diff --check
```

新增／扩展的有效回归覆盖包括：

- 流式编辑只改变受影响 part／block；长空白摘要、最终回答分类和 reactive Set 按 key 更新。
- 隐藏 pane 不再读取工具详情，重新显示恢复；实际 Vue 生命周期中暂停滚动观察并恢复锚点／底部状态。
- expanded tree 的深度、请求并发、顺序和 abort；共享 near-viewport observer 的替换与释放。
- 多 pane 共享 model 的最后释放；附件共用预算、URL 释放、取消和异步判重竞争。
- 跨 text node 的高亮偏移与当前命中保留；Composer Unicode／换行／附件 chip selection offsets。
- SSE 各分隔位置、CRLF／UTF-8、较大未完成 frame、合并与顺序、EOF drain、close 后不再回调。
- UTF-8 日志尾部与 identity；prompt history 共享与字符预算。
- SQL 小结果查询、fork membership、查询计划、GC 外键保护、snapshot 竞争、revision 单调性与缓存逐项淘汰。
- binary／legacy 上传一致性、workspace staging 与 symlink guard；已有 schema 加索引保留 rows，错误 schema／错误同名索引不被修复。

测试入口：[transcriptPerformance.test.ts](../../packages/agena-web/tests/transcriptPerformance.test.ts)、[performanceLifecycle.test.ts](../../packages/agena-web/tests/performanceLifecycle.test.ts)、[sseStreamIntegrity.test.js](../../packages/agena-web/tests/sseStreamIntegrity.test.js)、[toolPresentationRendering.test.ts](../../packages/agena-web/tests/toolPresentationRendering.test.ts)、[engine_tests.rs](../../crates/agena-storage-sqlite/src/engine_tests.rs)、[workspace_uploads.rs](../../crates/agena-api-server/src/router_contract_tests/workspace_uploads.rs) 及各 Rust 模块内部 tests。

现有「消息结束才推进导航 recency」语义保留；排序、fork／rewind、完整复制、已保存草稿、通知顺序与权限边界没有通过删减数据来规避工作。

## 11. 实际页面验收边界

当前 Browser 环境的 tab 列表为空，localhost 也无法取得可用浏览器。本次未启动真实用户会话来模拟压力，也未改写用户数据库。以下是后续实际页面验收需要观察的场景，不是本次已经取得的测量结果：

| 场景                                                       | 应观察的结果                                                                         |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| 6000+ messages 持续流式输出，同时输入和滚动                | Performance 中主要工作归属、长任务、输入延迟、布局与绘制时间                         |
| 多个分屏可见，同时打开大量后台标签页                       | 后台 pane 请求、rAF、observer 应暂停；可见分屏继续更新；CPU 不因后台重复读取持续增长 |
| 会话阅读中切标签页再切回                                   | 缓存保留时恢复阅读锚点；底部跟随状态恢复；图片／Markdown 高度变化后仍稳定            |
| 完整展开大量侧栏子会话                                     | 请求并发不失控；总行数、DOM 数与初次展开耗时仍需测量                                 |
| 在长会话首次开启 Vim／全文搜索                             | 首次 DOM 模型成本、搜索反馈、跳转正确性及富内容重排后的失效处理                      |
| 同文件多分屏 Monaco，关闭其中一方                          | 另一方可继续编辑；undo、view state、选择区和重新挂载行为                             |
| 多 pane 加附件，取消／发送失败／切 session／打开图片查看器 | 草稿保留、object URL 释放、Blob budget 与图片解码后的进程内存                        |
| 持续切换／关闭标签页 30–60 分钟                            | detached DOM、observer、timer、Blob 和 Worker／model 的 retained size                |

本次没有实现消息列表或侧栏的真正 DOM 虚拟化。`content-visibility`、near-viewport 富渲染与隐藏 pane 卸载减少后台及屏幕外工作，但可见 pane 中的消息行、完全展开的侧栏行仍随数据量增长；浏览器查找、初次 Vim DOM 模型与极大单个 JSON frame 也保留规模成本。实际页面 profiling 应据此判断是否需要进一步采用可变高度虚拟列表或分段文本模型。

## 12. 继续检查与新增修复（2026-10-07）

本轮继续沿「展示读取 → 可见性 → 组件生命周期 → 编辑器计算 → 生产产物」追查。下面记录已经修改并验证的具体问题，避免把代码中的 async import、请求 abort 或一个局部基准直接等同于整页性能保证。

### 12.1 可见性、共享订阅与读取调度

| 新发现的问题                                                                              | 修复与目的                                                                                                                         | 代码                                                                                                                                                                                                                                                                                                                                                 |
| ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 一个复杂设置 pane 内大量字段分别监听 document visibility，多 pane 又放大 listener 数量    | 按 Document 共享一个 listener，以 scope 引用计数释放；可见性合并 document 与各 pane 的 `isVisible`，保持未聚焦但可见的分屏正常更新 | [usePaneVisibility.ts](../../packages/agena-web/src/composables/usePaneVisibility.ts)                                                                                                                                                                                                                                                                |
| 设置、预览等已挂载子组件的共享资源 lease 在 pane 隐藏后仍保留                             | visible retain、hidden release、unmount release；同一消费者始终最多持有一个 lease                                                  | [useVisibleSubscription.ts](../../packages/agena-web/src/composables/useVisibleSubscription.ts)、[usePolling.ts](../../packages/agena-web/src/composables/usePolling.ts)                                                                                                                                                                             |
| 调用 refresh 时可见，但派发 read 的 microtask 执行前已经切走，仍会发出隐藏读取            | 在实际 read 执行前重查 enabled；未完成的 invalidation 留给下一次 resume                                                            | [revalidation.ts](../../packages/agena-web/src/lib/revalidation.ts)                                                                                                                                                                                                                                                                                  |
| Permissions、Memories、Usage 的手工刷新可重叠，隐藏后继续等结果，失败又清掉已经显示的列表 | 使用串行展示读取，接入全局最多 4 个 background reads、30 秒读取 timeout、隐藏取消／延后与 backend scope 校验；暂时失败保留已有结果 | [usePaneRead.ts](../../packages/agena-web/src/composables/usePaneRead.ts)、[PermissionsPanel.vue](../../packages/agena-web/src/components/settings/PermissionsPanel.vue)、[MemoriesPanel.vue](../../packages/agena-web/src/components/settings/MemoriesPanel.vue)、[UsagePanel.vue](../../packages/agena-web/src/components/settings/UsagePanel.vue) |
| Activities、模型目录 monitor、Provider OAuth 轮询在后台仍安排下一轮展示读取               | 读取和恢复入口检查 pane 可见性，隐藏取消相应读取／timer；保留原有 mutation、授权和错误退避语义                                     | [ActivitiesPanel.vue](../../packages/agena-web/src/components/settings/ActivitiesPanel.vue)、[ModelCatalogPanel.vue](../../packages/agena-web/src/components/settings/ModelCatalogPanel.vue)、[ProviderStudioPanel.vue](../../packages/agena-web/src/components/settings/ProviderStudioPanel.vue)                                                    |

回归用 30 个 pane 验证 document listener 共享，以及独立分屏的可见性。取消展示读取不会取消服务端正在运行的任务，也不会撤回已经发出的用户写入。

### 12.2 设置字段的请求风暴与同步文件工作

原有一个 ServerSettingField 同时读取 effective、file、global、workspace 四个来源。包含 200 个字段的页面会发出 800 个 GET，并反复处理相同配置文档；部分设置 API 的文件读写还直接运行在异步 executor 上。

新增 `GET /api/v1/settings/sources?paths=<JSON>` 后，同一 microtask、同一 scope／auth version／backend URL／mutation generation 的叶字段读取会分批。200 个普通字段的回归结果是 **4 个 GET**，四个来源和字段值均保留。

- 每批最多 64 个 paths；前端还按编码后 query 最多 6000 字符拆分，避免 quoted／Unicode paths 使请求 URL 过长。
- 服务端先验证整个 batch，再读取 effective、global、workspace 根文档各一次，并在服务端提取请求的 leaves；file 来源沿用既有 global 语义。
- 服务端限制 paths JSON 最多 16384 UTF-8 bytes，path 非空且每个最多 512 UTF-8 bytes；quoted path、数组下标、缺失值、revision 和重复 path 有 HTTP 回归。
- 相同 path 的 in-flight reads 共用请求。取消一个 consumer 不影响其他 pane；所有 consumer 退出才停止 transport，派发前已经全部取消的 batch 不请求。
- 每个 transport 有 15 秒 timeout。写入前后推进 generation；后续读取和 auth 变化后的读取不会加入旧 flight。完成后的设置结果不保留为永久缓存。
- 根文档 advanced editor 保留原四 API 的读取能力，同时共用 flight 并经过并发 limiter。
- settings 的读取、list、set、patch、delete、validate 同步文件工作移到 blocking pool；保留原文件锁和异步 reload。16 个并发设置写入的 HTTP 测试确认没有丢失其他字段；current-thread executor 测试确认等待文件工作时仍能运行其他异步任务。

代码：[runtimeSettingReads.ts](../../packages/agena-web/src/lib/runtimeSettingReads.ts)、[runtimeSettings.ts](../../packages/agena-web/src/lib/runtimeSettings.ts)、[settings.rs](../../crates/agena-api-server/src/rest/settings.rs)、[settings_sources.rs](../../crates/agena-api-server/src/router_contract_tests/settings_sources.rs)。

### 12.3 避免无必要的重建、旧回调覆盖与遗漏懒加载

| 触发场景                                                  | 原有问题                                                              | 修复后的行为                                                                                        |
| --------------------------------------------------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| 切换设置语言                                              | refresh nonce 变化使 workbench 销毁重建，再次读取服务端字段并丢失草稿 | 翻译 options、pages、labels 改为响应式 computed；切语言保留组件和草稿，不读取未改变字段             |
| 设置字段 path 或 target layer 切换                        | 旧 autosave timer 或迟到 write callback 可以操作新字段                | 同步清理旧 timer 和 read；写入捕获原 path／layer／reload，以 field identity 拦截迟到刷新和 emit     |
| 隐藏已加载设置字段再切回                                  | 重复读取未改变字段                                                    | 只恢复确实 pending 或中断的读取；不因简单显示动作重读所有已加载字段                                 |
| 刷新 Activities／Permissions／Memories／Usage，或临时错误 | loading/error 分支卸载整份已加载列表                                  | 数据和 rows 持续挂载，loading/error 单独显示；Intl 数字 formatter 在 Usage 内复用                   |
| 打开任意设置 section                                      | 所有 workbench 和插件 surface catalog 被提前加载                      | 六个顶级 workbench 及其子面板改为 async components；conversation catalog 只在相关界面首次需要时读取 |
| 打开文件查看器但尚未使用特定展示                          | 普通 Monaco、diff Monaco、Markdown renderer 提前进入静态依赖          | FileViewerPane 的三类重组件独立 async；WorkspaceDock 的 terminal／preview 入口也改为 async          |

手工 Refresh 仍保留主动重载设置 workbench 的原行为。locale 变更触发的重建已经去掉，不能把两种触发混为一谈。

代码：[SettingsPage.vue](../../packages/agena-web/src/pages/SettingsPage.vue)、[ServerSettingField.vue](../../packages/agena-web/src/components/settings/ServerSettingField.vue)、[FileViewerPane.vue](../../packages/agena-web/src/pages/files/components/FileViewerPane.vue)、[WorkspaceDockPanel.vue](../../packages/agena-web/src/layout/WorkspaceDockPanel.vue)，以及各 settings workbench／panel。

### 12.4 Monaco、长文件 diff 与多窗口模型隔离

| 问题                                                                                     | 已实现的修复                                                                                                                            | 代码                                                                                                                                                                                                                                     |
| ---------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| hunk 排序的 comparator 反复规范化、线性扫描整份行号映射                                  | 一次建立 prefix-maxima 索引，以二分定位；每个 hunk anchor 计算一次。保留 sparse 和非单调映射的既有语义                                  | [displayLineResolver.ts](../../packages/agena-web/src/components/editor/displayLineResolver.ts)、[MonacoDiffEditor.vue](../../packages/agena-web/src/components/MonacoDiffEditor.vue)                                                    |
| 容器一直是零尺寸时，每一帧重试首次 reveal                                                | 依赖实际 `onDidLayoutChange`，保留 pending reveal；零尺寸不启动 rAF 重试循环                                                            | MonacoDiffEditor.vue                                                                                                                                                                                                                     |
| 相同 hunk zones 在每次 diff update 或 busy 状态改变时全部销毁重建                        | model／version／zone 配置未变就复用；busy／active 只更新已有 buttons；行号 options cache 也包含 lineCount                               | MonacoDiffEditor.vue                                                                                                                                                                                                                     |
| 隐藏编辑器仍 loader.init、同步模型、更新 options、扫描 find、重建 decorations／zones     | 首次加载和上述展示工作按 pane visibility 暂停，异步 import 返回后再次检查；恢复时应用最新 props 并保留 editor 实例及 view state         | [MonacoCodeEditor.vue](../../packages/agena-web/src/components/MonacoCodeEditor.vue)、[Editor.ts](../../packages/agena-web/src/lib/monaco-editor/Editor.ts)、[useMonaco.ts](../../packages/agena-web/src/lib/monaco-editor/useMonaco.ts) |
| 查找内容没有变化，移动当前命中仍扫描全文并重建全部 decorations                           | 缓存 model identity、version、query 与 flags；移动命中只更新 current decoration，编辑／替换前检查最新 model version                     | [useMonacoFindSession.ts](../../packages/agena-web/src/components/editor/useMonacoFindSession.ts)                                                                                                                                        |
| 共享 model 的一方隐藏／关闭时，清掉另一 pane 的 CodeLens callbacks                       | CodeLens 按 model 维护 owner maps，各方只释放自身 entries，最后释放时清空                                                               | MonacoCodeEditor.vue                                                                                                                                                                                                                     |
| Git 从仓库／staged／commit／文件 A 切到 B，旧正文先写入 B 的 URI；错误刷新卸载已有编辑器 | 模型 identity 包含完整 scope，只在对应内容成功后推进 loaded identity；失败保留已有内容／editor，scope 未匹配时禁止旧 hunks 的新文件操作 | [GitEditorDiffViewer.vue](../../packages/agena-web/src/components/git/GitEditorDiffViewer.vue)                                                                                                                                           |
| 多分屏选择同一文件的不同历史版本，timeline URI 仅按文件路径共享，互相覆盖正文            | timeline model 增加 FileViewer 的 useId owner；同一 viewer 换版本复用 model／editor，不同 viewer 独立。关闭一方只释放自身 leases        | FileViewerPane.vue、MonacoDiffEditor.vue                                                                                                                                                                                                 |

普通 Files 编辑器的 selectedPath 已沿真实 `/workbench/fs/list` 的 absolute paths 生成，因此没有仅凭 workspace tree 的相对 path 就改写其共享 URI。历史内容独立于普通工作文件，按 viewer 隔离。

### 12.5 终端初始化、后台尺寸工作与重复快照

继续检查发现，第一轮暂停 Terminal 的输出订阅后，仍遗漏了 xterm 初始化、ResizeObserver、resize timer 和首次初始化的后续异步步骤。

- 后台 pane 不初始化 xterm，不派发首次 state read；初始化中切走会 abort，迟到返回不能在卸载后探测／启动其他终端。
- 首次初始化串行执行，快速 hide/show 若发生在旧 flight 释放前，会在其释放后恢复一次；不会因此丢失初始化。
- 隐藏时 disconnect ResizeObserver、清 resize timer，并取消 session probes；callback 实际执行前再次检查可见性及非零宽高，不向服务端发送隐藏容器的尺寸。
- 零尺寸时保留输出 buffer，获得有效尺寸后再初始化、重放必要内容；显示已有 pane 复用 xterm 实例。
- 终端列表逐项探测原先是无上限 Promise.all，现在各项经过全局最多 4 并发 limiter；相同列表在 in-flight 时共用 Promise，列表变化保留一次必要的后续读取。隐藏时排队请求不派发，取消不当成 session 已被删除。
- 相同 version／updatedAt 的重复快照不再重建本地状态、重放输出或重新探测整份列表。名称、pin、recency、排序变化不会探测未改变的 terminal membership；保持当前 viewport／output，确实切换 active session 才重放对应 buffer。主动刷新和实际 membership 变化仍会探测。
- SSE 已解析好的状态对象直接应用，取消每次先 stringify 整份 terminal list 再 parse 的重复工作。

代码：[TerminalPage.vue](../../packages/agena-web/src/pages/TerminalPage.vue)。回归使用实际 Vue 生命周期和 xterm 接口替身，覆盖隐藏初始化、零尺寸恢复、实例复用、重复快照／名称修改、20 sessions 的探测并发与取消，以及卸载后的迟到返回。

### 12.6 预览 probe、iframe 重载与旧请求竞争

Preview Dock／PageSidebar 的 live sessions retain 接入 pane 可见性。相同 iframe URL 保持原 frame；同 URL 的 pending probe 共享。隐藏、卸载、地址为空或进入错误状态时，清 frame timer、取消 probe 并推进 request identity。迟到结果必须仍对应当前 previewSrc，防止旧地址重新覆盖已经清空的 frame。重试入口先清除旧 iframeError，避免旧错误永久挡住新请求。

新增 probe 优先 HEAD。开发服务器不支持 HEAD 或需要错误详情时 fallback GET；成功时取消 body，错误正文最多读取 16 KiB，整个 probe 有 10 秒 timeout，并沿用当前 UI auth headers。这样避免为了确认 preview 就下载、重写整份 HTML 后，再让 iframe 重复下载一遍。

代码：[WorkspacePreviewDockPanel.vue](../../packages/agena-web/src/features/workspacePreview/components/WorkspacePreviewDockPanel.vue)、[WorkspacePreviewPageSidebar.vue](../../packages/agena-web/src/features/workspacePreview/components/WorkspacePreviewPageSidebar.vue)、[previewProxyProbe.ts](../../packages/agena-web/src/features/workspacePreview/api/previewProxyProbe.ts)。

隐藏时保留 iframe DOM，以保留预览网页自身状态。这里控制的是宿主应用的订阅、probe 和刷新；没有宣称可以暂停任意 iframe 内部的第三方脚本。

### 12.7 首屏整库图标加载与生产产物检查

生产构建通过后检查 HTML modulepreload 和入口静态依赖，发现 `@remixicon/vue` 的发布 ESM 没有标注 component constructors 的 purity。即使应用使用具名 imports，未使用的 defineComponent 调用仍保留，导致首屏静态图标 chunk 包含 **3227 个组件、约 2.7 MB JS**。

新增仅针对该包 ESM 的构建插件：从 AST 确认 Vue defineComponent 的本地别名和 Ri 组件声明，只为这些调用添加 `/*#__PURE__*/`，让构建器正常丢弃未使用图标。不会把应用其他初始化函数或全局函数一并标成 pure；保留 source maps。新增 magic-string 0.30.21 是已在依赖树中的版本，此次只明确声明为开发依赖。

使用实际 Vite build 的回归 fixture 只导出 RiAddLine、RiCloseLine，验证两个组件保留、其他图标没有残留、图标构造数为 2，且 debug source maps 保留原 source。再检查完整生产 build：

| 入口静态依赖指标     |      图标裁剪前 |      图标裁剪后 |
| -------------------- | --------------: | --------------: |
| JS 原始大小合计      | 4,255,765 bytes | 1,619,577 bytes |
| 各模块 gzip 大小合计 |   984,230 bytes |   483,427 bytes |
| 图标组件构造数       |            3227 |              96 |
| 静态 JS 模块数       |              21 |              21 |

比较的是本轮其他修复已经完成后，启用该插件前后的两份生产产物；原始 JS 减少约 **61.9%**，gzip 合计减少约 **50.9%**。没有把首次路由的动态 chunks、CSS、fonts、workers 或实际网络／JS 执行时间计入这些数字。96 指首屏静态图的图标组件数，其他动态页面需要的图标仍随相应依赖加载。

代码：[remixiconTreeShaking.ts](../../packages/agena-web/scripts/remixiconTreeShaking.ts)、[vite.config.ts](../../packages/agena-web/vite.config.ts)。可复跑的检查：[inspect-startup-bundle.ts](../../packages/agena-web/scripts/inspect-startup-bundle.ts)；原始记录：[frontend-startup-bundle-benchmark.json](./frontend-startup-bundle-benchmark.json)。

### 12.8 长文件 diff 的 CPU 基准

原始数据：[frontend-editor-performance-benchmark.json](./frontend-editor-performance-benchmark.json)。脚本：[benchmark-editor-performance.ts](../../packages/agena-web/scripts/benchmark-editor-performance.ts)。从修复基线的 MonacoDiffEditor 提取旧函数，与当前索引比较；每种样本先验证结果相同，再做 1 次 warmup、3 次采样并取中位数。

| 行数／hunks         | 修复前 ms | 修复后 ms |
| ------------------- | --------: | --------: |
| 2000 行／200 hunks  |    15.103 |     0.093 |
| 10000 行／300 hunks |   148.303 |     0.100 |
| 50000 行／500 hunks |  1348.600 |     0.436 |

环境与上一轮相同，为 Apple M5、darwin arm64、Bun 1.4.0。这里仅测量行号映射与 hunk 定位／排序的 CPU 路径，**不包含 Monaco 的 diff 算法、浏览器 layout／paint 或整个 Git 页面加载时间**。不能把 1349 ms → 0.44 ms 描述成整页打开只需 0.44 ms。

从 `packages/agena-web` 目录运行：

```sh
bun scripts/benchmark-editor-performance.ts --output=../../docs/research/frontend-editor-performance-benchmark.json
bun scripts/inspect-startup-bundle.ts --label=current --output=../../docs/research/frontend-startup-bundle-benchmark.json
```

### 12.9 本轮最终验证与尚未取得的证据

| 检查                              | 本轮结果                                                |
| --------------------------------- | ------------------------------------------------------- |
| 完整前端 `bun test`               | 707 pass、0 fail、6611 expect；205 files，14.71 s       |
| `bun run build`                   | imports、settings i18n、vue-tsc 与生产构建通过          |
| `cargo check -p agena-api-server` | 通过                                                    |
| `cargo test -p agena-api-server`  | 73 pass、0 fail（含本轮新增的 4 个 API／blocking 回归） |
| `git diff --check`                | 通过                                                    |

第 10 节的 675 个前端测试、7 crate 的 650 个 Rust 测试是上一轮结果，保留原采样日期和含义。本轮后续没有修改其他六个后端 crate，因此没有重复运行相同未变代码的全套 Rust 测试。

新增回归：[performanceFollowup.test.ts](../../packages/agena-web/tests/performanceFollowup.test.ts)（14 cases／5036 assertions）、[performanceFollowupRendering.test.ts](../../packages/agena-web/tests/performanceFollowupRendering.test.ts)（17 cases，含实际 Vue 生命周期与 SSR 已加载 rows 的模板渲染）、[iconTreeShaking.test.ts](../../packages/agena-web/tests/iconTreeShaking.test.ts)（实际 Vite 产物裁剪）。组件测试中的 Monaco、xterm 与 fetch 使用测试替身；它们证明生命周期、取消、复用和输出规则，不能提供真实浏览器的 FPS、输入延迟或内存曲线。

仍沿用第 11 节的页面验收边界：消息／侧栏未实现真正 DOM 虚拟化；首次完整 Vim DOM 建模仍随规模增长；极端 Monaco 用户正则可能存在同步回溯，任意 iframe 内部脚本不由宿主暂停。本轮没有取得真实页面长任务、FPS、输入延迟或 30–60 分钟 retained memory 测量，因此不作「任意规模都完全不卡顿」的结论。

以上测试和基准是原性能工作分支在记录时取得的结果；当时变更尚未提交或部署，设置和 SQLite 写入回归使用隔离测试数据。2026-10-07 的主分支整合将后端、前端和记录分别保存为 `d7ab5070`、`b066857a`、`b4a9f1f2`，并保留主分支后续的 Tokio、写队列、每个 session 的缓存索引及取消处理。此次整合按用户要求仅做编译检查，不重新运行测试或基准；原记录的测试数量不能视为整合后再次验证的结果。
