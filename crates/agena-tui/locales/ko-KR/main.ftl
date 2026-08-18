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
hub-hint-section = 섹션
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
session-state-awaiting-interaction = 사용자 입력 대기
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

## Settings Studio core locale coverage
## Long policy descriptions intentionally continue to use the verified English fallback.

permission-studio-new-rule-label = + 새로운 규칙

permission-studio-new-rule-value = 이름 *

permission-studio-catalog-tags-title = 도구 태그 규칙 추가

permission-studio-catalog-names-title = Tool Access 규칙 추가

permission-studio-catalog-footer = 아래 결과 · Space toggle · 입력 모드 선택 · 조건 취소

permission-studio-catalog-tag-detail = {$count} 등록 도구에 의해 사용

permission-studio-catalog-custom-label = + 사용자 정의 규칙 ...

permission-studio-catalog-custom-search = 사용자 정의 새로운 수동 태그 도구 이름

overlay-settings-title = 설정

overlay-settings-footer = Ctrl+R 새로 고침 · ←/→ 스위치 팬 · 탭/Shift+ 탭 사이클 팬 · ↑/↓ 선택 · 열려있는 입력 · 조건 닫기

overlay-settings-sections = 섹션

overlay-settings-options = 옵션

overlay-settings-group-core = 핵심

overlay-settings-group-application = 애플리케이션

overlay-settings-group-session = 세션

overlay-settings-group-system = 시스템

overlay-settings-default-section-title = 이름 *

overlay-settings-empty-section = 선택 없음.

overlay-settings-empty-items = 이 단면도에 있는 조정 없음.

overlay-settings-empty-detail = 섹션을 선택하고 검사하거나 편집 할 수있는 옵션을 선택합니다.

overlay-settings-detail-current = 현재 가치: {$value}

overlay-settings-detail-path = 경로: {$path}

overlay-settings-detail-action = 이 설정을 열고 편집합니다.

settings-detail-action-screen = 이 화면을 엽니다.

overlay-settings-edit-title = 편집 {$field}

overlay-settings-edit-file-value = 파일 override: {$value}

overlay-settings-edit-effective-value = 효과적인 가치: {$value}

overlay-choice-clear-settings-detail = {$field}에 대한 파일 override 제거.

overlay-settings-section-plugins-label = 플러그인 및 도구

overlay-settings-section-plugins-summary = 플러그인 구성, 도구, 하네스 및 진단

overlay-settings-section-providers-label = 모델 및 공급자

overlay-settings-section-providers-summary = {$count} 구성 공급자

overlay-settings-section-model-catalog-label = 모델 카탈로그

overlay-settings-section-model-catalog-summary = {$count} 항목

overlay-settings-section-permissions-label = 권한

overlay-settings-section-permissions-summary = {$count} 지속 권한 규칙(s)

overlay-settings-section-tracing-summary = 로그 필터 및 진단

overlay-settings-section-ui-label = 모양

overlay-settings-section-ui-summary = Locale 및 인터페이스 설정

overlay-settings-section-ui-description = 지속적인 언어, 색깔, 도표 및 주제 조정.

overlay-settings-section-runtime-session-label = 런타임 및 세션

overlay-settings-section-runtime-session-summary = 공급자 클라이언트 identities 및 context compaction

settings-permission-global-label = 글로벌 권한

settings-permission-global-detail = 모든 세션에 대한 기본.

settings-permission-workspace-label = 작업 공간 권한

settings-permission-workspace-detail = 현재 프로젝트를 위한 Override 층.

settings-permission-current-label = 현재 세션 권한

settings-permission-current-detail = 현재 세션에만 적용됩니다.

settings-permission-effective-label = 효과적인 권한

settings-permission-layer-global = 주요사업

settings-permission-layer-workspace = 작업 공간

settings-permission-layer-session = 회사연혁

settings-permission-layer-effective = * 필수

settings-runtime-thinking-label = 생각 모드

settings-runtime-thinking-description = current-session는 형태 override를 생각한다

settings-runtime-speed-label = 속도 모드

settings-runtime-speed-description = 현재 보유 속도 모드 override

settings-runtime-verbosity-label = 언어 선택

settings-runtime-verbosity-description = 현재 세션 동사성 override

settings-field-default-provider-label = 기본 모델

settings-field-permission-approval-model-label = 자동 승인 모델

settings-field-ui-locale-label = 언어

settings-field-ui-locale-description = 언어 선택

settings-field-tui-color-scheme-label = 터미널 색 구성표

settings-field-tui-theme-label = TUI 플러그인 테마

settings-field-tui-theme-description = 선택 플러그인 제공된 semantic 색상 팔레트

settings-choice-tui-color-scheme-auto = 터미널 배경을 자동으로 감지

settings-choice-tui-color-scheme-dark = 어두운 끝 배경을 위한 색깔을 낙관하십시오

settings-choice-tui-color-scheme-light = 가벼운 맨끝 배경을 위한 색깔을 낙관하십시오

settings-field-tui-graphics-label = 고급 터미널 그래픽

settings-choice-tui-graphics-auto = 자동으로 네이티브 그래픽을 협상하고 안전하게 Unicode로 돌아갑니다 (추천)

settings-choice-tui-graphics-native = 전문 구성 터미널 경로에 대한 원시 그래픽 협상

settings-choice-tui-graphics-unicode = 기본 그래픽 및 사용 deterministic Unicode/text 렌더링

settings-field-activity-default-expanded-label = 활동 기본 펼치기

settings-field-activity-kind-description = 이 활동 종류에 대한 기본 확장 상태.

settings-field-activity-tool-label = Tool 기본 확장

settings-field-activity-tool-description = 이 정확한 공구를 위한 과태 확장 국가.

settings-activity-kind-reasoning-label = 회사 소개

settings-activity-kind-operation-label = 도구 작업

settings-activity-kind-operation-description = 도구 호출 및 그 결과.

settings-activity-kind-resource-label = 지원하다

settings-activity-kind-resource-description = 첨부 파일 및 기타 리소스 내용.

settings-activity-kind-skill_reference-label = 기술 참조

settings-activity-kind-skill_reference-description = 답변에 사용되는 기술에 대한 참조.

settings-activity-kind-interaction-label = 회사연혁

settings-activity-kind-interaction-description = 사용자 입력 요청 및 대화 형 프롬프트.

settings-activity-kind-hook-label = 훅

settings-activity-kind-hook-description = 세션 후크 실행 및 Lifecycle 이벤트.

settings-activity-kind-error-label = 계정 관리

settings-activity-kind-error-description = 실패된 가동 및 끝 실패.

settings-activity-kind-notice-label = 공지사항

settings-activity-kind-notice-description = 배경 통지 및 정보 행.

settings-activity-kind-text-label = 이름 *

settings-activity-kind-text-description = 일반 텍스트 및 텍스트 artifact 콘텐츠.

settings-field-tracing-filter-label = 애플리케이션 로그 수준

settings-field-tracing-filter-description = 기본값 추적 로그 레벨

settings-field-tracing-database-label = 데이터베이스 로그 수준

settings-field-tracing-database-description = 데이터베이스 추적 로그 레벨

settings-field-tracing-adapter-label = 어댑터 로그 수준

settings-field-tracing-adapter-description = 공급자 어댑터 추적 로그 레벨

settings-config-open-file-detail = 이 경로에 대한 agena.json을 엽니 다

settings-source-unset = 설정하기

settings-source-configured = 구성: {$value}

settings-source-effective = 효과적인: {$value}

settings-source-file-effective = 파일 : {$file} / 유효 : {$effective}

settings-source-file-found = {$path} (확장)

settings-source-file-missing = {$path} (생각될 것입니다)

settings-source-row-config-file = Config 파일

settings-source-row-workspace-config-file = Workspace 설정 파일

settings-source-row-file-value = 파일 값

settings-source-row-workspace-value = Workspace 가치

settings-source-row-effective-value = 효과적인 가치

settings-source-row-write-target = 관련 기사

settings-source-row-layers = 활동 층

settings-source-current-session = 현재 세션 런타임 데이터

settings-source-current-session-runtime = 현재 세션 실행 옵션

settings-detail-values-heading = 제품정보

settings-detail-sources-heading = 이름 *

settings-detail-action-readonly = 읽기 전용 효과적인보기를 엽니 다.

settings-detail-action-file = backing config 파일을 엽니다.

settings-harness-browser-label = 비밀번호

settings-harness-shell-label = 포탄 마구

settings-harness-editor-label = 편집자 마구

settings-field-parse-bool = {$field}는 true/false 또는 on/off와 같은 불린을 기대합니다.

settings-field-parse-integer = {$field}는 불명한 정수값을 기대합니다.

settings-field-parse-float = {$field}는 숫자값을 기대합니다.

settings-choice-adapter-fallback = 어댑터

settings-choice-default-provider-detail = {$adapter}/{$model}

settings-plugin-workbench-label = 플러그인 설정 워크벤치

settings-mcp-server-label = Agena MCP 서버

settings-mcp-server-value = toggle 활성화 / 비활성화

settings-mcp-server-enabled = 이름 *

settings-mcp-server-disabled = 이름 *

settings-mcp-status-unavailable = 상태 unavailable

settings-mcp-ready = 지원하다

settings-mcp-needs-attention = 지원하다

settings-mcp-auth-label = MCP 인증

settings-mcp-auth-none = 익명: 모든 노출 도구

settings-mcp-auth-oauth = 전체 OAuth

settings-mcp-auth-mixed = 혼합: 공공 발견, per-tool OAuth

settings-mcp-anonymous-access-label = 혼합 인증 익명 도구 접근

settings-mcp-anonymous-access-none = 없음 (추천)

settings-mcp-anonymous-access-read-only = permission-contract 읽기 전용 도구

settings-mcp-registration-label = 이름 *

settings-mcp-pkce-label = 사이트맵

settings-mcp-client-registration-label = OAuth 클라이언트 등록

settings-mcp-client-registration-cimd = CIMD (추천)

settings-mcp-client-registration-dcr = CIMD + 동적 클라이언트 등록

settings-mcp-public-url-label = 공개 MCP URL

settings-mcp-public-url-value = 관련 기사

settings-mcp-public-url-auto = 청취자-local fallback

settings-mcp-oauth-issuer-label = OAuth 발급자 URL

