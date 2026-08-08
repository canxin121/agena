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
no-session-selected-hint = /sessions でセッションを選ぶか、入力欄に入力を始めて新しいセッションを作成してください。
composer-session-new = 新しいセッション
composer-placeholder = Agena へ入力。先頭で Up を押すと履歴。/ コマンド。Ctrl+O 添付。

status-global = / 下方向検索 | ? 上方向検索 | Ctrl+C 2回で終了
status-sessions = セッション: /sessions
status-transcript = VIEW: i で入力 | j/k スクロール | / 検索 | c 最後をコピー | y コピー
status-composer = INSERT: Esc で戻る | Ctrl+Enter 今すぐ送信 | Ctrl+J 改行 | 先頭で Up 履歴 | / コマンド | Ctrl+G アイテム | Ctrl+R 入力 | Ctrl+L 承認

help-title = ヘルプ
help-header = Agena TUI
help-section-sessions = セッション切替
help-sessions-line-1 = /sessions で検索できるセッション切替を開く
help-sessions-line-2 = Up/Down、PageUp/PageDown で選択移動
help-sessions-line-3 = Enter で選択したセッションを開く
help-section-transcript = トランスクリプトペイン
help-transcript-line-1 = i で INSERT に入り、j/k または矢印キーでスクロール
help-transcript-line-2 = Space / Shift+Space / Ctrl+B でページ移動
help-transcript-line-3 = Ctrl+D / Ctrl+U で半ページ移動
help-transcript-line-4 = 上端付近で PageUp を押すと古いメッセージを読み込む
help-transcript-line-5 = g/G で先頭または末尾へ移動
help-transcript-line-6 = / で下方向、? で上方向に検索し、n で同方向、N で逆方向に移動
help-transcript-line-7 = c で最後の assistant メッセージをコピー、y で全文コピー、Y で表示範囲をコピー
help-section-composer = 入力欄
help-composer-line-1 = Esc で VIEW に戻り、Enter で送信
help-composer-line-2 = Shift+Enter または Ctrl+J で改行
help-composer-line-3 = Ctrl+A/E/B/F/P/N で移動、Ctrl+Left/Right で単語単位移動
help-composer-line-4 = Ctrl+H/D/W/U/K/Y で shell や editor 風に編集
help-composer-line-5 = 行境界では Ctrl+A/E で前後の行に続けて移動可能
help-composer-line-6 = Ctrl+O でワークスペースファイルを検索して添付
help-composer-line-7 = Ctrl+E で $VISUAL/$EDITOR を開く
help-composer-line-8 = Ctrl+T でクリップボード画像を添付
help-composer-line-9 = 貼り付けたテキストは直接入力され、単一のファイルパスは添付になり、添付は原子的に扱われます
help-composer-line-10 = カーソルが入力欄の先頭にあるとき Up で履歴を開き、Ctrl+P で保留メッセージを編集、Ctrl+X でキャンセルします
help-section-actions = 操作
help-actions-line-1 = Ctrl+N でセッション作成、n/N で検索結果を移動
help-actions-line-2 = r ブロック中または保留中のセッションを続行、U 使用状況分析を開く
help-actions-line-3 = a/A/d/D で最初の保留中の権限リクエストに応答
help-actions-line-4 = Composer で Ctrl+R を押すと最初の保留中ユーザー入力を開く
help-actions-line-5 = マウスキャプチャは無効なので、端末本来の選択やコピーが使えます
help-actions-line-6 = Ctrl+C を2回押すと終了

overlay-session-search-title = セッション検索
overlay-session-search-prompt = セッションタイトルを検索
overlay-transcript-search-title = トランスクリプト検索
overlay-transcript-search-prompt = 読み込み済みメッセージ内を検索
overlay-line-footer = 入力して編集

