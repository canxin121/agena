# TUI 响应性检查与修复记录

检查日期：2026-11-01。基线提交：`c6e813da4`。本文记录五轮检查。各轮“未提交”的说明对应当轮检查结束时的状态；后续按明确要求整理的提交及复验见文末。未替换正在运行的 Agena 进程。

## 确认的问题和处理

| 路径 | 原因 | 处理 |
| --- | --- | --- |
| `agena-tui-app/app_lifecycle.rs` | 每个流事件都会触发完整绘制；暂停后定时器可能追赶积压；输入模式依赖下一次绘制 | 按需合并绘制，跳过错过的维护 tick；输入目标在每次读取前同步；媒体完成主动唤醒绘制，而不是等待空闲重绘 |
| `agena-tui-app/app_backend/file_index.rs`、`transport.rs` | 每次补全复制全索引、转小写、全量排序；锁竞争可能导致暂时空列表 | 不可变索引通过 ArcSwap 发布；索引刷新时预处理匹配字段；有界堆只保留最佳候选；保持旧排序语义 |
| `agena-tui-app/app_composer_state.rs` | 周期性草稿和历史记录写入在主循环执行原子写入及 fsync | 周期性保存和提示历史移到后台，互斥和版本号防止旧快照覆盖；强制持久化保留其耐久性语义 |
| `third_party/crossterm/src/event/source/unix/mio.rs` | EOF 以及 EIO 等不可恢复读取错误没有退出内层循环 | EOF 返回明确错误；不可恢复读取错误向上传播，不再忙循环 |
| `third_party/crossterm/src/event/source/unix/tty.rs` | 已解析的第二个按键仍等待下一次 fd 就绪；HUP/ERR 没有作为输入终止处理 | 先排空解析队列，再等待就绪；区分 EOF 与 WouldBlock，并处理挂断事件 |
| `agena-tui/terminal_input.rs`、`agena-tui-platform/terminal/mod.rs` | 连续立即就绪、协议响应或旧式粘贴可能在一个 async poll 内长期循环 | 显式消费协作式调度预算；输入仍保存在唯一终端拥有者中，取消等待不会丢失已读取状态 |
| `agena-tui-platform/helper_runner.rs`、`agena-process/blocking.rs` | 等待直接子进程有超时，但之后 join 管道线程没有超时；后代保留管道会无限等待 | 复用统一进程树及作用域内管道策略；stdin、stdout、stderr 与进程完成共享期限，回收后代，输出受限 |
| `agena-tui-platform/clipboard/image.rs` | WSL PowerShell 剪贴板回退使用无期限 `Command::output` | 使用有界辅助进程调用 |
| `agena-tui-app/app_clipboard.rs` | 原生剪贴板、tmux 复制和 PNG 编码在事件循环执行 | 专用线程隔离原生调用，最多一个进行中任务，超时反馈；OSC 52 仍由终端拥有者写入；粘贴结果校验会话、编辑器代次和焦点；过期图片结果清理临时文件 |
| `agena-tui-app/app_session_interactive/execution.rs` | 队列满时丢弃状态栏完成消息，`refresh_in_flight` 永远不清除 | 必须交付完成消息，通过异步等待队列容量保留正确性 |
| `agena-tui-app/app_session_helpers.rs` | 状态命令的后代持有 stdout，父进程完成后仍等待 EOF 到超时 | 父进程完成后先终止残留进程树，再收集 EOF |
| `agena-tui-media/background.rs` | 首次图片解码、SVG 和原生公式栅格化在绘制路径同步执行 | 仅实时 TUI 启用后台渲染；全局最多两个媒体工作线程；去重、失败缓存、条目及像素预算；弱引用复用仍被会话行持有的图片；本地图片重读期间保留旧画面；完成后通知 UI |
| `agena-tui-transcript/renderer/transcript_ast/render.rs` | 图片占位可能显示整个 Base64 URL，或把 Base64 的末段误认为文件名 | 内嵌数据只显示标签；图片描述和源预览限制长度，保持 UTF-8 边界 |
| `agena-tui-media/render.rs` | 图片 stat 后可能增长，随后无界读取突破字节限制 | 实际读取最多限额加一个字节，并拒绝超限内容 |

## 第三轮：长输入、长会话和终端图片编码

本轮继续在前两轮工作树上修改，没有重置已有更改。

