cli-about = Agena 터미널 채팅 애플리케이션

pane-sessions = 세션
pane-sessions-search = 세션 [{$query}]
pane-transcript = 대화 기록
pane-messages = 메시지
pane-composer = 입력창 [{$session}]

session-meta = #{$id}  {$message_count}개 메시지  {$updated}
session-running = 실행 중
sessions-empty = 세션을 찾을 수 없습니다
sessions-loading-more = 더 많은 세션을 불러오는 중...
sessions-more = 더 불러올 세션이 있습니다

transcript-header-lines = 줄 {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = 찾기={$query} ({$current}/{$total})
transcript-header-tail = 꼬리 따라가기
transcript-header-loading = 로딩 중
transcript-header-loading-older = 이전 메시지 로딩 중
transcript-header-busy = 바쁨
transcript-loading-older = 이전 메시지를 불러오는 중...
transcript-more-older = 더 이전 메시지가 있습니다. 위로 스크롤하거나 PageUp 을 누르세요.
transcript-empty-session = 이 세션에는 아직 메시지가 없습니다.

no-session-selected = 선택된 세션이 없습니다.
no-session-selected-hint = /sessions 로 세션을 선택하거나 입력창에 바로 입력을 시작해 새 세션을 만드세요.
composer-session-new = 새 세션
composer-placeholder = Agena에 입력. Alt+Up 기록. / 명령. F3 첨부.

status-global = / 아래 검색 | ? 위 검색 | Ctrl+C 두 번 종료
status-sessions = 세션: /sessions [검색]
status-transcript = VIEW: i 입력 | j/k 스크롤 | / 검색 | c 마지막 복사 | y 복사
status-composer = INSERT: Esc 돌아가기 | Ctrl+Enter 즉시 전송 | Ctrl+J 줄바꿈 | Alt+Up/Down 기록 | / 명령

help-title = 도움말
help-header = Agena TUI
help-section-sessions = 세션 전환
help-sessions-line-1 = /sessions 로 검색 가능한 세션 전환 창 열기
help-sessions-line-2 = Up/Down, PageUp/PageDown 으로 선택 이동
help-sessions-line-3 = Enter 로 선택한 세션 열기
help-section-transcript = 대화 기록 창
help-transcript-line-1 = i로 INSERT에 들어가고 j/k 또는 화살표로 스크롤
help-transcript-line-2 = Space / Shift+Space / Ctrl+B 페이지 이동
help-transcript-line-3 = Ctrl+D / Ctrl+U 반 페이지 이동
help-transcript-line-4 = 상단 근처에서 PageUp 을 누르면 이전 메시지 로드
help-transcript-line-5 = g/G 로 맨 위 또는 맨 아래 이동
help-transcript-line-6 = / 는 아래로, ? 는 위로 검색하며 n 은 같은 방향, N 은 반대 방향으로 이동
help-transcript-line-7 = c 마지막 assistant 메시지 복사, y 전체 복사, Y 보이는 영역 복사
help-section-composer = 입력창
help-composer-line-1 = Esc로 VIEW에 돌아가고 Enter 전송
help-composer-line-2 = Alt/Shift+Enter 또는 Ctrl+J 줄바꿈
help-composer-line-3 = Ctrl+A/E/B/F/P/N 이동, Alt+B/F 또는 Alt/Ctrl+Left/Right 단어 이동
help-composer-line-4 = Ctrl+H/D/W/U/K/Y 로 shell 또는 editor 스타일 편집
help-composer-line-5 = 줄 경계에서 Ctrl+A/E 는 이전/다음 줄로 이어서 이동 가능
help-composer-line-6 = F3, Ctrl+O, Alt+O 로 워크스페이스 파일 검색 후 첨부
help-composer-line-7 = F4 또는 Alt+E 로 $VISUAL/$EDITOR 열기
help-composer-line-8 = F6 또는 Alt+I 로 클립보드 이미지 첨부
help-composer-line-9 = 붙여넣은 텍스트는 바로 입력되고, 단일 파일 경로는 첨부되며, 첨부는 원자적으로 유지됩니다
help-composer-line-10 = Alt+Up/Down으로 보낸 프롬프트를 불러옵니다
help-section-actions = 동작
help-actions-line-1 = n 세션 생성
help-actions-line-2 = r 차단되었거나 보류 중인 세션 계속
help-actions-line-3 = a/A/d/D 로 첫 번째 권한 요청 응답
help-actions-line-4 = Composer에서 Alt+U로 첫 번째 사용자 입력 요청 열기
help-actions-line-5 = 마우스 캡처가 꺼져 있어 터미널 기본 선택/복사가 그대로 동작합니다
help-actions-line-6 = Ctrl+C를 두 번 눌러 종료

overlay-session-search-title = 세션 검색
overlay-session-search-prompt = 세션 제목 검색
overlay-transcript-search-title = 기록 검색
overlay-transcript-search-prompt = 로드된 메시지 안에서 검색
overlay-line-footer = 입력해 편집

overlay-attach-title = 파일 첨부
overlay-attach-prompt = 경로나 검색어를 입력하세요. Enter 로 선택된 파일을 첨부합니다.
overlay-attach-no-match = 일치하는 파일이 없습니다
overlay-attach-matches = 일치 결과
overlay-attach-footer = Tab 선택 경로 채우기

overlay-user-input-title = 대기 중인 사용자 입력
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = 사용자 정의 값 허용
overlay-user-input-reply-format = 답변 형식: question_id=value;other_id=value1,value2
overlay-user-input-cancel-hint = Ctrl+D 로 요청 취소
overlay-user-input-footer = Ctrl+D 취소

flash-terminal-event-error = 터미널 이벤트 오류: {$error}
flash-created-session = 세션을 만들었습니다 {$title}
flash-permission-reply-sent = 권한 응답을 보냈습니다: {$label}
flash-user-input-reply-sent = 사용자 입력 응답을 보냈습니다
flash-large-paste-staged = 큰 붙여넣기를 입력창에 임시 보관했습니다
flash-attached = {$path} 을(를) 첨부했습니다
flash-composer-updated = 외부 편집기 내용으로 입력창을 갱신했습니다
flash-prompt-history-empty = 프롬프트 기록이 비어 있습니다
flash-prompt-history-items = 프롬프트 기록을 불러오기 전에 첨부나 준비된 붙여넣기를 지워 주세요
flash-external-editor-failed = 외부 편집기 실패: {$error}
flash-clipboard-image-attached = 클립보드 이미지를 첨부했습니다: {$width}x{$height} {$format}
flash-clipboard-image-attach-failed = 클립보드 이미지 첨부 실패: {$error}
flash-no-loaded-transcript = 복사할 로드된 기록이 없습니다
flash-copied-loaded-transcript = 로드된 기록을 클립보드에 복사했습니다
flash-no-assistant-message = 복사할 assistant 메시지가 없습니다
flash-no-assistant-message-text = 마지막 assistant 메시지에 복사할 로드된 텍스트가 없습니다
flash-copied-assistant-message = 마지막 assistant 메시지를 클립보드에 복사했습니다
flash-no-visible-transcript = 복사할 보이는 텍스트가 없습니다
flash-copied-visible-transcript = 보이는 내용을 클립보드에 복사했습니다
flash-clipboard-copy-failed = 클립보드 복사 실패: {$error}

message-role-user = 사용자
message-role-assistant = 어시스턴트
message-role-system = 시스템

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed

message-parts-not-loaded = {$count}개 파트가 아직 로드되지 않았습니다
message-usage = 사용량: in={$input} out={$output} reasoning={$reasoning}
message-finish = finish: {$finish}
message-empty = (빈 메시지)
message-thinking = 생각: {$summary}
message-command-status = 상태: {$status}, exit={$exit}
message-file-changes = 파일 변경
message-file-changes-preview-one = 파일 1개: {$paths}
message-file-changes-preview-many = 파일 {$count}개: {$paths}
message-file-changes-more = 외 {$count}개
message-search = 검색: {$query}
message-todo-list = 할 일 목록
message-error = 오류 [{$code}]: {$message}
message-attachments = 첨부
message-awaiting-user-input = 사용자 입력 대기 중: {$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = 파트 상세를 사용할 수 없습니다
message-tool-pending = 대기 중: {$label}
message-tool-running = 실행 중: {$label}
message-tool-done = 완료: {$label}
message-tool-failed = 실패: {$label}
message-tool-cancelled = 취소: {$label}
message-tool-result-blocks = {$count}개 결과 블록

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

time-just-now = 방금 전
time-minutes-ago = {$count}분 전
time-hours-ago = {$count}시간 전
time-days-ago = {$count}일 전

session-default-title = 새 세션 {$time}
session-default-base = 새 세션
session-fallback-title = 세션 {$id}

user-input-error-empty = 답변은 비워 둘 수 없습니다
user-input-error-invalid-segment = 잘못된 답변 조각: {$segment}
user-input-error-unknown-question = 알 수 없는 질문 ID: {$question_id}
user-input-error-missing-answer = 질문 {$question_id} 에는 최소 하나의 답변이 필요합니다
user-input-error-no-answers = 답변에 아무 내용도 없습니다

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

paste-label = {$count}자 붙여넣기
paste-label-append = {$count}자 붙여넣기, 전송 시 추가
paste-placeholder = [{$count}자 붙여넣기]

permission-label-allow-once = 한 번 허용
permission-label-allow-always = 항상 허용
permission-label-deny-once = 한 번 거부
permission-label-deny-always = 항상 거부

permission-summary-pending = 권한 대기 중: {$reason}
permission-summary-allow-once = 한 번 허용됨: {$reason}
permission-summary-allow-always = 항상 허용됨: {$reason}
permission-summary-deny-once = 한 번 거부됨: {$reason}
permission-summary-deny-always = 항상 거부됨: {$reason}