overlay-attach-title = ファイルを添付
overlay-attach-prompt = パスまたは検索語を入力してください。Enter で選択中のファイルを添付します。
overlay-attach-no-match = 一致するファイルがありません
overlay-attach-matches = 一致結果
overlay-attach-footer = Tab で選択パスを入力

overlay-user-input-title = 保留中のユーザー入力
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = カスタム値を許可
overlay-user-input-reply-format = 返信形式: 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X でリクエストをキャンセル
overlay-user-input-footer = Ctrl+X でキャンセル

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
flash-message-interrupting = 実行中の処理を中断します - メッセージは次に送信されます

message-role-user = user
message-role-assistant = assistant
message-role-system = system

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed
message-state-policy-denied = blocked by permission policy
message-state-user-declined = declined by user
message-state-capability-unavailable = capability unavailable
message-state-tool-unavailable = tool unavailable

message-parts-not-loaded = {$count} 個のパートが未読み込みです
message-usage = 使用量: in={$input} out={$output} reasoning={$reasoning}
message-finish = finish: {$finish}
message-empty = （空メッセージ）
message-thinking = 思考: {$summary}
message-command-status = 状態: {$status}, exit={$exit}
message-file-changes = ファイル変更
message-file-changes-preview-one = 1 件のファイル: {$paths}
message-file-changes-preview-many = {$count} 件のファイル: {$paths}
message-file-changes-more = 残り {$count} 件
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
message-tool-cancelled = 中止: {$label}
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
attachment-kind-directory = フォルダ
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

permission-summary-allow-once = 1 回許可: {$reason}
permission-summary-allow-always = 常に許可: {$reason}
permission-summary-deny-once = 1 回拒否: {$reason}
permission-summary-deny-always = 常に拒否: {$reason}

failure-detail-message = メッセージ
failure-detail-code = エラーコード
failure-detail-category = カテゴリ
failure-detail-responsibility = 責任
failure-detail-impact = 影響
failure-detail-recovery = 復旧
failure-detail-retry = 再試行
failure-category-invalid-input = 入力が無効
failure-category-not-found = 見つかりません
failure-category-conflict = 競合
failure-category-permission-required = 権限が必要
failure-category-permission-denied = 権限が拒否されました
failure-category-authentication-required = 認証が必要
failure-category-rate-limited = レート制限
failure-category-quota-exceeded = クォータ超過
failure-category-timeout = タイムアウト
failure-category-dependency-unavailable = 依存関係が利用不可
failure-category-protocol-failure = プロトコルエラー
failure-category-data-corruption = データ整合性の問題
failure-category-internal = 内部エラー
failure-responsibility-caller = リクエスト
failure-responsibility-policy = ポリシー
failure-responsibility-dependency = 依存関係
failure-responsibility-system = システム
failure-impact-request-rejected = リクエスト拒否
failure-impact-operation-failed = 操作失敗
failure-impact-operation-paused = 操作一時停止
failure-impact-partial-success = 部分成功
failure-impact-background-task-failed = バックグラウンドタスク失敗
failure-impact-runtime-degraded = ランタイム低下
failure-impact-fatal-startup-failure = 致命的な起動失敗
failure-recovery-none = 自動復旧なし
failure-recovery-refresh = 更新
failure-recovery-reauthenticate = 再認証
failure-recovery-open-settings = 設定を開く
failure-recovery-request-permission = 権限を要求
failure-recovery-ask-user = ユーザーに確認
failure-recovery-retry = 再試行
failure-recovery-choose-alternative = 代替手段を選択
failure-recovery-restart-plugin = プラグインを再起動
failure-recovery-restart-runtime = ランタイムを再起動
failure-retry-never = 再試行しない
failure-retry-correct-input = 入力を修正して再試行
failure-retry-after-user-action = ユーザー操作後に再試行
failure-retry-after-refresh = 更新後に再試行
failure-retry-immediate-once = すぐに一度再試行
failure-retry-backoff = バックオフで再試行
failure-retry-use-alternative = 代替手段を使用
failure-retry-unknown = 不明