| 路径 | 新确认的问题 | 第三轮处理 |
| --- | --- | --- |
| `agena-tui-components/editor.rs` | 同一帧反复按全部文本分行、折行；长逻辑行尾部的每个可见软行都重新扫描前缀 | 共享不可变逻辑行和折行缓存，最多保留两个宽度；编辑文本时失效，光标移动与只读副本复用；每个软行记录准确字节范围，绘制只处理可见文本 |
| `agena-tui-app/transcript_state.rs`、`action_cursor.rs`、`view_main.rs` | 无选区的空闲绘制也按整个历史分配选择范围；聚焦控件高亮扫描所有历史 | 绘制只计算可见窗口的选区和控件范围；显式复制仍覆盖完整选区 |
| `agena-tui-media/graphics_jobs.rs`、`render.rs` | 解码虽已移到后台，但缩放、选择合成、PNG/原生协议编码仍占用绘制线程；缓存满后整批清空 | 两个有界编码工作线程；未消费结果也占并发名额；LRU 单条淘汰、失败缓存和存储预算；终端代次变化丢弃旧结果；完成只唤醒重绘，不反复使 Markdown 缓存失效 |
| 同上：协议差异 | 直接缓存整幅控制单元可能让 Kitty/Sixel 滚动时重新编码，或在工作线程中提前标记首次上传 | iTerm2 的裁剪在后台生成相对单元；Kitty/Sixel 复用已编码协议并保留轻量的滚动放置/切片；Kitty 首次上传仅在真实 UI 帧绘制时确认 |
| `agena-tui-media/graphics_selection.rs` | 未选中的图片跳过目标像素限制，特殊尺寸可能诱发巨大分配 | 无论有无选区都验证零尺寸、最大边长和像素预算 |
| `agena-tui-app/app_status_context.rs` | 队列满、队列关闭或任务 panic 时丢弃请求专属完成回调，所属界面可能一直显示加载 | 先预留队列容量；失败仍交给原完成回调；捕获操作创建/轮询中的 panic；拒绝回调延至输入处理结束、弹窗状态恢复后执行，每次主循环最多处理 32 个，避免重入重试 |
| `agena-tui/terminal_input.rs` | 旧式粘贴超过 256 KiB 时退化成几十万次独立按键；前一分块的 IME 状态可能吞掉下一块内部换行 | 满块以一个 Paste 事件交付，保持 UTF-8 边界；当前块尚在积累时，不让上一块的 IME 确认规则吞掉换行 |
| `agena-tui-transcript/renderer/transcript_ast*` | 缓存命中仍深拷贝整棵 Markdown 树，并复制长源码维护 LRU | 只读渲染共享 `Arc` 语法树；源码索引与 LRU 共用 `Arc<str>`；保留公开可修改返回值语义，未缓存的独占树直接移动而非重复克隆 |

新增测试覆盖 20,000 行会话的可见窗口、光标位于复合 emoji 内部的旧行为、CJK/组合字符/内联附件样式、缓存修改与克隆隔离、真实 iTerm2 裁剪及选区像素对比、Kitty 首次上传和滚动不重传、Sixel 滚动复用、队列拒绝与 panic 清理、拒绝回调时序和批量预算，以及 256 KiB 粘贴、跨块换行和 IME 确认。

### 长输入绘制微基准

63,000 字节混合 ASCII/中文输入，100 列、8 行视口，调试测试构建，每项运行 15 次取中位数。对照函数保留自第三轮修改前的 `render_wrapped_view`，两者可见文字和光标位置先做相等检查。

| 测量 | 修改前 | 修改后 |
| --- | ---: | ---: |
| 缓存已建立后的重复编辑框绘制 | 199.751 ms | 0.411 ms |

这只测重复绘制：没有计入文本插入、首次折行、网络、终端输出和整屏布局；不能当作生产构建端到端输入延迟。测试位于 `editor_responsiveness_tests.rs`，常规运行标记为忽略的手动微基准，已单独执行通过。复现命令：

```sh
cargo test -p agena-tui-components --lib editor_layout_microbenchmark --offline -- --ignored --nocapture
```

## 第四轮：媒体完成风暴、重复内容页和长行内存

继续保留前面各轮修改，本轮集中检查首次渲染、后台结果的消费及缓存失效。没有将实际 tty 写出移到无序后台线程：终端仍由原有唯一拥有者负责。

