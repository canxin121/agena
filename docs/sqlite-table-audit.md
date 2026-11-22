# Agena SQLite 逐表审计

复审在 `agena-sqlite-query-storage` worktree 的 `perf/sqlite-query-storage` 分支完成。`agena-2026-11-22.9` 的实际初始化库含 **72 张业务表、119 个显式索引、45 个触发器和 5 个视图**。数据库使用新数据目录初始化，归档格式为 JSONL v1。 [安装与数据目录](installation.md) [归档写入](../crates/agena-storage-sqlite/src/kernel/archive.rs)

一次 B-tree 点查通常为 O(log N)，定位后读取 K 个连续候选约为 O(log N + K)，其中 N 是索引行数。H 表示会话可见历史条数，F 表示共享该历史的分支数。过滤条件无法由索引判定时，扫描量还取决于候选数量和命中率。空间估算包含历史事实、载荷字节和二级索引。删除行会释放可复用页，数据库文件是否缩小取决于实际的 SQLite 回收方式。

## 实例、工作区与权限

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_instance` | 初始化写入实例 UUID 和 schema 基线。目录刷新和失效清理在同一行更新抓取时间、来源和 32 字节摘要。 | `id=1` 限定单行。目录状态的三个字段一起为空或一起有值。刷新保存状态前核对当前抓取时间和摘要，失效清理核对已读的抓取时间。 |
| `agena_commits` | 命令批次、内容提交、归档发布在事务内递增 `sequence`。订阅和资源版本读取提交水位。 | 常量空间。频繁写入同一页是全实例提交顺序的组成部分。批量提交减少写盘频率，拆分计数器会改变游标语义。 |
| `agena_command_results` | 在同一事务中记录命令结果，重试按请求 UUID 查同一请求的摘要和原始提交位置。普通写入和归档复用回执读写。 | UUID 使用 16 字节 BLOB，摘要固定 32 字节，位置存整数列。可缩小的大 JSON 用 zlib 无损压缩。原始回执仍可重放，存储随成功命令及其编码结果增长。 |
| `agena_server_state` | 应用按键读取或 upsert 实例 JSON，值相同时跳过更新。主键索引只保存键。 | 空间随键数和文档大小增长。每键存当前文档，移除了无读取入口的更新时间。 |
| `agena_workspaces` | 按唯一 `path` 创建或查找，详情按整数 ID，列表按 `updated_at_ms,id`。项目目录用会话的 `workspace_id,owner_bot_id` 检查可见性。 | Web 连续翻页使用目录版本、搜索条件和页大小对应的游标。未访问页的直接跳转仍支持 OFFSET。带前置通配符的 `LIKE` 扫描排序范围内的候选路径。 |
| `agena_workspace_statistics` | 按工作区读取可见会话总数、根会话数、置顶、运行和待关注数量。会话归属、可见性和处理状态改变时在同一事务更新。 | 每个有统计的工作区一行，列表一次取回计数和明细。逐工作区的聚合成本由扫描其会话变为主键点查。 |
| `agena_model_catalog_entries` | 按 `model_id` 读整份官方目录。准备定义 JSON 和摘要后再进入写事务，定义变化时按剩余 ID 删除离开目录的行，每 100 行一批按 `model_id` upsert，JSON 相同时跳过写入。 | 编码及计算摘要仍访问 M 条定义和 B 字节。变化刷新比较新旧目录，实际修改行数为新增、删除和内容更新数之和。完整读取需要构造模型 BTreeMap 和解码所有定义。 |
| `agena_permission_rules` | 主键用于编辑和撤销。三个部分唯一索引绑定 global、workspace、session 的主体键。解析和快照在同一条 `UNION ALL` SQL 中读取各作用域。列表按 `updated_at_ms,id`，全局和工作区分别有按序索引。 | 精确解析最多三次定位，快照访问目标作用域的规则。作用域形状由 CHECK 保证。撤销保留原行并从解析中过滤，管理端仍可查看。任意子串搜索仍扫描该列表范围。 |

读写入口见 [实例 schema](../crates/agena-storage-sqlite/src/schema.rs)、[命令回执](../crates/agena-storage-sqlite/src/kernel/receipts.rs)、[实例状态](../crates/agena-storage-sqlite/src/server_state_repository.rs)、[工作区](../crates/agena-storage-sqlite/src/workspace_repository.rs)、[模型目录](../crates/agena-storage-sqlite/src/model_catalog_repository.rs) 和 [权限仓储](../crates/agena-storage-sqlite/src/permission_rule_repository.rs)。

## 会话、执行与交互

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_sessions` | 创建、配置、重命名、标记、fork 和软删除以整数 ID 为入口。工作区、全局、父节点、Bot 与 surface、置顶和收藏有对应索引。代理状态和 Bot 归属由配置 JSON 投影。 | 普通列表按序读取并逐行查摘要，稀疏过滤会越过更多候选。代理恢复的部分索引只留有状态的会话。族根只在定位会话后读取，移除了无查询使用的族根索引。硬删除仍涉及后代和归档引用。 |
| `agena_session_statistics` | 每会话一行。维护条目、用户消息、已经开始的用户消息、队列、子会话计数和处理状态。创建和会话范围或可见性变更时更新 overview 的工作区键。 | 普通列表和 `/runs` 按主键点查。工作区 overview 从只包含可见、非 ready 会话的部分索引读取本工作区候选。全局状态索引服务普通状态过滤。rewind 按移除范围扣减已开始消息数。条目数、用户数和最大时间由保留的段边汇总。 |
| `agena_session_runs` | 按 `(session_id,owner_entry_id,entry_id)` 保存可见 run 边界。首次开始、首个响应、fork、rewind、clear 和归档导入与历史成员变更同事务维护。普通追加按新条目的 ID 定位 run 和首次响应。 | 首页和深页按逻辑键取 K 条，下一页游标需要物理位置点查。rewind 先用会话段主键定位移除的后缀，再由条目计算待删除 run 的完整主键。成本由移除的历史条目数决定，分支复制仍访问共享前缀。 |
| `agena_session_response_segments` | 保存每个回复出现过的段、段尾累计条目数。首次响应把早期 notice 纳入回复，后续追加更新尾段。fork 复制完整前缀段，剪切和 rewind 只重算边界段，导入由历史成员重建。 | 每个回复与相交段一行。精确大小读取最后一行，分页从请求位置向前读取有该回复的段，每段至多 128 个候选。后缀索引服务 fork、rewind 和 clear。 |
| `agena_executions` | 发送或控制动作建立 execution，retry 推进 `current_generation`。UUID 查详情，`session_id,message_id` 查最新输入，按 `session_id,id` 遍历。 | 组合唯一键 `(id,session_id)` 支持 attempt 复合外键。领取和清队列从活跃 attempt 进入。推进代次时同事务移除旧代次尚待发布的频道回执。指定执行的文本读取使用代次索引定位可见源。 |
| `agena_attempts` | 每执行代次一行。领取、完成、取消、恢复更新状态和时刻。复合主键定位代次，部分索引覆盖活跃、运行中、终态和非空响应锚点。归档先写代次，再写响应条目，响应外键随循环引用在提交时校验。 | 终态代次随历史积累。领取检查最多访问 A 个当前活跃代次。会话有频道绑定时，终态转换写待发布投影。指定执行代次只读取其可见段中的文本源。 |
| `agena_execution_purposes` | continuation 和 compaction 的创建与导入写入。控制快照、历史与提示词按 execution ID 点查，普通用户执行用缺省语义。 | 稀疏扩展表，空间随控制执行数增长。使用 `WITHOUT ROWID` 让窄数据和主键共用 B-tree。 |
| `agena_entries` | 追加输入、notice、工具、交互、输出和压缩条目时写入。PK 读正文，origin、attempt、message、round 和资源索引支持不同查找。读取页先取身份和字节数，再应用分支冻结事实。 | 条目和多数索引随历史增长。合并历史与 prompt 的正文预检索引。压缩检查点有独立的小型部分索引，普通轮次可直接判定缺失。精确条目查询与软删除过滤需要保留可见性检查。 |
| `agena_rounds` | 开始和结束模型轮次写入 phase、provider state 与输入元信息。冻结源时在同一事务写入源数量和字节预算，导入发布按新 ID 重算。按 attempt、session 或 execution 查轮次。 | 空间随轮次及状态字节数增长。活跃索引避开终态，用量用部分时间索引。回放块在建立时用已经写入的 provider state 尺寸计算预算，读预算省去轮次逐项探查，移除了原尺寸覆盖索引。 |
| `agena_tool_calls` | prepare、activate、完成及恢复写入工具事实。按 ID 或 round 内 call ID 查详情，部分索引定位草稿和活动调用，覆盖索引预检输入和结果尺寸。 | 工具结果通常比元数据大。会话用量从本会话轮次查仅含账单的工具索引。实例用量保留时间索引，两者各存每个计费工具调用的窄键。 |
| `agena_interactions` | 工具权限和用户输入的请求、呈现、回复、取消同事务更新。按 tool/attempt 查回复，待处理索引按 session 或 tool 定位，字节索引支持提示词预检。 | 大部分请求最终成为历史记录。终态请求与结果各自提供证据。回收前要确认分支、提示词和归档中的可达性。 |
| `agena_deliveries` | 后台、子代理和集成投递按 business key 去重，按 session 读 pending，观察时关联 round，消耗后按 resume execution 和 generation 回查来源。 | 移除了无消费者的 claimed 状态和租约列。两种目标索引都排除空目标。pending 部分索引用于领取、取消和恢复，有目标的全状态索引服务反向关系。已消费历史仍承担幂等，续跑索引只维护有续跑身份的行。 |
| `agena_background_activities` | 启动或结束进程、终端、代理和监控写入。活动列表以 session 的 running 索引读取，工具和 entry 有反向索引，分支读取冻结结局。归档从 `(entry_id,id)` 取候选后按主键关联冻结记录。 | 每活动一行，活跃索引只随运行中记录增长。每工具最多 16 个活动。终态含结果和审计信息，清除前需同时检查 entry、content stream 和冻结引用。 |

读写入口见 [命令](../crates/agena-storage-sqlite/src/kernel/commands.rs)、[会话生命周期](../crates/agena-storage-sqlite/src/kernel/lifecycle.rs)、[摘要查询](../crates/agena-storage-sqlite/src/kernel/summaries.rs)、[详情读取](../crates/agena-storage-sqlite/src/kernel/reads.rs) 和 [后台活动](../crates/agena-storage-sqlite/src/kernel/activity.rs)。

