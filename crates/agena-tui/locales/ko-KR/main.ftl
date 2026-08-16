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
hub-title = 세션 허브
hub-action-create = 새 세션
hub-action-list = 세션 목록
hub-action-refresh = 새로고침
hub-hint-move = 이동
hub-hint-focus = 포커스
hub-hint-open = 열기
hub-hint-back = 뒤로
hub-section-attention = 주의 필요
hub-section-running = 실행 중
hub-section-recent = 최근
hub-empty-attention = 주의가 필요한 세션이 없습니다
hub-empty-running = 실행 중인 세션이 없습니다
hub-empty-recent = 최근 세션이 없습니다
hub-section-new = 새 세션
hub-empty-new = 만들 세션이 없습니다
hub-item-new = + 새 세션
hub-item-new-detail = Enter 로 새 세션 만들기
hub-action-search = 검색
hub-action-clear-search = 검색 지우기
hub-search-placeholder = 세션을 필터링하려면 입력…
hub-search-active-empty = 입력하여 필터링…
hub-search-active = 필터:{$query}
command-hub-summary = 세션 허브 열기
command-background-summary = 세션 허브로 돌아가기;세션은 계속 실행
hub-empty = 아직 세션이 없습니다. Ctrl+N으로 만드세요.
context-help-context-hub = 세션 허브
context-help-summary-hub = 주의가 필요한 세션, 실행 중인 세션, 최근 세션을 확인하고 새 세션을 만듭니다.
context-help-key-create-session = 새 세션을 만듭니다.
context-help-key-session-list = 전체 세션 목록을 엽니다.

