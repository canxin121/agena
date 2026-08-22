cli-about = Agena Terminal-Chat-Anwendung

pane-sessions = Sitzungen
pane-sessions-search = Sitzungen [{$query}]
pane-transcript = Transkript
pane-messages = Nachrichten
pane-composer = Eingabe [{$session}]

session-meta = #{$id}  {$message_count} Msg  {$updated}
session-running = laeuft
sessions-empty = Keine Sitzungen gefunden
sessions-loading-more = Weitere Sitzungen werden geladen...
sessions-more = Weitere Sitzungen verfuegbar
hub-title = Sitzungs-Hub
hub-action-create = neue Sitzung
hub-action-list = Sitzungsliste
hub-action-refresh = aktualisieren
hub-hint-move = bewegen
hub-hint-focus = Fokus
hub-hint-section = Abschnitt
hub-hint-open = öffnen
hub-hint-back = zurück
hub-section-attention = Benötigt Aufmerksamkeit
hub-section-running = Läuft
hub-section-recent = Zuletzt
hub-empty-attention = Keine Sitzungen benötigen Aufmerksamkeit
hub-empty-running = Keine Sitzungen laufen
hub-empty-recent = Keine kürzlichen Sitzungen
hub-section-new = Neue Sitzung
hub-empty-new = Keine Sitzung zu erstellen
hub-item-new = + Neue Sitzung
hub-item-new-detail = Eingabe zum Erstellen einer Sitzung
hub-action-search = suchen
hub-action-clear-search = Suche löschen
hub-search-placeholder = Tippen zum Filtern von Sitzungen…
hub-search-active-empty = Tippen zum Filtern…
hub-search-active = Filter:{$query}
command-hub-summary = Sitzungs-Hub öffnen
command-background-summary = Zum Hub zurückkehren;Sitzung läuft weiter
hub-empty = Noch keine Sitzungen. Erstellen Sie eine mit Ctrl+N.
context-help-context-hub = Sitzungs-Hub
context-help-summary-hub = Zeigt Sitzungen mit Aufmerksamkeitsbedarf, laufende und kürzliche Sitzungen, und erstellt neue Sitzungen.
context-help-key-create-session = Eine neue Sitzung erstellen.
context-help-key-session-list = Die vollständige Sitzungsliste öffnen.