## 历史成员关系、快照与分支

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_history_segments` | 首次追加、尾段装满、复制共享尾段或导入时分配整型 ID。clear、rewind、withdraw 将完全移除的边交给维护器。 | 每个实际段一行。没有任何边引用的段可随成员一起回收。普通私有尾段复用原 ID。归档显式分配新段 ID 时同时检查 `sqlite_sequence` 的高水位，避免重复使用已清理的旧 ID。 |
| `agena_segment_entries` | 段建立时写入 `(segment_id,ordinal,entry_id)`，挂上当前或退役边后受不可变触发器保护。PK 用于顺序页。`UNIQUE(entry_id,segment_id)` 同时服务选定条目反查、成员资格检查和同段去重。 | 普通追加在写事务里暂时摘下私有尾边，只插入新成员。共享尾段只复制可见前缀，单段上限 128。重用被截断的私有尾段前会删除越过边界的残余成员。每条成员维护主树与一棵反向树，反查成本取决于条目仍存活的段副本数。 |
| `agena_session_segments` | 当前历史按 `session_id,sequence` 读范围。每条边记录其可见区间的用户数和最大创建时间。fork 复制保留边的统计，裁剪时只重算发生变化的最后一段。追加使用新条目的种类和时间更新尾段统计。反向索引用段 ID 和会话定位边。 | 新增空间为每条当前边两个整数。设 G 为保留段数，摘要重算定位后读 O(G) 条窄边，部分段至多再读 128 条成员。复制前缀按 G 增长，正文和段内成员保持共享。 |
| `agena_history_snapshots` | 导出打开时保存 token、代次和末段位置。按 token 校验权限，按 `session_id,generation` 判定旧段仍被引用。释放快照与退役边清理在同一事务内执行。 | 全局最多 8 个快照，每会话最多 2 个。数据库存根与边界，导出流不需要在应用内存保存所有历史。 |
| `agena_retired_session_segments` | 旧快照需继续读取的边和完全摘除的边都记录代次区间。快照用 `(session_id,sequence,since_generation)` 唯一索引按顺序选边。回收按单调 ID 框定至多 64 条，批量删除无快照引用的边。 | 空间取决于固定快照和待回收边数。批量取得被删边对应的段 ID 后，按当前与退役边的反向索引判断段是否可回收。每段最多 128 个成员。 |
| `agena_branch_entries` | fork 批量冻结继承条目的修订、执行状态、响应锚点和资源可见性。PK 按会话与 entry 点查，反向索引定位冻结引用。 | 标量以窄列存储，空交互、活动与附件数组按 NULL 保存。内容源列表从冻结源关系生成，条目行省去重复的 UUID 数组。嵌套分支复用上层冻结值，冻结条目关系仍按 O(F × H) 增长。 |
| `agena_branch_sources` | `(session_id,source_key)` 保存冻结 epoch、字节边界、状态及丢失计数。嵌套 fork 对已冻结的源直接引用最早持有该头部的行。首次冻结和归档导入写入完整头部。 | 每个分支与可见源保留窄关联。嵌套分支无需复制头部。复合外键与触发器保证共享行属于同一源且只指向完整头部。 |
| `agena_branch_rounds` | fork 冻结 phase、provider state 可见性和完成时间。分支提示词和轮次详情以复合 PK 查询。 | 每分支每个可见轮次一小行。归档验证从 round 索引找可能的条目再点查分支事实，避免每个轮次重复扫分支前缀。 |
| `agena_branch_activities` | fork 冻结可见结局与完成时刻。活动详情按 `(session_id,activity_id)` 读取。导出先按条目查活动，再逐个点查冻结标记。 | 分支共享实际结果，冻结行随继承活动数增长。导出逐条目核对候选的成本受单工具上限约束。反向索引支持活动父表的引用检查。 |

读写入口见 [历史分页](../crates/agena-storage-sqlite/src/kernel/history_views.rs)、[分支冻结](../crates/agena-storage-sqlite/src/kernel/branches.rs)、[快照与退役 GC](../crates/agena-storage-sqlite/src/kernel/snapshots.rs) 和 [段写入](../crates/agena-storage-sqlite/src/kernel/commands.rs)。

视图 `agena_branch_source_heads` 从分支键回到本行或共享头部。工具详情、源读取与 JSONL 导出先按可见范围定位，再取冻结头部。 [分支源读写](../crates/agena-storage-sqlite/src/kernel/branches.rs)

## 模型输入与压缩

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_prompt_windows` | 每会话记录 rewrite epoch、checkpoint、through 位置及压缩状态。compaction、rewind、clear 和归档导入更新。checkpoint 与 through 有反向索引。 | 一会话一行。按 PK 读窗口，前缀裁剪需尊重 checkpoint 的保留边。当前窗口变动会同步推进版本，便于检测旧游标。 |
| `agena_compaction_rounds` | 开始压缩时记录基准 checkpoint 和 epoch。完成时核对，历史读取用于解释压缩状态，归档重建。 | 随压缩轮次数增长。基准 checkpoint 的反向索引支持查引用。普通轮次无需占用这些行。 |
| `agena_prompt_input_blocks` | 以整数主键保存块身份。捕获校验条目位于本会话的可见历史，再按稳定的 64 个 entry ID 区间划分连续输入。通过 32 字节 SHA-256 摘要的唯一索引复用有序冻结事实完全相同的块。 | 每个不同的块一行。重复窗口只增加轮次引用。稀疏或频繁变化的事实会产生更多块，短窗口的摘要和索引固定开销占比较高。 |
| `agena_prompt_input_entries` | 每块保存局部 ordinal、entry ID、revision、响应锚点、执行与工具状态及交互是否已决定。按块主键顺序展开，`UNIQUE(entry_id,block_id)` 同时支持源关系的外键和冻结编辑校验的反查。 | 每个不同块最多 64 行。非空响应锚点有部分反向索引。各轮均可共享实际条目正文与工具结果。 |
| `agena_round_input_blocks` | 用 `(round_id,start_ordinal)` 按序登记块。`UNIQUE(block_id,round_id)` 支持按块反查轮次以及校验源关系所属的轮次和块。 | 大窗口中的连续区间用一个窄引用代替多条逐轮事实。导出使用已索引的起始 ordinal 找游标所在块，最多回读该块 64 项。 |
| `agena_round_input_sources` | 按 `(round_id,source_key)` 冻结 epoch、已提交位置、保留起点、状态及丢失计数。两个复合外键核对轮次和块成员关系。继承源直接读取分支冻结列。 | 源字节预算写入轮次，预检直接读取。导出先读最多 64 个源键及字节数，再批量重建约 256 KiB 预算内的头部，同时校验 epoch 和提交边界并取得内容流的所属条目。每轮源头单独保存。 |
| `agena_prompt_replay_blocks` | 用冻结 replay 集合的 SHA-256 摘要去重。每块保存成员数和冻结 provider state 的字节预算。 | 同一轮的预算从其链接的块求和，读 B 个块即可核算 V 个 replay 的预算。每块覆盖固定的 64 个轮次 ID，稀疏窗口中的 B 可接近 V。 |
| `agena_prompt_replay_entries` | 用 `(block_id,source_round_id)` 保存源轮次、冻结 phase 和 provider state 可见性。用源轮次反向索引支持外键查找。 | 各块存实际存在的成员，底层 provider state 由源轮次保管。完整回放仍需展开 V 条，并读取冻结时可见的正文。 |
| `agena_round_replay_blocks` | 每个轮次和 ID 区间保存一条块引用。主键支持有序展开和导出游标，块 ID 有反向索引。 | 总空间随各轮的块引用数增长。移除窗口首尾条目通常只改变它们所在的块，生成的相同内部块可复用。 |

视图 `agena_round_inputs` 将块起始 ordinal 与块内 ordinal 相加，供读取、预算预检、源和回放捕获及归档导出使用。读取先在 SQL 核算 10,000 项和 8 MiB 的预算，再按块序、块内序逐行解码。每轮仍需核对选中成员并读取各条冻结事实。用 U 表示不同块的数量，Q 表示这些块实际保存的事实行，L 表示轮次到块的引用数，清单的关系空间为 O(U + Q + L)。 [输入窗口](../crates/agena-storage-sqlite/src/kernel/input_window.rs) [冻结输入](../crates/agena-storage-sqlite/src/kernel/prompt_inputs.rs) [输入预算与回放](../crates/agena-storage-sqlite/src/kernel/prompt.rs) [源清单](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs)

视图 `agena_round_input_replays` 按轮次、块区间和源轮次展开冻结清单。JSONL 导出从游标对应区间开始按主键读，每页 200 项，同时取回源轮次所属的执行输入。源轮次的状态正文在轮次结束时写入，冻结时只在可见标记为真时计入预算。 [回放块与分页](../crates/agena-storage-sqlite/src/kernel/prompt_replays.rs)

## 内容与附件

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_content_streams` | 开启捕获后持续更新 head、epoch、状态和丢失计数。按唯一 UUID 读头，分支和轮次源都通过 `INTEGER PRIMARY KEY` 关联源身份。按 entry 或 activity 读源，按会话或代次定位 open 源以终止和恢复。 | 每条源一行，源所属条目和静态描述集中保存在此。会话终止和执行代次的活跃索引覆盖不同顺序。活跃源上限为 128，正文存于块表。 |
| `agena_content_blocks` | 以 `(source_key,logical_start)` 追加不可变块，按区间读文本或事件。正文至多 64 KiB 时内联。保留维护按批删除 sealed 前缀。 | 空间随编码后的字节和块数增长。重叠校验定位前驱。公开 UUID 在内容流解析为整型键，客户端游标继续使用原逻辑位置。 |
| `agena_content_block_pieces` | 大块以 `(source_key,block_start,byte_offset)` 存固定 64 KiB 分片，最后一片可更短。归档按键读一片，正文读取按原块校验和重建。 | 导出逐片读取的内存随片长增长。多分片的记录头和 B-tree 占用少量额外空间。保留清理级联回收。 |
| `agena_content_checkpoints` | 事件捕获时登记 checkpoint 所在块。按 `source_key,logical_sequence` 找最近检查点，并按 `source_key,block_start` 随被删块清理。 | 空间随检查点数增长，两个顺序各用于定位或清理。增加检查点会降低回放长尾，同时增加块和索引体积。 |
| `agena_content_capture_losses` | 捕获丢失时写入连续、不可变的包号区间。分页按 `(source_key,first_packet)` 定位游标前驱，核对窗口终点。读取页、归档读取和导入验证按源与区间起点访问。 | 游标校验为一次主键范围定位。读取页用区间起点限定上下界，停在冻结头之内。同一位置连续丢失可在写入前合并。 |
| `agena_content_gaps` | 保留维护 upsert 前缀 gap，读和导入依据 `source_key,logical_start` 核对。 | 当前保留路径对每源更新起点 0 的一行，`retained_start` 同步推进。逻辑 gap 保留给客户端区分可读尾部和被回收的范围。 |
| `agena_blobs` | 创建、上传完成、GC 标记更新 metadata。引用和 pin 变化时维护无保护标记。按可回收时间索引取到期项，退役后按顺序清理块。 | 仅无保护且未退役的行进入判定索引。回收前仍核对引用和 pin，每批最多退役 16 个对象、删除 8 个块。 |
| `agena_blob_chunks` | 上传和导入写 128 KiB 对齐分块，读范围按 `blob_id,logical_start`，GC 每批至多删 8 块。 | 主要随文件字节增长。表使用复合主键和不可变触发器；未来若对相同摘要的块去重，需验证写中断、引用计数和导出的相同字节语义。 |
| `agena_blob_references` | 附件提交关联 entry、blob、field，编辑排队消息替换其引用。按 entry 预检和读描述，GC 以 `blob_id` 反查引用。 | 每引用一条窄边。顺序按字段号转换，单条 entry 读取有 16 项预算。跨分支仍用共享正文及冻结的 blob 可见性。 |
| `agena_blob_pins` | 上传与读取能力以 `blob_id,token,owner_epoch` 固定 blob。释放后 GC 删除 pin，重启清理上任进程记录。 | 上限 128。生命周期由内存中的句柄和宽限期管理。移除了恒为最大值的过期列与无读路径的索引。 |

读写入口见 [内容流](../crates/agena-storage-sqlite/src/kernel/content.rs)、[内容保留](../crates/agena-storage-sqlite/src/kernel/content/retention.rs)、[丢失记录](../crates/agena-storage-sqlite/src/kernel/content/loss.rs) 和 [附件](../crates/agena-storage-sqlite/src/kernel/blobs.rs)。

## Bot

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_bots` | UUID 点查，创建、修订和归档写入。活跃名称由部分唯一索引保护。活跃列表一次查询联查头像、prompt、本地会话并按当前默认规则排序。 | 产品上限 256 个 Bot。列表仍返回完整 profile，总内存随实际头像及说明总字节数增长。 |
| `agena_bot_avatars` | Bot 写入同一事务按 bot_id 替换或删除。详情和列表联查头像。 | 隔离了可达 256 KiB 的 data URL。若列表开始积累多个大头像，按需要投影或单独取图比增加排序索引更节省读取和序列化。 |
| `agena_bot_prompts` | 以 bot_id 存 prompt ID 列表和发布开关，更新 Bot 时同事务写或删除。 | 每 Bot 至多一行，按主键联查；全量列表会读这几个小字段。当前查询不需要对 JSON 内各 prompt 建索引。 |
| `agena_bot_local_sessions` | 独立本地对话按 bot_id 保存，session_id 唯一。列表用主键联查映射和可见会话。 | 每 Bot 至多一映射。点查和更新为对数成本，复用列表联查免去逐 Bot 往返。 |
| `agena_bot_receipts` | 保存创建、更新、归档请求的输入摘要和原始结果。按 request key 点查后补当前本地会话映射。 | 回执数可以超过 Bot 数。大结果无损压缩，解码恢复原始 JSON 字节。更新 Bot 和写回执使用各自的事务。 |

