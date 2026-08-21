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
hub-title = セッションハブ
hub-action-create = 新規セッション
hub-action-list = セッション一覧
hub-action-refresh = 更新
hub-hint-move = 移動
hub-hint-focus = フォーカス
hub-hint-section = セクション
hub-hint-open = 開く
hub-hint-back = 戻る
hub-section-attention = 対応が必要
hub-section-running = 実行中
hub-section-recent = 最近
hub-empty-attention = 対応が必要なセッションはありません
hub-empty-running = 実行中のセッションはありません
hub-empty-recent = 最近のセッションはありません
hub-section-new = 新規セッション
hub-empty-new = 作成できるセッションはありません
hub-item-new = + 新規セッション
hub-item-new-detail = Enter で新しいセッションを作成
hub-action-search = 検索
hub-action-clear-search = 検索をクリア
hub-search-placeholder = 入力してセッションを絞り込み…
hub-search-active-empty = 入力して絞り込み…
hub-search-active = フィルター:{$query}
command-hub-summary = セッションハブを開く
command-background-summary = セッションハブに戻る;セッションは実行を継続
hub-empty = まだセッションがありません。Ctrl+N で作成してください。
context-help-context-hub = セッションハブ
context-help-summary-hub = 対応が必要なセッション、実行中、最近のセッションを表示し、新しいセッションを作成します。
context-help-key-create-session = 新しいセッションを作成します。
context-help-key-session-list = セッションの完全な一覧を開きます。