| 路径 | 新确认的问题 | 第四轮处理 |
| --- | --- | --- |
| `agena-tui-media/usage.rs`、`agena-tui-app/transcript_state.rs` | 媒体全局代次变化使无媒体的历史乃至已完成图片再次布局；大量图片可能互相触发重排 | 记录每个条目是否还在等待媒体结果，只让待完成/可重试条目观察媒体代次；文字和稳定图片复用条目；全部条目稳定时连聚合历史向量也复用；内容、宽度、主题变化仍正常失效 |
| `agena-tui-media/background.rs` | 未下载的离屏远程图也安排解码任务，生成错误结果再触发一轮布局；超过失败缓存容量可能循环空转 | 确认已有远程字节后才申请解码名额；384 个未下载的离屏 URL 测试验证解码缓存仍空、工作名额不减少、本地代次不变化 |
| `agena-tui-media/graphics_jobs.rs`、`render.rs`、`app_lifecycle.rs` | 完成的编码结果持有名额，但原来只在下一次图像请求时消费；切换到无图界面后可能长期占住名额 | 主循环独立回收编码完成结果；空图像视口也回收，不依赖再次请求同一张图 |
| `agena-tui-media/background.rs`、`render.rs` | Unicode 彩色半块预览仍在布局线程缩放整张原图 | 实时预览通过现有有界媒体工作池缩放，只捕获原图 Arc；待完成时保留最终行数；缓存命中直接组合小图颜色；测试对照同步结果验证颜色和透明合成完全一致 |
| `agena-tui-transcript/content.rs` | 重复/心跳/重连内容页没有新数据也创建新帧，导致布局失效；结构化文档及无内嵌引用文本重复复制 | 所有游标/载荷校验仍执行；无可见变化保留帧 Arc，完成状态/元数据/分页模式变化仍发布新帧；结构化文档共享 Arc，无内嵌引用时源文本和显示文本共享；临时错误与永久失败仍区分 |
| `agena-tui-app/transcript_state.rs` | 冷历史渲染为每个 assistant 部件顺序查找原 Part，形成平方级查找 | 冷缓存需要时才一次建立 Part ID 索引；缓存命中不建立索引，重复 ID 保持旧逻辑的首项语义 |
| `agena-tui-transcript/renderer/transcript_text.rs` | 超长单行、巨大代码块及语言推断无规模限制地进入同步语法高亮 | 高亮预算为总计 128 KiB、单行 8 KiB、最多 2,048 行（制表符按展开后计算）；超限只降级颜色，保留全部源文本和复制内容；普通代码保留多行词法状态 |
| `agena-tui-transcript/render_model.rs`、代码/表格/公式/日志渲染路径 | 每个视觉折行各存完整语义行的 String，长行产生“源行长度 × 折行数”的复制元数据放大 | `navigation_copy_text` 改为共享 `Arc<str>`，同一语义行只建立一次；普通按视觉行选择的 `copy_text` 保持原含义，实际复制动作才生成独立 String；代码、表格、公式和日志的重复导航文本统一共享 |

第四轮新增用例验证：纯文字布局不会因图片完成失效；实际半块图片完成后多次外部媒体代次变化仍复用历史向量；未下载远程图片不创建空任务；隐藏界面独立回收任务；半块预览与同步渲染像素一致；重复文本/PTY 页保持帧身份但错误页仍被拒绝；文档元数据更新不复制整份文档；长代码高亮降级不丢内容；64 KiB 长代码行在 12/80 列视口下共享同一复制源；多行表格导航文本同样共享。

注意：本轮没有新增“端到端延迟提升倍数”的宣称。以上主要是行为与分配身份测试，前文微基准仅属于各自注明的测量条件。

## 第五轮：搜索、单词导航、后台关闭及终端输出

本轮在前四轮工作树上继续检查，没有重置更改，也没有将 stdout 改为并发后台写出。