读写入口见 [Bot 仓储](../crates/agena-storage-sqlite/src/bot_repository.rs)、[Bot 注册和重放](../crates/agena-runtime-session/src/session/manager/bot/registry.rs) 和 [本地对话](../crates/agena-storage-sqlite/src/kernel/lifecycle.rs)。

## Assistant

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_assistant_profiles` | 按 workspace ID 读当前偏好。配置变更校验 revision，事务内 upsert 与保存回执。 | 每工作区一行，主键足够。当前维护了 JSON 中的 revision 和单独列，未来可统一其存储并保留校验。 |
| `agena_assistant_global_profile` | 无工作区会话按 `singleton=1` 取全局偏好，独立更新。 | 单行。独立作用域让全局偏好、项目偏好和提醒预算保持各自的默认值和版本。 |
| `agena_assistant_profile_receipts` | 全局和工作区配置按 `(scope_id,request_key)` 重放，`0` 表示全局，正数表示工作区。写偏好和回执使用同一事务。 | 单个 BLOB 保存 32 字节摘要和有标记的结果。重放先比对摘要，再解码原始偏好。合并编码避免在接近溢出页阈值时为作用域多用一个记录头字节。 |
| `agena_assistant_reminders` | 创建时按来源会话与 request key 去重，单条更新按 ID 和 revision。到期读取先探查原顺序头部，待派发稀疏时用有效截止索引。 | 每作用域最多 32 个未结束提醒。到期批最多 32。大量可派发候选紧随较长的冷却前缀时，保留原派发顺序仍需扫描此前缀。 |
| `agena_assistant_contact_budget` | 预订和完成按 `(scope_id,local_day)` 更新。日期按作用域时区计算并存为日序整数，`0` 作用域表示全局。 | 每个作用域每天至多一行。全局用部分索引读当日与最近联系时刻，工作区按主键与作用域索引定位。跨午夜时，最近联系可以来自前一天。 |

读写入口见 [提醒、配置和预算仓储](../crates/agena-storage-sqlite/src/companion_repository.rs) 和 [全局表声明](../crates/agena-storage-sqlite/src/schema.rs)。

## 集成与频道

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_integration_bindings` | 创建按来源和 request key 去重；修改以 ID 和 revision 比较。工作区与 Bot 各有归属排序索引，enabled 索引服务 poll。 | 每 owner 有容量限制。连接设置、状态及水位以小行和 JSON 保存。对账只选 enabled 或仍有未结束事件的连接。 |
| `agena_integration_inbox` | 按 `(binding_id,event_id)` 去重。最近列表按接收时间，入箱领取和配额按绑定与 epoch 在部分索引内定位。 | 每个当前 epoch 至多有 128 个待领取事件。一次领取最多返回 32 项，清理旧 epoch 也有独立批次上限。状态和执行身份的更新只写窄行。 |
| `agena_integration_event_payloads` | 以整数 inbox ID 存单份事件 JSON。事件入箱和正文写入在同一事务。 | 计数、领取和状态更新使用元数据，返回事件时才联查正文。保留原始输入供去重、执行和审计使用。 |
| `agena_integration_outbox` | 按 ID 幂等写出站项，成功插入时同步消耗同 ID 的待发布结果。领取合并零截止时间和到期重试范围，按创建时间与 ID 取 32 项。 | 领取、重试和故障恢复直接更新状态列。传输结果及幂等身份保留，近期记录自动重排队时只更新候选行。 |
| `agena_integration_delivery_text` | 以整数出站键存消息正文，送出和展示历史时按键联查。 | 同一出站项的多次尝试共用正文。记录体积随入队消息字节数增长，状态变化不反复写消息。 |
| `agena_channel_sessions` | 同绑定同 scope key 唯一，按 session 读和恢复。待发布结果、交互决定和通知回执以触发器同步维护 `has_work`。 | 每绑定最多 2,048 个映射。对账通过就绪部分索引定位，空闲映射不占该索引。删除映射级联移除待发布投影。 |
| `agena_channel_pending_outputs` | 保存当前代次尚待发布的结果。终态转换入队，retry、映射删除或出站回执提交移除。以会话和完成位置读最多 16 项。 | 每个待发布结果一行，主键按会话有序扫描，唯一 delivery ID 负责定点确认。单会话对账的候选量由待发布批次决定。 |
| `agena_channel_access_audit` | 策略修订、平台撤回、人工 review 写审计。手工修改按 `(binding_id,request_key)` 重放。最近 100 条按 `(binding_id,at_ms DESC,request_key)` 读取。 | 结果无损压缩，原始审计证据单独保存。仅重放路径解码完整结果。 |
| `agena_channel_user_budget` | 频道事件通过验证和去重后，用条件 UPSERT 原子更新 `(binding_id,actor,utc_day)`。 | 每主体每天至多一行，按主键检查并消费配额，一条语句完成。 |
| `agena_channel_rooms` | 首次有效观察写入 `(binding_id,resource,kind)` 唯一行，与成员 upsert 同事务。房间列表从此覆盖索引读 binding 的范围。 | 列表成本随房间数 R 增长，空间是每种房间身份一行及唯一键。多人房间节省重复前缀，单成员房间增加一行及唯一键开销。 |
| `agena_channel_participants` | 按 `(room_id,actor)` 保存已验证观察、首次与最后观察时间以及名称。按房间和 actor 游标取成员。 | 每主体一行，重复的 binding UUID、房间标识和 kind 存在房间表。无搜索时先点查房间再读小范围。子串搜索仍按房间扫描候选。 |

读写入口见 [集成仓储](../crates/agena-storage-sqlite/src/integration_repository.rs)、[频道访问与审计](../crates/agena-storage-sqlite/src/integration_repository/channel_access.rs) 和 [参与者](../crates/agena-storage-sqlite/src/integration_repository/channel_participants.rs)。

视图 `agena_integration_events` 用整数键为需要正文的读取恢复完整入箱事件。视图 `agena_channel_session_work` 供触发器核对单个会话是否还有待发布的结果或待通知的交互。 [入箱表及视图](../crates/agena-storage-sqlite/src/integration_repository.rs) [频道就绪视图](../crates/agena-storage-sqlite/src/integration_repository/channel_access.rs)

## 调度器

| 表 | 主要读写及索引 | 复杂度与空间判断 |
| --- | --- | --- |
| `agena_scheduler_jobs` | 按 ID 保存作业 JSON 和热字段。deadline、expired claim、owner/session 和 pending work 各用对应索引，领取和续租校验持有者。 | 作业和调度历史独立读走读池，续租只更新热字段。删除了无读入口的 `updated_at_ms`。已完成作业仍可查询。 |
| `agena_scheduler_history` | 保存作业结果，按全局时间或 job ID 顺序读，写入事务同时删除最早超额记录。 | 全局最多 1,000 条。两个索引分别服务时间顺序和单作业查询。每次裁剪用 COUNT 决定超额，工作量由保留上限约束。 |

读写入口见 [调度器 schema](../crates/agena-scheduler/src/schema.rs)、[任务与历史存储](../crates/agena-scheduler/src/store.rs) 和 [到期及租约扫描](../crates/agena-scheduler/src/store/scheduling.rs)。

## SQLite 内部表与归档工作表

- `sqlite_sequence` 随 AUTOINCREMENT 表保存已分配高水位。清理掉最后一个段后，归档仍以高水位分配身份。删除此内部记录会改变新 ID 的分配。实验的 `ANALYZE` 可创建 `sqlite_stat1` 和 `sqlite_stat4`，它们保存统计信息，属于 SQLite 内部对象。 [归档发布](../crates/agena-storage-sqlite/src/kernel/import.rs)
- 退役边扫描进度由串行写入器持有，按事务提交结果推进。释放快照后完成当前扫描，再回查此前经过的范围。重启清除旧快照根并从头扫描留存的退役边，原来用于保存游标的 `agena_history_snapshot_gc_state` 已删除。 [快照回收](../crates/agena-storage-sqlite/src/kernel/snapshots.rs)
- 暂存数据库有简化的 `agena_workspaces` 外键占位、归档涉及的业务表和八个私有工作表。输入与 replay 用平面表保留 JSONL v1 的序号和事实。分支与轮次源以 UUID 暂存完整头部，验证源列表、所属条目、位置和代次。回放校验每轮项数和可见 provider state 是否存在。解析、摘要核对和全图验证在取得主库写锁前执行。 [暂存与验证](../crates/agena-storage-sqlite/src/kernel/import.rs)

| 私有表 | 用途与复杂度 |
| --- | --- |
| `import_entries` | 记录条目 ID、是否属于可见历史和 ordinal。按 `(member,ordinal)` 定位每段起点并构造段成员，通过行主键定位 prompt 的 through 引用。新段边的统计按段成员主键顺序构造。空间随归档条目和依赖条目数增长。 |
| `import_source_owners` | 逐条记录 content ID 对应的 entry，以 content ID 查重和验证清单两端的拥有关系。随源数增长。 |
| `import_source_bounds` | 记录每源的物理起点。验证块总长及保留区间时按 content ID 与 stream 关联。随源数增长。 |
| `import_maps` | 复合 PK `(kind,old)` 保存旧 ID 到新 ID、epoch 的映射，供验证后重建关系和重写 JSON。随归档所含身份数增长，单个引用按键查。 |
| `import_pieces` | 按 content、块起点和偏移暂存分片，校验合并后删除对应分片。单片 64 KiB，临时磁盘用量受当前块大小和 SQLite 页复用影响。 |
| `import_events` | 按源和序号记录事件与块关系，以主键对比 checkpoint 声明。随归档内的事件数增长，住在私有数据库里。 |
| `import_validation_state` | 保存每源最后验证过的 terminal/document 状态，使下一块可以继续验证。按 content ID 点查与 upsert，容量取决于需要保留的各源当前状态大小。 |
| `import_json_remaps` | 在暂存阶段记录输入交互仍待重写的新编号位置和已解析的结构。发布时通过映射及 SQL JSON 函数重写对应字段。空间只随含这类引用的行数和载荷增长。 |
| 导出 `entries` | 在私有 SQLite 文件登记固定成员与跨条目依赖，以 PK 去重，以待输出部分索引保持成员顺序，再输出依赖。只写整数位置，在私有事务内批量构建和消费，导出结束丢弃。真正实例数据库的读取在写 sink 前结束。 |
| 导出 `rounds` | 审计发现原表登记 round ID 后从未读取，已移除。轮次依赖仍从导出的实际轮次事实推进。 |

