cli-about = Agena 終端聊天應用

pane-sessions = 工作階段
pane-sessions-search = 工作階段 [{$query}]
pane-transcript = 對話記錄
pane-messages = 訊息
pane-composer = 輸入區 [{$session}]

session-meta = #{$id}  {$message_count} 則訊息  {$updated}
session-running = 執行中
sessions-empty = 找不到工作階段
sessions-loading-more = 正在載入更多工作階段...
sessions-more = 還有更多工作階段可載入
hub-title = 工作階段中心
hub-action-create = 新增工作階段
hub-action-list = 工作階段清單
hub-action-refresh = 重新整理
hub-hint-move = 移動
hub-hint-focus = 焦點
hub-hint-section = 分組
hub-hint-open = 開啟
hub-hint-back = 返回
hub-section-attention = 需要關注
hub-section-running = 執行中
hub-section-recent = 最近
hub-empty-attention = 沒有需要關注的工作階段
hub-empty-running = 沒有執行中的工作階段
hub-empty-recent = 沒有最近的工作階段
hub-section-new = 新增工作階段
hub-empty-new = 沒有可建立的工作階段
hub-item-new = + 新增工作階段
hub-item-new-detail = 按 Enter 建立新的工作階段
hub-action-search = 搜尋
hub-action-clear-search = 清除搜尋
hub-search-placeholder = 輸入以篩選工作階段…
hub-search-active-empty = 輸入以篩選…
hub-search-active = 篩選:{$query}
command-hub-summary = 開啟工作階段中心
command-background-summary = 返回工作階段中心;工作階段繼續執行
hub-empty = 還沒有工作階段，按 Ctrl+N 建立一個。
context-help-context-hub = 工作階段中心
context-help-summary-hub = 檢視需要關注、執行中和最近的工作階段，並建立新的工作階段。
context-help-key-create-session = 建立新的工作階段。
context-help-key-session-list = 開啟完整的工作階段清單。

