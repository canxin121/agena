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
hub-title = Centro de sesiones
hub-action-create = nueva sesión
hub-action-list = lista de sesiones
hub-action-refresh = actualizar
hub-hint-move = mover
hub-hint-focus = foco
hub-hint-section = sección
hub-hint-open = abrir
hub-hint-back = atrás
hub-section-attention = Requieren atención
hub-section-running = En ejecución
hub-section-recent = Recientes
hub-empty-attention = Ninguna sesión requiere atención
hub-empty-running = No hay sesiones en ejecución
hub-empty-recent = No hay sesiones recientes
hub-section-new = Nueva sesión
hub-empty-new = No hay sesión que crear
hub-item-new = + Nueva sesión
hub-item-new-detail = Intro para crear una sesión
hub-action-search = buscar
hub-action-clear-search = borrar búsqueda
hub-search-placeholder = Escribe para filtrar sesiones…
hub-search-active-empty = Escribe para filtrar…
hub-search-active = Filtro:{$query}
command-hub-summary = Abrir el centro de sesiones
command-background-summary = Volver al centro;la sesión sigue
hub-empty = Aún no hay sesiones. Cree una con Ctrl+N.
context-help-context-hub = Centro de sesiones
context-help-summary-hub = Consulte las sesiones que requieren atención, en ejecución y recientes, y cree una nueva sesión.
context-help-key-create-session = Crear una nueva sesión.
context-help-key-session-list = Abrir la lista completa de sesiones.