导出图的空间随可见成员与依赖闭包增长。初次登记只按固定快照读取 ID 和物理位置，每 128 条批量写入。执行代次、工具内容引用、轮次输入、源与 replay 去重后批量核对或登记。读事务在写私有工作表和写 sink 前结束，JSONL 按原游标顺序逐条输出。导入在暂存阶段完成公开内容 UUID 的映射和相关 JSON 重写，发布时关联旧源键和新源键。输入与 replay 各自以 1,024 条为一页重映射后分块。初始统计从段边取得条目数和用户数。发布按依赖顺序写入，仅延迟声明为循环的引用，提交失败时整个归档及回执一起回滚。 [导出工作图](../crates/agena-storage-sqlite/src/kernel/archive.rs) [快照分页](../crates/agena-storage-sqlite/src/kernel/snapshots.rs) [导入发布](../crates/agena-storage-sqlite/src/kernel/import.rs)

## 索引与外键复审

按真实初始化库的 `sqlite_schema`、`index_xinfo` 和 `foreign_key_list` 核对声明、查询入口、索引顺序以及删除或回收路径。对 96 组外键的子表列执行等值探查，89 组可通过主键、索引或索引前缀定位，7 组全范围扫描。完整的表名、列、索引条件、外键反向引用和读写源码行号在本 worktree 的 `target/sqlite-v9-audit/`。

| 扫描对应的父表 | 子表与条件 | 当前写路径 |
| --- | --- | --- |
| 条目 `agena_entries` | 分支冻结条目的 `response_entry_id`、run 边界的 `entry_id` | clear 和 rewind 改变成员与 run 关系，正文保留在事实表。回收历史段走 `segment_id` 范围。 |
| 执行 `agena_executions` | 投递的 `resume_execution_id` | 续跑读取带已消费状态，使用部分索引。执行记录保留各代次及幂等依赖。 |
| 会话 `agena_sessions` | 附件和投递的 `session_id`，会话自身的 `root_session_id`、`parent_session_id` | 会话软删除。正常的子会话列表附带可见性条件，使用部分索引。附件清理以 blob ID 和引用反查索引为入口。 |

段、内容块、附件和工作区的实际删除入口有对应的外键反查索引。四组前缀相同的唯一键同时承担单体身份与复合拥有关系，分别位于 attempt、activity、execution 和 tool call。退役边的有序读取复用唯一索引，在表行中读取终止代次和实际段范围。轮次源用一个反向索引覆盖两种块外键。按 `(block_id,entry_id)` 探查先从索引定位块，再筛该块的条目；当前保留轮次和冻结事实，硬清理大量共享块时需要重新测量这一前缀范围。 [内核索引与约束](../crates/agena-storage-sqlite/src/kernel/schema.rs) [附件 GC](../crates/agena-storage-sqlite/src/kernel/blobs.rs) [段 GC](../crates/agena-storage-sqlite/src/kernel/snapshots.rs)

归档发布原先把所有外键延迟到提交。存在未完成的循环引用时，SQLite 插入每条新正文都会检查现有的 run 和分支响应锚点，两条子表路径各扫描已有行。限定延迟外键后，发布使用循环引用已有的反向索引，选定样本的正文和内容流插入工作量与这两张历史表的大小脱钩。 [外键声明](../crates/agena-storage-sqlite/src/kernel/schema.rs) [发布事务](../crates/agena-storage-sqlite/src/kernel/import.rs)

## API 与后续复杂度

- `/runs` 按所属用户输入排序，支持在上一轮正文追加前先排队下一轮。索引分页加一个物理位置点查，精确用户数来自统计表。回复超出子页时，按回复键取最后一段的累计条目数。 [运行边界](../crates/agena-storage-sqlite/src/kernel/history_runs.rs) [分页](../crates/agena-storage-sqlite/src/kernel/history_views.rs)
- 续跑、自动标题和撤回校验共用最近用户条目的有序查询。先按段用户数跳过无用户消息的尾部，再在命中段内查找用户条目和当前 revision。长输出尾部的探查按段数增长。 [最近用户条目](../crates/agena-storage-sqlite/src/kernel/reads.rs) [会话生命周期](../crates/agena-storage-sqlite/src/kernel/lifecycle.rs)
- `/parts?run_ids=...` 对每个去重后的 ID 分别读取有匹配条目的段，再合并最多 32 组有界子页。物理游标保留交错追加顺序，分支按冻结响应锚点过滤。显式 `ids` 在同一读事务批查至多 256 个指定条目。 [历史读取](../crates/agena-storage-sqlite/src/kernel/history_views.rs) [API 投影](../crates/agena-application/src/application_parts.rs)
- 提示词条目和 provider replay 的存储量由不同块的事实行及各轮的引用数决定。单轮仍要展开 P 个条目、S 个源、V 个 replay，再读取相应正文。各轮重复用的源头继续随所有轮次的 S 总和增长。共享源头需要把 epoch、可见边界、状态与丢失计数纳入相等条件。 [冻结输入](../crates/agena-storage-sqlite/src/kernel/prompt_inputs.rs) [提示词读写](../crates/agena-storage-sqlite/src/kernel/prompt.rs)
- 工作区 overview 的待关注列表直接在工作区范围取候选。精确统计点查物化计数，工作区列表只请求一遍统计。应用工作区、权限、模型目录和 Bot 查询复用已有 WAL 读池，事务内的写入校验与回读保持在同一个写事务。 [概览和精确统计](../crates/agena-storage-sqlite/src/kernel/summaries.rs) [仓储组合](../crates/agena-storage-sqlite/src/kernel.rs)
- 工具详情读继承源时用一条联查同时取得可见身份和冻结 epoch，正文继续按源分开读取。归档从当前条目的活动候选定位冻结记录。遇到自有条目时只查一次冻结条目资格，省去源、轮次和活动三次附加查询。 [工具详情](../crates/agena-storage-sqlite/src/kernel/reads.rs) [归档条目读取](../crates/agena-storage-sqlite/src/kernel/archive.rs)
- 频道对账按持久化待发布结果分页。通知路径直接读取缺少回执的待处理交互 ID。取执行回复时按 execution 和 generation 过滤，再检查分支成员资格。新版最多考虑 4,096 个匹配文本源，传输仍使用既有的正文读取上限。 [频道对账](../crates/agena-storage-sqlite/src/integration_repository.rs) [执行文本读取](../crates/agena-storage-sqlite/src/kernel/history_views.rs)
- 内容丢失页从主键按区间读出请求的窗口。归档和继承分支共用受提交位置限制的读取，尾页成本由该边界内的窗口数决定。 [捕获丢失读取](../crates/agena-storage-sqlite/src/kernel/content/loss.rs)
- 工作区和权限管理页的任意子串过滤仍随候选范围增长，使用 OFFSET 的页面还要越过被跳过的候选。 [工作区](../crates/agena-storage-sqlite/src/workspace_repository.rs) [权限](../crates/agena-storage-sqlite/src/permission_rule_repository.rs)
- 幂等回执、已处理投递、权限审计和软删除历史的空间持续增长。清理这些行之前需要区分审计证据、重复请求原始结果和仍可达的引用。调度历史、blob、摘除的历史段和快照有各自的分批清理。 [提交回执](../crates/agena-storage-sqlite/src/kernel/receipts.rs) [附件 GC](../crates/agena-storage-sqlite/src/kernel/blobs.rs) [快照 GC](../crates/agena-storage-sqlite/src/kernel/snapshots.rs) [调度历史](../crates/agena-scheduler/src/store.rs)

## 本轮基准（.8 至 .9）

用两版实际初始化的 schema，在同一组 SQLite 3.45.1 合成数据上核对结果和查询计划。页大小为 4 KiB。

| 场景 | .8 | .9 |
| --- | ---: | ---: |
| 空库 | 255 页 | 253 页 |
| 全局加 100 个工作区，各 365 天的提醒预算及索引 | 456 页 | 326 页 |
| 同一数据查询全局和单工作区的当日配额及最近联系 | 36 / 43 VM 步 | 40 / 46 VM 步 |
| 4,000 条编码后为 947 B 的全局配置回执 | 1,006 页 | 1,005 页 |
| 同一回执按作用域和请求键重放 | 14 VM 步 | 13 VM 步 |
| 模型目录状态点查和刷新 SQL | 12 / 51 VM 步 | 10 / 33 VM 步 |

预算只访问目标作用域的索引范围。全局当日计数和最近联系各有一棵小型部分索引，工作区分别使用主键和最近联系索引。目录元数据和模型定义在同一事务中更新，缓存清理使用读到的时间校验当前快照。 [提醒预算](../crates/agena-storage-sqlite/src/companion_repository.rs) [模型目录事务](../crates/agena-storage-sqlite/src/model_catalog_repository.rs)

### .9 验证记录

- storage、scheduler、runtime session、API server 和 Assistant 共 429 项通过，另有 1 项子进程夹具按原标记跳过。新增和扩展的回归覆盖同请求键的作用域隔离、重启后重放原偏好、两地独立跨午夜和目录失效后实例身份可用。
- Assistant 的四项 HTTP 系统场景通过，覆盖偏好持久化与工作区隔离、提醒到期、重启与取消、接口权限。全 workspace 目标编译、格式检查和差异检查通过。编译报告一条既有的 TUI 未使用导入告警。
- `.9` 的实际初始化与重开通过，`integrity_check` 返回 `ok`，`foreign_key_check` 无违规行。`.8` 的测试副本被当前基线拒绝，主库字节保持原样。基准脚本、逐表清单和测试日志位于 `target/sqlite-v9-audit/`。

## 历史基准（.7 至 .8）

查询对照使用 SQLite 3.45.1、4 KiB 页和同样的合成数据。VM 步数衡量 SQLite 执行工作量。长回复一组列出无 `ANALYZE` 的结果，带统计的查询计划也验证了有界读取。

| 场景 | .7 | .8 |
| --- | ---: | ---: |
| 99,999 个回复条目，预取尾页 129 条 | 7,301,394 步 | 7,658 步 |
| 同一回复取深游标页 | 4,901,237 步 | 7,494 步 |
| 同一回复精确计数 | 4,900,112 步 | 22 步 |
| 一个工作区有 10,000 个会话，精确统计 | 376,760 步 | 32 步 |
| 2,048 个空闲频道映射，读待对账会话 | 40,993 步 | 23 步 |
| 12,800 个活跃入箱事件，统计当前 epoch | 64,415 步 | 540 步 |
| 10,000 个活跃提醒中有 32 个可派发 | 50,108 步 | 1,929 步 |
| 50,000 个受保护 blob 后面有一个孤儿，累计退役操作 | 1,718,909 步 | 140 步 |
| 20,000 个 96 B 正文块及检查点 | 1,485 页 | 924 页 |
| 8 个嵌套分支，每个继承 10,000 个相同冻结头 | 1,688 页 | 1,004 页 |
| 10 万轮次样本的会话索引 | 396 页 | 256 页 |
| 领取一条 24 KiB 出箱消息 | 16 个 WAL 帧 | 3 个 WAL 帧 |

长回复样本的段计数表与索引共占 7 页。嵌套头部每 64 项读取由 977 增至 1,378 步。8 个独立冻结且全部不同的头部占用由 1,688 增至 1,715 页。共享用于嵌套分支的内容源，冻结条目、轮次、活动和模型轮次的头部继续各自持久化。 [回复计数](../crates/agena-storage-sqlite/src/kernel/history_runs.rs) [共享冻结源](../crates/agena-storage-sqlite/src/kernel/branches.rs)

单个 16 MiB 正文块按 64 KiB 分片读取并校验 SHA，合成耗时从 2.54–2.68 秒降到 50.95–57.02 毫秒，样本库增加约 0.8%。计时包含 SQLite 读事务和哈希，完整导出还涉及鉴权、JSON 编码和 sink 写入。普通事件读取和导入校验仍重建单块。 [分片读取](../crates/agena-storage-sqlite/src/kernel/archive.rs) [事件读取](../crates/agena-storage-sqlite/src/kernel/content/events.rs)

10,000 个提醒全部可派发时为 268 → 300 步。9,000 个冷却项后面有 1,000 个可派发项时为 45,268 → 47,321 步，有效截止索引占 142 页。相同范围中没有可派发提醒时为 50,013 → 347 步。 [提醒索引和读取](../crates/agena-storage-sqlite/src/companion_repository.rs)

