# 会话一致性与更新时序审计（第二轮）

## 范围与结论

本轮承接 `docs/session-state-audit.md`，继续检查会话状态在 SQLite、运行监督器、内容流、HTTP/SSE、资源版本缓存和 Web Store 之间的传播。目标仓库是 `/home/canxin/Git/ai_research/agena`，分支 `master`，检查起点为 `aa47c881a` 加上一轮尚未提交的工作区修改。

重点覆盖普通会话、fork、fork 的 fork、subagent 及其直接执行，以及 continue、compact、暂停恢复、队列取消、内容结束和连接恢复。这次不仅检查最终数据库值，也用受控屏障、满队列和延迟 Promise 检查事件顺序。发现的问题先在未修复的相应路径上运行回归并确认失败，再修改生产路径。

最终测试结果、构建门禁及覆盖边界见末节。没有把有限测试解释为所有调度顺序、外部服务或第三方工具的绝对正确性证明。

## 状态和版本的定义

| 定义 | 表示什么 | 本轮要求 |
| --- | --- | --- |
| Session ID | 会话的配置、执行控制和资源读取边界 | 共享历史 part ID 不意味着可以跨会话传播可变状态；分支必须读取自己的投影。 |
| Metadata version | 标题、选择、配置等元数据的并发控制版本 | 保持既有 If-Match 语义；不能用它判断工具、队列或执行状态的新旧。 |
| Control version | 持久化会话控制快照的版本 | API 兼容字段 `latest_event_seq` 实际承载这个值，Web 映射为 `controlVersion`；它不是 SSE 游标。 |
| Attempt generation | 同一 execution 的执行代次 | 注册、恢复和认领必须识别代次；旧 supervisor 不能接管、删除或替代新一代的登记。 |
| Entry revision | 条目事实的修订版本 | 父会话的新 revision 不应修改 fork 保存的历史事实。 |
| Kernel watch cursor | owner epoch、订阅范围、真实 commit sequence 和 ordinal | 显式 Reset 与普通 replay 淘汰具有不同的恢复边界，不能混用。 |
| Resource revision / ETag | API 资源的失效与条件读取标记 | Reset 后失效一次，空闲时稳定；后续提交继续使相应资源失效。 |
| Capture state | 原始内容在捕获时的历史事实 | fork 截止时为 Open 的历史记录继续为 Open。有限冻结视图的传输完成状态单独处理。 |

这些版本属于不同的状态域，不能仅因为字段都叫 version 或 sequence 就相互替代。

## 已复现并修复的问题

### 1. 父会话更新污染已加载的 fork 和嵌套 fork

Web `applyEvent` 原先根据共享 part ID，主动把父会话的 `part_updated` 递归传播到其他已加载会话。父工具稍后完成时，两个冻结分支中的工具结果和 run 状态也会改变，抵消服务端已经建立的分支冻结语义。

现在只更新事件明确指定的会话。相同 part ID 在另一个会话中的事实由该会话的 API 投影决定。回归同时加载父会话和两层分支，更新父 run 和工具后，父会话必须变化，两层分支的完整消息快照必须保持原值。

### 2. 较晚返回的旧发送回执回滚完整执行状态

控制状态可在 metadata version 不变时前进。原 Web 只比较 metadata version：较新的 GET 已显示完成，迟到的旧发送回执仍能恢复旧的等待交互状态、队列和暂停标志。

现在完整状态读取和 mutation envelope 通过同一个 control version 检查，再统一写入 parts、state、queue、interaction、attention、usage 和 background activities。较小版本被拒绝，相等版本允许幂等读取或补齐。删除会话会清理对应高水位。

回归按顺序应用 control 12 的完成快照和 control 10 的旧回执，检查完成状态、空队列、未暂停、无旧交互、无旧提醒、用量 12 和已完成的后台活动全部保持一致。

### 3. 会话列表和重新打开缓存会话时状态滞后

没有 control version 的旧列表响应也可能覆盖已排序的完整状态。反过来，如果简单禁止列表写状态，又可能使后台开始的新执行长期显示为 Ready。

现在列表继续更新元数据，同时将执行状态作为需要校验的提示。已有完整状态的会话保留该状态，标记状态资源失效；可见会话安排完整状态读取，未显示会话在 retain 或选择时重新校验。缓存中存在消息页不再被当作执行状态仍然新鲜的证据。

回归既检查慢列表不能回滚新状态，也检查重新打开缓存会话可以读取后台新出现的 Running 状态及其执行上下文。

### 4. 会话切换时旧请求覆盖导航、目录和加载标记