| 路径 | 新确认的问题 | 第五轮处理 |
| --- | --- | --- |
| `agena-tui-app/transcript_state.rs` | 每次搜索输入调用完整布局失效，清空 Part 投影、条目缓存和布局；每次跳转还复制全部匹配行 | 搜索只更新现有行的匹配投影，不重新排版；原始内容/宽度/主题变化仍正常重建；跳转只读取目标索引，不复制匹配向量，并对过期索引做边界处理 |
| `agena-tui-session/session_helpers.rs` | ASCII 搜索每命中一次都为整个剩余后缀收集字符位置；非 ASCII 高亮与大小写不敏感的匹配列表不一致 | 预处理查询并在行集合复用；一次小写匹配，再线性映射到原始 UTF-8 范围；不再构建后缀索引表；测试 Unicode 字节缩短、字符展开、上下文 sigma 和所有非平凡小写映射 |
| `agena-tui-app/view/view_main.rs`、`agena-tui-transcript/navigation.rs` | 每个样式 span 遍历所有匹配范围；定位首次搜索目标顺序扫描匹配列表 | 高亮顺序消费匹配范围，保留跨 span 样式和文本；首次搜索目标使用二分查找，保持严格前/后查找和循环跳转语义 |
| `agena-tui-app/transcript_state/word_motion.rs` | 每次 `w/b/e/ge` 或 WORD 移动为整段历史生成字素和字符串 | 先从光标方向读取 16 行，需要越过临时边界才按倍数扩展；无长期全历史字素缓存；对照原完整历史算法验证 Unicode、表格分隔、软折行、空白和计数；实际到达边界且目标不再变化时立即结束超大重复计数 |
| `agena-tui-app/app_status_context.rs` | actor 已达到并发上限后只等 JoinSet；UI 消息通道关闭时，没有独立取消路径 | 所有 actor 等待都同时观察 UI 通道关闭；关闭后丢弃排队请求并取消 UI 侧仍等待的任务；完成消息投递失败直接结束，不继续启动排队工作。这不宣称撤销已被服务端接受的业务操作 |
| `agena-tui-platform/terminal/output.rs`、`terminal/mod.rs` | 后端产生大量小块 stdout 写入；退出/挂起误用 `Terminal::flush()` 重画帧差异，并不只是刷新 writer | 64 KiB 有界写入缓冲，仍由唯一终端拥有者串行操作；正常帧与协议事务显式刷新；挂起/退出刷新实际 writer 而非重画；panic hook 已恢复终端时丢弃残留半帧，Terminal Drop 的最终光标命令也不会带出旧缓冲 |

### 第五轮行为及操作数检查

20,000 行合成历史中，普通单词移动只读取初始 16 行；多种方向、单词/WORD、中文、emoji、表格和超过初始窗口的计数均对照完整历史算法。超大 `usize::MAX` 重复计数用例验证在真实边界停止，不按剩余计数空转。

构造 2,000 个有颜色变化的终端单元，使用计数 writer 对比直接后端与缓冲后端：两者输出均为 **43,305 字节，逐字节相同**；底层 `Write::write` 调用由 **24,137 次降为 1 次**。这不是实测系统调用数、网络吞吐或端到端帧延迟；慢终端输出的背压仍可能阻塞。证据在 `/tmp/agena-tui-round5-output-verified.log`。

搜索用例验证：编辑查询不更换布局行/节点向量、不丢条目缓存；新内容到达仍更新结果；Unicode 命中和高亮一致；32 KiB 密集命中不再逐命中建立后缀表；4,096 个富文本 span 的高亮保持全部文字与样式。

## 回归覆盖

第五轮单元回归扩大为 8 个相关包，增加 `agena-tui-session` 的完整单测：**285 项通过、0 项失败**。其中应用层 162 项、会话组件 26 项、基础 TUI 9 项、通用组件 8 项、媒体 29 项、平台 10 项、会话渲染 41 项；`agena-process` 为 0 项 lib 测试。相对第四轮，本轮新增 20 项测试，并额外纳入会话组件中 23 项已有测试，因此不是将总数差 43 项都算成新增。既有手动微基准仍有 1 项按设计忽略。另有 **59 项独立会话渲染集成测试全部通过**；该命令重复运行的 41 项 lib 测试未重复计数。

第五轮日志：`/tmp/agena-tui-round5-verified.log`、`/tmp/agena-tui-round5-integration.log`、`/tmp/agena-tui-round5-clippy-complete.log`、`/tmp/agena-tui-round5-cli-complete.log`。终端 writer 操作数及逐字节对照在 `/tmp/agena-tui-round5-output-verified.log`。