transcript-header-lines = lineas {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = buscar={$query} ({$current}/{$total})
transcript-header-tail = seguir final
transcript-header-loading = cargando
transcript-header-loading-older = cargando mensajes anteriores
transcript-header-busy = ocupado
transcript-loading-older = Cargando mensajes anteriores...
transcript-more-older = Hay mensajes anteriores disponibles. Desplacese hacia arriba o pulse PageUp.
transcript-empty-session = Todavia no hay mensajes en esta sesion.
transcript-footer-pending = se enviará cuando termine la ejecución activa
transcript-footer-pending-preview = siguiente envío: {$preview}
transcript-footer-plugin-block = {$label}: {$body}
transcript-export-path-is-directory = la ruta de exportación es un directorio: {$path}
transcript-export-title = Exportación de transcripción de Agena
transcript-export-session-id = - ID de sesión: {$id}
transcript-export-exported-at = - Fecha de exportación: {$time}
transcript-export-messages-loaded = - Mensajes cargados: {$count}
transcript-export-older-messages-omitted = - Mensajes anteriores omitidos: {$value}
transcript-export-parent-session = - Sesión principal: #{$id}
transcript-export-child-sessions = - Sesiones secundarias: {$count}
transcript-export-empty = _No hay mensajes cargados en esta sesión._
transcript-node-kind-message = mensaje
transcript-node-kind-markdown = bloque Markdown
transcript-node-kind-heading = encabezado
transcript-node-kind-quote = cita
transcript-node-kind-alert = aviso
transcript-node-kind-code = bloque de código
transcript-node-kind-list = lista
transcript-node-kind-table = tabla
transcript-node-kind-math = fórmula
transcript-node-kind-image = imagen
transcript-node-kind-footnote = nota al pie
transcript-node-kind-diagram = diagrama
transcript-node-kind-activity = bloques de actividad
transcript-footer-side-active = conversación paralela activa · sal con /parent o mediante la navegación
transcript-footer-side-return = conversación paralela abierta · vuelve cambiando de sesión
transcript-history-more = ↑ Cargar mensajes anteriores · desplázate hacia arriba o pulsa aquí
transcript-history-loading = … Cargando mensajes anteriores
transcript-history-start = Inicio de la conversación
transcript-history-error = ↑ Reintentar la carga del historial · { $error }
transcript-history-stalled = El cursor del historial no ha avanzado. Reintenta la carga.
transcript-fold-loading = … Cargando partes anteriores de la respuesta
transcript-fold-error = Error al cargar · Intro para reintentar · { $error }

session-state-creating = creando
session-state-ready = finalizada recientemente
session-state-running = en ejecucion
session-state-awaiting-interaction = esperando tu respuesta
session-state-interrupted = interrumpida
session-state-failed = fallida

no-session-selected = No hay ninguna sesion seleccionada.
no-session-selected-hint = Use /sessions para elegir una sesion, o empiece a escribir en el editor para crear una.
composer-session-new = nueva sesion
composer-placeholder = Mensaje para Agena. Arriba al inicio abre el historial. / comandos. Ctrl+O archivo.

status-global = / busca abajo | ? busca arriba | Ctrl+C dos veces sale
status-sessions = Sesiones: /sessions
status-transcript = VIEW: i inserta | j/k desplaza | / busca | c copia ultima | y copia
status-composer = INSERT: Esc vuelve | Ctrl+Enter envia ahora | Ctrl+J nueva linea | Arriba al inicio historial | / comandos | Ctrl+G items | Ctrl+R entrada | Ctrl+L aprobacion

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
help-composer-line-2 = Shift+Enter o Ctrl+J inserta salto de linea
help-composer-line-3 = Ctrl+A/E/B/F/P/N mueven, Ctrl+Left/Right saltan por palabra
help-composer-line-4 = Ctrl+H/D/W/U/K/Y editan como shell o editor
help-composer-line-5 = En un limite de linea, Ctrl+A/E puede continuar a la linea anterior/siguiente
help-composer-line-6 = Ctrl+O busca archivos del workspace para adjuntar
help-composer-line-7 = Ctrl+E abre $VISUAL/$EDITOR para el editor
help-composer-line-8 = Ctrl+T adjunta una imagen del portapapeles
help-composer-line-9 = El texto pegado se inserta directamente; una sola ruta de archivo se adjunta y los adjuntos permanecen atomicos
help-composer-line-10 = Arriba abre el historial cuando el cursor esta al inicio; Ctrl+P edita el mensaje pendiente y Ctrl+X lo cancela
help-section-actions = Acciones
help-actions-line-1 = Ctrl+N crea una sesion; n/N navega resultados de busqueda
help-actions-line-2 = r continua una sesion bloqueada o pendiente; U abre las estadísticas de uso
help-actions-line-3 = a/A/d/D responden a la primera solicitud de permiso pendiente
help-actions-line-4 = Ctrl+R abre la primera solicitud de entrada pendiente desde el editor
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
overlay-user-input-questions = Preguntas
overlay-user-input-summary = Resumen
overlay-user-input-detail = Detalles
overlay-user-input-prompt-panel = Instrucción
overlay-user-input-choices = Opciones
overlay-user-input-auto-resolve = Continuación automática en {$remaining}
overlay-user-input-choice-single-short = Una opción
overlay-user-input-choice-multiple-short = Varias opciones
overlay-user-input-custom-input = Entrada personalizada
overlay-user-input-custom-input-hint = Entrada personalizada (e / pegar)
overlay-user-input-submit = Enviar
overlay-user-input-review-intro = Revisa tus respuestas antes de enviarlas.
overlay-user-input-no-questions = No hay preguntas.
overlay-user-input-other = Otro
overlay-user-input-unanswered = sin responder
overlay-user-input-custom-empty = Pulsa Espacio para escribir una respuesta personalizada
overlay-user-input-review-feedback = Comentarios para el agente
overlay-user-input-review-feedback-empty = Pulsa Enter o e para escribir comentarios, o elige una decisión
overlay-user-input-review-footer-hint = Enter sobre una opción = enviar · Enter sobre el título = contraer/expandir
overlay-user-input-missing-answer = Falta responder {$label}
overlay-user-input-footer-review = j/k ↑/↓ mover | Enter elegir | e editar | Ctrl+Y/Ctrl+E desplazar línea | PgUp/PgDn páginas | Ctrl+X cancelar
overlay-user-input-footer-question = Tab/Shift+Tab páginas | Espacio alternar | Ctrl+D borrar | Ctrl+Y/Ctrl+E desplazar línea | PgUp/PgDn páginas | Ctrl+X cancelar
overlay-user-input-wizard-keys = ←→ páginas · Espacio seleccionar · Enter siguiente / confirmar resumen
overlay-user-input-page = Pregunta { $current } / { $total }
overlay-user-input-previous = Anterior
overlay-user-input-next = Siguiente
overlay-user-input-cancel = Cancelar
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = valor personalizado permitido
overlay-user-input-reply-format = Formato de respuesta: 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X cancela la solicitud
overlay-user-input-footer = Ctrl+X cancelar

flash-terminal-event-error = error de evento del terminal: {$error}
flash-created-session = sesion creada {$title}
flash-permission-reply-sent = respuesta de permiso enviada: {$label}
flash-user-input-reply-sent = respuesta de entrada de usuario enviada
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
flash-message-interrupting = interrumpiendo la ejecucion activa - el mensaje se enviara a continuacion

message-role-user = usuario
message-role-assistant = asistente
message-role-system = sistema

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed
message-state-policy-denied = blocked by permission policy
message-state-user-declined = declined by user
message-state-capability-unavailable = capability unavailable
message-state-tool-unavailable = tool unavailable

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
message-user-input-replied = entrada de usuario respondida: {$request_id}
message-question-line = - {$question}
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
attachment-kind-directory = carpeta
attachment-generic = adjunto
attachment-chip-image = {$kind}: {$filename} ({$width}x{$height}, {$size})
attachment-chip-other = {$kind}: {$filename} ({$size})
attachment-placeholder = [{$kind} {$filename}]

bytes-gb = {$value} GB
bytes-mb = {$value} MB
bytes-kb = {$value} KB
bytes-b = {$value} B


permission-label-allow-once = permitir una vez
permission-label-allow-always = permitir siempre
permission-label-deny-once = denegar una vez
permission-label-deny-always = denegar siempre

permission-summary-allow-once = Permiso concedido una vez: {$reason}
permission-summary-allow-always = Permiso concedido siempre: {$reason}
permission-summary-deny-once = Permiso denegado una vez: {$reason}
permission-summary-deny-always = Permiso denegado siempre: {$reason}

failure-detail-message = Mensaje
failure-detail-code = Código de error
failure-detail-category = Categoría
failure-detail-responsibility = Responsabilidad
failure-detail-impact = Impacto
failure-detail-recovery = Recuperación
failure-detail-retry = Reintento
failure-category-invalid-input = Entrada no válida
failure-category-not-found = No encontrado
failure-category-conflict = Conflicto
failure-category-permission-required = Permiso requerido
failure-category-permission-denied = Permiso denegado
failure-category-authentication-required = Autenticación requerida
failure-category-rate-limited = Límite de tasa
failure-category-quota-exceeded = Cuota superada
failure-category-timeout = Tiempo agotado
failure-category-dependency-unavailable = Dependencia no disponible
failure-category-protocol-failure = Error de protocolo
failure-category-data-corruption = Problema de integridad de datos
failure-category-internal = Error interno
failure-responsibility-caller = La solicitud
failure-responsibility-policy = Política
failure-responsibility-dependency = La dependencia
failure-responsibility-system = El sistema
failure-impact-request-rejected = Solicitud rechazada
failure-impact-operation-failed = Operación fallida
failure-impact-operation-paused = Operación en pausa
failure-impact-partial-success = Éxito parcial
failure-impact-background-task-failed = Tarea en segundo plano fallida
failure-impact-runtime-degraded = Runtime degradado
failure-impact-fatal-startup-failure = Error fatal de inicio
failure-recovery-none = Sin recuperación automática
failure-recovery-refresh = Actualizar
failure-recovery-reauthenticate = Reautenticar
failure-recovery-open-settings = Abrir ajustes
failure-recovery-request-permission = Solicitar permiso
failure-recovery-ask-user = Preguntar al usuario
failure-recovery-retry = Reintentar
failure-recovery-choose-alternative = Elegir una alternativa
failure-recovery-restart-plugin = Reiniciar el plugin
failure-recovery-restart-runtime = Reiniciar el runtime
failure-retry-never = No reintentar
failure-retry-correct-input = Corregir la entrada y reintentar
failure-retry-after-user-action = Reintentar tras acción del usuario
failure-retry-after-refresh = Reintentar tras actualizar
failure-retry-immediate-once = Reintentar una vez de inmediato
failure-retry-backoff = Reintentar con backoff
failure-retry-use-alternative = Usar una alternativa
failure-retry-unknown = Desconocido

## Settings Studio core locale coverage
## Long policy descriptions intentionally continue to use the verified English fallback.

permission-studio-new-rule-label = + nueva regla

permission-studio-new-rule-value = Crear

permission-studio-catalog-tags-title = Add Tool Tag Rules

permission-studio-catalog-names-title = Agregar las reglas de acceso a la herramienta

permission-studio-catalog-footer = Abajo a los resultados · Modo de selección de espacio · Cancelación de esc

permission-studio-catalog-tag-detail = Used by {$count} registered tool(s)

permission-studio-catalog-custom-label = + Regla personalizada...

permission-studio-catalog-custom-search = nuevo manual etiqueta nombre de la herramienta

overlay-settings-title = Configuración

overlay-settings-footer = Ctrl+R refres · ←/→ interruptores de panes · Tab/Shift+ Tab cycle panes · ↑/↓ select · Entrar abierto · Esc close

overlay-settings-sections = Secciones

overlay-settings-options = Opciones

overlay-settings-group-core = Principal

overlay-settings-group-application = Aplicación

overlay-settings-group-session = Sesión

overlay-settings-group-system = Sistema

overlay-settings-default-section-title = Sección

overlay-settings-empty-section = No se ha seleccionado ninguna sección.

overlay-settings-empty-items = No hay ajustes en esta sección.

overlay-settings-empty-detail = Seleccione una sección y una opción para inspeccionarla o editarla.

overlay-settings-detail-current = Current value: {$value}

overlay-settings-detail-path = Path: {$path}

overlay-settings-detail-action = Abre o edita esta configuración.

settings-detail-action-screen = Abre esta pantalla.

overlay-settings-edit-title = Edit {$field}

overlay-settings-edit-file-value = File override: {$value}

overlay-settings-edit-effective-value = Effective value: {$value}

overlay-choice-clear-settings-detail = Eliminar la anulación del archivo para {$field}.

overlay-settings-section-plugins-label = Plugins y herramientas

overlay-settings-section-plugins-summary = Plugins, herramientas y diagnósticos

overlay-settings-section-providers-label = Modelos y proveedores

overlay-settings-section-providers-summary = {$count} configured providers

overlay-settings-section-model-catalog-label = Catálogo de modelos

overlay-settings-section-model-catalog-summary = {$count} entries

overlay-settings-section-permissions-label = Permisos

overlay-settings-section-permissions-summary = {$count} persisted permission rule(s)

overlay-settings-section-tracing-summary = Filtros de registro y diagnósticos

overlay-settings-section-ui-label = Apariencia

overlay-settings-section-ui-summary = Locale y preferencias de interfaz

overlay-settings-section-ui-description = Lenguaje persistente, color, gráficos y configuración temática.

overlay-settings-section-runtime-session-label = Runtime y sesión

overlay-settings-section-runtime-session-summary = Identidades del cliente proveedor y compactación de contexto

settings-permission-global-label = Global Permission

settings-permission-global-detail = Base de referencia para todos los períodos de sesiones.

settings-permission-workspace-label = Permiso del espacio de trabajo

settings-permission-workspace-detail = Superar la capa para el proyecto actual.

settings-permission-current-label = Permiso de sesión actual

settings-permission-current-detail = Se aplica únicamente al actual período de sesiones.

settings-permission-effective-label = Permiso efectivo

settings-permission-layer-global = Global

settings-permission-layer-workspace = Espacio de trabajo

settings-permission-layer-session = Período de sesiones

settings-permission-layer-effective = Eficacia

settings-runtime-thinking-label = Modo de pensamiento

settings-runtime-thinking-description = el modo de pensamiento actual de la sesión anula

settings-runtime-speed-label = Modo de velocidad

settings-runtime-speed-description = modo de velocidad de la sesión actual

settings-runtime-verbosity-label = Verbosidad

settings-runtime-verbosity-description = la verbosidad actual anula

settings-field-permission-approval-model-label = Modelo de aprobación automática global

settings-field-ui-locale-label = Idioma

settings-field-ui-locale-description = Lenguaje de interfaz

settings-field-tui-color-scheme-label = Esquema de colores del terminal

settings-field-tui-theme-label = Tema de plugin TUI

settings-field-tui-theme-description = Paleta de color semántica proporcionada por plugin opcional

settings-choice-tui-color-scheme-auto = Detectar el fondo terminal automáticamente

settings-choice-tui-color-scheme-dark = Optimize colores para un fondo terminal oscuro

settings-choice-tui-color-scheme-light = Optimize los colores para un fondo terminal de luz

settings-field-tui-graphics-label = Gráficos enriquecidos del terminal

settings-choice-tui-graphics-auto = Negociar gráficas nativas automáticamente y regresar con seguridad a Unicode (recomendado)

settings-choice-tui-graphics-native = negociación gráfica nativa de fuerza para un camino terminal configurado por expertos

settings-choice-tui-graphics-unicode = Desactivar los gráficos nativos y utilizar la renderización determinística Unicode/text

settings-field-activity-default-expanded-label = Expandir actividades de forma predeterminada

settings-field-activity-kind-description = Estado de expansión predeterminado para este tipo de actividad.

settings-field-activity-tool-label = Ampliación predeterminada de la herramienta

settings-field-activity-tool-description = Estado de expansión predeterminado para esta herramienta exacta.

settings-activity-kind-reasoning-label = Razonamiento

settings-activity-kind-operation-label = Operaciones de herramientas

settings-activity-kind-operation-description = Las llamadas de herramientas y sus resultados.

settings-activity-kind-resource-label = Recursos

settings-activity-kind-resource-description = Adjuntos y otros contenidos de recursos.

settings-activity-kind-skill_reference-label = Referencias de comando

settings-activity-kind-skill_reference-description = Referencias a los comandos seleccionados en la respuesta.

settings-activity-kind-interaction-label = Interacciones

settings-activity-kind-interaction-description = Solicitudes de entrada de usuario e indicaciones interactivas.

settings-activity-kind-hook-label = Ganchos

settings-activity-kind-hook-description = Carreras de gancho de sesión y eventos de ciclo de vida.

settings-activity-kind-error-label = Errores

settings-activity-kind-error-description = Operaciones fallidas y fallas terminales.

settings-activity-kind-notice-label = Avisos

settings-activity-kind-notice-description = Avisos de antecedentes y filas de información.

settings-activity-kind-text-label = Texto

settings-activity-kind-text-description = Texto y contenido del artefacto de texto.

settings-field-tracing-filter-label = Nivel de registro de la aplicación

settings-field-tracing-filter-description = Nivel de registro por defecto

settings-field-tracing-database-label = Nivel de registro de la base de datos

settings-field-tracing-database-description = Nivel de registro de la base de datos

settings-field-tracing-adapter-label = Nivel de registro del adaptador

settings-field-tracing-adapter-description = Nivel de registro del adaptador del proveedor

settings-config-open-file-detail = Open agena.json para este camino

settings-source-unset = No está listo.

settings-source-configured = Configured: {$value}

settings-source-effective = Effective: {$value}

settings-source-file-effective = File: {$file} / Effective: {$effective}

settings-source-file-found = {$path} (found)

settings-source-file-missing = {$path} (se creará)

settings-source-row-config-file = Archivo de confianza

settings-source-row-workspace-config-file = Archivo de config espacio de trabajo

settings-source-row-file-value = Valor de archivo

settings-source-row-workspace-value = Valor del espacio de trabajo

settings-source-row-effective-value = Valor efectivo

settings-source-row-write-target = Escribe a

settings-source-row-layers = capas activas

settings-source-current-session = datos del período de sesiones en curso

settings-source-current-session-runtime = opciones actuales del período de sesiones

settings-detail-values-heading = Valores

settings-detail-sources-heading = Fuentes

settings-detail-action-readonly = Abra la vista sólo eficaz.

settings-detail-action-file = Abre el archivo de configuración de respaldo.




settings-field-parse-bool = {$field} expects a boolean like true/false or on/off

settings-field-parse-integer = {$field} expects an unsigned integer value

settings-field-parse-float = {$field} expects a numeric value

settings-choice-adapter-fallback = adaptador

settings-plugin-workbench-label = Banco de trabajo de configuración de plugins

settings-mcp-server-label = Servidor MCP de Agena

settings-mcp-server-value = toggle enabled/disabled

settings-mcp-server-enabled = habilitado

settings-mcp-server-disabled = discapacitados

settings-mcp-status-unavailable = Estado no disponible

settings-mcp-ready = listo

settings-mcp-needs-attention = necesidades de atención

settings-mcp-auth-label = Autenticación MCP

settings-mcp-auth-none = anónimo: cada herramienta expuesta

settings-mcp-auth-oauth = Total OAuth

settings-mcp-auth-mixed = mixto: descubrimiento público, per-tool OAuth

settings-mcp-anonymous-access-label = Acceso anónimo a herramientas con autenticación mixta

settings-mcp-anonymous-access-none = ninguno (recomendado)

settings-mcp-anonymous-access-read-only = Herramientas de uso exclusivo de permisos

settings-mcp-registration-label = registro

settings-mcp-pkce-label = PKCE

settings-mcp-public-url-label = URL pública de MCP

settings-mcp-public-url-value = edición

settings-mcp-public-url-auto = retroceso de escucha-local

settings-mcp-oauth-issuer-label = URL del emisor OAuth

settings-mcp-oauth-issuer-derived = derivada del origen de los recursos del MCP

settings-mcp-oauth-password-label = Contraseña OAuth de MCP

settings-mcp-oauth-password-value = fijado o reemplazado

settings-mcp-oauth-password-configured = Configurado por contraseña específica MCP

settings-mcp-oauth-password-ui-fallback = usando la contraseña de UI

settings-mcp-oauth-password-not-configured = no configurado

settings-mcp-oauth-password-clear-label = Clear MCP OAuth Password

settings-field-runtime-codex-version-label = Codex Client Version

settings-field-runtime-claude-version-label = Claude Code Version

settings-field-runtime-gemini-version-label = Gemini CLI Version

settings-field-session-compaction-auto-label = Compactación automática

settings-field-session-compaction-reserved-tokens-label = Tokens reservados para compactación

settings-client-versions-refresh-label = Actualizaciones del cliente

settings-client-versions-refresh-value = más reciente

settings-client-versions-entry-label = Proveedor Versiones del cliente

settings-client-versions-entry-value = codex · claude · gemini

settings-client-versions-section-label = Versiones de cliente

settings-client-versions-section-summary = Versiones de identidad de tiempos de ejecución

settings-provider-workbench-label = Lista de proveedores

settings-provider-workbench-value = {$count} provider(s)

settings-global-default-model-label = Modelo predeterminado global

settings-global-default-model-description = Modelo que se usa cuando una sesión no especifica uno; el modelo explícito de la sesión siempre tiene prioridad.
settings-workspace-default-model-label = Modelo predeterminado del espacio de trabajo
settings-workspace-default-model-description = Modelo predeterminado guardado en la configuración del proyecto de este espacio de trabajo; aquí sustituye al valor global.

settings-model-default-mode-inherit-detail = Utilice el modo nativo predeterminado del modelo seleccionado.

settings-provider-new-label = + Nuevo proveedor

settings-provider-existing-detail = {$count} adapters configured

settings-model-catalog-open-label = Catálogo de modelo abierto

settings-files-open-config-label = Open agena.json

settings-files-open-config-present = presentes

settings-files-open-config-create = crear en abierto

permission-studio-field-path-workspace = Path Workspace Defaults

permission-studio-field-path-external = Path External Defaults

permission-studio-field-path-rules = Reglas de ruta

permission-studio-field-network-defaults = Network Defaults

permission-studio-field-network-rules = Normas de red

permission-studio-field-tool-names = Nombres de la herramienta

permission-studio-field-tool-rules = Reglas de instrumentos

permission-studio-field-prompt-json = Introduce JSON para {$field}. Deja el editor vacío para borrar esta anulación.

permission-studio-detail-override = Anulación

permission-studio-detail-effective = Eficacia

permission-studio-detail-override-inline = Override {$value}

permission-studio-detail-effective-inline = Effective {$value}

permission-studio-detail-read-only = Este documento de permiso es sólo leído aquí.

permission-studio-detail-mode-editable = Entra abre el selector de modo para este campo.

permission-studio-detail-text-editable = Introduzca edita esta sola clave o patrón.

permission-studio-detail-remove-hint = Entrar elimina este artículo inmediatamente.

permission-studio-detail-navigate-hint = Entrar abre esta sección.

permission-studio-overview-target = Meta

permission-studio-overview-source = Fuente

permission-studio-overview-scope = Ámbito

permission-studio-overview-override = Anulación

permission-studio-overview-effective = Eficacia

permission-studio-section-workspace = Espacio de trabajo

permission-studio-section-external = Externo

permission-studio-section-rules = Reglas

permission-studio-section-defaults = Defectos

permission-studio-source-global = mundial

permission-studio-source-workspace = espacio de trabajo

permission-studio-source-session = período de sesiones

permission-studio-source-effective = efectiva

permission-studio-settings-override = override {$value}

permission-studio-settings-effective = effective {$value}

permission-studio-mode-read = read {$value}

permission-studio-mode-write = write {$value}

permission-studio-network-default = {$label} {$value}

permission-studio-page-overview = Sinopsis

permission-studio-page-path = Camino

permission-studio-page-path-defaults = Sistema de archivos / Zonas predeterminadas

permission-studio-page-path-rules = Sistema de archivos / Reglas de ruta

permission-studio-page-network = Red

permission-studio-page-network-zones = Red / Zonas de red

permission-studio-page-network-rules = Red / Reglas de dominio

permission-studio-page-tools = Herramientas

permission-studio-page-tool-tags = Acceso a la herramienta / Reglas de etiqueta

permission-studio-page-tool-names = Acceso a herramientas / Reglas de nombre

permission-studio-page-tool-command-rules = Acceso a herramientas / Reglas de comando

permission-studio-page-names = Nombres

permission-studio-page-tool-rules = Reglas de instrumentos

permission-studio-nav-overview = Sinopsis

permission-studio-nav-filesystem = Sistema de archivos

permission-studio-nav-default-zones = Zonas predeterminadas

permission-studio-nav-path-rules = Reglas de ruta

permission-studio-nav-network = Red

permission-studio-nav-network-zones = Zonas de red

permission-studio-nav-domain-rules = Reglas de dominio

permission-studio-nav-tool-access = Acceso a herramientas

permission-studio-nav-name-rules = Reglas de nombre

permission-studio-nav-command-rules = Reglas de comando

permission-studio-path-workspace-read = Leer espacio de trabajo

permission-studio-path-workspace-write = Escribir espacio de trabajo

permission-studio-path-external-read = Lectura externa

permission-studio-path-external-write = Escritura externa

permission-studio-path-rule-read = Modo de lectura

permission-studio-path-rule-write = Modo de escritura

permission-studio-network-internet = Internet

permission-studio-network-private = Privado

permission-studio-network-loopback = Loopback

permission-studio-tool-default = Valor predeterminado de herramientas

permission-studio-tool-default-summary = default {$value}

permission-studio-add-path-rule = Add Path Rule

permission-studio-add-network-rule = Add Network Target

permission-studio-add-name = Agregar nombre

permission-studio-add-tool-rule = Add Tool Rule

permission-studio-rule-key = Clave

permission-studio-rule-pattern = Patrón

permission-studio-rule-target = Meta

permission-studio-rule-mode = Modo

permission-studio-tool-rule-fallback = Fallback Mode

permission-studio-error-empty-value = {$field} no puede estar vacío.

overlay-providers-title = Proveedores

overlay-providers-prompt = Elija un proveedor para configurarlo

overlay-provider-list-title = Lista de proveedores

overlay-provider-list-prompt = Buscar proveedores configurados

overlay-provider-list-footer = Seleccione Crear Proveedor o un proveedor existente, luego pulse Enter

overlay-provider-list-create-label = + Proveedor nuevo

overlay-provider-list-row-detail-no-model = {$adapter} · {$count} configured adapters

overlay-provider-studio-title = Config del proveedor

overlay-provider-studio-header = Config del proveedor

overlay-provider-studio-footer = Paneles de Tab/Shift+Tab · Arrows select · Anillo de espacio · Enter edit · Ctrl+D delete selected · Ctrl+R refrescante · Ctrl+N add model · Ctrl+ Un adaptador de ahorro · Ctrl+S proveedor de ahorro · Esc close

overlay-provider-studio-providers = Proveedores

overlay-provider-studio-draft = Proyecto

overlay-provider-studio-adapters = Adaptadores

overlay-provider-studio-models = Modelos

overlay-provider-studio-catalog = Catálogo modelo
overlay-provider-studio-catalog-picker-title = Elegir plantilla de catálogo

overlay-provider-studio-catalog-picker-footer = Arriba/Abajo seleccionar · Ctrl+F buscar · Izquierda/Derecha paginar · Intro rellena los campos del modelo · Esc atrás

overlay-provider-studio-catalog-match = Catálogo: {$model}

overlay-provider-studio-catalog-selected = Catálogo seleccionado: {$model}

overlay-provider-studio-catalog-loading = Buscando coincidencia en el catálogo…

overlay-provider-studio-catalog-no-match = Sin coincidencia en el catálogo · elige en los ajustes de modelo

overlay-provider-studio-detail = Detalle

overlay-provider-studio-adapter-models-empty = Seleccione adaptadores, luego enumera sus modelos en vivo

overlay-provider-studio-models-empty = No hay modelos de adaptador disponibles

overlay-provider-studio-catalog-empty = No hay entradas de catálogo coinciden con esta consulta

overlay-provider-studio-new-provider-detail = Proyecto de proveedor de servicios

overlay-provider-studio-provider-row-detail-no-model = {$adapter} · {$count} configured adapters

overlay-provider-studio-model-count = {$count} models

overlay-provider-studio-loaded = cargado

overlay-provider-studio-error = error

overlay-provider-studio-configured = configurado

overlay-provider-studio-live-list = lista en vivo

overlay-provider-studio-not-listed = no incluido

overlay-provider-studio-not-supported = no apoyado por el actual contrato de austeridad

overlay-provider-studio-edit-title = Editar campo

overlay-provider-studio-edit-prompt = Update {$field}

overlay-provider-studio-edit-footer = Tipo para editar

overlay-provider-studio-model-edit-footer = Ctrl+S guardan configuración modelo

overlay-provider-studio-model-json-title = Model Config · {$adapter}/{$model}

overlay-provider-studio-model-json-prompt = Editar el modelo de proveedor persistido JSON.

overlay-provider-studio-model-title = Model · {$adapter}/{$model}

overlay-provider-studio-model-footer = Las flechas seleccionan · Enter edita · Ctrl+S guarda · Ctrl+D elimina · Esc vuelve

overlay-provider-delete-title = Suprimir Proveedor

overlay-provider-delete-adapter-title = Delete Adapter

overlay-provider-delete-model-title = Eliminar el modelo

overlay-provider-studio-model-edit-title = Editar modelo de campo

overlay-provider-studio-model-field-prompt = Update {$field}

overlay-provider-studio-new-model-title = Agregar modelo

overlay-provider-studio-edit-auth-mode-prompt = Actualizar el modo de auth (none ¦

overlay-provider-studio-edit-auth-subtype-prompt = Actualizar el subtipo de auth (api: custom TENIDO cline api TEN gitlab api TENED Sigv4 · credencial: openai chatgpt ANTERIGTH copilot TEN TERRI TEN google adc ANTE sap ai core)

overlay-provider-studio-edit-auth-login-method-prompt = Actualizar el método de inicio de sesión de auth (dispositivo Silencioso navegador)

provider-studio-auth-status-pending = pendientes

provider-studio-auth-status-unset = unset

provider-studio-auth-status-none = ninguno

provider-studio-auth-status-select-subtype = select subtype

provider-studio-auth-status-select-issuer = select subtype

provider-studio-auth-status-configured = configurado

provider-studio-auth-status-partial = parcial

provider-studio-summary-env = env

provider-studio-summary-callback = callback

provider-studio-summary-redirect = redirección

provider-studio-summary-account = cuenta

provider-studio-summary-name = nombre

provider-studio-summary-user = usuario

provider-studio-summary-email = email

provider-studio-summary-profile = perfil

provider-studio-summary-region = región

provider-studio-summary-code = código

provider-studio-summary-state = state {$state}

provider-studio-summary-tokens-set = tokens set

provider-studio-summary-keys-set = llaves

provider-studio-summary-set-field = set {$field}

provider-studio-summary-review-fields = auth fields

provider-studio-summary-start-browser = navegador OAuth

provider-studio-summary-restart-browser = reinicio navegador OAuth

provider-studio-summary-open-authorize = abierta autorización URL

provider-studio-summary-start-device = inicio de sesión del dispositivo

provider-studio-summary-restart-device = reinicio del dispositivo

provider-studio-summary-open-verify = URL de verificación abierta

provider-studio-summary-finish-callback = cambio de llamada final

provider-studio-summary-poll-every = poll every {$seconds}s

provider-studio-summary-paste-callback = pasta URL de llamada

provider-studio-summary-poll-now = encuesta ahora

provider-studio-summary-start-auth-first = primer comienzo

provider-studio-summary-poll-browser = resultado del navegador

provider-studio-auth-openai-ready = El navegador OAuth está listo. Abra la URL de autorización a continuación.

provider-studio-auth-openai-device-ready = El inicio de sesión de OpenAI mediante dispositivo está listo. Abre la URL de verificación de abajo e introduce {$code}

provider-studio-auth-authorize = authorize {$url}

provider-studio-auth-redirect = redirect {$url}

provider-studio-auth-paste-callback = Pega la URL de redirección en Callback URL y pulsa p · estado {$state}

provider-studio-auth-copilot-ready = El inicio de sesión mediante dispositivo está listo. Abre la URL de verificación de abajo e introduce {$code}

provider-studio-auth-verify = verify {$url}

provider-studio-auth-poll = Pulsa p para consultar ahora · cada {$seconds}s

provider-studio-auth-gitlab-ready = El navegador GitLab OAuth está listo. Abra la URL de autorización a continuación.

provider-studio-auth-atomgit-ready = AtomGit navegador sesión listo · la URL de autorización se muestra abajo

provider-studio-auth-finish-browser = Completa el flujo en el navegador y luego pulsa p · estado {$state}

flash-settings-updated = updated {$path}

flash-settings-cleared = cleared {$path}

flash-provider-save-error-settings-object = configuración del proveedor existente debe ser un objeto JSON

command-settings-summary = Abra el banco de trabajo de configuración unificada para modelos, permisos, plugins, tiempo de ejecución, sesiones, interfaz y diagnósticos

settings-mcp-public-url-updated = Agena MCP URL pública actualizada

settings-mcp-oauth-issuer-updated = Agena MCP OAuth emisor URL actualizada

settings-mcp-oauth-password-updated = Agena MCP OAuth password updated

settings-mcp-server-enabled-flash = Servidor MCP de Agena habilitado

settings-mcp-server-disabled-flash = Desactivado servidor Agena MCP

settings-mcp-auth-mode-updated = Modo de autenticación de Agena MCP establecido en {$mode}

settings-mcp-anonymous-access-updated = Acceso anónimo a herramientas de Agena MCP establecido en {$policy}

settings-mcp-oauth-password-cleared = Agena MCP OAuth password cleared

permission-studio-command-pattern-title = {$tool_name} command pattern

settings-tool-api-list-description = Enumerar herramientas de ejecución.

settings-tool-api-search-description = Herramientas de ejecución de búsqueda.

settings-tool-api-help-description = Inspeccione los contratos de herramientas de ejecución.

settings-tool-api-tags-description = List execution-tool tags.

settings-tool-api-call-description = Invoca una herramienta de ejecución.

settings-tool-api-plugins-list-description = Enumerar plugins de herramientas.

settings-tool-api-plugins-search-description = Buscar plugins de herramientas.

settings-tool-api-plugins-tags-description = List tool-plugin tags.

permission-studio-command-pattern-help = Introduzca un patrón glob de comando de shell, por ejemplo `git status` o `git push *`.

permission-studio-rename-unsupported = Esta entrada no se puede renombrar; elimínela y vuelva a crearla.

# Settings, provider, permission, catalog, MCP, and diagnostics completion
overlay-editor-footer-single-line = Escribe para editar
overlay-editor-footer-multiline = Ctrl+S guardar
context-help-title = Ayuda contextual
context-help-eyebrow = Interfaz actual
context-help-footer = ↑/↓ desplazarse · Esc o Ctrl+H cerrar
context-help-global-hint = Ctrl+H ayuda
context-help-context-composer-items = Artículos del compositor
context-help-context-suggestions = Sugerencias
context-help-context-usage = Panel de uso
context-help-context-plan-viewer = Visor de planos
context-help-context-user-input = Solicitud de entrada del usuario
context-help-context-plugin-list = Banco de trabajo de complementos · Lista
context-help-context-plugin-detail = Banco de trabajo de complementos · Detalles
context-help-context-plugin-config = Banco de trabajo de complementos · Configuración
context-help-context-plugin-actions = Configuración del complemento · Acciones
context-help-context-plugin-selection = Configuración del complemento · Selección
context-help-context-plugin-drilldown = Configuración del complemento · Desglose
context-help-context-plugin-diff = Configuración del complemento · Diferencia
context-help-key-delete = Eliminar el elemento seleccionado.
context-help-key-plugin-restart = Reinicie el complemento seleccionado cuando sea compatible.
overlay-permission-title = Solicitud de permiso
overlay-permission-details-title = Detalles
overlay-permission-action-tool = herramienta: { $tool }
overlay-permission-action-path = ruta { $access }: { $path }
overlay-permission-action-network = red: { $target }
overlay-permission-field-tool = Herramienta
overlay-permission-field-target = Comando o objetivo
overlay-permission-field-access = Acceso
overlay-permission-field-path = Camino
overlay-permission-field-workspace = Espacio de trabajo
overlay-permission-field-network = URL o destino de red
overlay-permission-field-host = Anfitrión
overlay-permission-field-reason = Por qué se necesita aprobación
overlay-permission-detail-request-id = Solicitar identificación
overlay-permission-detail-source = Fuente de política
overlay-permission-detail-scope = Alcance solicitado
overlay-permission-detail-operator = Solicitado por
overlay-permission-detail-trace = Seguimiento de decisión
overlay-permission-summary-more-approvals = También aprobando { $count } más acciones en esta llamada de herramienta
overlay-permission-detail-requested-actions = También solicita aprobación para
overlay-permission-detail-related-actions = Ya permitido en esta convocatoria
overlay-permission-choice-auto-approve = Aprobar automáticamente...
overlay-permission-rule-workbench-title = Regla de permiso
overlay-permission-rule-studio-footer = Flechas seleccionar · Entrar editar · Ctrl+O explorar ruta seleccionada · Ctrl+S guardar · Ctrl+D revocar · Esc cerrar
overlay-permission-rule-studio-footer-return = Flechas seleccionar · Ingresar editar · Ctrl+O explorar ruta seleccionada · Ctrl+S guardar · Ctrl+D revocar · Esc regresa a la solicitud de permiso
flash-permission-rule-browse-path-selection = Seleccione Ruta de destino o Raíz del espacio de trabajo antes de navegar.
overlay-permission-rule-choice-subject-title = Elija el tipo de tema
overlay-permission-rule-choice-subject-prompt = Elija el tipo de asunto de la regla.
overlay-permission-rule-choice-subject-tool-detail = coincidir con una herramienta o herramienta de tiempo de ejecución
overlay-permission-rule-choice-subject-path-access-detail = coincidir con el acceso al sistema de archivos
overlay-permission-rule-choice-subject-network-access-detail = coincidir con el acceso a la red
overlay-permission-rule-choice-access-title = Elija el tipo de acceso al camino
overlay-permission-rule-choice-access-prompt = Elija el modo de acceso al sistema de archivos.
overlay-permission-rule-choice-access-read-detail = permitir solo lecturas de archivos
overlay-permission-rule-choice-access-write-detail = permitir sólo escrituras de archivos
overlay-permission-rule-choice-access-read-write-detail = permitir lecturas y escrituras
overlay-permission-rule-choice-scope-title = Elija el alcance de la regla
overlay-permission-rule-choice-scope-prompt = Elija con qué amplitud debe persistir la regla.
overlay-permission-rule-choice-scope-session-detail = solo esta sesion
overlay-permission-rule-choice-scope-workspace-detail = todas las sesiones en este espacio de trabajo
overlay-permission-rule-choice-scope-global-detail = todos los espacios de trabajo
overlay-permission-rule-choice-mode-title = Elija el modo de regla
overlay-permission-rule-choice-mode-prompt = Elija permitir, preguntar o rechazar.
overlay-permission-rule-choice-mode-allow-detail = permitir siempre acciones coincidentes
overlay-permission-rule-choice-mode-auto-detail = dejar que el modelo de aprobación configurado decida; recurrir a un mensaje cuando no esté disponible
overlay-permission-rule-choice-mode-ask-detail = Preguntar antes de permitir acciones coincidentes.
overlay-permission-rule-choice-mode-deny-detail = siempre negar acciones coincidentes
overlay-permission-rule-editor-footer = Escribe para editar
overlay-permission-rule-editor-tool-name-title = Editar nombre de herramienta
overlay-permission-rule-editor-tool-name-prompt = Introduzca el nombre exacto de la herramienta.
overlay-permission-rule-editor-qualifier-title = Editar calificador
overlay-permission-rule-editor-qualifier-prompt = Introduzca un calificador opcional o déjelo vacío.
overlay-permission-rule-editor-workspace-root-title = Editar raíz del espacio de trabajo
overlay-permission-rule-editor-workspace-root-prompt = Introduzca un directorio raíz_espacio de trabajo opcional.
overlay-permission-rule-editor-target-path-title = Editar ruta de destino
overlay-permission-rule-editor-target-path-prompt = Ingrese la ruta o patrón de destino.
overlay-permission-rule-editor-network-target-title = Editar destino de red
overlay-permission-rule-editor-network-target-prompt = Introduzca un host, host:puerto o URL.
overlay-permission-rule-editor-session-id-title = Editar ID de sesión
overlay-permission-rule-editor-session-id-prompt = Ingrese la identificación de la sesión de destino.
overlay-permission-rule-browser-workspace-root-title = Elija la raíz del espacio de trabajo
overlay-permission-rule-browser-workspace-root-prompt = Busque directorios y presione Entrar para seleccionar uno.
overlay-permission-rule-browser-target-path-title = Elija la ruta de destino
overlay-permission-rule-browser-target-path-prompt = Busque archivos o directorios y presione Entrar para seleccionar uno.
overlay-permission-rule-browser-footer = Seleccione ../ o un directorio y presione Enter para explorar · seleccione un valor y presione Enter para aceptar
overlay-permission-rule-browser-empty = No hay archivos ni directorios coincidentes.
overlay-permission-rule-item-subject-kind = Tipo de sujeto
overlay-permission-rule-item-subject-kind-detail = Elija si esta regla se aplica a una herramienta, una ruta o un objetivo de red.
overlay-permission-rule-item-mode = Modo
overlay-permission-rule-item-mode-detail = Elija si se permiten, solicitan o rechazan acciones coincidentes.
overlay-permission-rule-item-scope = Alcance
overlay-permission-rule-item-scope-detail = Conserve esta regla para la sesión, el espacio de trabajo o globalmente.
overlay-permission-rule-item-session-id = ID de sesión
overlay-permission-rule-item-session-id-detail = ID de sesión de destino utilizada cuando alcance=sesión.
overlay-permission-rule-item-tool-name = Nombre de la herramienta
overlay-permission-rule-item-tool-name-detail = Nombre exacto de la herramienta para que coincida.
overlay-permission-rule-item-qualifier = Clasificatorio
overlay-permission-rule-item-qualifier-detail = Calificador opcional para reglas de herramientas más específicas.
overlay-permission-rule-item-access-kind = Tipo de acceso
overlay-permission-rule-item-access-kind-detail = Elija leer, escribir o leer_escribir.
overlay-permission-rule-item-target-path = Ruta de destino
overlay-permission-rule-item-target-path-detail = Patrón de ruta o ruta exacta a proteger.
overlay-permission-rule-item-workspace-root = Raíz del espacio de trabajo
overlay-permission-rule-item-workspace-root-detail = Directorio base opcional utilizado para interpretar rutas de destino relativas.
overlay-permission-rule-item-network-target = Objetivo de red
overlay-permission-rule-item-network-target-detail = Host, host:puerto o URL de destino que coincida.
overlay-permission-rule-detail-subject-kind = Las reglas de herramientas coinciden por nombre de herramienta y calificador opcional. Las reglas de ruta coinciden con el acceso al sistema de archivos. Las reglas de red coinciden con el acceso al host o a la URL.
overlay-permission-rule-detail-tool-name = Las reglas de herramientas requieren un nombre de herramienta exacto, por ejemplo `shell`, `read` o `web_search`.
overlay-permission-rule-detail-qualifier = El calificador es opcional. Déjelo vacío a menos que la herramienta o acción necesite una coincidencia más estrecha.
overlay-permission-rule-detail-path-access-kind = Utilice `read`, `write` o `read_write` dependiendo del acceso al sistema de archivos que desee igualar.
overlay-permission-rule-detail-workspace-root = Deje workspace_root vacío para heredar la raíz del espacio de trabajo en tiempo de ejecución. Configúrelo explícitamente cuando la ruta protegida se encuentre en otro lugar.
overlay-permission-rule-detail-target-path = Ingrese una ruta o patrón. Las rutas relativas se interpretan en relación con workspace_root cuando se establece.
overlay-permission-rule-detail-network-target = Ingrese un host, `host:port`, o una URL completa, dependiendo de qué tan específica deba ser la regla.
overlay-permission-rule-detail-scope = El alcance de la sesión es mejor para anulaciones temporales. El espacio de trabajo y los ámbitos globales persisten por más tiempo.
overlay-permission-rule-detail-session-id = Las reglas con ámbito de sesión requieren una identificación de sesión concreta.
overlay-permission-rule-detail-mode = Permitir permite que se realice la acción, solicita aprobación y denegar la bloquea.
overlay-workbench-details = Detalles
overlay-permission-studio-title = Permiso
overlay-permission-studio-footer-nested = Ctrl+N agregar · Entrar editar · Ctrl+E renombrar · Ctrl+D eliminar · Esc volver
permission-studio-catalog-prompt = Busque en el catálogo de herramientas en vivo. Seleccione una o más entradas o elija Regla personalizada para un valor que no esté registrado actualmente.
permission-studio-catalog-custom-detail = Agregue una etiqueta o nombre de herramienta que no esté en el catálogo en vivo actual.
flash-permission-studio-catalog-empty = Seleccione al menos una entrada antes de agregar reglas.
overlay-runtime-setting-current-value = Anulación actual: { $value }
overlay-settings-help-string = Introduzca texto. Déjelo vacío o escriba `clear` para eliminar la anulación del archivo.
overlay-settings-help-bool = Ingrese verdadero/falso, activado/desactivado, sí/no o 1/0. Déjelo vacío o escriba `clear` para eliminar la anulación del archivo.
overlay-settings-help-integer = Introduzca un número entero. Déjelo vacío o escriba `clear` para eliminar la anulación del archivo.
overlay-settings-help-float = Introduzca un número. Déjelo vacío o escriba `clear` para eliminar la anulación.
overlay-choice-clear-value = Borrar valor
overlay-settings-section-plugins-description = Configure plugins y consulte sus herramientas y diagnósticos.
overlay-settings-section-providers-description = Configure los proveedores y su comportamiento de red e inspeccione el catálogo de modelos.
overlay-settings-section-model-catalog-description = Explore el catálogo de modelos resuelto, inspeccione los metadatos del modelo y actualice la caché local.
overlay-settings-section-permissions-description = Edite los permisos globales, del espacio de trabajo y de la sesión actual por separado.
overlay-settings-section-runtime-session-description = Configure las versiones del cliente de compatibilidad y el comportamiento de compactación automática de la sesión.
settings-permission-effective-detail = Sólo lectura · combinado desde global, espacio de trabajo y sesión.
settings-permission-effective-read-only = El permiso efectivo es de sólo lectura; en su lugar, edite la sesión, el espacio de trabajo o la fuente global.
settings-field-permission-approval-model-description = Variantes de modelo y pensamiento/velocidad utilizadas para decisiones automáticas de permisos; las selecciones no disponibles recurren a Preguntar
settings-workspace-approval-model-label = Modelo de aprobación automática del espacio de trabajo
settings-workspace-approval-model-description = Modelo de aprobación guardado en la configuración del proyecto; aquí sustituye a la selección global.
settings-field-tui-color-scheme-description = Detecta automáticamente el fondo del terminal o fuerza una paleta clara u oscura
settings-field-tui-graphics-description = Muestre imágenes y escriba fórmulas con Kitty, Sixel o iTerm2 cuando sea compatible; Los cambios entran en vigor después de reiniciar la TUI.
settings-field-activity-default-expanded-description = Estado de expansión predeterminado para actividades sin una anulación específica del tipo. El razonamiento permanece expandido a menos que se establezca explícitamente su tipo.
settings-activity-kind-reasoning-description = El rastro completo del pensamiento del modelo. El valor predeterminado es expandido y se puede contraer por tipo.
runtime-setting-choice-supported-model = apoyado por el modelo actual
settings-plugin-workbench-detail = Abra el banco de trabajo de complementos estructurados para conocer el estado del tiempo de ejecución, la configuración, las herramientas, los comandos, los registros y los diagnósticos.
settings-mcp-server-detail = Alternar la superficie HTTP MCP en vivo de Agena. El proceso del servidor Agena conectado sigue siendo el tiempo de ejecución real.
settings-mcp-auth-detail = Ciclo sin autenticación, OAuth completo y autenticación mixta ChatGPT. El modo mixto mantiene públicos la inicialización y el descubrimiento de herramientas; las llamadas a herramientas permanecen protegidas por OAuth a menos que el acceso anónimo esté explícitamente habilitado.
settings-mcp-anonymous-access-none-detail = Valor predeterminado seguro: ninguna llamada a la herramienta es anónima; ChatGPT aún puede inicializar y descubrir el catálogo antes de iniciar sesión.
settings-mcp-anonymous-access-read-only-detail = Opción de participación de alto riesgo: las herramientas de solo lectura se pueden ejecutar de forma anónima y pueden revelar datos privados de espacio de trabajo, sistema de archivos, configuración o diagnóstico.
settings-mcp-anonymous-access-inactive-detail = Esta política se aplica únicamente en modo de autenticación mixta; cambie la autenticación a mixta para usarla.
settings-mcp-public-url-detail = Establezca la URL del recurso HTTPS MCP canónico. Las URL del túnel MCP seguro pueden incluir la ruta completa /v1/mcp/tunnel_id; Los encabezados de solicitud reenviados nunca son confiables como identidad OAuth.
settings-mcp-oauth-issuer-detail = Establezca el emisor del servidor de autorización público orientado al navegador. OAuth administrado por Agena requiere un origen sin ruta, como https://auth.example.com; déjelo vacío cuando OAuth y MCP utilicen el mismo dominio.
settings-mcp-oauth-password-detail = Establezca la contraseña que se muestra en la página de autorización de Agena OAuth. El servidor lo almacena como un hash Argon2.
settings-mcp-oauth-password-clear-detail = Elimine la contraseña específica de MCP y recurra a la contraseña de la interfaz de usuario del servidor, si está configurada.
settings-field-runtime-codex-version-description = Versión exacta de compatibilidad con @openai/codex utilizada en los encabezados de identidad de solicitud del proveedor.
settings-field-runtime-claude-version-description = Versión exacta de compatibilidad con @anthropic-ai/claude-code utilizada en los encabezados de identidad de solicitud del proveedor.
settings-field-runtime-gemini-version-description = Versión exacta de compatibilidad con @google/gemini-cli utilizada en los encabezados de identidad de solicitud del proveedor.
settings-field-session-compaction-auto-description = Sesiones compactas automáticamente a medida que se acercan al límite de la ventana de contexto.
settings-field-session-compaction-reserved-tokens-description = Tokens reservados desde la ventana contextual al decidir cuándo compactar; claro para utilizar el valor predeterminado calculado.
settings-client-versions-refresh-description = Obtenga las últimas versiones de paquetes compatibles de npm, conserve los tres valores exactos y vuelva a cargar el tiempo de ejecución.
settings-client-versions-entry-detail = Abra las versiones de compatibilidad exactas utilizadas en los encabezados de identidad de solicitud del proveedor.
settings-client-versions-section-description = Versiones de compatibilidad exactas utilizadas en los encabezados de identidad de solicitud de proveedor. Edite cada valor o presione Ctrl+R para actualizar desde npm.
settings-provider-workbench-detail = Abra la lista de proveedores con capacidad de búsqueda antes de configurar la autenticación, los adaptadores, el enrutamiento del modelo o nuevos proveedores.
settings-provider-new-detail = Cree un nuevo proveedor, enumere los modelos de adaptadores en vivo y edite la configuración del adaptador del proveedor; Elija el modelo por separado.
settings-model-catalog-open-detail = Inspeccione los metadatos del modelo resuelto y actualice la caché del catálogo de modelos local.
permission-studio-command-rules-shell-only = Las reglas de comando se aplican a shell.exec, shell.spawn, shell.watch, shell.open y shell.write; use una regla de nombre o la predeterminada para otras herramientas.
permission-studio-detail-editable = Enter abre un editor JSON multilínea para este segmento de permiso.
permission-studio-detail-add-hint = Enter crea este elemento y lo abre inmediatamente.
permission-studio-detail-full-config-editable = Enter abre el editor JSON avanzado para el documento completo.
overlay-permission-studio-delete-title = Eliminar regla
overlay-permission-studio-delete-body = Eliminar { $kind }: { $value }
flash-permission-studio-no-add = No se puede agregar ningún elemento en la sección actual.
flash-permission-studio-no-delete = No se puede eliminar ningún elemento en la sección actual.
flash-permission-studio-no-selection = Seleccione un elemento primero.
flash-permission-studio-context-lost = Se perdió el contexto del editor de permisos. Vuelva a abrir el estudio de permisos e inténtelo de nuevo.
value-default = predeterminado
value-none = ninguno
value-clear = claro
value-path = camino
value-network = red
value-workspace = espacio de trabajo
value-external = externo
value-permission-filesystem = Sistema de archivos
value-permission-network = Red
value-permission-tools = Herramientas
value-rule-count = { $count } regla(s)
value-custom = personalizado
value-internet = internet
value-private = privado
value-loopback = bucle invertido
value-name-count = { $count } nombre(s)
value-rule-set-count = { $count } conjunto(s) de reglas
value-open = abierto
composer-prompt-history-title = Historial rápido
overlay-commands-title = Paleta de comandos
overlay-commands-prompt = Acciones de búsqueda; Los comandos que necesitan texto continúan en el compositor.
overlay-lineage-title = Historial de sucursales [#{ $session }]
overlay-lineage-prompt = Explore el árbol de ramas actual y salte a una sesión de antepasado, hermano o hijo
overlay-rewind-title = Rebobinar sesión [#{ $session }]
overlay-rewind-prompt = Elija el mensaje del usuario para retractarse, junto con todo lo que sigue
overlay-picker-loading = Cargando...
overlay-picker-empty = No hay elementos coincidentes
overlay-picker-footer = Etiqueta seleccionada con tabulación
session-model-context-window = { $value }ctx
session-model-max-output = fuera { $value }
overlay-provider-studio-detail-footer = Teclas de flecha seleccionar · Ingresar editar · Esc atrás; las acciones de autenticación son visibles en la página principal del proveedor
overlay-provider-studio-configured-disk = configurado en disco; no forma parte del contrato de autenticación actual
overlay-provider-studio-new-model-prompt = Ingrese la identificación del modelo para agregar debajo del adaptador seleccionado.
provider-field-provider-id = ID del proveedor
provider-field-auth-mode = Modo de autenticación
provider-field-auth-subtype = Subtipo de autenticación
provider-field-auth-login-method = Método de inicio de sesión de autenticación
provider-field-start-auth = Iniciar autenticación
provider-field-continue-auth = Continuar autenticación
provider-field-auth-details = Detalles de autenticación
provider-field-base-url = URL base
provider-field-instance-url = URL de instancia
provider-field-api-key-source = Fuente de clave API
provider-field-api-key-value = Valor de clave API
provider-field-redirect-uri = URI de redireccionamiento
provider-field-callback-url = URL de devolución de llamada
provider-field-refresh-token = Actualizar token
provider-field-access-token = Token de acceso
provider-field-expires-at-ms = Vence a las (ms)
provider-field-account-id = ID de cuenta
provider-field-enterprise-domain = Dominio empresarial
provider-field-region = Región
provider-field-profile = Perfil
provider-field-access-key-id = ID de clave de acceso
provider-field-secret-access-key = Clave de acceso secreta
provider-field-session-token = Token de sesión
provider-field-service-key-env = Sobre de clave de servicio
provider-field-request-timeout = Solicitar tiempo de espera (segundos)
provider-field-connect-timeout = Tiempo de espera de conexión (segundos)
provider-field-adapter-id = ID del adaptador
provider-field-model-id = ID del modelo
provider-model-field-model-id = ID del modelo
provider-model-field-enabled = Habilitado
provider-model-field-native-compaction = Compactación nativa
provider-model-field-manual-overrides = Anulaciones manuales
provider-model-field-agena-tool-mode = Modo herramienta (agena_tools.mode)
agena-tool-mode-provider-protocol-label = protocolo_proveedor
agena-tool-mode-provider-protocol-detail = Transporte definiciones y llamadas de herramientas administradas por Agena a través del protocolo de herramientas de la API del proveedor.
agena-tool-mode-disabled-label = discapacitado
agena-tool-mode-disabled-detail = No exponga herramientas administradas por Agena o nativas del proveedor a este modelo.
provider-model-field-display-name = Nombre para mostrar
provider-model-field-lifecycle = Ciclo de vida
provider-model-field-context-window = Ventana de contexto
provider-model-field-max-input = Entrada máxima
provider-model-field-max-output = Salida máxima
provider-model-field-features = Características
provider-model-field-input-modalities = Modalidades de entrada
provider-model-field-output-modalities = Modalidades de salida
provider-model-field-thinking-modes = Modos de pensamiento
provider-model-field-speed-modes = Modos de velocidad
provider-model-field-description = Descripción
provider-model-enabled-detail = Si esta ruta modelo está habilitada.
provider-model-native-compaction-detail = Pruebe el punto final de compactación de conversaciones nativo de este proveedor antes de recurrir al resumen de texto de Agena.
provider-model-manual-overrides-detail = Activado: esta ruta conserva sus propios valores. Desactivado: sigue el catalogo y recibe sus actualizaciones en el siguiente listado.
provider-model-lifecycle-detail = Valor del ciclo de vida del modelo.
provider-auth-mode-none-detail = deshabilitar los metadatos de autenticación del proveedor
provider-auth-mode-api-detail = Autenticación estilo API con un subtipo de segunda etapa para puntos finales HTTP personalizados, API Cline, tokens de puerta de enlace de GitLab o Bedrock SigV4
provider-auth-mode-credential-detail = autenticación respaldada por credenciales resuelta desde un emisor local, seleccionada en el campo de subtipo de autenticación
provider-auth-kind-unset = desarmado
provider-auth-kind-none = ninguno
provider-auth-kind-api = API
provider-auth-kind-cline = cline_api
provider-auth-kind-gitlab = gitlab_api
provider-auth-kind-credential = credencial
provider-auth-kind-credential-with-issuer = credencial:{ $issuer }
provider-auth-kind-bedrock = lecho_sigv4
provider-auth-subtype-custom-label = personalizado
provider-auth-subtype-custom-detail = Clave API genérica + autenticación de URL base para proveedores HTTP compatibles con OpenAI, Anthropic o Gemini
provider-auth-subtype-cline-api-detail = Se corrigió el punto final de la API de Cline; solo se necesita la entrada de la clave API y el descubrimiento de modelos utiliza los modelos recomendados por Cline
provider-api-key-source-inline-detail = Almacene la clave API en línea en la configuración del proveedor
provider-api-key-source-env-detail = Leer la clave API de una variable de entorno
provider-auth-subtype-gitlab-api-detail = Autenticación del token de GitLab enrutada a través de adaptadores antrópicos o openai
provider-auth-subtype-bedrock-detail = Firma de AWS Bedrock SigV4
provider-auth-login-kind-browser-label = Navegador OAuth
provider-auth-login-kind-device-label = Inicio de sesión con código de dispositivo
provider-auth-login-kind-browser-detail = Abra la URL autorizada y luego finalice la devolución de llamada redirigida.
provider-auth-login-kind-device-detail = Abra una URL de verificación breve, ingrese un código de dispositivo y luego realice una encuesta.
provider-issuer-openai-chatgpt-label = openai_chatgpt
provider-issuer-github-copilot-label = github_copilot
provider-issuer-gitlab-label = gitlab
provider-issuer-google-adc-label = google_adc
provider-issuer-sap-ai-core-label = sap_ai_core
provider-issuer-openai-chatgpt-detail = Credenciales OpenAI ChatGPT
provider-issuer-github-copilot-detail = Credenciales del copiloto de GitHub
provider-issuer-gitlab-detail = Credenciales de GitLab OAuth
provider-issuer-google-adc-detail = Credenciales predeterminadas de la aplicación de Google
provider-issuer-sap-ai-core-detail = Autenticación de clave de servicio SAP AI Core
provider-instance-url-gitlab-detail = Punto final OAuth del navegador GitLab.com
provider-redirect-local-copy-detail = URL de devolución de llamada de localhost para copiar y pegar redirecciones de OAuth
provider-region-choice-detail = Región de AWS
provider-service-key-env-detail = clave de servicio SAP AI Core predeterminada var env
overlay-model-catalog-field-model-id = ID del modelo
overlay-model-catalog-field-display = Pantalla
overlay-model-catalog-field-origin = Origen
overlay-model-catalog-field-lifecycle = Ciclo de vida
overlay-model-catalog-field-dates = Fechas
overlay-model-catalog-field-limits = Límites
overlay-model-catalog-field-inputs = Entradas
overlay-model-catalog-field-output = Salida
overlay-model-catalog-field-features = Características
overlay-model-catalog-field-modes = Modos
overlay-model-catalog-field-defaults = Valores predeterminados
overlay-model-catalog-field-runtime = Tiempo de ejecución
overlay-model-catalog-field-pricing = Precios
overlay-model-catalog-field-source = Fuente
overlay-model-catalog-limits = ctx { $context } · en { $input } · fuera { $output }
overlay-model-catalog-lifecycle-active = activo
overlay-model-catalog-lifecycle-preview = vista previa
overlay-model-catalog-lifecycle-beta = beta
overlay-model-catalog-lifecycle-alpha = alfa
overlay-model-catalog-lifecycle-experimental = experimental
overlay-model-catalog-lifecycle-deprecated = obsoleto
overlay-model-catalog-date-release = lanzamiento { $value }
overlay-model-catalog-date-updated = actualizado { $value }
overlay-model-catalog-date-cutoff = corte { $value }
overlay-model-catalog-default-thinking = pensar
overlay-model-catalog-default-speed = velocidad
overlay-model-catalog-thinking-modes = modos de pensar
overlay-model-catalog-speed-modes = modos de velocidad
overlay-model-catalog-default-verbosity = verbosidad
overlay-model-catalog-default-temperature = temperatura
overlay-model-catalog-default-top-p = arriba_p
overlay-model-catalog-default-top-k = top_k
overlay-model-catalog-parallel-tools = herramientas paralelas
overlay-model-catalog-supports-verbosity = verbosidad
overlay-model-catalog-reasoning-interleaved = razonamiento entrelazado
overlay-model-catalog-reasoning-field = campo de razonamiento
overlay-model-catalog-open-weights = pesas abiertas
overlay-model-catalog-price-input = en { "$" }{ $value }/M
overlay-model-catalog-price-output = fuera { "$" }{ $value }/M
overlay-model-catalog-price-cache-read = lectura de caché { "$" }{ $value }/M
overlay-model-catalog-price-cache-write = escritura en caché { "$" }{ $value }/M
overlay-model-catalog-tier-count = { $count } nivel(es)
permission-rule-label-path = { $access } · { $path }
permission-rule-label-network = red · { $target }
value-unset = desarmado
value-auto = automático
value-allow = permitir
value-ask = preguntar
value-deny = negar
value-read = leer
value-write = escribir
value-read-write = leer_escribir
value-yes = si
value-no = no
value-session = sesión
value-global = mundial
value-add = Añadir
value-runtime-default = tiempo de ejecución predeterminado
value-permission-rule-subject-tool = herramienta
value-permission-rule-subject-path-access = ruta_acceso
value-permission-rule-subject-network-access = acceso_red
inline-fact-source = fuente
inline-fact-scope = alcance
inline-fact-operator = operador
flash-permission-rule-saved = regla de permiso guardada: { $name }
flash-permission-rule-revoked = regla de permiso revocado: { $name }
flash-permission-rule-context-lost = Se perdió el contexto del estudio de reglas de permiso.
flash-provider-studio-context-lost = Se perdió el contexto de configuración del proveedor.
permission-rule-error-session-id-integer = El ID de sesión debe ser un número entero.
permission-rule-error-tool-name-required = las reglas de herramientas requieren un nombre de herramienta
permission-rule-error-path-access-kind-required = las reglas de ruta requieren path_access_kind
permission-rule-error-target-path-required = las reglas de ruta requieren target_path
permission-rule-error-network-target-required = las reglas de red requieren un objetivo de red
permission-rule-error-session-id-required = el alcance de la sesión requiere una identificación de sesión
flash-server-config-edit-in-settings = El archivo de configuración pertenece al servidor. Edite sus valores en Configuración en lugar de abrir una ruta local del cliente.
flash-command-requires-session = esta acción requiere una sesión abierta
flash-session-busy = la sesión está ocupada
flash-provider-not-found = proveedor no encontrado: { $provider }
flash-permission-approval-model-updated = modelo de aprobación automática actualizado: { $provider }/{ $model }
flash-global-default-model-updated = modelo global predeterminado actualizado: { $provider }/{ $model }
flash-workspace-default-model-updated = modelo predeterminado del espacio de trabajo actualizado: {$provider}/{$model}
flash-workspace-approval-model-updated = modelo de aprobación automática del espacio de trabajo actualizado: {$provider}/{$model}
flash-provider-studio-adapter-required = seleccione un adaptador primero
flash-provider-studio-adapter-not-enabled = Verifique el adaptador seleccionado antes de agregar un modelo.
flash-provider-studio-adapter-unavailable = El modo de autenticación actual no permite seleccionar este adaptador.
flash-provider-studio-model-required = seleccione primero un modelo listado
flash-provider-studio-model-id-required = se requiere identificación del modelo
flash-provider-studio-no-auth-details = no hay detalles de autenticación disponibles para el modo de autenticación actual
flash-provider-studio-catalog-refreshed = catálogo de modelos actualizado
flash-provider-studio-invalid-model-json = modelo JSON no válido: { $error }
flash-provider-studio-live-listing-unavailable = El listado de modelos en vivo no está disponible para la autenticación { $auth }
flash-provider-studio-draft-listing-unsupported = El listado preliminar de modelos solo admite adaptadores con descubrimiento de modelos en vivo. No compatible: { $adapters }
flash-provider-studio-listing-auth-required = enumerar modelos de adaptadores requiere el descubrimiento de modelos en vivo para el par de autenticación/adaptador actual o un proveedor guardado existente; la autenticación actual es { $auth }
flash-provider-studio-invalid-auth-login-method = método de inicio de sesión de autenticación no válido
flash-provider-auth-openai-browser-started = Se inició la autenticación del navegador OpenAI. Abra la URL de autorización que se muestra en el cuadro de diálogo, luego pegue la URL redirigida en la URL de devolución de llamada y presione p.
flash-provider-auth-openai-device-started = Se inició el inicio de sesión en el dispositivo OpenAI. Abra la URL de verificación que se muestra en el cuadro de diálogo, ingrese el código { $code }, luego presione p.
flash-provider-auth-copilot-device-started = Se inició el inicio de sesión en el dispositivo Copilot. Abra la URL de verificación que se muestra en el cuadro de diálogo, ingrese el código { $code }, luego presione p.
flash-provider-auth-gitlab-browser-started = Se inició la autenticación del navegador GitLab. Abra la URL de autorización que se muestra en el cuadro de diálogo, luego pegue la URL redirigida en la URL de devolución de llamada y presione p.
flash-provider-auth-atomgit-browser-started = Se inició la autenticación del navegador AtomGit. Abra la URL de autorización que se muestra en el cuadro de diálogo, complete el inicio de sesión y luego presione p para sondear.
flash-provider-auth-openai-captured = Credencial OpenAI OAuth capturada en el borrador.
flash-provider-auth-openai-pending = El inicio de sesión del dispositivo OpenAI aún está pendiente. Finalice el paso de verificación y luego presione p nuevamente.
flash-provider-auth-copilot-pending = El inicio de sesión del dispositivo Copilot aún está pendiente. Complete la aprobación del navegador y luego presione p nuevamente.
flash-provider-auth-copilot-captured = Credencial Copilot OAuth capturada en el borrador.
flash-provider-auth-gitlab-captured = Credencial GitLab OAuth capturada en el borrador.
flash-provider-auth-atomgit-pending = El inicio de sesión del navegador AtomGit aún está pendiente. Finalice el flujo del navegador, luego presione p nuevamente.
flash-provider-auth-atomgit-captured = Credencial AtomGit OAuth capturada en el borrador.
flash-provider-auth-error-unsupported = el modo de autenticación actual no admite el inicio de sesión interactivo de OAuth
flash-provider-auth-error-start-browser-first = inicie la autenticación del navegador primero con Iniciar autenticación u o
flash-provider-auth-error-start-device-first = inicie la autenticación del dispositivo primero con Iniciar autenticación u o
flash-provider-auth-error-required-field = { $field } es obligatorio
flash-provider-save-draft = Proveedor guardado { $provider } con adaptador { $adapter }.
flash-provider-save-adapter-matches = Guardado { $provider }/{ $adapter } con { $listed } modelo(s) listado(s); { $matched } catálogo coincidente.
flash-provider-save-model = Guardado { $provider }/{ $adapter }/{ $model }.
flash-provider-save-configured-model = Modelo configurado guardado { $provider }/{ $adapter }/{ $model }.
flash-provider-delete-provider = Proveedor eliminado { $provider }.
flash-provider-delete-adapter = Se eliminó el adaptador configurado { $provider }/{ $adapter } y se eliminaron los modelos { $count }.
flash-provider-delete-model = Se eliminó el modelo configurado { $provider }/{ $adapter }/{ $model }.
flash-provider-studio-adapter-delete-empty = No se ha seleccionado ninguna configuración del adaptador para eliminar.
flash-provider-save-error-required-field = { $field } es obligatorio
flash-provider-save-error-unsupported-adapters = auth { $auth } no admite adaptadores: { $adapters }; Se esperaba uno de { $supported }
flash-provider-save-error-api-base-url = La autenticación de API requiere base_url cuando se utilizan el protocolo OpenAI, adaptadores Anthropic o Gemini
flash-provider-save-error-gitlab-token = La autenticación gitlab_api requiere una fuente de clave API
flash-provider-save-error-credential-base-url = emisor de credencial `{ $issuer }` requiere base_url
flash-provider-save-error-credential-service-key-env = emisor de credencial `{ $issuer }` requiere service_key_env
flash-provider-save-error-bedrock-key-pair = bedrock_sigv4 requiere access_key_id y secret_access_key juntos
flash-provider-save-error-select-model = seleccione al menos un modelo antes de guardar el proveedor
flash-provider-save-error-adapter-object = El adaptador de proveedor `{ $adapter }` debe ser un objeto JSON.
flash-provider-save-error-model-object = La configuración del modelo de proveedor debe ser un objeto JSON.
flash-provider-save-error-configured-adapter-object = La configuración del adaptador del proveedor configurada debe ser un objeto JSON.
flash-provider-save-error-configured-models-object = Los modelos de adaptador de proveedor configurados deben ser un objeto JSON.
flash-provider-client-versions-refreshed = Versiones de cliente actualizadas: Codex { $codex }, Claude { $claude }, Gemini { $gemini }
terminal-diagnostics-title = Diagnóstico de terminales
terminal-diagnostics-eyebrow = Compatibilidad y evidencia de protocolo.
terminal-diagnostics-footer = ↑/↓ desplazarse · c/y copiar informe · Esc cerrar
terminal-diagnostics-tip = Las capas de identidad del producto y entorno se basan en evidencia; SSH genérico no puede probar el terminal de punto final real.
terminal-diagnostics-copied = Diagnóstico del terminal copiado
terminal-diagnostics-unavailable = Los diagnósticos de terminal no están disponibles en este tiempo de ejecución.
terminal-diagnostics-summary = Informe terminal respaldado por evidencia · confianza del punto final { $confidence }
terminal-diagnostics-none = ninguno
terminal-diagnostics-unknown = desconocido
terminal-diagnostics-unavailable-value = no disponible
terminal-diagnostics-term-unset = TÉRMINO no está establecido
terminal-diagnostics-section-identity = Identidad
terminal-diagnostics-section-layers = Capas de entorno
terminal-diagnostics-section-color = Color y apariencia
terminal-diagnostics-section-protocols = Protocolos activos
terminal-diagnostics-section-providers = Proveedores e integraciones
terminal-diagnostics-section-warnings = Advertencias
terminal-diagnostics-field-product = Producto
terminal-diagnostics-field-version = Versión
terminal-diagnostics-field-parsed-version = Versión analizada
terminal-diagnostics-field-compatibility = Compatibilidad
terminal-diagnostics-field-confidence = confianza
terminal-diagnostics-field-source = Fuente seleccionada
terminal-diagnostics-field-evidence = evidencia
terminal-diagnostics-field-conflicts = Conflictos
terminal-diagnostics-color-configured = Modo configurado
terminal-diagnostics-color-detected-background = Fondo detectado
terminal-diagnostics-color-detected-appearance = Apariencia detectada
terminal-diagnostics-color-source = Fuente de detección
terminal-diagnostics-color-refresh = Actualización automática
terminal-diagnostics-color-generation = Generación de apariencia
terminal-diagnostics-color-effective-appearance = Paleta de texto efectiva
terminal-diagnostics-color-formula-foreground = Color del glifo de fórmula
terminal-diagnostics-color-formula-background = Fondo de imagen de fórmula
terminal-diagnostics-color-background-images = Imágenes de fondo
terminal-diagnostics-color-mode-auto = Automático
terminal-diagnostics-color-mode-dark = oscuridad forzada
terminal-diagnostics-color-mode-light = luz forzada
terminal-diagnostics-color-appearance-dark = oscuro
terminal-diagnostics-color-appearance-light = Luz
terminal-diagnostics-color-appearance-unknown = Desconocido
terminal-diagnostics-color-appearance-conservative = Colores terminales nativos conservadores (antecedentes desconocidos)
terminal-diagnostics-color-source-osc11 = Respuesta del terminal OSC 11
terminal-diagnostics-color-source-iterm-osc4 = Respuesta del terminal iTerm2 OSC 4;-2
terminal-diagnostics-color-source-colorfgbg = Respaldo del entorno COLORFGBG
terminal-diagnostics-color-source-term-background = TERM_BACKGROUND respaldo del entorno
terminal-diagnostics-color-source-vscode-theme = VSCODE_THEME_KIND respaldo del entorno
terminal-diagnostics-color-source-unavailable = No hay terminal utilizable ni evidencia ambiental
terminal-diagnostics-color-refresh-live = En recuperación de enfoque y reanudación terminal; las actualizaciones fallidas conservan el último color conocido
terminal-diagnostics-color-refresh-startup-only = Sólo inicio; el terminal no respondió a una consulta de color actualizable
terminal-diagnostics-color-formula-background-transparent = Transparente; sólo el color del glifo de fórmula sigue la apariencia
terminal-diagnostics-color-background-images-not-sampled = No muestreado; Los píxeles de fórmula transparentes conservan el fondo del terminal o la imagen de fondo debajo.
terminal-diagnostics-direct = directo
terminal-diagnostics-direct-description = No se detectó evidencia de SSH, Mosh, multiplexor o WSL.
terminal-diagnostics-layer-description = Detectado desde { $source }. Se desconocen el orden de las capas y la profundidad de anidación.
terminal-diagnostics-capability-description = punto final={ $status } · fuente={ $source } · ruta={ $path } · proveedor={ $provider }
terminal-diagnostics-path-clear = claro
terminal-diagnostics-path-forced = forzado por anulación
terminal-diagnostics-path-unverified = no verificado
terminal-diagnostics-path-blocked = bloqueado
terminal-diagnostics-provider-not-required = no requerido
terminal-diagnostics-provider-ready = listo
terminal-diagnostics-provider-missing = faltante o no implementado
terminal-diagnostics-helper-missing = No encontrado o no ejecutable.
terminal-diagnostics-helper-not-probed = No investigado porque el punto final no está identificado como Kitty.
terminal-diagnostics-no-warnings = No se detectaron advertencias de compatibilidad.
terminal-diagnostics-protocol-alternate-screen = Pantalla alternativa
terminal-diagnostics-protocol-bracketed-paste = pasta entre corchetes
terminal-diagnostics-protocol-focus = Informes de enfoque
terminal-diagnostics-protocol-mouse = captura del mouse
terminal-diagnostics-protocol-mouse-mode = Modo de cable del mouse
terminal-diagnostics-protocol-mouse-events = Eventos de mouse recibidos
terminal-diagnostics-protocol-mouse-last = Último evento del mouse
terminal-diagnostics-mouse-mode-button-sgr = Seguimiento de eventos de botones (DECSET 1002) con coordenadas SGR (DECSET 1006)
terminal-diagnostics-mouse-events-none = Ninguno. El terminal del punto final no ha entregado ningún evento de mouse a Agena; verifique la configuración del perfil de informes del mouse y de la rueda.
terminal-diagnostics-mouse-events-seen = { $count } evento(s)
terminal-diagnostics-mouse-last-none = Ninguno
terminal-diagnostics-protocol-keyboard = Desambiguación del teclado
terminal-diagnostics-protocol-key-events = Tipos de eventos de teclado
terminal-diagnostics-protocol-background = Consulta en segundo plano
terminal-diagnostics-protocol-native-clipboard = portapapeles nativo
terminal-diagnostics-protocol-osc52-write = escritura OSC 52
terminal-diagnostics-protocol-osc52-read = lectura OSC 52
terminal-diagnostics-protocol-progress = OSC 9;4 progreso
terminal-diagnostics-provider-kitty-clipboard = Portapapeles de gatito
terminal-diagnostics-provider-kitty-transfer = Transferencia de gatitos
terminal-diagnostics-provider-iterm-transfer = transferencia iTerm2
terminal-diagnostics-provider-inline-images = Imágenes en línea
terminal-diagnostics-provider-hyperlinks = Hipervínculos
terminal-diagnostics-provider-sync-output = Salida sincronizada
terminal-diagnostics-status-confirmed = confirmado
terminal-diagnostics-status-forced = forzado por anulación
terminal-diagnostics-status-profiled = perfilado
terminal-diagnostics-status-unsupported = sin apoyo
terminal-diagnostics-status-unknown = desconocido
terminal-diagnostics-source-user = anulación de usuario
terminal-diagnostics-source-environment = medio ambiente
terminal-diagnostics-source-helper = sonda auxiliar
terminal-diagnostics-source-terminal-query = consulta terminal
terminal-diagnostics-source-profile = perfil terminal
terminal-diagnostics-source-platform = plataforma predeterminada
terminal-diagnostics-source-conservative = default conservador
terminal-diagnostics-source-terminfo = compatibilidad terminfo
terminal-diagnostics-source-unknown = desconocido
terminal-diagnostics-confidence-explicit = explícito
terminal-diagnostics-confidence-strong = fuerte
terminal-diagnostics-confidence-compatibility = solo compatibilidad
terminal-diagnostics-confidence-unknown = desconocido

# Plugin Workbench i18n completion
plugin-workbench-action-diff = diferencias
plugin-workbench-action-refresh = actualizar
plugin-workbench-action-remove-selected = eliminar/restablecer selección
plugin-workbench-action-reset-all = restablecer todo
plugin-workbench-action-restart = reiniciar
plugin-workbench-action-save = guardar
plugin-workbench-action-validate = validar
plugin-workbench-actions = Acciones
plugin-workbench-authority-unavailable = Los datos de autoridad no están disponibles.
plugin-workbench-choices = Opciones
plugin-workbench-close-footer = Esc cerrar
plugin-workbench-column-after = Después
plugin-workbench-column-args = Args.
plugin-workbench-column-arguments = Argumentos
plugin-workbench-column-before = Antes
plugin-workbench-column-category = Categoría
plugin-workbench-column-change = Cambio
plugin-workbench-column-command = Comando
plugin-workbench-column-description = Descripción
plugin-workbench-column-field = Campo
plugin-workbench-column-inputs = Entradas
plugin-workbench-column-message = Mensaje
plugin-workbench-column-plugin = Plugin
plugin-workbench-column-section = Sección
plugin-workbench-column-severity = Gravedad
plugin-workbench-column-source = Origen
plugin-workbench-column-summary = Resumen
plugin-workbench-column-tool = Herramienta
plugin-workbench-column-version = Versión
plugin-workbench-column-visible-tool = Herramienta visible
plugin-workbench-command-arguments = Argumentos: {$command}
plugin-workbench-config = Configuración
plugin-workbench-config-action = Acción
plugin-workbench-config-choose-shape = elegir forma
plugin-workbench-config-choose-type = elegir tipo
plugin-workbench-config-default = Predeterminado
plugin-workbench-config-diff = Diferencias de configuración
plugin-workbench-config-dirty = modificado
plugin-workbench-config-drilldown-footer = Izquierda/Derecha celda · Arriba/Abajo fila · Enter editar · Ctrl+D eliminar/restablecer · Esc volver
plugin-workbench-config-saved = guardado
plugin-workbench-config-setting = Ajuste
plugin-workbench-config-state = Estado
plugin-workbench-config-state-changed = cambiado
plugin-workbench-config-state-default = predeterminado
plugin-workbench-config-state-dirty = modificado
plugin-workbench-config-state-error = error
plugin-workbench-config-state-inactive = inactivo
plugin-workbench-config-summary = {$status} · {$save_state}
plugin-workbench-config-title = {$plugin} / Configuración
plugin-workbench-config-type = Tipo
plugin-workbench-config-value = Valor
plugin-workbench-config-view-summary = Configuración efectiva · {$changed} campos cambiados · celda seleccionada: {$cell}
plugin-workbench-detail-footer = Tab/Shift+Tab sección · Arriba/Abajo desplazar · Esc volver
plugin-workbench-detail-tools-footer = Tab/Shift+Tab sección · Arriba/Abajo seleccionar · Enter configurar y ejecutar · Esc volver
plugin-workbench-filter-all = Todos
plugin-workbench-filter-other = otro
plugin-workbench-header-summary = Herramientas: {$tools}        Comandos: {$commands}        Configuración: {$config}
plugin-workbench-input-preview = Vista previa de entrada: {$tool}
plugin-workbench-last-result-failed = Último resultado · {$tool} · fallido
plugin-workbench-last-result-success = Último resultado · {$tool} · correcto
plugin-workbench-list-footer = Escriba para buscar · Arriba/Abajo seleccionar · Enter abrir · Esc cerrar
plugin-workbench-list-summary = Buscar plugins… {$query}        Transporte: {$transport}        Configuración: {$config}        {$shown}/{$total} mostrados
plugin-workbench-loading-actions = Cargando acciones…
plugin-workbench-loading-choices = Cargando opciones…
plugin-workbench-no-changes = Sin cambios
plugin-workbench-no-commands = No hay comandos.
plugin-workbench-no-config-section = No hay sección de configuración.
plugin-workbench-no-editable-rows = No hay filas editables.
plugin-workbench-no-filter-matches = Ningún plugin coincide con los filtros actuales.
plugin-workbench-no-issues = Sin problemas
plugin-workbench-no-logs = No hay registros.
plugin-workbench-no-selection = No hay ningún plugin seleccionado.
plugin-workbench-no-structured-arguments = No hay argumentos estructurados.
plugin-workbench-no-tools = No hay herramientas.
plugin-workbench-none = ninguno
plugin-workbench-none-declared = ninguno declarado
plugin-workbench-overview = Resumen
plugin-workbench-package-summary = Paquete: {$package}
plugin-workbench-plugin = Plugin
plugin-workbench-plugin-capabilities = Capacidades del plugin
plugin-workbench-plugins = Plugins
plugin-workbench-provenance = Procedencia: {$provenance}
plugin-workbench-sections = Secciones
plugin-workbench-severity-error = error
plugin-workbench-severity-warning = advertencia
plugin-workbench-status-invalid = No válido
plugin-workbench-status-issues = Problemas
plugin-workbench-status-missing = Falta
plugin-workbench-status-needs-restart = Requiere reinicio
plugin-workbench-status-runtime-issue = Problema de ejecución
plugin-workbench-status-schema-missing = Falta el esquema
plugin-workbench-status-valid = Válido
plugin-workbench-status-warning = Advertencia
plugin-workbench-summary = Consulta: {$query} · transporte {$transport} · configuración {$config} · {$shown}/{$total} mostrados
plugin-workbench-tab-capabilities = Capacidades
plugin-workbench-tab-commands = Comandos
plugin-workbench-tab-config = Configuración
plugin-workbench-tab-diagnostics = Diagnóstico
plugin-workbench-tab-logs = Registros
plugin-workbench-tab-tools = Herramientas
plugin-workbench-tabs = Pestañas
plugin-workbench-tags-summary = Etiquetas: {$tags}
plugin-workbench-tool-capabilities = Capacidades de herramientas
plugin-workbench-tools-help = Arriba/Abajo selecciona una herramienta. Enter abre el formulario de esquema controlado por el host; Ctrl+S valida y ejecuta.
plugin-workbench-transport = Transporte
plugin-workbench-trust-level = Nivel de confianza: {$level}
plugin-workbench-unavailable = no disponible

# Plugin Workbench structured editor i18n completion
plugin-workbench-editor-also-matches = también coincide con: {$matches}
plugin-workbench-editor-array-action-help = Enter menú de acciones · Ctrl+D elimina la fila seleccionada
plugin-workbench-editor-array-preview = Configurar… ({$count} elementos)
plugin-workbench-editor-configure = Configurar…
plugin-workbench-editor-format = formato: {$format}
plugin-workbench-editor-generic-object = Editor de objetos genérico
plugin-workbench-editor-index = Índice
plugin-workbench-editor-item = Elemento {$index}
plugin-workbench-editor-map = Editor de mapas
plugin-workbench-editor-no-fields = No hay campos.
plugin-workbench-editor-no-items = No hay elementos.
plugin-workbench-editor-object = Editor de objetos
plugin-workbench-editor-object-action-help = Enter menú de acciones · Añadir campo desde la celda Acción
plugin-workbench-editor-object-array = Editor de tabla para matrices de objetos
plugin-workbench-editor-object-array-help = Editar abre el elemento seleccionado con el mismo editor estructurado.
plugin-workbench-editor-object-preview = Configurar… ({$count} campos)
plugin-workbench-editor-preview = Vista previa
plugin-workbench-editor-primitive-array = Editor de matrices primitivas
plugin-workbench-editor-readonly = solo lectura
plugin-workbench-editor-schema-missing = Falta el esquema        Editor estructurado básico
plugin-workbench-editor-shape = Forma
plugin-workbench-editor-suggestions = Sugerencias
plugin-workbench-editor-tuple = Editor de tuplas
plugin-workbench-editor-type-summary = Tipo: {$type}        Editor de ruta: interfaz estructurada
plugin-workbench-field-state-available = disponible
plugin-workbench-field-state-custom = personalizado
plugin-workbench-field-state-map-key = clave del mapa
plugin-workbench-field-state-missing = falta
plugin-workbench-field-state-optional = opcional
plugin-workbench-field-state-required = obligatorio
plugin-workbench-kind-all-of = allOf
plugin-workbench-kind-any-of = anyOf
plugin-workbench-kind-array = matriz
plugin-workbench-kind-boolean = booleano
plugin-workbench-kind-integer = entero
plugin-workbench-kind-null = nulo
plugin-workbench-kind-number = número
plugin-workbench-kind-object = objeto
plugin-workbench-kind-one-of = oneOf
plugin-workbench-kind-string = cadena
plugin-workbench-kind-value = valor

overlay-provider-list-create-detail = Cree un borrador de proveedor y configure después la autenticación, los adaptadores y los modelos.

overlay-provider-delete-body = ¿Eliminar el proveedor {$provider} y todos los adaptadores/modelos configurados?

overlay-provider-delete-adapter-body = ¿Eliminar el adaptador configurado {$provider}/{$adapter}?

overlay-provider-delete-adapter-last-body = Este es el último adaptador configurado. Al confirmar se eliminará el proveedor.

overlay-provider-delete-model-body = ¿Eliminar el modelo configurado {$provider}/{$adapter}/{$model}?

permission-studio-page-tool-defaults = Acceso a herramientas / Valores predeterminados

# Generic inline content resources
content-loading = Cargando salida…
content-active = Salida en directo
content-complete = Salida completada
content-interrupted = Salida interrumpida
content-gap = Parte de la salida original ya no se conserva.
content-reconnecting = Reconectando la salida…

content-window = Parte de la salida queda fuera de la ventana mostrada.

# Chrome-only activity headlines (the stored English title is the fallback).
activity-title-answer = Respuesta
activity-title-thinking = Razonamiento
activity-title-text = Texto
activity-title-error = Error
activity-title-notice = Aviso
activity-title-hook = Hook
activity-title-compaction = Compactación del contexto
activity-title-provider-retry = Reintento del proveedor
activity-title-turn-limit = Límite de turnos
activity-title-user-input = Se requiere entrada del usuario
activity-title-pasted-text = Texto pegado

# Plugin Workbench, permission editor and transcript keys localised with the settings catalog.
permission-studio-access-read = {$pattern} · lectura
permission-studio-access-write = {$pattern} · escritura
plugin-workbench-editor-pair-prompt = Introduce los dos valores para {$title}.
plugin-workbench-error-field-exists = el campo `{$field}` ya existe
plugin-workbench-error-field-name-empty = el nombre del campo no puede estar vacío
plugin-workbench-error-fixed-schema-type = {$title} tiene un tipo de esquema fijo
plugin-workbench-error-invalid-branch-selection = selección de rama no válida
plugin-workbench-error-invalid-enum-selection = selección de enumeración no válida
plugin-workbench-error-invalid-type-selection = selección de tipo no válida
plugin-workbench-error-no-selectable-schema-type = {$title} no tiene un tipo de esquema seleccionable
plugin-workbench-error-no-selection = no hay selección disponible
plugin-workbench-error-plugin-unavailable = el complemento `{$plugin}` no está disponible
plugin-workbench-error-read-only = {$title} es de solo lectura
plugin-workbench-error-rename-failed = no se pudo renombrar el campo
plugin-workbench-error-row-not-object-field = la fila seleccionada no es un campo de objeto
plugin-workbench-error-schema-mismatch = el valor de configuración del complemento no coincide con su esquema.
plugin-workbench-error-unsupported-tab = el complemento `{$plugin}` solicitó una pestaña de Workbench no compatible `{$tab}`; se abre Config
transcript-image-embedded = imagen incrustada

# Activities, composer, diagnostics and usage analytics.
activities-title = Actividades en segundo plano
activities-empty = No hay actividades en segundo plano.
activities-empty-hint = Ejecuta shell.spawn, shell.open o tasks.run para crear una.
activities-section-active = En curso
activities-section-finished = Finalizadas
activities-footer-close = Esc cerrar
activities-footer-move = ↑↓ mover
activities-footer-detail = Enter detalles
activities-footer-stop = s {$control}
activities-footer-dismiss = d {$control}
activities-footer-clear = x limpiar
activities-footer-filters = f/k/t filtros
activities-footer-refresh = r actualizar
activities-filter-active-only = solo en curso
activities-status-summary = {$active} en curso · {$finished} finalizadas
activities-status-unknown = desconocido
activities-row-message = {$message}
activities-detail-title = Detalles / final del registro
activities-detail-no-selection = No hay selección.
activities-detail-waiting = Esperando la salida…
activities-detail-no-output = No se ha registrado ninguna salida.
activities-detail-tail-hint = Pulsa ↵ para seguir el registro de esta actividad.
activities-detail-command = $ {$command}
activities-detail-session = sesión #{$id}
activities-detail-source-part = parte de origen #{$id}
activities-detail-next-wake = próxima activación {$at}
activities-detail-output-separator = seguimiento en directo @ {$seq}
activities-detail-lines-omitted = … {$count} líneas anteriores ocultas
activity-command-reference-title = Comando: {$name}
activity-detail-type = Tipo: {$media_type}
activity-detail-size = Tamaño: {$size} bytes
activity-detail-dimensions = Dimensiones: {$width}×{$height}
activity-detail-preview-fields = {$count} campos
activity-detail-preview-items = {$count} elementos
composer-chip-command = [Comando: {$name}]
composer-command-line = Comando: {$name}
composer-clipboard-files = archivos del portapapeles: {$error}
composer-clipboard-image = imagen del portapapeles: {$error}
composer-clipboard-text = texto del portapapeles: {$error}
composer-error-attachment-path = la ruta del adjunto no es un archivo normal ni un directorio del espacio de trabajo del servidor: {$path}
composer-error-attachment-unavailable = la ruta del adjunto ya no está disponible en el servidor: {$path}
composer-error-clipboard-attachment-empty = el adjunto del portapapeles está vacío: {$path}
composer-error-clipboard-attachment-not-file = el adjunto del portapapeles no es un archivo normal: {$path}
composer-error-clipboard-attachment-filename = el adjunto del portapapeles no tiene un nombre de archivo válido: {$path}
composer-error-clipboard-attachment-too-large = el adjunto del portapapeles supera el límite de carga de 20 MiB: {$path}
flash-notification-copied = Copiado de la notificación.
flash-paste-limit-draft-unchanged = El texto pegado supera el límite de 1 MiB; el borrador no se ha modificado.
flash-ui-backend-task-failed = Error en la tarea de comando del backend de la interfaz: {$error}
help-diagnostics-kitty-line = Asistente Kitty {$version} · {$path} · portapapeles {$clipboard} · transferencia {$transfer}
permission-rule-label = {$tool} · {$qualifier}
rewind-turn-label = Turno {$sequence}
rewind-turn-detail = turno {$sequence} · {$time}
terminal-diagnostics-state-path-blocked = ruta bloqueada
terminal-diagnostics-state-path-unverified = ruta sin verificar
terminal-diagnostics-state-provider-unavailable = proveedor no disponible
transcript-diagram-title = ◇ Diagrama · {$label}
usage-title = Análisis de uso
usage-title-refreshing = Análisis de uso · actualizando…
usage-to-small = El terminal es demasiado pequeño para el análisis de uso
usage-view-providers = Proveedores
usage-sort-tokens = Tokens
usage-filter-all = Todos
usage-filter-period = período {$period}
usage-filter-view = vista {$view}
usage-filter-provider = proveedor {$provider}
usage-filter-model = modelo {$model}
usage-filter-subagents = subagentes {$state}
usage-filter-sort = orden {$sort}
usage-filter-refresh = actualizar
usage-footer = ↑/↓ filas Esc cerrar · Enter abrir sesión
usage-footer-open-session = Enter abrir sesión
usage-footer-rows = ↑/↓ filas Esc cerrar
usage-loading = Cargando estadísticas de uso…
usage-total-cost = Costo total
usage-cost-line = registrado {$recorded} · estimado {$estimated}
usage-tokens = Tokens
usage-tokens-line = entrada {$input} · salida {$output} · razonamiento {$reasoning}
usage-sessions = Sesiones
usage-active-days = {$count} días activos
usage-average-cost = {$cost} de media/solicitud
usage-requests = Solicitudes
usage-cache-hit = Aciertos de caché
usage-cache-line = lectura {$read} · escritura {$write} · 5 min {$write_5m} · 1 h {$write_1h}
usage-days-summary = {$days} días activos · {$average} de media/día · máximo {$peak} el {$date}
usage-unpriced-requests = ⚠ {$count} solicitudes sin precio
usage-empty = Sin uso
usage-empty-filtered = No hay uso para los filtros seleccionados
usage-collecting = Recopilando el uso de los mensajes…
usage-error-badge = Error
usage-error-body = No se pueden cargar las estadísticas de uso
usage-error-hint = Pulsa Ctrl+R para actualizar
usage-breakdown-provider = Desglose por proveedor
usage-breakdown-model = Desglose por modelo
usage-breakdown-session = Desglose por sesión
usage-breakdown-daily = Desglose diario
usage-by-provider = Por proveedor
usage-by-model = Por modelo
usage-daily-activity = Actividad diaria
usage-column-name = nombre
usage-column-cost = costo
usage-column-tokens = tokens
usage-column-requests = solicitudes
usage-column-cache = caché

# Remaining chrome keys for localized interface surfaces.
activities-status-pending = pendiente
activities-status-running = en ejecución
activities-status-waiting = en espera
activities-status-paused = en pausa
activities-status-succeeded = completada
activities-status-failed = fallida
activities-status-cancelled = cancelada
activities-status-stopped = detenida
activities-control-stop = detener
activities-control-pause = pausar
activities-control-resume = reanudar
activities-control-delete = eliminar
activities-control-dismiss = descartar
activities-kind-shell = Shell
activities-kind-monitor = Monitoreo
activities-kind-task = Tarea
activities-kind-cron = Tarea programada
activities-kind-runtime = Entorno de ejecución
activities-kind-browser = Navegador
activities-kind-activity = Actividad
activity-detail-details = Detalles
activity-detail-language = Idioma
activity-detail-notice = Aviso
activity-detail-request = Solicitud
activity-detail-source = Origen
activity-section-error = Error
activity-section-input = Entrada
activity-section-output = Salida
activity-title-plan-review = Revisión del plan
composer-chip-file = [Archivo: {$name}]
composer-chip-folder = [Carpeta: {$name}]
composer-clipboard-text-empty = el texto del portapapeles está vacío
composer-error-attachment-budget = el adjunto supera el límite de 20 MiB por archivo o de 40 MiB por mensaje
composer-error-attachment-outside-workspace = El adjunto debe estar dentro del espacio de trabajo activo.
composer-error-clipboard-attachments-budget = los adjuntos del portapapeles superan el límite total de carga de 40 MiB
composer-error-clipboard-file-changed = el archivo del portapapeles ha cambiado o supera el límite de lectura
composer-error-empty-message = El mensaje está vacío.
composer-error-paste-limit = El texto pegado supera el límite de 1 MiB. El portapapeles y el borrador actual no han cambiado.
composer-notice-paste-as-text = El contenido largo pegado se ha conservado como texto porque se alcanzó el límite de adjuntos.
context-help-context-activities = Actividades en segundo plano
flash-attachment-prepare-failed = No se pudo preparar un adjunto; revisa el borrador y vuelve a enviarlo.
flash-attachment-preparing = Se están preparando los adjuntos; el mensaje se enviará cuando estén listos.
flash-clipboard-osc52-sent = Solicitud de portapapeles enviada al terminal (OSC 52).
flash-command-no-file-view = Las referencias de comandos no tienen vista de archivo.
flash-download-outside-workspace = Las rutas de descarga no pueden salir del espacio de trabajo actual.
flash-download-relative-path = Usa una ruta relativa al espacio de trabajo actual.
flash-download-usage = Uso: /download <ruta-del-espacio-de-trabajo>
flash-ui-backend-queue-full = La cola de comandos del backend de la interfaz está llena; espera a que terminen las operaciones actuales
flash-ui-backend-unavailable = El servicio de comandos del backend de la interfaz no está disponible
help-diagnostics-kitty-helper = Asistente Kitty
message-role-tool = herramienta
permission-studio-command-pattern-suffix = + patrón de comando
search-picker-preview-title = Vista previa
search-picker-search-failed = Error en la búsqueda
search-picker-no-preview = No hay vista previa disponible
search-picker-page = Página {$current}/{$total}
search-picker-searching = buscando…
search-picker-loading-more = cargando más…
search-picker-error = error
search-picker-results = {$count} resultados
search-picker-footer-space = Space alternar · {$count} seleccionados
search-picker-footer-enter = Enter confirmar
search-picker-footer-navigate-page = ↑/↓ navegar · ←/→ página
search-picker-footer-page = ←/→ página
search-picker-footer-navigate = ↑/↓ navegar
search-picker-footer-enter-select = Enter seleccionar
search-picker-footer-close = Esc cerrar
settings-choice-log-level-detail = nivel de registro
terminal-diagnostics-state-confirmed = confirmado
terminal-diagnostics-state-forced = forzado
terminal-diagnostics-state-profiled = inferido del perfil
terminal-diagnostics-state-unsupported = no compatible
terminal-diagnostics-state-unknown = desconocido
timeline-label-role = Rol
timeline-label-state = Estado
timeline-label-revision = Revisión
timeline-label-run = Ejecución
timeline-label-parent-part = Parte superior
transcript-alert-note = Nota
transcript-alert-tip = Consejo
transcript-alert-important = Importante
transcript-alert-warning = Advertencia
transcript-alert-caution = Precaución
transcript-code-block-empty = (vacío)
transcript-diff-changes = Cambios
transcript-image-placeholder = Imagen
usage-view-overview = Resumen
usage-view-daily = Diario
usage-view-models = Modelos
usage-view-sessions = Sesiones
usage-sort-cost = Costo
usage-sort-requests = Solicitudes
usage-filter-subagents-included = incluidos
usage-filter-subagents-excluded = excluidos
search-picker-custom-value-label = Usar el valor escrito
search-picker-custom-value-detail = Aplicar exactamente {$value}
search-picker-custom-path-label = Usar la ruta escrita

context-help-key-toggle-favorite = Marcar o desmarcar como favorita la sesión seleccionada.
context-help-tip-ctrl-h = Ctrl+H siempre abre la ayuda de la interfaz que tienes delante.
context-help-section-navigation = Navegación
context-help-section-actions = Acciones
context-help-section-editing = Edición
context-help-section-workflow = Flujo de trabajo
context-help-section-search = Búsqueda y selección
context-help-section-selection = Selección
context-help-summary-sessions = Explora las sesiones cargadas y elige la conversación activa.
context-help-summary-transcript = Lee, navega, despliega, busca y copia la conversación actual.
context-help-summary-composer = Escribe un mensaje, gestiona elementos preparados y envía trabajo a la sesión activa.
context-help-summary-composer-items = Examina, abre, reordena o elimina archivos adjuntos y elementos pegados.
context-help-summary-history = Busca mensajes recientes, del más nuevo al más antiguo, sin salir del compositor.
context-help-summary-suggestions = Elige un comando o una mención de archivo que coincida con la búsqueda.
context-help-summary-editor = Edita un valor y envíalo o cancela.
context-help-summary-editor-multiline = Edita texto estructurado en varias líneas y guarda el valor completo.
context-help-summary-search-picker = Busca entre las opciones disponibles y activa un resultado.
context-help-summary-choice-list = Elige una opción visible sin cambiar el texto de búsqueda.
context-help-summary-timeline = Busca eventos de sesión y salta al mensaje relacionado.
context-help-summary-permission = Revisa la solicitud, consulta los detalles y toma una decisión explícita.
context-help-summary-details = Lee los detalles y vuelve a la página de decisión anterior.
context-help-summary-user-input = Responde a la pregunta actual y continúa con el resto de la solicitud.
context-help-summary-user-input-editor = Escribe una respuesta personalizada o vuelve a las opciones.
context-help-summary-user-input-review = Revisa todas las respuestas antes de enviar el resultado.
context-help-summary-user-input-decision-review = Elige la decisión final y desplázate por los detalles por separado.
context-help-summary-confirm = Confirma la operación mostrada o déjala sin cambios.
context-help-summary-usage = Consulta el uso y usa atajos para filtrar o actualizar el panel.
context-help-summary-model-catalog = Explora el catálogo de modelos y usa atajos para buscar, actualizar y cambiar de página.
context-help-summary-list = Recorre las filas visibles y activa el elemento seleccionado.
context-help-summary-panes = Cambia entre paneles visibles y opera la fila o control seleccionado.
context-help-summary-action-pane = Navega los permisos y usa atajos para gestionar reglas.
context-help-summary-provider = Configura campos, adaptadores y modelos del proveedor; guarda los cambios disponibles.
context-help-summary-plugin-list = Filtra complementos, ajusta los filtros visibles y abre uno.
context-help-summary-plugin-detail = Cambia entre pestañas de detalles y desplázate por la seleccionada.
context-help-summary-plugin-config = Navega la configuración de complementos y usa atajos para acciones de página.
context-help-summary-plugin-actions = Elige una acción para el valor de configuración seleccionado.
context-help-summary-plugin-selection = Selecciona uno o varios valores permitidos y confirma.
context-help-summary-plugin-drilldown = Navega las filas anidadas y activa la celda seleccionada.
context-help-summary-plugin-diff = Revisa los cambios de configuración pendientes antes de volver.
context-help-key-move = Mueve la selección un elemento.
context-help-key-page = Avanza una página visible.
context-help-key-half-page = Avanza media página visible de la conversación.
context-help-key-scroll-line = Desplaza la vista una línea sin mover el cursor.
context-help-key-jump-history = Recorre las posiciones recientes del cursor hacia atrás o adelante.
context-help-key-first-last = Salta al primer o último elemento.
context-help-key-open = Abre el elemento seleccionado.
context-help-key-session-scope = Elige el ámbito de sesiones All, Roots o Subtree.
context-help-key-session-cycle = Cambia el ámbito de sesiones.
context-help-key-transcript-vertical = Mueve una fila visual y conserva la columna de destino.
context-help-key-transcript-horizontal = Salta entre mensajes completos.
context-help-key-visual-select = Inicia selección Visual de carácter, línea o bloque; o/O cambia extremos, gv la restaura, y la copia y Esc la cancela.
context-help-key-count = Repite el siguiente movimiento de Vim.
context-help-key-insert-mode = Entra en modo INSERT del compositor.
context-help-key-toggle = Alterna el valor seleccionado o el bloque desplegable.
context-help-key-expand-all = Carga todas las partes sin cambiar sus detalles.
context-help-key-collapse-parts = Pliega esta respuesta (zc) o todas (zC) a las 5 partes más recientes e interacciones pendientes; conserva los detalles individuales.
context-help-key-copy = Copia una selección Visual, un movimiento Vim, la línea, el bloque Markdown o el mensaje completo.
context-help-key-search-transcript = Busca coincidencias en la conversación y las recorre.
context-help-key-word = Avanza por palabra o WORD, o al inicio, primer carácter no blanco o final de línea.
context-help-key-find = Busca la siguiente o anterior aparición del carácter (f/F/t/T) y repite con ; o ,.
context-help-key-text-object = Selecciona una palabra, párrafo, bloque Markdown o mensaje completo.
context-help-key-mouse = Desplázate, haz clic o arrastra para situar el cursor y seleccionar texto; arrastra la cabecera de sesión, el estado del modelo o el compositor y copia con Ctrl+Y (o y en modo VIEW).
context-help-key-line-start-end = Salta al inicio, primer carácter no blanco o final de la línea visual.
context-help-key-count-prefix = Escribe un número antes de un movimiento Vim para repetirlo (por ejemplo, 5j, 3w, 10G).
context-help-key-viewport = Salta a la línea superior, central o inferior visible.
context-help-key-scroll-viewport = Desplaza la vista para colocar el cursor arriba, en el centro o abajo.
context-help-key-visual-endpoints = Cambia el extremo activo de la selección Visual.
context-help-key-visual-restore = Restaura la selección Visual más reciente.
context-help-key-cancel-selection = Cancela la selección Visual o de texto con el ratón.
context-help-key-copy-line = Copia la línea actual.
context-help-key-copy-motion = Copia desde el cursor hasta el final de línea o según un movimiento Vim.
context-help-key-copy-text-object = Copia una palabra, párrafo, bloque Markdown o mensaje completo.
context-help-key-send = Envía el mensaje actual del compositor.
context-help-key-send-now = Interrumpe la ejecución activa y envía este mensaje ahora.
context-help-key-newline = Inserta un salto de línea sin enviar.
context-help-key-view-mode = Vuelve al modo VIEW de la conversación.
context-help-key-editor-move = Mueve el cursor de texto por carácter, línea o palabra.
context-help-key-editor-edit = Borra, elimina o restaura texto con edición al estilo de shell.
context-help-key-history = Abre el historial de mensajes del compositor.
context-help-key-recover = Edita en el compositor el mensaje pendiente.
context-help-key-cancel-pending = Cancela un mensaje que aún se está enviando.
context-help-key-clear-composer = Borra el compositor y todos los elementos preparados.
context-help-key-items = Abre los elementos preparados del compositor.
context-help-key-commands = Abre o escribe comandos con barra.
context-help-key-attach = Adjunta un archivo.
context-help-key-insert-content = Abre el panel para insertar rutas locales y archivos adjuntos.
context-help-key-external-editor = Edita el mensaje en el editor externo configurado.
context-help-key-image = Pega archivos, imágenes o texto del portapapeles.
context-help-key-pending-requests = Abre la solicitud pendiente de entrada o permiso.
context-help-key-next = Avanza al siguiente elemento.
context-help-key-previous = Vuelve al elemento anterior.
context-help-key-close = Cierra esta interfaz.
context-help-key-older = Selecciona una entrada anterior del historial.
context-help-key-newer = Selecciona una entrada más reciente.
context-help-key-newer-stay = Selecciona una entrada más reciente sin cerrar el historial en la última.
context-help-key-accept = Acepta la selección actual.
context-help-key-filter = Escribe para filtrar los resultados visibles.
context-help-key-fill = Rellena el compositor con la sugerencia seleccionada.
context-help-key-edit-text = Escribe y edita el valor actual.
context-help-key-submit = Envía el valor actual.
context-help-key-cancel = Cancela sin aplicar los cambios.
context-help-key-save = Guarda el valor editado completo.
context-help-key-refresh = Actualiza los datos actuales.
context-help-key-provider-add-model = Añade un modelo al adaptador de proveedor seleccionado.
context-help-key-provider-save-adapter = Guarda el adaptador de proveedor seleccionado.
context-help-key-usage-period = Cambia el periodo del informe.
context-help-key-usage-view = Cambia la vista del panel.
context-help-key-usage-provider = Cambia el filtro de proveedor.
context-help-key-usage-model = Cambia el filtro de modelo.
context-help-key-usage-subagents = Incluye o excluye subagentes.
context-help-key-usage-sort = Cambia el orden.
context-help-key-model-catalog-search = Edita la búsqueda del catálogo de modelos.
context-help-key-model-catalog-page = Carga la página anterior o siguiente del catálogo.
context-help-key-plugin-transport = Cambia el filtro de transporte.
context-help-key-plugin-config-filter = Cambia el filtro de estado de configuración.
context-help-key-plugin-validate = Valida la configuración actual del complemento.
context-help-key-plugin-reset = Restablece la configuración del complemento.
context-help-key-plugin-diff = Muestra u oculta los cambios pendientes.
context-help-key-permission-add = Añade una regla en esta sección de permisos.
context-help-key-permission-rename = Cambia el nombre de la regla seleccionada.
context-help-key-permission-browse = Busca en el campo de ruta seleccionado.
context-help-key-activate = Activa la fila, control o acción seleccionados.
context-help-key-jump-message = Salta al mensaje relacionado con el evento.
context-help-key-back = Vuelve al nivel anterior.
context-help-key-next-question = Avanza o retrocede entre preguntas y la revisión.
context-help-key-custom-answer = Edita una respuesta personalizada.
context-help-key-clear = Borra la respuesta o anulación seleccionada.
context-help-key-cancel-request = Cierra o cancela toda la solicitud.
context-help-key-edit-answer = Vuelve a editar la respuesta seleccionada.
context-help-key-scroll-body = Desplaza el cuerpo independiente de revisión.
context-help-key-submit-all = Envía todas las respuestas revisadas.
context-help-key-confirm = Confirma la operación mostrada.
context-help-key-focus-next = Cambia el foco entre paneles y controles visibles.
context-help-key-pane-horizontal = Ve directamente al panel izquierdo o derecho.
context-help-key-horizontal = Muévete entre columnas, celdas o acciones visibles.
context-help-key-next-tab = Cambia entre pestañas de detalles.
context-help-key-scroll = Desplaza el contenido actual.
context-help-summary-session-work = Archivos, tareas en segundo plano y estado de reintentos de esta sesión.
context-help-key-work-toggle = Despliega o pliega el panel del espacio de trabajo.
context-help-key-work-tabs = Cambia entre archivos, tareas y estado.
context-help-key-work-staged = Cambia entre cambios preparados y de trabajo.
context-help-key-work-more = Carga más diferencias.
context-help-key-work-control = Ejecuta la acción disponible de la tarea seleccionada.
context-help-key-work-leave = Vuelve al compositor.
context-help-key-answer-next = Selecciona la respuesta y pasa a la página siguiente.
context-help-key-summary-action = Edita una respuesta, vuelve, envía o cancela la fila de resumen seleccionada.

session-view-all = todas
session-view-roots = raíces
session-view-subtree = subárbol
session-view-subtree-root = subárbol #{$id}
session-awaiting-approval = esperando aprobación
session-awaiting-plan-approval = esperando aprobación del plan
session-awaiting-user-input = esperando respuesta
session-awaiting-model = esperando modelo
session-idle = inactiva
session-tag-parent = principal
session-tag-child = secundaria
session-tag-current = actual
session-tag-ancestor = antecesora
session-tag-sibling = hermana
session-tag-leaf = terminal
session-summary-root = raíz #{$id}
session-summary-depth = profundidad {$depth}
session-summary-side-branches = {$count} ramas
session-summary-descendants = {$count} descendientes
session-summary-parent = principal #{$id}
session-summary-children = {$count} sesiones secundarias
session-status-plan = plan {$slug}
session-status-plan-with-steps = plan {$slug} · {$steps} pasos
session-status-thinking = {$value}
session-status-speed = {$value}
flash-session-view-mode = vista de sesiones: {$mode}
flash-session-title-empty = el título de la sesión no puede estar vacío
flash-session-renamed = sesión renombrada a {$title}
flash-session-favorited = {$title} añadida a favoritos
flash-session-unfavorited = {$title} eliminada de favoritos
flash-session-compacted = contexto de la sesión compactado
flash-session-rewound = creada una rama tras retirar {$target}; mensaje restaurado en el editor
flash-large-paste-no-file-view = los fragmentos grandes pegados se pueden quitar, pero no tienen vista de archivo
flash-quit-confirm = pulsa Ctrl+C otra vez para salir
flash-run-cancelling-quit = cancelando la ejecución activa; pulsa Ctrl+C otra vez para salir
flash-config-dir-prepare-failed = no se pudo preparar el directorio de configuración {$path}: {$error}
flash-config-file-create-failed = no se pudo crear el archivo de configuración {$path}: {$error}
flash-clipboard-paste-failed = error al pegar desde el portapapeles: {$error}
flash-clipboard-attachments-truncated = el portapapeles contiene más recursos de los que admite este mensaje; se han adjuntado los primeros {$count}
flash-attachment-count-limit = un mensaje puede adjuntar como máximo {$count} recursos
flash-attach-images-only = El selector solo permite adjuntar archivos de imagen.
flash-composer-drafts-load-failed = no se pudieron cargar los borradores del editor: {$error}
flash-composer-drafts-save-failed = no se pudieron guardar los borradores del editor: {$error}
flash-prompt-history-load-failed = no se pudo cargar el historial de prompts: {$error}
flash-prompt-history-save-failed = no se pudo guardar el historial de prompts: {$error}
flash-copied-mouse-selection = selección del ratón copiada al portapapeles
flash-transcript-line-copied = línea seleccionada de la transcripción copiada al portapapeles
flash-transcript-no-selection = selecciona contenido visible de la transcripción haciendo clic o usando una tecla de navegación
flash-transcript-exported = transcripción exportada a {$path}
flash-transcript-export-failed = error al exportar la transcripción: {$error}
flash-transcript-pager-failed = error al abrir la transcripción en el paginador: {$error}
flash-timeline-event-copied = evento seleccionado de la cronología copiado
flash-transcript-node-expanded = {$kind} expandido
flash-transcript-node-collapsed = {$kind} contraído
flash-transcript-parts-revealed = cargando hasta {$count} partes más
flash-transcript-node-copied = {$kind} actual copiado
flash-plan-viewer-requires-session = el visor del plan requiere una sesión abierta
flash-plan-viewer-no-plan = no hay ningún plan cargado para esta sesión
flash-snapshot-ready = instantánea lista: {$path} ({$branch})
flash-snapshot-attached = instantánea adjuntada: {$path} ({$branch})
flash-snapshot-exited = instantánea {$action}: {$path}
flash-commit-created = commit creado: {$sha} {$summary}
flash-pr-created = solicitud de incorporación creada: {$url}
flash-attachment-inspect-failed = no se pudo inspeccionar el adjunto {$path}: {$error}
flash-session-awaiting-approval = la sesión está esperando aprobación
flash-session-awaiting-plan-approval = la sesión está esperando aprobación del plan
flash-session-awaiting-user-input = la sesión está esperando una respuesta
flash-no-permission-request = no hay solicitudes de permiso pendientes
flash-no-user-input-request = no hay solicitudes de respuesta pendientes
flash-plugins-enabled = complementos activados
flash-plugins-disabled = complementos desactivados
flash-plugin-config-not-object = la configuración del complemento {$plugin_id} no es un objeto JSON y no se puede reescribir
flash-plugin-config-enabled = complemento {$plugin_id} activado; configuración conservada y runtime recargado
flash-plugin-config-disabled = complemento {$plugin_id} desactivado; configuración conservada y runtime recargado
flash-tool-description-mode-set = modo de descripción de herramientas establecido en {$mode}
flash-plugin-ui-display-mode-set = modo de visualización de la interfaz de complementos establecido en {$mode}
flash-model-selected = modelo seleccionado: {$model}
flash-model-cleared = anulación del modelo eliminada
flash-model-provider-required = se necesita un proveedor; selecciona uno primero o usa proveedor/modelo
flash-provider-studio-catalog-template-applied = campos del modelo rellenados desde {$model}; guarda para conservarlos
flash-thinking-mode-set = modo de razonamiento establecido en {$mode}
flash-thinking-mode-cleared = anulación del modo de razonamiento eliminada
flash-thinking-mode-required = se necesita seleccionar un modo de razonamiento
flash-speed-mode-set = modo de velocidad establecido en {$mode}
flash-speed-mode-cleared = anulación del modo de velocidad eliminada
flash-speed-mode-required = se necesita el nombre del modo de velocidad
flash-parent-session-missing = la sesión actual no tiene sesión principal
flash-command-usage = uso: {$usage}
flash-command-does-not-support-attachments = los comandos slash no admiten adjuntos preparados
flash-send-in-flight = el mensaje anterior todavía se está enviando; el nuevo mensaje permanece en el editor
flash-answer-pending-request = responde primero a la solicitud pendiente; el mensaje permanece en el editor
flash-message-queued = mensaje pendiente; se enviará cuando termine la ejecución actual
flash-message-replaced = ya hay un mensaje pendiente; se ha sustituido
flash-steer-sent = mensaje dirigido a la ejecución activa
flash-steer-failed-fallback-queue = no se pudo dirigir la ejecución activa; el mensaje se ha conservado
flash-run-cancelled = ejecución cancelada
flash-cancel-failed = error al cancelar
flash-user-command-empty = el contenido del comando de usuario está vacío
flash-side-requires-session = abre una sesión antes de iniciar una conversación secundaria
flash-side-already-open = ya hay una conversación secundaria abierta
flash-memory-forgotten = memoria eliminada: {$name}
flash-no-pending-message = no hay mensajes pendientes
flash-pending-cancelled = mensaje pendiente cancelado
flash-command-prefill-requires-empty-composer = vacía el editor antes de rellenarlo con un comando
flash-temperature-set = temperatura establecida en {$value}
flash-temperature-cleared = anulación de temperatura eliminada
flash-temperature-invalid = valor de temperatura no válido
flash-verbosity-set = nivel de detalle establecido en {$value}
flash-verbosity-cleared = anulación del nivel de detalle eliminada
flash-parallel-tool-calls-set = llamadas paralelas a herramientas establecidas en {$value}
flash-parallel-tool-calls-cleared = anulación de llamadas paralelas a herramientas eliminada
flash-max-output-set = máximo de tokens de salida establecido en {$value}
flash-max-output-cleared = anulación del máximo de salida eliminada
flash-max-output-invalid = valor máximo de tokens de salida no válido
flash-system-set = instrucción del sistema actualizada
flash-system-cleared = anulación de la instrucción del sistema eliminada
flash-runtime-status = estado de la sesión: {$summary}
flash-diagnostics-summary = diagnósticos: cwd={$cwd} session={$session} queue={$queue} runtime={$runtime}
flash-session-permission-updated = permisos de sesión actualizados.
session-work-tasks = Tareas
session-work-files = Archivos
session-work-requests = Solicitudes
session-work-waiting = Esperando herramientas
session-work-status = Estado de ejecución
session-work-workspace = Cambios registrados de la sesión · rutas literales de herramientas
session-work-ready = Listo
session-work-back = Atrás
session-work-switch = Cambiar →
session-work-staged = Preparado ⇄
session-work-working = Árbol de trabajo ⇄
session-work-stop = Detener
session-work-pause = Pausar
session-work-resume = Reanudar
session-work-delete = Eliminar
session-work-dismiss = Descartar
session-work-log-tail = Últimas 200 líneas / 128 KiB
session-work-empty = No hay contenido en esta vista
session-work-footer = Volver a la entrada
session-work-more-diff = Cargar más diferencias
session-work-status-pending = En cola
session-work-status-running = En ejecución
session-work-status-waiting = En espera
session-work-status-paused = En pausa
session-work-status-succeeded = Completada
session-work-status-failed = Fallida
session-work-status-cancelled = Cancelada
session-work-status-stopped = Detenida
session-work-next-wake = Próximo aviso:
session-work-title = Sesión
session-work-loading = Cargando archivos
session-work-previous = ‹ Anterior
session-work-next = Siguiente ›
session-work-recording-incomplete = El registro puede estar incompleto (las escrituras del shell son declaraciones)
session-work-operation-history = Historial de operaciones, no una diferencia neta de la sesión (sin línea base completa).
session-work-operation-diff = Diferencia completa de la operación de parche (puede incluir otros archivos).
session-work-recorded-truncated = Diferencia truncada; solo se puede cargar el contenido persistido.
session-work-no-recorded-diff = No hay diferencias textuales registradas.

overlay-snapshot-title = Instantánea
overlay-rename-title = Cambiar nombre de la sesión
overlay-rename-prompt = Edita el título de la sesión
overlay-choice-footer = Intro: seleccionar · Esc: cerrar
overlay-choice-footer-select = Intro: seleccionar · Esc: cerrar
overlay-help-footer = Flechas: desplazarse · Esc: cerrar
overlay-attach-image-title = Adjuntar imagen
overlay-attach-image-prompt = Escribe una ruta o término de búsqueda de imagen. Intro adjunta la imagen seleccionada.
overlay-attach-image-no-match = No se han encontrado imágenes
overlay-attach-browser-prompt = Explora la ruta actual. Escribe el nombre de un archivo o carpeta para filtrar, o edita la ruta para cambiar de directorio.
overlay-attach-image-browser-prompt = Explora las imágenes de la ruta actual. Escribe el nombre de una imagen o carpeta para filtrar, o edita la ruta para cambiar de directorio.
overlay-attach-browser-footer = ↓ seleccionar · → abrir carpeta · ← carpeta superior · Intro adjuntar · Esc cerrar
overlay-path-browser-list-title = Explorador
overlay-plan-approval-title = Aprobación del plan
overlay-plan-approval-summary = Revisa el plan activo antes de que continúe la implementación.
overlay-plan-approval-footer = a aprobar una vez | s aprobar sesión | d rechazar
overlay-plan-slug = plan={$slug}
overlay-plan-path = ruta: {$path}
overlay-plan-steps = pasos={$steps}
overlay-plan-preview-heading = Vista previa
overlay-permission-request-id = request_id: {$request_id}
overlay-permission-reason = motivo: {$reason}
overlay-permission-explanation = explicación: {$explanation}
overlay-permission-session = sesión: #{$session}
overlay-permission-requested-actions = Acciones solicitadas
overlay-permission-related-actions = Acciones relacionadas
overlay-permission-fact-source = origen={$value}
overlay-permission-fact-scope = ámbito={$value}
overlay-permission-fact-operator = operador={$value}
overlay-permission-footer = a permitir una vez | s permitir sesión | d denegar
overlay-permission-footer-edit-rule = {$footer} | e editar regla
overlay-permission-footer-action = ↑/↓ seleccionar · Intro activar · Esc cerrar
overlay-permission-footer-scope = ↑/↓ seleccionar · Intro activar · Esc volver
overlay-permission-footer-details = Esc volver
overlay-permission-decision-heading = Elige cómo gestionar esta solicitud
overlay-permission-scope-heading = Aplicar esta decisión a
overlay-permission-choice-allow = Permitir…
overlay-permission-choice-deny = Denegar…
overlay-permission-choice-edit-rule = Crear regla específica…
overlay-permission-choice-auto-approve-busy = Aprobando automáticamente…
overlay-permission-choice-once = Solo esta vez
overlay-permission-choice-session = Sesión actual
overlay-permission-choice-workspace = Espacio de trabajo actual
overlay-permission-choice-global = Global
overlay-permission-choice-allow-always-session = Permitir siempre (sesión)
overlay-permission-choice-allow-always-workspace = Permitir siempre (espacio de trabajo)
overlay-permission-choice-allow-always-global = Permitir siempre (global)
overlay-permission-choice-deny-always-session = Denegar siempre (sesión)
overlay-permission-choice-deny-always-workspace = Denegar siempre (espacio de trabajo)
overlay-permission-choice-deny-always-global = Denegar siempre (global)
overlay-permission-rule-fields = Campos de la regla
overlay-permission-rule-empty-detail = Selecciona un campo de permiso para editarlo.
overlay-permission-rule-item-browse-target-path = Explorar ruta de destino
overlay-permission-rule-item-browse-target-path-detail = Elige un archivo o carpeta del espacio de trabajo o del sistema de archivos.
overlay-permission-rule-item-browse-workspace-root = Explorar raíz del espacio de trabajo
overlay-permission-rule-item-browse-workspace-root-detail = Elige un directorio para workspace_root.
overlay-permission-rule-item-browser-value = abrir explorador
overlay-permission-rule-item-save = Guardar regla
overlay-permission-rule-item-save-detail = Validar y guardar el borrador actual.
overlay-permission-rule-item-revoke = Revocar regla
overlay-permission-rule-item-revoke-detail = Revocar la regla guardada existente.
overlay-permission-rule-detail-browse-workspace-root = Abre el explorador de directorios y elige una carpeta para workspace_root.
overlay-permission-rule-detail-browse-target-path = Abre el explorador del sistema de archivos y elige un archivo o carpeta para rellenar target_path.
overlay-permission-rule-detail-revoke = Revocar desactiva la regla guardada sin borrar su historial.
overlay-permission-rule-preview-heading = Vista previa
overlay-permission-rule-preview-label = etiqueta: {$label}
overlay-permission-rule-preview-mode = modo: {$mode}
overlay-permission-rule-preview-scope = ámbito: {$scope}
overlay-permission-rule-preview-subject-tool = sujeto: herramienta ({$tool})
overlay-permission-rule-preview-subject-path = sujeto: acceso a ruta ({$access})
overlay-permission-rule-preview-subject-network = sujeto: acceso a red
overlay-permission-rule-preview-qualifier = calificador: {$qualifier}
overlay-permission-rule-preview-target = destino: {$target}
overlay-permission-rule-preview-workspace-root = workspace_root: {$path}
overlay-permission-rule-preview-session-id = session_id: {$session}
overlay-permission-rule-preview-invalid = regla no válida: {$error}
overlay-workbench-overview = Resumen
overlay-permission-studio-footer = ←/→ o Tab/Mayús+Tab cambiar panel · Flechas seleccionar · Intro editar · Esc volver
overlay-permission-studio-sections = Secciones de permisos
overlay-permission-studio-nav = Navegación
overlay-detail-current-value = Valor actual
overlay-choice-current-unavailable-detail = Valor actual; no está entre las opciones disponibles
overlay-choice-clear-runtime-detail = Quitar la anulación de sesión actual de {$field}.
overlay-choice-clear-provider-detail = Establecer {$field} como valor vacío del borrador.
overlay-choice-clear-permission-subject = Restablecer el tipo de sujeto a la regla de herramienta predeterminada.
overlay-choice-clear-permission-access-kind = Borrar la anulación del tipo de acceso a la ruta.
overlay-choice-clear-permission-scope = Restablecer el ámbito de la regla al espacio de trabajo.
overlay-choice-clear-permission-mode = Restablecer el modo de la regla a preguntar.
overlay-choice-clear-permission-override-detail = Quitar la anulación actual de {$field}.
overlay-client-versions-title = Versiones de cliente del proveedor
overlay-insert-content-title = Insertar contenido
overlay-insert-content-prompt = Elige el contenido que quieres insertar en la posición del cursor del editor
overlay-resume-title = Reanudar sesión
overlay-resume-prompt = Escribe para filtrar sesiones por título, ID o linaje
overlay-resume-empty = No se han encontrado sesiones
overlay-session-search-scope-all = todas
overlay-session-search-scope-roots = raíces
overlay-session-search-scope-subtree = subárbol
overlay-session-search-tail-more = más
overlay-session-search-tail-end = fin
overlay-session-search-footer-local = ámbito {$scope} · {$total} resultados
overlay-session-search-footer-remote = ámbito {$scope} · {$loaded} cargados · {$tail}
overlay-lineage-empty = No se han encontrado sesiones de ramas relacionadas
overlay-rewind-empty = No hay mensajes de usuario completados que se puedan retirar
overlay-rewind-confirm-title = Confirmar retroceso
overlay-rewind-confirm-keep = Retirar {$target} y todos los mensajes posteriores
overlay-rewind-confirm-warning = Una nueva rama conservará solo el historial anterior a este mensaje de usuario.
overlay-rewind-confirm-draft = El mensaje de usuario retirado se restaurará en el editor de la nueva rama; el borrador actual permanecerá en la sesión original.
overlay-confirm-footer = Y confirmar
overlay-snapshot-remove-title = Confirmar eliminación de instantánea
overlay-snapshot-remove-body = ¿Eliminar la instantánea activa y salir de la sesión actual?
overlay-snapshot-remove-force = La eliminación forzada descartará los cambios sin confirmar de la instantánea.
overlay-model-providers-title = Proveedores de modelos
overlay-model-providers-prompt = Elige un proveedor antes de seleccionar un modelo
overlay-models-title = Modelos [{$provider}]
overlay-models-prompt = Elige un modelo para el proveedor {$provider}
overlay-session-model-title = Modelo de la sesión
overlay-session-model-prompt = Buscar modelos
overlay-session-model-list-title = Modelos
overlay-session-model-footer = Escribe para filtrar
overlay-model-catalog-title = Catálogo de modelos
overlay-model-catalog-footer = Ctrl+F buscar · Ctrl+R actualizar · ←/→ página anterior/siguiente · ↑/↓ seleccionar · Esc cerrar
overlay-model-catalog-search-title = Búsqueda en el catálogo de modelos
overlay-model-catalog-search-prompt = Filtra el catálogo por ID, nombre visible, origen o descripción
overlay-model-catalog-entries = Modelos
overlay-model-catalog-detail = Detalles
overlay-model-catalog-query-all = <todos>
overlay-model-catalog-summary = consulta {$query} · página {$start}-{$end} / {$total} · {$count} modelos
overlay-permission-rule-overview-draft = Borrador
overlay-permission-rule-overview-mode = Modo
overlay-permission-rule-overview-scope = Ámbito
overlay-permission-rule-overview-session = Sesión
overlay-permission-rule-overview-workspace-root = Raíz del espacio de trabajo
overlay-children-title = Sesiones secundarias [#{$session}]
overlay-children-prompt = Elige una sesión secundaria para abrir
overlay-children-empty = No se han encontrado sesiones secundarias
overlay-timeline-title = Cronología [#{$session}]
overlay-timeline-prompt = Filtra por tipo de evento, comando o metadatos del mensaje
overlay-timeline-empty = No se han encontrado eventos
overlay-timeline-footer = Escribe para filtrar · Flechas seleccionar · Intro abrir mensaje relacionado · Esc cerrar
overlay-timeline-events = Eventos
overlay-timeline-detail = Detalles

timeline-summary-permission-requested = permiso solicitado [{$risk}]: {$reason}
timeline-summary-permission-replied = respuesta al permiso: {$kind}
timeline-summary-permission-rule-created = regla de permisos n.º {$id} creada
timeline-summary-permission-rule-updated = regla de permisos n.º {$id} actualizada
timeline-summary-permission-rule-revoked = regla de permisos n.º {$id} revocada
timeline-detail-permission-session-id = id_de_sesión: {$id}
timeline-detail-permission-request-id = id_de_solicitud: {$id}
timeline-detail-permission-reason = motivo: {$reason}
timeline-detail-permission-risk = riesgo: {$risk}
timeline-detail-permission-explanation = explicación: {$explanation}
timeline-detail-permission-source = origen: {$source}
timeline-detail-permission-scope = ámbito: {$scope}
timeline-detail-permission-operator = operador: {$operator}
timeline-detail-permission-reply-kind = tipo: {$kind}
timeline-detail-permission-rule-id = id_de_regla: {$id}
timeline-detail-permission-rule-action-key = clave_de_acción: {$action}
timeline-detail-permission-rule-mode = modo: {$mode}
timeline-detail-permission-rule-scope = ámbito: {$scope}
timeline-detail-permission-rule-source = origen: {$source}
timeline-type-execution-started = ejecución iniciada
timeline-type-execution-completed = ejecución completada
timeline-type-execution-cancelled = ejecución cancelada
timeline-type-execution-failed = error en la ejecución
timeline-type-compaction-completed = compactación del contexto completada
timeline-type-subtask-status-changed = estado de la subtarea cambiado
timeline-type-stream-error = error del flujo
timeline-type-message-part-checkpointed = parte del mensaje guardada
timeline-type-message-part-delta = cambio en la parte del mensaje
timeline-type-command-begin = inicio del comando
timeline-type-command-output-delta = cambio en la salida del comando
timeline-type-command-end = fin del comando
timeline-type-permission-requested = permiso solicitado
timeline-type-permission-replied = respuesta al permiso
timeline-type-permission-rule-created = regla de permisos creada
timeline-type-permission-rule-updated = regla de permisos actualizada
timeline-type-permission-rule-revoked = regla de permisos revocada
timeline-type-session-goal-updated = objetivo de la sesión actualizado
timeline-type-run-started = ejecución iniciada
timeline-type-run-completed = ejecución completada
timeline-type-run-aborted = ejecución interrumpida
timeline-type-user-message-appended = mensaje del usuario añadido
timeline-type-assistant-message-completed = respuesta del asistente completada
timeline-type-tool-call-issued = llamada a herramienta iniciada
timeline-type-tool-call-completed = llamada a herramienta completada
timeline-type-plugin-event = evento del complemento
timeline-summary-execution-started = sesión n.º {$id}
timeline-summary-message-part-checkpointed = mensaje n.º {$message_id}, parte n.º {$part_id} guardada ({$kind})
timeline-summary-message-part-delta = mensaje n.º {$message_id}, parte n.º {$part_id}, {$field} (+{$count} caracteres)
timeline-summary-command-output-bytes = {$count} bytes
timeline-summary-command-output-delta = {$stream} {$preview}
timeline-summary-command-end = {$status}, salida={$exit_code} ({$duration_ms} ms)
timeline-summary-session-goal-updated = objetivo actualizado: {$status} · {$objective}
timeline-summary-run-started = ejecución {$id}
timeline-summary-run-completed = ejecución {$id} ({$finish})
timeline-summary-run-aborted = ejecución {$id} interrumpida ({$reason})
timeline-summary-user-message-appended = usuario n.º {$id}
timeline-summary-assistant-message-completed = asistente n.º {$id} ({$finish})
timeline-summary-tool-call-issued = herramienta {$name}, llamada={$call_id}
timeline-summary-tool-call-completed = llamada a herramienta {$call_id} completada
timeline-summary-system-notice-appended = sistema n.º {$message_id}: {$kind}
timeline-summary-plugin-event = complemento {$plugin_id}/{$kind_label}
timeline-label-seq = secuencia
timeline-label-created = creado
timeline-label-type = tipo
timeline-label-event-id = id del evento
timeline-label-causation-id = id de la causa
timeline-label-correlation-id = id de correlación
timeline-label-session-id = id de sesión
timeline-label-error-code = código de error
timeline-label-outcome = resultado
timeline-label-error-message = mensaje de error
timeline-label-message-id = id del mensaje
timeline-label-part-id = id de la parte
timeline-label-part-kind = tipo de parte
timeline-label-status = estado
timeline-label-summary = resumen
timeline-label-field = campo
timeline-label-delta = cambio
timeline-label-call-id = id de llamada
timeline-label-command = comando
timeline-label-cwd = directorio de trabajo
timeline-label-stream = flujo
timeline-label-bytes = bytes
timeline-label-preview = vista previa
timeline-label-exit-code = código de salida
timeline-label-duration-ms = duración (ms)
timeline-label-request-id = id de solicitud
timeline-label-reason = motivo
timeline-label-risk = riesgo
timeline-label-explanation = explicación
timeline-label-source = origen
timeline-label-scope = ámbito
timeline-label-operator = operador
timeline-label-requested-actions = acciones solicitadas
timeline-label-related-actions = acciones relacionadas
timeline-label-reply-kind = tipo de respuesta
timeline-label-rule-id = id de regla
timeline-label-action-key = clave de acción
timeline-label-mode = modo
timeline-label-goal-id = id del objetivo
timeline-label-objective = objetivo
timeline-label-completed-at = completado el
timeline-label-run-id = id de ejecución
timeline-label-model = modelo
timeline-label-finish = finalización
timeline-label-message = mensaje
timeline-label-name = nombre
timeline-label-kind = tipo
timeline-label-text = texto
timeline-label-plugin-id = id del complemento
timeline-label-kind-label = etiqueta del tipo
timeline-label-payload = contenido
timeline-label-trace = traza

workbench-editor-title-add-field = Añadir campo
workbench-editor-title-edit = Editar { $title }
workbench-editor-title-edit-with-label = Editar { $title } · { $label }
workbench-editor-title-rename = Cambiar el nombre de { $title }
workbench-editor-placeholder-type-to-edit = Escribe para editar
workbench-editor-prompt-add-field = Introduce un nombre de campo para { $path }. Si el esquema admite varios tipos o estructuras de valores, el editor te preguntará después de crearlo.
workbench-editor-prompt-new-field-name = Introduce el nombre del campo nuevo.
workbench-select-branch-title = Seleccionar rama
workbench-select-branch-prompt = Elige la estructura del esquema
workbench-select-type-title = Seleccionar tipo
workbench-select-type-prompt = Elige el tipo JSON para { $path }
workbench-picker-title-select = Seleccionar { $title }
workbench-picker-no-choices = No hay opciones disponibles
workbench-picker-no-actions = No hay acciones disponibles
workbench-picker-choose-one = Elige un valor
workbench-picker-choose-many = Elige uno o varios valores
workbench-picker-space-toggle = Espacio: alternar
workbench-action-more = Más acciones
workbench-action-field = Acciones del campo
workbench-action-add-item = Añadir elemento
workbench-action-add-item-desc = Añade al final un elemento nuevo con el valor predeterminado de { $title }.
workbench-action-add-field = Añadir campo
workbench-action-add-field-desc = Añade un campo nuevo dentro de { $title }.
workbench-action-insert-before = Insertar antes
workbench-action-insert-before-desc = Inserta un elemento nuevo con el valor predeterminado antes de este elemento del array.
workbench-action-insert-after = Insertar después
workbench-action-insert-after-desc = Inserta un elemento nuevo con el valor predeterminado después de este elemento del array.
workbench-action-duplicate-item = Duplicar elemento
workbench-action-duplicate-item-desc = Duplica { $title } dentro de este array.
workbench-action-move-up = Mover arriba
workbench-action-move-up-desc = Sube una posición este elemento del array.
workbench-action-move-down = Mover abajo
workbench-action-move-down-desc = Baja una posición este elemento del array.
workbench-action-remove-item = Quitar elemento
workbench-action-remove-item-desc = Quita { $title } de este array.
workbench-action-rename-field = Cambiar nombre del campo
workbench-action-rename-field-desc = Cambia el nombre de la clave de { $title }.
workbench-action-reset-field = Restablecer campo
workbench-action-reset-field-desc = Restaura { $title } al valor predeterminado del complemento.
workbench-action-reset-group = Restablecer grupo
workbench-action-reset-group-desc = Restaura todos los campos de { $title } a los valores predeterminados del complemento.
workbench-action-switch-shape-desc = Cambia la estructura activa de { $title }.
workbench-action-switch-type-desc = Cambia el tipo activo de { $title }.
workbench-warning-add-requires-object-or-array = añadir está disponible para nodos de tipo objeto y array
workbench-warning-cannot-add-item = no se puede añadir otro elemento en esta posición del array
workbench-warning-cannot-duplicate-item = no se puede duplicar este elemento del array
workbench-warning-cannot-insert-item = no se puede insertar un elemento en esta posición del array
workbench-warning-cannot-remove-item = no se puede quitar este elemento del array
workbench-warning-root-config-not-deletable = la configuración raíz no se puede eliminar
workbench-warning-row-not-object-field = la fila seleccionada no corresponde a un campo de objeto
workbench-warning-no-tools = este complemento no ofrece herramientas
workbench-error-append-array-item = no se pudo añadir el elemento al array
workbench-error-config-update = No se ha podido actualizar la configuración del complemento.
workbench-error-tool-input-prepare = No se han podido preparar los datos de entrada de la herramienta del complemento. Revisa el registro de diagnóstico e inténtalo de nuevo.
workbench-error-args-invalid-json = Los argumentos de la herramienta no son JSON válido.
workbench-error-args-not-object = los argumentos de la herramienta del complemento deben ser un objeto JSON
workbench-error-tool-unavailable = La herramienta del complemento ya no está disponible.
workbench-error-args-schema-mismatch = Los argumentos de la herramienta no se ajustan al esquema del complemento.
workbench-error-tool-requires-session = La herramienta del complemento requiere una sesión activa.
workbench-tool-default-run = Ejecutar esta herramienta del complemento.
workbench-tool-tags-none = no se han declarado etiquetas
workbench-tool-editor-title = Ejecutar { $plugin } / { $tool }
workbench-tool-editor-description = { $summary } Etiquetas: { $tags }. Edita debajo los argumentos JSON. Al enviar, se autoriza una sola vez esta llamada exacta a la herramienta; las reglas de denegación guardadas siguen aplicándose.
workbench-tool-editor-placeholder = Ctrl+S validar y ejecutar · Esc cancelar · Intro nueva línea
workbench-tool-completed = herramienta del complemento completada
workbench-tool-completed-no-output = La herramienta se ha completado correctamente sin producir resultados.

value-reset = restablecer
value-selected = seleccionado
value-inactive = inactivo
value-unrestricted = sin restricciones
value-inherit = heredar
value-inherits-runtime-defaults = hereda los valores predeterminados del runtime
value-default-true = predeterminado (verdadero)
value-tools = herramientas
value-plugin-tools = herramientas del complemento
value-inherit-off = herencia desactivada: {$sections}
value-path-summary = ruta: {$detail}
value-network-summary = red: {$detail}
value-plugin-override-count = {$count} anulaciones de complementos
value-tools-summary = herramientas: {$detail}
value-char-count = {$count} caracteres
value-item-count = {$count} elementos
value-on = activado
value-off = desactivado
value-enabled = habilitado
value-disabled = deshabilitado
value-unknown = desconocido
value-hidden = oculto
value-visible = visible
value-config-owned = gestionado por la configuración
value-file-backed = basado en archivo
value-editable = editable
value-read-only = de solo lectura
value-built-in = integrado
value-built-in-defaults = valores predeterminados integrados
value-config-backed = basado en la configuración
value-markdown-backed = basado en Markdown
value-runtime-registered = registrado en runtime
value-edit = Editar
value-rename = Cambiar nombre
value-duplicate = Duplicar
value-delete = Eliminar
value-runtime-config = configuración de runtime
value-runtime-config-file = configuración de runtime (agena.json)

command-side-summary = Abre una conversación secundaria bifurcada sin interrumpir la ejecución actual
command-commit-summary = Crea un commit con los cambios preparados
command-pr-summary = Crea una solicitud de incorporación de cambios con opciones mínimas basadas en gh
command-help-summary = Muestra la ayuda de la interfaz actual
command-commands-summary = Abre la paleta de comandos
command-new-summary = Crea una sesión nueva
command-sessions-summary = Abre el selector de sesiones
command-lineage-summary = Abre un selector del historial de ramas de la sesión actual
command-rewind-summary = Retira un mensaje del usuario y todos los mensajes posteriores en una rama nueva
command-rename-summary = Cambia el nombre de la sesión actual o seleccionada
command-favorite-summary = Añade o quita de favoritos la sesión actual o seleccionada
command-timeline-summary = Abre una cronología de eventos con búsqueda para la sesión actual o seleccionada
command-export-summary = Exporta la transcripción cargada como Markdown y la abre en el editor
command-pager-summary = Abre la transcripción cargada en un paginador de terminal
command-continue-summary = Continúa la sesión actual bloqueada o pendiente
command-compact-summary = Resume el contexto de la sesión actual
command-user-input-summary = Responde a la primera solicitud de respuesta pendiente
command-allow-summary = Permite una vez la primera solicitud de permiso pendiente
command-allow-always-summary = Permite siempre la primera solicitud de permiso pendiente
command-deny-summary = Deniega una vez la primera solicitud de permiso pendiente
command-deny-always-summary = Deniega siempre la primera solicitud de permiso pendiente
command-attach-summary = Adjunta un archivo mediante el selector interactivo
command-download-summary = Descarga un archivo del espacio de trabajo mediante el proveedor activo de transferencia por terminal
command-editor-summary = Abre el editor externo para el compositor
command-image-summary = Adjunta una imagen mediante el selector interactivo
command-paste-summary = Pega archivos, imágenes o texto del portapapeles en el compositor
command-copy-summary = Copia la transcripción cargada
command-copy-message-summary = Copia el último mensaje del asistente
command-copy-visible-summary = Copia la parte visible de la transcripción
command-model-summary = Abre el selector de modelo de sesión para elegir proveedor/adaptador/modelo
command-fork-summary = Abre una bifurcación de la sesión actual con todo su historial
command-children-summary = Abre el selector de sesiones secundarias
command-parent-summary = Salta a la sesión principal
command-diagnostics-summary = Muestra un resumen de diagnóstico anonimizado para enviar comentarios
command-status-summary = Muestra el estado actual de la sesión, la ejecución, el espacio de trabajo y la interfaz
command-usage-summary = Abre las analíticas interactivas de tokens y costes
command-activities-summary = Abre el panel de actividades en segundo plano
command-plan-summary = Abre el visor del plan de la sesión actual
command-btw-summary = Pregunta por separado mientras continúa la conversación principal. Solo herramientas de lectura; no modifica la conversación principal.

provider-adapter-rule-api-anthropic-detail = Adaptador HTTP de Anthropic; usa base_url de autenticación del proveedor y permite listar modelos en borrador.
provider-adapter-rule-api-gemini-detail = Adaptador HTTP de Gemini; usa base_url de autenticación del proveedor y permite listar modelos en borrador.
provider-adapter-rule-api-openai-detail = Adaptador explícito de OpenAI Responses o Chat Completions; usa base_url de autenticación del proveedor y permite listar modelos en borrador.
provider-adapter-rule-bedrock-sigv4-amazon-bedrock-detail = Adaptador de Amazon Bedrock firmado con AWS SigV4.
provider-adapter-rule-cline-api-openai-detail = API de Cline mediante el adaptador openai_chat_completions; base_url y la detección de modelos se mantienen fijos en el endpoint alojado de Cline.
provider-adapter-rule-github-copilot-anthropic-detail = Token de Copilot mediante el adaptador anthropic.
provider-adapter-rule-github-copilot-openai-detail = Token de Copilot mediante el adaptador OpenAI Responses o Chat Completions seleccionado.
provider-adapter-rule-gitlab-auth-anthropic-detail = Pasarela de GitLab mediante el adaptador anthropic.
provider-adapter-rule-gitlab-auth-openai-detail = Pasarela de GitLab mediante el adaptador OpenAI Responses o Chat Completions seleccionado.
provider-adapter-rule-gitlab-credential-anthropic-detail = Credencial OAuth de GitLab mediante el adaptador anthropic.
provider-adapter-rule-gitlab-credential-openai-detail = Credencial OAuth de GitLab mediante el adaptador OpenAI Responses o Chat Completions seleccionado.
provider-adapter-rule-google-adc-openai-detail = Adaptador openai_chat_completions de estilo Vertex con capability_family=gemini; requiere base_url de autenticación del proveedor.
provider-adapter-rule-none-ollama-detail = Adaptador de Ollama; los campos de autenticación permanecen vacíos y el endpoint se configura en el adaptador.
provider-adapter-rule-openai-chatgpt-openai-detail = Adaptador openai_responses con el backend oficial ChatGPT Codex; las credenciales proceden de la sesión local de OpenAI.
provider-adapter-rule-sap-ai-core-openai-detail = Adaptador openai_chat_completions de SAP AI Core; requiere base_url de autenticación del proveedor y service_key_env.
provider-api-key-env-anthropic-detail = Variable de entorno de la clave API de Anthropic
provider-api-key-env-gemini-detail = Variable de entorno de la clave API de Gemini
provider-api-key-env-gitlab-detail = Variable de entorno del token de GitLab
provider-api-key-env-openai-detail = Variable de entorno de la clave API compatible con OpenAI
provider-api-key-env-opencode-detail = Variable de entorno de la clave API de Opencode
provider-api-key-env-shared-gateway-detail = Variable de entorno de la clave API de la pasarela compartida
provider-api-key-env-vertex-detail = Variable de entorno del token de acceso de Google Vertex
provider-field-api-key = Clave API
provider-field-api-key-env = Variable de entorno de la clave API
provider-field-auth-status = Estado de autenticación
provider-field-avatar-url = URL del avatar
provider-field-display-name = Nombre visible
provider-field-email = Correo electrónico
provider-field-username = Nombre de usuario
provider-issuer-atomgit-detail = Credenciales de AtomGit
provider-issuer-atomgit-label = atomgit
provider-model-catalog-selection-note = Pulsa Intro para elegir una plantilla. Al guardar se escriben sus campos, no el ID del catálogo.
provider-model-field-catalog-template = Plantilla del catálogo
provider-profile-choice-detail = Perfil de la configuración compartida de AWS
provider-profile-default-detail = Perfil predeterminado de AWS
provider-redirect-local-studio-detail = URL de retorno de localhost que coincide con la ruta web de Studio

message-input-activity-attachment = Adjunto
message-input-activity-skill = Comando
message-activity-response-running = Generando respuesta
message-activity-response-completed = Respuesta completada
message-activity-response-failed = Error en la respuesta
message-activity-response-cancelled = Respuesta cancelada
message-activity-continue-running = Continuando la respuesta
message-activity-continue-completed = Continuación completada
message-activity-continue-failed = Error en la continuación
message-activity-continue-cancelled = Continuación cancelada
message-activity-compact-running = Compactando el contexto
message-activity-compact-completed = Contexto compactado
message-activity-compact-failed = Error al compactar el contexto
message-activity-compact-cancelled = Compactación del contexto cancelada
message-activity-permission-running = Aplicando la respuesta al permiso
message-activity-permission-completed = Respuesta al permiso aplicada
message-activity-permission-failed = Error al aplicar la respuesta al permiso
message-activity-permission-cancelled = Aplicación de la respuesta al permiso cancelada
message-activity-input-running = Enviando la respuesta del usuario
message-activity-input-completed = Respuesta del usuario enviada
message-activity-input-failed = Error al enviar la respuesta del usuario
message-activity-input-cancelled = Envío de la respuesta del usuario cancelado
message-compaction-unchanged = El contexto ya es compacto. Se conservó el historial existente.
message-compaction-title-manual = Contexto compactado
message-compaction-title-auto = Contexto compactado automáticamente
message-compaction-title-reactive = Contexto compactado tras superar el límite
message-compaction-token-summary = {$before} → {$after} tokens · reducción del {$percent}%
message-compaction-strategy-local = resumen local
message-compaction-strategy-native = punto de control nativo del proveedor
message-compaction-strategy-line = Estrategia: {$strategy}
message-compaction-generation-line = Generación {$generation}
message-permission = permiso
message-tool-summary = [herramienta] {$status} | {$label}
message-tool-output-collapsed = … {$lines} más
message-activity-run-collapsed = Intro: cargar 5 · n Intro: cargar siguiente · Ctrl+Mayús+Intro: cargar todo · … {$count} partes anteriores ocultas

btw-title = BTW · pregunta de solo lectura
btw-description = Pregunta por separado mientras continúa la conversación principal. Herramientas de solo lectura; no se modifica la conversación principal.
btw-input = Pregunta BTW · Intro enviar · Mayús+Intro nueva línea
btw-footer = Intro preguntar · RePág/AvPág desplazarse · Ctrl+C detener · Tab/Esc volver al chat
btw-send = Preguntar
btw-copy = Copiar
btw-copied = Respuesta BTW copiada
btw-clear = Borrar
btw-leave = Volver al chat
btw-stop = Detener
btw-stopped = Detenido · respuesta parcial conservada
btw-loading = Respondiendo…
composer-missing-staged-item = falta el elemento preparado para {$placeholder}
composer-placeholder-out-of-sync = el estado del marcador del editor no está sincronizado
composer-placeholder-range-invalid = el rango del marcador del editor no es válido
composer-prompt-history-label = historial>
composer-prompt-history-no-matches = no hay coincidencias en el historial de prompts
composer-status-hidden-pending-dialog = diálogo pendiente oculto mientras escribes
composer-status-history = historial {$current}/{$total}
composer-status-history-query = historial {$current}/{$total} consulta={$query}
composer-status-mention = mención {$query}
composer-status-pending-approval = {$count} aprobaciones pendientes (Ctrl+L)
composer-status-pending-user-input = {$count} respuestas pendientes (Ctrl+R)
composer-status-selected-item = elemento {$current}/{$total} {$label}
composer-status-slash = comando / {$query}
failure-detail-section-title = Detalles del error
file-change-moved = movido
help-composer-line-11 = Ctrl+G abre elementos integrados cuando están disponibles; comandos como /model o /rewind se ejecutan desde el editor
help-section-user-commands = Comandos de usuario
help-sessions-line-4 = /rename, /timeline y /lineage gestionan la sesión seleccionada
help-sessions-line-5 = Usa los controles del selector para alternar entre los ámbitos All, Roots y Subtree
help-transcript-line-8 = Usa /rename, /timeline, /lineage, /settings, /export o /pager para las acciones y la configuración de la sesión
hub-action-directory = Plegar carpeta
hub-action-favorite = Añadir a favoritos
hub-action-pin = Fijar/soltar
hub-action-unfavorite = Quitar de favoritos
hub-directory-more = + Mostrar más sesiones ({ $count } restantes)
hub-page-next = → Página siguiente ({ $page })
hub-page-previous = ← Página anterior ({ $page })
hub-section-directories = Carpetas
hub-section-favorites = Favoritas
hub-section-pinned = Fijadas
hub-section-search = Resultados de búsqueda
inline-fact-adapter = adaptador
inline-fact-model = modelo
inline-fact-parallel-tools = herramientas_en_paralelo
inline-fact-provider = proveedor
inline-fact-speed = velocidad
inline-fact-thinking = razonamiento
inline-fact-verbosity = nivel de detalle
insert-content-file-detail = Adjunta una ruta de archivo o carpeta del espacio de trabajo.
insert-content-file-label = Archivo
operation-command-exit-code = salida {$code}
operation-diff-summary = diff: {$count} líneas
operation-diff-summary-detailed = diff: {$files} archivos, +{$added} -{$deleted}
operation-diff-summary-detailed-one = diff: 1 archivo, +{$added} -{$deleted}
operation-diff-summary-detailed-one-rename = diff: 1 archivo, +{$added} -{$deleted}, {$renamed} renombrado(s)
operation-diff-summary-detailed-renames = diff: {$files} archivos, +{$added} -{$deleted}, {$renamed} renombrado(s)
operation-nested-task-summary = {$title} ({$status})
operation-search-heading = búsqueda: {$query}
operation-search-results-heading = resultados de búsqueda
permission-label-auto-approve = aprobación automática
permission-rule-error-expected-structured = se esperaba una regla estructurada que empiece por tool/path y termine en allow|ask|deny
permission-rule-error-invalid-mode = el modo de permiso debe ser allow, ask o deny
permission-rule-error-invalid-option-format = se esperaba una opción clave=valor; se recibió `{$token}`
permission-rule-error-invalid-scope = el ámbito debe ser session, workspace o global
permission-rule-error-invalid-shell-args = argumentos con formato de shell no válidos
permission-rule-error-invalid-subject = el sujeto de la regla debe empezar por `tool`, `path` o `network`
permission-rule-error-session-token-required = el ámbito de sesión requiere session=<id>
permission-rule-error-unknown-option = opción de regla de permisos desconocida: {$key}
plan-inline-footer = Tab/Esc entrada · j/k desplazarse · r actualizar · f expandir
plan-viewer-autorun-off = ejecución automática: desactivada
plan-viewer-autorun-on = ejecución automática: activada
plan-viewer-empty = Aún no hay ningún plan.
plan-viewer-footer = ↑/↓/RePág/AvPág desplazarse · r actualizar · a ejecución automática · q cerrar
plan-viewer-loading = Cargando el plan
plan-viewer-refresh = Actualizar
plan-viewer-title = Plan
run-options-summary-parallel-tools = herramientas-en-paralelo {$value}
run-options-summary-speed = velocidad {$value}
run-options-summary-thinking = razonamiento {$value}
run-options-summary-verbosity = nivel de detalle {$value}
runtime-setting-apply-cleared = {$field} eliminado
runtime-setting-apply-updated = {$field} actualizado
runtime-setting-choice-parallel-detail = aplicar una anulación de llamadas paralelas a herramientas
runtime-setting-error-bool = {$field} debe ser true/false, on/off, yes/no o 1/0
runtime-setting-error-finite = {$field} debe ser un número finito
runtime-setting-error-number = {$field} debe ser un valor numérico
runtime-setting-error-positive-int = {$field} debe ser un entero positivo
runtime-setting-summary-override-value = anulación {$value}
runtime-status-default = valores predeterminados de la sesión
runtime-status-display = visualización {$value}
runtime-status-display-default = predeterminada
runtime-status-display-plugin = complemento
runtime-status-keys = teclas q={$queue} enviar={$send}
runtime-status-theme = tema {$value}
runtime-status-tui-blocks = bloques_tui {$count}
side-return = Secundaria · volver a la principal
snapshot-action-exit = salida
snapshot-action-keep = conservada
snapshot-action-remove = eliminada
status-part-agent = agente={$value}
status-part-cwd = directorio={$value}
status-part-model = modelo={$value}
status-part-parallel-tools = herramientas_en_paralelo={$value}
status-part-permission = permiso={$value}
status-part-permissions = permisos={$count}
status-part-speed = velocidad={$value}
status-part-state = estado={$value}
status-part-task = tarea={$value}
status-part-thinking = razonamiento={$value}
status-part-tools = herramientas={$count}
status-part-user-input = respuestas={$count}
status-part-verbosity = nivel_de_detalle={$value}
status-part-workspace = espacio={$value}
surface-mode-insert = INSERTAR
surface-mode-navigate = NAVEGAR
surface-mode-select = SELECCIONAR
terminal-notification-interrupted = interrumpido
terminal-notification-permission = esperando aprobación
terminal-notification-user-input = esperando respuesta
terminal-notification-working = trabajando
terminal-title-interrupted = interrumpido
terminal-title-permission = esperando aprobación
terminal-title-user-input = esperando respuesta
terminal-title-working = trabajando
tool-detail-input = Entrada
tool-detail-metadata = Metadatos
tool-detail-output = Salida
tool-presentation-data = Datos de presentación
tool-technical-details = Detalles técnicos

transcript-rich-fallback = Vista previa no disponible: {$reason}. Se muestra el código original.
transcript-interactive-app = Abre esta aplicación en Agena Web para usar la interfaz y ejecutar sus servicios.