原 scoped selection 等待 hydrate 完成后才导航。A 的读取慢、B 的读取快时，B 完成后迟到的 A 会把 pane 导航回去。workspace 读取也可能把 B 的目录改为 A 的目录。消息加载流程则使用请求开始时的 selected 标志，导致 A 结束后清掉仍在加载的 B 的标记。

现在同一 pane 的多个使用者共享选择所有权；等待结束后必须仍拥有该选择才能导航。目录水合有 window 级所有权，旧请求可以填充自己的会话缓存，但不能修改新选择的目录。取消选择撤销目录所有权。错误和 loading 的写入使用完成时的当前会话身份。

三个回归分别控制消息页和 workspace Promise 的完成顺序，验证导航、目录和加载标记。

### 5. Continue 的新代次被旧监督器吞掉，旧清理也可能删除新登记

执行的终态提交可以先于 supervisor 清理完成。这个窗口中 Continue 已创建新 generation，但 `active` 原先仅按 execution ID 判断是否已有运行对象，因而返回旧对象，不注册新一代。

`RunControl` 现在记录 generation。相同或较新的已登记代次保持幂等；真正的新代次创建自己的监督器，并保留原有 predecessor 顺序。清理时使用对象身份检查，仅删除当前任务拥有的登记，旧任务仍独立完成自己的 settled 通知。

回归在终态提交后保留旧登记，继续同一 execution，验证新代次有新对象、重复同代登记幂等、旧回执不替换新对象、旧清理不删除新登记，以及新一代不依赖下一次 send 或重启即可进展。

### 6. 旧排队监督器可以认领后来重试生成的新代次

真实 SQLite 回归确认：`Queued(g1) → cancel_queued → Continue(g2)` 后，旧监督器仅调用 `claim_next(execution_id)` 会认领 g2，可能把旧 options 或 budget 用到新尝试上。

新增 `claim_next_generation(request, session, execution, expected_generation)`。SQLite 在同一事务中先比较 generation，再检查暂停、FIFO 并认领。不匹配返回 `StaleAttempt`，不会推进 g2 的 phase 或 control version。Runtime 使用新接口，已被替代的任务正常退出，不取消新代次。

原 `claim_next` 保留兼容。内部命令新增字段使用 serde default 和 None 省略规则；回归验证旧 JSON 形状的解析和重新序列化保持一致。

### 7. 暂停后立即恢复，可能只恢复数据库而没有恢复调度

旧监督器的 claim 已被 writer 以 QueuePaused 拒绝，但它还留在 `active` 中时，resume 会看到同代登记而跳过注册。随后旧监督器清理，形成“pause=false、durable Queued、没有监督器”的窗口，需要等待后台扫描或下一次提交。

现在 QueuePaused 退出有明确分类。退出先释放执行锁、按身份清理登记、通知 settled 并释放 admission permit，再与 resume 使用相同 submission 锁复核并恢复队列。仍处于暂停时不会执行。QueueOutOfOrder 不采用这种立即重建方式，避免已有 running owner 时形成忙循环。

回归使用真实 writer 的 QueuePaused 结果，在恢复期间保留旧登记，并占用其余 admission slots，验证旧 permit 被及时释放、同一代次立即得到新的监督器并进展，不依赖后台扫描。

### 8. 内容生产者退出后的终态落库，却没有会话级通知

原后台 checkpoint 会把 abandoned capture 写成 Interrupted、增加 entry revision 并通知 content watch，却不增加 control version、不发布 session change。只观察会话状态或资源版本的客户端因此可能长期保留旧值。

现在终态 checkpoint 在原事务内推进对应会话的 control version，并分配真实 commit sequence。成功顺序为：COMMIT → metadata/history cache 失效 → live acknowledge → Content feed publication。最后一个 producer 释放时主动唤醒 writer，减少等待默认 500 ms 扫描的延迟。

普通字节 checkpoint 仍只更新内容流，不产生逐块会话事件。回归预热缓存，检查 accepted tail 完整、entry 终态和修订更新、控制版本前进、事件游标单调、重连不重复，以及普通 64 KiB checkpoint 不产生控制事件。

### 9. 显式 Reset 在同一 head 重连时可被漏掉

原实现将 replay floor 设置为当前 head，并允许 cursor 等于 floor。客户端收到最后一个 Change、尚未收到 Reset 就断线时，同 head 重连会被错误判定为已同步。

新增独立的 `reset_floor`：显式 Reset 后恢复 cursor 必须严格越过该边界；普通容量淘汰仍保留原来的 inclusive floor。没有伪造或改写真实 commit sequence。