### .8 验证记录

- storage、scheduler、runtime session 和 API server 共 402 项通过。另有 1 项子进程夹具按原标记跳过。对话、rewind 和 Telegram 的 10 项 HTTP 系统场景均通过。Web 的 1,038 项测试与 TypeScript 类型检查通过。
- 全 workspace 目标编译、格式检查和差异检查通过。编译报告 TUI 的一条既有未使用导入告警。 [告警位置](../crates/agena-tui-app/src/app_permission_helpers/layers.rs)
- 两版实际初始化库的 `integrity_check` 均返回 `ok`，`foreign_key_check` 均无违规行。当前测试卷的异常恢复使用文件复制路径。基准脚本、逐表清单和测试日志位于 `target/sqlite-v8-audit/`。

## 历史基准（2026-11-22.6 至 2026-11-22.7）

从两版实际初始化库加载同一组合成数据。SQLite 3.45.1，页大小为 4 KiB。页数包含对应表和索引，步数为对应 SQL 的 VM 指令数。

| 各插入 100,000 行 | `.6` 页数 | `.7` 页数 | `.6` 插入步数 | `.7` 插入步数 |
| --- | ---: | ---: | ---: | ---: |
| `agena_segment_entries` | 1,053 | 702 | 10,300,029 | 9,800,028 |
| `agena_prompt_input_entries` | 1,447 | 1,095 | 12,000,031 | 11,500,030 |
| `agena_round_input_blocks` | 1,059 | 702 | 7,200,031 | 6,700,030 |

三张表各减少一棵隐式唯一 B-tree。段成员反查从 17 降到 16 步，顺序读取 128 个成员从 800 降到 795 步。展开 1,280 个冻结输入从 10,358 降到 10,356 步。三个重复写入与两个错误的源归属都被拒绝，有效源记录可插入。空库由 246 页减至 241 页，其中一页原用于保存快照回收游标。 [表和索引声明](../crates/agena-storage-sqlite/src/kernel/schema.rs)

4,096 个段位置各有 64 个退役区间时，移除重复排序索引使对应表和索引从 4,494 页降至 3,377 页。读取 64 条快照边，在最旧、中间和最新代次各少 17 步，返回相同的边界。另在 SQLite 3.50.4 的 32 页冷缓存中读取最新代次的退役边，两种索引布局都产生 4,372 次 SQLite 页缓存未命中。不同代次的历史边较多时，页读取量仍取决于实际候选区间数。 [快照段页](../crates/agena-storage-sqlite/src/kernel/snapshots.rs)

| 场景 | `.6` | `.7` |
| --- | ---: | ---: |
| 回收 64 条退役边和 8,192 个段成员 | 131 条 SQL，539,191 步 | 3 条 SQL，506,955 步 |
| 64 条退役边全部被快照固定 | 67 条 SQL，1,845 步 | 2 条 SQL，1,850 步 |
| 100,000 个捕获丢失区间，校验深页游标 | 360,020 步 | 34 步 |
| 读冻结头之前最后 2 个丢失区间 | 399,916 步 | 54 步 |
| 当前头有足够区间，按上限返回 257 项 | 2,842 步 | 2,845 步 |
| 发布 128 轮输入和响应，run 与分支条目表各已有 10,000 行 | 15,460,769 步 | 86,046 步 |
| 同样发布 128 轮，run 与分支条目表为空 | 100,769 步 | 86,046 步 |

回收样本计入级联删除段成员的成本。归档样本测量正文及内容流的插入语句，包含真实的执行、响应和内容循环引用。读丢失区间的样本包含源在 fork 后继续增长的长尾。 [快照回收](../crates/agena-storage-sqlite/src/kernel/snapshots.rs) [捕获丢失范围](../crates/agena-storage-sqlite/src/kernel/content/loss.rs) [归档发布](../crates/agena-storage-sqlite/src/kernel/import.rs)

### 内容存储的空间试验

内容流已有整型 `source_key`。在 20,000 个块、20,000 个检查点的原型里，使用整型外键替代重复 UUID 后，96 字节块的表及索引从 1,567 页降到 1,036 页，256 字节块从 2,437 页降到 1,951 页，4 KiB 块从 23,378 页降到 22,998 页。顺序读取 8 块时，两种直接键均为 71 步。查询内先解析 UUID 时为 83 步。收益集中在短块、检查点和窄元数据。落地涉及内容提交、保留清理、读取与归档，应复用已校验的源身份。 [当前内容表声明](../crates/agena-storage-sqlite/src/kernel/schema.rs) [内容保留](../crates/agena-storage-sqlite/src/kernel/content/retention.rs)

### 2026-11-22.7 回归记录

- storage、scheduler、runtime session 和 API server 共 394 项 Rust 测试通过，1 项夹具按标记跳过。覆盖释放快照后回查已扫描边、多个退役边指向同一段、分支持有段、冻结丢失页的游标合法性，以及归档提交失败后重新发布。 [回收回归](../crates/agena-storage-sqlite/src/kernel/snapshots.rs) [冻结内容回归](../crates/agena-storage-sqlite/tests/branch_content.rs) [归档回归](../crates/agena-storage-sqlite/tests/session_branches.rs)
- 对话、rewind 和 Telegram 的 10 项 HTTP 场景通过。Workspace 全目标编译、格式及差异检查通过。`agena-tui-app` 有一条既有的未使用导入告警。 [告警位置](../crates/agena-tui-app/src/app_permission_helpers/layers.rs)
- 两版初始化库通过 `integrity_check` 和 `foreign_key_check`。表、索引、外键、读写入口和测量脚本及日志位于 `target/sqlite-index-gc-audit/`。

## 历史基准（2026-11-22.5 至 2026-11-22.6）

对照使用两版实际初始化库，在 SQLite 3.45.1 上加载相同的合成数据。历史样本有 100,000 条用户和 notice 记录，分为 782 段。表中的步数是 SQLite VM 指令数，每页为 4 KiB。

| 历史操作 | `.5` | `.6` |
| --- | ---: | ---: |
| 截断后保留 99,999 条，重算统计 | 956,813 步 | 8,661 步 |
| 同一截断，修改段成员范围并重算统计 | 956,937 步 | 9,091 步 |
| 截断后保留 50,000 条，修改段成员范围并重算统计 | 489,264 步 | 14,218 步 |
| 越过 10,000 条非用户条目，定位最近用户消息 | 60,731 步 | 1,022 步 |
| 最近用户消息位于尾部第一条 | 54 步 | 75 步 |
| 向私有尾段追加一条用户消息，含 run 和成员维护 | 569 步 | 575 步 |
| 向私有尾段追加首条文本响应，含 run 和成员维护 | 712 步 | 678 步 |
| 会话段边及其反向索引 | 8 页 | 10 页 |

截断的测量覆盖边界写入和保留前缀统计。快照退役、run 后缀裁剪和活跃执行结算各自独立。分支创建还需冻结每个可见条目的事实。段内的最大创建时间覆盖可见区间内的所有条目，导入时的初始 `last_message_at_ms` 取可见用户消息的最大时间。单条追加路径按条目主键更新统计并设置首次响应锚点，移除了此前无调用者的批量追加分支。 [会话段定义](../crates/agena-storage-sqlite/src/kernel/schema.rs) [截断与统计](../crates/agena-storage-sqlite/src/kernel/lifecycle.rs) [最近用户消息](../crates/agena-storage-sqlite/src/kernel/reads.rs) [追加与 run](../crates/agena-storage-sqlite/src/kernel/history_runs.rs)

| 归档操作 | `.5` | `.6` |
| --- | ---: | ---: |
| 导入 100,000 个成员，构建段关系及发布统计 | 9,276,060 步 | 7,533,575 步 |
| 同一导入，发布统计 SQL | 1,500,114 步 | 6,370 步 |
| 导出 200 项 replay，读实例页及来源 | 201 条 SQL，4,867 步 | 1 条 SQL，4,267 步 |
| 同一 replay 页，在私有图登记依赖 | 200 条 SQL，6,188 步 | 1 条 SQL，5,206 步 |
| 导出 64 个源，取头部并校验边界 | 65 条 SQL，4,449 步 | 1 条 SQL，2,633 步 |

导入按暂存成员的顺序建段，段边按主键顺序聚合，查询计划省去了 `DISTINCT`、`ORDER BY` 和 `GROUP BY` 的临时 B-tree。导出对照核对了字段、顺序和依赖集合，源样本含 12 个由分支固定的旧 epoch。实例页和私有图各自统计 VM 步数。主库发布的身份重映射、导出的字节预算预检和 JSONL 写出分别执行。 [导入暂存与发布](../crates/agena-storage-sqlite/src/kernel/import.rs) [归档依赖登记](../crates/agena-storage-sqlite/src/kernel/archive.rs) [源头读取](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs)

### 2026-11-22.6 回归记录

- storage、scheduler、runtime session 和 API server 共 393 项 Rust 测试通过，1 项夹具按测试标记跳过。新用例覆盖混合类型、时间与 ordinal 顺序不同的归档、部分段裁剪和私有或共享尾段重新追加。跨页 replay、源边界和旧读快照也通过。 [历史与导入回归](../crates/agena-storage-sqlite/tests/session_branches.rs) [快照及分页回归](../crates/agena-storage-sqlite/tests/branch_content.rs)
- 对话、rewind 和 Telegram 的 10 项 HTTP 系统场景通过。workspace 全目标编译、格式和差异检查通过。`agena-tui-app` 有一条既有的未使用导入告警。 [告警位置](../crates/agena-tui-app/src/app_permission_helpers/layers.rs)
- 两版初始化库均通过 `integrity_check` 和 `foreign_key_check`。逐表清单核对了 90 组外键及其反查计划。脚本、清单和测试日志位于 `target/sqlite-prefix-audit/`。

## 历史基准（2026-11-22.4 至 2026-11-22.5）

两版都来自实际初始化库，在 SQLite 3.45.1 上运行合成数据并核对输出。页数包含相关 B-tree 的表和索引，每页 4 KiB。VM 步数描述对应 SQLite 工作量。

| 100 轮回放存储 | 2026-11-22.4 | 2026-11-22.5 |
| --- | ---: | ---: |
| 每轮 1,000 项，复用同一窗口 | 784 页 | 23 页 |
| 每轮 1,000 项，窗口滑动一条 | 784 页 | 86 页 |
| 每轮 1,000 项，互不重叠 | 839 页 | 946 页 |
| 每轮复用同一组 1,000 个稀疏 ID | 815 页 | 687 页 |
| 每轮只有一个互异的 replay | 2 页 | 8 页 |

捕获 1,000 个 replay，首次建块从 75,113 增至 117,605 个 VM 步，下一轮滑动一次从 75,099 降至 69,111 步。滑动轮的预算预检从 19,823 降至 145 步，完整回放查询从 11,812 增至 15,691 步。两项合计从 31,635 降至 15,836 步。取深游标后的 200 项从 1,612 增至 1,647 步，计划继续用两层主键范围。块布局的收益取决于窗口重叠和 ID 密度。 [回放写入及预算](../crates/agena-storage-sqlite/src/kernel/prompt_replays.rs)

| 100,000 条历史的后缀裁剪 | 2026-11-22.4 | 2026-11-22.5 |
| --- | ---: | ---: |
| 移除最后一条，成员关系和 run 裁剪 | 3,250,177 步 | 377 步 |
| 移除末尾 128 条，成员关系和 run 裁剪 | 3,252,283 步 | 7,510 步 |
| 移除末尾 50,000 条，成员关系和 run 裁剪 | 3,031,279 步 | 2,115,394 步 |