transcript-header-lines = 줄 {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = 찾기={$query} ({$current}/{$total})
transcript-header-tail = 꼬리 따라가기
transcript-header-loading = 로딩 중
transcript-header-loading-older = 이전 메시지 로딩 중
transcript-header-busy = 바쁨
transcript-loading-older = 이전 메시지를 불러오는 중...
transcript-more-older = 더 이전 메시지가 있습니다. 위로 스크롤하거나 PageUp 을 누르세요.
transcript-empty-session = 이 세션에는 아직 메시지가 없습니다.

session-state-creating = 생성 중
session-state-ready = 최근 완료
session-state-running = 실행 중
session-state-awaiting-user = 사용자 입력 대기
session-state-interrupted = 중단됨
session-state-failed = 실패

no-session-selected = 선택된 세션이 없습니다.
no-session-selected-hint = /sessions 로 세션을 선택하거나 입력창에 바로 입력을 시작해 새 세션을 만드세요.
composer-session-new = 새 세션
composer-placeholder = Agena에 입력. 맨 앞에서 Up을 누르면 기록. / 명령. Ctrl+O 첨부.

status-global = / 아래 검색 | ? 위 검색 | Ctrl+C 두 번 종료
status-sessions = 세션: /sessions
status-transcript = VIEW: i 입력 | j/k 스크롤 | / 검색 | c 마지막 복사 | y 복사
status-composer = INSERT: Esc 돌아가기 | Ctrl+Enter 즉시 전송 | Ctrl+J 줄바꿈 | 맨 앞에서 Up 기록 | / 명령 | Ctrl+G 항목 | Ctrl+R 입력 | Ctrl+L 승인

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
help-composer-line-2 = Shift+Enter 또는 Ctrl+J 줄바꿈
help-composer-line-3 = Ctrl+A/E/B/F/P/N 이동, Ctrl+Left/Right 단어 이동
help-composer-line-4 = Ctrl+H/D/W/U/K/Y 로 shell 또는 editor 스타일 편집
help-composer-line-5 = 줄 경계에서 Ctrl+A/E 는 이전/다음 줄로 이어서 이동 가능
help-composer-line-6 = Ctrl+O 로 워크스페이스 파일 검색 후 첨부
help-composer-line-7 = Ctrl+E 로 $VISUAL/$EDITOR 열기
help-composer-line-8 = Ctrl+T 로 클립보드 이미지 첨부
help-composer-line-9 = 붙여넣은 텍스트는 바로 입력되고, 단일 파일 경로는 첨부되며, 첨부는 원자적으로 유지됩니다
help-composer-line-10 = 커서가 입력창 맨 앞에 있을 때 Up으로 기록을 열고 Ctrl+P로 대기 메시지를 편집하고 Ctrl+X로 취소합니다
help-section-actions = 동작
help-actions-line-1 = Ctrl+N 세션 생성, n/N 검색 결과 이동
help-actions-line-2 = r 차단되었거나 보류 중인 세션 계속; U 사용량 분석 열기
help-actions-line-3 = a/A/d/D 로 첫 번째 권한 요청 응답
help-actions-line-4 = Composer에서 Ctrl+R로 첫 번째 사용자 입력 요청 열기
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
overlay-user-input-reply-format = 답변 형식: 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X 로 요청 취소
overlay-user-input-footer = Ctrl+X 취소

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
flash-message-interrupting = 실행 중인 작업을 중단합니다 - 메시지는 다음에 전송됩니다

message-role-user = 사용자
message-role-assistant = 어시스턴트
message-role-system = 시스템

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed
message-state-policy-denied = blocked by permission policy
message-state-user-declined = declined by user
message-state-capability-unavailable = capability unavailable
message-state-tool-unavailable = tool unavailable

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
message-user-input-replied = 사용자 입력에 답변함：{$request_id}
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
attachment-kind-directory = 폴더
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

permission-summary-allow-once = 한 번 허용됨: {$reason}
permission-summary-allow-always = 항상 허용됨: {$reason}
permission-summary-deny-once = 한 번 거부됨: {$reason}
permission-summary-deny-always = 항상 거부됨: {$reason}

failure-detail-message = 메시지
failure-detail-code = 오류 코드
failure-detail-category = 범주
failure-detail-responsibility = 책임
failure-detail-impact = 영향
failure-detail-recovery = 복구
failure-detail-retry = 재시도
failure-category-invalid-input = 잘못된 입력
failure-category-not-found = 찾을 수 없음
failure-category-conflict = 충돌
failure-category-permission-required = 권한 필요
failure-category-permission-denied = 권한 거부됨
failure-category-authentication-required = 인증 필요
failure-category-rate-limited = 요청 제한
failure-category-quota-exceeded = 할당량 초과
failure-category-timeout = 시간 초과
failure-category-dependency-unavailable = 종속성 사용 불가
failure-category-protocol-failure = 프로토콜 오류
failure-category-data-corruption = 데이터 무결성 문제
failure-category-internal = 내부 오류
failure-responsibility-caller = 요청
failure-responsibility-policy = 정책
failure-responsibility-dependency = 종속성
failure-responsibility-system = 시스템
failure-impact-request-rejected = 요청 거부됨
failure-impact-operation-failed = 작업 실패
failure-impact-operation-paused = 작업 일시 중지
failure-impact-partial-success = 부분 성공
failure-impact-background-task-failed = 백그라운드 작업 실패
failure-impact-runtime-degraded = 런타임 저하
failure-impact-fatal-startup-failure = 치명적 시작 실패
failure-recovery-none = 자동 복구 없음
failure-recovery-refresh = 새로고침
failure-recovery-reauthenticate = 재인증
failure-recovery-open-settings = 설정 열기
failure-recovery-request-permission = 권한 요청
failure-recovery-ask-user = 사용자에게 물어보기
failure-recovery-retry = 재시도
failure-recovery-choose-alternative = 대안 선택
failure-recovery-restart-plugin = 플러그인 재시작
failure-recovery-restart-runtime = 런타임 재시작
failure-retry-never = 재시도 안 함
failure-retry-correct-input = 입력을 수정하고 재시도
failure-retry-after-user-action = 사용자 작업 후 재시도
failure-retry-after-refresh = 새로고침 후 재시도
failure-retry-immediate-once = 즉시 한 번 재시도
failure-retry-backoff = 백오프로 재시도
failure-retry-use-alternative = 대안 사용
failure-retry-unknown = 알 수 없음
