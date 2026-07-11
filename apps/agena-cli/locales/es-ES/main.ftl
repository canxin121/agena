cli-about = Aplicacion de chat en terminal de Agena

pane-sessions = Sesiones
pane-sessions-search = Sesiones [{$query}]
pane-transcript = Transcripcion
pane-messages = Mensajes
pane-composer = Entrada [{$session}]

session-meta = #{$id}  {$message_count} msg  {$updated}
session-running = en ejecucion
sessions-empty = No se encontraron sesiones
sessions-loading-more = Cargando mas sesiones...
sessions-more = Hay mas sesiones disponibles

transcript-header-lines = lineas {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = buscar={$query} ({$current}/{$total})
transcript-header-tail = seguir final
transcript-header-loading = cargando
transcript-header-loading-older = cargando mensajes anteriores
transcript-header-busy = ocupado
transcript-loading-older = Cargando mensajes anteriores...
transcript-more-older = Hay mensajes anteriores disponibles. Desplacese hacia arriba o pulse PageUp.
transcript-empty-session = Todavia no hay mensajes en esta sesion.

no-session-selected = No hay ninguna sesion seleccionada.
no-session-selected-hint = Use /sessions para elegir una sesion, o empiece a escribir en el editor para crear una.
composer-session-new = nueva sesion
composer-placeholder = Mensaje para Agena. Alt+Up historial. / comandos. F3 archivo.

status-global = / busca abajo | ? busca arriba | Ctrl+C dos veces sale
status-sessions = Sesiones: /sessions [busqueda]
status-transcript = VIEW: i inserta | j/k desplaza | / busca | c copia ultima | y copia
status-composer = INSERT: Esc vuelve | Ctrl+Enter envia ahora | Ctrl+J nueva linea | Alt+Up/Down historial | / comandos

help-title = Ayuda
help-header = Agena TUI
help-section-sessions = Selector de sesiones
help-sessions-line-1 = /sessions abre el selector de sesiones con busqueda
help-sessions-line-2 = Up/Down, PageUp/PageDown mueven la seleccion
help-sessions-line-3 = Enter abre la sesion seleccionada
help-section-transcript = Panel de transcripcion
help-transcript-line-1 = i entra en INSERT; j/k o flechas desplazan
help-transcript-line-2 = Space / Shift+Space / Ctrl+B paginan
help-transcript-line-3 = Ctrl+D / Ctrl+U media pagina
help-transcript-line-4 = PageUp cerca del borde superior carga mensajes anteriores
help-transcript-line-5 = g/G salta al inicio o al final
help-transcript-line-6 = / busca hacia abajo y ? hacia arriba; n repite la direccion y N la invierte
help-transcript-line-7 = c copia el ultimo mensaje del asistente, y copia la transcripcion cargada, Y copia la vista visible
help-section-composer = Editor
help-composer-line-1 = Esc vuelve a VIEW; Enter envia
help-composer-line-2 = Alt/Shift+Enter o Ctrl+J inserta salto de linea
help-composer-line-3 = Ctrl+A/E/B/F/P/N mueven, Alt+B/F o Alt/Ctrl+Left/Right saltan por palabra
help-composer-line-4 = Ctrl+H/D/W/U/K/Y editan como shell o editor
help-composer-line-5 = En un limite de linea, Ctrl+A/E puede continuar a la linea anterior/siguiente
help-composer-line-6 = F3, Ctrl+O o Alt+O buscan archivos del workspace para adjuntar
help-composer-line-7 = F4 o Alt+E abren $VISUAL/$EDITOR para el editor
help-composer-line-8 = F6 o Alt+I adjuntan una imagen del portapapeles
help-composer-line-9 = El texto pegado se inserta directamente; una sola ruta de archivo se adjunta y los adjuntos permanecen atomicos
help-composer-line-10 = Alt+Up/Down recupera prompts enviados
help-section-actions = Acciones
help-actions-line-1 = n crea una sesion
help-actions-line-2 = r continua una sesion bloqueada o pendiente
help-actions-line-3 = a/A/d/D responden a la primera solicitud de permiso pendiente
help-actions-line-4 = Alt+U abre la primera solicitud de entrada pendiente desde el editor
help-actions-line-5 = La captura del raton esta desactivada para conservar la seleccion y copia nativas del terminal
help-actions-line-6 = Ctrl+C dos veces sale

overlay-session-search-title = Busqueda de sesiones
overlay-session-search-prompt = Buscar en los titulos de sesion
overlay-transcript-search-title = Busqueda en la transcripcion
overlay-transcript-search-prompt = Buscar dentro de los mensajes cargados
overlay-line-footer = Escriba para editar

overlay-attach-title = Adjuntar archivo
overlay-attach-prompt = Escriba una ruta o termino de busqueda. Enter adjunta el archivo seleccionado.
overlay-attach-no-match = No hay archivos coincidentes
overlay-attach-matches = Coincidencias
overlay-attach-footer = Tab rellena la ruta seleccionada

overlay-user-input-title = Entrada de usuario pendiente
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = valor personalizado permitido
overlay-user-input-reply-format = Formato de respuesta: question_id=value;other_id=value1,value2
overlay-user-input-cancel-hint = Ctrl+D cancela la solicitud
overlay-user-input-footer = Ctrl+D cancelar

flash-terminal-event-error = error de evento del terminal: {$error}
flash-created-session = sesion creada {$title}
flash-permission-reply-sent = respuesta de permiso enviada: {$label}
flash-user-input-reply-sent = respuesta de entrada de usuario enviada
flash-large-paste-staged = pegado grande preparado en el editor
flash-attached = {$path} adjuntado
flash-composer-updated = editor actualizado desde el editor externo
flash-prompt-history-empty = el historial de prompts esta vacio
flash-prompt-history-items = quita adjuntos o pegados preparados antes de recuperar el historial de prompts
flash-external-editor-failed = fallo del editor externo: {$error}
flash-clipboard-image-attached = imagen del portapapeles adjuntada: {$width}x{$height} {$format}
flash-clipboard-image-attach-failed = fallo al adjuntar la imagen del portapapeles: {$error}
flash-no-loaded-transcript = no hay transcripcion cargada para copiar
flash-copied-loaded-transcript = transcripcion cargada copiada al portapapeles
flash-no-assistant-message = no hay mensaje del asistente para copiar
flash-no-assistant-message-text = el ultimo mensaje del asistente no tiene texto cargado para copiar
flash-copied-assistant-message = ultimo mensaje del asistente copiado al portapapeles
flash-no-visible-transcript = no hay texto visible para copiar
flash-copied-visible-transcript = vista visible copiada al portapapeles
flash-clipboard-copy-failed = fallo al copiar al portapapeles: {$error}

message-role-user = usuario
message-role-assistant = asistente
message-role-system = sistema

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed

message-parts-not-loaded = {$count} partes sin cargar
message-usage = uso: in={$input} out={$output} reasoning={$reasoning}
message-finish = finish: {$finish}
message-empty = (mensaje vacio)
message-thinking = pensando: {$summary}
message-command-status = estado: {$status}, exit={$exit}
message-file-changes = cambios de archivos
message-file-changes-preview-one = 1 archivo: {$paths}
message-file-changes-preview-many = {$count} archivos: {$paths}
message-file-changes-more = +{$count} mas
message-search = busqueda: {$query}
message-todo-list = lista de tareas
message-error = error [{$code}]: {$message}
message-attachments = adjuntos
message-awaiting-user-input = esperando entrada de usuario: {$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = detalle de parte no disponible
message-tool-pending = pendiente: {$label}
message-tool-running = ejecutando: {$label}
message-tool-done = listo: {$label}
message-tool-failed = fallo: {$label}
message-tool-cancelled = cancelado: {$label}
message-tool-result-blocks = {$count} bloques de resultado

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

time-just-now = justo ahora
time-minutes-ago = hace {$count} min
time-hours-ago = hace {$count} h
time-days-ago = hace {$count} d

session-default-title = Nueva sesion {$time}
session-default-base = Nueva sesion
session-fallback-title = Sesion {$id}

user-input-error-empty = la respuesta no puede estar vacia
user-input-error-invalid-segment = segmento de respuesta invalido: {$segment}
user-input-error-unknown-question = id de pregunta desconocido: {$question_id}
user-input-error-missing-answer = la pregunta {$question_id} debe tener al menos una respuesta
user-input-error-no-answers = la respuesta no contenia respuestas

attachment-kind-image = image
attachment-kind-audio = audio
attachment-kind-video = video
attachment-kind-pdf = pdf
attachment-kind-file = archivo
attachment-generic = adjunto
attachment-chip-image = {$kind}: {$filename} ({$width}x{$height}, {$size})
attachment-chip-other = {$kind}: {$filename} ({$size})
attachment-placeholder = [{$kind} {$filename}]

bytes-gb = {$value} GB
bytes-mb = {$value} MB
bytes-kb = {$value} KB
bytes-b = {$value} B

paste-label = pegado de {$count} caracteres
paste-label-append = pegado de {$count} caracteres, anexar al enviar
paste-placeholder = [pegado de {$count} caracteres]

permission-label-allow-once = permitir una vez
permission-label-allow-always = permitir siempre
permission-label-deny-once = denegar una vez
permission-label-deny-always = denegar siempre

permission-summary-pending = Esperando permiso: {$reason}
permission-summary-allow-once = Permiso concedido una vez: {$reason}
permission-summary-allow-always = Permiso concedido siempre: {$reason}
permission-summary-deny-once = Permiso denegado una vez: {$reason}
permission-summary-deny-always = Permiso denegado siempre: {$reason}