后缀第一条是用户输入。样本包含该条及随后物理排序的历史，删除 run 键时仍按原消息归属定位，SQLite 的临时键列表随删除候选数增长。退役快照和排队执行结算为独立工作。`.5` 保留 99,999 条时，重算保留历史的条目数、用户消息数和时间仍约 962,000 步。 [run 裁剪](../crates/agena-storage-sqlite/src/kernel/history_runs.rs) [截断事务](../crates/agena-storage-sqlite/src/kernel/lifecycle.rs)

| 100 个工作区各 1,000 个会话 | 2026-11-22.4 | 2026-11-22.5 |
| --- | ---: | ---: |
| 全部 ready，完整 overview SQL | 2,182 步 | 2,105 步 |
| 全局 5,000 条待关注，目标工作区取 50 条 | 30,539 步 | 260 步 |
| 同一分布，完整 overview SQL | 35,987 步 | 5,710 步 |
| 同一分布，会话统计及索引 | 923 页 | 981 页 |

工作区键和部分索引给样本增加 58 页。新索引只维护本地可见的非 ready 会话，配置或作用域改变时通过触发器同步工作区键。常规列表和实例级状态统计继续读全局索引。 [统计触发器与索引](../crates/agena-storage-sqlite/src/kernel/schema.rs) [overview SQL](../crates/agena-storage-sqlite/src/kernel/summaries.rs)

### 源头共享试验

- 对 100 轮各 1,000 个源头做了引用共享试验，保留 `(source_key,head_id)` 的归属外键和反查索引。源头全部稳定时，关联表及索引从 2,609 页降到 1,200 页。每轮 10% 的源头推进时为 1,595 页，全部推进时增至 5,224 页。该试验测量空间和逐轮展开结果，实际设计还要计入摘要写入以及导出预检读取每页头部尺寸的成本。 [轮次源定义](../crates/agena-storage-sqlite/src/kernel/schema.rs) [源头导出](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs)

### 2026-11-22.5 回归记录

- storage、scheduler、runtime session 和 API server 串行复跑共 392 项通过，1 项子进程夹具按测试标记为 ignored。覆盖 replay 页及导入重分块、跨工作区和可见性更新、交错响应的 rewind 与原快照保留。首次并行运行有一项压缩预算测试收到容量拒绝，该项在串行复跑中通过。 [回放及 rewind 回归](../crates/agena-storage-sqlite/tests/branch_content.rs) [工作区范围回归](../crates/agena-storage-sqlite/src/kernel/schema.rs)
- 测试发现多行触发器的名称在重开时未正确切分。DDL 校验现按 ASCII 空白识别名称，数据库重开与 WAL 恢复用例通过。两份初始化库的 `integrity_check` 均返回 `ok`，`foreign_key_check` 无记录。 [schema 声明校验](../crates/agena-storage-sqlite/src/schema/validation.rs)
- 对话、rewind 和 Telegram 的 10 项 HTTP 系统场景通过。`cargo check --workspace --all-targets --offline --locked`、格式和差异检查通过。基准脚本、索引计划和测试日志位于 `target/sqlite-replay-audit/`。

## 历史基准（2026-11-22.3 至 2026-11-22.4）

两版都使用实际初始化的 schema，合成操作在 SQLite 3.45.1 上执行并逐项比对结果。页大小为 4 KiB。分支 B-tree 样本统计冻结条目表、冻结源表及其索引，查询 VM 步数只计对应 SQL。

| 场景 | 2026-11-22.3 | 2026-11-22.4 |
| --- | ---: | ---: |
| 8 个分支均继承 1 万条条目及 1 万个源 | 12,656 页 | 2,668 页 |
| 同样的 8 个分支，每分支只有 1,000 个源 | 2,126 页 | 1,110 页 |
| 归档含 1 万条冻结活动的分支，核对单条目的 1 个活动 | 70,026 步 | 38 步 |
| 同一分支，核对没有活动的条目 | 70,011 步 | 12 步 |
| 工具详情读取 128 个继承源的描述符与 epoch | 129 条 SQL，4,108 步 | 1 条 SQL，3,095 步 |
| 捕获 1,000 个继承源到轮次 | 193,168 步 | 155,153 步 |
| 发布 1 万个分支源，含重写和身份映射 | 50,002 条 SQL，1,490,028 步 | 1 条 SQL，1,010,033 步 |

密集样本的冻结源表和索引由 10,828 页减至 1,732 页，冻结条目表和索引由 1,828 页减至 936 页。首次捕获 1 万个源头从 980,062 增至 1,600,058 步，嵌套捕获从 790,036 增至 1,280,036 步。SQLite 要投影并检查各个冻结列。清空 WAL 后只提交源头捕获，首轮写入从 5,607,352 降至 910,552 字节，嵌套写入从 6,163,552 降至 1,034,152 字节。数字取自各自的合成事务，首轮 fork 还包括复制段、冻结条目和摘要更新。 [冻结列和读取](../crates/agena-storage-sqlite/src/kernel/content/heads.rs) [分支捕获](../crates/agena-storage-sqlite/src/kernel/branches.rs)

旧活动查询先从 `agena_branch_activities` 读取整个分支的候选。逐条目归档 H 个条目、分支共有 A 个活动时，这一步最多检查 O(H × A) 个候选。新查询从 `agena_background_activities(entry_id,id)` 定位当前条目的活动。1,000 条和 10,000 条冻结活动的实测查询步数一致。自有工具的源描述查询两版均为 2,447 步。 [归档查询](../crates/agena-storage-sqlite/src/kernel/archive.rs) [工具详情](../crates/agena-storage-sqlite/src/kernel/reads.rs)

### 余下增长点的试验

- `agena_round_input_replays` 的块共享样本用相同的 100 轮、每轮 1,000 个 replay。在同一窗口场景，表和索引由 802 页降至 25 页，逐条滑动由 803 页降至 87 页。各轮互不重叠时由 858 页增至 961 页。该试验验证了展开顺序和冻结状态相等。落地仍涉及归档分页、导入后重新分块和 provider state 的预检预算。 [回放读取](../crates/agena-storage-sqlite/src/kernel/prompt.rs)
- 10 万个会话中分布着 5,000 个待关注会话，overview 读取单一工作区的 50 项时，全局状态索引使这部分执行 30,594 步。将工作区、状态和会话 ID 放进试验投影后为 296 步。同一工作区的精确统计扫描其 1,000 个会话，约 39,045 步。维护这类投影的复杂度来自工作区迁出、删除、临时会话与 Bot 可见性，以及 attempt 和 interaction 状态变化。 [会话摘要查询](../crates/agena-storage-sqlite/src/kernel/summaries.rs)
- rewind 的 run 裁剪仍检查保留及移除的全部 run 行。可从待裁剪的段范围计算待删除 run 的完整主键；统计的 `last_message_at_ms` 还需要保留历史的最大创建时间。分支的条目级冻结关系及源头也继续随分支增长。 [run 边界](../crates/agena-storage-sqlite/src/kernel/history_runs.rs) [截断与统计](../crates/agena-storage-sqlite/src/kernel/lifecycle.rs)

### 2026-11-22.4 回归记录

- storage、scheduler、runtime session 和 API server 共 390 项通过，1 项子进程夹具按测试标记为 ignored。扩展的分支回归通过嵌套 fork、工具详情与 JSONL v1 往返验证迟到源仍不可见，原有源的 epoch、丢失计数和冻结活动状态可恢复。 [分支归档回归](../crates/agena-storage-sqlite/tests/branch_content.rs)
- 对话、rewind 和 Telegram 的 10 项 HTTP 系统场景通过。两份实际初始化库均通过 `integrity_check` 和 `foreign_key_check`。
- `cargo check --workspace --all-targets --offline --locked`、`cargo fmt --all -- --check` 和 `git diff --check HEAD` 通过。脚本、逐表清单和验证日志位于 `target/sqlite-branch-audit/`。

## 历史基准（2026-11-22.2 至 2026-11-22.3）

两份 schema 由 Agena 实际初始化，合成数据使用 SQLite 3.45.1。页数包含冻结条目的表和索引，每页 4 KiB。两边逐轮比对展开后的条目、顺序和冻结字段。

| 冻结输入分布 | 2026-11-22.2 | 2026-11-22.3 |
| --- | ---: | ---: |
| 100 轮各 1,000 个输入，使用同一窗口 | 1,534 页 | 38 页 |
| 同规模，窗口每轮滑动一条 | 1,535 页 | 147 页 |
| 同规模，各轮窗口无重叠 | 1,628 页 | 1,690 页 |
| 同一窗口，条目取自稀疏的全局 ID | 1,595 页 | 1,023 页 |
| 100 个互异的单条目轮次 | 4 页 | 9 页 |

滑动样本用 213 个块、8,032 条块内事实、1,650 条轮次块引用保存了原有的 100,000 项。首轮写入 1,000 个输入为 176,076 → 196,498 个 SQLite VM 步；下一轮滑动一次为 176,028 → 115,543 步。两版条目预算预检均约 58,000 步。捕获需要逐条校验历史成员和冻结字段，计算摘要和编码块还会消耗 Rust CPU 与内存。首轮建块、完全不重叠的窗口及很短的唯一窗口承担额外写入和空间开销。 [冻结输入](../crates/agena-storage-sqlite/src/kernel/prompt_inputs.rs)

| 同一轮导出 1,000 个源头 | 收集事务含最后一页 | 源清单 SQL 语句数 | 源清单 SQLite VM 步 |
| --- | ---: | ---: | ---: |
| 全是小头部 | 1,001 → 17 | 1,001 → 33 | 56,045 → 32,770 |
| 第一条头部超过页预算 | 1,001 → 18 | 1,001 → 35 | 56,045 → 33,193 |
| 中途有一条头部超过页预算 | 1,001 → 19 | 1,001 → 37 | 56,045 → 33,550 |

两阶段分页在同一读事务先确定范围，再重建选中的源头，最后一次空页只做范围探查。样本按持久化的头部字节数加每项 256 字节计页，校验两版输出完全相同，超预算头部始终独占一页。清单 SQL 的 VM 步数不含事务边界、快照权限校验和 JSONL 写入。源关联本身是独立成本，1,000 个小源的表和索引由 17 页增至 18 页。 [归档分页](../crates/agena-storage-sqlite/src/kernel/archive.rs) [源清单](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs)

### 队列索引试验

- 入箱按 epoch 计数的表达式部分索引，在 128 条待处理时从 1,055 降到 789 步。12,800 条跨 epoch 候选时从 89,759 降到 789 步，额外索引占 61 页。当前 worker 每批按序处理并标记旧 epoch，索引收益主要来自积压的旧 epoch 行。 [入箱计数和领取](../crates/agena-storage-sqlite/src/integration_repository.rs)
- 出箱按创建顺序加索引，在 30,000 条重试全部到期时从 241,869 降到 825 步。在同规模仅末尾 32 条到期时从 1,864 增到 120,696 步。生产使用原截止时间范围索引，两种试验索引均未引入。 [出箱领取](../crates/agena-storage-sqlite/src/integration_repository.rs)

### 2026-11-22.3 回归记录

- storage、scheduler、runtime session 和 API server 共 390 项通过，1 项标记为 ignored。新回归核对滑动窗口共享的三个内部块、跨 200 项导出页和 1,024 项导入页的条目顺序，以及 100 个源跨字节预算和项目数边界后的状态、错误信息及提交位置。 [块共享和源分页回归](../crates/agena-storage-sqlite/tests/branch_content.rs)
- 对话、rewind 和 Telegram 的 10 项 HTTP 系统场景通过。两份初始化库的 `integrity_check` 都返回 `ok`，`foreign_key_check` 均为 0 行。
- `cargo check --workspace --all-targets --offline --locked`、`cargo fmt --all -- --check` 和 `git diff --check HEAD` 通过。基准脚本、查询计划清单和回归日志位于 `target/sqlite-input-block-audit/`。