settings-mcp-oauth-issuer-derived = MCP 자원 근원에서 파생하는

settings-mcp-oauth-password-label = MCP OAuth 암호

settings-mcp-oauth-password-value = 설정 또는 교체

settings-mcp-oauth-password-configured = MCP 별 비밀번호 설정

settings-mcp-oauth-password-ui-fallback = UI 비밀번호 삭제

settings-mcp-oauth-password-not-configured = 이름 *

settings-mcp-oauth-password-clear-label = MCP OAuth 비밀

settings-field-runtime-codex-version-label = Codex 클라이언트 버전

settings-field-runtime-claude-version-label = Claude 코드 버전

settings-field-runtime-gemini-version-label = Gemini CLI 버전

settings-field-session-compaction-auto-label = 자동 압축

settings-field-session-compaction-reserved-tokens-label = 압축 예약 토큰

settings-client-versions-refresh-label = 클라이언트 버전 새로 고침

settings-client-versions-refresh-value = 최신 정보

settings-client-versions-entry-label = 공급자 클라이언트 버전

settings-client-versions-entry-value = 코덱 · 클로드 · gemini

settings-client-versions-section-label = 클라이언트 버전

settings-client-versions-section-summary = Runtime 정체성 버전

settings-provider-workbench-label = 공급자 명부

settings-provider-workbench-value = {$count} 공급자 (s)

settings-provider-default-mode-inherit-detail = 이 모드에 대한 model/provider 기본값을 사용합니다.

settings-provider-new-label = + 새로운 공급자

settings-provider-existing-detail = {$count} 어댑터 구성

settings-model-catalog-open-label = 모델 카탈로그

settings-files-open-config-label = agena.json을 여십시오

settings-files-open-config-present = 이름 *

settings-files-open-config-create = 공지사항

permission-studio-field-path-workspace = Path Workspace 기본

permission-studio-field-path-external = Path 외부 기본값

permission-studio-field-path-rules = 경로 규칙

permission-studio-field-network-defaults = 네트워크 기본값

permission-studio-field-network-rules = 네트워크 규칙

permission-studio-field-tool-names = 도구 이름

permission-studio-field-tool-rules = 도구 규칙

permission-studio-field-prompt-json = JSON을 {$field}에 입력합니다. 이 override를 취소하려면 편집기를 빈 상태로 둡니다.

permission-studio-detail-override = 관련 제품

permission-studio-detail-effective = * 필수

permission-studio-detail-override-inline = 오버라이드 {$value}

permission-studio-detail-effective-inline = 효과적인 {$value}

permission-studio-detail-read-only = 이 권한 문서는 여기에 읽기 전용입니다.

permission-studio-detail-mode-editable = 이 하나의 필드에 모드 선택기를 엽니다.

permission-studio-detail-text-editable = 이 단일 키 또는 패턴을 편집합니다.

permission-studio-detail-remove-hint = 이 아이템을 즉시 제거하십시오.

permission-studio-detail-navigate-hint = 이 섹션을 엽니다.

permission-studio-overview-target = 제품정보

permission-studio-overview-source = 이름 *

permission-studio-overview-scope = 관련 상품

permission-studio-overview-override = 관련 제품

permission-studio-overview-effective = * 필수

permission-studio-section-workspace = 작업 공간

permission-studio-section-external = 기타 제품

permission-studio-section-rules = 이름 *

permission-studio-section-defaults = 기본 사항

permission-studio-source-global = 글로벌

permission-studio-source-workspace = 작업 공간

permission-studio-source-session = 이름 *

permission-studio-source-effective = 제품 정보

permission-studio-settings-override = 오버라이드 {$value}

permission-studio-settings-effective = 효과적인 {$value}

permission-studio-mode-read = 읽기 {$value}

permission-studio-mode-write = 쓰기 {$value}

permission-studio-network-default = {$label} {$value}

permission-studio-page-overview = 제품정보

permission-studio-page-path = 오시는 길

permission-studio-page-path-defaults = 파일 시스템 / 기본 영역

permission-studio-page-path-rules = 파일 시스템 / 경로 규칙

permission-studio-page-network = 회사연혁

permission-studio-page-network-zones = 네트워크 / 네트워크 영역

permission-studio-page-network-rules = 네트워크 / 도메인 규칙

permission-studio-page-tools = 회사 소개

permission-studio-page-tool-tags = 도구 액세스 / 태그 규칙

permission-studio-page-tool-names = 도구 접근 / 이름 규칙

permission-studio-page-tool-command-rules = 도구 접근 / 명령 규칙

permission-studio-page-names = 이름 *

permission-studio-page-tool-rules = 도구 규칙

permission-studio-nav-overview = 제품정보

permission-studio-nav-filesystem = 파일 시스템

permission-studio-nav-default-zones = 기본 영역

permission-studio-nav-path-rules = 경로 규칙

permission-studio-nav-network = 네트워크

permission-studio-nav-network-zones = 네트워크 영역

permission-studio-nav-domain-rules = 도메인 규칙

permission-studio-nav-tool-access = 도구 접근

permission-studio-nav-name-rules = 이름 규칙

permission-studio-nav-command-rules = 명령 규칙

permission-studio-path-workspace-read = 작업 공간 읽기

permission-studio-path-workspace-write = 작업 공간 쓰기

permission-studio-path-external-read = 외부 읽기

permission-studio-path-external-write = 외부 쓰기

permission-studio-path-rule-read = 읽기 모드

permission-studio-path-rule-write = 쓰기 모드

permission-studio-network-internet = 인터넷 연결

permission-studio-network-private = 한국어

permission-studio-network-loopback = 루프백

permission-studio-tool-default = 도구 기본값

permission-studio-tool-default-summary = 기본 {$value}

permission-studio-add-path-rule = 경로 규칙 추가

permission-studio-add-network-rule = Network Target 추가

permission-studio-add-name = 이름 *

permission-studio-add-tool-rule = 도구 규칙 추가

permission-studio-rule-key = 이름 *

permission-studio-rule-pattern = 제품 정보

permission-studio-rule-target = 제품정보

permission-studio-rule-mode = 주요 특징

permission-studio-tool-rule-fallback = Fallback 형태

permission-studio-error-empty-value = {$field}는 비어있을 수 없습니다.

overlay-providers-title = 회사 소개

overlay-providers-prompt = 기본 모델을 사용하여 공급자를 선택합니다.

overlay-provider-list-title = 공급자 명부

overlay-provider-list-prompt = 회사 소개

overlay-provider-list-footer = Create Provider 또는 기존 공급자를 선택한 다음 Enter를 누릅니다.

overlay-provider-list-create-label = + 새로운 공급자

overlay-provider-list-row-detail-no-model = {$adapter} · {$count} 구성 어댑터

overlay-provider-studio-title = 공급자 Config

overlay-provider-studio-header = 공급자 Config

overlay-provider-studio-footer = 탭/Shift+Tab 패널 · Arrows select · Space toggle · 편집 입력 · Ctrl + D delete selected · Ctrl + R 새로 고침 · Ctrl + N 모델을 추가 · Ctrl + N 어댑터 · Ctrl + S 득점 공급자 · 조건 닫기

overlay-provider-studio-providers = 공급자

overlay-provider-studio-draft = 사이트맵

overlay-provider-studio-adapters = 어댑터

overlay-provider-studio-models = 모델

overlay-provider-studio-catalog = 모델 카탈로그

overlay-provider-studio-detail = 제품 정보

overlay-provider-studio-adapter-models-empty = 어댑터를 선택한 다음 라이브 모델을 나열

overlay-provider-studio-models-empty = 사용 가능한 어댑터 모델이 없습니다

overlay-provider-studio-catalog-empty = 이 쿼리를 일치하는 카탈로그 항목 없음

overlay-provider-studio-new-provider-detail = 빈 공급자 초안

overlay-provider-studio-provider-row-detail-no-model = {$adapter} · {$count} 구성 어댑터

overlay-provider-studio-model-count = {$count} 모델

overlay-provider-studio-loaded = 로드 중 ...

overlay-provider-studio-error = 오류 수정

overlay-provider-studio-configured = 설치하기

overlay-provider-studio-live-list = 비밀번호

overlay-provider-studio-not-listed = 견적 요청

overlay-provider-studio-not-supported = 현재 auth 계약에 의해 지원되지 않음

overlay-provider-studio-edit-title = 연락처

overlay-provider-studio-edit-prompt = 업데이트 {$field}

overlay-provider-studio-edit-footer = 유형 편집

overlay-provider-studio-model-edit-footer = Ctrl+S 득점 모델 구성

overlay-provider-studio-model-json-title = 모형 Config · {$adapter}/{$model}

overlay-provider-studio-model-json-prompt = persisted 공급자 모형 JSON를 편집하십시오.

overlay-provider-studio-model-title = 모형 · {$adapter}/{$model}

overlay-provider-studio-model-footer = 화살표 선택 · 편집 입력 · Ctrl + S 저장 · Ctrl + D 제거 · 에스크로 백업

overlay-provider-delete-title = 회사 소개

overlay-provider-delete-adapter-title = 어댑터 삭제

overlay-provider-delete-model-title = 모델 삭제

overlay-provider-studio-model-edit-title = 편집 모델 필드

overlay-provider-studio-model-field-prompt = 업데이트 {$field}

overlay-provider-studio-new-model-title = 모델 추가

overlay-provider-studio-edit-auth-mode-prompt = 업데이트 auth 모드 (없음 | api | 자격 증명)

overlay-provider-studio-edit-auth-subtype-prompt = auth subtype(api: custom | cline api | gitlab api | bedrock sigv4 · credential: openai chatgpt | github copilot | gitlab | google adc | sap ai core)

overlay-provider-studio-edit-auth-login-method-prompt = 업데이트 auth 로그인 방법 (장치 | 브라우저)

provider-studio-auth-status-pending = 뚱 베어

provider-studio-auth-status-unset = 지원하다

provider-studio-auth-status-none = 이름 *

provider-studio-auth-status-select-subtype = 선택 subtype

provider-studio-auth-status-select-issuer = 선택 subtype

provider-studio-auth-status-configured = 설치하기

provider-studio-auth-status-partial = 이름 *

provider-studio-summary-env = 뚱 베어

provider-studio-summary-callback = 콜백