transcript-header-lines = Zeilen {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = suche={$query} ({$current}/{$total})
transcript-header-tail = am Ende folgen
transcript-header-loading = laedt
transcript-header-loading-older = laedt aeltere Nachrichten
transcript-header-busy = beschaeftigt
transcript-loading-older = Aeltere Nachrichten werden geladen...
transcript-more-older = Aeltere Nachrichten sind verfuegbar. Nach oben scrollen oder PageUp druecken.
transcript-empty-session = In dieser Sitzung gibt es noch keine Nachrichten.

session-state-creating = wird erstellt
session-state-ready = kuerzlich beendet
session-state-running = laeuft
session-state-awaiting-interaction = wartet auf Sie
session-state-interrupted = unterbrochen
session-state-failed = fehlgeschlagen

no-session-selected = Keine Sitzung ausgewaehlt.
no-session-selected-hint = /sessions waehlt eine Sitzung, oder tippen Sie direkt in die Eingabe, um eine neue Sitzung zu erstellen.
composer-session-new = neue Sitzung
composer-placeholder = Nachricht an Agena. Up am Anfang oeffnet den Verlauf. / Befehle. Ctrl+O Datei.

status-global = / abwaerts suchen | ? aufwaerts suchen | Ctrl+C zweimal beendet
status-sessions = Sitzungen: /sessions
status-transcript = VIEW: i Eingabe | j/k scrollen | / suchen | c letzte kopieren | y kopieren
status-composer = INSERT: Esc zurueck | Ctrl+Enter jetzt senden | Ctrl+J neue Zeile | Up am Anfang Verlauf | / Befehle | Ctrl+G Items | Ctrl+R Eingabe | Ctrl+L Freigabe

help-title = Hilfe
help-header = Agena TUI
help-section-sessions = Sitzungswechsler
help-sessions-line-1 = /sessions oeffnet den durchsuchbaren Sitzungswechsler
help-sessions-line-2 = Up/Down, PageUp/PageDown bewegen die Auswahl
help-sessions-line-3 = Enter oeffnet die ausgewaehlte Sitzung
help-section-transcript = Transkriptfenster
help-transcript-line-1 = i wechselt zu INSERT; j/k oder Pfeile scrollen
help-transcript-line-2 = Space / Shift+Space / Ctrl+B blaettern
help-transcript-line-3 = Ctrl+D / Ctrl+U halbe Seite
help-transcript-line-4 = PageUp nahe dem oberen Rand laedt aeltere Nachrichten
help-transcript-line-5 = g/G springt zum Anfang oder Ende
help-transcript-line-6 = / sucht abwaerts, ? sucht aufwaerts; n folgt der Richtung, N kehrt sie um
help-transcript-line-7 = c kopiert die letzte Assistant-Nachricht, y das geladene Transkript, Y den sichtbaren Bereich
help-section-composer = Eingabe
help-composer-line-1 = Esc wechselt zu VIEW; Enter sendet
help-composer-line-2 = Shift+Enter oder Ctrl+J fuegt einen Zeilenumbruch ein
help-composer-line-3 = Ctrl+A/E/B/F/P/N bewegen, Ctrl+Left/Right springen wortweise
help-composer-line-4 = Ctrl+H/D/W/U/K/Y bearbeiten wie in Shell oder Editor
help-composer-line-5 = An Zeilengrenzen kann Ctrl+A/E zur vorherigen/naechsten Zeile weitergehen
help-composer-line-6 = Ctrl+O sucht Workspace-Dateien zum Anhaengen
help-composer-line-7 = Ctrl+E oeffnet $VISUAL/$EDITOR fuer die Eingabe
help-composer-line-8 = Ctrl+T haengt ein Zwischenablagebild an
help-composer-line-9 = Eingefuegter Text wird direkt eingefuegt; ein einzelner Dateipfad wird angehaengt, und Anhaenge bleiben atomar
help-composer-line-10 = Up oeffnet am Anfang des Eingabefelds den Verlauf; Ctrl+P bearbeitet die wartende Nachricht und Ctrl+X bricht sie ab
help-section-actions = Aktionen
help-actions-line-1 = Ctrl+N erstellt eine Sitzung; n/N navigiert Suchtreffer
help-actions-line-2 = r setzt eine blockierte oder wartende Sitzung fort; U öffnet die Nutzungsstatistik
help-actions-line-3 = a/A/d/D antworten auf die erste offene Berechtigungsanfrage
help-actions-line-4 = Ctrl+R oeffnet die erste offene Benutzereingabeanfrage im Composer
help-actions-line-5 = Mouse Capture ist deaktiviert, damit normale Terminal-Auswahl und Kopieren weiter funktionieren
help-actions-line-6 = Zweimal Ctrl+C beendet

overlay-session-search-title = Sitzungssuche
overlay-session-search-prompt = Sitzungstitel durchsuchen
overlay-transcript-search-title = Transkriptsuche
overlay-transcript-search-prompt = Innerhalb geladener Nachrichten suchen
overlay-line-footer = Tippen zum Bearbeiten

overlay-attach-title = Datei anhaengen
overlay-attach-prompt = Geben Sie einen Pfad oder Suchbegriff ein. Enter haengt die ausgewaehlte Datei an.
overlay-attach-no-match = Keine passenden Dateien
overlay-attach-matches = Treffer
overlay-attach-footer = Tab uebernimmt den Pfad

overlay-user-input-title = Ausstehende Benutzereingabe
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = benutzerdefinierter Wert erlaubt
overlay-user-input-reply-format = Antwortformat: 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X bricht die Anfrage ab
overlay-user-input-footer = Ctrl+X abbrechen

flash-terminal-event-error = Terminalereignisfehler: {$error}
flash-created-session = Sitzung erstellt {$title}
flash-permission-reply-sent = Berechtigungsantwort gesendet: {$label}
flash-user-input-reply-sent = Benutzereingabeantwort gesendet
flash-large-paste-staged = Grosse Einfuegung in der Eingabe vorgemerkt
flash-attached = {$path} angehaengt
flash-composer-updated = Eingabe aus externem Editor aktualisiert
flash-prompt-history-empty = Prompt-Verlauf ist leer
flash-prompt-history-items = Entfernen Sie Anhaenge oder vorgemerkte Einfuegungen, bevor Sie den Prompt-Verlauf abrufen
flash-external-editor-failed = Externer Editor fehlgeschlagen: {$error}
flash-clipboard-image-attached = Zwischenablagebild angehaengt: {$width}x{$height} {$format}
flash-clipboard-image-attach-failed = Anhaengen des Zwischenablagebilds fehlgeschlagen: {$error}
flash-no-loaded-transcript = Kein geladenes Transkript zum Kopieren
flash-copied-loaded-transcript = Geladenes Transkript in die Zwischenablage kopiert
flash-no-assistant-message = Keine Assistant-Nachricht zum Kopieren
flash-no-assistant-message-text = Letzte Assistant-Nachricht hat keinen geladenen Text zum Kopieren
flash-copied-assistant-message = Letzte Assistant-Nachricht in die Zwischenablage kopiert
flash-no-visible-transcript = Kein sichtbarer Text zum Kopieren
flash-copied-visible-transcript = Sichtbarer Bereich in die Zwischenablage kopiert
flash-clipboard-copy-failed = Zwischenablage-Kopie fehlgeschlagen: {$error}
flash-message-interrupting = unterbreche die aktive Ausfuehrung - die Nachricht wird als Naechstes gesendet

message-role-user = benutzer
message-role-assistant = assistent
message-role-system = system

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed
message-state-policy-denied = blocked by permission policy
message-state-user-declined = declined by user
message-state-capability-unavailable = capability unavailable
message-state-tool-unavailable = tool unavailable

message-parts-not-loaded = {$count} Teile nicht geladen
message-usage = Nutzung: in={$input} out={$output} reasoning={$reasoning}
message-finish = finish: {$finish}
message-empty = (leere Nachricht)
message-thinking = Denken: {$summary}
message-command-status = Status: {$status}, exit={$exit}
message-file-changes = Dateiaenderungen
message-file-changes-preview-one = 1 Datei: {$paths}
message-file-changes-preview-many = {$count} Dateien: {$paths}
message-file-changes-more = +{$count} weitere
message-search = Suche: {$query}
message-todo-list = Aufgabenliste
message-error = Fehler [{$code}]: {$message}
message-attachments = Anhaenge
message-awaiting-user-input = Warten auf Benutzereingabe: {$request_id}
message-user-input-replied = beantwortete Benutzereingabe: {$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = Teildetails nicht verfuegbar
message-tool-pending = wartet: {$label}
message-tool-running = laeuft: {$label}
message-tool-done = fertig: {$label}
message-tool-failed = fehlgeschlagen: {$label}
message-tool-cancelled = abgebrochen: {$label}
message-tool-result-blocks = {$count} Ergebnisbloecke

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

time-just-now = gerade eben
time-minutes-ago = vor {$count} Min.
time-hours-ago = vor {$count} Std.
time-days-ago = vor {$count} T.

session-default-title = Neue Sitzung {$time}
session-default-base = Neue Sitzung
session-fallback-title = Sitzung {$id}

user-input-error-empty = Antwort darf nicht leer sein
user-input-error-invalid-segment = Ungueltiger Antwortabschnitt: {$segment}
user-input-error-unknown-question = Unbekannte Fragen-ID: {$question_id}
user-input-error-missing-answer = Frage {$question_id} muss mindestens eine Antwort haben
user-input-error-no-answers = Die Antwort enthielt keine Werte

attachment-kind-image = image
attachment-kind-audio = audio
attachment-kind-video = video
attachment-kind-pdf = pdf
attachment-kind-file = datei
attachment-kind-directory = ordner
attachment-generic = anhang
attachment-chip-image = {$kind}: {$filename} ({$width}x{$height}, {$size})
attachment-chip-other = {$kind}: {$filename} ({$size})
attachment-placeholder = [{$kind} {$filename}]

bytes-gb = {$value} GB
bytes-mb = {$value} MB
bytes-kb = {$value} KB
bytes-b = {$value} B

paste-label = Einfuegung mit {$count} Zeichen
paste-label-append = Einfuegung mit {$count} Zeichen, beim Senden anhaengen
paste-placeholder = [Einfuegung mit {$count} Zeichen]

permission-label-allow-once = einmal erlauben
permission-label-allow-always = immer erlauben
permission-label-deny-once = einmal ablehnen
permission-label-deny-always = immer ablehnen

permission-summary-allow-once = Einmal erlaubt: {$reason}
permission-summary-allow-always = Immer erlaubt: {$reason}
permission-summary-deny-once = Einmal abgelehnt: {$reason}
permission-summary-deny-always = Immer abgelehnt: {$reason}

failure-detail-message = Nachricht
failure-detail-code = Fehlercode
failure-detail-category = Kategorie
failure-detail-responsibility = Verantwortung
failure-detail-impact = Auswirkung
failure-detail-recovery = Wiederherstellung
failure-detail-retry = Wiederholung
failure-category-invalid-input = Ungültige Eingabe
failure-category-not-found = Nicht gefunden
failure-category-conflict = Konflikt
failure-category-permission-required = Berechtigung erforderlich
failure-category-permission-denied = Berechtigung verweigert
failure-category-authentication-required = Authentifizierung erforderlich
failure-category-rate-limited = Rate begrenzt
failure-category-quota-exceeded = Kontingent überschritten
failure-category-timeout = Zeitüberschreitung
failure-category-dependency-unavailable = Abhängigkeit nicht verfügbar
failure-category-protocol-failure = Protokollfehler
failure-category-data-corruption = Datenintegritätsproblem
failure-category-internal = Interner Fehler
failure-responsibility-caller = Die Anfrage
failure-responsibility-policy = Richtlinie
failure-responsibility-dependency = Die Abhängigkeit
failure-responsibility-system = Das System
failure-impact-request-rejected = Anfrage abgelehnt
failure-impact-operation-failed = Operation fehlgeschlagen
failure-impact-operation-paused = Operation pausiert
failure-impact-partial-success = Teilerfolg
failure-impact-background-task-failed = Hintergrundaufgabe fehlgeschlagen
failure-impact-runtime-degraded = Laufzeit degradiert
failure-impact-fatal-startup-failure = Schwerwiegender Startfehler
failure-recovery-none = Keine automatische Wiederherstellung
failure-recovery-refresh = Aktualisieren
failure-recovery-reauthenticate = Neu anmelden
failure-recovery-open-settings = Einstellungen öffnen
failure-recovery-request-permission = Berechtigung anfragen
failure-recovery-ask-user = Benutzer fragen
failure-recovery-retry = Wiederholen
failure-recovery-choose-alternative = Alternative wählen
failure-recovery-restart-plugin = Plugin neu starten
failure-recovery-restart-runtime = Laufzeit neu starten
failure-retry-never = Nicht wiederholen
failure-retry-correct-input = Eingabe korrigieren und wiederholen
failure-retry-after-user-action = Nach Benutzeraktion wiederholen
failure-retry-after-refresh = Nach Aktualisierung wiederholen
failure-retry-immediate-once = Einmal sofort wiederholen
failure-retry-backoff = Mit Backoff wiederholen
failure-retry-use-alternative = Alternative verwenden
failure-retry-unknown = Unbekannt

## Settings Studio core locale coverage
## Long policy descriptions intentionally continue to use the verified English fallback.

permission-studio-new-rule-label = + Neue Regel

permission-studio-new-rule-value = Schaffen

permission-studio-catalog-tags-title = Add Tool Tag Regeln

permission-studio-catalog-names-title = Add Tool Access Regeln

permission-studio-catalog-footer = Down to Results · Space toggle · Enter wählen Modus · Esc annullieren

permission-studio-catalog-tag-detail = Verwendet von {$count} registriertes Tool(s)

permission-studio-catalog-custom-label = + Zollregel ...

permission-studio-catalog-custom-search = Benutzerdefinierter neuer manueller Tag Tool Name

overlay-settings-title = Einstellungen

overlay-settings-footer = STRG+R Refresh · ←/→ Schaltflächen · Tab/Shift+ Tab-Zyklusscheiben · ↑/↓ select · Enter open · Esc close

overlay-settings-sections = Bereiche

overlay-settings-options = Optionen

overlay-settings-group-core = Kern

overlay-settings-group-application = Anwendung

overlay-settings-group-session = Sitzung

overlay-settings-group-system = System

overlay-settings-default-section-title = Abschnitt

overlay-settings-empty-section = Kein Abschnitt ausgewählt.

overlay-settings-empty-items = Keine Einstellungen in diesem Abschnitt.

overlay-settings-empty-detail = Wählen Sie einen Abschnitt und eine Option zum Überprüfen oder Bearbeiten.

overlay-settings-detail-current = Aktueller Wert: {$value}

overlay-settings-detail-path = Pfad: {$path}

overlay-settings-detail-action = Öffnen oder bearbeiten Sie diese Einstellung.

settings-detail-action-screen = Öffnen Sie diesen Bildschirm.

overlay-settings-edit-title = Edit {$field}

overlay-settings-edit-file-value = File Override: {$value}

overlay-settings-edit-effective-value = Effektiver Wert: {$value}

overlay-choice-clear-settings-detail = Entfernen Sie die Datei override für {$field}.

overlay-settings-section-plugins-label = Plugins und Werkzeuge

overlay-settings-section-plugins-summary = Plugin-Konfiguration, Tools, Gurte und Diagnosen

overlay-settings-section-providers-label = Modelle und Anbieter

overlay-settings-section-providers-summary = {$count} Konfigurierte Anbieter

overlay-settings-section-model-catalog-label = Modellkatalog

overlay-settings-section-model-catalog-summary = {$count} Einträge

overlay-settings-section-permissions-label = Berechtigungen

overlay-settings-section-permissions-summary = {$count} persisted permission rule(s)

overlay-settings-section-tracing-summary = Protokollfilter und Diagnose

overlay-settings-section-ui-label = Darstellung

overlay-settings-section-ui-summary = Lokale und Schnittstellenpräferenzen

overlay-settings-section-ui-description = Anhaltende Sprache, Farbe, Grafiken und Themeneinstellungen.

overlay-settings-section-runtime-session-label = Laufzeit und Sitzung

overlay-settings-section-runtime-session-summary = Provider-Client-Identitäten und Kontext-Kompaktierung

settings-permission-global-label = Globale Erlaubnis

settings-permission-global-detail = Baseline für alle Sitzungen.

settings-permission-workspace-label = Arbeitsbereich Berechtigung

settings-permission-workspace-detail = Override Layer für das aktuelle Projekt.

settings-permission-current-label = Aktuelle Sitzungsgenehmigung

settings-permission-current-detail = Gilt nur für die aktuelle Sitzung.

settings-permission-effective-label = Effektive Erlaubnis

settings-permission-layer-global = Global

settings-permission-layer-workspace = Arbeitsbereich

settings-permission-layer-session = Sitzung

settings-permission-layer-effective = Wirksam

settings-runtime-thinking-label = Denkmodus

settings-runtime-thinking-description = Current-Session-Denkmodus überschreiben

settings-runtime-speed-label = Geschwindigkeitsregelung

settings-runtime-speed-description = Current Session Speed Mode Override Übersetzung

settings-runtime-verbosity-label = Prägung

settings-runtime-verbosity-description = current-session verbosity überschreiben

settings-field-permission-approval-model-label = Automatisches Freigabemodell

settings-field-ui-locale-label = Sprache

settings-field-ui-locale-description = Schnittstellensprache

settings-field-tui-color-scheme-label = Terminal-Farbschema

settings-field-tui-theme-label = TUI-Plugin-Theme

settings-field-tui-theme-description = Optionale Plugin-bereitgestellte semantische Farbpalette

settings-choice-tui-color-scheme-auto = Terminalhintergrund automatisch erkennen

settings-choice-tui-color-scheme-dark = Optimieren Sie Farben für einen dunklen Terminalhintergrund

settings-choice-tui-color-scheme-light = Optimieren Sie Farben für einen hellen Terminalhintergrund

settings-field-tui-graphics-label = Erweiterte Terminalgrafik

settings-choice-tui-graphics-auto = Automatisch native Grafiken aushandeln und sicher auf Unicode zurückgreifen (empfohlen)

settings-choice-tui-graphics-native = Erzwingen Sie native Grafikverhandlungen für einen von Experten konfigurierten Terminalpfad

settings-choice-tui-graphics-unicode = Deaktivieren Sie native Grafiken und verwenden Sie deterministisches Unicode/Text-Rendering

settings-field-activity-default-expanded-label = Aktivitäten standardmäßig erweitern

settings-field-activity-kind-description = Standard-Erweiterungszustand für diese Aktivitätsart.

settings-field-activity-tool-label = Tool Default Erweiterung

settings-field-activity-tool-description = Standard-Erweiterungszustand für genau dieses Werkzeug.

settings-activity-kind-reasoning-label = Argumentation

settings-activity-kind-operation-label = Tool-Operationen

settings-activity-kind-operation-description = Tool Calls und deren Ergebnisse.

settings-activity-kind-resource-label = Ressourcen

settings-activity-kind-resource-description = Anlagen und sonstige Ressourceninhalte.

settings-activity-kind-skill_reference-label = Qualifikationsreferenzen

settings-activity-kind-skill_reference-description = Verweise auf die in der Antwort verwendeten Fähigkeiten.

settings-activity-kind-interaction-label = Wechselwirkungen

settings-activity-kind-interaction-description = Benutzereingabeanforderungen und interaktive Eingabeaufforderungen.

settings-activity-kind-hook-label = Haken

settings-activity-kind-hook-description = Session Hook Runs und Lifecycle Events.

settings-activity-kind-error-label = Fehler

settings-activity-kind-error-description = Fehlgeschlagene Operationen und Terminalausfälle.

settings-activity-kind-notice-label = Bekanntmachungen

settings-activity-kind-notice-description = Hintergrundhinweise und Informationszeilen.

settings-activity-kind-text-label = Text

settings-activity-kind-text-description = Klartext und Textartefaktinhalt.

settings-field-tracing-filter-label = Anwendungsprotokollstufe

settings-field-tracing-filter-description = Standardprotokollierungsebene

settings-field-tracing-database-label = Datenbankprotokollstufe

settings-field-tracing-database-description = Datenbankverfolgungsprotokollebene

settings-field-tracing-adapter-label = Adapterprotokollstufe

settings-field-tracing-adapter-description = Provider Adapter Tracing Log Level

settings-config-open-file-detail = Open agena.json für diesen Weg

settings-source-unset = Nicht eingestellt

settings-source-configured = Konfiguriert: {$value}

settings-source-effective = Effektiv: {$value}

settings-source-file-effective = Datei: {$file} / Effektiv: {$effective}

settings-source-file-found = {$path} (gefunden)

settings-source-file-missing = {$path} (wird erstellt)

settings-source-row-config-file = Konfigurierungsdatei

settings-source-row-workspace-config-file = Workspace-Konfigurationsdatei

settings-source-row-file-value = Dateiwert

settings-source-row-workspace-value = Arbeitsbereichswert

settings-source-row-effective-value = Effektivwert

settings-source-row-write-target = Schreibt an

settings-source-row-layers = Aktive Schichten

settings-source-current-session = aktuelle Sitzungslaufzeitdaten

settings-source-current-session-runtime = Aktuelle Session Run Optionen

settings-detail-values-heading = Werte

settings-detail-sources-heading = Quellen

settings-detail-action-readonly = Öffnen Sie die Read-Only Effective View.

settings-detail-action-file = Öffnen Sie die Backing Config-Datei.

settings-harness-browser-label = Browser Harness

settings-harness-shell-label = Hülle

settings-harness-editor-label = Herausgeberin Harness

settings-field-parse-bool = {$field} erwartet ein boolesches wie true/false oder on/off

settings-field-parse-integer = {$field} erwartet einen unsignierten Ganzzahlwert

settings-field-parse-float = {$field} erwartet einen numerischen Wert

settings-choice-adapter-fallback = Adapter


settings-plugin-workbench-label = Plugin-Konfigurationsarbeitsbereich

settings-mcp-server-label = Agena MCP-Server

settings-mcp-server-value = Toggle aktiviert/deaktiviert

settings-mcp-server-enabled = ermöglicht

settings-mcp-server-disabled = Behinderte

settings-mcp-status-unavailable = Status nicht verfügbar

settings-mcp-ready = bereit

settings-mcp-needs-attention = braucht Aufmerksamkeit

settings-mcp-auth-label = MCP-Authentifizierung

settings-mcp-auth-none = Anonym: Jedes exponierte Tool

settings-mcp-auth-oauth = Vollständiges OAuth

settings-mcp-auth-mixed = gemischt: Public Discovery, per Tool OAuth

settings-mcp-anonymous-access-label = Anonymer Werkzeugzugriff bei gemischter Authentifizierung

settings-mcp-anonymous-access-none = Keine (empfohlen)

settings-mcp-anonymous-access-read-only = Authority-Contract Read-Only-Tools

settings-mcp-registration-label = Registrierung

settings-mcp-pkce-label = PKCE

settings-mcp-client-registration-label = OAuth-Clientregistrierung

settings-mcp-client-registration-cimd = CIMD nur (empfohlen)

settings-mcp-client-registration-dcr = CIMD + Dynamische Kundenregistrierung

settings-mcp-public-url-label = Öffentliche MCP-URL

settings-mcp-public-url-value = Edit

settings-mcp-public-url-auto = Hörer-lokaler Fallback

settings-mcp-oauth-issuer-label = OAuth-Aussteller-URL

settings-mcp-oauth-issuer-derived = abgeleitet von MCP Resource Origin

settings-mcp-oauth-password-label = MCP-OAuth-Passwort

settings-mcp-oauth-password-value = Set oder Ersatz

settings-mcp-oauth-password-configured = MCP-spezifisches Passwort konfiguriert

settings-mcp-oauth-password-ui-fallback = Verwendung von UI Password Fallback

settings-mcp-oauth-password-not-configured = nicht konfiguriert

settings-mcp-oauth-password-clear-label = MCP OAuth Password

settings-field-runtime-codex-version-label = Codex Client Version

settings-field-runtime-claude-version-label = Claude Code Version

settings-field-runtime-gemini-version-label = Gemini CLI Version

settings-field-session-compaction-auto-label = Automatische Komprimierung

settings-field-session-compaction-reserved-tokens-label = Reservierte Komprimierungs-Tokens

settings-client-versions-refresh-label = Kundenversionen aktualisieren

settings-client-versions-refresh-value = Abrufen letzten

settings-client-versions-entry-label = Clientversionen des Anbieters

settings-client-versions-entry-value = Codex · claude · gemini

settings-client-versions-section-label = Clientversionen

settings-client-versions-section-summary = Laufzeitidentitätsversionen

settings-provider-workbench-label = Anbieterliste

settings-provider-workbench-value = {$count} Anbieter(s)

settings-global-default-model-label = Standardmodell

settings-global-default-model-description = Modell für Sitzungen ohne eigene Modellangabe; ein explizites Sitzungsmodell hat immer Vorrang.

settings-model-default-mode-inherit-detail = Verwenden Sie den nativen Standardmodus des ausgewählten Modells.

settings-provider-new-label = + Neuer Anbieter

settings-provider-existing-detail = {$count} Adapter konfiguriert

settings-model-catalog-open-label = Offener Modellkatalog

settings-files-open-config-label = Open agena.json

settings-files-open-config-present = vorhanden

settings-files-open-config-create = Create on Open

permission-studio-field-path-workspace = Trasse Workspace Defaults

permission-studio-field-path-external = Externe Trassenausfälle

permission-studio-field-path-rules = Trasseregeln

permission-studio-field-network-defaults = Netzausfälle

permission-studio-field-network-rules = Netzregeln

permission-studio-field-tool-names = Werkzeugnamen

permission-studio-field-tool-rules = Werkzeugregeln

permission-studio-field-prompt-json = Geben Sie JSON für {$field} ein. Lassen Sie den Editor leer, um diesen Override zu löschen.

permission-studio-detail-override = Übersteuern

permission-studio-detail-effective = Wirksam

permission-studio-detail-override-inline = Überschreiben {$value}

permission-studio-detail-effective-inline = Effektiv {$value}

permission-studio-detail-read-only = Dieses Berechtigungsdokument wird hier nur gelesen.

permission-studio-detail-mode-editable = Enter öffnet den Mode Picker für dieses eine Feld.

permission-studio-detail-text-editable = Enter bearbeitet diesen einzelnen Schlüssel oder Muster.

permission-studio-detail-remove-hint = Enter entfernt diesen Artikel sofort.

permission-studio-detail-navigate-hint = Enter öffnet diesen Abschnitt.

permission-studio-overview-target = Ziel

permission-studio-overview-source = Quelle

permission-studio-overview-scope = Anwendungsbereich

permission-studio-overview-override = Übersteuern

permission-studio-overview-effective = Wirksam

permission-studio-section-workspace = Arbeitsbereich

permission-studio-section-external = Außen

permission-studio-section-rules = Vorschriften

permission-studio-section-defaults = Ausfälle

permission-studio-source-global = global

permission-studio-source-workspace = Arbeitsbereich

permission-studio-source-session = Sitzung

permission-studio-source-effective = wirksam

permission-studio-settings-override = Überschreiben {$value}

permission-studio-settings-effective = Effektiv {$value}

permission-studio-mode-read = Lesen Sie {$value}

permission-studio-mode-write = Schreibe {$value}

permission-studio-network-default = {$label} {$value}

permission-studio-page-overview = Übersicht

permission-studio-page-path = Wegstrecke

permission-studio-page-path-defaults = Dateisystem / Standardzonen

permission-studio-page-path-rules = Dateisystem / Pfadregeln

permission-studio-page-network = Netz

permission-studio-page-network-zones = Netzwerk / Netzwerkzonen

permission-studio-page-network-rules = Netzwerk / Domainregeln

permission-studio-page-tools = Werkzeuge

permission-studio-page-tool-tags = Tool Access / Tag Regeln

permission-studio-page-tool-names = Werkzeugzugriff / Namensregeln

permission-studio-page-tool-command-rules = Werkzeugzugriff / Befehlsregeln

permission-studio-page-names = Namen

permission-studio-page-tool-rules = Werkzeugregeln

permission-studio-nav-overview = Übersicht

permission-studio-nav-filesystem = Dateisystem

permission-studio-nav-default-zones = Standardzonen

permission-studio-nav-path-rules = Pfadregeln

permission-studio-nav-network = Netzwerk

permission-studio-nav-network-zones = Netzwerkzonen

permission-studio-nav-domain-rules = Domainregeln

permission-studio-nav-tool-access = Werkzeugzugriff

permission-studio-nav-name-rules = Namensregeln

permission-studio-nav-command-rules = Befehlsregeln

permission-studio-path-workspace-read = Arbeitsbereich lesen

permission-studio-path-workspace-write = Arbeitsbereich schreiben

permission-studio-path-external-read = Extern lesen

permission-studio-path-external-write = Extern schreiben

permission-studio-path-rule-read = Lesemodus

permission-studio-path-rule-write = Schreibmodus

permission-studio-network-internet = Internet

permission-studio-network-private = Privat

permission-studio-network-loopback = Loopback

permission-studio-tool-default = Werkzeugstandard

permission-studio-tool-default-summary = Standard {$value}

permission-studio-add-path-rule = Add Path Rule

permission-studio-add-network-rule = Netzwerkziel hinzufügen

permission-studio-add-name = Name angeben

permission-studio-add-tool-rule = Add Tool Rule

permission-studio-rule-key = Schlüssel

permission-studio-rule-pattern = Muster

permission-studio-rule-target = Ziel

permission-studio-rule-mode = Modus

permission-studio-tool-rule-fallback = Fallback-Modus

permission-studio-error-empty-value = {$field} kann nicht leer sein.

overlay-providers-title = Anbieter

overlay-providers-prompt = Wählen Sie einen Anbieter zur Konfiguration aus

overlay-provider-list-title = Anbieterliste

overlay-provider-list-prompt = Suchkonfigurierte Anbieter

overlay-provider-list-footer = Wählen Sie Create Provider oder einen bestehenden Provider aus und drücken Sie dann Enter

overlay-provider-list-create-label = + Neuer Anbieter

overlay-provider-list-row-detail-no-model = {$adapter} · {$count} konfigurierte Adapter

overlay-provider-studio-title = Anbieter Config

overlay-provider-studio-header = Anbieter Config

overlay-provider-studio-footer = Tab/Shift+Tab panels · Pfeile auswählen · Space toggle · Enter edit · Strg+D ausgewählt löschen · Strg+R aktualisieren · Strg+N Modell hinzufügen · Strg+ Ein Save-Adapter · Ctrl+S save provider · Esc close

overlay-provider-studio-providers = Anbieter

overlay-provider-studio-draft = Entwurf

overlay-provider-studio-adapters = Adapter

overlay-provider-studio-models = Modelle

overlay-provider-studio-catalog = Modellkatalog

overlay-provider-studio-detail = Einzelheiten

overlay-provider-studio-adapter-models-empty = Wählen Sie Adapter aus und listen Sie dann ihre Live-Modelle auf

overlay-provider-studio-models-empty = Keine Adaptermodelle verfügbar

overlay-provider-studio-catalog-empty = Keine Katalogeinträge passen zu dieser Abfrage

overlay-provider-studio-new-provider-detail = Leerer Anbieterentwurf

overlay-provider-studio-provider-row-detail-no-model = {$adapter} · {$count} konfigurierte Adapter

overlay-provider-studio-model-count = {$count} Modelle

overlay-provider-studio-loaded = beladen

overlay-provider-studio-error = Fehler

overlay-provider-studio-configured = konfiguriert

overlay-provider-studio-live-list = Live-Liste

overlay-provider-studio-not-listed = nicht aufgeführt

overlay-provider-studio-not-supported = Nicht unterstützt durch den aktuellen Auth-Vertrag

overlay-provider-studio-edit-title = Edit Field

overlay-provider-studio-edit-prompt = Update {$field}

overlay-provider-studio-edit-footer = Typ zum Bearbeiten

overlay-provider-studio-model-edit-footer = Ctrl+S speichern Modellkonfiguration

overlay-provider-studio-model-json-title = Modellkonfiguration · {$adapter}/{$model}

overlay-provider-studio-model-json-prompt = Bearbeiten Sie das persistente Anbietermodell JSON.

overlay-provider-studio-model-title = Modell · {$adapter}/{$model}

overlay-provider-studio-model-footer = Pfeile auswählen · Bearbeiten eingeben · Strg+S speichern · Strg+D entfernen · Esc zurück

overlay-provider-delete-title = Anbieter löschen

overlay-provider-delete-adapter-title = Adapter löschen

overlay-provider-delete-model-title = Löschmodell

overlay-provider-studio-model-edit-title = Modellfeld bearbeiten

overlay-provider-studio-model-field-prompt = Update {$field}

overlay-provider-studio-new-model-title = Add-Modell

overlay-provider-studio-edit-auth-mode-prompt = Aktualisieren Sie den Auth-Modus (none | api | credential)

overlay-provider-studio-edit-auth-subtype-prompt = Aktualisieren auth Subtype (api: custom | cline api | gitlab api | bedrock sigv4 · credential: openai chatgpt | github copilot | gitlab | google adc | sap ai core)

overlay-provider-studio-edit-auth-login-method-prompt = Aktualisieren Sie die Auth-Login-Methode (Gerät | Browser)

provider-studio-auth-status-pending = anhängig

provider-studio-auth-status-unset = entschärft

provider-studio-auth-status-none = nicht

provider-studio-auth-status-select-subtype = Select Subtype

provider-studio-auth-status-select-issuer = Select Subtype

provider-studio-auth-status-configured = konfiguriert

provider-studio-auth-status-partial = teilweise

provider-studio-summary-env = env

provider-studio-summary-callback = Rückruf

provider-studio-summary-redirect = Redirect

provider-studio-summary-account = Konto

provider-studio-summary-name = Name

provider-studio-summary-user = Nutzer

provider-studio-summary-email = E-Mail

provider-studio-summary-profile = Profil

provider-studio-summary-region = Region

provider-studio-summary-code = Code

provider-studio-summary-state = Zustand {$state}

provider-studio-summary-tokens-set = Tokensatz

provider-studio-summary-keys-set = Schlüsselsatz

provider-studio-summary-set-field = set {$field}

provider-studio-summary-review-fields = Prüfung Authenfelder

provider-studio-summary-start-browser = Start Browser OAuth

provider-studio-summary-restart-browser = Browser OAuth neu starten

provider-studio-summary-open-authorize = Offene Autorisierung URL

provider-studio-summary-start-device = Startgeräteanmeldung

provider-studio-summary-restart-device = Wiederanmelden des Geräts

provider-studio-summary-open-verify = Offene Verifizierungs-URL

provider-studio-summary-finish-callback = Finish Callback Exchange

provider-studio-summary-poll-every = Alle {$seconds}s

provider-studio-summary-paste-callback = Paste Callback URL

provider-studio-summary-poll-now = Umfrage jetzt

provider-studio-summary-start-auth-first = Start auth first

provider-studio-summary-poll-browser = Umfrage Browser Ergebnis

provider-studio-auth-openai-ready = Browser OAuth ist bereit. Öffnen Sie die Authorize URL unten.

provider-studio-auth-openai-device-ready = OpenAI Device Login ist bereit. Öffnen Sie die Verifizierungs-URL unten und geben Sie {$code} ein

provider-studio-auth-authorize = autorisieren {$url}

provider-studio-auth-redirect = Redirect {$url}

provider-studio-auth-paste-callback = Fügen Sie die umgeleitete URL in Callback URL ein und drücken Sie dann p · state {$state}

provider-studio-auth-copilot-ready = Die Geräteanmeldung ist bereit. Öffnen Sie die Verifizierungs-URL unten und geben Sie {$code} ein

provider-studio-auth-verify = verifizieren {$url}

provider-studio-auth-poll = Jetzt p auf pollen drücken · alle {$seconds}s

provider-studio-auth-gitlab-ready = GitLab Browser OAuth ist bereit. Öffnen Sie die Authorize URL unten.

provider-studio-auth-atomgit-ready = AtomGit Browsersitzung bereit · die Autorisierungs-URL wird unten angezeigt

provider-studio-auth-finish-browser = Beenden Sie den Browserfluss und drücken Sie dann p · state {$state}

flash-settings-updated = aktualisiert {$path}

flash-settings-cleared = gelöscht {$path}

flash-provider-save-error-settings-object = bestehende Providereinstellungen müssen ein JSON-Objekt sein

command-settings-summary = Öffnen Sie die Workbench für einheitliche Einstellungen für Modelle, Berechtigungen, Plugins, Laufzeit, Sitzungen, Schnittstelle und Diagnose

settings-mcp-public-url-updated = Agena MCP Public URL aktualisiert

settings-mcp-oauth-issuer-updated = Agena MCP OAuth Emittenten-URL aktualisiert

settings-mcp-oauth-password-updated = Alterna MCP OAuth Passwort aktualisiert

settings-mcp-server-enabled-flash = Agena MCP-Server aktiviert

settings-mcp-server-disabled-flash = Agena MCP-Server deaktiviert

settings-mcp-auth-mode-updated = Agena MCP-Authentifizierungsmodus eingestellt auf {$mode}

settings-mcp-anonymous-access-updated = Agena MCP anonymer Toolzugriff auf {$policy}

settings-mcp-client-registration-updated = Agena MCP-Clientregistrierung auf {$policy} eingestellt

settings-mcp-oauth-password-cleared = Alterna MCP OAuth Passwort gelöscht

permission-studio-command-pattern-title = {$tool_name} Befehlsmuster

settings-tool-api-list-description = Execution Tools aufzählen.

settings-tool-api-search-description = Search Execution Tools.

settings-tool-api-help-description = Prüfung von Ausführungs-Tool-Verträgen.

settings-tool-api-tags-description = Listenausführungstool-Tags.

settings-tool-api-call-description = Rufen Sie ein Ausführungstool auf.

settings-tool-api-plugins-list-description = Aufzählen von Tool Plugins.

settings-tool-api-plugins-search-description = Search Tool Plugins.

settings-tool-api-plugins-tags-description = Liste Tool-Plugin-Tags.

permission-studio-command-pattern-help = Geben Sie ein Shell-Befehls-Glob ein, z. B. `git status` oder `git push *`.

permission-studio-rename-unsupported = Dieser Eintrag kann nicht umbenannt werden; löschen Sie ihn und erstellen Sie ihn neu.

# Settings, provider, permission, catalog, MCP, and diagnostics completion
overlay-editor-footer-single-line = Geben Sie Folgendes ein, um es zu bearbeiten
overlay-editor-footer-multiline = Ctrl+S speichern
context-help-title = Kontexthilfe
context-help-eyebrow = Aktuelle Schnittstelle
context-help-footer = ↑/↓ scrollen · Esc oder Ctrl+H schließen
context-help-global-hint = Ctrl+H Hilfe
context-help-context-composer-items = Komponistenartikel
context-help-context-suggestions = Vorschläge
context-help-context-usage = Nutzungs-Dashboard
context-help-context-plan-viewer = Plan-Viewer
context-help-context-user-input = Benutzereingabeanforderung
context-help-context-plugin-list = Plugin-Workbench · Liste
context-help-context-plugin-detail = Plugin-Workbench · Details
context-help-context-plugin-config = Plugin Workbench · Konfig
context-help-context-plugin-actions = Plugin-Konfiguration · Aktionen
context-help-context-plugin-selection = Plugin-Konfiguration · Auswahl
context-help-context-plugin-drilldown = Plugin-Konfiguration · Drilldown
context-help-context-plugin-diff = Plugin-Konfiguration · Diff
context-help-key-delete = Entfernen Sie das ausgewählte Element.
context-help-key-plugin-restart = Starten Sie das ausgewählte Plugin neu, wenn es unterstützt wird.
overlay-permission-title = Erlaubnisanfrage
overlay-permission-details-title = Einzelheiten
overlay-permission-action-tool = Werkzeug: { $tool }
overlay-permission-action-path = Pfad { $access }: { $path }
overlay-permission-action-network = Netzwerk: { $target }
overlay-permission-field-tool = Werkzeug
overlay-permission-field-target = Befehl oder Ziel
overlay-permission-field-access = Zugang
overlay-permission-field-path = Pfad
overlay-permission-field-workspace = Arbeitsbereich
overlay-permission-field-network = URL oder Netzwerkziel
overlay-permission-field-host = Gastgeber
overlay-permission-field-reason = Warum eine Genehmigung erforderlich ist
overlay-permission-detail-request-id = ID anfordern
overlay-permission-detail-source = Richtlinienquelle
overlay-permission-detail-scope = Gewünschter Umfang
overlay-permission-detail-operator = Angefordert von
overlay-permission-detail-trace = Entscheidungsspur
overlay-permission-summary-more-approvals = Außerdem werden { $count } weitere Aktionen in diesem Tool-Aufruf genehmigt
overlay-permission-detail-requested-actions = Bitte auch um Genehmigung
overlay-permission-detail-related-actions = In diesem Anruf bereits zulässig
overlay-permission-choice-auto-approve = Automatisch genehmigen…
overlay-permission-rule-workbench-title = Berechtigungsregel
overlay-permission-rule-studio-footer = Pfeile auswählen · Eingabetaste Bearbeiten · Ctrl+O Ausgewählten Pfad durchsuchen · Ctrl+S Speichern · Ctrl+D widerrufen · Esc schließen
overlay-permission-rule-studio-footer-return = Pfeile auswählen · Eingabetaste Bearbeiten · Ctrl+O Ausgewählten Pfad durchsuchen · Ctrl+S Speichern · Ctrl+D widerrufen · Esc kehrt zur Berechtigungsanfrage zurück
flash-permission-rule-browse-path-selection = Wählen Sie vor dem Durchsuchen den Zielpfad oder das Arbeitsbereichsstammverzeichnis aus.
overlay-permission-rule-choice-subject-title = Wählen Sie die Art des Betreffs
overlay-permission-rule-choice-subject-prompt = Wählen Sie den Regelsubjekttyp aus.
overlay-permission-rule-choice-subject-tool-detail = einem Tool oder Runtime-Tool entsprechen
overlay-permission-rule-choice-subject-path-access-detail = Passen Sie den Dateisystemzugriff an
overlay-permission-rule-choice-subject-network-access-detail = Passen Sie den Netzwerkzugriff an
overlay-permission-rule-choice-access-title = Wählen Sie Pfadzugriffsart
overlay-permission-rule-choice-access-prompt = Wählen Sie den Dateisystem-Zugriffsmodus.
overlay-permission-rule-choice-access-read-detail = Nur das Lesen von Dateien zulassen
overlay-permission-rule-choice-access-write-detail = Nur Dateischreibvorgänge zulassen
overlay-permission-rule-choice-access-read-write-detail = Erlaubt sowohl Lese- als auch Schreibvorgänge
overlay-permission-rule-choice-scope-title = Wählen Sie Regelumfang
overlay-permission-rule-choice-scope-prompt = Wählen Sie aus, wie weitreichend die Regel bestehen bleiben soll.
overlay-permission-rule-choice-scope-session-detail = nur diese Sitzung
overlay-permission-rule-choice-scope-workspace-detail = Alle Sitzungen in diesem Arbeitsbereich
overlay-permission-rule-choice-scope-global-detail = alle Arbeitsbereiche
overlay-permission-rule-choice-mode-title = Wählen Sie den Regelmodus
overlay-permission-rule-choice-mode-prompt = Wählen Sie „Zulassen“, „Fragen“ oder „Ablehnen“.
overlay-permission-rule-choice-mode-allow-detail = immer passende Aktionen zulassen
overlay-permission-rule-choice-mode-auto-detail = Lassen Sie das konfigurierte Genehmigungsmodell entscheiden. Greifen Sie auf eine Eingabeaufforderung zurück, wenn diese nicht verfügbar ist
overlay-permission-rule-choice-mode-ask-detail = Geben Sie eine Eingabeaufforderung ein, bevor Sie entsprechende Aktionen zulassen
overlay-permission-rule-choice-mode-deny-detail = entsprechende Aktionen immer ablehnen
overlay-permission-rule-editor-footer = Geben Sie Folgendes ein, um es zu bearbeiten
overlay-permission-rule-editor-tool-name-title = Werkzeugnamen bearbeiten
overlay-permission-rule-editor-tool-name-prompt = Geben Sie den genauen Werkzeugnamen ein.
overlay-permission-rule-editor-qualifier-title = Qualifizierer bearbeiten
overlay-permission-rule-editor-qualifier-prompt = Geben Sie ein optionales Qualifikationsmerkmal ein oder lassen Sie es leer.
overlay-permission-rule-editor-workspace-root-title = Arbeitsbereichsstamm bearbeiten
overlay-permission-rule-editor-workspace-root-prompt = Geben Sie optional ein workspace_root-Verzeichnis ein.
overlay-permission-rule-editor-target-path-title = Zielpfad bearbeiten
overlay-permission-rule-editor-target-path-prompt = Geben Sie den Zielpfad oder das Zielmuster ein.
overlay-permission-rule-editor-network-target-title = Netzwerkziel bearbeiten
overlay-permission-rule-editor-network-target-prompt = Geben Sie einen Host, Host:Port oder eine URL ein.
overlay-permission-rule-editor-session-id-title = Sitzungs-ID bearbeiten
overlay-permission-rule-editor-session-id-prompt = Geben Sie die Zielsitzungs-ID ein.
overlay-permission-rule-browser-workspace-root-title = Wählen Sie Workspace Root
overlay-permission-rule-browser-workspace-root-prompt = Durchsuchen Sie Verzeichnisse und drücken Sie die Eingabetaste, um eines auszuwählen.
overlay-permission-rule-browser-target-path-title = Wählen Sie Zielpfad
overlay-permission-rule-browser-target-path-prompt = Durchsuchen Sie Dateien oder Verzeichnisse und drücken Sie die Eingabetaste, um eines auszuwählen.
overlay-permission-rule-browser-footer = Wählen Sie ../ oder ein Verzeichnis aus und drücken Sie zum Durchsuchen die Eingabetaste. Wählen Sie einen Wert aus und drücken Sie zur Bestätigung die Eingabetaste
overlay-permission-rule-browser-empty = Keine passenden Dateien oder Verzeichnisse.
overlay-permission-rule-item-subject-kind = Betreff Art
overlay-permission-rule-item-subject-kind-detail = Wählen Sie aus, ob diese Regel für ein Werkzeug, einen Pfad oder ein Netzwerkziel gilt.
overlay-permission-rule-item-mode = Modus
overlay-permission-rule-item-mode-detail = Wählen Sie aus, ob passende Aktionen erlaubt, angefordert oder abgelehnt werden sollen.
overlay-permission-rule-item-scope = Umfang
overlay-permission-rule-item-scope-detail = Behalten Sie diese Regel für die Sitzung, den Arbeitsbereich oder global bei.
overlay-permission-rule-item-session-id = Sitzungs-ID
overlay-permission-rule-item-session-id-detail = Zielsitzungs-ID, die verwendet wird, wenn „scope=session“.
overlay-permission-rule-item-tool-name = Werkzeugname
overlay-permission-rule-item-tool-name-detail = Genauer passender Werkzeugname.
overlay-permission-rule-item-qualifier = Qualifikant
overlay-permission-rule-item-qualifier-detail = Optionaler Qualifizierer für spezifischere Werkzeugregeln.
overlay-permission-rule-item-access-kind = Zugriffsart
overlay-permission-rule-item-access-kind-detail = Wählen Sie „Lesen“, „Schreiben“ oder „read_write“.
overlay-permission-rule-item-target-path = Zielpfad
overlay-permission-rule-item-target-path-detail = Pfadmuster oder genauer Pfad zum Schutz.
overlay-permission-rule-item-workspace-root = Arbeitsbereichsstamm
overlay-permission-rule-item-workspace-root-detail = Optionales Basisverzeichnis, das zur Interpretation relativer Zielpfade verwendet wird.
overlay-permission-rule-item-network-target = Netzwerkziel
overlay-permission-rule-item-network-target-detail = Passender Host, Host:Port oder URL-Ziel.
overlay-permission-rule-detail-subject-kind = Werkzeugregeln stimmen nach Werkzeugname und optionalem Qualifikationsmerkmal überein. Pfadregeln entsprechen dem Dateisystemzugriff. Netzwerkregeln entsprechen dem Host- oder URL-Zugriff.
overlay-permission-rule-detail-tool-name = Werkzeugregeln erfordern einen genauen Werkzeugnamen, zum Beispiel `shell`, `read` oder `web_search`.
overlay-permission-rule-detail-qualifier = Der Qualifizierer ist optional. Lassen Sie es leer, es sei denn, das Werkzeug oder die Aktion erfordert eine engere Übereinstimmung.
overlay-permission-rule-detail-path-access-kind = Verwenden Sie `read`, `write` oder `read_write`, je nachdem, welchen Dateisystemzugriff Sie anpassen möchten.
overlay-permission-rule-detail-workspace-root = Lassen Sie „workspace_root“ leer, um das Stammverzeichnis des Laufzeit-Arbeitsbereichs zu erben. Legen Sie es explizit fest, wenn der geschützte Pfad anderswo liegt.
overlay-permission-rule-detail-target-path = Geben Sie einen Pfad oder ein Muster ein. Relative Pfade werden bei der Festlegung anhand von workspace_root interpretiert.
overlay-permission-rule-detail-network-target = Geben Sie einen Host, `host:port`, oder eine vollständige URL ein, je nachdem, wie spezifisch die Regel sein soll.
overlay-permission-rule-detail-scope = Der Sitzungsbereich eignet sich am besten für vorübergehende Außerkraftsetzungen. Arbeitsbereiche und globale Bereiche bleiben länger bestehen.
overlay-permission-rule-detail-session-id = Sitzungsbezogene Regeln erfordern eine konkrete Sitzungs-ID.
overlay-permission-rule-detail-mode = „Zulassen“ lässt die Aktion durch, bittet um Genehmigung und „Verweigern“ blockiert sie.
overlay-workbench-details = Einzelheiten
overlay-permission-studio-title = Erlaubnis
overlay-permission-studio-footer-nested = Ctrl+N hinzufügen · Enter bearbeiten · Ctrl+E umbenennen · Ctrl+D entfernen · Esc zurück
permission-studio-catalog-prompt = Durchsuchen Sie den Live-Tool-Katalog. Wählen Sie einen oder mehrere Einträge aus oder wählen Sie „Benutzerdefinierte Regel“ für einen Wert, der derzeit nicht registriert ist.
permission-studio-catalog-custom-detail = Fügen Sie einen Tag- oder Werkzeugnamen hinzu, der nicht im aktuellen Live-Katalog enthalten ist.
flash-permission-studio-catalog-empty = Wählen Sie mindestens einen Eintrag aus, bevor Sie Regeln hinzufügen.
overlay-runtime-setting-current-value = Aktuelle Überschreibung: { $value }
overlay-settings-help-string = Geben Sie Text ein. Lassen Sie das Feld leer oder geben Sie `clear` ein, um die Dateiüberschreibung zu entfernen.
overlay-settings-help-bool = Geben Sie wahr/falsch, ein/aus, ja/nein oder 1/0 ein. Lassen Sie das Feld leer oder geben Sie `clear` ein, um die Dateiüberschreibung zu entfernen.
overlay-settings-help-integer = Geben Sie eine ganze Zahl ein. Lassen Sie das Feld leer oder geben Sie `clear` ein, um die Dateiüberschreibung zu entfernen.
overlay-settings-help-float = Geben Sie eine Zahl ein. Lassen Sie das Feld leer oder geben Sie `clear` ein, um die Überschreibung zu entfernen.
overlay-choice-clear-value = Klarer Wert
overlay-settings-section-plugins-description = Konfigurieren Sie Plugins, überprüfen Sie deren Tools und Diagnosefunktionen und verwalten Sie Browser-, Shell- und Editor-Kabelbäume.
overlay-settings-section-providers-description = Konfigurieren Sie Anbieter und deren Netzwerkverhalten und überprüfen Sie den Modellkatalog.
overlay-settings-section-model-catalog-description = Durchsuchen Sie den aufgelösten Modellkatalog, überprüfen Sie die Modellmetadaten und aktualisieren Sie den lokalen Cache.
overlay-settings-section-permissions-description = Bearbeiten Sie globale Berechtigungen, Arbeitsbereichsberechtigungen und Berechtigungen für die aktuelle Sitzung separat.
overlay-settings-section-runtime-session-description = Konfigurieren Sie Kompatibilitäts-Clientversionen und das Verhalten der automatischen Sitzungskomprimierung.
settings-permission-effective-detail = Schreibgeschützt · Zusammengeführt aus Global, Arbeitsbereich und Sitzung.
settings-permission-effective-read-only = Die effektive Berechtigung ist schreibgeschützt; Bearbeiten Sie stattdessen die Sitzung, den Arbeitsbereich oder die globale Quelle.
settings-field-permission-approval-model-description = Modell- und Denk-/Geschwindigkeitsvarianten, die für automatische Berechtigungsentscheidungen verwendet werden; Nicht verfügbare Auswahlmöglichkeiten fallen auf „Fragen“ zurück
settings-field-tui-color-scheme-description = Erkennen Sie automatisch den Hintergrund des Terminals oder erzwingen Sie eine helle oder dunkle Palette
settings-field-tui-graphics-description = Zeigen Sie Bilder an und setzen Sie Formeln mit Kitty, Sixel oder iTerm2, sofern unterstützt; Änderungen werden nach einem Neustart der TUI wirksam
settings-field-activity-default-expanded-description = Standarderweiterungsstatus für Aktivitäten ohne artspezifische Überschreibung. Die Argumentation bleibt erweitert, es sei denn, ihre Art wird explizit festgelegt.
settings-activity-kind-reasoning-description = Der vollständige Gedankengang des Modells. Standardmäßig ist es erweitert und kann pro Art reduziert werden.
runtime-setting-choice-supported-model = vom aktuellen Modell unterstützt
settings-plugin-workbench-detail = Öffnen Sie die strukturierte Plugin-Workbench für Laufzeitstatus, Konfiguration, Tools, Operationen, Protokolle und Diagnosen.
settings-mcp-server-detail = Schalten Sie die Live-HTTP-MCP-Oberfläche von Agena um. Der angeschlossene Agena-Serverprozess bleibt die eigentliche Laufzeit.
settings-mcp-auth-detail = Wechseln Sie zwischen No-Auth, Full OAuth und ChatGPT Mixed-Auth. Im gemischten Modus bleiben die Initialisierung und die Tool-Erkennung öffentlich; Toolaufrufe bleiben OAuth-geschützt, es sei denn, der anonyme Zugriff wird explizit aktiviert.
settings-mcp-anonymous-access-none-detail = Sichere Standardeinstellung: Kein Werkzeugaufruf ist anonym. ChatGPT kann den Katalog weiterhin initialisieren und erkennen, bevor er sich anmeldet.
settings-mcp-anonymous-access-read-only-detail = Opt-in mit hohem Risiko: Nur-Lese-Tools können anonym ausgeführt werden und können private Arbeitsbereichs-, Dateisystem-, Konfigurations- oder Diagnosedaten offenlegen.
settings-mcp-anonymous-access-inactive-detail = Diese Richtlinie gilt nur im gemischten Authentifizierungsmodus; Schalten Sie die Authentifizierung auf gemischt um, um sie zu verwenden.
settings-mcp-client-registration-cimd-detail = Akzeptieren Sie nur OpenAI ChatGPT-Client-ID-Metadatendokumente. Der nicht authentifizierte öffentliche DCR-Endpunkt bleibt deaktiviert.
settings-mcp-client-registration-dcr-detail = Kompatibilitätsmodus: Stellen Sie auch die öffentliche dynamische Client-Registrierung bereit. Nur aktivieren, wenn ein Client CIMD nicht verwenden kann.
settings-mcp-public-url-detail = Legen Sie die kanonische HTTPS-MCP-Ressourcen-URL fest. Sichere MCP-Tunnel-URLs können den vollständigen /v1/mcp/tunnel_id-Pfad enthalten; Weitergeleitete Anforderungsheader werden niemals als OAuth-Identität vertrauenswürdig.
settings-mcp-oauth-issuer-detail = Legen Sie den öffentlichen, browserseitigen Autorisierungsserver-Aussteller fest. Von Agena verwaltetes OAuth erfordert einen Ursprung ohne Pfad, z. B. https://auth.example.com; Lassen Sie es leer, wenn OAuth und MCP dieselbe Domäne verwenden.
settings-mcp-oauth-password-detail = Legen Sie das auf der Agena OAuth-Autorisierungsseite angezeigte Passwort fest. Es wird vom Server als Argon2-Hash gespeichert.
settings-mcp-oauth-password-clear-detail = Entfernen Sie das MCP-spezifische Passwort und greifen Sie auf das Server-UI-Passwort zurück, sofern konfiguriert.
settings-field-runtime-codex-version-description = Genaue @openai/codex-Kompatibilitätsversion, die in den Headern der Anbieteranforderungsidentität verwendet wird.
settings-field-runtime-claude-version-description = Genaue @anthropic-ai/claude-code-Kompatibilitätsversion, die in Identitätsheadern der Anbieteranforderung verwendet wird.
settings-field-runtime-gemini-version-description = Genaue @google/gemini-cli-Kompatibilitätsversion, die in den Headern der Anbieteranforderungsidentität verwendet wird.
settings-field-session-compaction-auto-description = Sitzungen automatisch komprimieren, wenn sie sich der Kontextfenstergrenze nähern.
settings-field-session-compaction-reserved-tokens-description = Im Kontextfenster reservierte Token bei der Entscheidung, wann komprimiert werden soll; Es ist klar, dass der berechnete Standardwert verwendet werden soll.
settings-client-versions-refresh-description = Rufen Sie die neuesten kompatiblen Paketversionen von npm ab, behalten Sie alle drei genauen Werte bei und laden Sie die Laufzeit neu.
settings-client-versions-entry-detail = Öffnen Sie die genauen Kompatibilitätsversionen, die in den Headern der Anbieteranforderungsidentität verwendet werden.
settings-client-versions-section-description = Genaue Kompatibilitätsversionen, die in den Headern der Anbieteranforderungsidentität verwendet werden. Bearbeiten Sie jeden Wert oder drücken Sie Ctrl+R, um von npm zu aktualisieren.
settings-provider-workbench-detail = Öffnen Sie die durchsuchbare Anbieterliste, bevor Sie Authentifizierung, Adapter, Modellrouting oder neue Anbieter konfigurieren.
settings-provider-new-detail = Erstellen Sie einen neuen Anbieter, listen Sie Live-Adaptermodelle auf und bearbeiten Sie die Konfiguration des Anbieteradapters. Wählen Sie das Modell separat aus.
settings-model-catalog-open-detail = Überprüfen Sie die Metadaten des aufgelösten Modells und aktualisieren Sie den Cache des lokalen Modellkatalogs.
permission-studio-command-rules-shell-only = Befehlsregeln gelten nur für das kanonische Shell-Tool (agena.shell.run); Verwenden Sie eine Namensregel oder die Standardeinstellung für andere Tools.
permission-studio-detail-editable = Enter öffnet einen mehrzeiligen JSON-Editor für dieses Berechtigungssegment.
permission-studio-detail-add-hint = Enter erstellt dieses Element und öffnet es sofort.
permission-studio-detail-full-config-editable = Enter öffnet den erweiterten JSON-Editor für das gesamte Dokument.
overlay-permission-studio-delete-title = Regel löschen
overlay-permission-studio-delete-body = Löschen Sie { $kind }: { $value }
flash-permission-studio-no-add = Im aktuellen Abschnitt kann kein Element hinzugefügt werden.
flash-permission-studio-no-delete = Im aktuellen Abschnitt kann kein Element gelöscht werden.
flash-permission-studio-no-selection = Wählen Sie zunächst einen Artikel aus.
flash-permission-studio-context-lost = Der Kontext des Berechtigungseditors ging verloren. Öffnen Sie das Permission Studio erneut und versuchen Sie es erneut.
value-default = Standard
value-none = keine
value-clear = klar
value-path = Pfad
value-network = Netzwerk
value-workspace = Arbeitsbereich
value-external = extern
value-permission-filesystem = Dateisystem
value-permission-network = Netzwerk
value-permission-tools = Werkzeuge
value-rule-count = { $count } Regel(n)
value-custom = Brauch
value-internet = Internet
value-private = privat
value-loopback = Loopback
value-name-count = { $count } Name(n)
value-rule-set-count = { $count } Regelsatz(e)
value-open = offen
composer-prompt-history-title = Prompt-Geschichte
overlay-commands-title = Befehlspalette
overlay-commands-prompt = Suchaktionen; Befehle, die Text benötigen, werden im Composer fortgesetzt
overlay-skill-studio-title = Fähigkeiten verwalten
overlay-lineage-title = Filialverlauf [#{ $session }]
overlay-lineage-prompt = Erkunden Sie den aktuellen Zweigbaum und springen Sie zu einer Vorfahren-, Geschwister- oder untergeordneten Sitzung
overlay-rewind-title = Sitzung zurückspulen [#{ $session }]
overlay-rewind-prompt = Wählen Sie die Benutzernachricht aus, die Sie zurückziehen möchten, sowie alles, was danach folgt
overlay-picker-loading = Laden...
overlay-picker-empty = Keine passenden Artikel
overlay-picker-footer = Mit der Tabulatortaste wird das ausgewählte Etikett gefüllt
session-model-context-window = { $value } ctx
session-model-max-output = aus { $value }
overlay-provider-studio-detail-footer = Pfeiltasten auswählen · Eingabetaste bearbeiten · Esc zurück; Authentifizierungsaktionen sind auf der Hauptseite des Anbieters sichtbar
overlay-provider-studio-configured-disk = auf Festplatte konfiguriert; nicht Teil des aktuellen Authentifizierungsvertrags
overlay-provider-studio-new-model-prompt = Geben Sie die Modell-ID ein, die unter dem ausgewählten Adapter hinzugefügt werden soll.
provider-field-provider-id = Anbieter-ID
provider-field-auth-mode = Authentifizierungsmodus
provider-field-auth-subtype = Auth-Subtyp
provider-field-auth-login-method = Auth-Anmeldemethode
provider-field-start-auth = Authentifizierung starten
provider-field-continue-auth = Weiter Auth
provider-field-auth-details = Authentifizierungsdetails
provider-field-base-url = Basis-URL
provider-field-instance-url = Instanz-URL
provider-field-api-key-source = API-Schlüsselquelle
provider-field-api-key-value = API-Schlüsselwert
provider-field-redirect-uri = Umleitungs-URI
provider-field-callback-url = Rückruf-URL
provider-field-refresh-token = Aktualisierungstoken
provider-field-access-token = Zugriffstoken
provider-field-expires-at-ms = Läuft ab um (ms)
provider-field-account-id = Konto-ID
provider-field-enterprise-domain = Unternehmensdomäne
provider-field-region = Region
provider-field-profile = Profil
provider-field-access-key-id = Zugriffsschlüssel-ID
provider-field-secret-access-key = Geheimer Zugangsschlüssel
provider-field-session-token = Sitzungstoken
provider-field-service-key-env = Dienstschlüsselumgebung
provider-field-request-timeout = Anforderungszeitlimit (Sekunden)
provider-field-connect-timeout = Verbindungs-Timeout (Sekunden)
provider-field-adapter-id = Adapter-ID
provider-field-model-id = Modell-ID
provider-model-field-model-id = Modell-ID
provider-model-field-enabled = Aktiviert
provider-model-field-native-compaction = Native Verdichtung
provider-model-field-agena-tool-mode = Werkzeugmodus (agena_tools.mode)
agena-tool-mode-provider-protocol-label = Provider_Protokoll
agena-tool-mode-provider-protocol-detail = Transportieren Sie von Agena verwaltete Tool-Definitionen und -Aufrufe über das Tool-Protokoll der Anbieter-API.
agena-tool-mode-disabled-label = deaktiviert
agena-tool-mode-disabled-detail = Setzen Sie diesem Modell keine von Agena verwalteten oder anbieternativen Tools aus.
provider-model-field-display-name = Anzeigename
provider-model-field-lifecycle = Lebenszyklus
provider-model-field-context-window = Kontextfenster
provider-model-field-max-input = Maximaler Eingang
provider-model-field-max-output = Maximale Leistung
provider-model-field-features = Funktionen
provider-model-field-input-modalities = Eingabemodalitäten
provider-model-field-output-modalities = Ausgabemodalitäten
provider-model-field-thinking-modes = Denkmodi
provider-model-field-speed-modes = Geschwindigkeitsmodi
provider-model-field-description = Beschreibung
provider-model-enabled-detail = Ob diese Modellroute aktiviert ist.
provider-model-native-compaction-detail = Probieren Sie den nativen Konversationskomprimierungsendpunkt dieses Anbieters aus, bevor Sie auf die Textzusammenfassung von Agena zurückgreifen.
provider-model-lifecycle-detail = Modelllebenszykluswert.
provider-auth-mode-none-detail = Deaktivieren Sie die Metadaten der Anbieterauthentifizierung
provider-auth-mode-api-detail = Authentifizierung im API-Stil mit einem Untertyp der zweiten Stufe für benutzerdefinierte HTTP-Endpunkte, Cline API, GitLab-Gateway-Tokens oder Bedrock SigV4
provider-auth-mode-credential-detail = Anmeldeinformationsgestützte Authentifizierung, gelöst von einem lokalen Aussteller, ausgewählt im Feld „Authentifizierungsuntertyp“.
provider-auth-kind-unset = nicht gesetzt
provider-auth-kind-none = keine
provider-auth-kind-api = API
provider-auth-kind-cline = cline_api
provider-auth-kind-gitlab = gitlab_api
provider-auth-kind-credential = Berechtigung
provider-auth-kind-credential-with-issuer = Anmeldeinformationen:{ $issuer }
provider-auth-kind-bedrock = bedrock_sigv4
provider-auth-subtype-custom-label = Brauch
provider-auth-subtype-custom-detail = Allgemeiner API-Schlüssel + Basis-URL-Authentifizierung für OpenAI-kompatible, Anthropic- oder Gemini-HTTP-Anbieter
provider-auth-subtype-cline-api-detail = Cline-API-Endpunkt behoben; Es ist nur die Eingabe eines API-Schlüssels erforderlich und die Modellerkennung verwendet von Cline empfohlene Modelle
provider-api-key-source-inline-detail = Speichern Sie den API-Schlüssel inline in der Anbieterkonfiguration
provider-api-key-source-env-detail = Lesen Sie den API-Schlüssel aus einer Umgebungsvariablen
provider-auth-subtype-gitlab-api-detail = Die GitLab-Token-Authentifizierung wird über OpenAI- oder Anthropic-Adapter weitergeleitet
provider-auth-subtype-bedrock-detail = AWS Bedrock SigV4-Signierung
provider-auth-login-kind-browser-label = Browser-OAuth
provider-auth-login-kind-device-label = Gerätecode-Anmeldung
provider-auth-login-kind-browser-detail = Öffnen Sie die Autorisierungs-URL und beenden Sie dann den umgeleiteten Rückruf.
provider-auth-login-kind-device-detail = Öffnen Sie eine kurze Bestätigungs-URL, geben Sie einen Gerätecode ein und führen Sie dann eine Umfrage durch.
provider-issuer-openai-chatgpt-label = openai_chatgpt
provider-issuer-github-copilot-label = github_copilot
provider-issuer-gitlab-label = Gitlab
provider-issuer-google-adc-label = google_adc
provider-issuer-sap-ai-core-label = sap_ai_core
provider-issuer-openai-chatgpt-detail = OpenAI ChatGPT-Anmeldeinformationen
provider-issuer-github-copilot-detail = GitHub-Copilot-Anmeldeinformationen
provider-issuer-gitlab-detail = GitLab OAuth-Anmeldeinformationen
provider-issuer-google-adc-detail = Standardanmeldeinformationen für die Google-Anwendung
provider-issuer-sap-ai-core-detail = Authentifizierung des SAP AI Core-Dienstschlüssels
provider-instance-url-gitlab-detail = GitLab.com-Browser-OAuth-Endpunkt
provider-redirect-local-copy-detail = Localhost-Rückruf-URL zum Kopieren/Einfügen von OAuth-Weiterleitungen
provider-region-choice-detail = AWS-Region
provider-service-key-env-detail = Standardumgebungsvariable des SAP AI Core-Dienstschlüssels
overlay-model-catalog-field-model-id = Modell-ID
overlay-model-catalog-field-display = Anzeige
overlay-model-catalog-field-origin = Herkunft
overlay-model-catalog-field-lifecycle = Lebenszyklus
overlay-model-catalog-field-dates = Termine
overlay-model-catalog-field-limits = Grenzen
overlay-model-catalog-field-inputs = Eingaben
overlay-model-catalog-field-output = Ausgabe
overlay-model-catalog-field-features = Funktionen
overlay-model-catalog-field-modes = Modi
overlay-model-catalog-field-defaults = Standardeinstellungen
overlay-model-catalog-field-runtime = Laufzeit
overlay-model-catalog-field-pricing = Preise
overlay-model-catalog-field-source = Quelle
overlay-model-catalog-limits = ctx { $context } · rein { $input } · raus { $output }
overlay-model-catalog-lifecycle-active = aktiv
overlay-model-catalog-lifecycle-preview = Vorschau
overlay-model-catalog-lifecycle-beta = Beta
overlay-model-catalog-lifecycle-alpha = Alpha
overlay-model-catalog-lifecycle-experimental = experimentell
overlay-model-catalog-lifecycle-deprecated = veraltet
overlay-model-catalog-date-release = Veröffentlichung { $value }
overlay-model-catalog-date-updated = aktualisiert { $value }
overlay-model-catalog-date-cutoff = Cutoff { $value }
overlay-model-catalog-default-thinking = denke
overlay-model-catalog-default-speed = Geschwindigkeit
overlay-model-catalog-thinking-modes = Denkmodi
overlay-model-catalog-speed-modes = Geschwindigkeitsmodi
overlay-model-catalog-default-verbosity = Ausführlichkeit
overlay-model-catalog-default-temperature = Temp
overlay-model-catalog-default-top-p = top_p
overlay-model-catalog-default-top-k = top_k
overlay-model-catalog-parallel-tools = parallele Werkzeuge
overlay-model-catalog-supports-verbosity = Ausführlichkeit
overlay-model-catalog-reasoning-interleaved = verschachteltes Denken
overlay-model-catalog-reasoning-field = Argumentationsfeld
overlay-model-catalog-open-weights = offene Gewichte
overlay-model-catalog-price-input = in { "$" }{ $value }/M
overlay-model-catalog-price-output = aus { "$" }{ $value }/M
overlay-model-catalog-price-cache-read = Cache lesen { "$" }{ $value }/M
overlay-model-catalog-price-cache-write = Cache schreiben { "$" }{ $value }/M
overlay-model-catalog-tier-count = { $count } Stufe(n)
permission-rule-label-path = { $access } · { $path }
permission-rule-label-network = Netzwerk · { $target }
value-unset = nicht gesetzt
value-auto = auto
value-allow = erlauben
value-ask = fragen
value-deny = leugnen
value-read = lesen
value-write = schreiben
value-read-write = read_write
value-yes = ja
value-no = Nein
value-session = Sitzung
value-global = global
value-add = Hinzufügen
value-runtime-default = Laufzeitstandard
value-permission-rule-subject-tool = Werkzeug
value-permission-rule-subject-path-access = path_access
value-permission-rule-subject-network-access = Netzwerkzugriff
inline-fact-source = Quelle
inline-fact-scope = Umfang
inline-fact-operator = Betreiber
flash-permission-rule-saved = gespeicherte Berechtigungsregel: { $name }
flash-permission-rule-revoked = widerrufene Berechtigungsregel: { $name }
flash-permission-rule-context-lost = Der Kontext des Berechtigungsregelstudios ging verloren
flash-provider-studio-context-lost = Der Provider-Konfigurationskontext ging verloren
permission-rule-error-session-id-integer = Die Sitzungs-ID muss eine Ganzzahl sein
permission-rule-error-tool-name-required = Werkzeugregeln erfordern einen Werkzeugnamen
permission-rule-error-path-access-kind-required = Pfadregeln erfordern path_access_kind
permission-rule-error-target-path-required = Pfadregeln erfordern target_path
permission-rule-error-network-target-required = Netzwerkregeln erfordern ein Netzwerkziel
permission-rule-error-session-id-required = Der Sitzungsbereich erfordert eine Sitzungs-ID
flash-server-config-edit-in-settings = Die Konfigurationsdatei gehört zum Server. Bearbeiten Sie die Werte in den Einstellungen, anstatt einen lokalen Clientpfad zu öffnen.
flash-command-requires-session = Für diese Aktion ist eine offene Sitzung erforderlich
flash-session-busy = Die Sitzung ist beschäftigt
flash-provider-not-found = Anbieter nicht gefunden: { $provider }
flash-permission-approval-model-updated = Automatisches Genehmigungsmodell aktualisiert: { $provider }/{ $model }
flash-global-default-model-updated = Globales Standardmodell aktualisiert: { $provider }/{ $model }
flash-provider-studio-adapter-required = Wählen Sie zunächst einen Adapter aus
flash-provider-studio-adapter-not-enabled = Überprüfen Sie den ausgewählten Adapter, bevor Sie ein Modell hinzufügen
flash-provider-studio-adapter-unavailable = Der aktuelle Authentifizierungsmodus erlaubt die Auswahl dieses Adapters nicht
flash-provider-studio-model-required = Wählen Sie zunächst ein aufgelistetes Modell aus
flash-provider-studio-model-id-required = Modell-ID ist erforderlich
flash-provider-studio-no-auth-details = Für den aktuellen Authentifizierungsmodus sind keine Authentifizierungsdetails verfügbar
flash-provider-studio-catalog-refreshed = aktualisierter Modellkatalog
flash-provider-studio-invalid-model-json = Ungültiges Modell-JSON: { $error }
flash-provider-studio-live-listing-unavailable = Die Auflistung der Live-Modelle ist für die Autorisierung { $auth } nicht verfügbar.
flash-provider-studio-draft-listing-unsupported = Die Entwurfsmodellauflistung unterstützt nur Adapter mit Live-Modellerkennung. Nicht unterstützt: { $adapters }
flash-provider-studio-listing-auth-required = Das Auflisten von Adaptermodellen erfordert eine Live-Modellerkennung für das aktuelle Authentifizierungs-/Adapterpaar oder einen vorhandenen gespeicherten Anbieter. Die aktuelle Authentifizierung ist { $auth }
flash-provider-studio-invalid-auth-login-method = Ungültige Authentifizierungs-Anmeldemethode
flash-provider-auth-openai-browser-started = Die OpenAI-Browserauthentifizierung wurde gestartet. Öffnen Sie die im Dialogfeld angezeigte Autorisierungs-URL, fügen Sie dann die umgeleitete URL in „Rückruf-URL“ ein und drücken Sie p.
flash-provider-auth-openai-device-started = Die Anmeldung am OpenAI-Gerät wurde gestartet. Öffnen Sie die im Dialogfeld angezeigte Verifizierungs-URL, geben Sie den Code { $code } ein und drücken Sie dann p.
flash-provider-auth-copilot-device-started = Die Anmeldung am Copilot-Gerät wurde gestartet. Öffnen Sie die im Dialogfeld angezeigte Verifizierungs-URL, geben Sie den Code { $code } ein und drücken Sie dann p.
flash-provider-auth-gitlab-browser-started = Die GitLab-Browserauthentifizierung wurde gestartet. Öffnen Sie die im Dialogfeld angezeigte Autorisierungs-URL, fügen Sie dann die umgeleitete URL in „Rückruf-URL“ ein und drücken Sie p.
flash-provider-auth-atomgit-browser-started = Die AtomGit-Browserauthentifizierung wurde gestartet. Öffnen Sie die im Dialogfeld angezeigte Autorisierungs-URL, schließen Sie die Anmeldung ab und drücken Sie dann p, um abzufragen.
flash-provider-auth-openai-captured = Im Entwurf erfasste OpenAI-OAuth-Anmeldeinformationen.
flash-provider-auth-openai-pending = Die Anmeldung am OpenAI-Gerät steht noch aus. Beenden Sie den Verifizierungsschritt und drücken Sie dann erneut p.
flash-provider-auth-copilot-pending = Die Anmeldung am Copilot-Gerät steht noch aus. Schließen Sie die Browsergenehmigung ab und drücken Sie dann erneut p.
flash-provider-auth-copilot-captured = Im Entwurf erfasste Copilot-OAuth-Anmeldeinformationen.
flash-provider-auth-gitlab-captured = Im Entwurf erfasste GitLab-OAuth-Anmeldeinformationen.
flash-provider-auth-atomgit-pending = Die Anmeldung beim AtomGit-Browser steht noch aus. Beenden Sie den Browserablauf und drücken Sie dann erneut p.
flash-provider-auth-atomgit-captured = Im Entwurf erfasste AtomGit-OAuth-Anmeldeinformationen.
flash-provider-auth-error-unsupported = Der aktuelle Authentifizierungsmodus unterstützt keine interaktive OAuth-Anmeldung
flash-provider-auth-error-start-browser-first = Starten Sie zunächst die Browser-Authentifizierung mit Start Auth oder o
flash-provider-auth-error-start-device-first = Starten Sie zunächst die Geräteauthentifizierung mit Start Auth oder o
flash-provider-auth-error-required-field = { $field } ist erforderlich
flash-provider-save-draft = Gespeicherter Anbieter { $provider } mit Adapter { $adapter }.
flash-provider-save-adapter-matches = Gespeichert { $provider }/{ $adapter } mit { $listed } aufgelisteten Modell(en); { $matched } Katalog stimmt überein.
flash-provider-save-model = Gespeichert { $provider }/{ $adapter }/{ $model }.
flash-provider-save-configured-model = Gespeichertes konfiguriertes Modell { $provider }/{ $adapter }/{ $model }.
flash-provider-delete-provider = Gelöschter Anbieter { $provider }.
flash-provider-delete-adapter = Der konfigurierte Adapter { $provider }/{ $adapter } wurde gelöscht und die Modelle { $count } entfernt.
flash-provider-delete-model = Konfiguriertes Modell { $provider }/{ $adapter }/{ $model } gelöscht.
flash-provider-studio-adapter-delete-empty = Es sind keine Adaptereinstellungen zum Löschen ausgewählt.
flash-provider-save-error-required-field = { $field } ist erforderlich
flash-provider-save-error-unsupported-adapters = auth { $auth } unterstützt keine Adapter: { $adapters }; erwartet einen von { $supported }
flash-provider-save-error-api-base-url = Für die API-Authentifizierung ist base_url erforderlich, wenn das OpenAI-Protokoll, Anthropic- oder Gemini-Adapter verwendet werden
flash-provider-save-error-gitlab-token = Für die Authentifizierung von gitlab_api ist eine API-Schlüsselquelle erforderlich
flash-provider-save-error-credential-base-url = Der Aussteller der Anmeldeinformationen `{ $issuer }` erfordert base_url
flash-provider-save-error-credential-service-key-env = Der Aussteller der Anmeldeinformationen `{ $issuer }` erfordert service_key_env
flash-provider-save-error-bedrock-key-pair = bedrock_sigv4 erfordert access_key_id und Secret_access_key zusammen
flash-provider-save-error-select-model = Wählen Sie mindestens ein Modell aus, bevor Sie den Anbieter speichern
flash-provider-save-error-adapter-object = Der Anbieteradapter `{ $adapter }` muss ein JSON-Objekt sein
flash-provider-save-error-model-object = Die Anbietermodellkonfiguration muss ein JSON-Objekt sein
flash-provider-save-error-configured-adapter-object = Die konfigurierten Provider-Adaptereinstellungen müssen ein JSON-Objekt sein
flash-provider-save-error-configured-models-object = Die konfigurierten Anbieteradaptermodelle müssen ein JSON-Objekt sein
flash-provider-client-versions-refreshed = Aktualisierte Client-Versionen: Codex { $codex }, Claude { $claude }, Gemini { $gemini }
terminal-diagnostics-title = Terminaldiagnose
terminal-diagnostics-eyebrow = Kompatibilitäts- und Protokollnachweise
terminal-diagnostics-footer = ↑/↓ scrollen · c/y Bericht kopieren · Esc schließen
terminal-diagnostics-tip = Produktidentitäts- und Umgebungsebenen sind evidenzbasiert; Generisches SSH kann das tatsächliche Endpunktterminal nicht nachweisen.
terminal-diagnostics-copied = Terminaldiagnose kopiert
terminal-diagnostics-unavailable = Die Terminaldiagnose ist in dieser Laufzeit nicht verfügbar.
terminal-diagnostics-summary = Evidenzgestützter Terminalbericht · Endpunktvertrauen { $confidence }
terminal-diagnostics-none = keine
terminal-diagnostics-unknown = unbekannt
terminal-diagnostics-unavailable-value = nicht verfügbar
terminal-diagnostics-term-unset = TERM ist nicht festgelegt
terminal-diagnostics-section-identity = Identität
terminal-diagnostics-section-layers = Umgebungsebenen
terminal-diagnostics-section-color = Farbe und Aussehen
terminal-diagnostics-section-protocols = Aktive Protokolle
terminal-diagnostics-section-providers = Anbieter und Integrationen
terminal-diagnostics-section-warnings = Warnungen
terminal-diagnostics-field-product = Produkt
terminal-diagnostics-field-version = Version
terminal-diagnostics-field-parsed-version = Geparste Version
terminal-diagnostics-field-compatibility = Kompatibilität
terminal-diagnostics-field-confidence = Vertrauen
terminal-diagnostics-field-source = Ausgewählte Quelle
terminal-diagnostics-field-evidence = Beweise
terminal-diagnostics-field-conflicts = Konflikte
terminal-diagnostics-color-configured = Konfigurierter Modus
terminal-diagnostics-color-detected-background = Hintergrund erkannt
terminal-diagnostics-color-detected-appearance = Erkanntes Aussehen
terminal-diagnostics-color-source = Erkennungsquelle
terminal-diagnostics-color-refresh = Automatische Aktualisierung
terminal-diagnostics-color-generation = Erscheinungserzeugung
terminal-diagnostics-color-effective-appearance = Effektive Textpalette
terminal-diagnostics-color-formula-foreground = Farbe der Formelglyphe
terminal-diagnostics-color-formula-background = Formelbildhintergrund
terminal-diagnostics-color-background-images = Hintergrundbilder
terminal-diagnostics-color-mode-auto = Automatisch
terminal-diagnostics-color-mode-dark = Zwangsdunkel
terminal-diagnostics-color-mode-light = Zwangslicht
terminal-diagnostics-color-appearance-dark = Dunkel
terminal-diagnostics-color-appearance-light = Licht
terminal-diagnostics-color-appearance-unknown = Unbekannt
terminal-diagnostics-color-appearance-conservative = Konservative terminal-native Farben (Hintergrund unbekannt)
terminal-diagnostics-color-source-osc11 = Antwort des OSC 11-Terminals
terminal-diagnostics-color-source-iterm-osc4 = Antwort des iTerm2 OSC 4;-2-Terminals
terminal-diagnostics-color-source-colorfgbg = COLORFGBG-Umgebungs-Fallback
terminal-diagnostics-color-source-term-background = TERM_BACKGROUND-Umgebungs-Fallback
terminal-diagnostics-color-source-vscode-theme = VSCODE_THEME_KIND-Umgebungs-Fallback
terminal-diagnostics-color-source-unavailable = Keine verwendbaren Terminal- oder Umgebungsnachweise
terminal-diagnostics-color-refresh-live = Bei der Wiederherstellung des Fokus und der endgültigen Wiederaufnahme; Bei fehlgeschlagenen Aktualisierungen wird die letzte bekannte Farbe beibehalten
terminal-diagnostics-color-refresh-startup-only = Nur Startup; Das Terminal hat eine aktualisierbare Farbabfrage nicht beantwortet
terminal-diagnostics-color-formula-background-transparent = Transparent; Nur die Farbe der Formelglyphe folgt dem Aussehen
terminal-diagnostics-color-background-images-not-sampled = Nicht beprobt; Transparente Formelpixel bewahren den Terminalhintergrund oder das darunter liegende Hintergrundbild
terminal-diagnostics-direct = Direkt
terminal-diagnostics-direct-description = Es wurden keine SSH-, Mosh-, Multiplexer- oder WSL-Beweise gefunden.
terminal-diagnostics-layer-description = Erkannt aus { $source }. Ebenenreihenfolge und Verschachtelungstiefe sind unbekannt.
terminal-diagnostics-capability-description = Endpunkt={ $status } · Quelle={ $source } · Pfad={ $path } · Anbieter={ $provider }
terminal-diagnostics-path-clear = klar
terminal-diagnostics-path-forced = durch Override erzwungen
terminal-diagnostics-path-unverified = unbestätigt
terminal-diagnostics-path-blocked = blockiert
terminal-diagnostics-provider-not-required = nicht erforderlich
terminal-diagnostics-provider-ready = fertig
terminal-diagnostics-provider-missing = fehlen oder sind nicht implementiert
terminal-diagnostics-helper-missing = Nicht gefunden oder nicht ausführbar.
terminal-diagnostics-helper-not-probed = Nicht geprüft, da der Endpunkt nicht als Kitty identifiziert wird.
terminal-diagnostics-no-warnings = Es wurden keine Kompatibilitätswarnungen erkannt.
terminal-diagnostics-protocol-alternate-screen = Alternativer Bildschirm
terminal-diagnostics-protocol-bracketed-paste = Klammerpaste
terminal-diagnostics-protocol-focus = Fokusberichterstattung
terminal-diagnostics-protocol-mouse = Mauserfassung
terminal-diagnostics-protocol-mouse-mode = Mausdrahtmodus
terminal-diagnostics-protocol-mouse-events = Empfangene Mausereignisse
terminal-diagnostics-protocol-mouse-last = Letztes Mausereignis
terminal-diagnostics-mouse-mode-button-sgr = Schaltflächenereignisverfolgung (DECSET 1002) mit SGR-Koordinaten (DECSET 1006)
terminal-diagnostics-mouse-events-none = Keine. Das Endpunktterminal hat kein Mausereignis an Agena übermittelt; Überprüfen Sie die Profileinstellungen für Maus- und Rad-Berichte.
terminal-diagnostics-mouse-events-seen = { $count } Ereignis(e)
terminal-diagnostics-mouse-last-none = Keine
terminal-diagnostics-protocol-keyboard = Begriffsklärung der Tastatur
terminal-diagnostics-protocol-key-events = Tastaturereignistypen
terminal-diagnostics-protocol-background = Hintergrundabfrage
terminal-diagnostics-protocol-native-clipboard = Native Zwischenablage
terminal-diagnostics-protocol-osc52-write = OSC 52 schreiben
terminal-diagnostics-protocol-osc52-read = OSC 52 gelesen
terminal-diagnostics-protocol-progress = OSC 9;4 Fortschritt
terminal-diagnostics-provider-kitty-clipboard = Kitty-Zwischenablage
terminal-diagnostics-provider-kitty-transfer = Kitty-Transfer
terminal-diagnostics-provider-iterm-transfer = iTerm2-Übertragung
terminal-diagnostics-provider-inline-images = Inline-Bilder
terminal-diagnostics-provider-hyperlinks = Hyperlinks
terminal-diagnostics-provider-sync-output = Synchronisierte Ausgabe
terminal-diagnostics-status-confirmed = bestätigt
terminal-diagnostics-status-forced = durch Override erzwungen
terminal-diagnostics-status-profiled = profiliert
terminal-diagnostics-status-unsupported = nicht unterstützt
terminal-diagnostics-status-unknown = unbekannt
terminal-diagnostics-source-user = Benutzerüberschreibung
terminal-diagnostics-source-environment = Umgebung
terminal-diagnostics-source-helper = Hilfssonde
terminal-diagnostics-source-terminal-query = Terminalabfrage
terminal-diagnostics-source-profile = Terminalprofil
terminal-diagnostics-source-platform = Plattformstandard
terminal-diagnostics-source-conservative = konservativer Standard
terminal-diagnostics-source-terminfo = terminfo-Kompatibilität
terminal-diagnostics-source-unknown = unbekannt
terminal-diagnostics-confidence-explicit = explizit
terminal-diagnostics-confidence-strong = stark
terminal-diagnostics-confidence-compatibility = Nur Kompatibilität
terminal-diagnostics-confidence-unknown = unbekannt

# Plugin Workbench i18n completion
plugin-workbench-action-diff = Diff
plugin-workbench-action-refresh = Aktualisieren
plugin-workbench-action-remove-selected = Auswahl entfernen/zurücksetzen
plugin-workbench-action-reset-all = Alles zurücksetzen
plugin-workbench-action-restart = Neu starten
plugin-workbench-action-save = Speichern
plugin-workbench-action-validate = Validieren
plugin-workbench-actions = Aktionen
plugin-workbench-authority-unavailable = Berechtigungsdaten sind nicht verfügbar.
plugin-workbench-choices = Auswahl
plugin-workbench-close-footer = Esc schließen
plugin-workbench-column-after = Nachher
plugin-workbench-column-args = Arg.
plugin-workbench-column-arguments = Argumente
plugin-workbench-column-before = Vorher
plugin-workbench-column-category = Kategorie
plugin-workbench-column-change = Änderung
plugin-workbench-column-operation = Operation
plugin-workbench-column-description = Beschreibung
plugin-workbench-column-field = Feld
plugin-workbench-column-inputs = Eingaben
plugin-workbench-column-message = Meldung
plugin-workbench-column-plugin = Plugin
plugin-workbench-column-section = Abschnitt
plugin-workbench-column-severity = Schweregrad
plugin-workbench-column-source = Quelle
plugin-workbench-column-summary = Zusammenfassung
plugin-workbench-column-tool = Tool
plugin-workbench-column-version = Version
plugin-workbench-column-visible-tool = Sichtbares Tool
plugin-workbench-operation-arguments = Argumente: {$operation}
plugin-workbench-config = Konfiguration
plugin-workbench-config-action = Aktion
plugin-workbench-config-choose-shape = Form auswählen
plugin-workbench-config-choose-type = Typ auswählen
plugin-workbench-config-default = Standard
plugin-workbench-config-diff = Konfigurations-Diff
plugin-workbench-config-dirty = geändert
plugin-workbench-config-drilldown-footer = Links/Rechts Zelle · Oben/Unten Zeile · Enter bearbeiten · Ctrl+D entfernen/zurücksetzen · Esc zurück
plugin-workbench-config-saved = gespeichert
plugin-workbench-config-setting = Einstellung
plugin-workbench-config-state = Status
plugin-workbench-config-state-changed = überschrieben
plugin-workbench-config-state-default = Standard
plugin-workbench-config-state-dirty = geändert
plugin-workbench-config-state-error = Fehler
plugin-workbench-config-state-inactive = inaktiv
plugin-workbench-config-summary = {$status} · {$save_state}
plugin-workbench-config-title = {$plugin} / Konfiguration
plugin-workbench-config-type = Typ
plugin-workbench-config-value = Wert
plugin-workbench-config-view-summary = Effektive Konfiguration · {$changed} geänderte Felder · ausgewählte Zelle: {$cell}
plugin-workbench-detail-footer = Tab/Shift+Tab Abschnitt · Oben/Unten scrollen · Esc zurück
plugin-workbench-detail-tools-footer = Tab/Shift+Tab Abschnitt · Oben/Unten auswählen · Enter konfigurieren und ausführen · Esc zurück
plugin-workbench-filter-all = Alle
plugin-workbench-filter-other = andere
plugin-workbench-header-summary = Tools: {$tools}        Operationen: {$operations}        Konfiguration: {$config}
plugin-workbench-input-preview = Eingabevorschau: {$tool}
plugin-workbench-last-result-failed = Letztes Ergebnis · {$tool} · fehlgeschlagen
plugin-workbench-last-result-success = Letztes Ergebnis · {$tool} · erfolgreich
plugin-workbench-list-footer = Tippen zum Suchen · Oben/Unten auswählen · Enter öffnen · Esc schließen
plugin-workbench-list-summary = Plugins suchen… {$query}        Transport: {$transport}        Konfiguration: {$config}        {$shown}/{$total} angezeigt
plugin-workbench-loading-actions = Aktionen werden geladen…
plugin-workbench-loading-choices = Auswahl wird geladen…
plugin-workbench-no-changes = Keine Änderungen
plugin-workbench-no-operations = Keine Operationen.
plugin-workbench-no-config-section = Kein Konfigurationsabschnitt.
plugin-workbench-no-editable-rows = Keine bearbeitbaren Zeilen.
plugin-workbench-no-filter-matches = Keine Plugins entsprechen den aktuellen Filtern.
plugin-workbench-no-issues = Keine Probleme
plugin-workbench-no-logs = Keine Protokolle.
plugin-workbench-no-selection = Kein Plugin ausgewählt.
plugin-workbench-no-structured-arguments = Keine strukturierten Argumente.
plugin-workbench-no-tools = Keine Tools.
plugin-workbench-none = keine
plugin-workbench-none-declared = keine angegeben
plugin-workbench-overview = Übersicht
plugin-workbench-package-summary = Paket: {$package}
plugin-workbench-plugin = Plugin
plugin-workbench-plugin-capabilities = Plugin-Fähigkeiten
plugin-workbench-plugins = Plugins
plugin-workbench-provenance = Herkunft: {$provenance}
plugin-workbench-sections = Abschnitte
plugin-workbench-severity-error = Fehler
plugin-workbench-severity-warning = Warnung
plugin-workbench-status-invalid = Ungültig
plugin-workbench-status-issues = Probleme
plugin-workbench-status-missing = Fehlt
plugin-workbench-status-needs-restart = Neustart erforderlich
plugin-workbench-status-runtime-issue = Laufzeitproblem
plugin-workbench-status-schema-missing = Schema fehlt
plugin-workbench-status-valid = Gültig
plugin-workbench-status-warning = Warnung
plugin-workbench-summary = Abfrage: {$query} · Transport {$transport} · Konfiguration {$config} · {$shown}/{$total} angezeigt
plugin-workbench-tab-capabilities = Fähigkeiten
plugin-workbench-tab-operations = Operationen
plugin-workbench-tab-config = Konfiguration
plugin-workbench-tab-diagnostics = Diagnose
plugin-workbench-tab-logs = Protokolle
plugin-workbench-tab-tools = Tools
plugin-workbench-tabs = Registerkarten
plugin-workbench-tags-summary = Tags: {$tags}
plugin-workbench-tool-capabilities = Tool-Fähigkeiten
plugin-workbench-tools-help = Oben/Unten wählt ein Tool. Enter öffnet das vom Host bereitgestellte Schemaformular; Ctrl+S validiert und führt es aus.
plugin-workbench-transport = Transport
plugin-workbench-trust-level = Vertrauensstufe: {$level}
plugin-workbench-unavailable = nicht verfügbar


# Plugin Workbench structured editor i18n completion
plugin-workbench-editor-also-matches = passt auch zu: {$matches}
plugin-workbench-editor-array-action-help = Enter Aktionsmenü · Ctrl+D ausgewählte Zeile entfernen
plugin-workbench-editor-array-preview = Konfigurieren… ({$count} Elemente)
plugin-workbench-editor-configure = Konfigurieren…
plugin-workbench-editor-format = Format: {$format}
plugin-workbench-editor-generic-object = Allgemeiner Objekteditor
plugin-workbench-editor-index = Index
plugin-workbench-editor-item = Element {$index}
plugin-workbench-editor-map = Map-Editor
plugin-workbench-editor-no-fields = Keine Felder.
plugin-workbench-editor-no-items = Keine Elemente.
plugin-workbench-editor-object = Objekteditor
plugin-workbench-editor-object-action-help = Enter Aktionsmenü · Feld über die Aktionszelle hinzufügen
plugin-workbench-editor-object-array = Tabelleneditor für Objekt-Arrays
plugin-workbench-editor-object-array-help = Bearbeiten öffnet das ausgewählte Element im gleichen strukturierten Editor.
plugin-workbench-editor-object-preview = Konfigurieren… ({$count} Felder)
plugin-workbench-editor-preview = Vorschau
plugin-workbench-editor-primitive-array = Editor für primitive Arrays
plugin-workbench-editor-readonly = schreibgeschützt
plugin-workbench-editor-schema-missing = Schema fehlt        Einfacher strukturierter Editor
plugin-workbench-editor-shape = Form
plugin-workbench-editor-suggestions = Vorschläge
plugin-workbench-editor-tuple = Tupel-Editor
plugin-workbench-editor-type-summary = Typ: {$type}        Pfadeditor: strukturierte Oberfläche
plugin-workbench-field-state-available = verfügbar
plugin-workbench-field-state-custom = benutzerdefiniert
plugin-workbench-field-state-map-key = Map-Schlüssel
plugin-workbench-field-state-missing = fehlt
plugin-workbench-field-state-optional = optional
plugin-workbench-field-state-required = erforderlich
plugin-workbench-kind-all-of = allOf
plugin-workbench-kind-any-of = anyOf
plugin-workbench-kind-array = Array
plugin-workbench-kind-boolean = boolesch
plugin-workbench-kind-integer = Ganzzahl
plugin-workbench-kind-null = null
plugin-workbench-kind-number = Zahl
plugin-workbench-kind-object = Objekt
plugin-workbench-kind-one-of = oneOf
plugin-workbench-kind-string = Zeichenfolge
plugin-workbench-kind-value = Wert

overlay-provider-list-create-detail = Erstellen Sie einen Anbieterentwurf und konfigurieren Sie anschließend Authentifizierung, Adapter und Modelle.

overlay-provider-delete-body = Anbieter {$provider} und alle konfigurierten Adapter/Modelle löschen?

overlay-provider-delete-adapter-body = Konfigurierten Adapter {$provider}/{$adapter} löschen?

overlay-provider-delete-adapter-last-body = Dies ist der letzte konfigurierte Adapter. Bei Bestätigung wird der Anbieter gelöscht.

overlay-provider-delete-model-body = Konfiguriertes Modell {$provider}/{$adapter}/{$model} löschen?