回归分别检查：旧 cursor 触发 needs_snapshot，新建 watch 不反复吐出 Reset；普通 replay eviction 仍接受 inclusive floor。相同的旧 cursor 在没有新 commit 前反复重连，仍会保守要求 snapshot。资源版本消费者在消费重置后保持一个有界 watcher，避免每次空闲条件读取重新触发同一个 reset；对应回归验证两次同 head Reset 各使 token 失效一次，两者之间空闲读取稳定，之后新 commit 仍能失效。

### 10. SSE 恢复通知受到背压时漏掉中间提交

原 Reset 恢复先向客户端发送 Lagged 通知，再注册 replacement watch。发送在满队列上等待时发生的提交不属于旧 watch，也不属于随后建立的新 watch。

现在先注册 replacement，再投递恢复通知。回归填满 mpsc 队列，将恢复 future 停在发送处，在窗口中发布 commit，再释放队列，确认 replacement 能收到这次提交。

### 11. 兼容会话流用 metadata version 跳过必要 snapshot

`/sessions/:id/changes/stream?since_version=...` 原先在 metadata version 相同时跳过首个 snapshot，但编辑 queued input 等操作可以改变 parts 而不改变这个 version。

兼容流现在每次连接都发送有界初始 snapshot。`since_version` 保留为兼容提示，不再作为完整会话同步证明，也不改变 If-Match 的含义。HTTP 回归在受控文本流的 BEFORE 前缀已发布、provider 仍被屏障阻塞时读取基线；放行 AFTER 并完成执行后，确认 metadata version 不变、同一 assistant part 的 revision 前进。携带原 version 重连必须收到相同身份的最新 revision 和 completed 状态，并核对用户文档。

现代 Web 使用 global changes stream，这个问题的直接影响范围是兼容端点的客户端；没有把它描述为现代 Web 主订阅入口的问题。

### 12. 冻结的 Open 内容被当作活跃流，正常读完后返回错误

fork 截止时的内容可能仍记录 CaptureState::Open，但冻结视图不会再接收父会话后续输出。原 API 将其投影为 Active，读完后等待一个已关闭的 receiver，返回 content_error，客户端因此不断重连。

Storage 现在显式提供 `ContentWatch::is_snapshot()`。API 只对 snapshot+Open 的有限视图投影 ContentResource::Complete。GET 与 stream 使用相同规则，分页读完正常结束；历史 CaptureState、tool phase 和 run state 保持不变，真实 live watch 的 Closed 错误也没有被吞掉。

应用层回归覆盖 4-byte 分页和跨页 Unicode；HTTP 回归覆盖父会话、fork、嵌套 fork，确认分支只读到冻结前缀、GET 和 stream 完成、无 content_error、历史 part 仍保留 running，而父内容仍为 Active。

## 验证结果

| 检查 | 结果 |
| --- | --- |
| 八个相关 Rust crate 的 `--lib --tests` | **240 passed、0 failed**；1 个 ignored 是由 crash recovery 测试调用的 `crash_fixture_child` 子进程入口。 |
| Web 全套测试，Bun 1.3.14 | **261 passed、0 failed**；32 个测试文件、1,633 条断言，2.21 秒。 |
| Web 生产构建，Bun 1.3.14 | 图标、import、三组 i18n 检查、vue-tsc 和 Vite build 均通过。 |
| Web 修改文件格式 | Prettier check 通过。 |
| Agena 可执行文件 | `cargo build --locked --offline -p agena --bin agena` 通过。 |
| Rust 格式 | `cargo fmt --all -- --check` 通过。 |
| 全工作区 Clippy | `--workspace --all-targets --locked --offline -- -D warnings` 通过。 |
| Plugin host 可选 feature | `--all-targets --all-features -- -D warnings` 通过。 |
| Runtime 能力门禁 | `CARGO_NET_OFFLINE=true python3 scripts/ci/verify-runtime-capabilities.py` 通过。 |
| HTTP 场景 | **11 passed、0 failed、0 skipped，49.735 秒**；真实新程序与保留的旧程序参与测试。 |
| 差异完整性 | `git diff --check` 通过；本轮 31 个源码/测试文件与执行机逐文件 SHA-256 核对一致。 |

Web 新增 7 项实际 Store/pane 回归全部先复现目标故障，再通过修复后的验证。初步验证同时在机器已有的 Bun 1.3.10 上通过；最终按仓库 `.bun-version` 使用 1.3.14 复核，没有修改全局工具安装或锁文件。