provider-studio-summary-redirect = 관련 기사

provider-studio-summary-account = 계좌정보

provider-studio-summary-name = 이름 *

provider-studio-summary-user = 사용자 이름

provider-studio-summary-email = 이름 *

provider-studio-summary-profile = 이름 *

provider-studio-summary-region = 이름 *

provider-studio-summary-code = 이름 *

provider-studio-summary-state = 국가 {$state}

provider-studio-summary-tokens-set = 토큰 세트

provider-studio-summary-keys-set = 키 설정

provider-studio-summary-set-field = 설정 {$field}

provider-studio-summary-review-fields = 리뷰 auth 필드

provider-studio-summary-start-browser = 브라우저 OAuth 시작

provider-studio-summary-restart-browser = 브라우저 OAuth를 다시 시작

provider-studio-summary-open-authorize = URL을 편집

provider-studio-summary-start-device = 장치 로그인

provider-studio-summary-restart-device = 장치 로그인

provider-studio-summary-open-verify = 비밀번호

provider-studio-summary-finish-callback = 끝 콜백 교환

provider-studio-summary-poll-every = 모든 {$seconds}s

provider-studio-summary-paste-callback = 페이스 북

provider-studio-summary-poll-now = 현재 위치

provider-studio-summary-start-auth-first = 처음 시작

provider-studio-summary-poll-browser = 비밀번호

provider-studio-auth-openai-ready = 브라우저 OAuth가 준비되어 있습니다. 아래 URL을 엽니다.

provider-studio-auth-openai-device-ready = OpenAI 장치 로그인이 준비되어 있습니다. 아래 검증 URL을 열고 {$code}을 입력하십시오.

provider-studio-auth-authorize = {$url} 인증

provider-studio-auth-redirect = 리디렉션 {$url}

provider-studio-auth-paste-callback = 콜백 URL로 리디렉션된 URL을 붙여, 그 후 p · state {$state}를 누릅니다.

provider-studio-auth-copilot-ready = 장치 로그인이 준비되어 있습니다. 아래 검증 URL을 열고 {$code}을 입력하십시오.

provider-studio-auth-verify = {$url} 인증

provider-studio-auth-poll = 지금 투표로 p를 누르십시오 · 각 {$seconds}s

provider-studio-auth-gitlab-ready = GitLab 브라우저 OAuth가 준비되어 있습니다. 아래 URL을 엽니다.

provider-studio-auth-atomgit-ready = AtomGit 브라우저 세션 준비 · 저자 URL은 아래에 표시됩니다

provider-studio-auth-finish-browser = 브라우저 흐름을 완료하고 p · state {$state}를 누릅니다.

flash-settings-updated = 업데이트 {$path}

flash-settings-cleared = 클리어 {$path}

flash-provider-save-error-settings-object = 기존 공급자 설정은 JSON 객체이어야 합니다.

command-settings-summary = 모델, 권한, 플러그인, 실행 시간, 세션, 인터페이스 및 진단에 대한 통합 설정 작업 벤치를 엽니 다

settings-mcp-public-url-updated = Agena MCP 공개 URL 업데이트

settings-mcp-oauth-issuer-updated = Agena MCP OAuth 발행자 URL 업데이트

settings-mcp-oauth-password-updated = 나이나 MCP OAuth 암호 업데이트

settings-mcp-server-enabled-flash = Agena MCP 서버 활성화

settings-mcp-server-disabled-flash = Agena MCP 서버 비활성화

settings-mcp-auth-mode-updated = Agena MCP 인증 모드 설정 {$mode}

settings-mcp-anonymous-access-updated = Agena MCP 익명 도구 액세스 설정 {$policy}

settings-mcp-client-registration-updated = Agena MCP 클라이언트 등록 설정 {$policy}

settings-mcp-oauth-password-cleared = 나이나 MCP OAuth 암호가 삭제됨

permission-studio-command-pattern-title = {$tool_name} 명령 패턴

settings-tool-api-list-description = 실행 도구.

settings-tool-api-search-description = 검색 실행 도구.

settings-tool-api-help-description = Inspect 실행 도구 계약.

settings-tool-api-tags-description = 실행 도구 태그 목록.

settings-tool-api-call-description = 실행 툴을 호출합니다.

settings-tool-api-plugins-list-description = Enumerate 도구 플러그인.

settings-tool-api-plugins-search-description = 검색 도구 플러그인.

settings-tool-api-plugins-tags-description = 도구 플러그인 태그 목록.

permission-studio-command-pattern-help = 셸 명령 glob 패턴을 입력하세요. 예: `git status` 또는 `git push *`.

permission-studio-rename-unsupported = 이 항목은 이름을 바꿀 수 없습니다. 삭제한 후 다시 만드세요.

