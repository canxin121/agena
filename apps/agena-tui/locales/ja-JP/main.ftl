cli-about = Agena のターミナルチャットアプリ

pane-sessions = セッション
pane-sessions-search = セッション [{$query}]
pane-transcript = トランスクリプト
pane-messages = メッセージ
pane-composer = 入力欄 [{$session}]

session-meta = #{$id}  {$message_count} 件  {$updated}
session-running = 実行中
sessions-empty = セッションが見つかりません
sessions-loading-more = さらにセッションを読み込み中...
sessions-more = さらに読み込めるセッションがあります

transcript-header-lines = 行 {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = 検索={$query} ({$current}/{$total})
transcript-header-tail = 末尾追従
transcript-header-loading = 読み込み中
transcript-header-loading-older = 古いメッセージを読み込み中
transcript-header-busy = ビジー
transcript-loading-older = 古いメッセージを読み込み中...
transcript-more-older = さらに古いメッセージがあります。上にスクロールするか PageUp を押してください。
transcript-empty-session = このセッションにはまだメッセージがありません。

no-session-selected = セッションが選択されていません。
no-session-selected-hint = Alt+S でセッションを選ぶか、入力欄に入力を始めて新しいセッションを作成してください。
composer-session-new = 新しいセッション
composer-placeholder = Agena へ入力。Enter 送信。Alt+Up 履歴。/ コマンド。F3 添付。

status-global = Alt+S セッション | Alt+P コマンド | ? ヘルプ | q/Ctrl+C 終了
status-sessions = セッション: Alt+S 切替 | /sessions [検索] | /search [検索]
status-transcript = 記録: j/k スクロール | / 検索 | c 最後をコピー | y コピー | v ページャ
status-composer = 入力: Enter キュー/送信 | Ctrl+Enter 今すぐ送信 | Alt+Up/Down 履歴 | Shift+Enter 改行 | / コマンド | Tab チャット

help-title = ヘルプ
help-header = Agena TUI
help-section-sessions = セッション切替
help-sessions-line-1 = Alt+S で検索できるセッション切替を開く
help-sessions-line-2 = Up/Down、PageUp/PageDown で選択移動
help-sessions-line-3 = Enter で選択したセッションを開く
help-section-transcript = トランスクリプトペイン
help-transcript-line-1 = j/k または矢印キーでスクロール
help-transcript-line-2 = Space / Shift+Space / Ctrl+F / Ctrl+B でページ移動
help-transcript-line-3 = Ctrl+D / Ctrl+U で半ページ移動
help-transcript-line-4 = 上端付近で PageUp を押すと古いメッセージを読み込む
help-transcript-line-5 = g/G で先頭または末尾へ移動
help-transcript-line-6 = / または Ctrl+F で読み込み済み内容を検索し、n/N で結果を移動
help-transcript-line-7 = c で最後の assistant メッセージをコピー、y で全文コピー、Y で表示範囲をコピー
help-section-composer = 入力欄
help-composer-line-1 = Enter で送信
help-composer-line-2 = Alt/Shift+Enter または Ctrl+J で改行
help-composer-line-3 = Ctrl+A/E/B/F/P/N で移動、Alt+B/F または Alt/Ctrl+Left/Right で単語単位移動
help-composer-line-4 = Ctrl+H/D/W/U/K/Y で shell や editor 風に編集
help-composer-line-5 = 行境界では Ctrl+A/E で前後の行に続けて移動可能
help-composer-line-6 = F3、Ctrl+O、Alt+O でワークスペースファイルを検索して添付
help-composer-line-7 = F4 または Alt+E で $VISUAL/$EDITOR を開く
help-composer-line-8 = F6 または Alt+I でクリップボード画像を添付
help-composer-line-9 = 単一のファイルパス貼り付けは添付になり、大きな貼り付けはインラインプレースホルダになり、添付は原子的に扱われます
help-composer-line-10 = Alt+Up/Down で送信済みプロンプトを呼び出し、Alt+P でコマンドパレットを開きます
help-section-actions = 操作
help-actions-line-1 = n セッション作成
help-actions-line-2 = r ブロック中または保留中のセッションを続行
help-actions-line-3 = a/A/d/D で最初の保留中の権限リクエストに応答
help-actions-line-4 = u で最初の保留中のユーザー入力リクエストを開く
help-actions-line-5 = マウスキャプチャは無効なので、端末本来の選択やコピーが使えます
help-actions-line-6 = q または Ctrl+C で終了

overlay-session-search-title = セッション検索
overlay-session-search-prompt = セッションタイトルを検索
overlay-transcript-search-title = トランスクリプト検索
overlay-transcript-search-prompt = 読み込み済みメッセージ内を検索
overlay-line-footer = Enter で適用 | Esc で閉じる

overlay-attach-title = ファイルを添付
overlay-attach-prompt = パスまたは検索語を入力してください。Enter で選択中のファイルを添付します。
overlay-attach-no-match = 一致するファイルがありません
overlay-attach-matches = 一致結果
overlay-attach-footer = Enter で添付 | Tab で選択パスを入力 | Up/Down で移動 | Esc で閉じる

overlay-user-input-title = 保留中のユーザー入力
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = カスタム値を許可
overlay-user-input-reply-format = 返信形式: question_id=value;other_id=value1,value2
overlay-user-input-cancel-hint = Ctrl+D でリクエストをキャンセル
overlay-user-input-footer = Enter で送信 | Esc で閉じる | Ctrl+D でキャンセル

flash-terminal-event-error = 端末イベントエラー: {$error}
flash-created-session = セッションを作成しました {$title}
flash-permission-reply-sent = 権限への応答を送信しました: {$label}
flash-user-input-reply-sent = ユーザー入力への応答を送信しました
flash-large-paste-staged = 大きな貼り付けを入力欄に一時保存しました
flash-attached = {$path} を添付しました
flash-composer-updated = 外部エディタの内容で入力欄を更新しました
flash-prompt-history-empty = プロンプト履歴は空です
flash-prompt-history-items = プロンプト履歴を呼び出す前に、添付またはステージ済み貼り付けを消してください
flash-external-editor-failed = 外部エディタに失敗しました: {$error}
flash-clipboard-image-attached = クリップボード画像を添付しました: {$width}x{$height} {$format}
flash-clipboard-image-attach-failed = クリップボード画像の添付に失敗しました: {$error}
flash-no-loaded-transcript = コピーできる読み込み済み内容がありません
flash-copied-loaded-transcript = 読み込み済みトランスクリプトをクリップボードにコピーしました
flash-no-assistant-message = コピーできる assistant メッセージがありません
flash-no-assistant-message-text = 最後の assistant メッセージにコピーできる読み込み済みテキストがありません
flash-copied-assistant-message = 最後の assistant メッセージをクリップボードにコピーしました
flash-no-visible-transcript = コピーできる表示テキストがありません
flash-copied-visible-transcript = 表示中の内容をクリップボードにコピーしました
flash-clipboard-copy-failed = クリップボードへのコピーに失敗しました: {$error}

message-role-user = user
message-role-assistant = assistant
message-role-system = system

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed

message-parts-not-loaded = {$count} 個のパートが未読み込みです
message-usage = 使用量: in={$input} out={$output} reasoning={$reasoning}
message-finish = finish: {$finish}
message-empty = （空メッセージ）
message-thinking = 思考: {$summary}
message-command-status = 状態: {$status}, exit={$exit}
message-file-changes = ファイル変更
message-search = 検索: {$query}
message-todo-list = TODO リスト
message-error = エラー [{$code}]: {$message}
message-attachments = 添付
message-awaiting-user-input = ユーザー入力待ち: {$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = パート詳細を利用できません
message-tool-pending = 保留: {$label}
message-tool-running = 実行中: {$label}
message-tool-done = 完了: {$label}
message-tool-failed = 失敗: {$label}
message-tool-result-blocks = {$count} 個の結果ブロック

todo-status-pending = pending
todo-status-in-progress = in_progress
todo-status-completed = completed
todo-status-cancelled = cancelled

todo-priority-high = high
todo-priority-medium = medium
todo-priority-low = low

file-change-added = added
file-change-updated = updated
file-change-deleted = deleted

time-just-now = たった今
time-minutes-ago = {$count} 分前
time-hours-ago = {$count} 時間前
time-days-ago = {$count} 日前

session-default-title = 新しいセッション {$time}
session-default-base = 新しいセッション
session-fallback-title = セッション {$id}

user-input-error-empty = 返信は空にできません
user-input-error-invalid-segment = 無効な返信セグメント: {$segment}
user-input-error-unknown-question = 不明な質問 ID: {$question_id}
user-input-error-missing-answer = 質問 {$question_id} には少なくとも 1 つの回答が必要です
user-input-error-no-answers = 返信に回答が含まれていません

attachment-kind-image = image
attachment-kind-audio = audio
attachment-kind-video = video
attachment-kind-pdf = pdf
attachment-kind-file = file
attachment-generic = attachment
attachment-chip-image = {$kind}: {$filename} ({$width}x{$height}, {$size})
attachment-chip-other = {$kind}: {$filename} ({$size})
attachment-placeholder = [{$kind} {$filename}]

bytes-gb = {$value} GB
bytes-mb = {$value} MB
bytes-kb = {$value} KB
bytes-b = {$value} B

paste-label = {$count} 文字の貼り付け
paste-label-append = {$count} 文字の貼り付け、送信時に追加
paste-placeholder = [{$count} 文字の貼り付け]

permission-label-allow-once = 1 回許可
permission-label-allow-always = 常に許可
permission-label-deny-once = 1 回拒否
permission-label-deny-always = 常に拒否

permission-summary-pending = 権限待ち: {$reason}
permission-summary-allow-once = 1 回許可: {$reason}
permission-summary-allow-always = 常に許可: {$reason}
permission-summary-deny-once = 1 回拒否: {$reason}
permission-summary-deny-always = 常に拒否: {$reason}