## 历史基准（2026-11-22.1 至 2026-11-22.2）

两版都从实际初始化的库提取 schema，数据为合成样本。SQLite 3.45.1 用于比较 VM 步数、结果与查询计划。pager 实验用 SQLite CLI 3.50.4，每组查询新开连接，先加载 schema，再记录目标语句的缓存缺页数。入箱和出箱用 `ANALYZE` 后的计划，原始日志也记录了无统计信息的计划。

| 场景 | 2026-11-22.1 | 2026-11-22.2 |
| --- | ---: | ---: |
| 100 轮各 1,000 个源，关联表及索引所占的 4 KiB 页 | 9,856 页 | 2,825 页 |
| 单轮 1,000 个源，仅取精确源预算 | 11,024 VM 步 | 10 VM 步 |
| 同一预算预检的 pager 缺页 | 23 | 2 |
| 预算预检后读取同一轮源头的累计 pager 缺页 | 136 | 64 |
| 128 条活跃入箱候选，按优先级取 32 项 | 3,065 VM 步 | 275 VM 步 |
| 12,800 条跨 epoch 的活跃入箱候选，按同一顺序取 32 项 | 283,067 VM 步 | 275 VM 步 |
| 10 万条零截止时间的出箱消息，取最早 32 项 | 800,720 VM 步 | 1,700 VM 步 |
| 同规模混合出箱消息，含 100 条已到期重试及 400 条未来重试 | 797,617 VM 步 | 2,986 VM 步 |
| 导入 3 万个轮次源，读取暂存列信息的开销 | 1,110,000 VM 步 | 5,259 VM 步 |

源关系的整型主键和二级索引减少了重复 UUID，样本中的内容流表及其 UUID 索引均为 55 页。预算读取复用已经定位到的轮次行，源数量增加时该部分的额外读取成本为 O(1)。捕获 1,000 个源并计算精确预算从 215,107 增至 217,173 步，增加的工作集中在写入轮次汇总。源详情经主键连接按整数键流式读取，在 Rust 按 UUID 排序以保持调用方可见顺序。 [源清单与汇总](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs) [提示词读取](../crates/agena-storage-sqlite/src/kernel/prompt.rs)

出箱查询通过原有索引等值范围读取零截止时间的候选。正、负非零截止时间分别从可到期范围内按原顺序取候选，再合并三批。重试候选全部到期的 10 万条样本为 800,945 → 801,867 步，领取仍受已到期的非零截止时间记录数限制。空队列为 40 → 108 步。入箱索引在写入活跃事件时固定优先级，按序读取一批后，再执行 epoch 和预算检查。 [队列索引与读取](../crates/agena-storage-sqlite/src/integration_repository.rs)

归档的列信息对照只测 schema 查询。原来每次插入事实都读一次 `PRAGMA table_xinfo`，新版将暂存库的可写列一次读取，在内存复用于事实绑定及发布。JSONL v1 使用公开 UUID 和原有字段，源内部键与预算在发布事务里重建。 [归档导入](../crates/agena-storage-sqlite/src/kernel/import.rs) [归档分页](../crates/agena-storage-sqlite/src/kernel/archive.rs)

### 按相对位置分块试验

用 100 轮、每轮 1,000 条冻结条目比较当时的逐轮表及其索引与按相对位置每 64 条一块的试验布局。完整冻结字段参与去重，比较了每轮展开后的字段和顺序。三组试验各产生 1,600 条轮次到块的关系。

| 窗口分布 | 逐轮表页数 | 试验页数 | 不同块数 |
| --- | ---: | ---: | ---: |
| 100 轮使用相同窗口 | 1,534 | 22 | 16 |
| 每轮向前滑动一个条目 | 1,535 | 1,017 | 1,096 |
| 各轮输入无重叠 | 1,628 | 1,619 | 1,600 |

按块保存时，固定块长 B、U 个不同冻结块占 O(U × B) 个事实槽，各轮额外保存所需的块引用。滑动窗口使按相对位置分块的块对齐发生变化。试验页数只计冻结条目与块关系，脚本和查询计划在 `target/sqlite-manifest-audit/`。

### 2026-11-22.2 回归记录

- storage、scheduler、runtime session 和 API server 共 388 项通过，1 项标记为 ignored。归档回归核对重新映射的源、冻结边界和存储的预算。出箱回归覆盖负时钟、未来重试和相同创建时间的跨范围排序。 [归档回归](../crates/agena-storage-sqlite/tests/session_branches.rs) [出箱回归](../crates/agena-storage-sqlite/tests/integrations.rs)
- 对话、rewind 和 Telegram 的 10 项 HTTP 系统场景通过。两份初始化库的 `integrity_check` 都返回 `ok`，`foreign_key_check` 均为 0 行。
- `cargo check --workspace --all-targets --offline --locked`、`cargo fmt --all -- --check` 和 `git diff --check HEAD` 通过。

## 历史基准（2026-11-22 至 .1）

合成数据使用两份由 Agena 实际初始化的 schema，在 SQLite 3.45.1 上核对查询结果、执行计划和 B-tree 页数。历史追加实验预置正文与执行事实，测量段成员、会话边和水位的写入。目录 WAL 实验在每次刷新前清空已提交的 WAL，使用约 1 KiB 的定义。

| 场景 | 2026-11-22 | 2026-11-22.1 |
| --- | ---: | ---: |
| 向含 96 个成员的私有尾段追加一个成员 | 14,886 VM 步 | 387 VM 步 |
| 8,193 次单条追加，相关表累计变更行数 | 1,089,412 | 32,772 |
| 同一历史累计分配的段 ID | 8,193 | 65 |
| 清空同一历史并运行 GC 后，仍占空间的已摘除成员行 | 8,193 | 0 |
| 1 万条模型定义中各增、删、改一条，目录数据行修改次数 | 20,000 | 3 |
| 同一目录更改的 WAL 字节数 | 47,095,752 | 24,752 |
| 单会话 10 万个内容流，查 16 个 open 流，有统计信息 | 500,045 VM 步 | 96 VM 步 |
| 同一数据的会话范围索引，占用 4 KiB 页 | 1,312 | 1 |
| 1 万个待处理投递中 16 个有目标，目标 pending 索引 | 23 页 | 1 页 |
| 校验归档暂存中 3 万个轮次源的持久边界 | 2,429,922 VM 步 | 1,440,024 VM 步 |

私有尾段有 L 个成员，追加 K 个成员的常规行写入从随 L 加 K 增长变成随 K 增长，外加两次引用探查和固定的边与水位写入。分支或导出固定旧段时走可见范围复制。clear 和截断将待回收边放入退役表，独立于内容检查点的 500 ms 定时器驱动分批回收。样本的两个回收批次分别处理 64 和 1 条边，8,193 条原始记录存于事实表。 [历史追加](../crates/agena-storage-sqlite/src/kernel/commands.rs) [退役和回收](../crates/agena-storage-sqlite/src/kernel/snapshots.rs)

两种内容查询计划在无统计信息时分别用 500,047 和 96 步。待处理投递的精确目标探查两版均为 21 步。目录扫描和比较成本随目录规模增加，实际写入量由变更集决定。归档样本的轮次源校验从 90,001 条 SQL 语句合并为一次有序流式查询。分支源单独核验，暂存连接将查询预读限制为一行，避免放大大记录的内存占用。 [模型目录](../crates/agena-storage-sqlite/src/model_catalog_repository.rs) [归档暂存](../crates/agena-storage-sqlite/src/kernel/import.rs)

### 进一步优化的实测边界

- 5 万个工作区取相同的第 40,001 至 40,050 项，OFFSET 用 881,421 VM 步，现有时间和 ID 游标用 1,590 步。搜索一个没有匹配项的子串用 250,023 步，成本来自路径过滤。 [工作区仓储](../crates/agena-storage-sqlite/src/workspace_repository.rs) [分页入口](../crates/agena-application/src/service/workspaces.rs)
- 保持提醒的到期和 ID 顺序，试验有效截止时间索引。1 万条中仅 32 条可执行时，50,113 → 433 步。全部到期时，268 → 90,141 步，原因是需要扫描并排序候选才能保持原派发顺序。 [提醒领取](../crates/agena-storage-sqlite/src/companion_repository.rs)
- 10 万条轮次源的尺寸覆盖索引占 1,301 页，主表占 3,253 页。预算聚合用覆盖索引为 11,024 步，从主键范围读为 11,026 步。主键方案节约索引空间与写入维护，需要检验只预检和冷缓存条件下的页面读取。长期空间上限仍由各轮输入、源和回放关系数决定。块级共享需要以完整冻结事实作为相等条件，并能按轮恢复清单顺序。 [输入预算](../crates/agena-storage-sqlite/src/kernel/prompt.rs) [源清单](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs)

### 2026-11-22.1 回归记录

- storage、scheduler、runtime session 和 API server 共 387 项通过，1 项标记为 ignored。旧快照在追加与清空后仍返回原边界，清空回收段成员后可核对原始事实。同一目录只改动变更的条目，导入后的分支轮次恢复正确的源边界。 [历史尾段回归](../crates/agena-storage-sqlite/tests/branch_content.rs) [清空回归](../crates/agena-storage-sqlite/tests/session_clear.rs) [归档回归](../crates/agena-storage-sqlite/tests/session_branches.rs)
- 正式 Agena 的对话、rewind 和 Telegram 共 10 项 HTTP 系统场景通过。两份初始化库的 `integrity_check` 都返回 `ok`，`foreign_key_check` 均为 0 行。
- `cargo check --workspace --all-targets --offline --locked`、`cargo fmt --all -- --check` 和 `git diff --check HEAD` 通过。基准脚本和日志在 `target/sqlite-growth-audit/`。

## 历史基准（.4 至 2026-11-22）

使用两份由 Agena 初始化的 schema，在 SQLite 3.46.0 上运行合成数据与仓储查询。比较了结果与 `EXPLAIN QUERY PLAN`。以下 VM 步数对应单个 SQL 路径。

| 场景 | .4 | 2026-11-22 |
| --- | ---: | ---: |
| 1 万个终态执行，取 16 个待发布结果 | 300,231 步 | 208 步 |
| 10 万个终态执行，取 16 个待发布结果 | 3,000,231 步 | 208 步 |
| 10 万个执行的结果均已入箱，空闲轮询 | 3,000,055 步 | 34 步 |
| 3 万条历史，校验并冻结 1,000 条输入 | 274,328 步 | 183,076 步 |
| 1,000 条输入、1,000 个回放和 1,000 个源，预算检查返回行数 | 3,001 行 | 2 行 |
| 100 轮各保留 1,000 个源，源表及三个索引的 4 KiB 页数 | 14,954 页 | 6,930 页 |

在 1 万条历史里查第一轮回复，旧窗口读完 4,096 条较新的无关记录后未找到目标。索引查询用 119 步返回该可见文本源。回复入箱与移除待发布行在同一事务，retry 后迟到的旧代次结果无法写入新回执。 [执行文本读取](../crates/agena-storage-sqlite/src/kernel/history_views.rs) [频道回执](../crates/agena-storage-sqlite/src/integration_repository.rs)

冻结 1,000 个源的单条写入由约 11.2 万步增至 20.2 万步。单次聚合预算检查由 76,072 步增至 88,120 步，返回相同总预算 1,342,633 字节。开始轮次只核算一次输入及回放预算，再单独验收源预算。读取端逐行解码正文和源边界，源表本身在这份样本中从 10,964 页降至 2,940 页。 [预算与提示词读取](../crates/agena-storage-sqlite/src/kernel/prompt.rs) [源捕获](../crates/agena-storage-sqlite/src/kernel/prompt_sources.rs)