# Settings, provider, permission, catalog, MCP, and diagnostics completion
overlay-editor-footer-single-line = 수정하려면 입력하세요.
overlay-editor-footer-multiline = Ctrl+S 저장
context-help-title = 상황별 도움말
context-help-eyebrow = 현재 인터페이스
context-help-footer = ↑/↓ 스크롤 · Esc 또는 Ctrl+H 닫기
context-help-global-hint = Ctrl+H 도움말
context-help-context-composer-items = 작곡가 항목
context-help-context-suggestions = 제안
context-help-context-usage = 사용량 대시보드
context-help-context-plan-viewer = 계획 뷰어
context-help-context-user-input = 사용자 입력 요청
context-help-context-plugin-list = 플러그인 워크벤치 · 목록
context-help-context-plugin-detail = 플러그인 워크벤치 · 세부정보
context-help-context-plugin-config = 플러그인 워크벤치 · 구성
context-help-context-plugin-actions = 플러그인 구성 · 작업
context-help-context-plugin-selection = 플러그인 구성 · 선택
context-help-context-plugin-drilldown = 플러그인 구성 · 드릴다운
context-help-context-plugin-diff = 플러그인 구성 · 차이점
context-help-key-delete = 선택한 항목을 제거합니다.
context-help-key-plugin-restart = 지원되면 선택한 플러그인을 다시 시작하세요.
overlay-permission-title = 허가 요청
overlay-permission-details-title = 세부정보
overlay-permission-action-tool = 도구: { $tool }
overlay-permission-action-path = 경로 { $access }: { $path }
overlay-permission-action-network = 네트워크: { $target }
overlay-permission-field-tool = 도구
overlay-permission-field-target = 명령 또는 대상
overlay-permission-field-access = 액세스
overlay-permission-field-path = 경로
overlay-permission-field-workspace = 작업공간
overlay-permission-field-network = URL 또는 네트워크 대상
overlay-permission-field-host = 호스트
overlay-permission-field-reason = 승인이 필요한 이유
overlay-permission-detail-request-id = 요청 ID
overlay-permission-detail-source = 정책 소스
overlay-permission-detail-scope = 요청된 범위
overlay-permission-detail-operator = 요청자
overlay-permission-detail-trace = 결정 추적
overlay-permission-summary-more-approvals = 또한 이 도구 호출에서 { $count } 추가 작업을 승인합니다.
overlay-permission-detail-requested-actions = 또한 승인을 요청하는 중입니다.
overlay-permission-detail-related-actions = 이 통화에는 이미 허용되었습니다.
overlay-permission-choice-auto-approve = 자동 승인…
overlay-permission-rule-workbench-title = 권한 규칙
overlay-permission-rule-studio-footer = 화살표 선택 · Enter 편집 · Ctrl+O 선택한 경로 찾아보기 · Ctrl+S 저장 · Ctrl+D 취소 · Esc 닫기
overlay-permission-rule-studio-footer-return = 화살표 선택 · Enter 편집 · Ctrl+O 선택한 경로 찾아보기 · Ctrl+S 저장 · Ctrl+D 취소 · Esc는 권한 요청으로 돌아갑니다.
flash-permission-rule-browse-path-selection = 찾아보기 전에 대상 경로 또는 작업공간 루트를 선택하십시오.
overlay-permission-rule-choice-subject-title = 주제 종류 선택
overlay-permission-rule-choice-subject-prompt = 규칙 제목 유형을 선택합니다.
overlay-permission-rule-choice-subject-tool-detail = 도구 또는 런타임 도구 일치
overlay-permission-rule-choice-subject-path-access-detail = 파일 시스템 액세스 일치
overlay-permission-rule-choice-subject-network-access-detail = 네트워크 액세스 일치
overlay-permission-rule-choice-access-title = 경로 액세스 종류 선택
overlay-permission-rule-choice-access-prompt = 파일 시스템 액세스 모드를 선택하십시오.
overlay-permission-rule-choice-access-read-detail = 파일 읽기만 허용
overlay-permission-rule-choice-access-write-detail = 파일 쓰기만 허용
overlay-permission-rule-choice-access-read-write-detail = 읽기와 쓰기를 모두 허용
overlay-permission-rule-choice-scope-title = 규칙 범위 선택
overlay-permission-rule-choice-scope-prompt = 규칙이 얼마나 광범위하게 유지되어야 하는지 선택합니다.
overlay-permission-rule-choice-scope-session-detail = 이번 세션만
overlay-permission-rule-choice-scope-workspace-detail = 이 작업공간의 모든 세션
overlay-permission-rule-choice-scope-global-detail = 모든 작업공간
overlay-permission-rule-choice-mode-title = 규칙 모드 선택
overlay-permission-rule-choice-mode-prompt = 허용, 요청 또는 거부를 선택하세요.
overlay-permission-rule-choice-mode-allow-detail = 항상 일치하는 작업 허용
overlay-permission-rule-choice-mode-auto-detail = 구성된 승인 모델이 결정하도록 합니다. 사용할 수 없을 때 프롬프트로 돌아갑니다.
overlay-permission-rule-choice-mode-ask-detail = 일치하는 작업을 허용하기 전에 프롬프트
overlay-permission-rule-choice-mode-deny-detail = 항상 일치하는 작업을 거부합니다.
overlay-permission-rule-editor-footer = 수정하려면 입력하세요.
overlay-permission-rule-editor-tool-name-title = 도구 이름 편집
overlay-permission-rule-editor-tool-name-prompt = 정확한 도구 이름을 입력하세요.
overlay-permission-rule-editor-qualifier-title = 한정자 편집
overlay-permission-rule-editor-qualifier-prompt = 선택적 한정자를 입력하거나 비워 두세요.
overlay-permission-rule-editor-workspace-root-title = 작업공간 루트 편집
overlay-permission-rule-editor-workspace-root-prompt = 선택적 작업공간_루트 디렉토리를 입력하십시오.
overlay-permission-rule-editor-target-path-title = 대상 경로 편집
overlay-permission-rule-editor-target-path-prompt = 대상 경로나 패턴을 입력하세요.
overlay-permission-rule-editor-network-target-title = 네트워크 대상 편집
overlay-permission-rule-editor-network-target-prompt = 호스트, 호스트:포트 또는 URL을 입력하세요.
overlay-permission-rule-editor-session-id-title = 세션 ID 편집
overlay-permission-rule-editor-session-id-prompt = 대상 세션 ID를 입력하세요.
overlay-permission-rule-browser-workspace-root-title = 작업공간 루트 선택
overlay-permission-rule-browser-workspace-root-prompt = 디렉토리를 찾아보고 Enter를 눌러 하나를 선택하십시오.
overlay-permission-rule-browser-target-path-title = 대상 경로 선택
overlay-permission-rule-browser-target-path-prompt = 파일이나 디렉터리를 찾아보고 Enter 키를 눌러 하나를 선택합니다.
overlay-permission-rule-browser-footer = ../ 또는 디렉토리를 선택하고 Enter를 눌러 찾아보세요. · 값을 선택하고 Enter를 눌러 수락하세요.
overlay-permission-rule-browser-empty = 일치하는 파일이나 디렉터리가 없습니다.
overlay-permission-rule-item-subject-kind = 주제 종류
overlay-permission-rule-item-subject-kind-detail = 이 규칙이 도구, 경로 또는 네트워크 대상에 적용되는지 여부를 선택합니다.
overlay-permission-rule-item-mode = 모드
overlay-permission-rule-item-mode-detail = 일치하는 작업을 허용할지, 요청할지, 거부할지 선택합니다.
overlay-permission-rule-item-scope = 범위
overlay-permission-rule-item-scope-detail = 세션, 작업 영역 또는 전역적으로 이 규칙을 유지합니다.
overlay-permission-rule-item-session-id = 세션 ID
overlay-permission-rule-item-session-id-detail = 범위=세션일 때 사용되는 대상 세션 ID입니다.
overlay-permission-rule-item-tool-name = 도구 이름
overlay-permission-rule-item-tool-name-detail = 일치하는 정확한 도구 이름입니다.
overlay-permission-rule-item-qualifier = 예선
overlay-permission-rule-item-qualifier-detail = 보다 구체적인 도구 규칙에 대한 선택적 한정자입니다.
overlay-permission-rule-item-access-kind = 액세스 종류
overlay-permission-rule-item-access-kind-detail = 읽기, 쓰기 또는 read_write를 선택합니다.
overlay-permission-rule-item-target-path = 대상 경로
overlay-permission-rule-item-target-path-detail = 보호할 경로 패턴 또는 정확한 경로입니다.
overlay-permission-rule-item-workspace-root = 작업공간 루트
overlay-permission-rule-item-workspace-root-detail = 상대 대상 경로를 해석하는 데 사용되는 선택적 기본 디렉터리입니다.
overlay-permission-rule-item-network-target = 네트워크 대상
overlay-permission-rule-item-network-target-detail = 일치시킬 호스트, 호스트:포트 또는 URL 대상입니다.
overlay-permission-rule-detail-subject-kind = 도구 규칙은 도구 이름 및 선택적 한정자와 일치합니다. 경로 규칙은 파일 시스템 액세스와 일치합니다. 네트워크 규칙은 호스트 또는 URL 액세스와 일치합니다.
overlay-permission-rule-detail-tool-name = 공구 규칙에는 정확한 공구 이름이 필요합니다(예: `shell`, `read` 또는 `web_search`).
overlay-permission-rule-detail-qualifier = 한정자는 선택 사항입니다. 도구나 작업에 더 좁은 범위의 일치가 필요한 경우가 아니면 비워 두세요.
overlay-permission-rule-detail-path-access-kind = 일치시키려는 파일 시스템 액세스에 따라 `read`, `write` 또는 `read_write`을 사용하십시오.
overlay-permission-rule-detail-workspace-root = 런타임 작업공간 루트를 상속하려면 작업공간_루트를 비워 두세요. 보호된 경로가 다른 곳에 있는 경우 이를 명시적으로 설정하십시오.
overlay-permission-rule-detail-target-path = 경로나 패턴을 입력하세요. 상대 경로는 설정 시 작업 공간 루트에 대해 해석됩니다.
overlay-permission-rule-detail-network-target = 규칙의 구체적인 정도에 따라 호스트, `host:port` 또는 전체 URL을 입력하세요.
overlay-permission-rule-detail-scope = 세션 범위는 임시 재정의에 가장 적합합니다. 작업공간 및 전역 범위는 더 오래 지속됩니다.
overlay-permission-rule-detail-session-id = 세션 범위 규칙에는 구체적인 세션 ID가 필요합니다.
overlay-permission-rule-detail-mode = 허용은 작업을 허용하고, 승인을 요청하고, 거부하면 차단합니다.
overlay-workbench-details = 세부정보
overlay-permission-studio-title = 허가
overlay-permission-studio-footer-nested = Ctrl+N 추가 · 편집 입력 · Ctrl+E 이름 바꾸기 · Ctrl+D 제거 · Esc 뒤로
permission-studio-catalog-prompt = 라이브 도구 카탈로그를 검색하세요. 하나 이상의 항목을 선택하거나 현재 등록되지 않은 값에 대해 사용자 지정 규칙을 선택합니다.
permission-studio-catalog-custom-detail = 현재 라이브 카탈로그에 없는 태그 또는 도구 이름을 추가하세요.
flash-permission-studio-catalog-empty = 규칙을 추가하기 전에 항목을 하나 이상 선택하세요.
overlay-runtime-setting-current-value = 현재 재정의: { $value }
overlay-settings-help-string = 텍스트를 입력하세요. 파일 재정의를 제거하려면 비워 두거나 `clear`을 입력하세요.
overlay-settings-help-bool = 참/거짓, 설정/해제, 예/아니요 또는 1/0을 입력합니다. 파일 재정의를 제거하려면 비워 두거나 `clear`을 입력하세요.
overlay-settings-help-integer = 정수를 입력하세요. 파일 재정의를 제거하려면 비워 두거나 `clear`을 입력하세요.
overlay-settings-help-float = 숫자를 입력하세요. 재정의를 제거하려면 비워 두거나 `clear`을 입력하세요.
overlay-choice-clear-value = 명확한 가치
overlay-settings-section-plugins-description = 플러그인을 구성하고, 도구 및 진단을 검사하고, 브라우저, 셸 및 편집기 하네스를 관리하세요.
overlay-settings-section-providers-description = 기본 모델 경로를 선택하고, 공급자와 해당 네트워크 동작을 구성하고, 모델 카탈로그를 검사합니다.
overlay-settings-section-model-catalog-description = 확인된 모델 카탈로그를 찾아보고, 모델 메타데이터를 검사하고, 로컬 캐시를 새로 고칩니다.
overlay-settings-section-permissions-description = 전역, 작업 공간 및 현재 세션 권한을 별도로 편집합니다.
overlay-settings-section-runtime-session-description = 호환성 클라이언트 버전 및 자동 세션 압축 동작을 구성합니다.
settings-permission-effective-detail = 읽기 전용 · 전역, 작업 공간 및 세션에서 병합되었습니다.
settings-permission-effective-read-only = 유효 권한은 읽기 전용입니다. 대신 세션, 작업공간 또는 전역 소스를 편집하세요.
settings-field-default-provider-description = 세션 재정의가 활성화되지 않은 경우 사용되는 공급자, 어댑터 및 모델 경로
settings-field-permission-approval-model-description = 자동 권한 결정에 사용되는 모델 및 사고/속도 변형. 사용할 수 없는 선택 항목은 질문으로 대체됩니다.
settings-field-tui-color-scheme-description = 터미널 배경을 자동으로 감지하거나 밝거나 어두운 팔레트를 강제 적용합니다.
settings-field-tui-graphics-description = 지원되는 경우 Kitty, Sixel 또는 iTerm2를 사용하여 이미지 및 조판 수식을 표시합니다. TUI를 다시 시작하면 변경 사항이 적용됩니다.
settings-field-activity-default-expanded-description = 종류별 재정의가 없는 활동의 기본 확장 상태입니다. 추론의 종류가 명시적으로 설정되지 않는 한 추론은 확장된 상태로 유지됩니다.
settings-activity-kind-reasoning-description = 모델의 전체 사고 흔적. 기본값은 확장이며 종류별로 축소할 수 있습니다.
runtime-setting-choice-supported-model = 현재 모델에서 지원됨
settings-plugin-workbench-detail = 런타임 상태, 구성, 도구, 명령, 로그 및 진단을 위한 구조화된 플러그인 워크벤치를 엽니다.
settings-mcp-server-detail = Agena의 라이브 HTTP MCP 표면을 전환합니다. 연결된 Agena 서버 프로세스는 실제 런타임으로 유지됩니다.
settings-mcp-auth-detail = 인증 없음, 전체 OAuth 및 ChatGPT 혼합 인증을 순환합니다. 혼합 모드는 초기화 및 도구 검색을 공개로 유지합니다. 익명 액세스가 명시적으로 활성화되지 않는 한 도구 호출은 OAuth로 보호된 상태로 유지됩니다.
settings-mcp-anonymous-access-none-detail = 안전한 기본값: 도구 호출은 익명이 아닙니다. ChatGPT는 로그인하기 전에 카탈로그를 초기화하고 검색할 수 있습니다.
settings-mcp-anonymous-access-read-only-detail = 고위험 옵트인: 읽기 전용 도구는 익명으로 실행될 수 있으며 개인 작업 공간, 파일 시스템, 구성 또는 진단 데이터를 공개할 수 있습니다.
settings-mcp-anonymous-access-inactive-detail = 이 정책은 혼합 인증 모드에만 적용됩니다. 인증을 혼합으로 전환하여 사용하세요.
settings-mcp-client-registration-cimd-detail = OpenAI ChatGPT 클라이언트 ID 메타데이터 문서만 허용합니다. 인증되지 않은 공개 DCR 엔드포인트는 비활성화된 상태로 유지됩니다.
settings-mcp-client-registration-dcr-detail = 호환 모드: 공개 동적 클라이언트 등록도 노출됩니다. 클라이언트가 CIMD를 사용할 수 없는 경우에만 활성화합니다.
settings-mcp-public-url-detail = 표준 HTTPS MCP 리소스 URL을 설정합니다. 보안 MCP 터널 URL에는 전체 /v1/mcp/tunnel_id 경로가 포함될 수 있습니다. 전달된 요청 헤더는 OAuth ID로 신뢰되지 않습니다.
settings-mcp-oauth-issuer-detail = 공용 브라우저에 표시되는 인증 서버 발급자를 설정합니다. Agena 관리 OAuth에는 https://auth.example.com과 같은 경로가 없는 원본이 필요합니다. OAuth와 MCP가 동일한 도메인을 사용하는 경우에는 비워 두세요.
settings-mcp-oauth-password-detail = Agena OAuth 인증 페이지에 표시된 비밀번호를 설정하세요. 이는 서버에 Argon2 해시로 저장됩니다.
settings-mcp-oauth-password-clear-detail = MCP 특정 비밀번호를 제거하고 구성된 경우 서버 UI 비밀번호로 대체합니다.
settings-field-runtime-codex-version-description = 공급자 요청 ID 헤더에 사용되는 정확한 @openai/codex 호환 버전입니다.
settings-field-runtime-claude-version-description = 공급자 요청 ID 헤더에 사용되는 정확한 @anthropic-ai/claude-code 호환성 버전입니다.
settings-field-runtime-gemini-version-description = 공급자 요청 ID 헤더에 사용되는 정확한 @google/gemini-cli 호환 버전입니다.
settings-field-session-compaction-auto-description = 컨텍스트 창 제한에 접근하면 자동으로 세션을 압축합니다.
settings-field-session-compaction-reserved-tokens-description = 압축 시기를 결정할 때 컨텍스트 창에서 예약된 토큰입니다. 계산된 기본값을 사용하려면 선택을 취소하세요.
settings-client-versions-refresh-description = npm에서 최신 호환 패키지 버전을 가져오고, 세 가지 정확한 값을 모두 유지하고, 런타임을 다시 로드하세요.
settings-client-versions-entry-detail = 공급자 요청 ID 헤더에 사용된 정확한 호환성 버전을 엽니다.
settings-client-versions-section-description = 공급자 요청 ID 헤더에 사용되는 정확한 호환성 버전입니다. 각 값을 편집하거나 Ctrl+R을 눌러 npm에서 새로고침하세요.
settings-provider-workbench-detail = 인증, 어댑터, 모델 라우팅 또는 새 공급자를 구성하기 전에 검색 가능한 공급자 목록을 엽니다.
settings-provider-new-detail = 새 공급자를 만들고, 라이브 어댑터 모델을 나열하고, 공급자 어댑터 구성을 편집합니다. 글로벌 모델을 별도로 선택하십시오.
settings-model-catalog-open-detail = 확인된 모델 메타데이터를 검사하고 로컬 모델 카탈로그 캐시를 새로 고칩니다.
permission-studio-command-rules-shell-only = 명령 규칙은 쉘 도구(shell / bash / agena.shell.run / agena.process.run)에만 적용됩니다. 다른 도구의 경우 이름 규칙이나 기본값을 사용하세요.
permission-studio-detail-editable = Enter를 누르면 이 권한 슬라이스에 대한 여러 줄 JSON 편집기가 열립니다.
permission-studio-detail-add-hint = Enter를 누르면 이 항목이 생성되고 즉시 열립니다.
permission-studio-detail-full-config-editable = Enter를 누르면 전체 문서에 대한 고급 JSON 편집기가 열립니다.
overlay-permission-studio-delete-title = 규칙 삭제
overlay-permission-studio-delete-body = { $kind } 삭제: { $value }
flash-permission-studio-no-add = 현재 섹션에는 항목을 추가할 수 없습니다.
flash-permission-studio-no-delete = 현재 섹션에서는 항목을 삭제할 수 없습니다.
flash-permission-studio-no-selection = 먼저 항목을 선택하세요.
flash-permission-studio-context-lost = 권한 편집기 컨텍스트가 손실되었습니다. 권한 스튜디오를 다시 열고 다시 시도해 보세요.
value-default = 기본값
value-none = 없음
value-clear = 명확한
value-path = 경로
value-network = 네트워크
value-workspace = 작업 공간
value-external = 외부
value-permission-filesystem = 파일 시스템
value-permission-network = 네트워크
value-permission-tools = 도구
value-rule-count = { $count } 규칙
value-custom = 관습
value-internet = 인터넷
value-private = 비공개
value-loopback = 루프백
value-name-count = { $count } 이름
value-rule-set-count = { $count } 규칙 세트
value-open = 열다
composer-prompt-history-title = 프롬프트 내역
overlay-commands-title = 명령 팔레트
overlay-commands-prompt = 검색 활동; 텍스트가 필요한 명령은 작성기에서 계속됩니다.
overlay-skill-studio-title = 기술 관리
overlay-lineage-title = 지점 연혁 [#{ $session }]
overlay-lineage-prompt = 현재 분기 트리를 탐색하고 상위, 형제 또는 하위 세션으로 이동합니다.
overlay-rewind-title = 세션 되감기 [#{ $session }]
overlay-rewind-prompt = 철회할 사용자 메시지와 그 이후의 모든 메시지를 선택하세요.
overlay-picker-loading = 로드 중...
overlay-picker-empty = 일치하는 항목이 없습니다.
overlay-picker-footer = 탭에서 선택한 라벨 채우기
session-model-context-window = { $value } ctx
session-model-max-output = { $value } 밖으로
overlay-provider-studio-detail-footer = 화살표 키 선택 · 편집 입력 · Esc 뒤로; 인증 작업은 기본 공급자 페이지에 표시됩니다.
overlay-provider-studio-configured-disk = 디스크에 구성됨 현재 인증 계약의 일부가 아닙니다.
overlay-provider-studio-new-model-prompt = 선택한 어댑터 아래에 추가할 모델 ID를 입력하세요.
provider-field-provider-id = 제공자 ID
provider-field-auth-mode = 인증 모드
provider-field-auth-subtype = 인증 하위 유형
provider-field-auth-login-method = 인증 로그인 방법
provider-field-start-auth = 인증 시작
provider-field-continue-auth = 계속 인증
provider-field-auth-details = 인증 세부정보
provider-field-base-url = 기본 URL
provider-field-instance-url = 인스턴스 URL
provider-field-api-key-source = API 키 소스
provider-field-api-key-value = API 키 값
provider-field-redirect-uri = 리디렉션 URI
provider-field-callback-url = 콜백 URL
provider-field-refresh-token = 새로고침 토큰
provider-field-access-token = 액세스 토큰
provider-field-expires-at-ms = 만료 시간(밀리초)
provider-field-account-id = 계정 ID
provider-field-enterprise-domain = 엔터프라이즈 도메인
provider-field-region = 지역
provider-field-profile = 프로필
provider-field-access-key-id = 액세스 키 ID
provider-field-secret-access-key = 비밀 액세스 키
provider-field-session-token = 세션 토큰
provider-field-service-key-env = 서비스 키 환경
provider-field-default-adapter = 기본 어댑터
provider-field-request-timeout = 요청 시간 초과(초)
provider-field-connect-timeout = 연결 시간 초과(초)
provider-field-adapter-id = 어댑터 ID
provider-field-model-id = 모델 ID
provider-model-field-model-id = 모델 ID
provider-model-field-enabled = 활성화됨
provider-model-field-native-compaction = 기본 압축
provider-model-field-agena-tool-mode = 도구 모드(agena_tools.mode)
agena-tool-mode-provider-protocol-label = 공급자_프로토콜
agena-tool-mode-provider-protocol-detail = 공급자 API의 도구 프로토콜을 통해 Agena 관리 도구 정의 및 호출을 전송합니다.
agena-tool-mode-disabled-label = 장애인
agena-tool-mode-disabled-detail = Agena 관리 또는 공급자 기본 도구를 이 모델에 노출하지 마십시오.
provider-model-field-display-name = 표시 이름
provider-model-field-lifecycle = 수명주기
provider-model-field-context-window = 컨텍스트 창
provider-model-field-max-input = 최대 입력
provider-model-field-max-output = 최대 출력
provider-model-field-features = 특징
provider-model-field-input-modalities = 입력 방식
provider-model-field-output-modalities = 출력 방식
provider-model-field-thinking-modes = 사고 모드
provider-model-field-speed-modes = 속도 모드
provider-model-field-description = 설명
provider-model-enabled-detail = 이 모델 경로가 활성화되어 있는지 여부입니다.
provider-model-native-compaction-detail = Agena의 텍스트 요약기로 돌아가기 전에 이 공급자의 기본 대화 압축 엔드포인트를 사용해 보세요.
provider-model-lifecycle-detail = 모델 수명주기 값.
provider-auth-mode-none-detail = 공급자 인증 메타데이터 비활성화
provider-auth-mode-api-detail = 사용자 정의 HTTP 엔드포인트, Cline API, GitLab 게이트웨이 토큰 또는 Bedrock SigV4에 대한 2단계 하위 유형을 사용하는 API 스타일 인증
provider-auth-mode-credential-detail = 자격 증명 지원 인증은 인증 하위 유형 필드에서 선택된 로컬 발급자로부터 확인됩니다.
provider-auth-kind-unset = 설정되지 않음
provider-auth-kind-none = 없음
provider-auth-kind-api = API
provider-auth-kind-cline = cline_api
provider-auth-kind-gitlab = gitlab_api
provider-auth-kind-credential = 자격 증명
provider-auth-kind-credential-with-issuer = 자격 증명:{ $issuer }
provider-auth-kind-bedrock = bedrock_sigv4
provider-auth-subtype-custom-label = 관습
provider-auth-subtype-custom-detail = OpenAI 호환, Anthropic 또는 Gemini HTTP 공급자를 위한 일반 API 키 + 기본 URL 인증
provider-auth-subtype-cline-api-detail = Cline API 엔드포인트를 수정했습니다. API 키 입력만 필요하며 모델 검색은 Cline 권장 모델을 사용합니다.
provider-api-key-source-inline-detail = 공급자 구성에 API 키 인라인을 저장합니다.
provider-api-key-source-env-detail = 환경 변수에서 API 키 읽기
provider-auth-subtype-gitlab-api-detail = openai 또는 Anthropic 어댑터를 통해 라우팅되는 GitLab 토큰 인증
provider-auth-subtype-bedrock-detail = AWS Bedrock SigV4 서명
provider-auth-login-kind-browser-label = 브라우저 OAuth
provider-auth-login-kind-device-label = 장치 코드 로그인
provider-auth-login-kind-browser-detail = 승인 URL을 연 다음 리디렉션된 콜백을 완료하세요.
provider-auth-login-kind-device-detail = 짧은 확인 URL을 열고 장치 코드를 입력한 후 폴링하세요.
provider-issuer-openai-chatgpt-label = openai_chatgpt
provider-issuer-github-copilot-label = github_copilot
provider-issuer-gitlab-label = gitlab
provider-issuer-google-adc-label = google_adc
provider-issuer-sap-ai-core-label = sap_ai_core
provider-issuer-openai-chatgpt-detail = OpenAI ChatGPT 자격 증명
provider-issuer-github-copilot-detail = GitHub Copilot 자격 증명
provider-issuer-gitlab-detail = GitLab OAuth 자격 증명
provider-issuer-google-adc-detail = Google 애플리케이션 기본 자격 증명
provider-issuer-sap-ai-core-detail = SAP AI Core 서비스 키 인증
provider-instance-url-gitlab-detail = GitLab.com 브라우저 OAuth 엔드포인트
provider-redirect-local-copy-detail = OAuth 리디렉션 복사/붙여넣기를 위한 localhost 콜백 URL
provider-region-choice-detail = AWS 지역
provider-service-key-env-detail = 기본 SAP AI Core 서비스 키 env var
overlay-model-catalog-field-model-id = 모델 ID
overlay-model-catalog-field-display = 디스플레이
overlay-model-catalog-field-origin = 원산지
overlay-model-catalog-field-lifecycle = 수명주기
overlay-model-catalog-field-dates = 날짜
overlay-model-catalog-field-limits = 한도
overlay-model-catalog-field-inputs = 입력
overlay-model-catalog-field-output = 출력
overlay-model-catalog-field-features = 특징
overlay-model-catalog-field-modes = 모드
overlay-model-catalog-field-defaults = 기본값
overlay-model-catalog-field-runtime = 런타임
overlay-model-catalog-field-pricing = 가격
overlay-model-catalog-field-source = 소스
overlay-model-catalog-limits = ctx { $context } · in { $input } · out { $output }
overlay-model-catalog-lifecycle-active = 활성
overlay-model-catalog-lifecycle-preview = 미리보기
overlay-model-catalog-lifecycle-beta = 베타
overlay-model-catalog-lifecycle-alpha = 알파
overlay-model-catalog-lifecycle-experimental = 실험적인
overlay-model-catalog-lifecycle-deprecated = 더 이상 사용되지 않음
overlay-model-catalog-date-release = { $value } 출시
overlay-model-catalog-date-updated = { $value } 업데이트됨
overlay-model-catalog-date-cutoff = 마감 { $value }
overlay-model-catalog-default-thinking = 생각하다
overlay-model-catalog-default-speed = 속도
overlay-model-catalog-thinking-modes = 사고방식
overlay-model-catalog-speed-modes = 속도 모드
overlay-model-catalog-default-verbosity = 장황함
overlay-model-catalog-default-temperature = 온도
overlay-model-catalog-default-top-p = top_p
overlay-model-catalog-default-top-k = top_k
overlay-model-catalog-parallel-tools = 병렬 도구
overlay-model-catalog-supports-verbosity = 장황함
overlay-model-catalog-reasoning-interleaved = 인터리브 추론
overlay-model-catalog-reasoning-field = 추론 분야
overlay-model-catalog-open-weights = 오픈 웨이트
overlay-model-catalog-price-input = { "$" }{ $value }/M
overlay-model-catalog-price-output = 밖으로 { "$" }{ $value }/M
overlay-model-catalog-price-cache-read = 캐시 읽기 { "$" }{ $value }/M
overlay-model-catalog-price-cache-write = 캐시 쓰기 { "$" }{ $value }/M
overlay-model-catalog-tier-count = { $count } 등급
permission-rule-label-path = { $access } · { $path }
permission-rule-label-network = 네트워크 · { $target }
value-unset = 설정되지 않음
value-auto = 자동
value-allow = 허용하다
value-ask = 묻다
value-deny = 부정하다
value-read = 읽다
value-write = 쓰다
value-read-write = 읽기_쓰기
value-yes = 응
value-no = 아니
value-session = 세션
value-global = 글로벌
value-add = 추가
value-runtime-default = 런타임 기본값
value-permission-rule-subject-tool = 도구
value-permission-rule-subject-path-access = 경로_액세스
value-permission-rule-subject-network-access = 네트워크_액세스
inline-fact-source = 출처
inline-fact-scope = 범위
inline-fact-operator = 운영자
flash-permission-rule-saved = 저장된 권한 규칙: { $name }
flash-permission-rule-revoked = 취소된 권한 규칙: { $name }
flash-permission-rule-context-lost = 권한 규칙 스튜디오 컨텍스트가 손실되었습니다.
flash-provider-studio-context-lost = 공급자 구성 컨텍스트가 손실되었습니다.
permission-rule-error-session-id-integer = 세션 ID는 정수여야 합니다.
permission-rule-error-tool-name-required = 도구 규칙에는 도구 이름이 필요합니다.
permission-rule-error-path-access-kind-required = 경로 규칙에는 path_access_kind가 필요합니다.
permission-rule-error-target-path-required = 경로 규칙에는 target_path가 필요합니다.
permission-rule-error-network-target-required = 네트워크 규칙에는 네트워크 대상이 필요합니다.
permission-rule-error-session-id-required = 세션 범위에는 세션 ID가 필요합니다
flash-server-config-edit-in-settings = 구성 파일은 서버에 속합니다. 클라이언트-로컬 경로를 여는 대신 설정에서 해당 값을 편집하세요.
flash-command-requires-session = 이 작업을 수행하려면 공개 세션이 필요합니다.
flash-session-busy = 세션이 바빠요
flash-provider-selected = 선택한 제공업체: { $provider }(기본값 { $model })
flash-provider-cleared = 공급자/모델 재정의가 지워졌습니다.
flash-provider-not-found = 공급자를 찾을 수 없습니다: { $provider }
flash-provider-default-updated = 기본 공급자 경로 업데이트됨: { $provider }/{ $model }
flash-permission-approval-model-updated = 자동 승인 모델 업데이트됨: { $provider }/{ $model }
flash-provider-studio-adapter-required = 먼저 어댑터를 선택하세요
flash-provider-studio-adapter-not-enabled = 모델을 추가하기 전에 선택한 어댑터를 확인하세요
flash-provider-studio-adapter-unavailable = 현재 인증 모드에서는 이 어댑터를 선택할 수 없습니다.
flash-provider-studio-model-required = 나열된 모델을 먼저 선택하세요
flash-provider-studio-model-id-required = 모델 ID가 필요합니다
flash-provider-studio-no-auth-details = 현재 인증 모드에는 인증 세부정보가 없습니다.
flash-provider-studio-catalog-refreshed = 새로워진 모델 카탈로그
flash-provider-studio-invalid-model-json = 잘못된 모델 JSON: { $error }
flash-provider-studio-live-listing-unavailable = 인증 { $auth }에는 실시간 모델 목록을 사용할 수 없습니다.
flash-provider-studio-draft-listing-unsupported = 초안 모델 목록은 라이브 모델 검색이 가능한 어댑터만 지원합니다. 지원되지 않음: { $adapters }
flash-provider-studio-listing-auth-required = 어댑터 모델을 나열하려면 현재 인증/어댑터 쌍 또는 기존에 저장된 공급자에 대한 실시간 모델 검색이 필요합니다. 현재 인증은 { $auth }입니다.
flash-provider-studio-invalid-auth-login-method = 잘못된 인증 로그인 방법
flash-provider-auth-openai-browser-started = OpenAI 브라우저 인증이 시작되었습니다. 대화 상자에 표시된 인증 URL을 연 다음 리디렉션된 URL을 콜백 URL에 붙여넣고 p를 누르세요.
flash-provider-auth-openai-device-started = OpenAI 장치 로그인이 시작되었습니다. 대화 상자에 표시된 확인 URL을 열고 코드 { $code }을 입력한 다음 p를 누르세요.
flash-provider-auth-copilot-device-started = Copilot 장치 로그인이 시작되었습니다. 대화 상자에 표시된 확인 URL을 열고 코드 { $code }을 입력한 다음 p를 누르세요.
flash-provider-auth-gitlab-browser-started = GitLab 브라우저 인증이 시작되었습니다. 대화 상자에 표시된 인증 URL을 연 다음 리디렉션된 URL을 콜백 URL에 붙여넣고 p를 누르세요.
flash-provider-auth-atomgit-browser-started = AtomGit 브라우저 인증이 시작되었습니다. 대화 상자에 표시된 인증 URL을 열고 로그인을 완료한 다음 p를 눌러 폴링합니다.
flash-provider-auth-openai-captured = OpenAI OAuth 자격 증명이 초안에 캡처되었습니다.
flash-provider-auth-openai-pending = OpenAI 장치 로그인이 아직 보류 중입니다. 확인 단계를 마친 후 다시 p를 누르세요.
flash-provider-auth-copilot-pending = Copilot 장치 로그인이 아직 보류 중입니다. 브라우저 승인을 완료한 후 다시 p를 누르세요.
flash-provider-auth-copilot-captured = Copilot OAuth 자격 증명이 초안에 캡처되었습니다.
flash-provider-auth-gitlab-captured = GitLab OAuth 자격 증명이 초안에 캡처되었습니다.
flash-provider-auth-atomgit-pending = AtomGit 브라우저 로그인이 아직 보류 중입니다. 브라우저 흐름을 마친 다음 p를 다시 누릅니다.
flash-provider-auth-atomgit-captured = AtomGit OAuth 자격 증명이 초안에 캡처되었습니다.
flash-provider-auth-error-unsupported = 현재 인증 모드는 대화형 OAuth 로그인을 지원하지 않습니다.
flash-provider-auth-error-start-browser-first = 먼저 Start Auth 또는 o를 사용하여 브라우저 인증을 시작하세요.
flash-provider-auth-error-start-device-first = Start Auth 또는 o를 사용하여 먼저 장치 인증을 시작하세요.
flash-provider-auth-error-required-field = { $field }이 필요합니다
flash-provider-save-draft = { $adapter } 어댑터를 사용하여 공급자 { $provider }을 저장했습니다.
flash-provider-save-adapter-matches = { $provider }/{ $adapter }을 { $listed } 나열된 모델로 저장했습니다. { $matched } 카탈로그가 일치합니다.
flash-provider-save-model = { $provider }/{ $adapter }/{ $model }에 저장되었습니다.
flash-provider-save-configured-model = 구성된 모델 { $provider }/{ $adapter }/{ $model }을 저장했습니다.
flash-provider-delete-provider = 제공업체 { $provider }을(를) 삭제했습니다.
flash-provider-delete-adapter = 구성된 어댑터 { $provider }/{ $adapter }을 삭제하고 { $count } 모델을 제거했습니다.
flash-provider-delete-model = 구성된 모델 { $provider }/{ $adapter }/{ $model }을 삭제했습니다.
flash-provider-studio-adapter-delete-empty = 삭제할 어댑터 설정을 선택하지 않았습니다.
flash-provider-save-error-required-field = { $field }이 필요합니다
flash-provider-save-error-unsupported-default-adapter = 인증 { $auth }은 defaults.adapter `{ $adapter }`을 지원하지 않습니다. { $supported } 중 하나가 필요합니다.
flash-provider-save-error-unsupported-adapters = 인증 { $auth }은 어댑터를 지원하지 않습니다: { $adapters }; { $supported } 중 하나가 필요합니다.
flash-provider-save-error-api-base-url = OpenAI 프로토콜, Anthropic 또는 Gemini 어댑터를 사용할 때 API 인증에는 base_url이 필요합니다.
flash-provider-save-error-gitlab-token = gitlab_api 인증에는 API 키 소스가 필요합니다
flash-provider-save-error-credential-base-url = 자격증명 발급자 `{ $issuer }`에는 base_url이 필요합니다.
flash-provider-save-error-credential-service-key-env = 자격 증명 발급자 `{ $issuer }`에는 service_key_env가 필요합니다.
flash-provider-save-error-bedrock-key-pair = bedrock_sigv4에는 access_key_id와 secret_access_key가 함께 필요합니다.
flash-provider-save-error-select-model = 공급자를 저장하기 전에 하나 이상의 모델을 선택하십시오.
flash-provider-save-error-adapter-object = 공급자 어댑터 `{ $adapter }`은 JSON 개체여야 합니다.
flash-provider-save-error-model-object = 공급자 모델 구성은 JSON 객체여야 합니다.
flash-provider-save-error-configured-adapter-object = 구성된 공급자 어댑터 설정은 JSON 객체여야 합니다.
flash-provider-save-error-configured-models-object = 구성된 공급자 어댑터 모델은 JSON 객체여야 합니다.
flash-provider-client-versions-refreshed = 업데이트된 클라이언트 버전: Codex { $codex }, Claude { $claude }, Gemini { $gemini }
terminal-diagnostics-title = 터미널 진단
terminal-diagnostics-eyebrow = 호환성 및 프로토콜 증거
terminal-diagnostics-footer = ↑/↓ 스크롤 · c/y 보고서 복사 · Esc 닫기
terminal-diagnostics-tip = 제품 ID 및 환경 계층은 증거 기반입니다. 일반 SSH는 실제 엔드포인트 터미널을 증명할 수 없습니다.
terminal-diagnostics-copied = 터미널 진단이 복사되었습니다.
terminal-diagnostics-unavailable = 이 런타임에서는 터미널 진단을 사용할 수 없습니다.
terminal-diagnostics-summary = 증거 지원 단말 보고서 · 엔드포인트 신뢰도 { $confidence }
terminal-diagnostics-none = 없음
terminal-diagnostics-unknown = 알 수 없음
terminal-diagnostics-unavailable-value = 이용할 수 없음
terminal-diagnostics-term-unset = TERM이 설정되지 않았습니다.
terminal-diagnostics-section-identity = 아이덴티티
terminal-diagnostics-section-layers = 환경 레이어
terminal-diagnostics-section-color = 색상 및 외관
terminal-diagnostics-section-protocols = 활성 프로토콜
terminal-diagnostics-section-providers = 공급자 및 통합
terminal-diagnostics-section-warnings = 경고
terminal-diagnostics-field-product = 제품
terminal-diagnostics-field-version = 버전
terminal-diagnostics-field-parsed-version = 구문 분석된 버전
terminal-diagnostics-field-compatibility = 호환성
terminal-diagnostics-field-confidence = 자신감
terminal-diagnostics-field-source = 선택한 소스
terminal-diagnostics-field-evidence = 증거
terminal-diagnostics-field-conflicts = 충돌
terminal-diagnostics-color-configured = 구성된 모드
terminal-diagnostics-color-detected-background = 감지된 배경
terminal-diagnostics-color-detected-appearance = 감지된 모습
terminal-diagnostics-color-source = 탐지 소스
terminal-diagnostics-color-refresh = 자동 새로고침
terminal-diagnostics-color-generation = 외관 생성
terminal-diagnostics-color-effective-appearance = 효과적인 텍스트 팔레트
terminal-diagnostics-color-formula-foreground = 수식 문자 모양 색상
terminal-diagnostics-color-formula-background = 수식 이미지 배경
terminal-diagnostics-color-background-images = 배경 이미지
terminal-diagnostics-color-mode-auto = 자동
terminal-diagnostics-color-mode-dark = 강제 다크
terminal-diagnostics-color-mode-light = 강제 조명
terminal-diagnostics-color-appearance-dark = 어둠
terminal-diagnostics-color-appearance-light = 빛
terminal-diagnostics-color-appearance-unknown = 알 수 없음
terminal-diagnostics-color-appearance-conservative = 보수적인 터미널 기본 색상(배경을 알 수 없음)
terminal-diagnostics-color-source-osc11 = OSC 11 터미널 응답
terminal-diagnostics-color-source-iterm-osc4 = iTerm2 OSC 4;-2 터미널 응답
terminal-diagnostics-color-source-colorfgbg = COLORFGBG 환경 대체
terminal-diagnostics-color-source-term-background = TERM_BACKGROUND 환경 대체
terminal-diagnostics-color-source-vscode-theme = VSCODE_THEME_KIND 환경 대체
terminal-diagnostics-color-source-unavailable = 사용 가능한 터미널 또는 환경 증거가 없습니다.
terminal-diagnostics-color-refresh-live = 초점 회복 및 터미널 재개 시; 새로 고침이 실패하면 마지막으로 알려진 색상이 유지됩니다.
terminal-diagnostics-color-refresh-startup-only = 시작 전용; 터미널이 새로 고침 가능한 색상 쿼리에 응답하지 않았습니다.
terminal-diagnostics-color-formula-background-transparent = 투명함; 수식 문자 모양 색상만 모양을 따릅니다.
terminal-diagnostics-color-background-images-not-sampled = 샘플링되지 않았습니다. 투명한 수식 픽셀은 터미널 배경 또는 배경 이미지를 아래에 유지합니다.
terminal-diagnostics-direct = 직접
terminal-diagnostics-direct-description = SSH, Mosh, 멀티플렉서 또는 WSL 증거가 감지되지 않았습니다.
terminal-diagnostics-layer-description = { $source }에서 감지되었습니다. 레이어 순서와 중첩 깊이를 알 수 없습니다.
terminal-diagnostics-capability-description = 끝점={ $status } · 소스={ $source } · 경로={ $path } · 공급자={ $provider }
terminal-diagnostics-path-clear = 명확한
terminal-diagnostics-path-forced = 재정의로 강제됨
terminal-diagnostics-path-unverified = 확인되지 않은
terminal-diagnostics-path-blocked = 차단됨
terminal-diagnostics-provider-not-required = 필요하지 않음
terminal-diagnostics-provider-ready = 준비
terminal-diagnostics-provider-missing = 누락되었거나 구현되지 않음
terminal-diagnostics-helper-missing = 찾을 수 없거나 실행할 수 없습니다.
terminal-diagnostics-helper-not-probed = 엔드포인트가 키티로 식별되지 않아 프로브되지 않았습니다.
terminal-diagnostics-no-warnings = 호환성 경고가 감지되지 않았습니다.
terminal-diagnostics-protocol-alternate-screen = 대체 화면
terminal-diagnostics-protocol-bracketed-paste = 괄호로 묶인 페이스트
terminal-diagnostics-protocol-focus = 집중보고
terminal-diagnostics-protocol-mouse = 마우스 캡처
terminal-diagnostics-protocol-mouse-mode = 마우스 와이어 모드
terminal-diagnostics-protocol-mouse-events = 마우스 이벤트 수신됨
terminal-diagnostics-protocol-mouse-last = 마지막 마우스 이벤트
terminal-diagnostics-mouse-mode-button-sgr = SGR 좌표(DECSET 1006)를 사용한 버튼 이벤트 추적(DECSET 1002)
terminal-diagnostics-mouse-events-none = 없음. 엔드포인트 터미널이 Agena에 마우스 이벤트를 전달하지 않았습니다. 마우스 보고 및 휠 보고 프로필 설정을 확인하세요.
terminal-diagnostics-mouse-events-seen = { $count } 이벤트
terminal-diagnostics-mouse-last-none = 없음
terminal-diagnostics-protocol-keyboard = 키보드 명확성
terminal-diagnostics-protocol-key-events = 키보드 이벤트 유형
terminal-diagnostics-protocol-background = 백그라운드 쿼리
terminal-diagnostics-protocol-native-clipboard = 기본 클립보드
terminal-diagnostics-protocol-osc52-write = OSC 52 쓰기
terminal-diagnostics-protocol-osc52-read = OSC 52 읽기
terminal-diagnostics-protocol-progress = OSC 9;4 진행
terminal-diagnostics-provider-kitty-clipboard = 키티 클립보드
terminal-diagnostics-provider-kitty-transfer = 키티 전송
terminal-diagnostics-provider-iterm-transfer = iTerm2 전송
terminal-diagnostics-provider-inline-images = 인라인 이미지
terminal-diagnostics-provider-hyperlinks = 하이퍼링크
terminal-diagnostics-provider-sync-output = 동기화된 출력
terminal-diagnostics-status-confirmed = 확인됨
terminal-diagnostics-status-forced = 재정의로 강제됨
terminal-diagnostics-status-profiled = 프로파일링된
terminal-diagnostics-status-unsupported = 지원되지 않는
terminal-diagnostics-status-unknown = 알 수 없음
terminal-diagnostics-source-user = 사용자 재정의
terminal-diagnostics-source-environment = 환경
terminal-diagnostics-source-helper = 도우미 프로브
terminal-diagnostics-source-terminal-query = 터미널 쿼리
terminal-diagnostics-source-profile = 터미널 프로필
terminal-diagnostics-source-platform = 플랫폼 기본값
terminal-diagnostics-source-conservative = 보수적 기본값
terminal-diagnostics-source-terminfo = 용어 정보 호환성
terminal-diagnostics-source-unknown = 알 수 없음
terminal-diagnostics-confidence-explicit = 명시적인
terminal-diagnostics-confidence-strong = 강한
terminal-diagnostics-confidence-compatibility = 호환성만
terminal-diagnostics-confidence-unknown = 알 수 없음


# Plugin Workbench i18n completion
plugin-workbench-action-diff = 차이
plugin-workbench-action-refresh = 새로 고침
plugin-workbench-action-remove-selected = 선택 항목 제거/초기화
plugin-workbench-action-reset-all = 모두 초기화
plugin-workbench-action-restart = 다시 시작
plugin-workbench-action-save = 저장
plugin-workbench-action-validate = 검증
plugin-workbench-actions = 작업
plugin-workbench-authority-unavailable = 권한 데이터를 사용할 수 없습니다.
plugin-workbench-choices = 선택 항목
plugin-workbench-close-footer = Esc 닫기
plugin-workbench-column-after = 변경 후
plugin-workbench-column-args = 인수
plugin-workbench-column-arguments = 인수
plugin-workbench-column-before = 변경 전
plugin-workbench-column-category = 범주
plugin-workbench-column-change = 변경
plugin-workbench-column-command = 명령
plugin-workbench-column-description = 설명
plugin-workbench-column-field = 필드
plugin-workbench-column-inputs = 입력
plugin-workbench-column-message = 메시지
plugin-workbench-column-plugin = 플러그인
plugin-workbench-column-section = 섹션
plugin-workbench-column-severity = 심각도
plugin-workbench-column-source = 출처
plugin-workbench-column-summary = 요약
plugin-workbench-column-tool = 도구
plugin-workbench-column-version = 버전
plugin-workbench-column-visible-tool = 표시 도구
plugin-workbench-command-arguments = 인수: {$command}
plugin-workbench-config = 설정
plugin-workbench-config-action = 작업
plugin-workbench-config-choose-shape = 형태 선택
plugin-workbench-config-choose-type = 유형 선택
plugin-workbench-config-default = 기본값
plugin-workbench-config-diff = 설정 차이
plugin-workbench-config-dirty = 저장 안 됨
plugin-workbench-config-drilldown-footer = 왼쪽/오른쪽 셀 · 위/아래 행 · Enter 편집 · Ctrl+D 제거/초기화 · Esc 뒤로
plugin-workbench-config-saved = 저장됨
plugin-workbench-config-setting = 설정 항목
plugin-workbench-config-state = 상태
plugin-workbench-config-state-changed = 변경됨
plugin-workbench-config-state-default = 기본
plugin-workbench-config-state-dirty = 저장 안 됨
plugin-workbench-config-state-error = 오류
plugin-workbench-config-state-inactive = 비활성
plugin-workbench-config-summary = {$status} · {$save_state}
plugin-workbench-config-title = {$plugin} / 설정
plugin-workbench-config-type = 유형
plugin-workbench-config-value = 값
plugin-workbench-config-view-summary = 유효 설정 · {$changed}개 필드 변경 · 선택한 셀: {$cell}
plugin-workbench-detail-footer = Tab/Shift+Tab 섹션 · 위/아래 스크롤 · Esc 뒤로
plugin-workbench-detail-tools-footer = Tab/Shift+Tab 섹션 · 위/아래 선택 · Enter 설정 및 실행 · Esc 뒤로
plugin-workbench-filter-all = 전체
plugin-workbench-filter-other = 기타
plugin-workbench-header-summary = 도구: {$tools}        명령: {$commands}        설정: {$config}
plugin-workbench-input-preview = 입력 미리보기: {$tool}
plugin-workbench-last-result-failed = 최근 결과 · {$tool} · 실패
plugin-workbench-last-result-success = 최근 결과 · {$tool} · 성공
plugin-workbench-list-footer = 입력하여 검색 · 위/아래 선택 · Enter 열기 · Esc 닫기
plugin-workbench-list-summary = 플러그인 검색… {$query}        전송: {$transport}        설정: {$config}        {$shown}/{$total} 표시
plugin-workbench-loading-actions = 작업 불러오는 중…
plugin-workbench-loading-choices = 선택 항목 불러오는 중…
plugin-workbench-no-changes = 변경 없음
plugin-workbench-no-commands = 명령이 없습니다.
plugin-workbench-no-config-section = 설정 섹션이 없습니다.
plugin-workbench-no-editable-rows = 편집 가능한 행이 없습니다.
plugin-workbench-no-filter-matches = 현재 필터와 일치하는 플러그인이 없습니다.
plugin-workbench-no-issues = 문제 없음
plugin-workbench-no-logs = 로그가 없습니다.
plugin-workbench-no-selection = 선택한 플러그인이 없습니다.
plugin-workbench-no-structured-arguments = 구조화된 인수가 없습니다.
plugin-workbench-no-tools = 도구가 없습니다.
plugin-workbench-none = 없음
plugin-workbench-none-declared = 선언 없음
plugin-workbench-overview = 개요
plugin-workbench-package-summary = 패키지: {$package}
plugin-workbench-plugin = 플러그인
plugin-workbench-plugin-capabilities = 플러그인 기능
plugin-workbench-plugins = 플러그인
plugin-workbench-provenance = 출처: {$provenance}
plugin-workbench-sections = 섹션
plugin-workbench-severity-error = 오류
plugin-workbench-severity-warning = 경고
plugin-workbench-status-invalid = 유효하지 않음
plugin-workbench-status-issues = 문제
plugin-workbench-status-missing = 없음
plugin-workbench-status-needs-restart = 다시 시작 필요
plugin-workbench-status-runtime-issue = 런타임 문제
plugin-workbench-status-schema-missing = 스키마 없음
plugin-workbench-status-valid = 유효함
plugin-workbench-status-warning = 경고
plugin-workbench-summary = 검색: {$query} · 전송 {$transport} · 설정 {$config} · {$shown}/{$total} 표시
plugin-workbench-tab-capabilities = 기능
plugin-workbench-tab-commands = 명령
plugin-workbench-tab-config = 설정
plugin-workbench-tab-diagnostics = 진단
plugin-workbench-tab-logs = 로그
plugin-workbench-tab-tools = 도구
plugin-workbench-tabs = 탭
plugin-workbench-tags-summary = 태그: {$tags}
plugin-workbench-tool-capabilities = 도구 기능
plugin-workbench-tools-help = 위/아래로 도구를 선택합니다. Enter로 호스트 관리 스키마 양식을 열고 Ctrl+S로 검증한 뒤 실행합니다.
plugin-workbench-transport = 전송
plugin-workbench-trust-level = 신뢰 수준: {$level}
plugin-workbench-unavailable = 사용할 수 없음


# Plugin Workbench structured editor i18n completion
plugin-workbench-editor-also-matches = 다음과도 일치: {$matches}
plugin-workbench-editor-array-action-help = Enter 작업 메뉴 · Ctrl+D로 선택한 행 제거
plugin-workbench-editor-array-preview = 구성… (항목 {$count}개)
plugin-workbench-editor-configure = 구성…
plugin-workbench-editor-format = 형식: {$format}
plugin-workbench-editor-generic-object = 일반 객체 편집기
plugin-workbench-editor-index = 인덱스
plugin-workbench-editor-item = 항목 {$index}
plugin-workbench-editor-map = 맵 편집기
plugin-workbench-editor-no-fields = 필드가 없습니다.
plugin-workbench-editor-no-items = 항목이 없습니다.
plugin-workbench-editor-object = 객체 편집기
plugin-workbench-editor-object-action-help = Enter 작업 메뉴 · 작업 셀에서 필드 추가
plugin-workbench-editor-object-array = 객체 배열 테이블 편집기
plugin-workbench-editor-object-array-help = 편집하면 선택한 항목이 같은 구조화 편집기에서 열립니다.
plugin-workbench-editor-object-preview = 구성… (필드 {$count}개)
plugin-workbench-editor-preview = 미리보기
plugin-workbench-editor-primitive-array = 기본 배열 편집기
plugin-workbench-editor-readonly = 읽기 전용
plugin-workbench-editor-schema-missing = 스키마 없음        기본 구조화 편집기
plugin-workbench-editor-shape = 형태
plugin-workbench-editor-suggestions = 제안
plugin-workbench-editor-tuple = 튜플 편집기
plugin-workbench-editor-type-summary = 유형: {$type}        경로 편집기: 구조화 GUI
plugin-workbench-field-state-available = 사용 가능
plugin-workbench-field-state-custom = 사용자 지정
plugin-workbench-field-state-map-key = 맵 키
plugin-workbench-field-state-missing = 누락
plugin-workbench-field-state-optional = 선택 사항
plugin-workbench-field-state-required = 필수
plugin-workbench-kind-all-of = allOf
plugin-workbench-kind-any-of = anyOf
plugin-workbench-kind-array = 배열
plugin-workbench-kind-boolean = 부울
plugin-workbench-kind-integer = 정수
plugin-workbench-kind-null = null
plugin-workbench-kind-number = 숫자
plugin-workbench-kind-object = 객체
plugin-workbench-kind-one-of = oneOf
plugin-workbench-kind-string = 문자열
plugin-workbench-kind-value = 값
