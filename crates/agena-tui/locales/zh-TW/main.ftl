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

transcript-header-lines = 行 {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = 搜尋={$query} ({$current}/{$total})
transcript-header-tail = 尾隨
transcript-header-loading = 載入中
transcript-header-loading-older = 正在載入更早訊息
transcript-header-busy = 忙碌
transcript-loading-older = 正在載入更早的訊息...
transcript-more-older = 還有更早的訊息。向上捲動或按 PageUp 繼續載入。
transcript-empty-session = 目前工作階段還沒有訊息。

no-session-selected = 尚未選擇工作階段。
no-session-selected-hint = 使用 /sessions 選擇工作階段，或直接在輸入區開始輸入以建立新工作階段。
composer-session-new = 新工作階段
composer-placeholder = 輸入給 Agena。游標在開頭時按上鍵查看歷史。/ 指令。Ctrl+O 附件。

status-global = / 向下搜尋 | ? 向上搜尋 | Ctrl+C 連按兩次離開
status-sessions = 工作階段：/sessions
status-transcript = 查看：i 進入插入 | j/k 捲動 | / 搜尋 | c 複製上一則 | y 複製
status-composer = 插入：Esc 返回查看 | Ctrl+Enter 立即送出 | Ctrl+J 換行 | 開頭按上鍵查看歷史 | Ctrl+Up 取回佇列訊息 | / 指令 | Ctrl+G 項目 | Ctrl+R 輸入 | Ctrl+L 權限

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
help-composer-line-10 = 游標位於輸入框開頭時按上鍵開啟歷史；Ctrl+Up 取回佇列訊息
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
overlay-user-input-reply-format = 回覆格式：question_id=value;other_id=value1,value2
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