transcript-header-lines = 行 {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = 検索={$query} ({$current}/{$total})
transcript-header-tail = 末尾追従
transcript-header-loading = 読み込み中
transcript-header-loading-older = 古いメッセージを読み込み中
transcript-header-busy = ビジー
transcript-loading-older = 古いメッセージを読み込み中...
transcript-more-older = さらに古いメッセージがあります。上にスクロールするか PageUp を押してください。
transcript-empty-session = このセッションにはまだメッセージがありません。

session-state-creating = 作成中
session-state-ready = 最近完了
session-state-running = 実行中
session-state-awaiting-interaction = 入力待ち
session-state-interrupted = 中断
session-state-failed = 失敗

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
message-user-input-replied = ユーザー入力に回答済み：{$request_id}
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

## Settings Studio core locale coverage
## Long policy descriptions intentionally continue to use the verified English fallback.

permission-studio-new-rule-label = + 新しいルール

permission-studio-new-rule-value = ダウンロード

permission-studio-catalog-tags-title = ツールタグルールを追加する

permission-studio-catalog-names-title = ツールアクセスルールの追加

permission-studio-catalog-footer = 結果ダウン・スペーストグル・選択モード・Escキャンセル

permission-studio-catalog-tag-detail = {$count} 登録されたツールで使用

permission-studio-catalog-custom-label = + カスタムルール...

permission-studio-catalog-custom-search = カスタム新しい手動タグツール名

overlay-settings-title = 設定

overlay-settings-footer = Ctrl+R リフレッシュ ・ ←/→ パンの切り替え ・ タブ/シフト+ タブサイクルパン・↑/↓ 選択・入る・Escクローズ

overlay-settings-sections = セクション

overlay-settings-options = オプション

overlay-settings-group-core = コア

overlay-settings-group-application = アプリケーション

overlay-settings-group-session = セッション

overlay-settings-group-system = システム

overlay-settings-default-section-title = セクション

overlay-settings-empty-section = 選択したセクションはありません。

overlay-settings-empty-items = このセクションの設定はありません。

overlay-settings-empty-detail = セクションを選択し、それを検査または編集するオプションを選択します。

overlay-settings-detail-current = 現在の値: {$value}

overlay-settings-detail-path = パス: {$path}

overlay-settings-detail-action = この設定を開いたり編集したりします。

settings-detail-action-screen = この画面を開きます。

overlay-settings-edit-title = 編集 {$field}

overlay-settings-edit-file-value = ファイルオーバーライド: {$value}

overlay-settings-edit-effective-value = 有効な価値: {$value}

overlay-choice-clear-settings-detail = {$field} のファイルオーバーライドを削除します。

overlay-settings-section-plugins-label = プラグインとツール

overlay-settings-section-plugins-summary = プラグインの設定、ツール、ハーネス、診断

overlay-settings-section-providers-label = モデルとプロバイダー

overlay-settings-section-providers-summary = {$count} 設定されたプロバイダ

overlay-settings-section-model-catalog-label = モデルカタログ

overlay-settings-section-model-catalog-summary = {$count} エントリ

overlay-settings-section-permissions-label = 権限

overlay-settings-section-permissions-summary = {$count} 永続許可規則

overlay-settings-section-tracing-summary = ログフィルタと診断

overlay-settings-section-ui-label = 外観

overlay-settings-section-ui-summary = ローカルおよびインターフェイスの好み

overlay-settings-section-ui-description = 持続的な言語、色、グラフィックおよび主題の設定。

overlay-settings-section-runtime-session-label = ランタイムとセッション

overlay-settings-section-runtime-session-summary = プロバイダーのクライアントのアイデンティティとコンテクストのコンパクト化

settings-permission-global-label = グローバル権限

settings-permission-global-detail = すべてのセッションのベースライン。

settings-permission-workspace-label = ワークスペースの許可

settings-permission-workspace-detail = 現在のプロジェクトのためのオーバーライドレイヤー。

settings-permission-current-label = 現在のセッション許可

settings-permission-current-detail = 現在のセッションのみに適用されます。

settings-permission-effective-label = 有効なパーミッション

settings-permission-layer-global = グローバル

settings-permission-layer-workspace = ワークスペース

settings-permission-layer-session = セッション

settings-permission-layer-effective = よくある質問

settings-runtime-thinking-label = モードを考える

settings-runtime-thinking-description = 現在のセッションはモードオーバーライドを考える

settings-runtime-speed-label = 速度モード

settings-runtime-speed-description = 現在のセッション速度モードオーバーライド

settings-runtime-verbosity-label = ヴェルボシティ

settings-runtime-verbosity-description = 現在のセッションの動詞オーバーライド

settings-field-default-provider-label = デフォルトモデル

settings-field-permission-approval-model-label = 自動承認モデル

settings-field-ui-locale-label = 言語

settings-field-ui-locale-description = インターフェイス言語

settings-field-tui-color-scheme-label = ターミナル配色

settings-field-tui-theme-label = TUI プラグインテーマ

settings-field-tui-theme-description = オプションのプラグインが証明されたセマンティックカラーパレット

settings-choice-tui-color-scheme-auto = 端末の背景を自動的に検出する

settings-choice-tui-color-scheme-dark = 暗いターミナルの背景のための色を最大限に活用して下さい

settings-choice-tui-color-scheme-light = 軽いターミナルの背景のための色を最大限に活用して下さい

settings-field-tui-graphics-label = リッチターミナルグラフィックス

settings-choice-tui-graphics-auto = ネイティブグラフィックスを自動的に交渉し、安全にUnicode(推奨)に戻る

settings-choice-tui-graphics-native = エキスパート構成のターミナルパスのためのネイティブグラフィックスの交渉を強制する

settings-choice-tui-graphics-unicode = ネイティブなグラフィックを無効化し、決定的なUnicode/textのレンダリングを使用する

settings-field-activity-default-expanded-label = アクティビティをデフォルトで展開

settings-field-activity-kind-description = このアクティビティのデフォルト拡張状態です。

settings-field-activity-tool-label = ツールのデフォルト拡張

settings-field-activity-tool-description = この正確なツールのデフォルトの拡張状態。

settings-activity-kind-reasoning-label = フィードバック

settings-activity-kind-operation-label = ツール操作

settings-activity-kind-operation-description = ツールの呼び出しと結果。

settings-activity-kind-resource-label = リソース

settings-activity-kind-resource-description = 添付ファイルおよびその他のリソースコンテンツ。

settings-activity-kind-skill_reference-label = スキルリファレンス

settings-activity-kind-skill_reference-description = 回答に使用するスキルへの参照。

settings-activity-kind-interaction-label = インタラクション

settings-activity-kind-interaction-description = ユーザーの入力要求および相互プロンプト。

settings-activity-kind-hook-label = ホック

settings-activity-kind-hook-description = セッションホックランとライフサイクルイベント。

settings-activity-kind-error-label = エラー

settings-activity-kind-error-description = 障害のある操作と端末の故障。

settings-activity-kind-notice-label = お知らせ

settings-activity-kind-notice-description = 背景の通知と情報列。

settings-activity-kind-text-label = テキスト

settings-activity-kind-text-description = テキストとテキストのアーティファクトのコンテンツをプレーンします。

settings-field-tracing-filter-label = アプリケーションログレベル

settings-field-tracing-filter-description = デフォルトトレースログレベル

settings-field-tracing-database-label = データベースログレベル

settings-field-tracing-database-description = データベースのトレースログレベル

settings-field-tracing-adapter-label = アダプターログレベル

settings-field-tracing-adapter-description = プロバイダのアダプターのトレースのログ・レベル

settings-config-open-file-detail = このパスでagena.jsonを開く

settings-source-unset = セットなし

settings-source-configured = 構成: {$value}

settings-source-effective = 有効: {$value}

settings-source-file-effective = ファイル: {$file} / 有効: {$effective}

settings-source-file-found = {$path} (創設者)

settings-source-file-missing = {$path} (作成します)

settings-source-row-config-file = ファイルの設定

settings-source-row-workspace-config-file = ワークスペースの設定ファイル

settings-source-row-file-value = ファイル値

settings-source-row-workspace-value = ワークスペースの価値

settings-source-row-effective-value = 有効な価値

settings-source-row-write-target = 書き込む

settings-source-row-layers = アクティブレイヤー

settings-source-current-session = 現在のセッションのランタイムデータ

settings-source-current-session-runtime = 現在のセッション実行オプション

settings-detail-values-heading = バリュー

settings-detail-sources-heading = ソース

settings-detail-action-readonly = 読み取り専用の効果的なビューを開きます。

settings-detail-action-file = バックアップ設定ファイルを開きます。

settings-harness-browser-label = ブラウザハーネス

settings-harness-shell-label = シェルハーネス

settings-harness-editor-label = エディターハーネス

settings-field-parse-bool = {$field} は true/false か on/off のような boolean を期待します

settings-field-parse-integer = {$field} は署名されていない整数値が期待されます

settings-field-parse-float = {$field} 数値の値が期待される

settings-choice-adapter-fallback = アダプター

settings-choice-default-provider-detail = {$adapter}/{$model}

settings-plugin-workbench-label = プラグイン設定ワークベンチ

settings-mcp-server-label = Agena MCP サーバー

settings-mcp-server-value = 有効/無効化

settings-mcp-server-enabled = 対応可能

settings-mcp-server-disabled = バリアフリー

settings-mcp-status-unavailable = ステータス未利用可能

settings-mcp-ready = 新着情報

settings-mcp-needs-attention = 必要性の注意

settings-mcp-auth-label = MCP 認証

settings-mcp-auth-none = 匿名: すべての露出された用具

settings-mcp-auth-oauth = 完全な OAuth

settings-mcp-auth-mixed = 混合: 公共の発見、per-tool OAuth

settings-mcp-anonymous-access-label = 混合認証の匿名ツールアクセス

settings-mcp-anonymous-access-none = なし(推奨)

settings-mcp-anonymous-access-read-only = 許可契約読み取り専用ツール

settings-mcp-registration-label = 会員登録

settings-mcp-pkce-label = ピクチャー

settings-mcp-client-registration-label = OAuth クライアント登録

settings-mcp-client-registration-cimd = CIMDのみ(推奨)

settings-mcp-client-registration-dcr = CIMD+ダイナミッククライアント登録

settings-mcp-public-url-label = 公開 MCP URL

settings-mcp-public-url-value = 編集

settings-mcp-public-url-auto = リスナーローカルフォールバック

settings-mcp-oauth-issuer-label = OAuth 発行者 URL

settings-mcp-oauth-issuer-derived = MCP リソースの起源から得られる

settings-mcp-oauth-password-label = MCP OAuth パスワード

settings-mcp-oauth-password-value = セットまたは取り替えて下さい

settings-mcp-oauth-password-configured = MCP固有のパスワードの設定

settings-mcp-oauth-password-ui-fallback = UIパスワードフォールバックを使用する

settings-mcp-oauth-password-not-configured = 設定されていない

settings-mcp-oauth-password-clear-label = MCP OAuthパスワードをクリアする

settings-field-runtime-codex-version-label = Codex クライアントバージョン

settings-field-runtime-claude-version-label = クロード コード バージョン

settings-field-runtime-gemini-version-label = Gemini CLI バージョン

settings-field-session-compaction-auto-label = 自動圧縮

settings-field-session-compaction-reserved-tokens-label = 圧縮予約トークン

settings-client-versions-refresh-label = クライアントバージョンをリフレッシュ

settings-client-versions-refresh-value = fetch 最新の

settings-client-versions-entry-label = プロバイダー クライアント バージョン

settings-client-versions-entry-value = コーデックス・クラウド・ジェミニ

settings-client-versions-section-label = クライアントバージョン

settings-client-versions-section-summary = Runtime ID バージョン

settings-provider-workbench-label = プロバイダーリスト

settings-provider-workbench-value = {$count} プロバイダー

settings-provider-default-mode-inherit-detail = このモードのモデル/provider デフォルトを使用してください。

settings-provider-new-label = + 新しいプロバイダー

settings-provider-existing-detail = {$count} 構成されるアダプター

settings-model-catalog-open-label = オープンモデルカタログ

settings-files-open-config-label = oldna.json を開きます。

settings-files-open-config-present = プレゼント

settings-files-open-config-create = オープンソース

permission-studio-field-path-workspace = パスワークスペースデフォルト

permission-studio-field-path-external = パス 外部デフォルト

permission-studio-field-path-rules = パスルール

permission-studio-field-network-defaults = ネットワークデフォルト

permission-studio-field-network-rules = ネットワークルール

permission-studio-field-tool-names = ツール名

permission-studio-field-tool-rules = ツールルール

permission-studio-field-prompt-json = {$field} の JSON を入力します。 このオーバーライドをクリアするためにエディタを空にします。

permission-studio-detail-override = オーバーライド

permission-studio-detail-effective = よくある質問

permission-studio-detail-override-inline = オーバーライド {$value}

permission-studio-detail-effective-inline = 有効な {$value}

permission-studio-detail-read-only = この許可文書は こちら でのみ読み込みます。

permission-studio-detail-mode-editable = このフィールドにモードピッカーを開きます。

permission-studio-detail-text-editable = この単一キーかパターンを編集して下さい。

permission-studio-detail-remove-hint = この項目をすぐに削除します。

permission-studio-detail-navigate-hint = このセクションを開きます。

permission-studio-overview-target = ターゲット

permission-studio-overview-source = ソース

permission-studio-overview-scope = スコープ

permission-studio-overview-override = オーバーライド

permission-studio-overview-effective = よくある質問

permission-studio-section-workspace = ワークスペース

permission-studio-section-external = 外部リンク

permission-studio-section-rules = ルールルール

permission-studio-section-defaults = デフォルト

permission-studio-source-global = グローバル

permission-studio-source-workspace = ワークスペース

permission-studio-source-session = セッション

permission-studio-source-effective = 有効期間

permission-studio-settings-override = オーバーライド {$value}

permission-studio-settings-effective = 有効な {$value}

permission-studio-mode-read = 読みます {$value}

permission-studio-mode-write = {$value} を書く

permission-studio-network-default = {$label} {$value}

permission-studio-page-overview = プロフィール

permission-studio-page-path = パス

permission-studio-page-path-defaults = ファイルシステム / デフォルト領域

permission-studio-page-path-rules = ファイルシステム / パスルール

permission-studio-page-network = ネットワーク

permission-studio-page-network-zones = ネットワーク / ネットワーク領域

permission-studio-page-network-rules = ネットワーク / ドメインルール

permission-studio-page-tools = ツール

permission-studio-page-tool-tags = ツールアクセス/タグのルール

permission-studio-page-tool-names = ツールアクセス / 名前ルール

permission-studio-page-tool-command-rules = ツールアクセス / コマンドルール

permission-studio-page-names = お名前 (必須)

permission-studio-page-tool-rules = ツールルール

permission-studio-nav-overview = プロフィール

permission-studio-nav-filesystem = ファイルシステム

permission-studio-nav-default-zones = デフォルト領域

permission-studio-nav-path-rules = パスルール

permission-studio-nav-network = ネットワーク

permission-studio-nav-network-zones = ネットワーク領域

permission-studio-nav-domain-rules = ドメインルール

permission-studio-nav-tool-access = ツールアクセス

permission-studio-nav-name-rules = 名前ルール

permission-studio-nav-command-rules = コマンドルール

permission-studio-path-workspace-read = ワークスペース読み取り

permission-studio-path-workspace-write = ワークスペース書き込み

permission-studio-path-external-read = 外部読み取り

permission-studio-path-external-write = 外部書き込み

permission-studio-path-rule-read = モードを読む

permission-studio-path-rule-write = モードを書く

permission-studio-network-internet = インターネット

permission-studio-network-private = プライベート

permission-studio-network-loopback = ループバック

permission-studio-tool-default = ツールのデフォルト

permission-studio-tool-default-summary = デフォルト {$value}

permission-studio-add-path-rule = パスルールを追加

permission-studio-add-network-rule = ネットワークターゲットの追加

permission-studio-add-name = 名前を追加

permission-studio-add-tool-rule = ツールルールの追加

permission-studio-rule-key = キーキー

permission-studio-rule-pattern = パターン

permission-studio-rule-target = ターゲット

permission-studio-rule-mode = モード

permission-studio-tool-rule-fallback = フォールバックモード

permission-studio-error-empty-value = {$field} 空にすることはできません。

overlay-providers-title = プロバイダー

overlay-providers-prompt = デフォルトモデルを使用するプロバイダを選ぶ

overlay-provider-list-title = プロバイダーリスト

overlay-provider-list-prompt = 構成されたプロバイダを検索

overlay-provider-list-footer = プロバイダーまたは既存のプロバイダーを選択し、Enterキーを押します。

overlay-provider-list-create-label = + 新しいプロバイダー

overlay-provider-list-row-detail-no-model = {$adapter} · {$count} 設定されたアダプター

overlay-provider-studio-title = プロバイダー Config

overlay-provider-studio-header = プロバイダー Config

overlay-provider-studio-footer = タブ/シフト+タブパネル・矢印選択・スペーストグル・入力編集・Ctrl+D削除選択・Ctrl+Rリフレッシュ・Ctrl+Nモデル追加・Ctrl+N 保存アダプタ・Ctrl+S保存プロバイダ・Escクローズ

overlay-provider-studio-providers = プロバイダー

overlay-provider-studio-draft = ドラフト

overlay-provider-studio-adapters = アダプター

overlay-provider-studio-models = モデル

overlay-provider-studio-catalog = モデル カタログ

overlay-provider-studio-detail = 詳細を見る

overlay-provider-studio-adapter-models-empty = アダプターを選択し、ライブモデルをリスト

overlay-provider-studio-models-empty = 利用可能なアダプターモデルはありません

overlay-provider-studio-catalog-empty = このクエリに一致するカタログエントリはありません

overlay-provider-studio-new-provider-detail = 空のプロバイダーの草案

overlay-provider-studio-provider-row-detail-no-model = {$adapter} · {$count} 設定されたアダプター

overlay-provider-studio-model-count = {$count} モデル

overlay-provider-studio-loaded = ロード

overlay-provider-studio-error = エラー

overlay-provider-studio-configured = 仕様

overlay-provider-studio-live-list = ライブリスト

overlay-provider-studio-not-listed = リストされていない

overlay-provider-studio-not-supported = 現在のauth契約でサポートされていない

overlay-provider-studio-edit-title = 編集フィールド

overlay-provider-studio-edit-prompt = 更新 {$field}

overlay-provider-studio-edit-footer = 編集するタイプ

overlay-provider-studio-model-edit-footer = Ctrl+S 保存モデル config

overlay-provider-studio-model-json-title = モデル Config・{$adapter}/{$model}

overlay-provider-studio-model-json-prompt = 持続的なプロバイダモデルJSONを編集します。

overlay-provider-studio-model-title = モデル・{$adapter}/{$model}

overlay-provider-studio-model-footer = 矢印選択・編集・Ctrl+S保存・Ctrl+D削除・Escバック入力

overlay-provider-delete-title = プロバイダーの削除

overlay-provider-delete-adapter-title = アダプターの削除

overlay-provider-delete-model-title = モデルを削除

overlay-provider-studio-model-edit-title = モデルフィールドの編集

overlay-provider-studio-model-field-prompt = 更新 {$field}

overlay-provider-studio-new-model-title = モデルを追加

overlay-provider-studio-edit-auth-mode-prompt = アップデート authモード(none | api | 認証)

overlay-provider-studio-edit-auth-subtype-prompt = アップデート auth サブタイプ (api: custom | cline api | gitlab api | gitlab api | sap ai core )

overlay-provider-studio-edit-auth-login-method-prompt = authのログイン方法の更新(デバイス | ブラウザ)

provider-studio-auth-status-pending = ペンディング

provider-studio-auth-status-unset = パスワード

provider-studio-auth-status-none = なし

provider-studio-auth-status-select-subtype = サブタイプを選択

provider-studio-auth-status-select-issuer = サブタイプを選択

provider-studio-auth-status-configured = 仕様

provider-studio-auth-status-partial = 部分的な

provider-studio-summary-env = ログイン

provider-studio-summary-callback = コールバック

provider-studio-summary-redirect = リダイレクト

provider-studio-summary-account = パスワード

provider-studio-summary-name = お名前 (必須)

provider-studio-summary-user = ユーザー

provider-studio-summary-email = 電子メール

provider-studio-summary-profile = プロフィール

provider-studio-summary-region = エリア

provider-studio-summary-code = コードコード

provider-studio-summary-state = 状態 {$state}

provider-studio-summary-tokens-set = トークンセット

provider-studio-summary-keys-set = キーセット

provider-studio-summary-set-field = セット {$field}

provider-studio-summary-review-fields = auth フィールドのレビュー

provider-studio-summary-start-browser = ブラウザ OAuth を起動する

provider-studio-summary-restart-browser = ブラウザーを再起動 OAuth

provider-studio-summary-open-authorize = URL の認証

provider-studio-summary-start-device = デバイスログインを開始する

provider-studio-summary-restart-device = デバイスのログインを再起動する

provider-studio-summary-open-verify = 認証URLを開く

provider-studio-summary-finish-callback = 終了コールバック交換

provider-studio-summary-poll-every = すべての {$seconds} を投票する

provider-studio-summary-paste-callback = コールバックURLを貼り付ける

provider-studio-summary-poll-now = 今すぐ投票する

provider-studio-summary-start-auth-first = 最初にauthを始めて下さい

provider-studio-summary-poll-browser = pollブラウザ結果

provider-studio-auth-openai-ready = ブラウザ OAuth が準備完了です。 下記のURLを認証します。

provider-studio-auth-openai-device-ready = OpenAIデバイスログインの準備ができました。 下記の認証URLを開き、{$code}を入力してください。

provider-studio-auth-authorize = 認証 {$url}

provider-studio-auth-redirect = リダイレクト {$url}

provider-studio-auth-paste-callback = リダイレクトURLをコールバックURLに貼り付け、p・state {$state} を押します。

provider-studio-auth-copilot-ready = デバイスログインが準備完了です。 下記の認証URLを開き、{$code}を入力してください。

provider-studio-auth-verify = {$url} を確かめて下さい

provider-studio-auth-poll = pを押して投票する · すべての {$seconds}s

provider-studio-auth-gitlab-ready = GitLabブラウザOAuthが準備完了です。 下記のURLを認証します。

provider-studio-auth-atomgit-ready = AtomGit ブラウザセッションの準備・ URL の承認は下記になります。

provider-studio-auth-finish-browser = ブラウザフローを終了し、p・state {$state} を押します。

flash-settings-updated = 更新 {$path}

flash-settings-cleared = クリア {$path}

flash-provider-save-error-settings-object = 既存のプロバイダの設定は JSON オブジェクトでなければなりません

command-settings-summary = モデル、パーミッション、プラグイン、ランタイム、セッション、インターフェイス、および診断用の統一された設定ワークベンチを開きます。

settings-mcp-public-url-updated = エイジナMCP公開URLを更新しました

settings-mcp-oauth-issuer-updated = エイジナ MCP OAuth 発行者 URL 更新

settings-mcp-oauth-password-updated = エイジナ MCP OAuth パスワードの更新

settings-mcp-server-enabled-flash = エイジナMCPサーバーが有効

settings-mcp-server-disabled-flash = エイジナ MCP サーバー 無効

settings-mcp-auth-mode-updated = エイジナ MCP 認証モードを {$mode} に設定

settings-mcp-anonymous-access-updated = エイジナ MCP 匿名ツール {$policy}

settings-mcp-client-registration-updated = エイジナ MCP クライアントの登録を {$policy} に設定

settings-mcp-oauth-password-cleared = エイジナ MCP OAuth パスワードクリア

permission-studio-command-pattern-title = {$tool_name} コマンドパターン

settings-tool-api-list-description = 実行ツールを実行します。

settings-tool-api-search-description = 実行ツールを検索します。

settings-tool-api-help-description = 実行ツール契約を点検します。

settings-tool-api-tags-description = 実行ツールタグをリストします。

settings-tool-api-call-description = 実行ツールを呼び出します。

settings-tool-api-plugins-list-description = ツールプラグインを列挙します。

settings-tool-api-plugins-search-description = ツールプラグインを検索します。

settings-tool-api-plugins-tags-description = ツールプラグインタグをリストします。

permission-studio-command-pattern-help = シェルコマンドの glob パターンを入力してください（例: `git status` または `git push *`）。

permission-studio-rename-unsupported = この項目の名前は変更できません。削除して作り直してください。

# Settings, provider, permission, catalog, MCP, and diagnostics completion
overlay-editor-footer-single-line = 編集する文字を入力してください
overlay-editor-footer-multiline = Ctrl+S 保存
context-help-title = コンテキストヘルプ
context-help-eyebrow = 現在のインターフェース
context-help-footer = ↑/↓スクロール・EscまたはCtrl+H閉じる
context-help-global-hint = Ctrl+H ヘルプ
context-help-context-composer-items = 作曲家アイテム
context-help-context-suggestions = 提案
context-help-context-usage = 使用状況ダッシュボード
context-help-context-plan-viewer = プランビューア
context-help-context-user-input = ユーザー入力リクエスト
context-help-context-plugin-list = プラグインワークベンチ・リスト
context-help-context-plugin-detail = プラグインワークベンチ · 詳細
context-help-context-plugin-config = プラグインワークベンチ・設定
context-help-context-plugin-actions = プラグイン設定・アクション
context-help-context-plugin-selection = プラグイン設定・選択
context-help-context-plugin-drilldown = プラグイン設定・ドリルダウン
context-help-context-plugin-diff = プラグイン設定・差分
context-help-key-delete = 選択した項目を削除します。
context-help-key-plugin-restart = サポートされている場合は、選択したプラグインを再起動します。
overlay-permission-title = 許可リクエスト
overlay-permission-details-title = 詳細
overlay-permission-action-tool = ツール: { $tool }
overlay-permission-action-path = パス { $access }: { $path }
overlay-permission-action-network = ネットワーク: { $target }
overlay-permission-field-tool = ツール
overlay-permission-field-target = コマンドまたはターゲット
overlay-permission-field-access = アクセス
overlay-permission-field-path = パス
overlay-permission-field-workspace = ワークスペース
overlay-permission-field-network = URLまたはネットワークターゲット
overlay-permission-field-host = ホスト
overlay-permission-field-reason = なぜ承認が必要なのか
overlay-permission-detail-request-id = リクエストID
overlay-permission-detail-source = ポリシーソース
overlay-permission-detail-scope = 要求された範囲
overlay-permission-detail-operator = リクエスト者
overlay-permission-detail-trace = 決定トレース
overlay-permission-summary-more-approvals = このツール呼び出しで { $count } 個の追加アクションも承認します
overlay-permission-detail-requested-actions = についても承認を求めています
overlay-permission-detail-related-actions = この通話ではすでに許可されています
overlay-permission-choice-auto-approve = 自動承認…
overlay-permission-rule-workbench-title = 許可ルール
overlay-permission-rule-studio-footer = 矢印で選択 · 編集を入力 · Ctrl+O 選択したパスを参照 · Ctrl+S 保存 · Ctrl+D 取り消し · Esc 閉じる
overlay-permission-rule-studio-footer-return = 矢印 選択 · Enter edit · Ctrl+O 選択したパスを参照 · Ctrl+S 保存 · Ctrl+D 取り消し · Esc 権限要求に戻る
flash-permission-rule-browse-path-selection = 参照する前に、ターゲット パスまたはワークスペース ルートを選択します。
overlay-permission-rule-choice-subject-title = 件名の種類を選択してください
overlay-permission-rule-choice-subject-prompt = ルールのサブジェクトのタイプを選択します。
overlay-permission-rule-choice-subject-tool-detail = ツールまたはランタイムツールと一致する
overlay-permission-rule-choice-subject-path-access-detail = ファイルシステムアクセスと一致する
overlay-permission-rule-choice-subject-network-access-detail = ネットワークアクセスに一致する
overlay-permission-rule-choice-access-title = パスのアクセス種類を選択してください
overlay-permission-rule-choice-access-prompt = ファイルシステムのアクセスモードを選択します。
overlay-permission-rule-choice-access-read-detail = ファイルの読み取りのみを許可する
overlay-permission-rule-choice-access-write-detail = ファイルの書き込みのみを許可する
overlay-permission-rule-choice-access-read-write-detail = 読み取りと書き込みの両方を許可する
overlay-permission-rule-choice-scope-title = ルールの範囲の選択
overlay-permission-rule-choice-scope-prompt = ルールをどの程度の範囲で維持するかを選択します。
overlay-permission-rule-choice-scope-session-detail = このセッションのみ
overlay-permission-rule-choice-scope-workspace-detail = このワークスペース内のすべてのセッション
overlay-permission-rule-choice-scope-global-detail = すべてのワークスペース
overlay-permission-rule-choice-mode-title = ルールモードの選択
overlay-permission-rule-choice-mode-prompt = 許可、質問、または拒否を選択します。
overlay-permission-rule-choice-mode-allow-detail = 一致するアクションを常に許可する
overlay-permission-rule-choice-mode-auto-detail = 構成された承認モデルに決定させます。利用できない場合はプロンプトに戻る
overlay-permission-rule-choice-mode-ask-detail = 一致するアクションを許可する前にプロンプトを表示する
overlay-permission-rule-choice-mode-deny-detail = 一致するアクションを常に拒否します
overlay-permission-rule-editor-footer = 編集する文字を入力してください
overlay-permission-rule-editor-tool-name-title = ツール名の編集
overlay-permission-rule-editor-tool-name-prompt = 正確なツール名を入力します。
overlay-permission-rule-editor-qualifier-title = 修飾子の編集
overlay-permission-rule-editor-qualifier-prompt = オプションの修飾子を入力するか、空のままにします。
overlay-permission-rule-editor-workspace-root-title = ワークスペースルートの編集
overlay-permission-rule-editor-workspace-root-prompt = オプションの workspace_root ディレクトリを入力します。
overlay-permission-rule-editor-target-path-title = ターゲットパスの編集
overlay-permission-rule-editor-target-path-prompt = ターゲットのパスまたはパターンを入力します。
overlay-permission-rule-editor-network-target-title = ネットワークターゲットの編集
overlay-permission-rule-editor-network-target-prompt = ホスト、ホスト:ポート、または URL を入力します。
overlay-permission-rule-editor-session-id-title = セッションIDの編集
overlay-permission-rule-editor-session-id-prompt = ターゲットのセッション ID を入力します。
overlay-permission-rule-browser-workspace-root-title = ワークスペースルートの選択
overlay-permission-rule-browser-workspace-root-prompt = ディレクトリを参照し、Enter キーを押してディレクトリを選択します。
overlay-permission-rule-browser-target-path-title = ターゲットパスの選択
overlay-permission-rule-browser-target-path-prompt = ファイルまたはディレクトリを参照し、Enter キーを押していずれかを選択します。
overlay-permission-rule-browser-footer = ../ またはディレクトリを選択し、Enter キーを押して参照します。値を選択し、Enter キーを押して受け入れます。
overlay-permission-rule-browser-empty = 一致するファイルまたはディレクトリがありません。
overlay-permission-rule-item-subject-kind = 件名の種類
overlay-permission-rule-item-subject-kind-detail = このルールをツール、パス、またはネットワーク ターゲットに適用するかどうかを選択します。
overlay-permission-rule-item-mode = モード
overlay-permission-rule-item-mode-detail = 一致するアクションを許可するか、要求するか、拒否するかを選択します。
overlay-permission-rule-item-scope = 範囲
overlay-permission-rule-item-scope-detail = このルールをセッション、ワークスペース、またはグローバルに永続化します。
overlay-permission-rule-item-session-id = セッションID
overlay-permission-rule-item-session-id-detail = スコープ=セッションの場合に使用されるターゲット セッション ID。
overlay-permission-rule-item-tool-name = ツール名
overlay-permission-rule-item-tool-name-detail = 一致する正確なツール名。
overlay-permission-rule-item-qualifier = 予選
overlay-permission-rule-item-qualifier-detail = より具体的なツール ルールのオプションの修飾子。
overlay-permission-rule-item-access-kind = アクセス種類
overlay-permission-rule-item-access-kind-detail = 読み取り、書き込み、または読み取り_書き込みを選択します。
overlay-permission-rule-item-target-path = ターゲットパス
overlay-permission-rule-item-target-path-detail = 保護するパス パターンまたは正確なパス。
overlay-permission-rule-item-workspace-root = ワークスペースルート
overlay-permission-rule-item-workspace-root-detail = 相対ターゲット パスを解釈するために使用されるオプションのベース ディレクトリ。
overlay-permission-rule-item-network-target = ネットワークターゲット
overlay-permission-rule-item-network-target-detail = 一致するホスト、ホスト:ポート、または URL ターゲット。
overlay-permission-rule-detail-subject-kind = ツール ルールは、ツール名とオプションの修飾子によって一致します。パスルールはファイルシステムアクセスと一致します。ネットワーク ルールはホストまたは URL アクセスと一致します。
overlay-permission-rule-detail-tool-name = ツール ルールには、`shell`、`read`、`web_search` などの正確なツール名が必要です。
overlay-permission-rule-detail-qualifier = 修飾子はオプションです。ツールまたはアクションでより狭い範囲の一致が必要な場合を除き、空のままにしておきます。
overlay-permission-rule-detail-path-access-kind = 照合するファイル システム アクセスに応じて、`read`、`write`、または `read_write` を使用します。
overlay-permission-rule-detail-workspace-root = ランタイム ワークスペース ルートを継承するには、workspace_root を空のままにしておきます。保護されたパスが別の場所にある場合は、明示的に設定します。
overlay-permission-rule-detail-target-path = パスまたはパターンを入力します。相対パスは、設定時に workspace_root に対して解釈されます。
overlay-permission-rule-detail-network-target = ルールの具体性の程度に応じて、ホスト、`host:port`、または完全な URL を入力します。
overlay-permission-rule-detail-scope = セッション スコープは一時的なオーバーライドに最適です。ワークスペースとグローバル スコープはより長く存続します。
overlay-permission-rule-detail-session-id = セッションスコープのルールには具体的なセッション ID が必要です。
overlay-permission-rule-detail-mode = 「許可」はアクションを許可し、承認を求めるプロンプトを表示し、「拒否」はアクションをブロックします。
overlay-workbench-details = 詳細
overlay-permission-studio-title = 許可
overlay-permission-studio-footer-nested = Ctrl+N 追加 · 編集を入力 · Ctrl+E 名前変更 · Ctrl+D 削除 · Esc 戻る
permission-studio-catalog-prompt = ライブツールカタログを検索します。 1 つ以上のエントリを選択するか、現在登録されていない値のカスタム ルールを選択します。
permission-studio-catalog-custom-detail = 現在のライブ カタログにないタグまたはツール名を追加します。
flash-permission-studio-catalog-empty = ルールを追加する前に、少なくとも 1 つのエントリを選択してください。
overlay-runtime-setting-current-value = 現在の上書き: { $value }
overlay-settings-help-string = テキストを入力します。空のままにするか、`clear` と入力してファイルのオーバーライドを削除します。
overlay-settings-help-bool = true/false、オン/オフ、はい/いいえ、または 1/0 を入力します。空のままにするか、`clear` と入力してファイルのオーバーライドを削除します。
overlay-settings-help-integer = 整数を入力してください。空のままにするか、`clear` と入力してファイルのオーバーライドを削除します。
overlay-settings-help-float = 数字を入力してください。空のままにするか、`clear` と入力してオーバーライドを削除します。
overlay-choice-clear-value = クリア値
overlay-settings-section-plugins-description = プラグインを構成し、そのツールと診断を検査し、ブラウザー、シェル、エディターのハーネスを管理します。
overlay-settings-section-providers-description = デフォルトのモデル ルートを選択し、プロバイダーとそのネットワーク動作を構成し、モデル カタログを検査します。
overlay-settings-section-model-catalog-description = 解決されたモデル カタログを参照し、モデルのメタデータを検査し、ローカル キャッシュを更新します。
overlay-settings-section-permissions-description = グローバル、ワークスペース、現在のセッションの権限を個別に編集します。
overlay-settings-section-runtime-session-description = 互換性のあるクライアントのバージョンと自動セッション圧縮動作を構成します。
settings-permission-effective-detail = 読み取り専用 · グローバル、ワークスペース、セッションからマージされます。
settings-permission-effective-read-only = 有効な権限は読み取り専用です。代わりにセッション、ワークスペース、またはグローバル ソースを編集してください。
settings-field-default-provider-description = セッションオーバーライドがアクティブでない場合に使用されるプロバイダー、アダプター、およびモデルルート
settings-field-permission-approval-model-description = 自動許可決定に使用されるモデルと思考/速度のバリアント。利用できない選択は「Ask」にフォールバックします
settings-field-tui-color-scheme-description = 端末の背景を自動的に検出するか、明るいパレットまたは暗いパレットを強制します
settings-field-tui-graphics-description = サポートされている場合は、Kitty、Sixel、または iTerm2 を使用して画像とタイプセット式を表示します。変更は TUI を再起動した後に有効になります
settings-field-activity-default-expanded-description = 種類固有のオーバーライドを持たないアクティビティのデフォルトの展開状態。推論の種類が明示的に設定されない限り、推論は拡張されたままになります。
settings-activity-kind-reasoning-description = モデルの完全な思考の軌跡。デフォルトでは展開されていますが、種類ごとに折りたたむことができます。
runtime-setting-choice-supported-model = 現行モデルでサポートされている
settings-plugin-workbench-detail = 構造化されたプラグイン ワークベンチを開いて、ランタイム ステータス、構成、ツール、操作、ログ、診断を表示します。
settings-mcp-server-detail = Agena のライブ HTTP MCP サーフェスを切り替えます。接続された Agena サーバー プロセスは実際のランタイムのままです。
settings-mcp-auth-detail = 非認証、完全な OAuth、および ChatGPT 混合認証をサイクルします。混合モードでは、初期化とツール検出が公開されたままになります。匿名アクセスが明示的に有効になっていない限り、ツール呼び出しは OAuth で保護されたままになります。
settings-mcp-anonymous-access-none-detail = 安全なデフォルト: どのツール呼び出しも匿名ではありません。 ChatGPT は、サインインする前にカタログを初期化し、検出することができます。
settings-mcp-anonymous-access-read-only-detail = 高リスクのオプトイン: 読み取り専用ツールは匿名で実行でき、プライベート ワークスペース、ファイル システム、構成、または診断データが公開される可能性があります。
settings-mcp-anonymous-access-inactive-detail = このポリシーは混合認証モードにのみ適用されます。認証を混合に切り替えて使用します。
settings-mcp-client-registration-cimd-detail = OpenAI ChatGPT クライアント ID メタデータ ドキュメントのみを受け入れます。認証されていないパブリック DCR エンドポイントは無効のままになります。
settings-mcp-client-registration-dcr-detail = 互換モード: パブリックの動的クライアント登録も公開します。クライアントが CIMD を使用できない場合にのみ有効にします。
settings-mcp-public-url-detail = 正規の HTTPS MCP リソース URL を設定します。安全な MCP トンネル URL には、完全な /v1/mcp/tunnel_id パスが含まれる場合があります。転送されたリクエスト ヘッダーは、OAuth ID として信頼されることはありません。
settings-mcp-oauth-issuer-detail = パブリックのブラウザ向け認可サーバー発行者を設定します。 Agena 管理の OAuth には、パスのないオリジン (https://auth.example.com など) が必要です。 OAuth と MCP が同じドメインを使用する場合は、空のままにしてください。
settings-mcp-oauth-password-detail = Agena OAuth 認証ページに表示されるパスワードを設定します。これはサーバーによって Argon2 ハッシュとして保存されます。
settings-mcp-oauth-password-clear-detail = MCP 固有のパスワードを削除し、サーバー UI パスワード (構成されている場合) に戻します。
settings-field-runtime-codex-version-description = プロバイダー要求 ID ヘッダーで使用される正確な @openai/codex 互換バージョン。
settings-field-runtime-claude-version-description = プロバイダー要求 ID ヘッダーで使用される正確な @anthropic-ai/claude-code 互換バージョン。
settings-field-runtime-gemini-version-description = プロバイダー リクエスト ID ヘッダーで使用される正確な @google/gemini-cli 互換バージョン。
settings-field-session-compaction-auto-description = コンテキスト ウィンドウの制限に近づくとセッションを自動的に圧縮します。
settings-field-session-compaction-reserved-tokens-description = いつ圧縮するかを決定するときにコンテキスト ウィンドウから予約されるトークン。計算されたデフォルトを使用する場合はクリアします。
settings-client-versions-refresh-description = npm から互換性のある最新のパッケージ バージョンをフェッチし、3 つの正確な値をすべて保持して、ランタイムをリロードします。
settings-client-versions-entry-detail = プロバイダー要求 ID ヘッダーで使用されている正確な互換性バージョンを開きます。
settings-client-versions-section-description = プロバイダー要求 ID ヘッダーで使用される正確な互換性バージョン。各値を編集するか、Ctrl+R を押して npm から更新します。
settings-provider-workbench-detail = 認証、アダプター、モデルルーティング、または新しいプロバイダーを構成する前に、検索可能なプロバイダーのリストを開きます。
settings-provider-new-detail = 新しいプロバイダーを作成し、ライブアダプターモデルをリストし、プロバイダーアダプター構成を編集します。グローバルモデルを別途選択してください。
settings-model-catalog-open-detail = 解決されたモデル メタデータを検査し、ローカル モデル カタログ キャッシュを更新します。
permission-studio-command-rules-shell-only = コマンド ルールは標準シェル ツール (agena.shell.run) にのみ適用されます。名前ルールまたは他のツールのデフォルトを使用します。
permission-studio-detail-editable = Enter を押すと、この権限スライスの複数行の JSON エディターが開きます。
permission-studio-detail-add-hint = Enter を押すとこのアイテムが作成され、すぐに開きます。
permission-studio-detail-full-config-editable = Enter を押すと、ドキュメント全体の高度な JSON エディターが開きます。
overlay-permission-studio-delete-title = ルールの削除
overlay-permission-studio-delete-body = { $kind } を削除: { $value }
flash-permission-studio-no-add = 現在のセクションには項目を追加できません。
flash-permission-studio-no-delete = 現在のセクションでは項目を削除できません。
flash-permission-studio-no-selection = 最初に項目を選択します。
flash-permission-studio-context-lost = 権限エディターのコンテキストが失われました。許可スタジオを再度開いて、再試行してください。
value-default = デフォルト
value-none = なし
value-clear = クリア
value-path = パス
value-network = ネットワーク
value-workspace = ワークスペース
value-external = 外部
value-permission-filesystem = ファイルシステム
value-permission-network = ネットワーク
value-permission-tools = ツール
value-rule-count = { $count } ルール
value-custom = カスタム
value-internet = インターネット
value-private = プライベート
value-loopback = ループバック
value-name-count = { $count } 名前
value-rule-set-count = { $count } ルール セット
value-open = 開く
composer-prompt-history-title = プロンプト履歴
overlay-commands-title = コマンドパレット
overlay-commands-prompt = 検索アクション。テキストが必要なコマンドはコンポーザーで続行されます
overlay-skill-studio-title = スキルの管理
overlay-lineage-title = ブランチ履歴 [#{ $session }]
overlay-lineage-prompt = 現在のブランチ ツリーを探索し、祖先、兄弟、または子のセッションにジャンプします
overlay-rewind-title = セッションを巻き戻す [#{ $session }]
overlay-rewind-prompt = 撤回するユーザー メッセージとその後のすべてを選択します
overlay-picker-loading = 読み込み中...
overlay-picker-empty = 一致するアイテムはありません
overlay-picker-footer = 選択したラベルをタブで埋める
session-model-context-window = { $value } ctx
session-model-max-output = { $value } から
overlay-provider-studio-detail-footer = 矢印キーで選択 · Enter edit · Esc back;認証アクションはメインのプロバイダー ページに表示されます
overlay-provider-studio-configured-disk = ディスク上に構成されます。現在の認証契約の一部ではありません
overlay-provider-studio-new-model-prompt = 選択したアダプターの下に追加するモデル ID を入力します。
provider-field-provider-id = プロバイダーID
provider-field-auth-mode = 認証モード
provider-field-auth-subtype = 認証サブタイプ
provider-field-auth-login-method = 認証ログイン方法
provider-field-start-auth = 認証を開始する
provider-field-continue-auth = 認証を続行
provider-field-auth-details = 認証の詳細
provider-field-base-url = ベースURL
provider-field-instance-url = インスタンスURL
provider-field-api-key-source = APIキーソース
provider-field-api-key-value = APIキーの値
provider-field-redirect-uri = リダイレクトURI
provider-field-callback-url = コールバック URL
provider-field-refresh-token = リフレッシュトークン
provider-field-access-token = アクセストークン
provider-field-expires-at-ms = 有効期限 (ミリ秒)
provider-field-account-id = アカウントID
provider-field-enterprise-domain = エンタープライズドメイン
provider-field-region = 地域
provider-field-profile = プロフィール
provider-field-access-key-id = アクセスキーID
provider-field-secret-access-key = シークレットアクセスキー
provider-field-session-token = セッショントークン
provider-field-service-key-env = サービスキー環境
provider-field-default-adapter = デフォルトのアダプター
provider-field-request-timeout = リクエストのタイムアウト (秒)
provider-field-connect-timeout = 接続タイムアウト (秒)
provider-field-adapter-id = アダプターID
provider-field-model-id = モデルID
provider-model-field-model-id = モデルID
provider-model-field-enabled = 有効
provider-model-field-native-compaction = ネイティブ圧縮
provider-model-field-agena-tool-mode = ツールモード (agena_tools.mode)
agena-tool-mode-provider-protocol-label = プロバイダー_プロトコル
agena-tool-mode-provider-protocol-detail = プロバイダー API のツール プロトコルを介して、Agena が管理するツール定義と呼び出しをトランスポートします。
agena-tool-mode-disabled-label = 無効化された
agena-tool-mode-disabled-detail = Agena 管理のツールやプロバイダーネイティブのツールをこのモデルに公開しないでください。
provider-model-field-display-name = 表示名
provider-model-field-lifecycle = ライフサイクル
provider-model-field-context-window = コンテキストウィンドウ
provider-model-field-max-input = 最大入力
provider-model-field-max-output = 最大出力
provider-model-field-features = 特長
provider-model-field-input-modalities = 入力モダリティ
provider-model-field-output-modalities = 出力モダリティ
provider-model-field-thinking-modes = 思考モード
provider-model-field-speed-modes = 速度モード
provider-model-field-description = 説明
provider-model-enabled-detail = このモデル ルートが有効かどうか。
provider-model-native-compaction-detail = Agena のテキスト サマライザーにフォールバックする前に、このプロバイダーのネイティブ会話圧縮エンドポイントを試してください。
provider-model-lifecycle-detail = モデルのライフサイクル値。
provider-auth-mode-none-detail = プロバイダ認証メタデータを無効にする
provider-auth-mode-api-detail = カスタム HTTP エンドポイント、Cline API、GitLab ゲートウェイ トークン、または Bedrock SigV4 の第 2 段階サブタイプを使用した API スタイルの認証
provider-auth-mode-credential-detail = 認証サブタイプ フィールドで選択された、ローカル発行者から解決された資格情報に裏付けされた認証
provider-auth-kind-unset = 設定を解除する
provider-auth-kind-none = なし
provider-auth-kind-api = API
provider-auth-kind-cline = クラインAPI
provider-auth-kind-gitlab = gitlab_api
provider-auth-kind-credential = 資格情報
provider-auth-kind-credential-with-issuer = credential:{ $issuer }
provider-auth-kind-bedrock = bedrock_sigv4
provider-auth-subtype-custom-label = カスタム
provider-auth-subtype-custom-detail = OpenAI 互換、Anthropic、または Gemini HTTP プロバイダーの汎用 API キー + ベース URL 認証
provider-auth-subtype-cline-api-detail = Fixed Cline API endpoint; API キーの入力のみが必要で、モデル検出は Cline 推奨モデルを使用します。
provider-api-key-source-inline-detail = Store the API key inline in the provider config
provider-api-key-source-env-detail = Read the API key from an environment variable
provider-auth-subtype-gitlab-api-detail = openai または anthropic アダプターを介してルーティングされる GitLab トークン認証
provider-auth-subtype-bedrock-detail = AWS Bedrock SigV4 signing
provider-auth-login-kind-browser-label = ブラウザOAuth
provider-auth-login-kind-device-label = デバイスコードログイン
provider-auth-login-kind-browser-detail = 承認 URL を開いて、リダイレクトされたコールバックを終了します。
provider-auth-login-kind-device-detail = 短い確認 URL を開いてデバイス コードを入力し、ポーリングします。
provider-issuer-openai-chatgpt-label = openai_chatgpt
provider-issuer-github-copilot-label = github_copilot
provider-issuer-gitlab-label = gitlab
provider-issuer-google-adc-label = google_adc
provider-issuer-sap-ai-core-label = sap_ai_core
provider-issuer-openai-chatgpt-detail = OpenAI ChatGPT credentials
provider-issuer-github-copilot-detail = GitHub Copilot credentials
provider-issuer-gitlab-detail = GitLab OAuth 認証情報
provider-issuer-google-adc-detail = Google Application Default Credentials
provider-issuer-sap-ai-core-detail = SAP AI Core service key auth
provider-instance-url-gitlab-detail = GitLab.com browser OAuth endpoint
provider-redirect-local-copy-detail = OAuth リダイレクトをコピー/ペーストするための localhost コールバック URL
provider-region-choice-detail = AWS リージョン
provider-service-key-env-detail = デフォルトの SAP AI コア サービス キーの環境変数
overlay-model-catalog-field-model-id = モデルID
overlay-model-catalog-field-display = ディスプレイ
overlay-model-catalog-field-origin = 起源
overlay-model-catalog-field-lifecycle = ライフサイクル
overlay-model-catalog-field-dates = 日付
overlay-model-catalog-field-limits = 限界
overlay-model-catalog-field-inputs = 入力
overlay-model-catalog-field-output = 出力
overlay-model-catalog-field-features = 特長
overlay-model-catalog-field-modes = モード
overlay-model-catalog-field-defaults = デフォルト
overlay-model-catalog-field-runtime = ランタイム
overlay-model-catalog-field-pricing = 価格設定
overlay-model-catalog-field-source = ソース
overlay-model-catalog-limits = ctx { $context } · イン { $input } · アウト { $output }
overlay-model-catalog-lifecycle-active = アクティブな
overlay-model-catalog-lifecycle-preview = プレビュー
overlay-model-catalog-lifecycle-beta = ベータ版
overlay-model-catalog-lifecycle-alpha = アルファ
overlay-model-catalog-lifecycle-experimental = 実験的な
overlay-model-catalog-lifecycle-deprecated = 廃止された
overlay-model-catalog-date-release = { $value } をリリース
overlay-model-catalog-date-updated = { $value } を更新しました
overlay-model-catalog-date-cutoff = カットオフ { $value }
overlay-model-catalog-default-thinking = 考える
overlay-model-catalog-default-speed = 速度
overlay-model-catalog-thinking-modes = 思考モード
overlay-model-catalog-speed-modes = 速度モード
overlay-model-catalog-default-verbosity = 冗長性
overlay-model-catalog-default-temperature = 温度
overlay-model-catalog-default-top-p = トップ_p
overlay-model-catalog-default-top-k = トップ_k
overlay-model-catalog-parallel-tools = 平行ツール
overlay-model-catalog-supports-verbosity = 冗長性
overlay-model-catalog-reasoning-interleaved = インターリーブ推論
overlay-model-catalog-reasoning-field = 推理分野
overlay-model-catalog-open-weights = オープンウェイト
overlay-model-catalog-price-input = { "$" }{ $value }/M で
overlay-model-catalog-price-output = 出力 { "$" }{ $value }/M
overlay-model-catalog-price-cache-read = キャッシュ読み取り { "$" }{ $value }/M
overlay-model-catalog-price-cache-write = キャッシュ書き込み { "$" }{ $value }/M
overlay-model-catalog-tier-count = { $count } 層
permission-rule-label-path = { $access } · { $path }
permission-rule-label-network = ネットワーク · { $target }
value-unset = 設定を解除する
value-auto = 自動
value-allow = 許可する
value-ask = 尋ねる
value-deny = 否定する
value-read = 読む
value-write = 書く
value-read-write = 読み取り/書き込み
value-yes = はい
value-no = いいえ
value-session = セッション
value-global = グローバル
value-add = 追加
value-runtime-default = 実行時のデフォルト
value-permission-rule-subject-tool = ツール
value-permission-rule-subject-path-access = パス_アクセス
value-permission-rule-subject-network-access = ネットワークアクセス
inline-fact-source = ソース
inline-fact-scope = 範囲
inline-fact-operator = オペレーター
flash-permission-rule-saved = 保存された権限ルール: { $name }
flash-permission-rule-revoked = 取り消された権限ルール: { $name }
flash-permission-rule-context-lost = パーミッション ルール スタジオ コンテキストが失われました
flash-provider-studio-context-lost = プロバイダー設定コンテキストが失われました
permission-rule-error-session-id-integer = セッション ID は整数である必要があります
permission-rule-error-tool-name-required = ツールルールにはツール名が必要です
permission-rule-error-path-access-kind-required = パスルールには path_access_kind が必要です
permission-rule-error-target-path-required = パスルールには target_path が必要です
permission-rule-error-network-target-required = ネットワーク ルールにはネットワーク ターゲットが必要です
permission-rule-error-session-id-required = セッションスコープにはセッションIDが必要です
flash-server-config-edit-in-settings = 設定ファイルはサーバーに属します。クライアントローカルのパスを開く代わりに、設定で値を編集します。
flash-command-requires-session = このアクションには開いたセッションが必要です
flash-session-busy = セッションがビジーです
flash-provider-selected = 選択されたプロバイダー: { $provider } (デフォルトは { $model })
flash-provider-cleared = プロバイダー/モデルのオーバーライドがクリアされました
flash-provider-not-found = プロバイダーが見つかりません: { $provider }
flash-provider-default-updated = デフォルトのプロバイダルートが更新されました: { $provider }/{ $model }
flash-permission-approval-model-updated = 自動承認モデルが更新されました: { $provider }/{ $model }
flash-provider-studio-adapter-required = 最初にアダプターを選択してください
flash-provider-studio-adapter-not-enabled = モデルを追加する前に、選択したアダプターを確認してください
flash-provider-studio-adapter-unavailable = 現在の認証モードでは、このアダプターを選択できません
flash-provider-studio-model-required = 最初にリストされているモデルを選択してください
flash-provider-studio-model-id-required = モデルIDは必須です
flash-provider-studio-no-auth-details = 現在の認証モードで使用できる認証の詳細はありません
flash-provider-studio-catalog-refreshed = モデルカタログを更新しました
flash-provider-studio-invalid-model-json = 無効なモデル JSON: { $error }
flash-provider-studio-live-listing-unavailable = ライブ モデル リストは認証に使用できません { $auth }
flash-provider-studio-draft-listing-unsupported = ドラフト モデル リストでは、ライブ モデル検出を備えたアダプターのみがサポートされます。サポートされていません: { $adapters }
flash-provider-studio-listing-auth-required = アダプター モデルをリストするには、現在の認証/アダプター ペアまたは既存の保存されたプロバイダーのライブ モデル検出が必要です。現在の認証は { $auth } です
flash-provider-studio-invalid-auth-login-method = 無効な認証ログイン方法
flash-provider-auth-openai-browser-started = OpenAIブラウザ認証を開始しました。ダイアログに表示された認証 URL を開き、リダイレクトされた URL をコールバック URL に貼り付けて p を押します。
flash-provider-auth-openai-device-started = OpenAI デバイスのログインが開始されました。ダイアログに表示される認証 URL を開き、コード { $code } を入力して、p を押します。
flash-provider-auth-copilot-device-started = Copilot デバイスのログインが開始されました。ダイアログに表示される認証 URL を開き、コード { $code } を入力して、p を押します。
flash-provider-auth-gitlab-browser-started = GitLab ブラウザ認証が開始されました。ダイアログに表示された認証 URL を開き、リダイレクトされた URL をコールバック URL に貼り付けて p を押します。
flash-provider-auth-atomgit-browser-started = AtomGit ブラウザ認証が開始されました。ダイアログに表示された認証 URL を開いてログインを完了し、p を押してポーリングします。
flash-provider-auth-openai-captured = OpenAI OAuth 認証情報がドラフトに取り込まれています。
flash-provider-auth-openai-pending = OpenAI デバイスのログインはまだ保留中です。確認手順が完了したら、もう一度 p を押します。
flash-provider-auth-copilot-pending = Copilot デバイスのログインはまだ保留中です。ブラウザの承認を完了してから、もう一度 p を押します。
flash-provider-auth-copilot-captured = ドラフトに取り込まれた Copilot OAuth 認証情報。
flash-provider-auth-gitlab-captured = GitLab OAuth 認証情報がドラフトに取り込まれています。
flash-provider-auth-atomgit-pending = AtomGit ブラウザへのログインはまだ保留中です。ブラウザのフローを終了し、もう一度 p を押します。
flash-provider-auth-atomgit-captured = AtomGit OAuth 認証情報がドラフトに取り込まれています。
flash-provider-auth-error-unsupported = 現在の認証モードは対話型 OAuth ログインをサポートしていません
flash-provider-auth-error-start-browser-first = Start Auth または o で最初にブラウザ認証を開始します。
flash-provider-auth-error-start-device-first = Start Auth または o を使用して最初にデバイス認証を開始します
flash-provider-auth-error-required-field = { $field } は必須です
flash-provider-save-draft = プロバイダー { $provider } をアダプター { $adapter } とともに保存しました。
flash-provider-save-adapter-matches = { $provider }/{ $adapter } を { $listed } リストされたモデルとともに保存しました。 { $matched } カタログが一致しました。
flash-provider-save-model = { $provider }/{ $adapter }/{ $model } を保存しました。
flash-provider-save-configured-model = 構成済みモデル { $provider }/{ $adapter }/{ $model } を保存しました。
flash-provider-delete-provider = プロバイダー { $provider } を削除しました。
flash-provider-delete-adapter = 構成済みアダプター { $provider }/{ $adapter } と { $count } モデルを削除しました。
flash-provider-delete-model = 構成済みモデル { $provider }/{ $adapter }/{ $model } を削除しました。
flash-provider-studio-adapter-delete-empty = 削除するアダプター設定が選択されていません。
flash-provider-save-error-required-field = { $field } は必須です
flash-provider-save-error-unsupported-default-adapter = auth { $auth } は、defaults.adapter `{ $adapter }` をサポートしていません。 { $supported } のいずれかが予想されます
flash-provider-save-error-unsupported-adapters = 認証 { $auth } はアダプターをサポートしていません: { $adapters }; { $supported } のいずれかが予想されます
flash-provider-save-error-api-base-url = OpenAI プロトコル、Anthropic、または Gemini アダプターを使用する場合、API 認証にはbase_url が必要です
flash-provider-save-error-gitlab-token = gitlab_api 認証には API キー ソースが必要です
flash-provider-save-error-credential-base-url = 認証情報発行者 `{ $issuer }` にはbase_urlが必要です
flash-provider-save-error-credential-service-key-env = 認証情報発行者 `{ $issuer }` には service_key_env が必要です
flash-provider-save-error-bedrock-key-pair = bedrock_sigv4 には、access_key_id と Secret_access_key を一緒に必要とします
flash-provider-save-error-select-model = プロバイダーを保存する前に少なくとも 1 つのモデルを選択してください
flash-provider-save-error-adapter-object = プロバイダー アダプター `{ $adapter }` は JSON オブジェクトである必要があります
flash-provider-save-error-model-object = プロバイダー モデルの構成は JSON オブジェクトである必要があります
flash-provider-save-error-configured-adapter-object = 構成されたプロバイダーアダプター設定は JSON オブジェクトである必要があります
flash-provider-save-error-configured-models-object = 構成されたプロバイダー アダプター モデルは JSON オブジェクトである必要があります
flash-provider-client-versions-refreshed = 更新されたクライアント バージョン: Codex { $codex }、Claude { $claude }、Gemini { $gemini }
terminal-diagnostics-title = 端末診断
terminal-diagnostics-eyebrow = 互換性とプロトコルの証拠
terminal-diagnostics-footer = ↑/↓ スクロール · c/y レポートのコピー · Esc 閉じる
terminal-diagnostics-tip = 製品アイデンティティと環境レイヤーは証拠に基づいています。汎用 SSH は実際のエンドポイント端末を証明できません。
terminal-diagnostics-copied = 端末診断がコピーされました
terminal-diagnostics-unavailable = このランタイムでは端末診断は利用できません。
terminal-diagnostics-summary = 証拠に裏付けられた端末レポート · エンドポイントの信頼度 { $confidence }
terminal-diagnostics-none = なし
terminal-diagnostics-unknown = 不明
terminal-diagnostics-unavailable-value = 利用不可
terminal-diagnostics-term-unset = TERM が設定されていません
terminal-diagnostics-section-identity = アイデンティティ
terminal-diagnostics-section-layers = 環境レイヤー
terminal-diagnostics-section-color = 色と外観
terminal-diagnostics-section-protocols = アクティブなプロトコル
terminal-diagnostics-section-providers = プロバイダーと統合
terminal-diagnostics-section-warnings = 警告
terminal-diagnostics-field-product = 製品
terminal-diagnostics-field-version = バージョン
terminal-diagnostics-field-parsed-version = 解析されたバージョン
terminal-diagnostics-field-compatibility = 互換性
terminal-diagnostics-field-confidence = 自信
terminal-diagnostics-field-source = 選択したソース
terminal-diagnostics-field-evidence = 証拠
terminal-diagnostics-field-conflicts = 紛争
terminal-diagnostics-color-configured = 設定済みモード
terminal-diagnostics-color-detected-background = 検出された背景
terminal-diagnostics-color-detected-appearance = 検出された外観
terminal-diagnostics-color-source = 検出源
terminal-diagnostics-color-refresh = 自動更新
terminal-diagnostics-color-generation = 外観の生成
terminal-diagnostics-color-effective-appearance = 効果的なテキストパレット
terminal-diagnostics-color-formula-foreground = 数式グリフの色
terminal-diagnostics-color-formula-background = 数式画像の背景
terminal-diagnostics-color-background-images = 背景画像
terminal-diagnostics-color-mode-auto = 自動
terminal-diagnostics-color-mode-dark = 強制ダーク
terminal-diagnostics-color-mode-light = 強制光
terminal-diagnostics-color-appearance-dark = 暗い
terminal-diagnostics-color-appearance-light = ライト
terminal-diagnostics-color-appearance-unknown = 不明
terminal-diagnostics-color-appearance-conservative = 保守的なターミナルネイティブカラー（背景不明）
terminal-diagnostics-color-source-osc11 = OSC 11 端末応答
terminal-diagnostics-color-source-iterm-osc4 = iTerm2 OSC 4;-2 端末応答
terminal-diagnostics-color-source-colorfgbg = COLORFGBG環境フォールバック
terminal-diagnostics-color-source-term-background = TERM_BACKGROUND 環境のフォールバック
terminal-diagnostics-color-source-vscode-theme = VSCODE_THEME_KIND 環境のフォールバック
terminal-diagnostics-color-source-unavailable = 使用可能な端末または環境の証拠がない
terminal-diagnostics-color-refresh-live = フォーカスの回復と端末の再開時。失敗した更新では、最後に知られた色が保持されます
terminal-diagnostics-color-refresh-startup-only = 起動時のみ。端末が更新可能な色のクエリに応答しませんでした
terminal-diagnostics-color-formula-background-transparent = 透明。式グリフの色のみが外観に従います
terminal-diagnostics-color-background-images-not-sampled = サンプリングされていません。透明な数式ピクセルは、端末の背景またはその下の背景画像を保持します
terminal-diagnostics-direct = 直接
terminal-diagnostics-direct-description = SSH、Mosh、マルチプレクサ、または WSL の証拠は検出されませんでした。
terminal-diagnostics-layer-description = { $source } から検出されました。レイヤーの順序とネストの深さは不明です。
terminal-diagnostics-capability-description = エンドポイント={ $status } · ソース={ $source } · パス={ $path } · プロバイダー={ $provider }
terminal-diagnostics-path-clear = クリア
terminal-diagnostics-path-forced = オーバーライドによって強制される
terminal-diagnostics-path-unverified = 未確認
terminal-diagnostics-path-blocked = ブロックされました
terminal-diagnostics-provider-not-required = 必要ありません
terminal-diagnostics-provider-ready = 準備完了
terminal-diagnostics-provider-missing = 欠落しているか実装されていない
terminal-diagnostics-helper-missing = 見つからないか、実行可能ではありません。
terminal-diagnostics-helper-not-probed = エンドポイントが Kitty として識別されないため、プローブされません。
terminal-diagnostics-no-warnings = 互換性に関する警告は検出されませんでした。
terminal-diagnostics-protocol-alternate-screen = 代替画面
terminal-diagnostics-protocol-bracketed-paste = 括弧付きペースト
terminal-diagnostics-protocol-focus = レポートに焦点を当てる
terminal-diagnostics-protocol-mouse = マウスキャプチャ
terminal-diagnostics-protocol-mouse-mode = マウスワイヤーモード
terminal-diagnostics-protocol-mouse-events = 受信したマウスイベント
terminal-diagnostics-protocol-mouse-last = 最後のマウスイベント
terminal-diagnostics-mouse-mode-button-sgr = SGR 座標 (DECSET 1006) を使用したボタン イベント トラッキング (DECSET 1002)
terminal-diagnostics-mouse-events-none = なし。エンドポイント端末はマウス イベントを Agena に配信していません。マウスレポートおよびホイールレポートのプロファイル設定を確認してください。
terminal-diagnostics-mouse-events-seen = { $count } イベント
terminal-diagnostics-mouse-last-none = なし
terminal-diagnostics-protocol-keyboard = キーボードの曖昧さ回避
terminal-diagnostics-protocol-key-events = キーボードイベントの種類
terminal-diagnostics-protocol-background = バックグラウンドクエリ
terminal-diagnostics-protocol-native-clipboard = ネイティブクリップボード
terminal-diagnostics-protocol-osc52-write = OSC52書き込み
terminal-diagnostics-protocol-osc52-read = OSC52読み取り
terminal-diagnostics-protocol-progress = OSC 9;4 の進行状況
terminal-diagnostics-provider-kitty-clipboard = キティのクリップボード
terminal-diagnostics-provider-kitty-transfer = キティの譲渡
terminal-diagnostics-provider-iterm-transfer = iTerm2 転送
terminal-diagnostics-provider-inline-images = インライン画像
terminal-diagnostics-provider-hyperlinks = ハイパーリンク
terminal-diagnostics-provider-sync-output = 同期出力
terminal-diagnostics-status-confirmed = 確認された
terminal-diagnostics-status-forced = オーバーライドによって強制される
terminal-diagnostics-status-profiled = プロファイルされた
terminal-diagnostics-status-unsupported = サポートされていない
terminal-diagnostics-status-unknown = 不明
terminal-diagnostics-source-user = ユーザーオーバーライド
terminal-diagnostics-source-environment = 環境
terminal-diagnostics-source-helper = ヘルパープローブ
terminal-diagnostics-source-terminal-query = 端末クエリ
terminal-diagnostics-source-profile = 端末プロファイル
terminal-diagnostics-source-platform = プラットフォームのデフォルト
terminal-diagnostics-source-conservative = 保守的なデフォルト
terminal-diagnostics-source-terminfo = terminfo の互換性
terminal-diagnostics-source-unknown = 不明
terminal-diagnostics-confidence-explicit = 明示的な
terminal-diagnostics-confidence-strong = 強い
terminal-diagnostics-confidence-compatibility = 互換性のみ
terminal-diagnostics-confidence-unknown = 不明

# Plugin Workbench i18n completion
plugin-workbench-action-diff = 差分
plugin-workbench-action-refresh = 更新
plugin-workbench-action-remove-selected = 選択項目を削除/リセット
plugin-workbench-action-reset-all = すべてリセット
plugin-workbench-action-restart = 再起動
plugin-workbench-action-save = 保存
plugin-workbench-action-validate = 検証
plugin-workbench-actions = アクション
plugin-workbench-authority-unavailable = 権限情報を利用できません。
plugin-workbench-choices = 選択肢
plugin-workbench-close-footer = Esc で閉じる
plugin-workbench-column-after = 変更後
plugin-workbench-column-args = 引数
plugin-workbench-column-arguments = 引数
plugin-workbench-column-before = 変更前
plugin-workbench-column-category = カテゴリ
plugin-workbench-column-change = 変更
plugin-workbench-column-operation = 操作
plugin-workbench-column-description = 説明
plugin-workbench-column-field = フィールド
plugin-workbench-column-inputs = 入力
plugin-workbench-column-message = メッセージ
plugin-workbench-column-plugin = プラグイン
plugin-workbench-column-section = セクション
plugin-workbench-column-severity = 重大度
plugin-workbench-column-source = ソース
plugin-workbench-column-summary = 概要
plugin-workbench-column-tool = ツール
plugin-workbench-column-version = バージョン
plugin-workbench-column-visible-tool = 表示ツール
plugin-workbench-operation-arguments = 引数: {$operation}
plugin-workbench-config = 設定
plugin-workbench-config-action = アクション
plugin-workbench-config-choose-shape = 形式を選択
plugin-workbench-config-choose-type = 型を選択
plugin-workbench-config-default = 既定値
plugin-workbench-config-diff = 設定差分
plugin-workbench-config-dirty = 未保存
plugin-workbench-config-drilldown-footer = ←/→ セル · ↑/↓ 行 · Enter 編集 · Ctrl+D 削除/リセット · Esc 戻る
plugin-workbench-config-saved = 保存済み
plugin-workbench-config-setting = 設定項目
plugin-workbench-config-state = 状態
plugin-workbench-config-state-changed = 変更済み
plugin-workbench-config-state-default = 既定
plugin-workbench-config-state-dirty = 未保存
plugin-workbench-config-state-error = エラー
plugin-workbench-config-state-inactive = 無効
plugin-workbench-config-summary = {$status} · {$save_state}
plugin-workbench-config-title = {$plugin} / 設定
plugin-workbench-config-type = 型
plugin-workbench-config-value = 値
plugin-workbench-config-view-summary = 有効な設定 · {$changed} 件の変更 · 選択セル: {$cell}
plugin-workbench-detail-footer = Tab/Shift+Tab セクション · ↑/↓ スクロール · Esc 戻る
plugin-workbench-detail-tools-footer = Tab/Shift+Tab セクション · ↑/↓ 選択 · Enter 設定して実行 · Esc 戻る
plugin-workbench-filter-all = すべて
plugin-workbench-filter-other = その他
plugin-workbench-header-summary = ツール: {$tools}        操作: {$operations}        設定: {$config}
plugin-workbench-input-preview = 入力プレビュー: {$tool}
plugin-workbench-last-result-failed = 直近の結果 · {$tool} · 失敗
plugin-workbench-last-result-success = 直近の結果 · {$tool} · 成功
plugin-workbench-list-footer = 入力して検索 · ↑/↓ 選択 · Enter 開く · Esc 閉じる
plugin-workbench-list-summary = プラグイン検索… {$query}        トランスポート: {$transport}        設定: {$config}        {$shown}/{$total} 件表示
plugin-workbench-loading-actions = アクションを読み込み中…
plugin-workbench-loading-choices = 選択肢を読み込み中…
plugin-workbench-no-changes = 変更なし
plugin-workbench-no-operations = 操作はありません。
plugin-workbench-no-config-section = 設定セクションがありません。
plugin-workbench-no-editable-rows = 編集可能な行がありません。
plugin-workbench-no-filter-matches = 現在のフィルターに一致するプラグインはありません。
plugin-workbench-no-issues = 問題なし
plugin-workbench-no-logs = ログはありません。
plugin-workbench-no-selection = プラグインが選択されていません。
plugin-workbench-no-structured-arguments = 構造化引数はありません。
plugin-workbench-no-tools = ツールはありません。
plugin-workbench-none = なし
plugin-workbench-none-declared = 宣言なし
plugin-workbench-overview = 概要
plugin-workbench-package-summary = パッケージ: {$package}
plugin-workbench-plugin = プラグイン
plugin-workbench-plugin-capabilities = プラグイン機能
plugin-workbench-plugins = プラグイン
plugin-workbench-provenance = 由来: {$provenance}
plugin-workbench-sections = セクション
plugin-workbench-severity-error = エラー
plugin-workbench-severity-warning = 警告
plugin-workbench-status-invalid = 無効
plugin-workbench-status-issues = 問題
plugin-workbench-status-missing = 未設定
plugin-workbench-status-needs-restart = 再起動が必要
plugin-workbench-status-runtime-issue = 実行時の問題
plugin-workbench-status-schema-missing = スキーマなし
plugin-workbench-status-valid = 有効
plugin-workbench-status-warning = 警告
plugin-workbench-summary = 検索: {$query} · トランスポート {$transport} · 設定 {$config} · {$shown}/{$total} 件表示
plugin-workbench-tab-capabilities = 機能
plugin-workbench-tab-operations = 操作
plugin-workbench-tab-config = 設定
plugin-workbench-tab-diagnostics = 診断
plugin-workbench-tab-logs = ログ
plugin-workbench-tab-tools = ツール
plugin-workbench-tabs = タブ
plugin-workbench-tags-summary = タグ: {$tags}
plugin-workbench-tool-capabilities = ツール機能
plugin-workbench-tools-help = ↑/↓ でツールを選択します。Enter でホスト管理のスキーマフォームを開き、Ctrl+S で検証して実行します。
plugin-workbench-transport = トランスポート
plugin-workbench-trust-level = 信頼レベル: {$level}
plugin-workbench-unavailable = 利用不可


# Plugin Workbench structured editor i18n completion
plugin-workbench-editor-also-matches = 次にも一致: {$matches}
plugin-workbench-editor-array-action-help = Enter アクションメニュー · Ctrl+D で選択行を削除
plugin-workbench-editor-array-preview = 設定…（{$count} 項目）
plugin-workbench-editor-configure = 設定…
plugin-workbench-editor-format = 形式: {$format}
plugin-workbench-editor-generic-object = 汎用オブジェクトエディター
plugin-workbench-editor-index = インデックス
plugin-workbench-editor-item = 項目 {$index}
plugin-workbench-editor-map = マップエディター
plugin-workbench-editor-no-fields = フィールドはありません。
plugin-workbench-editor-no-items = 項目はありません。
plugin-workbench-editor-object = オブジェクトエディター
plugin-workbench-editor-object-action-help = Enter アクションメニュー · アクションセルからフィールドを追加
plugin-workbench-editor-object-array = オブジェクト配列テーブルエディター
plugin-workbench-editor-object-array-help = 編集すると、選択項目が同じ構造化エディターで開きます。
plugin-workbench-editor-object-preview = 設定…（{$count} フィールド）
plugin-workbench-editor-preview = プレビュー
plugin-workbench-editor-primitive-array = プリミティブ配列エディター
plugin-workbench-editor-readonly = 読み取り専用
plugin-workbench-editor-schema-missing = スキーマなし        基本構造化エディター
plugin-workbench-editor-shape = 形式
plugin-workbench-editor-suggestions = 候補
plugin-workbench-editor-tuple = タプルエディター
plugin-workbench-editor-type-summary = 型: {$type}        パスエディター: 構造化 GUI
plugin-workbench-field-state-available = 利用可能
plugin-workbench-field-state-custom = カスタム
plugin-workbench-field-state-map-key = マップキー
plugin-workbench-field-state-missing = 不足
plugin-workbench-field-state-optional = 任意
plugin-workbench-field-state-required = 必須
plugin-workbench-kind-all-of = allOf
plugin-workbench-kind-any-of = anyOf
plugin-workbench-kind-array = 配列
plugin-workbench-kind-boolean = 真偽値
plugin-workbench-kind-integer = 整数
plugin-workbench-kind-null = null
plugin-workbench-kind-number = 数値
plugin-workbench-kind-object = オブジェクト
plugin-workbench-kind-one-of = oneOf
plugin-workbench-kind-string = 文字列
plugin-workbench-kind-value = 値

overlay-provider-list-create-detail = プロバイダーの下書きを作成し、認証、アダプター、モデルを設定します。

overlay-provider-delete-body = プロバイダー {$provider} と設定済みのすべてのアダプター/モデルを削除しますか？

overlay-provider-delete-adapter-body = 設定済みアダプター {$provider}/{$adapter} を削除しますか？

overlay-provider-delete-adapter-last-body = これは最後の設定済みアダプターです。確定するとプロバイダーも削除されます。

overlay-provider-delete-model-body = 設定済みモデル {$provider}/{$adapter}/{$model} を削除しますか？