验证日志位于执行机的 `/tmp/agena-consistency-*.log`，核心最终测试为 `core-verified.log`，指定版本 Web 测试和构建为 `web-pinned-tests.log`、`web-pinned-build.log`，完整 HTTP 最终结果为 `http-verified.log`。兼容重连的受控流用例还分别用旧、新可执行文件验证：旧程序因跳过 snapshot 超时失败，新程序严格核对身份、state、revision、content 和用户 document 后通过，记录在 `compat-final-red.log` 与 `compat-final-green.log`。

HTTP 套件实际覆盖：

| 场景 | 验证点 |
| --- | --- |
| 父会话、fork、嵌套 fork | 冻结前缀、独立 continue、compact、实际文件工具、取消与重启后的上下文。 |
| Subagent | 真正的 agent 工具调用、子会话文件读取、wait、后续 compact、完成结果不重复投递。 |
| 非默认 workspace | root 与两层 fork 读取所属 workspace 的文件。 |
| 崩溃恢复 | abandoned attempt 在首次读取前转为终态，队列按 FIFO 恢复且不依赖发送客户端存活。 |
| 暂停 compact | maintenance purpose 跨重启保留，并按 compact 路径恢复。 |
| 旧库升级 | 用旧二进制创建真实数据库与分支，再由新二进制打开，核对历史并执行嵌套分支操作。 |
| 兼容流重连 | 执行完成而 metadata version 不变时，重连获得最新 part 修订。 |
| 冻结内容 stream | fork 与嵌套 fork 的有限视图正常结束，无 content_error；父流保持自己的生命周期。 |

### 可重复运行的主要命令

Web 命令使用仓库指定的 Bun 1.3.14。

```bash
cargo test --locked --offline --no-fail-fast \
  -p agena-domain -p agena-storage -p agena-storage-sqlite \
  -p agena-runtime-session -p agena-runtime-tools \
  -p agena-application -p agena-api -p agena-api-server --lib --tests

cd packages/agena-web
bun test ./tests
bun run build
cd ../..

cargo build --locked --offline -p agena --bin agena
AGENA_TEST_BINARY=/absolute/path/to/current/agena \
AGENA_TEST_PREVIOUS_BINARY=/absolute/path/to/previous/agena \
PYTHONDONTWRITEBYTECODE=1 \
python3 scripts/agena/tests/test_server_lifecycle.py SessionRestartTests -v

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo clippy --locked --offline -p agena-plugin-host --all-targets --all-features -- -D warnings
CARGO_NET_OFFLINE=true python3 scripts/ci/verify-runtime-capabilities.py
```

主要新增回归在：

- `crates/agena-storage/src/kernel/watch.rs`
- `crates/agena-storage-sqlite/tests/content_consistency.rs`
- `crates/agena-storage-sqlite/tests/claim_generation.rs`
- `crates/agena-runtime-session/src/session/manager/commands/queue_tests.rs`
- `crates/agena-application/src/application_content/snapshot_tests.rs`
- `crates/agena-api-server/src/live/tests.rs`
- `crates/agena-api-server/src/revisions.rs`
- `packages/agena-web/tests/chatConsistency.test.ts`
- `scripts/agena/tests/test_server_lifecycle.py`

## 验证边界与交付状态

测试使用临时 SQLite、受控 provider、真实 HTTP 服务器和真实 Pinia/Vue pane 上下文。HTTP 回归覆盖真实服务的持久化、重启、旧 schema 升级、fork/subagent/compact 路径；不连接真实外部模型服务。Web 这次验证的是 store、pane 上下文、受控 API 时序和生产构建，没有运行真实浏览器端到端点击测试。

HTTP compact 用例使用测试模型的本地 summarizer 路径，不能替代真实外部 provider 原生 compact API 的兼容性验证。工具正常工作仍受角色、permission ceiling 和已有能力限制约束；subagent 不因此获得额外权限。外部 ChatGPT→smyl connector 的传输和第三方工具服务没有由本轮测试获得全面可用性证明。

COMMIT 到 live acknowledge 之间，独立的并发冷读仍可能短暂观察到不同层的 phase；同一 SQLite 控制快照内部一致，accepted bytes 已落盘或保留在 live buffer，feed 发布和命令回执之前 acknowledge 已完成。本轮没有复现因此造成的丢字或漏通知，没有为未证实的风险扩展重构。

本报告记录第二轮源码、回归和构建验证。审计未重启用户现有服务；验证服务器使用独立临时数据目录。Git 提交与推送状态以仓库历史和交付记录为准。新行为对应本次验证的代码，不能将源码验证结果当作已部署实例的运行观测。
