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
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = valor personalizado permitido
overlay-user-input-reply-format = Formato de respuesta: 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X cancela la solicitud
overlay-user-input-footer = Ctrl+X cancelar

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
attachment-kind-directory = carpeta
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

overlay-choice-clear-settings-detail = Remove the file override for {$field}.

overlay-settings-section-plugins-label = Plugins y herramientas

overlay-settings-section-plugins-summary = Configuración de enchufe, herramientas, arneses y diagnósticos

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

settings-field-permission-approval-model-label = Modelo de aprobación automática

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

settings-activity-kind-skill_reference-label = Referencias de Habilidad

settings-activity-kind-skill_reference-description = Referencias a las habilidades utilizadas en la respuesta.

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

settings-source-file-missing = {$path} (will be created)

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

settings-harness-browser-label = Arnés del navegador

settings-harness-shell-label = Shell Harness

settings-harness-editor-label = Editor Harness

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

settings-mcp-client-registration-label = Registro de cliente OAuth

settings-mcp-client-registration-cimd = CIMD solamente (recomendado)

settings-mcp-client-registration-dcr = CIMD + registro dinámico del cliente

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

permission-studio-field-prompt-json = Enter JSON for {$field}. Leave the editor empty to clear this override.

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

permission-studio-error-empty-value = {$field} cannot be empty.

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

overlay-provider-studio-model-footer = Arrows select · Enter edit · Ctrl+S save · Ctrl+D remove · Esc back

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

provider-studio-auth-openai-device-ready = OpenAI device login is ready. Open the verification URL below and enter {$code}

provider-studio-auth-authorize = authorize {$url}

provider-studio-auth-redirect = redirect {$url}

provider-studio-auth-paste-callback = paste the redirected URL into Callback URL, then press p · state {$state}

provider-studio-auth-copilot-ready = Device login is ready. Open the verification URL below and enter {$code}

provider-studio-auth-verify = verify {$url}

provider-studio-auth-poll = press p to poll now · every {$seconds}s

provider-studio-auth-gitlab-ready = El navegador GitLab OAuth está listo. Abra la URL de autorización a continuación.

provider-studio-auth-atomgit-ready = AtomGit navegador sesión listo · la URL de autorización se muestra abajo

provider-studio-auth-finish-browser = finish the browser flow, then press p · state {$state}

flash-settings-updated = updated {$path}

flash-settings-cleared = cleared {$path}

flash-provider-save-error-settings-object = configuración del proveedor existente debe ser un objeto JSON

command-settings-summary = Abra el banco de trabajo de configuración unificada para modelos, permisos, plugins, tiempo de ejecución, sesiones, interfaz y diagnósticos

settings-mcp-public-url-updated = Agena MCP URL pública actualizada

settings-mcp-oauth-issuer-updated = Agena MCP OAuth emisor URL actualizada

settings-mcp-oauth-password-updated = Agena MCP OAuth password updated

settings-mcp-server-enabled-flash = Servidor MCP de Agena habilitado

settings-mcp-server-disabled-flash = Desactivado servidor Agena MCP

settings-mcp-auth-mode-updated = Agena MCP authentication mode set to {$mode}

settings-mcp-anonymous-access-updated = Agena MCP anonymous tool access set to {$policy}