transcript-header-lines = 行 {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = 搜尋={$query} ({$current}/{$total})
transcript-header-tail = 尾隨
transcript-header-loading = 載入中
transcript-header-loading-older = 正在載入更早訊息
transcript-header-busy = 忙碌
transcript-loading-older = 正在載入更早的訊息...
transcript-more-older = 還有更早的訊息。向上捲動或按 PageUp 繼續載入。
transcript-empty-session = 目前工作階段還沒有訊息。

session-state-creating = 正在建立
session-state-ready = 最近結束
session-state-running = 正在執行
session-state-awaiting-interaction = 等待你處理
session-state-interrupted = 已中斷
session-state-failed = 已失敗

no-session-selected = 尚未選擇工作階段。
no-session-selected-hint = 使用 /sessions 選擇工作階段，或直接在輸入區開始輸入以建立新工作階段。
composer-session-new = 新工作階段
composer-placeholder = 輸入給 Agena。游標在開頭時按上鍵查看歷史。/ 指令。Ctrl+O 附件。

status-global = / 向下搜尋 | ? 向上搜尋 | Ctrl+C 連按兩次離開
status-sessions = 工作階段：/sessions
status-transcript = 查看：i 進入插入 | j/k 捲動 | / 搜尋 | c 複製上一則 | y 複製
status-composer = 插入：Esc 返回查看 | Ctrl+Enter 立即送出 | Ctrl+J 換行 | 開頭按上鍵查看歷史 | / 指令 | Ctrl+G 項目 | Ctrl+R 輸入 | Ctrl+L 權限

help-title = 說明
help-header = Agena TUI
help-section-sessions = 工作階段切換器
help-sessions-line-1 = /sessions 開啟可搜尋的工作階段切換器
help-sessions-line-2 = Up/Down、PageUp/PageDown 移動選取
help-sessions-line-3 = Enter 開啟所選工作階段
help-section-transcript = 對話記錄面板
help-transcript-line-1 = i 進入插入模式；j/k 或方向鍵捲動
help-transcript-line-2 = Space / Shift+Space / Ctrl+B 翻頁
help-transcript-line-3 = Ctrl+D / Ctrl+U 半頁捲動
help-transcript-line-4 = 在頂部附近按 PageUp 會載入更早訊息
help-transcript-line-5 = g/G 跳到頂部或底部
help-transcript-line-6 = / 向下搜尋，? 向上搜尋；n 沿目前方向繼續，N 反向跳轉
help-transcript-line-7 = c 複製最後一則 assistant 訊息，y 複製已載入全文，Y 複製目前可見視窗
help-section-composer = 輸入區
help-composer-line-1 = Esc 返回查看模式；Enter 送出
help-composer-line-2 = Shift+Enter 或 Ctrl+J 插入換行
help-composer-line-3 = Ctrl+A/E/B/F/P/N 移動，Ctrl+Left/Right 以詞跳轉
help-composer-line-4 = Ctrl+H/D/W/U/K/Y 依 shell 或編輯器習慣編輯
help-composer-line-5 = 在行邊界處，Ctrl+A/E 可繼續跨到上一行或下一行
help-composer-line-6 = Ctrl+O 搜尋工作區檔案並附加
help-composer-line-7 = Ctrl+E 用 $VISUAL/$EDITOR 開啟外部編輯器
help-composer-line-8 = Ctrl+T 附加剪貼簿圖片
help-composer-line-9 = 貼上的文字會直接插入輸入區；貼上單一檔案路徑會直接附加，附件保持原子化
help-composer-line-10 = 游標位於輸入框開頭時按上鍵開啟歷史；Ctrl+P 編輯待發訊息、Ctrl+X 取消待發訊息
help-section-actions = 操作
help-actions-line-1 = Ctrl+N 建立工作階段；n/N 跳轉搜尋結果
help-actions-line-2 = r 繼續被阻擋或待處理的工作階段；U 開啟用量統計
help-actions-line-3 = a/A/d/D 回覆第一個待處理權限請求
help-actions-line-4 = 在輸入區用 Ctrl+R 開啟第一個待處理使用者輸入請求
help-actions-line-5 = 已停用滑鼠捕捉，終端原生選取與複製仍可使用
help-actions-line-6 = Ctrl+C 連按兩次離開

overlay-session-search-title = 工作階段搜尋
overlay-session-search-prompt = 搜尋工作階段標題
overlay-transcript-search-title = 記錄搜尋
overlay-transcript-search-prompt = 在已載入訊息中搜尋
overlay-line-footer = 輸入以編輯

overlay-attach-title = 附加檔案
overlay-attach-prompt = 輸入路徑或搜尋詞。Enter 會附加目前選中的檔案。
overlay-attach-no-match = 沒有相符的檔案
overlay-attach-matches = 相符結果
overlay-attach-footer = Tab 填入選中路徑

overlay-user-input-title = 待處理使用者輸入
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = 允許自訂值
overlay-user-input-reply-format = 回覆格式：0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X 取消此請求
overlay-user-input-footer = Ctrl+X 取消

flash-terminal-event-error = 終端事件錯誤：{$error}
flash-created-session = 已建立工作階段 {$title}
flash-permission-reply-sent = 權限回覆已送出：{$label}
flash-user-input-reply-sent = 使用者輸入回覆已送出
flash-large-paste-staged = 大段貼上已暫存到輸入區
flash-attached = 已附加 {$path}
flash-composer-updated = 輸入區內容已從外部編輯器更新
flash-prompt-history-empty = 提示詞歷史是空的
flash-prompt-history-items = 召回提示詞歷史前，請先清空附件或已暫存的貼上內容
flash-external-editor-failed = 外部編輯器失敗：{$error}
flash-clipboard-image-attached = 已附加剪貼簿圖片：{$width}x{$height} {$format}
flash-clipboard-image-attach-failed = 附加剪貼簿圖片失敗：{$error}
flash-no-loaded-transcript = 沒有可複製的已載入內容
flash-copied-loaded-transcript = 已將已載入內容複製到剪貼簿
flash-no-assistant-message = 沒有可複製的 assistant 訊息
flash-no-assistant-message-text = 最後一則 assistant 訊息沒有已載入文字可複製
flash-copied-assistant-message = 已將最後一則 assistant 訊息複製到剪貼簿
flash-no-visible-transcript = 沒有可複製的目前可見文字
flash-copied-visible-transcript = 已將目前可見內容複製到剪貼簿
flash-clipboard-copy-failed = 剪貼簿複製失敗：{$error}
flash-message-interrupting = 正在中斷目前執行 - 訊息將立即傳送

message-role-user = 使用者
message-role-assistant = 助手
message-role-system = 系統

message-state-pending = 待處理
message-state-in-progress = 進行中
message-state-completed = 已完成
message-state-failed = 失敗
message-state-policy-denied = 已被使用者權限策略禁止
message-state-user-declined = 使用者已拒絕
message-state-capability-unavailable = 目前執行環境不具備此能力
message-state-tool-unavailable = 工具無法使用

message-parts-not-loaded = 還有 {$count} 個分段未載入
message-usage = 用量：輸入={$input} 輸出={$output} 推理={$reasoning}
message-finish = 結束原因：{$finish}
message-empty = （空訊息）
message-thinking = 思考：{$summary}
message-command-status = 狀態：{$status}，結束碼={$exit}
message-file-changes = 檔案變更
message-file-changes-preview-one = 1 個檔案：{$paths}
message-file-changes-preview-many = {$count} 個檔案：{$paths}
message-file-changes-more = 另 {$count} 個
message-search = 搜尋：{$query}
message-todo-list = 待辦清單
message-error = 錯誤 [{$code}]：{$message}
message-attachments = 附件
message-awaiting-user-input = 等待使用者輸入：{$request_id}
message-user-input-replied = 使用者輸入已回覆：{$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = 此分段詳情不可用
message-tool-pending = 待執行：{$label}
message-tool-running = 執行中：{$label}
message-tool-done = 完成：{$label}
message-tool-failed = 失敗：{$label}
message-tool-cancelled = 已取消：{$label}
message-tool-result-blocks = {$count} 個結果區塊

todo-status-pending = 待處理
todo-status-in-progress = 進行中
todo-status-completed = 已完成
todo-status-cancelled = 已取消

todo-priority-high = 高
todo-priority-medium = 中
todo-priority-low = 低

file-change-added = 新增
file-change-updated = 更新
file-change-deleted = 刪除

time-just-now = 剛剛
time-minutes-ago = {$count} 分鐘前
time-hours-ago = {$count} 小時前
time-days-ago = {$count} 天前

session-default-title = 新工作階段 {$time}
session-default-base = 新工作階段
session-fallback-title = 工作階段 {$id}

user-input-error-empty = 回覆不能為空
user-input-error-invalid-segment = 回覆片段無效：{$segment}
user-input-error-unknown-question = 未知的問題 ID：{$question_id}
user-input-error-missing-answer = 問題 {$question_id} 至少需要一個答案
user-input-error-no-answers = 回覆中沒有任何答案

attachment-kind-image = 圖片
attachment-kind-audio = 音訊
attachment-kind-video = 影片
attachment-kind-pdf = PDF
attachment-kind-file = 檔案
attachment-kind-directory = 資料夾
attachment-generic = 附件
attachment-chip-image = {$kind}：{$filename} ({$width}x{$height}, {$size})
attachment-chip-other = {$kind}：{$filename} ({$size})
attachment-placeholder = [{$kind} {$filename}]

bytes-gb = {$value} GB
bytes-mb = {$value} MB
bytes-kb = {$value} KB
bytes-b = {$value} B

paste-label = 貼上 {$count} 個字元
paste-label-append = 貼上 {$count} 個字元，送出時追加
paste-placeholder = [貼上 {$count} 個字元]

permission-label-allow-once = 允許一次
permission-label-allow-always = 永遠允許
permission-label-deny-once = 拒絕一次
permission-label-deny-always = 永遠拒絕

permission-summary-allow-once = 已允許一次：{$reason}
permission-summary-allow-always = 已永遠允許：{$reason}
permission-summary-deny-once = 已拒絕一次：{$reason}
permission-summary-deny-always = 已永遠拒絕：{$reason}

failure-detail-message = 訊息
failure-detail-code = 錯誤碼
failure-detail-category = 分類
failure-detail-responsibility = 責任方
failure-detail-impact = 影響
failure-detail-recovery = 復原建議
failure-detail-retry = 重試策略
failure-category-invalid-input = 輸入無效
failure-category-not-found = 找不到
failure-category-conflict = 衝突
failure-category-permission-required = 需要權限
failure-category-permission-denied = 權限被拒絕
failure-category-authentication-required = 需要身分驗證
failure-category-rate-limited = 請求過於頻繁
failure-category-quota-exceeded = 配額已用盡
failure-category-timeout = 逾時
failure-category-dependency-unavailable = 依賴無法使用
failure-category-protocol-failure = 協定錯誤
failure-category-data-corruption = 資料完整性問題
failure-category-internal = 內部錯誤
failure-responsibility-caller = 請求方
failure-responsibility-policy = 政策
failure-responsibility-dependency = 依賴方
failure-responsibility-system = 系統
failure-impact-request-rejected = 請求被拒絕
failure-impact-operation-failed = 操作失敗
failure-impact-operation-paused = 操作暫停
failure-impact-partial-success = 部分成功
failure-impact-background-task-failed = 背景工作失敗
failure-impact-runtime-degraded = 執行環境降級
failure-impact-fatal-startup-failure = 嚴重啟動失敗
failure-recovery-none = 無自動復原
failure-recovery-refresh = 重新整理
failure-recovery-reauthenticate = 重新登入
failure-recovery-open-settings = 開啟設定
failure-recovery-request-permission = 申請權限
failure-recovery-ask-user = 詢問使用者
failure-recovery-retry = 重試
failure-recovery-choose-alternative = 選擇替代方案
failure-recovery-restart-plugin = 重新啟動外掛
failure-recovery-restart-runtime = 重新啟動執行環境
failure-retry-never = 不要重試
failure-retry-correct-input = 修正輸入後重試
failure-retry-after-user-action = 使用者操作後重試
failure-retry-after-refresh = 重新整理後重試
failure-retry-immediate-once = 立即重試一次
failure-retry-backoff = 退避重試
failure-retry-use-alternative = 使用替代方案
failure-retry-unknown = 未知

## Settings Studio parity imported from zh-CN
## Missing Settings keys are converted from the reviewed Simplified Chinese catalog.

permission-studio-new-rule-label = + 新建規則

permission-studio-new-rule-value = 建立

permission-studio-catalog-tags-title = 添加工具標籤規則

permission-studio-catalog-names-title = 添加工具訪問規則

permission-studio-catalog-prompt = 搜索目前實時工具目錄。可多選已有條目，也可選擇“自定義規則”填寫尚未註冊的值。

permission-studio-catalog-footer = 向下進入結果 · Space 切換 · Enter 選擇模式 · Esc 取消

permission-studio-catalog-tag-detail = 目前有 {$count} 個已註冊工具使用

permission-studio-catalog-custom-label = + 自定義規則…

permission-studio-catalog-custom-detail = 添加目前實時目錄中不存在的標籤或工具名稱。

permission-studio-catalog-custom-search = 自定義 新建 手動 標籤 工具 名稱

overlay-settings-title = 設定

overlay-settings-footer = Ctrl+R 重新整理 · 左右鍵切換面板 · Tab/Shift+Tab 循環面板 · 上下鍵選擇 · Enter 打開 · Esc 關閉

overlay-settings-sections = 分區

overlay-settings-options = 選項

overlay-settings-group-core = 核心

overlay-settings-group-application = 應用

overlay-settings-group-session = 工作階段

overlay-settings-group-system = 系統

overlay-settings-default-section-title = 分區

overlay-settings-empty-section = 目前未選擇分區。

overlay-settings-empty-items = 這個分區裡沒有設定項。

overlay-settings-empty-detail = 選擇一個分區和選項以查看或編輯它。

overlay-settings-detail-current = 目前值：{$value}

overlay-settings-detail-path = 路徑：{$path}

overlay-settings-detail-action = 打開或編輯這個設定。

settings-detail-action-screen = 打開這個頁面。

overlay-settings-edit-title = 編輯 {$field}

overlay-settings-edit-file-value = 檔案覆蓋值：{$value}

overlay-settings-edit-effective-value = 生效值：{$value}

overlay-settings-help-string = 輸入文本。留空或輸入 `clear` 可移除檔案覆蓋值。

overlay-settings-help-bool = 輸入 true/false、on/off、yes/no 或 1/0。留空或輸入 `clear` 可移除檔案覆蓋值。

overlay-settings-help-integer = 輸入整數。留空或輸入 `clear` 可移除檔案覆蓋值。

overlay-settings-help-float = 輸入數字。留空或輸入 `clear` 可移除覆蓋值。

overlay-choice-clear-settings-detail = 移除 {$field} 的檔案覆蓋值。

overlay-settings-section-plugins-label = 插件與工具

overlay-settings-section-plugins-summary = 插件配置、工具、運行環境與診斷

overlay-settings-section-plugins-description = 配置插件、查看工具和診斷，並管理瀏覽器、Shell 與編輯器 Harness。

overlay-settings-section-providers-label = 模型與服務商

overlay-settings-section-providers-summary = {$count} 個已配置服務商

overlay-settings-section-providers-description = 配置服務商及其網絡行為，並查看模型目錄。

overlay-settings-section-model-catalog-label = 模型目錄

overlay-settings-section-model-catalog-summary = {$count} 個條目

overlay-settings-section-model-catalog-description = 瀏覽解析後的 model catalog，查看條目元數據，並重新整理本地緩存。

overlay-settings-section-permissions-label = 權限

overlay-settings-section-permissions-summary = {$count} 條持久化權限規則

overlay-settings-section-permissions-description = 分別編輯全局、工作區與目前工作階段的權限。

overlay-settings-section-tracing-summary = 日誌過濾與診斷

overlay-settings-section-ui-label = 外觀

overlay-settings-section-ui-summary = 語言與界面偏好

overlay-settings-section-ui-description = 持久化的語言、配色、圖形與主題設定。

overlay-settings-section-runtime-session-label = 執行階段與工作階段

overlay-settings-section-runtime-session-summary = Provider 客戶端身份與上下文壓縮

overlay-settings-section-runtime-session-description = 配置兼容客戶端版本，以及工作階段自動壓縮行為。

settings-permission-global-label = 全局權限

settings-permission-global-detail = 所有工作階段的預設基線。

settings-permission-workspace-label = 工作區權限

settings-permission-workspace-detail = 目前項目的覆蓋層。

settings-permission-current-label = 目前工作階段權限

settings-permission-current-detail = 僅覆蓋目前工作階段。

settings-permission-effective-label = 實際生效權限

settings-permission-effective-detail = 只讀 · 全局/工作區/工作階段合併結果。

settings-permission-effective-read-only = 實際生效權限是隻讀結果；請改 session、workspace 或 global 來源。

settings-permission-layer-global = 全局

settings-permission-layer-workspace = 工作區

settings-permission-layer-session = 工作階段

settings-permission-layer-effective = 生效

settings-runtime-thinking-label = Think 模式

settings-runtime-thinking-description = 目前工作階段 think 模式覆蓋

settings-runtime-speed-label = Speed 模式

settings-runtime-speed-description = 目前工作階段 speed 模式覆蓋

settings-runtime-verbosity-label = 詳細程度

settings-runtime-verbosity-description = 目前工作階段 verbosity 覆蓋

settings-field-permission-approval-model-label = 自動核准模型

settings-field-permission-approval-model-description = 用於自動權限決策的模型及 think/speed variant；不可用時自動回退到 ask

settings-field-ui-locale-label = 語言

settings-field-ui-locale-description = 界面語言

settings-field-tui-color-scheme-label = 終端機配色模式

settings-field-tui-color-scheme-description = 自動檢測終端機背景，或強制使用亮色/暗色配色

settings-field-tui-theme-label = TUI 插件主題

settings-field-tui-theme-description = 可選的插件語義色主題

settings-choice-tui-color-scheme-auto = 自動檢測終端機背景

settings-choice-tui-color-scheme-dark = 針對暗色終端機背景優化

settings-choice-tui-color-scheme-light = 針對亮色終端機背景優化

settings-field-tui-graphics-label = 豐富終端機圖形

settings-field-tui-graphics-description = 在支持的終端機中通過 Kitty、Sixel 或 iTerm2 顯示圖片和排版公式；重啟 TUI 後生效

settings-choice-tui-graphics-auto = 自動協商原生圖形，不支持時安全回退到 Unicode（推薦）

settings-choice-tui-graphics-native = 為已由專家配置好的終端機鏈路強制協商原生圖形

settings-choice-tui-graphics-unicode = 關閉原生圖形，使用確定性的 Unicode/文本渲染

settings-field-activity-default-expanded-label = 預設展開活動

settings-field-activity-default-expanded-description = 未單獨配置種類的活動的預設展開狀態。推理（reasoning）預設展開，除非單獨設定其種類。

settings-field-activity-kind-description = 該活動種類的預設展開狀態。

settings-field-activity-tool-label = 工具預設展開

settings-field-activity-tool-description = 該精確工具的預設展開狀態。

settings-activity-kind-reasoning-label = 推理

settings-activity-kind-reasoning-description = 模型的完整思考過程。預設展開，可按種類摺疊。

settings-activity-kind-operation-label = 工具操作

settings-activity-kind-operation-description = 工具調用及其結果。

settings-activity-kind-resource-label = 資源

settings-activity-kind-resource-description = 附件及其他資源內容。

settings-activity-kind-skill_reference-label = 技能引用

settings-activity-kind-skill_reference-description = 回覆中使用的技能引用。

settings-activity-kind-interaction-label = 交互

settings-activity-kind-interaction-description = 用戶輸入請求和交互提示。

settings-activity-kind-hook-label = 鉤子

settings-activity-kind-hook-description = 工作階段鉤子運行與生命週期事件。

settings-activity-kind-error-label = 錯誤

settings-activity-kind-error-description = 失敗操作與終端機故障。

settings-activity-kind-notice-label = 通知

settings-activity-kind-notice-description = 後臺通知與信息行。

settings-activity-kind-text-label = 文本

settings-activity-kind-text-description = 純文本與文本工件內容。

settings-field-tracing-filter-label = 應用程式日誌層級

settings-field-tracing-filter-description = 預設 tracing 日誌級別

settings-field-tracing-database-label = 資料庫日誌層級

settings-field-tracing-database-description = database tracing 日誌級別

settings-field-tracing-adapter-label = 適配器日誌層級

settings-field-tracing-adapter-description = provider adapter tracing 日誌級別

settings-config-open-file-detail = 打開 agena.json 查看或編輯這個路徑

settings-source-unset = 未設定

settings-source-configured = 已配置：{$value}

settings-source-effective = 生效：{$value}

settings-source-file-effective = 檔案：{$file} / 生效：{$effective}

settings-source-file-found = {$path}（已找到）

settings-source-file-missing = {$path}（儲存時建立）

settings-source-row-config-file = 配置檔案

settings-source-row-workspace-config-file = 工作區配置檔案

settings-source-row-file-value = 檔案值

settings-source-row-workspace-value = 工作區值

settings-source-row-effective-value = 生效值

settings-source-row-write-target = 寫入位置

settings-source-row-layers = 目前層級

settings-source-current-session = 目前 session 運行時數據

settings-source-current-session-runtime = 目前 session run options

settings-detail-values-heading = 值

settings-detail-sources-heading = 來源

settings-detail-action-readonly = 打開只讀的實際生效視圖。

settings-detail-action-file = 打開背後的配置檔案。

settings-harness-browser-label = Browser 執行環境

settings-harness-shell-label = Shell 執行環境

settings-harness-editor-label = Editor 執行環境

settings-field-parse-bool = {$field} 需要布爾值，例如 true/false 或 on/off

settings-field-parse-integer = {$field} 需要無符號整數值

settings-field-parse-float = {$field} 需要數值

settings-choice-adapter-fallback = 適配器


settings-plugin-workbench-label = 插件設定工作台

settings-plugin-workbench-detail = 打開結構化插件工作台，查看運行時狀態、配置、工具、操作、日誌和診斷。

settings-mcp-server-label = Agena MCP 伺服器

settings-mcp-server-value = 切換啟用/關閉

settings-mcp-server-enabled = 已啟用

settings-mcp-server-disabled = 已關閉

settings-mcp-status-unavailable = 狀態不可用

settings-mcp-ready = 就緒

settings-mcp-needs-attention = 需要處理

settings-mcp-server-detail = 切換 Agena 實時 HTTP MCP 能力；實際運行者始終是所連接的 Agena server 進程。

settings-mcp-auth-label = MCP 鑑權

settings-mcp-auth-none = 匿名：所有已暴露工具

settings-mcp-auth-oauth = 完整 OAuth

settings-mcp-auth-mixed = 混合模式：公開發現，工具級 OAuth

settings-mcp-auth-detail = 在無鑑權、完整 OAuth 和 ChatGPT 混合鑑權之間循環。混合模式公開初始化與工具發現；除非顯式啟用匿名訪問，否則所有工具調用仍受 OAuth 保護。

settings-mcp-anonymous-access-label = 混合鑑權匿名工具訪問

settings-mcp-anonymous-access-none = 無（推薦）

settings-mcp-anonymous-access-read-only = 權限契約確認的只讀工具

settings-mcp-anonymous-access-none-detail = 安全預設值：沒有工具調用可匿名執行；ChatGPT 仍可在登錄前完成初始化並發現工具目錄。

settings-mcp-anonymous-access-read-only-detail = 高風險選項：只讀工具可匿名執行，仍可能洩露私有工作區、檔案系統、配置或診斷數據。

settings-mcp-anonymous-access-inactive-detail = 此策略僅用於混合鑑權模式；先將鑑權切換為混合模式。

settings-mcp-registration-label = 註冊

settings-mcp-pkce-label = PKCE

settings-mcp-client-registration-label = OAuth 客戶端註冊

settings-mcp-client-registration-cimd = 僅 CIMD（推薦）

settings-mcp-client-registration-dcr = CIMD + 動態客戶端註冊

settings-mcp-client-registration-cimd-detail = 僅接受 OpenAI ChatGPT Client ID Metadata Document；關閉無需鑑權的公網 DCR 端點。

settings-mcp-client-registration-dcr-detail = 兼容模式：同時開放公網動態客戶端註冊。僅在客戶端無法使用 CIMD 時啟用。

settings-mcp-public-url-label = MCP 公共 URL

settings-mcp-public-url-value = 編輯

settings-mcp-public-url-auto = 監聽器本地回退地址

settings-mcp-public-url-detail = 設定規範 HTTPS MCP resource URL。Secure MCP Tunnel 可以保留完整 /v1/mcp/tunnel_id 路徑；絕不信任轉發請求頭來定義 OAuth 身份。

settings-mcp-oauth-issuer-label = OAuth 簽發者 URL

settings-mcp-oauth-issuer-derived = 從 MCP resource origin 推導

settings-mcp-oauth-issuer-detail = 設定瀏覽器可訪問的公共授權服務器 issuer。Agena 內置 OAuth 要求使用不帶路徑的 origin，例如 https://auth.example.com；OAuth 與 MCP 同域時可留空自動推導。

settings-mcp-oauth-password-label = MCP OAuth 密碼

settings-mcp-oauth-password-value = 設定或替換

settings-mcp-oauth-password-configured = 已配置 MCP 專用密碼

settings-mcp-oauth-password-ui-fallback = 使用 UI 密碼回退

settings-mcp-oauth-password-not-configured = 未配置

settings-mcp-oauth-password-detail = 設定 Agena OAuth 授權頁面使用的密碼；密碼由 server 以 Argon2 哈希儲存。

settings-mcp-oauth-password-clear-label = 清除 MCP OAuth 密碼

settings-mcp-oauth-password-clear-detail = 刪除 MCP 專用密碼；如果配置了 UI 密碼，則回退到 UI 密碼。

settings-field-runtime-codex-version-label = Codex 客戶端版本

settings-field-runtime-codex-version-description = Provider 請求身份 Header 使用的 @openai/codex 精確兼容版本。

settings-field-runtime-claude-version-label = Claude Code 版本

settings-field-runtime-claude-version-description = Provider 請求身份 Header 使用的 @anthropic-ai/claude-code 精確兼容版本。

settings-field-runtime-gemini-version-label = Gemini CLI 版本

settings-field-runtime-gemini-version-description = Provider 請求身份 Header 使用的 @google/gemini-cli 精確兼容版本。

settings-field-session-compaction-auto-label = 自動壓縮

settings-field-session-compaction-auto-description = 工作階段接近上下文窗口上限時自動壓縮。

settings-field-session-compaction-reserved-tokens-label = 壓縮保留 Token

settings-field-session-compaction-reserved-tokens-description = 判斷壓縮時從上下文窗口中預留的 Token 數；清除後使用自動計算值。

settings-client-versions-refresh-label = 重新整理客戶端版本

settings-client-versions-refresh-value = 獲取最新版本

settings-client-versions-refresh-description = 從 npm 獲取最新兼容包版本，持久化三個精確版本值，並重載運行時。

settings-client-versions-entry-label = Provider 客戶端版本

settings-client-versions-entry-value = codex · claude · gemini

settings-client-versions-entry-detail = 打開 Provider 請求身份 Header 使用的三個精確兼容版本。

settings-client-versions-section-label = 客戶端版本

settings-client-versions-section-summary = 運行時身份版本

settings-client-versions-section-description = Provider 請求身份 Header 使用的精確兼容版本。可逐個編輯，按 Ctrl+R 從 npm 重新整理。

settings-provider-workbench-label = 服務商列表

settings-provider-workbench-value = {$count} 個服務商

settings-provider-workbench-detail = 先打開可搜索的服務商列表，再配置認證、adapter、模型路由或新建服務商。

settings-model-default-mode-inherit-detail = 使用所選 model 的原生預設模式。

settings-provider-new-label = + 新建 provider

settings-provider-new-detail = 建立新 provider，列出 live adapter models，並編輯 provider adapter 配置；模型需單獨選擇。

settings-provider-existing-detail = 已配置 {$count} 個 adapter

settings-model-catalog-open-label = 打開 Model Catalog

settings-model-catalog-open-detail = 查看解析後的 model 元數據，並重新整理本地 model catalog 緩存。

settings-files-open-config-label = 打開 agena.json

settings-files-open-config-present = 已存在

settings-files-open-config-create = 打開時建立

permission-studio-field-path-workspace = 路徑工作區預設值

permission-studio-field-path-external = 路徑外部預設值

permission-studio-field-path-rules = 路徑規則

permission-studio-field-network-defaults = 網絡預設值

permission-studio-field-network-rules = 網絡規則

permission-studio-field-tool-names = 工具名稱

permission-studio-field-tool-rules = 工具規則

permission-studio-command-rules-shell-only = 命令規則只能配置標準 shell 工具（agena.shell.run）；其他工具請用名稱規則或預設策略。

permission-studio-field-prompt-json = 輸入 {$field} 的 JSON。留空可清除此覆蓋值。

permission-studio-detail-override = 覆蓋值

permission-studio-detail-effective = 生效值

permission-studio-detail-override-inline = 覆蓋 {$value}

permission-studio-detail-effective-inline = 生效 {$value}

permission-studio-detail-editable = Enter 會為這一段權限打開多行 JSON 編輯器。

permission-studio-detail-read-only = 這個權限文檔在這裡是只讀的。

permission-studio-detail-mode-editable = Enter 會為這個字段打開 mode 選擇器。

permission-studio-detail-text-editable = Enter 會編輯這一個 key 或 pattern。

permission-studio-detail-add-hint = Enter 會建立這個條目並立即打開它。

permission-studio-detail-remove-hint = Enter 會立即移除這個條目。

permission-studio-detail-navigate-hint = Enter 會打開這個分區。

permission-studio-detail-full-config-editable = Enter 會為完整文檔打開高級 JSON 編輯器。

permission-studio-overview-target = 目標

permission-studio-overview-source = 來源

permission-studio-overview-scope = 作用域

permission-studio-overview-override = 覆蓋值

permission-studio-overview-effective = 生效值

permission-studio-section-workspace = 工作區

permission-studio-section-external = 外部

permission-studio-section-rules = 規則

permission-studio-section-defaults = 預設值

permission-studio-source-global = 全局

permission-studio-source-workspace = 工作區

permission-studio-source-session = 工作階段

permission-studio-source-effective = 實際生效

permission-studio-settings-override = 覆蓋 {$value}

permission-studio-settings-effective = 生效 {$value}

permission-studio-mode-read = 讀 {$value}

permission-studio-mode-write = 寫 {$value}

permission-studio-network-default = {$label} {$value}

permission-studio-page-overview = 概覽

permission-studio-page-path = 路徑

permission-studio-page-path-defaults = 檔案系統 / 預設區域

permission-studio-page-path-rules = 檔案系統 / 路徑規則

permission-studio-page-network = 網絡

permission-studio-page-network-zones = 網絡 / 網絡區域

permission-studio-page-network-rules = 網絡 / 域名規則

permission-studio-page-tools = 工具

permission-studio-page-tool-tags = 工具權限 / 標籤規則

permission-studio-page-tool-names = 工具權限 / 名稱規則

permission-studio-page-tool-command-rules = 工具權限 / 命令規則

permission-studio-page-names = 名稱

permission-studio-page-tool-rules = 工具規則

permission-studio-nav-overview = 概覽

permission-studio-nav-filesystem = 檔案系統

permission-studio-nav-default-zones = 預設區域

permission-studio-nav-path-rules = 路徑規則

permission-studio-nav-network = 網路

permission-studio-nav-network-zones = 網路區域

permission-studio-nav-domain-rules = 域名規則

permission-studio-nav-tool-access = 工具權限

permission-studio-nav-name-rules = 名稱規則

permission-studio-nav-command-rules = 命令規則

permission-studio-path-workspace-read = 工作區讀

permission-studio-path-workspace-write = 工作區寫

permission-studio-path-external-read = 外部讀

permission-studio-path-external-write = 外部寫

permission-studio-path-rule-read = 讀 mode

permission-studio-path-rule-write = 寫 mode

permission-studio-network-internet = 公網

permission-studio-network-private = 私網

permission-studio-network-loopback = 迴環

permission-studio-tool-default = 工具預設值

permission-studio-tool-default-summary = 預設 {$value}

permission-studio-add-path-rule = 添加路徑規則

permission-studio-add-network-rule = 添加網絡目標

permission-studio-add-name = 添加名稱

permission-studio-add-tool-rule = 添加工具規則

permission-studio-rule-key = 鍵

permission-studio-rule-pattern = 模式

permission-studio-rule-target = 目標

permission-studio-rule-mode = 權限模式

permission-studio-tool-rule-fallback = 兜底 mode

permission-studio-error-empty-value = {$field} 不能為空。

overlay-providers-title = Provider 列表

overlay-providers-prompt = 選擇一個 provider 進行配置

overlay-provider-list-title = Provider 列表

overlay-provider-list-prompt = 搜索已配置的 provider

overlay-provider-list-footer = 選擇“新建 Provider”或已有 Provider，然後按 Enter

overlay-provider-list-create-label = + 新建 Provider

overlay-provider-list-create-detail = 建立服務商草稿，然後設定認證、適配器與模型。

overlay-provider-list-row-detail-no-model = {$adapter} · 已配置 {$count} 個 adapters

overlay-provider-studio-title = Provider 配置

overlay-provider-studio-header = Provider 配置

overlay-provider-studio-footer = Tab/Shift+Tab 切換面板 · 方向鍵選擇 · Space 切換 · Enter 編輯 · Ctrl+D 刪除選中項 · Ctrl+R 重新整理 · Ctrl+N 新增模型 · Ctrl+A 儲存 Adapter · Ctrl+S 儲存 Provider · Esc 關閉

overlay-provider-studio-providers = 服務商

overlay-provider-studio-draft = 草稿

overlay-provider-studio-adapters = 適配器

overlay-provider-studio-models = 模型

overlay-provider-studio-catalog = 模型 Catalog

overlay-provider-studio-detail = 詳情

overlay-provider-studio-detail-footer = 方向鍵選擇 · Enter 編輯 · Esc 返回；認證操作位於 Provider 主頁面的可見操作行

overlay-provider-studio-adapter-models-empty = 先選擇 adapter，再列出 live model 列表

overlay-provider-studio-models-empty = 目前沒有可用的適配器模型

overlay-provider-studio-catalog-empty = 目前查詢沒有匹配的 catalog 條目

overlay-provider-studio-new-provider-detail = 空的 provider 草稿

overlay-provider-studio-provider-row-detail-no-model = {$adapter} · 已配置 {$count} 個 adapters

overlay-provider-studio-model-count = {$count} 個模型

overlay-provider-studio-loaded = 已載入

overlay-provider-studio-error = 錯誤

overlay-provider-studio-configured = 已配置

overlay-provider-studio-live-list = 實時列表

overlay-provider-studio-configured-disk = 已在磁盤中配置，但不屬於目前認證契約

overlay-provider-studio-not-listed = 未列出

overlay-provider-studio-not-supported = 目前認證契約不支持

overlay-provider-studio-edit-title = 編輯字段

overlay-provider-studio-edit-prompt = 更新 {$field}

overlay-provider-studio-edit-footer = 輸入以編輯

overlay-provider-studio-model-edit-footer = Ctrl+S 儲存模型配置

overlay-provider-studio-model-json-title = 模型配置 · {$adapter}/{$model}

overlay-provider-studio-model-json-prompt = 編輯持久化的 provider model JSON。

overlay-provider-studio-model-title = 模型 · {$adapter}/{$model}

overlay-provider-studio-model-footer = 方向鍵選擇 · Enter 編輯 · Ctrl+S 儲存 · Ctrl+D 刪除 · Esc 返回

overlay-provider-delete-title = 刪除 Provider

overlay-provider-delete-body = 刪除服務商 {$provider} 以及其所有已設定的適配器和模型？

overlay-provider-delete-adapter-title = 刪除 Adapter

overlay-provider-delete-adapter-body = 刪除已設定的適配器 {$provider}/{$adapter}？

overlay-provider-delete-adapter-last-body = 這是最後一個已設定的適配器。確認後會一併刪除整個服務商。

overlay-provider-delete-model-title = 刪除模型

overlay-provider-delete-model-body = 刪除已設定的模型 {$provider}/{$adapter}/{$model}？

overlay-provider-studio-model-edit-title = 編輯模型字段

overlay-provider-studio-model-field-prompt = 更新 {$field}

overlay-provider-studio-new-model-title = 添加模型

overlay-provider-studio-new-model-prompt = 輸入要添加到目前 adapter 下的 model id。

overlay-provider-studio-edit-auth-mode-prompt = 更新 auth mode（none | api | credential）

overlay-provider-studio-edit-auth-subtype-prompt = 更新 auth subtype（api：custom | cline_api | gitlab_api | bedrock_sigv4；credential：openai_chatgpt | github_copilot | gitlab | google_adc | sap_ai_core）

overlay-provider-studio-edit-auth-login-method-prompt = 更新登錄方式（device | browser）

provider-studio-auth-status-pending = 待完成

provider-studio-auth-status-unset = 未設定

provider-studio-auth-status-none = 無

provider-studio-auth-status-select-subtype = 選擇子類

provider-studio-auth-status-select-issuer = 選擇子類

provider-studio-auth-status-configured = 已配置

provider-studio-auth-status-partial = 部分已填

provider-studio-summary-env = 環境變量

provider-studio-summary-callback = 回調

provider-studio-summary-redirect = 重定向

provider-studio-summary-account = 賬號

provider-studio-summary-name = 名稱

provider-studio-summary-user = 用戶

provider-studio-summary-email = 郵箱

provider-studio-summary-profile = Profile

provider-studio-summary-region = 區域

provider-studio-summary-code = 代碼

provider-studio-summary-state = 狀態 {$state}

provider-studio-summary-tokens-set = 已設定 token

provider-studio-summary-keys-set = 已設定密鑰

provider-studio-summary-set-field = 設定 {$field}

provider-studio-summary-review-fields = 查看認證字段

provider-studio-summary-start-browser = 開始瀏覽器 OAuth

provider-studio-summary-restart-browser = 重新開始瀏覽器 OAuth

provider-studio-summary-open-authorize = 打開授權 URL

provider-studio-summary-start-device = 開始設備登錄

provider-studio-summary-restart-device = 重新開始設備登錄

provider-studio-summary-open-verify = 打開驗證 URL

provider-studio-summary-finish-callback = 完成回調換取

provider-studio-summary-poll-every = 每 {$seconds} 秒輪詢

provider-studio-summary-paste-callback = 粘貼 Callback URL

provider-studio-summary-poll-now = 立即輪詢

provider-studio-summary-start-auth-first = 先開始認證

provider-studio-summary-poll-browser = 輪詢瀏覽器結果

provider-studio-auth-openai-ready = 瀏覽器 OAuth 已就緒，打開下面的授權 URL

provider-studio-auth-openai-device-ready = OpenAI 設備登錄已就緒，打開下面的驗證 URL 並輸入 {$code}

provider-studio-auth-authorize = 授權 {$url}

provider-studio-auth-redirect = 重定向 {$url}

provider-studio-auth-paste-callback = 將跳轉後的 URL 粘貼到 Callback URL，然後按 p · 狀態 {$state}

provider-studio-auth-copilot-ready = 設備登錄已就緒，打開下面的驗證 URL 並輸入 {$code}

provider-studio-auth-verify = 驗證 {$url}

provider-studio-auth-poll = 按 p 立即輪詢 · 每 {$seconds} 秒一次

provider-studio-auth-gitlab-ready = GitLab 瀏覽器 OAuth 已就緒，打開下面的授權 URL

provider-studio-auth-atomgit-ready = AtomGit 瀏覽器工作階段已就緒，下面顯示授權 URL

provider-studio-auth-finish-browser = 完成瀏覽器流程後按 p · 狀態 {$state}

flash-server-config-edit-in-settings = 配置檔案位於服務端。請直接在“設定”中編輯配置值，不能將服務端路徑作為客戶端本地檔案打開。

flash-settings-updated = 已更新 {$path}

flash-settings-cleared = 已清空 {$path}

flash-provider-save-error-settings-object = 現有 provider settings 必須是一個 JSON object

command-settings-summary = 打開統一設定工作台，管理模型、權限、插件、運行時、工作階段、界面與診斷

settings-mcp-public-url-updated = Agena MCP 公共 URL 已更新

settings-mcp-oauth-issuer-updated = Agena MCP OAuth 頒發者 URL 已更新

settings-mcp-oauth-password-updated = Agena MCP OAuth 密碼已更新

settings-mcp-server-enabled-flash = Agena MCP 服務已啟用

settings-mcp-server-disabled-flash = Agena MCP 服務已禁用

settings-mcp-auth-mode-updated = Agena MCP 身份驗證模式已設為 {$mode}

settings-mcp-anonymous-access-updated = Agena MCP 匿名工具訪問策略已設為 {$policy}

settings-mcp-client-registration-updated = Agena MCP 客戶端註冊策略已設為 {$policy}

settings-mcp-oauth-password-cleared = Agena MCP OAuth 密碼已清除

permission-studio-command-pattern-title = {$tool_name} 命令模式

permission-studio-command-pattern-help = 輸入 Shell 命令 glob，例如 `git status` 或 `git push *`。

permission-studio-rename-unsupported = 此條目無法重命名；請刪除後重新創建。

settings-tool-api-list-description = 枚舉執行工具。

settings-tool-api-search-description = 搜索執行工具。

settings-tool-api-help-description = 查看執行工具契約。

settings-tool-api-tags-description = 列出執行工具標籤。

settings-tool-api-call-description = 調用執行工具。

settings-tool-api-plugins-list-description = 枚舉工具插件。

settings-tool-api-plugins-search-description = 搜索工具插件。

settings-tool-api-plugins-tags-description = 列出工具插件標籤。

# Settings, provider, permission, catalog, MCP, and diagnostics completion
overlay-editor-footer-single-line = 輸入要編輯的內容
overlay-editor-footer-multiline = Ctrl+S 儲存
context-help-title = 上下文幫助
context-help-eyebrow = 目前介面
context-help-footer = ↑/↓ 捲動 · Esc 或 Ctrl+H 關閉
context-help-global-hint = Ctrl+H 幫助
context-help-context-composer-items = 作曲家專案
context-help-context-suggestions = 建議
context-help-context-usage = 使用情況儀表板
context-help-context-plan-viewer = 計劃查看器
context-help-context-user-input = 使用者輸入請求
context-help-context-plugin-list = 插件工作台·列表
context-help-context-plugin-detail = 插件工作台·詳情
context-help-context-plugin-config = 插件工作台·配置
context-help-context-plugin-actions = 插件配置 · 操作
context-help-context-plugin-selection = 插件配置·選擇
context-help-context-plugin-drilldown = 插件配置·深入分析
context-help-context-plugin-diff = 插件配置·差異
context-help-key-delete = 刪除所選項目。
context-help-key-plugin-restart = 如果支持，請重新啟動選定的插件。
overlay-permission-title = 許可請求
overlay-permission-details-title = 詳情
overlay-permission-action-tool = 工具：{ $tool }
overlay-permission-action-path = 路徑{ $access }：{ $path }
overlay-permission-action-network = 網路：{ $target }
overlay-permission-field-tool = 工具
overlay-permission-field-target = 命令或目標
overlay-permission-field-access = 訪問
overlay-permission-field-path = 路徑
overlay-permission-field-workspace = 工作空間
overlay-permission-field-network = URL 或網路目標
overlay-permission-field-host = 主持人
overlay-permission-field-reason = 為什麼需要批准
overlay-permission-detail-request-id = 請求ID
overlay-permission-detail-source = 政策來源
overlay-permission-detail-scope = 要求的範圍
overlay-permission-detail-operator = 請求者
overlay-permission-detail-trace = 決策追蹤
overlay-permission-summary-more-approvals = 同時批准此工具呼叫中的 { $count } 更多操作
overlay-permission-detail-requested-actions = 也請求批准
overlay-permission-detail-related-actions = 已允許參與此通話
overlay-permission-choice-auto-approve = 自動批准...
overlay-permission-rule-workbench-title = 權限規則
overlay-permission-rule-studio-footer = 箭頭選擇 · 進入編輯 · Ctrl+O 瀏覽選定路徑 · Ctrl+S 儲存 · Ctrl+D 關閉 · Ctrl+D 撤銷 Esc
overlay-permission-rule-studio-footer-return = 箭頭選擇 · 進入編輯 · Ctrl+O 瀏覽選定路徑 · Ctrl+S 儲存 · Ctrl+D
flash-permission-rule-browse-path-selection = 瀏覽之前選擇目標路徑或工作區根目錄。
overlay-permission-rule-choice-subject-title = 選擇主題類型
overlay-permission-rule-choice-subject-prompt = 選擇規則主題類型。
overlay-permission-rule-choice-subject-tool-detail = 匹配工具或運行時工具
overlay-permission-rule-choice-subject-path-access-detail = 匹配檔案系統訪問
overlay-permission-rule-choice-subject-network-access-detail = 匹配網路訪問
overlay-permission-rule-choice-access-title = 選擇路徑存取類型
overlay-permission-rule-choice-access-prompt = 選擇檔案系統存取模式。
overlay-permission-rule-choice-access-read-detail = 允許文件只讀
overlay-permission-rule-choice-access-write-detail = 只允許檔案寫入
overlay-permission-rule-choice-access-read-write-detail = 允許讀取和寫入
overlay-permission-rule-choice-scope-title = 選擇規則範圍
overlay-permission-rule-choice-scope-prompt = 選擇規則應持續的範圍。
overlay-permission-rule-choice-scope-session-detail = 僅此一屆
overlay-permission-rule-choice-scope-workspace-detail = 此工作區中的所有會話
overlay-permission-rule-choice-scope-global-detail = 所有工作區
overlay-permission-rule-choice-mode-title = 選擇規則模式
overlay-permission-rule-choice-mode-prompt = 選擇允許、詢問或拒絕。
overlay-permission-rule-choice-mode-allow-detail = 始終允許匹配的操作
overlay-permission-rule-choice-mode-auto-detail = 讓配置的審批模型決定；不可用時退回到提示
overlay-permission-rule-choice-mode-ask-detail = 在允許匹配操作之前提示
overlay-permission-rule-choice-mode-deny-detail = 始終拒絕匹配的操作
overlay-permission-rule-editor-footer = 輸入要編輯的內容
overlay-permission-rule-editor-tool-name-title = 編輯工具名稱
overlay-permission-rule-editor-tool-name-prompt = 輸入準確的工具名稱。
overlay-permission-rule-editor-qualifier-title = 編輯預選賽
overlay-permission-rule-editor-qualifier-prompt = 輸入可選限定符或留空。
overlay-permission-rule-editor-workspace-root-title = 編輯工作區根目錄
overlay-permission-rule-editor-workspace-root-prompt = 輸入可選的workspace_root 目錄。
overlay-permission-rule-editor-target-path-title = 編輯目標路徑
overlay-permission-rule-editor-target-path-prompt = 輸入目標路徑或模式。
overlay-permission-rule-editor-network-target-title = 編輯網路目標
overlay-permission-rule-editor-network-target-prompt = 輸入主機、主機:連接埠或 URL。
overlay-permission-rule-editor-session-id-title = 編輯會話 ID
overlay-permission-rule-editor-session-id-prompt = 輸入目標會話 ID。
overlay-permission-rule-browser-workspace-root-title = 選擇工作空間根目錄
overlay-permission-rule-browser-workspace-root-prompt = 瀏覽目錄並按 Enter 鍵選擇一個。
overlay-permission-rule-browser-target-path-title = 選擇目標路徑
overlay-permission-rule-browser-target-path-prompt = 瀏覽檔案或目錄並按 Enter 鍵選擇一個。
overlay-permission-rule-browser-footer = 選擇../或目錄並按Enter鍵瀏覽·選擇一個值並按Enter鍵接受
overlay-permission-rule-browser-empty = 沒有符合的檔案或目錄。
overlay-permission-rule-item-subject-kind = 學科種類
overlay-permission-rule-item-subject-kind-detail = 選擇此規則是否套用於工具、路徑或網路目標。
overlay-permission-rule-item-mode = 模式
overlay-permission-rule-item-mode-detail = 選擇是否允許、詢問或拒絕符合操作。
overlay-permission-rule-item-scope = 適用範圍
overlay-permission-rule-item-scope-detail = 在會話、工作區或全域中保留此規則。
overlay-permission-rule-item-session-id = 會話ID
overlay-permission-rule-item-session-id-detail = 當範圍=會話時使用的目標會話ID。
overlay-permission-rule-item-tool-name = 工具名稱
overlay-permission-rule-item-tool-name-detail = 精確比對的工具名稱。
overlay-permission-rule-item-qualifier = 預選賽
overlay-permission-rule-item-qualifier-detail = 更具體的工具規則的可選限定符。
overlay-permission-rule-item-access-kind = 訪問類型
overlay-permission-rule-item-access-kind-detail = 選擇讀、寫或read_write。
overlay-permission-rule-item-target-path = 目標路徑
overlay-permission-rule-item-target-path-detail = 要保護的路徑模式或確切路徑。
overlay-permission-rule-item-workspace-root = 工作空間根目錄
overlay-permission-rule-item-workspace-root-detail = 用於解釋相對目標路徑的可選基底目錄。
overlay-permission-rule-item-network-target = 網路目標
overlay-permission-rule-item-network-target-detail = 要匹配的主機、主機:連接埠或 URL 目標。
overlay-permission-rule-detail-subject-kind = 工具規則按工具名稱和可選限定符進行比對。路徑規則匹配檔案系統存取。網路規則符合主機或 URL 存取。
overlay-permission-rule-detail-tool-name = 工具規則需要準確的工具名稱，例如 `shell`、`read` 或 `web_search`。
overlay-permission-rule-detail-qualifier = 限定符是可選的。將其保留為空，除非工具或操作需要更窄的匹配。
overlay-permission-rule-detail-path-access-kind = 根據您要符合的檔案系統存取權限，使用 `read`、`write` 或 `read_write`。
overlay-permission-rule-detail-workspace-root = 將workspace_root 留空以繼承執行階段工作空間根。當受保護的路徑位於其他地方時明確設定它。
overlay-permission-rule-detail-target-path = 輸入路徑或模式。相對路徑在設定時根據workspace_root 進行解釋。
overlay-permission-rule-detail-network-target = 輸入主機、`host:port` 或完整 URL，取決於規則的具體程度。
overlay-permission-rule-detail-scope = 會話範圍最適合臨時覆蓋。工作空間和全域作用域持續時間較長。
overlay-permission-rule-detail-session-id = 會話範圍的規則需要具體的會話 ID。
overlay-permission-rule-detail-mode = 「允許」允許操作通過，要求提示批准，「拒絕」則阻止操作。
overlay-workbench-details = 詳情
overlay-permission-studio-title = 授權
overlay-permission-studio-footer-nested = Ctrl+N 新增 · 輸入編輯 · Ctrl+E 重新命名 · Ctrl+D 刪除 · Esc 返回
flash-permission-studio-catalog-empty = 新增規則之前至少選擇一項。
overlay-runtime-setting-current-value = 目前覆蓋：{ $value }
overlay-choice-clear-value = 清晰的價值
runtime-setting-choice-supported-model = 當前型號支援
overlay-permission-studio-delete-title = 刪除規則
overlay-permission-studio-delete-body = 刪除 { $kind }: { $value }
flash-permission-studio-no-add = 當前部分無法新增任何項目。
flash-permission-studio-no-delete = 當前部分中的任何項目都無法刪除。
flash-permission-studio-no-selection = 首先選擇一個項目。
flash-permission-studio-context-lost = 權限編輯器上下文遺失。重新開啟權限工作室並重試。
value-default = 預設
value-none = 無
value-clear = 清晰
value-path = 路徑
value-network = 網路
value-workspace = 工作區
value-external = 外部的
value-permission-filesystem = 檔案系統
value-permission-network = 網路
value-permission-tools = 工具
value-rule-count = { $count } 規則
value-custom = 客製化
value-internet = 網際網路
value-private = 私人的
value-loopback = 環回
value-name-count = { $count } 姓名
value-rule-set-count = { $count } 規則集
value-open = 打開
composer-prompt-history-title = 即時歷史記錄
overlay-commands-title = 命令面板
overlay-commands-prompt = 搜尋動作；需要文字的命令在編輯器中繼續
overlay-skill-studio-title = 管理技能
overlay-lineage-title = 分支歷史 [#{ $session }]
overlay-lineage-prompt = 探索目前分支樹並跳到祖先、兄弟或子會話
overlay-rewind-title = 回放會議 [#{ $session }]
overlay-rewind-prompt = 選擇要撤回的用戶訊息及其後的所有內容
overlay-picker-loading = 加載中...
overlay-picker-empty = 沒有匹配的項目
overlay-picker-footer = Tab 填入選定的標籤
session-model-context-window = { $value } ctx
session-model-max-output = 出 { $value }
provider-field-provider-id = 提供者 ID
provider-field-auth-mode = 認證模式
provider-field-auth-subtype = 驗證子類型
provider-field-auth-login-method = 認證登入方式
provider-field-start-auth = 開始認證
provider-field-continue-auth = 繼續驗證
provider-field-auth-details = 授權詳情
provider-field-base-url = 基本網址
provider-field-instance-url = 實例網址
provider-field-api-key-source = API金鑰來源
provider-field-api-key-value = API鍵值
provider-field-redirect-uri = 重定向URI
provider-field-callback-url = 回呼地址
provider-field-refresh-token = 刷新令牌
provider-field-access-token = 訪問令牌
provider-field-expires-at-ms = 過期時間（毫秒）
provider-field-account-id = 帳戶ID
provider-field-enterprise-domain = 企業域
provider-field-region = 地區
provider-field-profile = 公司簡介
provider-field-access-key-id = 存取密鑰 ID
provider-field-secret-access-key = 秘密存取密鑰
provider-field-session-token = 會話令牌
provider-field-service-key-env = 服務密鑰環境
provider-field-request-timeout = 請求超時（秒）
provider-field-connect-timeout = 連接逾時（秒）
provider-field-adapter-id = 適配器ID
provider-field-model-id = 型號編號
provider-model-field-model-id = 型號編號
provider-model-field-enabled = 啟用
provider-model-field-native-compaction = 原生壓縮
provider-model-field-agena-tool-mode = 工具模式（agena_tools.mode）
agena-tool-mode-provider-protocol-label = 提供者協議
agena-tool-mode-provider-protocol-detail = 透過提供者 API 的工具協定傳輸 Agena 管理的工具定義和呼叫。
agena-tool-mode-disabled-label = 殘障人士
agena-tool-mode-disabled-detail = 請勿向此模型公開 Agena 管理的或提供者本機的工具。
provider-model-field-display-name = 顯示名稱
provider-model-field-lifecycle = 生命週期
provider-model-field-context-window = 上下文視窗
provider-model-field-max-input = 最大輸入
provider-model-field-max-output = 最大輸出
provider-model-field-features = 特點
provider-model-field-input-modalities = 輸入方式
provider-model-field-output-modalities = 輸出方式
provider-model-field-thinking-modes = 思考模式
provider-model-field-speed-modes = 速度模式
provider-model-field-description = 描述
provider-model-enabled-detail = 該模型路由是否啟用。
provider-model-native-compaction-detail = 在返回 Agena 的文字摘要器之前，請嘗試該提供者的本機對話壓縮端點。
provider-model-lifecycle-detail = 模型生命週期價值。
provider-auth-mode-none-detail = 禁用提供者身份驗證元數據
provider-auth-mode-api-detail = API 風格的身份驗證，具有用於自訂 HTTP 端點、Cline API、GitLab 網關令牌或 Bedrock SigV4 的第二階段子類型
provider-auth-mode-credential-detail = 由本機頒發者解析的憑證支援的身份驗證，在身份驗證子類型欄位中選擇
provider-auth-kind-unset = 未設定
provider-auth-kind-none = 無
provider-auth-kind-api = 應用程式介面
provider-auth-kind-cline = cline_api
provider-auth-kind-gitlab = gitlab_api
provider-auth-kind-credential = 憑證
provider-auth-kind-credential-with-issuer = 憑證：{ $issuer }
provider-auth-kind-bedrock = bedrock_sigv4
provider-auth-subtype-custom-label = 客製化
provider-auth-subtype-custom-detail = 適用於 OpenAI 相容、Anthropic 或 Gemini HTTP 提供者的通用 API 金鑰 + 基本 URL 驗證
provider-auth-subtype-cline-api-detail = 修正了 Cline API 端點；只需要輸入API金鑰，模型發現使用Cline推薦的模型
provider-api-key-source-inline-detail = 將 API 金鑰內聯儲存在提供者配置中
provider-api-key-source-env-detail = 從環境變數中讀取 API 金鑰
provider-auth-subtype-gitlab-api-detail = 透過 openai 或人類適配器路由的 GitLab 令牌身份驗證
provider-auth-subtype-bedrock-detail = AWS Bedrock SigV4 簽名
provider-auth-login-kind-browser-label = 瀏覽器OAuth
provider-auth-login-kind-device-label = 設備碼登入
provider-auth-login-kind-browser-detail = 開啟授權URL，然後完成重定向回調。
provider-auth-login-kind-device-detail = 開啟一個簡短的驗證 URL，輸入裝置代碼，然後輪詢。
provider-issuer-openai-chatgpt-label = openai_chatgpt
provider-issuer-github-copilot-label = github_copilot
provider-issuer-gitlab-label = gitlab
provider-issuer-google-adc-label = Google_adc
provider-issuer-sap-ai-core-label = sap_ai_core
provider-issuer-openai-chatgpt-detail = OpenAI ChatGPT 憑證
provider-issuer-github-copilot-detail = GitHub Copilot 憑證
provider-issuer-gitlab-detail = 亞搏體育appGitLab OAuth憑證
provider-issuer-google-adc-detail = Google 應用程式預設憑證
provider-issuer-sap-ai-core-detail = SAP AI Core 服務金鑰驗證
provider-instance-url-gitlab-detail = GitLab.com 瀏覽器 OAuth 端點
provider-redirect-local-copy-detail = 用於複製/貼上 OAuth 重定向的本機回呼 URL
provider-region-choice-detail = AWS 區域
provider-service-key-env-detail = 預設 SAP AI Core 服務金鑰環境變數
overlay-model-catalog-field-model-id = 型號編號
overlay-model-catalog-field-display = 顯示
overlay-model-catalog-field-origin = 產地
overlay-model-catalog-field-lifecycle = 生命週期
overlay-model-catalog-field-dates = 棗子
overlay-model-catalog-field-limits = 限制
overlay-model-catalog-field-inputs = 輸入
overlay-model-catalog-field-output = 輸出
overlay-model-catalog-field-features = 特點
overlay-model-catalog-field-modes = 模式
overlay-model-catalog-field-defaults = 預設值
overlay-model-catalog-field-runtime = 運行時
overlay-model-catalog-field-pricing = 定價
overlay-model-catalog-field-source = 來源
overlay-model-catalog-limits = ctx { $context } · 在 { $input } · 出 { $output }
overlay-model-catalog-lifecycle-active = 活躍的
overlay-model-catalog-lifecycle-preview = 預覽
overlay-model-catalog-lifecycle-beta = 貝塔
overlay-model-catalog-lifecycle-alpha = 阿爾法
overlay-model-catalog-lifecycle-experimental = 實驗性的
overlay-model-catalog-lifecycle-deprecated = 已棄用
overlay-model-catalog-date-release = 釋放 { $value }
overlay-model-catalog-date-updated = 已更新 { $value }
overlay-model-catalog-date-cutoff = 截止 { $value }
overlay-model-catalog-default-thinking = 認為
overlay-model-catalog-default-speed = 速度
overlay-model-catalog-thinking-modes = 思考模式
overlay-model-catalog-speed-modes = 速度模式
overlay-model-catalog-default-verbosity = 冗長
overlay-model-catalog-default-temperature = 溫度
overlay-model-catalog-default-top-p = 頂部_p
overlay-model-catalog-default-top-k = 前k個
overlay-model-catalog-parallel-tools = 平行工具
overlay-model-catalog-supports-verbosity = 冗長
overlay-model-catalog-reasoning-interleaved = 交錯推理
overlay-model-catalog-reasoning-field = 推理場
overlay-model-catalog-open-weights = 開放重量
overlay-model-catalog-price-input = 在 { "$" }{ $value }/M
overlay-model-catalog-price-output = 出 { "$" }{ $value }/M
overlay-model-catalog-price-cache-read = 快取讀取 { "$" }{ $value }/M
overlay-model-catalog-price-cache-write = 快取寫入 { "$" }{ $value }/M
overlay-model-catalog-tier-count = { $count } 層
permission-rule-label-path = { $access } · { $path }
permission-rule-label-network = 網路·{ $target }
value-unset = 未設定
value-auto = 汽車
value-allow = 允許
value-ask = 問
value-deny = 否認
value-read = 讀
value-write = 寫
value-read-write = 讀寫
value-yes = 是的
value-no = 不
value-session = 會議
value-global = 全球
value-add = 添加
value-runtime-default = 運行時預設值
value-permission-rule-subject-tool = 工具
value-permission-rule-subject-path-access = 路徑訪問
value-permission-rule-subject-network-access = 網路存取
inline-fact-source = 來源
inline-fact-scope = 範圍
inline-fact-operator = 操作員
flash-permission-rule-saved = 已儲存的權限規則：{ $name }
flash-permission-rule-revoked = 已撤銷的權限規則：{ $name }
flash-permission-rule-context-lost = 權限規則工作室上下文遺失
flash-provider-studio-context-lost = 提供者配置上下文遺失
permission-rule-error-session-id-integer = 會話 ID 必須是整數
permission-rule-error-tool-name-required = 工具規則需要工具名稱
permission-rule-error-path-access-kind-required = 路徑規則需要path_access_kind
permission-rule-error-target-path-required = 路徑規則需要 target_path
permission-rule-error-network-target-required = 網路規則需要網路目標
permission-rule-error-session-id-required = 會話範圍需要會話 ID
flash-command-requires-session = 此操作需要一個開放的會話
flash-session-busy = 會話正忙
flash-provider-not-found = 找不到提供者：{ $provider }
flash-permission-approval-model-updated = 自動核准模型更新：{ $provider }/{ $model }
flash-provider-studio-adapter-required = 首先選擇一個適配器
flash-provider-studio-adapter-not-enabled = 新增型號之前檢查所選適配器
flash-provider-studio-adapter-unavailable = 目前的身份驗證模式不允許選擇此適配器
flash-provider-studio-model-required = 首先選擇列出的型號
flash-provider-studio-model-id-required = 型號 ID 為必填項
flash-provider-studio-no-auth-details = 當前身份驗證模式沒有可用的身份驗證詳細信息
flash-provider-studio-catalog-refreshed = 更新的模型目錄
flash-provider-studio-invalid-model-json = 無效模型 JSON：{ $error }
flash-provider-studio-live-listing-unavailable = 即時模型清單無法用於驗證 { $auth }
flash-provider-studio-draft-listing-unsupported = 草稿模型清單僅支援具有即時模型發現的適配器。不支援：{ $adapters }
flash-provider-studio-listing-auth-required = 列出適配器模型需要目前驗證/適配器對或現有已儲存的提供者的即時模型發現；目前驗證是 { $auth }
flash-provider-studio-invalid-auth-login-method = 無效的身份驗證登入方法
flash-provider-auth-openai-browser-started = OpenAI 瀏覽器身份驗證已啟動。開啟對話方塊中顯示的授權 URL，然後將重新導向的 URL 貼到回呼 URL 中並按 p。
flash-provider-auth-openai-device-started = OpenAI設備登入開始。開啟對話方塊中顯示的驗證 URL，輸入代碼 { $code }，然後按 p。
flash-provider-auth-copilot-device-started = 副駕駛設備登入開始。開啟對話方塊中顯示的驗證 URL，輸入代碼 { $code }，然後按 p。
flash-provider-auth-gitlab-browser-started = GitLab 瀏覽器驗證已啟動。開啟對話方塊中顯示的授權 URL，然後將重新導向的 URL 貼到回呼 URL 中並按 p。
flash-provider-auth-atomgit-browser-started = AtomGit 瀏覽器身份驗證已啟動。開啟對話方塊中顯示的授權 URL，完成登錄，然後按 p 進行輪詢。
flash-provider-auth-openai-captured = OpenAI OAuth 憑證會擷取到草稿中。
flash-provider-auth-openai-pending = OpenAI 設備登入仍待處理。完成驗證步驟，然後再按 p。
flash-provider-auth-copilot-pending = 副駕駛設備登入仍待處理。完成瀏覽器批准，然後再次按 p。
flash-provider-auth-copilot-captured = Copilot OAuth 憑證會擷取到草稿中。
flash-provider-auth-gitlab-captured = GitLab OAuth 憑證會擷取到草稿中。
flash-provider-auth-atomgit-pending = AtomGit 瀏覽器登入仍有待處理。完成瀏覽器流程，然後再按 p。
flash-provider-auth-atomgit-captured = AtomGit OAuth 憑證擷取到草稿中。
flash-provider-auth-error-unsupported = 目前的auth模式不支援互動式OAuth登入
flash-provider-auth-error-start-browser-first = 首先使用 Start Auth 或 o 啟動瀏覽器驗證
flash-provider-auth-error-start-device-first = 首先使用 Start Auth 或 o 啟動裝置驗證
flash-provider-auth-error-required-field = { $field } 是必要的
flash-provider-save-draft = 已儲存提供者 { $provider } 和適配器 { $adapter }。
flash-provider-save-adapter-matches = 使用 { $listed } 列出的模型保存了 { $provider }/{ $adapter } ； { $matched } 目錄匹配。
flash-provider-save-model = 已儲存{ $provider }/{ $adapter }/{ $model }。
flash-provider-save-configured-model = 已儲存組態模型 { $provider }/{ $adapter }/{ $model }。
flash-provider-delete-provider = 已刪除提供者 { $provider }。
flash-provider-delete-adapter = 刪除了配置的適配器 { $provider }/{ $adapter } 並刪除了 { $count } 模型。
flash-provider-delete-model = 刪除了配置的模型 { $provider }/{ $adapter }/{ $model }。
flash-provider-studio-adapter-delete-empty = 未選擇要刪除的適配器設定。
flash-provider-save-error-required-field = { $field } 是必要的
flash-provider-save-error-unsupported-adapters = auth { $auth } 不支援適配器：{ $adapters }；應為 { $supported } 之一
flash-provider-save-error-api-base-url = 使用 OpenAI 協定、Anthropic 或 Gemini 適配器時，api 驗證需要 base_url
flash-provider-save-error-gitlab-token = gitlab_api auth 需要 API 金鑰來源
flash-provider-save-error-credential-base-url = 憑證頒發者 `{ $issuer }` 需要 base_url
flash-provider-save-error-credential-service-key-env = 憑證頒發者 `{ $issuer }` 需要 service_key_env
flash-provider-save-error-bedrock-key-pair = bedrock_sigv4需要同時使用access_key_id和secret_access_key
flash-provider-save-error-select-model = 在保存提供者之前至少選擇一種模型
flash-provider-save-error-adapter-object = 提供者適配器 `{ $adapter }` 必須是 JSON 對象
flash-provider-save-error-model-object = 提供者模型配置必須是 JSON 對象
flash-provider-save-error-configured-adapter-object = 配置的提供者適配器設定必須是 JSON 對象
flash-provider-save-error-configured-models-object = 配置的提供者適配器模型必須是 JSON 對象
flash-provider-client-versions-refreshed = 更新的客戶端版本：Codex { $codex }、Claude { $claude }、Gemini { $gemini }
terminal-diagnostics-title = 終端診斷
terminal-diagnostics-eyebrow = 相容性和協議證據
terminal-diagnostics-footer = ↑/↓ 捲動 · c/y 複製報告 · Esc 關閉
terminal-diagnostics-tip = 產品標識和環境層是基於證據的；通用SSH無法證明真實的端點終端。
terminal-diagnostics-copied = 終端診斷已複製
terminal-diagnostics-unavailable = 終端診斷在此運作時不可用。
terminal-diagnostics-summary = 有證據支持的終端報告 · 終點置信度 { $confidence }
terminal-diagnostics-none = 無
terminal-diagnostics-unknown = 未知
terminal-diagnostics-unavailable-value = 不可用
terminal-diagnostics-term-unset = 術語未設定
terminal-diagnostics-section-identity = 身分
terminal-diagnostics-section-layers = 環境層
terminal-diagnostics-section-color = 顏色和外觀
terminal-diagnostics-section-protocols = 活動協議
terminal-diagnostics-section-providers = 提供者和集成
terminal-diagnostics-section-warnings = 警告
terminal-diagnostics-field-product = 產品展示
terminal-diagnostics-field-version = 版本
terminal-diagnostics-field-parsed-version = 解析版本
terminal-diagnostics-field-compatibility = 相容性
terminal-diagnostics-field-confidence = 信心
terminal-diagnostics-field-source = 選定的來源
terminal-diagnostics-field-evidence = 證據
terminal-diagnostics-field-conflicts = 衝突
terminal-diagnostics-color-configured = 配置模式
terminal-diagnostics-color-detected-background = 偵測到背景
terminal-diagnostics-color-detected-appearance = 檢測到的外觀
terminal-diagnostics-color-source = 檢測來源
terminal-diagnostics-color-refresh = 自動重新整理
terminal-diagnostics-color-generation = 外觀世代
terminal-diagnostics-color-effective-appearance = 有效的文字調色板
terminal-diagnostics-color-formula-foreground = 公式字形顏色
terminal-diagnostics-color-formula-background = 公式圖像背景
terminal-diagnostics-color-background-images = 背景圖片
terminal-diagnostics-color-mode-auto = 汽車
terminal-diagnostics-color-mode-dark = 強制黑暗
terminal-diagnostics-color-mode-light = 強制光
terminal-diagnostics-color-appearance-dark = 黑暗
terminal-diagnostics-color-appearance-light = 光
terminal-diagnostics-color-appearance-unknown = 未知
terminal-diagnostics-color-appearance-conservative = 保守的終端本機顏色（背景未知）
terminal-diagnostics-color-source-osc11 = OSC 11 終端響應
terminal-diagnostics-color-source-iterm-osc4 = iTerm2 OSC 4;-2 終端響應
terminal-diagnostics-color-source-colorfgbg = COLORFGBG 環境回退
terminal-diagnostics-color-source-term-background = TERM_BACKGROUND 環境回退
terminal-diagnostics-color-source-vscode-theme = VSCODE_THEME_KIND 環境回退
terminal-diagnostics-color-source-unavailable = 沒有可用的終端或環境證據
terminal-diagnostics-color-refresh-live = 关于焦点恢复和终端恢复；失败的刷新保留最后已知的颜色
terminal-diagnostics-color-refresh-startup-only = 仅限启动；终端没有回答可刷新颜色查询
terminal-diagnostics-color-formula-background-transparent = 透明；只有公式字形顏色遵循外觀
terminal-diagnostics-color-background-images-not-sampled = 未取样；透明公式像素保留终端背景或下面的背景图像
terminal-diagnostics-direct = 直接
terminal-diagnostics-direct-description = 未检测到 SSH、Mosh、多路复用器或 WSL 证据。
terminal-diagnostics-layer-description = 從 { $source } 偵測到。層順序和嵌套深度未知。
terminal-diagnostics-capability-description = 端點 = { $status } · 來源 = { $source } · 路徑 = { $path } · 提供者 = { $provider }
terminal-diagnostics-path-clear = 清晰
terminal-diagnostics-path-forced = 強制覆蓋
terminal-diagnostics-path-unverified = 未經驗證
terminal-diagnostics-path-blocked = 被阻止
terminal-diagnostics-provider-not-required = 不需要
terminal-diagnostics-provider-ready = 準備好了
terminal-diagnostics-provider-missing = 缺失或未實施
terminal-diagnostics-helper-missing = 未找到或不可執行。
terminal-diagnostics-helper-not-probed = 未探測到，因為端點未識別為 Kitty。
terminal-diagnostics-no-warnings = 未偵測到相容性警告。
terminal-diagnostics-protocol-alternate-screen = 備用螢幕
terminal-diagnostics-protocol-bracketed-paste = 括號貼上
terminal-diagnostics-protocol-focus = 焦點報道
terminal-diagnostics-protocol-mouse = 滑鼠捕捉
terminal-diagnostics-protocol-mouse-mode = 滑鼠線模式
terminal-diagnostics-protocol-mouse-events = 收到滑鼠事件
terminal-diagnostics-protocol-mouse-last = 最後一次滑鼠事件
terminal-diagnostics-mouse-mode-button-sgr = 使用 SGR 座標 (DECSET 1006) 進行按鈕事件追蹤 (DECSET 1002)
terminal-diagnostics-mouse-events-none = 沒有。端點終端尚未向Agena下發任何滑鼠事件；檢查其滑鼠報告和滾輪報告設定檔設定。
terminal-diagnostics-mouse-events-seen = { $count } 事件
terminal-diagnostics-mouse-last-none = 無
terminal-diagnostics-protocol-keyboard = 鍵盤消歧
terminal-diagnostics-protocol-key-events = 鍵盤事件類型
terminal-diagnostics-protocol-background = 後台查詢
terminal-diagnostics-protocol-native-clipboard = 本機剪貼簿
terminal-diagnostics-protocol-osc52-write = OSC 52 寫入
terminal-diagnostics-protocol-osc52-read = OSC 52 讀取
terminal-diagnostics-protocol-progress = OSC 9;4 進展
terminal-diagnostics-provider-kitty-clipboard = 小貓剪貼簿
terminal-diagnostics-provider-kitty-transfer = 貓咪轉運
terminal-diagnostics-provider-iterm-transfer = iTerm2 轉賬
terminal-diagnostics-provider-inline-images = 內嵌影像
terminal-diagnostics-provider-hyperlinks = 超連結
terminal-diagnostics-provider-sync-output = 同步輸出
terminal-diagnostics-status-confirmed = 已確認
terminal-diagnostics-status-forced = 強制覆蓋
terminal-diagnostics-status-profiled = 異型
terminal-diagnostics-status-unsupported = 不支援的
terminal-diagnostics-status-unknown = 未知
terminal-diagnostics-source-user = 使用者覆蓋
terminal-diagnostics-source-environment = 環境
terminal-diagnostics-source-helper = 輔助探針
terminal-diagnostics-source-terminal-query = 終端查詢
terminal-diagnostics-source-profile = 終端簡介
terminal-diagnostics-source-platform = 平台預設
terminal-diagnostics-source-conservative = 保守預設
terminal-diagnostics-source-terminfo = 術語資訊相容性
terminal-diagnostics-source-unknown = 未知
terminal-diagnostics-confidence-explicit = 明確的
terminal-diagnostics-confidence-strong = 強
terminal-diagnostics-confidence-compatibility = 僅相容性
terminal-diagnostics-confidence-unknown = 未知


# Plugin Workbench i18n completion
plugin-workbench-action-diff = 差異
plugin-workbench-action-refresh = 重新整理
plugin-workbench-action-remove-selected = 移除/重設所選項目
plugin-workbench-action-reset-all = 全部重設
plugin-workbench-action-restart = 重新啟動
plugin-workbench-action-save = 儲存
plugin-workbench-action-validate = 驗證
plugin-workbench-actions = 操作
plugin-workbench-authority-unavailable = 權限來源資料無法使用。
plugin-workbench-choices = 選項
plugin-workbench-close-footer = Esc 關閉
plugin-workbench-column-after = 修改後
plugin-workbench-column-args = 參數
plugin-workbench-column-arguments = 參數
plugin-workbench-column-before = 修改前
plugin-workbench-column-category = 類別
plugin-workbench-column-change = 變更
plugin-workbench-column-operation = 操作
plugin-workbench-column-description = 說明
plugin-workbench-column-field = 欄位
plugin-workbench-column-inputs = 輸入
plugin-workbench-column-message = 訊息
plugin-workbench-column-plugin = 外掛
plugin-workbench-column-section = 區段
plugin-workbench-column-severity = 嚴重程度
plugin-workbench-column-source = 來源
plugin-workbench-column-summary = 摘要
plugin-workbench-column-tool = 工具
plugin-workbench-column-version = 版本
plugin-workbench-column-visible-tool = 可見工具
plugin-workbench-operation-arguments = 參數：{$operation}
plugin-workbench-config = 設定
plugin-workbench-config-action = 操作
plugin-workbench-config-choose-shape = 選擇結構
plugin-workbench-config-choose-type = 選擇類型
plugin-workbench-config-default = 預設值
plugin-workbench-config-diff = 設定差異
plugin-workbench-config-dirty = 未儲存
plugin-workbench-config-drilldown-footer = 左右鍵切換儲存格 · 上下鍵切換列 · Enter 編輯 · Ctrl+D 移除/重設 · Esc 返回
plugin-workbench-config-saved = 已儲存
plugin-workbench-config-setting = 設定項目
plugin-workbench-config-state = 狀態
plugin-workbench-config-state-changed = 已修改
plugin-workbench-config-state-default = 預設
plugin-workbench-config-state-dirty = 未儲存
plugin-workbench-config-state-error = 錯誤
plugin-workbench-config-state-inactive = 未啟用
plugin-workbench-config-summary = {$status} · {$save_state}
plugin-workbench-config-title = {$plugin} / 設定
plugin-workbench-config-type = 類型
plugin-workbench-config-value = 值
plugin-workbench-config-view-summary = 生效設定 · {$changed} 個欄位已修改 · 目前儲存格：{$cell}
plugin-workbench-detail-footer = Tab/Shift+Tab 切換區段 · 上下鍵捲動 · Esc 返回
plugin-workbench-detail-tools-footer = Tab/Shift+Tab 切換區段 · 上下鍵選擇 · Enter 設定並執行 · Esc 返回
plugin-workbench-filter-all = 全部
plugin-workbench-filter-other = 其他
plugin-workbench-header-summary = 工具：{$tools}        操作：{$operations}        設定：{$config}
plugin-workbench-input-preview = 輸入預覽：{$tool}
plugin-workbench-last-result-failed = 最近結果 · {$tool} · 失敗
plugin-workbench-last-result-success = 最近結果 · {$tool} · 成功
plugin-workbench-list-footer = 輸入以搜尋 · 上下鍵選擇 · Enter 開啟 · Esc 關閉
plugin-workbench-list-summary = 搜尋外掛… {$query}        傳輸方式：{$transport}        設定：{$config}        顯示 {$shown}/{$total}
plugin-workbench-loading-actions = 正在載入操作…
plugin-workbench-loading-choices = 正在載入選項…
plugin-workbench-no-changes = 沒有變更
plugin-workbench-no-operations = 沒有操作。
plugin-workbench-no-config-section = 沒有設定區段。
plugin-workbench-no-editable-rows = 沒有可編輯列。
plugin-workbench-no-filter-matches = 沒有外掛符合目前篩選條件。
plugin-workbench-no-issues = 沒有問題
plugin-workbench-no-logs = 沒有日誌。
plugin-workbench-no-selection = 尚未選擇外掛。
plugin-workbench-no-structured-arguments = 沒有結構化參數。
plugin-workbench-no-tools = 沒有工具。
plugin-workbench-none = 無
plugin-workbench-none-declared = 未宣告
plugin-workbench-overview = 概覽
plugin-workbench-package-summary = 套件：{$package}
plugin-workbench-plugin = 外掛
plugin-workbench-plugin-capabilities = 外掛能力
plugin-workbench-plugins = 外掛
plugin-workbench-provenance = 來源：{$provenance}
plugin-workbench-sections = 區段
plugin-workbench-severity-error = 錯誤
plugin-workbench-severity-warning = 警告
plugin-workbench-status-invalid = 無效
plugin-workbench-status-issues = 問題
plugin-workbench-status-missing = 未設定
plugin-workbench-status-needs-restart = 需要重新啟動
plugin-workbench-status-runtime-issue = 執行階段問題
plugin-workbench-status-schema-missing = 缺少 Schema
plugin-workbench-status-valid = 有效
plugin-workbench-status-warning = 警告
plugin-workbench-summary = 查詢：{$query} · 傳輸方式 {$transport} · 設定 {$config} · 顯示 {$shown}/{$total}
plugin-workbench-tab-capabilities = 能力
plugin-workbench-tab-operations = 操作
plugin-workbench-tab-config = 設定
plugin-workbench-tab-diagnostics = 診斷
plugin-workbench-tab-logs = 日誌
plugin-workbench-tab-tools = 工具
plugin-workbench-tabs = 分頁
plugin-workbench-tags-summary = 標籤：{$tags}
plugin-workbench-tool-capabilities = 工具能力
plugin-workbench-tools-help = 使用上下鍵選擇工具。Enter 開啟由主機管理的 Schema 表單；Ctrl+S 驗證並執行。
plugin-workbench-transport = 傳輸方式
plugin-workbench-trust-level = 信任層級：{$level}
plugin-workbench-unavailable = 無法使用


# Plugin Workbench structured editor i18n completion
plugin-workbench-editor-also-matches = 同時符合：{$matches}
plugin-workbench-editor-array-action-help = Enter 開啟操作選單 · Ctrl+D 移除所選列
plugin-workbench-editor-array-preview = 設定…（{$count} 個項目）
plugin-workbench-editor-configure = 設定…
plugin-workbench-editor-format = 格式：{$format}
plugin-workbench-editor-generic-object = 通用物件編輯器
plugin-workbench-editor-index = 索引
plugin-workbench-editor-item = 項目 {$index}
plugin-workbench-editor-map = 映射編輯器
plugin-workbench-editor-no-fields = 沒有欄位。
plugin-workbench-editor-no-items = 沒有項目。
plugin-workbench-editor-object = 物件編輯器
plugin-workbench-editor-object-action-help = Enter 開啟操作選單 · 從「操作」儲存格新增欄位
plugin-workbench-editor-object-array = 物件陣列表格編輯器
plugin-workbench-editor-object-array-help = 編輯會在同一個結構化編輯器中開啟所選項目。
plugin-workbench-editor-object-preview = 設定…（{$count} 個欄位）
plugin-workbench-editor-preview = 預覽
plugin-workbench-editor-primitive-array = 基礎類型陣列編輯器
plugin-workbench-editor-readonly = 唯讀
plugin-workbench-editor-schema-missing = 缺少 Schema        基礎結構化編輯器
plugin-workbench-editor-shape = 結構
plugin-workbench-editor-suggestions = 建議值
plugin-workbench-editor-tuple = Tuple 編輯器
plugin-workbench-editor-type-summary = 類型：{$type}        路徑編輯器：結構化介面
plugin-workbench-field-state-available = 可設定
plugin-workbench-field-state-custom = 自訂
plugin-workbench-field-state-map-key = 映射鍵
plugin-workbench-field-state-missing = 缺少
plugin-workbench-field-state-optional = 選填
plugin-workbench-field-state-required = 必填
plugin-workbench-kind-all-of = allOf
plugin-workbench-kind-any-of = anyOf
plugin-workbench-kind-array = 陣列
plugin-workbench-kind-boolean = 布林值
plugin-workbench-kind-integer = 整數
plugin-workbench-kind-null = 空值
plugin-workbench-kind-number = 數值
plugin-workbench-kind-object = 物件
plugin-workbench-kind-one-of = oneOf
plugin-workbench-kind-string = 字串
plugin-workbench-kind-value = 值