第五轮最终源码重新运行上述单元回归后仍为 285 项通过；全部 8 个包的严格 Clippy（`--lib --tests --no-deps --offline -- -D warnings`）、生产入口 `cargo check -p agena --offline`、修改包格式检查及 `git diff --check` 均通过。没有 Git 暂存/提交、生产二进制替换或应用重启。缓冲写出测试与协议顺序测试使用计数 writer，不等同于跨平台真实终端或慢 SSH 的端到端延迟测量。

第四轮最终相关模块单元测试：**242 项通过、0 项失败**；应用层 151 项，基础 TUI 9 项，组件 8 项，媒体 29 项，平台 5 项，会话渲染 40 项。组件中的 1 项既有手动微基准仍按设计在常规套件中忽略，本轮未重新宣称其耗时。另执行 **59 项独立会话渲染集成测试**，全部通过；`--tests` 命令重复执行的 40 项 lib 测试不重复计数。本轮新增的 15 项单元用例均通过。

第四轮严格 Clippy（所列 7 个包，`--lib --tests --no-deps --offline -- -D warnings`）、生产入口 `cargo check -p agena --offline`、修改包的格式检查及 `git diff --check` 均通过。日志：`/tmp/agena-tui-round4-final.log`、`/tmp/agena-tui-round4-integration.log`、`/tmp/agena-tui-round4-clippy-final.log`、`/tmp/agena-tui-round4-cli.log`。没有暂存/提交更改，也没有替换或重启运行中的程序。

第三轮最终相关模块单元测试：**227 项通过、0 项失败**；应用层 149 项，基础 TUI 9 项，组件 8 项，媒体 23 项，平台 5 项，会话渲染 33 项。另有 1 项标记为手动运行的长输入微基准，已单独执行通过。`agena-tui-transcript --tests` 另外执行 **59 项独立集成测试**（附件 4、内嵌 UI 2、Markdown 22、部件标题 14、工具失败 7、工具视图 7、会话布局 3），均通过；该命令中重复执行的 33 项 lib 测试不重复计数。第三轮严格 Clippy（所有列出的包及组件，`--lib --tests --no-deps -- -D warnings`）通过。

第三轮日志：`/tmp/agena-tui-round3-final.log`、`/tmp/agena-tui-round3-integration-final.log`、`/tmp/agena-tui-round3-clippy-complete.log`、`/tmp/agena-tui-round3-cli-complete.log`、`/tmp/agena-tui-round3-editor-bench.log`。

第三轮最终生产入口检查 `cargo check -p agena --offline` 已通过；所修改包的格式检查和 `git diff --check` 均通过。未执行生产二进制替换、应用重启、Git 暂存或提交。

第二轮工作区单元测试：205 项通过、0 项失败。应用层 143 项，基础 TUI 4 项，组件 5 项，媒体 17 项，平台 5 项，会话渲染 31 项；`agena-process` 本身没有 lib 单元测试，其新增管道接口由平台测试覆盖。生产 CLI 的 `cargo check -p agena --offline`、所列包的严格 Clippy（`-D warnings`）、格式检查及 `git diff --check` 均通过。

增加了子进程退出但后代仍持有管道、stdin 不消费、stdout/stderr 超限、输入完整交付、终端 EOF、解析缓冲按键、状态栏队列满、状态命令后代、剪贴板超时及并发上限、延迟粘贴目标失效、临时文件清理、媒体去重及缓存、真实 PNG 解码、图片重读保留画面、媒体完成通知、Base64 占位和图片增长读取等测试。

工作区验证命令：

```sh
cargo test -p agena-tui-app -p agena-tui-session -p agena-tui -p agena-tui-components \
  -p agena-tui-platform -p agena-tui-media -p agena-tui-transcript \
  -p agena-process --lib --offline -- --test-threads=4
cargo test -p agena-tui-transcript --tests --offline -- --test-threads=4
cargo check -p agena --offline
cargo clippy -p agena-tui-app -p agena-tui-session -p agena-tui-components -p agena-tui-media -p agena-tui-platform \
  -p agena-process -p agena-tui -p agena-tui-transcript \
  --lib --tests --no-deps --offline -- -D warnings
```

Crossterm 被主工作区排除。第二轮通过 `target/tui-terminal-tests/Cargo.toml` 临时清单直接引用仓库内原源码，省略未用于单元测试的示例开发依赖，分别验证默认读取后端和 `use-dev-tty`。生产 manifest 未为测试而改写。两种后端各有 99 项通过、7 项原有忽略项。