settings-mcp-client-registration-updated = Agena MCP client registration set to {$policy}

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
overlay-settings-section-plugins-description = Configure complementos, inspeccione sus herramientas y diagnósticos, y administre los arneses del navegador, el shell y el editor.
overlay-settings-section-providers-description = Configure los proveedores y su comportamiento de red e inspeccione el catálogo de modelos.
overlay-settings-section-model-catalog-description = Explore el catálogo de modelos resuelto, inspeccione los metadatos del modelo y actualice la caché local.
overlay-settings-section-permissions-description = Edite los permisos globales, del espacio de trabajo y de la sesión actual por separado.
overlay-settings-section-runtime-session-description = Configure las versiones del cliente de compatibilidad y el comportamiento de compactación automática de la sesión.
settings-permission-effective-detail = Sólo lectura · combinado desde global, espacio de trabajo y sesión.
settings-permission-effective-read-only = El permiso efectivo es de sólo lectura; en su lugar, edite la sesión, el espacio de trabajo o la fuente global.
settings-field-permission-approval-model-description = Variantes de modelo y pensamiento/velocidad utilizadas para decisiones automáticas de permisos; las selecciones no disponibles recurren a Preguntar
settings-field-tui-color-scheme-description = Detecta automáticamente el fondo del terminal o fuerza una paleta clara u oscura
settings-field-tui-graphics-description = Muestre imágenes y escriba fórmulas con Kitty, Sixel o iTerm2 cuando sea compatible; Los cambios entran en vigor después de reiniciar la TUI.
settings-field-activity-default-expanded-description = Estado de expansión predeterminado para actividades sin una anulación específica del tipo. El razonamiento permanece expandido a menos que se establezca explícitamente su tipo.
settings-activity-kind-reasoning-description = El rastro completo del pensamiento del modelo. El valor predeterminado es expandido y se puede contraer por tipo.
runtime-setting-choice-supported-model = apoyado por el modelo actual
settings-plugin-workbench-detail = Abra el banco de trabajo de complementos estructurados para conocer el estado del tiempo de ejecución, la configuración, las herramientas, las operaciones, los registros y los diagnósticos.
settings-mcp-server-detail = Alternar la superficie HTTP MCP en vivo de Agena. El proceso del servidor Agena conectado sigue siendo el tiempo de ejecución real.
settings-mcp-auth-detail = Ciclo sin autenticación, OAuth completo y autenticación mixta ChatGPT. El modo mixto mantiene públicos la inicialización y el descubrimiento de herramientas; las llamadas a herramientas permanecen protegidas por OAuth a menos que el acceso anónimo esté explícitamente habilitado.
settings-mcp-anonymous-access-none-detail = Valor predeterminado seguro: ninguna llamada a la herramienta es anónima; ChatGPT aún puede inicializar y descubrir el catálogo antes de iniciar sesión.
settings-mcp-anonymous-access-read-only-detail = Opción de participación de alto riesgo: las herramientas de solo lectura se pueden ejecutar de forma anónima y pueden revelar datos privados de espacio de trabajo, sistema de archivos, configuración o diagnóstico.
settings-mcp-anonymous-access-inactive-detail = Esta política se aplica únicamente en modo de autenticación mixta; cambie la autenticación a mixta para usarla.
settings-mcp-client-registration-cimd-detail = Acepte únicamente documentos de metadatos de ID de cliente OpenAI ChatGPT; el punto final DCR público no autenticado permanece deshabilitado.
settings-mcp-client-registration-dcr-detail = Modo de compatibilidad: también expone el registro público de cliente dinámico. Habilítelo solo cuando un cliente no pueda usar CIMD.
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
permission-studio-command-rules-shell-only = Las reglas de comando solo se aplican a la herramienta shell canónica (agena.shell.run); use una regla de nombre o la predeterminada para otras herramientas.
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
overlay-skill-studio-title = Administrar habilidades
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
plugin-workbench-column-operation = Operación
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
plugin-workbench-operation-arguments = Argumentos: {$operation}
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
plugin-workbench-header-summary = Herramientas: {$tools}        Operaciones: {$operations}        Configuración: {$config}
plugin-workbench-input-preview = Vista previa de entrada: {$tool}
plugin-workbench-last-result-failed = Último resultado · {$tool} · fallido
plugin-workbench-last-result-success = Último resultado · {$tool} · correcto
plugin-workbench-list-footer = Escriba para buscar · Arriba/Abajo seleccionar · Enter abrir · Esc cerrar
plugin-workbench-list-summary = Buscar plugins… {$query}        Transporte: {$transport}        Configuración: {$config}        {$shown}/{$total} mostrados
plugin-workbench-loading-actions = Cargando acciones…
plugin-workbench-loading-choices = Cargando opciones…
plugin-workbench-no-changes = Sin cambios
plugin-workbench-no-operations = No hay operaciones.
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
plugin-workbench-tab-operations = Operaciones
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