两个初始化库的 `integrity_check` 均返回 `ok`，`foreign_key_check` 均无违规行。查询计划、合成数据脚本和验证日志在本 worktree 的 `target/sqlite-audit/`。

### 2026-11-22 回归记录

- storage、scheduler、runtime session、application 和 API server 共 462 项通过，1 项标记为 ignored，2 项失败。两个失败均发生于 `agena-application` 的 stdio bridge fixture 读取 `ready.json` 时，`.4` 对照也复现了这两个失败。
- 最终版 storage 和 scheduler 复跑 133 项通过，1 项标记为 ignored。覆盖频道待发布结果重启恢复、retry 后的迟到回执、分支文本可见性、输入写入回滚和归档恢复后的轮次源边界。 [频道回归](../crates/agena-storage-sqlite/tests/channel_access.rs) [分支回归](../crates/agena-storage-sqlite/tests/branch_content.rs) [归档回归](../crates/agena-storage-sqlite/tests/session_branches.rs)
- 正式 Agena HTTP 对话、rewind 和 Telegram 的 10 项系统场景通过。恢复夹具在同一事务内重建故障发生时尚未入箱的待发布项，验证 review 跳过旧结果并恢复本地和 Telegram 回复。 [恢复回归](../apps/agena/tests/scenarios/telegram/recovery.rs)
- `cargo check --workspace --all-targets --offline --locked`、`cargo fmt --all -- --check` 和 `git diff --check HEAD` 通过。

## 历史基准（.3 至 .4）

两份 schema 由 Agena 实际初始化，合成数据使用 SQLite 3.46.0 执行仓储中的查询。对比了查询结果和 `EXPLAIN QUERY PLAN`，并区分有无 `ANALYZE` 统计的场景。VM 步数是 SQLite 指令数。HTTP 请求还包含正文读取、应用投影和序列化。

| 场景 | .3 VM 步数 | .4 VM 步数 |
| --- | ---: | ---: |
| 10 万条历史，75% 前缀分支，`/runs` 取 8 条及游标，无统计 | 26,815 | 218 |
| 同一 `/runs` 查询，有统计 | 9,379 | 243 |
| 同一分支，`/parts?run_ids=...` 选择 2 个 run，返回 18 条，无统计 | 96,739 | 1,881 |
| 同一 `/parts` 查询，有统计 | 2,644 | 1,917 |
| 1 万个已结束执行和 2 个待处理执行，检查能否领取最早的队列项 | 120,080 | 82 |
| 20 个房间 × 5,000 个成员，读取房间列表，无统计 | 600,031 | 111 |
| 同一房间列表，有统计 | 232 | 115 |
| 10 万次工具调用，目标会话 500 次调用中有 50 次计费，查会话成本，无统计 | 8,726 | 5,870 |

| 4 KiB 页数或初次写入次数 | .3 | .4 |
| --- | ---: | ---: |
| 两代分支各继承 7.5 万条，冻结条目数据及反向索引 | 15,401 页 | 2,165 页 |
| 1 万条未关联 activity 的内容流，activity 反查索引 | 133 页 | 1 页 |
| 20 个房间 × 5,000 个成员，房间和成员 B-tree | 2,839 页 | 976 页 |
| 1 万个单成员房间，房间和成员 B-tree | 271 页 | 481 页 |
| 10 万条归档成员，导出私有表和待输出索引 | 2,978 页 | 842 页 |
| 同一归档初次登记成员 | 100,000 次插入 | 782 批插入 |

冻结条目压缩约 86% 的 B-tree 页。首次和嵌套 fork 的捕获仍遍历可见前缀。在同一内存样本上，首次捕获从约 1,587 万步增至 2,197 万步，嵌套从 895 万步增至 1,204 万步。每条记录的列写入与约束检查增加了 VM 指令，移除 JSON 序列化缩减了载荷。该次测量的执行耗时分别为 619 → 483 ms 与 424 → 417 ms。房间拆分增加单成员场景的存储成本，并在观察时多做一次房间身份写入检查。计费索引在近乎所有调用都计费的样本中占 318 页。 [分支捕获](../crates/agena-storage-sqlite/src/kernel/branches.rs) [频道成员](../crates/agena-storage-sqlite/src/integration_repository/channel_participants.rs) [用量索引](../crates/agena-storage-sqlite/src/kernel/usage.rs)

模型目录的相同快照刷新经真实仓储测试验证，只更新目录状态。归档回归同时验证快照分页、冻结边界和 JSONL 导入，导出登记阶段读取的正文计数为零。两份初始化库的 `integrity_check` 均返回 `ok`，`foreign_key_check` 无违规行。查询计划、合成数据脚本和日志在 `agena-sqlite-table-optimization` worktree 的 `target/table-optimization/`。 [模型目录测试](../crates/agena-storage-sqlite/src/model_catalog_repository.rs) [分支和归档测试](../crates/agena-storage-sqlite/tests/branch_content.rs)

### .4 回归记录

- storage、scheduler、runtime-session、application 和 API server 共 460 项通过、1 项 ignored、2 项失败。失败是既有的 stdio bridge 测试，均在等待 fixture 的 `ready.json` 时失败，`.3` 工作区记录了同样的错误。
- 10 项正式 HTTP 系统场景中 9 项通过，覆盖并发、流式取消、三种 rewind、Telegram 恢复和 Cron 重启后的投递。Cron 同截止时间删除场景达到 60 秒测试超时，单独复跑和 `.2` 对照成品均复现超时。
- `cargo check --workspace --all-targets --offline --locked`、`cargo fmt --all -- --check` 和 `git diff --check` 通过。

## 历史基准（.1 至 .2）

SQLite 3.46.0，固定生成的数据，两边使用实际初始化得到的 DDL。查询实验用 `ANALYZE` 更新统计信息并对比结果与 `EXPLAIN QUERY PLAN`。表中时间指标是 SQLite 虚拟机指令步数。它反映在 SQLite 内执行的工作量，HTTP 耗时还取决于磁盘、Rust 投影和序列化。以下为上一轮在合成数据上的记录。

| 场景 | 改动前 VM 步数 | 改动后 VM 步数 |
| --- | ---: | ---: |
| 1 万条历史，75% 前缀分支，按 run ID 取一致的 11 个条目 | 232,543 | 2,093 |
| 10 万条历史，同一选择 | 2,323,036 | 2,081 |
| 10 万条历史，分支内某个回复的精确条数 | 1,572,890 | 591 |
| 10 万条历史，分支统计重算 | 996,789 | 691,425 |
| 4,096 条目的普通轮次，检查是否有压缩 checkpoint | 24,590 | 23 |
| 10 万个会话，按代理状态选择 100 个待恢复会话 | 800,221 | 1,043 |
| 4 万条已消费投递，查某次续跑的投递来源 | 200,025 | 48 |
| 106,305 条规则，精确解析全局、工作区、会话三层 | 1,001,742 | 385 |
| 同一组规则，加载目标作用域的 339 条有效规则 | 1,144,528 | 11,001 |
| 同一组规则，按工作区及游标取 50 条最近记录 | 343,985 | 1,249 |
| 5 万条提醒，工作区和来源列表各取 100 项 | 118,811 / 28,852 | 617 / 635 |
| 同一组提醒，到期 worker 取 32 项 | 2,206 | 310 |
| 每作用域 3,650 天接触记录，工作区当日配额与最近联系 | 32,871 | 45 |
| 每作用域 3,650 天接触记录，全局当日配额与最近联系 | 29,218 | 36 |
| 12,000 条频道审计记录，读取最近 100 条 | 144,014 | 816 |
| 3 万条出站记录，取 32 条到期消息 | 11,709 | 10,933 |
| 暂存 1.5 万条历史、1,500 个冻结轮次，检查轮次依赖 | 约 67,495,500 | 31,521 |

同一份历史上，`/runs` 的逻辑边界查询用 2,337,965 步，已开始用户消息计数用 552,891 步。这两项随着分支可见历史增长，仍决定长会话的首次 `/runs` 读取成本。原有普通物理游标页已经按段和 ordinal 定位，10 万条历史的深游标样例取 65 条时为 2,172 → 2,104 步。

归档发布原本在主写事务内重新检查整个实例。实例内的历史条目从 1 万增至 10 万，`PRAGMA main.foreign_key_check` 从 483,480 增至 4,832,550 步。平时已启用外键约束，发布时把当前事务涉及的外键检查推迟到 COMMIT。实验用插入单个会话并提交来隔离这部分成本，两种已有数据量下均为 294 步。真正发布的耗时还包含包内的 JSON 引用改写和批量写入。真实归档回归覆盖了循环 FK、提交失败后的回滚与再次导入。 [发布事务](../crates/agena-storage-sqlite/src/kernel/import.rs) [发布回归](../crates/agena-storage-sqlite/tests/session_branches.rs)

| SQLite 页对照，页大小为 4 KiB | 改动前 | 改动后 |
| --- | ---: | ---: |
| 10 万条历史的 entry 正文预检索引 | 849 | 453 |
| 合成实例的代理状态索引，10 万个会话中 100 个有代理状态 | 347 | 1 |
| 合成实例的投递状态或续跑索引，4 万条消费记录中 1 条带续跑身份 | 610 | 1 |
| 3 万条出站记录的队列与恢复索引 | 193 | 81 |
| 暂存中的内核表与索引，1.5 万条历史 | 2,819 | 2,094 |
| 1 万条命令回执，单条结果 JSON 144 字节 | 840 | 593 |
| 1 万条命令回执，单条结果 JSON 1,024 字节 | 3,465 | 3,406 |

暂存所用的内核表从 40 张降到 29 张，显式索引从 81 个降到 7 个。导出工作图在一个私有事务里写入，并在条目输出后移出待处理索引。对含 1,024 字节结果的命令回执，试验性 `WITHOUT ROWID` 布局占用 11,408 页，因此保留 rowid。对 3,000 条包含 2 KiB 说明的工作区 Assistant 配置回执，摘要替换将实际 B-tree payload 从约 12.7 MB 降到 6.4 MB，该样本的分配页数仍为 3,418。页大小、溢出分配和载荷尺寸都会影响文件体积。

### 上一轮验证记录

- 以新数据目录启动成品，核对全部表名、索引、触发器与 schema 基线。SQLite `integrity_check` 返回 `ok`，`foreign_key_check` 无违规行。
- storage、scheduler、API server、application 和 runtime session 合计 460 项 Rust 测试通过，另有 1 项由测试标记为 ignored。权限回归检查三个作用域、撤销和同毫秒游标。历史回归检查分支成员、重复选择和前后页。归档回归检查延迟 FK 提交失败、原子回滚、相同 request ID 重试及已分配段 ID 高水位。 [权限回归](../crates/agena-storage-sqlite/tests/persisted_network_permission.rs) [历史回归](../crates/agena-storage-sqlite/tests/branch_content.rs) [归档回归](../crates/agena-storage-sqlite/tests/session_branches.rs)
- HTTP 系统场景覆盖对话、rewind、Assistant 提醒和 Telegram review。12 项在成组运行中通过，其中 fork rewind 使用实际 `/runs` 和筛选 `/parts` 验证继承与截断。并发对话在该组运行中一次遇到 kernel Busy，单独复跑通过。Python 的 7 个会话重启、流重连、分支和压缩场景通过。 [HTTP 回归](../apps/agena/tests/scenarios/rewind.rs) [生命周期测试](../scripts/agena/tests/test_server_lifecycle.py)
- 两个 stdio bridge 测试在改动前的构建也会失败，当前批次跳过了这两项。Assistant 自然记忆召回场景在本轮构建和原 `master` 的成品测试中均出现同样的偏好召回失败，系统批次跳过了该项。
- `cargo check --workspace --all-targets --offline --locked` 与 `git diff --check` 通过。本地查询计划、合成数据脚本、schema 快照与运行日志位于 `target/table-audit/`。