## 补全微基准

使用 50,000 条合成相对路径，最多返回 100 项；`rustc -O`，每项运行 15 次取中位数，索引建立时间不计入查询耗时。基线为上述提交中的旧搜索函数，优化版为当前工作树函数；每个查询先验证完整返回结果及排序相同。

| 查询 | 旧实现 | 优化后 | 本机测得倍数 |
| --- | ---: | ---: | ---: |
| 空查询 | 28.599 ms | 0.479 ms | 59.76× |
| `src` | 33.142 ms | 2.966 ms | 11.17× |
| `module1` | 20.408 ms | 3.947 ms | 5.17× |
| `file49999` | 14.732 ms | 3.680 ms | 4.00× |
| `notfound` | 14.334 ms | 3.255 ms | 4.40× |

本机一次性索引预处理为 16.420 ms。测试源码位于 `target/tui-file-bench.rs`，输出保存在 `/tmp/agena-tui-round2-file-bench.log`。这不是完整 TUI 的端到端按键延迟、CPU 利用率或帧率测量，也不能推定其他机器有相同比例收益。

## 保留的边界及未实测场景

强制草稿保存和正常退出的最后持久化仍保留同步耐久性保证；慢磁盘或异常文件系统可能延长这些操作。外部编辑器、pager 和需要独占 tty 的交互式传输会有意暂停 TUI，不应与非交互式后台进程混用。

不可取消的系统剪贴板 API 超时后，正在运行的单个原生调用可能继续存在；其并发名额不会提前释放，避免反复重试制造无界线程。UI 不等待它，后续剪贴板操作会报告忙碌，直到原调用实际返回。

第五轮的 64 KiB 输出缓冲只合并碎片写入，没有把终端设备变成非阻塞设备；管道/SSH/终端的消费速度不足时，实际刷新仍可能等待。单词导航只为实际移动所需区域分配字素，用户明确要求跨越很长范围时会扩大读取窗口；搜索仍需匹配已加载的行，但不会为每次查询重新执行 Markdown 布局。

后台化没有消除所有同步计算：首次 Markdown 解析和布局、预算内的语法高亮、原生图片的轻量放置/切片、以及实际 tty 输出仍可能耗时。原生图片的像素合成和编码、iTerm2 裁剪编码已移到后台；编码任务与像素解码任务各自最多两个，并非所有媒体工作总共只有两个线程。本轮未在 Windows/macOS 原生桌面剪贴板、所有 Kitty/tmux/SSH 组合或受限输出带宽终端上完成端到端性能测试，也未把单元测试结果当作所有环境下绝不卡顿的保证。

## 提交整理与推送前复验

五轮修复结束后，按明确要求进行本地提交整理。已重新获取远端引用并核实仓库默认主分支为 `master`，原始本地和远端基线均为 `c6e813da4`；使用本地整合分支 `fix/tui-responsiveness-audit`，不建立 Pull Request。生产源码在以下复验完成后未再修改。

代码按依赖顺序整理为四个提交，测试与对应实现一起提交，审计文档另作一个提交：

| 提交 | 内容 |
| --- | --- |
| `aaf6db7d9` | 平台辅助进程期限、终端输入边界、缓冲输出与相关测试 |
| `9c9c8abb5` | 编辑器布局缓存、Unicode 搜索匹配与差分测试 |
| `36a633968` | 有界媒体后台任务、协议编码、预览和缓存管理 |
| `e545cd049` | 应用主循环、持久化、剪贴板、会话缓存及搜索导航集成 |

推送前再次以 `--locked --offline` 执行：八个相关包的 lib 单测 **285 项通过、0 项失败、1 项既有手动基准忽略**；独立会话渲染集成测试 **59 项通过、0 项失败**（重复执行的 41 项 lib 测试不重复计数）；八个包的严格 Clippy（`--lib --tests --no-deps -- -D warnings`）及 `cargo check -p agena` 均通过。格式检查和 `git diff --check` 也通过。

本次复验日志：`/tmp/agena-tui-prepush-unit.log`、`/tmp/agena-tui-prepush-integration.log`、`/tmp/agena-tui-prepush-clippy.log`、`/tmp/agena-tui-prepush-cli.log`。提交范围只包含源码、依赖声明、锁文件和本审计记录，不包含构建产物、临时日志或基准可执行文件。
