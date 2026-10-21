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
overlay-user-input-questions = Fragen
overlay-user-input-summary = Zusammenfassung
overlay-user-input-detail = Details
overlay-user-input-prompt-panel = Eingabe
overlay-user-input-choices = Auswahl
overlay-user-input-auto-resolve = Automatische Fortsetzung in {$remaining}
overlay-user-input-choice-single-short = Einzelauswahl
overlay-user-input-choice-multiple-short = Mehrfachauswahl
overlay-user-input-custom-input = Eigene Eingabe
overlay-user-input-custom-input-hint = Eigene Eingabe (e / einfügen)
overlay-user-input-submit = Absenden
overlay-user-input-review-intro = Prüfen Sie Ihre Antworten, bevor Sie sie absenden.
overlay-user-input-no-questions = Keine Fragen.
overlay-user-input-other = Sonstiges
overlay-user-input-unanswered = unbeantwortet
overlay-user-input-custom-empty = Leertaste drücken, um eine eigene Antwort einzugeben
overlay-user-input-review-feedback = Rückmeldung an den Agenten
overlay-user-input-review-feedback-empty = Enter oder e drücken, um Rückmeldung einzugeben oder eine Entscheidung auszuwählen
overlay-user-input-review-footer-hint = H J K L / Pfeile: bewegen · Enter: ausführen · Esc: Titel
overlay-user-input-question-footer-hint = ↑/↓ J/K: Optionen · ←/→ H/L: Fragen · Leertaste: auswählen · Enter: weiter / senden · Esc: Titel
overlay-user-input-missing-answer = Antwort für {$label} fehlt
overlay-user-input-footer-review = j/k ↑/↓ bewegen | Enter auswählen | e bearbeiten | Ctrl+Y/Ctrl+E zeilenweise scrollen | PgUp/PgDn blättern | Ctrl+X abbrechen
overlay-user-input-footer-question = Tab/Shift+Tab Seiten | Leertaste umschalten | Ctrl+D löschen | Ctrl+Y/Ctrl+E zeilenweise scrollen | PgUp/PgDn blättern | Ctrl+X abbrechen
overlay-user-input-wizard-keys = ←→ Seiten · Leertaste auswählen · Enter weiter / Zusammenfassung bestätigen
overlay-user-input-page = Frage { $current } / { $total }
overlay-user-input-previous = Zurück
overlay-user-input-next = Weiter
overlay-user-input-cancel = Abbrechen
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = benutzerdefinierter Wert erlaubt
overlay-user-input-reply-format = Antwortformat: 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X bricht die Anfrage ab
overlay-user-input-footer = Ctrl+X abbrechen

flash-terminal-event-error = Terminalereignisfehler: {$error}
flash-created-session = Sitzung erstellt {$title}
flash-permission-reply-sent = Berechtigungsantwort gesendet: {$label}
flash-user-input-reply-sent = Benutzereingabeantwort gesendet
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
message-question-line = - {$question}
message-part-detail-unavailable = Teildetails nicht verfuegbar
message-tool-pending = wartet: {$label}
message-tool-running = laeuft: {$label}
message-tool-done = fertig: {$label}
message-tool-failed = fehlgeschlagen: {$label}
message-tool-cancelled = abgebrochen: {$label}
message-tool-result-blocks = {$count} Ergebnisbloecke
message-permission = Berechtigung
message-tool-summary = [Werkzeug] {$status} | {$label}
message-tool-output-collapsed =     … {$lines} weitere
message-activity-run-collapsed = Enter: 5 laden · n Enter · Ctrl+Shift+Enter: alle laden · … {$count} ältere Teile ausgeblendet

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

overlay-settings-section-plugins-summary = Plugin-Konfiguration, Tools und Diagnose

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

settings-field-permission-approval-model-label = Globales automatisches Genehmigungsmodell

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

settings-activity-kind-skill_reference-label = Befehlsreferenzen

settings-activity-kind-skill_reference-description = Verweise auf die in der Antwort gewählten Befehle.

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

settings-global-default-model-label = Globales Standardmodell

settings-global-default-model-description = Modell für Sitzungen ohne eigene Modellangabe; ein explizites Sitzungsmodell hat immer Vorrang.
settings-workspace-default-model-label = Standardmodell des Arbeitsbereichs
settings-workspace-default-model-description = Im Projekt dieses Arbeitsbereichs gespeichertes Standardmodell; es überschreibt hier den globalen Standard.

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
overlay-provider-studio-catalog-picker-title = Katalogvorlage wählen

overlay-provider-studio-catalog-picker-footer = Hoch/Runter auswählen · Ctrl+F suchen · Links/Rechts blättern · Enter übernimmt Modellfelder · Esc zurück

overlay-provider-studio-catalog-match = Katalog: {$model}

overlay-provider-studio-catalog-selected = Gewählter Katalog: {$model}

overlay-provider-studio-catalog-loading = Katalogtreffer wird gesucht…

overlay-provider-studio-catalog-no-match = Kein Katalogtreffer · in den Modelleinstellungen wählen

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
overlay-settings-section-plugins-description = Konfigurieren Sie Plugins und prüfen Sie deren Tools und Diagnoseinformationen.
overlay-settings-section-providers-description = Konfigurieren Sie Anbieter und deren Netzwerkverhalten und überprüfen Sie den Modellkatalog.
overlay-settings-section-model-catalog-description = Durchsuchen Sie den aufgelösten Modellkatalog, überprüfen Sie die Modellmetadaten und aktualisieren Sie den lokalen Cache.
overlay-settings-section-permissions-description = Bearbeiten Sie globale Berechtigungen, Arbeitsbereichsberechtigungen und Berechtigungen für die aktuelle Sitzung separat.
overlay-settings-section-runtime-session-description = Konfigurieren Sie Kompatibilitäts-Clientversionen und das Verhalten der automatischen Sitzungskomprimierung.
settings-permission-effective-detail = Schreibgeschützt · Zusammengeführt aus Global, Arbeitsbereich und Sitzung.
settings-permission-effective-read-only = Die effektive Berechtigung ist schreibgeschützt; Bearbeiten Sie stattdessen die Sitzung, den Arbeitsbereich oder die globale Quelle.
settings-field-permission-approval-model-description = Modell- und Denk-/Geschwindigkeitsvarianten, die für automatische Berechtigungsentscheidungen verwendet werden; Nicht verfügbare Auswahlmöglichkeiten fallen auf „Fragen“ zurück
settings-workspace-approval-model-label = Automatisches Genehmigungsmodell des Arbeitsbereichs
settings-workspace-approval-model-description = Im Projekt dieses Arbeitsbereichs gespeichertes Genehmigungsmodell; es überschreibt hier die globale Auswahl.
settings-field-tui-color-scheme-description = Erkennen Sie automatisch den Hintergrund des Terminals oder erzwingen Sie eine helle oder dunkle Palette
settings-field-tui-graphics-description = Zeigen Sie Bilder an und setzen Sie Formeln mit Kitty, Sixel oder iTerm2, sofern unterstützt; Änderungen werden nach einem Neustart der TUI wirksam
settings-field-activity-default-expanded-description = Standarderweiterungsstatus für Aktivitäten ohne artspezifische Überschreibung. Die Argumentation bleibt erweitert, es sei denn, ihre Art wird explizit festgelegt.
settings-activity-kind-reasoning-description = Der vollständige Gedankengang des Modells. Standardmäßig ist es erweitert und kann pro Art reduziert werden.
runtime-setting-choice-supported-model = vom aktuellen Modell unterstützt
settings-plugin-workbench-detail = Öffnen Sie die strukturierte Plugin-Workbench für Laufzeitstatus, Konfiguration, Tools, Befehle, Protokolle und Diagnosen.
settings-mcp-server-detail = Schalten Sie die Live-HTTP-MCP-Oberfläche von Agena um. Der angeschlossene Agena-Serverprozess bleibt die eigentliche Laufzeit.
settings-mcp-auth-detail = Wechseln Sie zwischen No-Auth, Full OAuth und ChatGPT Mixed-Auth. Im gemischten Modus bleiben die Initialisierung und die Tool-Erkennung öffentlich; Toolaufrufe bleiben OAuth-geschützt, es sei denn, der anonyme Zugriff wird explizit aktiviert.
settings-mcp-anonymous-access-none-detail = Sichere Standardeinstellung: Kein Werkzeugaufruf ist anonym. ChatGPT kann den Katalog weiterhin initialisieren und erkennen, bevor er sich anmeldet.
settings-mcp-anonymous-access-read-only-detail = Opt-in mit hohem Risiko: Nur-Lese-Tools können anonym ausgeführt werden und können private Arbeitsbereichs-, Dateisystem-, Konfigurations- oder Diagnosedaten offenlegen.
settings-mcp-anonymous-access-inactive-detail = Diese Richtlinie gilt nur im gemischten Authentifizierungsmodus; Schalten Sie die Authentifizierung auf gemischt um, um sie zu verwenden.
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

provider-field-api-key = API-Schlüssel
provider-issuer-atomgit-label = atomgit
provider-issuer-atomgit-detail = AtomGit-Anmeldedaten
provider-field-api-key-env = API-Schlüssel-Umgebungsvariable
provider-field-username = Benutzername
provider-field-display-name = Anzeigename
provider-field-email = E-Mail-Adresse
provider-field-avatar-url = Avatar-URL
provider-model-field-catalog-template = Katalogvorlage
provider-model-catalog-selection-note = Mit Enter eine Vorlage auswählen. Beim Speichern werden ihre Felder übernommen, nicht ihre Katalog-ID.
provider-adapter-rule-none-ollama-detail = Ollama-Adapter; Authentifizierungsfelder bleiben leer und der Endpunkt wird am Adapter festgelegt.
provider-adapter-rule-api-openai-detail = Expliziter OpenAI-Responses- oder Chat-Completions-Adapter; verwendet die providerseitige Authentifizierungsoption base_url und unterstützt das Auflisten von Modellen im Entwurf.
provider-adapter-rule-api-anthropic-detail = Anthropic-HTTP-Adapter; verwendet die providerseitige Authentifizierungsoption base_url und unterstützt das Auflisten von Modellen im Entwurf.
provider-adapter-rule-api-gemini-detail = Gemini-HTTP-Adapter; verwendet die providerseitige Authentifizierungsoption base_url und unterstützt das Auflisten von Modellen im Entwurf.
provider-adapter-rule-cline-api-openai-detail = Cline API über den openai_chat_completions-Adapter; base_url und Modellerkennung bleiben auf den gehosteten Cline-Endpunkt festgelegt.
provider-adapter-rule-gitlab-auth-openai-detail = GitLab-Gateway über den ausgewählten OpenAI-Responses- oder Chat-Completions-Adapter.
provider-adapter-rule-gitlab-auth-anthropic-detail = GitLab-Gateway über den anthropic-Adapter.
provider-adapter-rule-openai-chatgpt-openai-detail = openai_responses-Adapter mit dem offiziellen ChatGPT-Codex-Backend; Anmeldedaten stammen aus der lokalen OpenAI-Sitzung.
provider-adapter-rule-github-copilot-openai-detail = Copilot-Token über den ausgewählten OpenAI-Responses- oder Chat-Completions-Adapter.
provider-adapter-rule-github-copilot-anthropic-detail = Copilot-Token über den anthropic-Adapter.
provider-adapter-rule-gitlab-credential-openai-detail = GitLab-OAuth-Anmeldedaten über den ausgewählten OpenAI-Responses- oder Chat-Completions-Adapter.
provider-adapter-rule-gitlab-credential-anthropic-detail = GitLab-OAuth-Anmeldedaten über den anthropic-Adapter.
provider-adapter-rule-google-adc-openai-detail = OpenAI-Chat-Completions-Adapter im Vertex-Stil mit capability_family=gemini; erfordert die providerseitige Authentifizierungsoption base_url.
provider-adapter-rule-sap-ai-core-openai-detail = SAP-AI-Core-openai_chat_completions-Adapter; erfordert die providerseitigen Authentifizierungsoptionen base_url und service_key_env.
provider-adapter-rule-bedrock-sigv4-amazon-bedrock-detail = Amazon-Bedrock-Adapter, signiert mit AWS SigV4.
provider-redirect-local-studio-detail = Localhost-Rückruf-URL passend zur Studio-Webroute
provider-profile-choice-detail = AWS-Profil aus der gemeinsamen Konfiguration
provider-profile-default-detail = AWS-Standardprofil
provider-api-key-env-openai-detail = Umgebungsvariable für einen OpenAI-kompatiblen API-Schlüssel
provider-api-key-env-anthropic-detail = Umgebungsvariable für einen Anthropic-API-Schlüssel
provider-api-key-env-gemini-detail = Umgebungsvariable für einen Gemini-API-Schlüssel
provider-api-key-env-gitlab-detail = Umgebungsvariable für ein GitLab-Token
provider-api-key-env-vertex-detail = Umgebungsvariable für ein Google-Vertex-Zugriffstoken
provider-api-key-env-shared-gateway-detail = Umgebungsvariable für einen gemeinsamen Gateway-API-Schlüssel
provider-api-key-env-opencode-detail = Umgebungsvariable für einen Opencode-API-Schlüssel
provider-service-key-env-detail = Standard-Umgebungsvariable für den SAP-AI-Core-Dienstschlüssel
settings-provider-workbench-detail = Öffnen Sie die durchsuchbare Anbieterliste, bevor Sie Authentifizierung, Adapter, Modellrouting oder neue Anbieter konfigurieren.
settings-provider-new-detail = Erstellen Sie einen neuen Anbieter, listen Sie Live-Adaptermodelle auf und bearbeiten Sie die Konfiguration des Anbieteradapters. Wählen Sie das Modell separat aus.
settings-model-catalog-open-detail = Überprüfen Sie die Metadaten des aufgelösten Modells und aktualisieren Sie den Cache des lokalen Modellkatalogs.
permission-studio-command-rules-shell-only = Befehlsregeln gelten für shell.exec, shell.spawn, shell.watch, shell.open und shell.write; verwenden Sie eine Namensregel oder die Standardeinstellung für andere Tools.
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
provider-model-field-manual-overrides = Manuelle Überschreibungen
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
provider-model-manual-overrides-detail = Ein: Diese Route behaelt ihre eigenen Werte. Aus: Sie folgt dem Katalog und uebernimmt Aktualisierungen beim naechsten Auflisten.
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
flash-workspace-default-model-updated = Standardmodell des Arbeitsbereichs aktualisiert: {$provider}/{$model}
flash-workspace-approval-model-updated = automatisches Genehmigungsmodell des Arbeitsbereichs aktualisiert: {$provider}/{$model}
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

value-reset = zurücksetzen
value-selected = ausgewählt
value-inactive = inaktiv
value-unrestricted = uneingeschränkt
value-inherit = übernehmen
value-inherits-runtime-defaults = übernimmt Laufzeitvorgaben
value-default-true = standardmäßig aktiviert
value-tools = Werkzeuge
value-plugin-tools = Plugin-Werkzeuge
value-inherit-off = Übernahme deaktiviert: { $sections }
value-path-summary = Pfad: { $detail }
value-network-summary = Netzwerk: { $detail }
value-plugin-override-count = { $count } Plugin-Überschreibungen
value-tools-summary = Werkzeuge: { $detail }
value-char-count = { $count } Zeichen
value-item-count = { $count } Elemente
value-on = an
value-off = aus
value-enabled = aktiviert
value-disabled = deaktiviert
value-unknown = unbekannt
value-hidden = ausgeblendet
value-visible = sichtbar
value-config-owned = von der Konfiguration verwaltet
value-file-backed = dateigestützt
value-editable = bearbeitbar
value-read-only = schreibgeschützt
value-built-in = integriert
value-built-in-defaults = integrierte Standardwerte
value-config-backed = konfigurationsgestützt
value-markdown-backed = Markdown-gestützt
value-runtime-registered = zur Laufzeit registriert
value-edit = bearbeiten
value-rename = umbenennen
value-duplicate = duplizieren
value-delete = löschen
value-runtime-config = Laufzeitkonfiguration
value-runtime-config-file = Laufzeitkonfiguration (agena.json)
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
plugin-workbench-column-command = Befehl
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
plugin-workbench-command-arguments = Argumente: {$command}
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
plugin-workbench-header-summary = Tools: {$tools}        Befehle: {$commands}        Konfiguration: {$config}
plugin-workbench-input-preview = Eingabevorschau: {$tool}
plugin-workbench-last-result-failed = Letztes Ergebnis · {$tool} · fehlgeschlagen
plugin-workbench-last-result-success = Letztes Ergebnis · {$tool} · erfolgreich
plugin-workbench-list-footer = Tippen zum Suchen · Oben/Unten auswählen · Enter öffnen · Esc schließen
plugin-workbench-list-summary = Plugins suchen… {$query}        Transport: {$transport}        Konfiguration: {$config}        {$shown}/{$total} angezeigt
plugin-workbench-loading-actions = Aktionen werden geladen…
plugin-workbench-loading-choices = Auswahl wird geladen…
plugin-workbench-no-changes = Keine Änderungen
plugin-workbench-no-commands = Keine Befehle.
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
plugin-workbench-tab-commands = Befehle
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

permission-studio-page-tool-defaults = Werkzeugzugriff / Standardwerte

# Generic inline content resources
content-loading = Ausgabe wird geladen…
content-active = Live-Ausgabe
content-complete = Ausgabe abgeschlossen
content-interrupted = Ausgabe unterbrochen
content-gap = Ein Teil der ursprünglichen Ausgabe ist nicht mehr gespeichert.
content-reconnecting = Ausgabe wird erneut verbunden…

content-window = Ein Teil der Ausgabe liegt außerhalb des Anzeigefensters.

# Chrome-only activity headlines (the stored English title is the fallback).
activity-title-answer = Antwort
activity-title-thinking = Denken
activity-title-text = Text
activity-title-error = Fehler
activity-title-notice = Hinweis
activity-title-hook = Hook
activity-title-compaction = Kontextkomprimierung
activity-title-provider-retry = Anbieter-Wiederholung
activity-title-turn-limit = Turn-Limit
activity-title-user-input = Benutzereingabe erforderlich
activity-title-pasted-text = Eingefügter Text

# Plugin Workbench, permission editor and transcript keys localised with the settings catalog.
permission-studio-access-read = {$pattern} · lesen
permission-studio-access-write = {$pattern} · schreiben
plugin-workbench-editor-pair-prompt = Geben Sie die beiden Werte für {$title} ein.
plugin-workbench-error-field-exists = Feld `{$field}` existiert bereits
plugin-workbench-error-field-name-empty = Der Feldname darf nicht leer sein
plugin-workbench-error-fixed-schema-type = {$title} hat einen festen Schematyp
plugin-workbench-error-invalid-branch-selection = Ungültige Zweigauswahl
plugin-workbench-error-invalid-enum-selection = Ungültige Enumerationsauswahl
plugin-workbench-error-invalid-type-selection = Ungültige Typauswahl
plugin-workbench-error-no-selectable-schema-type = {$title} hat keinen auswählbaren Schematyp
plugin-workbench-error-no-selection = Keine Auswahl verfügbar
plugin-workbench-error-plugin-unavailable = Plugin `{$plugin}` ist nicht verfügbar
plugin-workbench-error-read-only = {$title} ist schreibgeschützt
plugin-workbench-error-rename-failed = Feld konnte nicht umbenannt werden
plugin-workbench-error-row-not-object-field = Die ausgewählte Zeile ist kein Objektfeld
plugin-workbench-error-schema-mismatch = Der Plugin-Konfigurationswert entspricht nicht seinem Schema.
plugin-workbench-error-unsupported-tab = Plugin `{$plugin}` hat einen nicht unterstützten Workbench-Tab `{$tab}` angefordert; Config wird geöffnet
transcript-image-embedded = embedded image

# Activities, composer, diagnostics and usage analytics.
activities-title = Hintergrundaktivitäten
activities-empty = Keine Hintergrundaktivitäten.
activities-empty-hint = Mit shell.spawn, shell.open oder tasks.run eine Aktivität erstellen.
activities-section-active = Aktiv
activities-section-finished = Beendet
activities-footer-close = Esc schließen
activities-footer-move = ↑↓ bewegen
activities-footer-detail = Enter Details
activities-footer-stop = s {$control}
activities-footer-dismiss = d {$control}
activities-footer-clear = x leeren
activities-footer-filters = f/k/t Filter
activities-footer-refresh = r aktualisieren
activities-filter-active-only = nur aktive
activities-status-summary = {$active} aktiv · {$finished} beendet
activities-status-unknown = unbekannt
activities-row-message = {$message}
activities-detail-title = Details / Protokollende
activities-detail-no-selection = Keine Auswahl.
activities-detail-waiting = Warte auf Ausgabe…
activities-detail-no-output = Keine Ausgabe aufgezeichnet.
activities-detail-tail-hint = ↵ drücken, um das Protokoll dieser Aktivität live zu verfolgen.
activities-detail-command = $ {$command}
activities-detail-session = Sitzung #{$id}
activities-detail-source-part = Quellteil #{$id}
activities-detail-next-wake = nächster Aufruf {$at}
activities-detail-output-separator = Live-Protokoll @ {$seq}
activities-detail-lines-omitted = … {$count} ältere Zeilen ausgeblendet
activity-command-reference-title = Befehl: {$name}
activity-detail-type = Typ: {$media_type}
activity-detail-size = Größe: {$size} Bytes
activity-detail-dimensions = Abmessungen: {$width}×{$height}
activity-detail-preview-fields = {$count} Felder
activity-detail-preview-items = {$count} Elemente
composer-chip-command = [Befehl: {$name}]
composer-command-line = Befehl: {$name}
composer-clipboard-files = Dateien aus der Zwischenablage: {$error}
composer-clipboard-image = Bild aus der Zwischenablage: {$error}
composer-clipboard-text = Text aus der Zwischenablage: {$error}
composer-error-attachment-path = Anhangspfad ist keine reguläre Datei oder kein Verzeichnis im Server-Arbeitsbereich: {$path}
composer-error-attachment-unavailable = Anhangspfad ist auf dem Server nicht mehr verfügbar: {$path}
composer-error-clipboard-attachment-empty = Anhang aus der Zwischenablage ist leer: {$path}
composer-error-clipboard-attachment-not-file = Anhang aus der Zwischenablage ist keine reguläre Datei: {$path}
composer-error-clipboard-attachment-filename = Anhang aus der Zwischenablage hat keinen verwendbaren Dateinamen: {$path}
composer-error-clipboard-attachment-too-large = Anhang aus der Zwischenablage überschreitet das Upload-Limit von 20 MiB: {$path}
flash-notification-copied = Aus der Benachrichtigung kopiert.
flash-paste-limit-draft-unchanged = Eingefügter Text überschreitet das Limit von 1 MiB; der Entwurf wurde nicht geändert.
flash-ui-backend-task-failed = Befehlsaufgabe des UI-Backends fehlgeschlagen: {$error}
help-diagnostics-kitty-line = Kitty-Helfer {$version} · {$path} · Zwischenablage {$clipboard} · Übertragung {$transfer}
permission-rule-label = {$tool} · {$qualifier}
rewind-turn-label = Gesprächsrunde {$sequence}
rewind-turn-detail = Runde {$sequence} · {$time}
terminal-diagnostics-state-path-blocked = Pfad blockiert
terminal-diagnostics-state-path-unverified = Pfad nicht überprüft
terminal-diagnostics-state-provider-unavailable = Anbieter nicht verfügbar
transcript-diagram-title = ◇ Diagramm · {$label}
usage-title = Nutzungsanalyse
usage-title-refreshing = Nutzungsanalyse · wird aktualisiert…
usage-to-small = Das Terminal ist zu klein für die Nutzungsanalyse
usage-view-providers = Anbieter
usage-sort-tokens = Tokens
usage-filter-all = Alle
usage-filter-period = Zeitraum {$period}
usage-filter-view = Ansicht {$view}
usage-filter-provider = Anbieter {$provider}
usage-filter-model = Modell {$model}
usage-filter-subagents = Unteragenten {$state}
usage-filter-sort = Sortierung {$sort}
usage-filter-refresh = aktualisieren
usage-footer = ↑/↓ Zeilen Esc schließen · Enter Sitzung öffnen
usage-footer-open-session = Enter Sitzung öffnen
usage-footer-rows = ↑/↓ Zeilen Esc schließen
usage-loading = Nutzungsstatistiken werden geladen…
usage-total-cost = Gesamtkosten
usage-cost-line = erfasst {$recorded} · geschätzt {$estimated}
usage-tokens = Tokens
usage-tokens-line = Eingabe {$input} · Ausgabe {$output} · Schlussfolgern {$reasoning}
usage-sessions = Sitzungen
usage-active-days = {$count} aktive Tage
usage-average-cost = {$cost} im Schnitt/Anfrage
usage-requests = Anfragen
usage-cache-hit = Cache-Treffer
usage-cache-line = Lesen {$read} · Schreiben {$write} · 5 Min. {$write_5m} · 1 Std. {$write_1h}
usage-days-summary = {$days} aktive Tage · {$average} im Schnitt/Tag · Höchstwert {$peak} am {$date}
usage-unpriced-requests = ⚠ {$count} Anfragen ohne Preis
usage-empty = Keine Nutzung
usage-empty-filtered = Keine Nutzung für die ausgewählten Filter
usage-collecting = Nachrichtennutzung wird erfasst…
usage-error-badge = Fehler
usage-error-body = Nutzungsstatistiken konnten nicht geladen werden
usage-error-hint = Ctrl+R zum Aktualisieren drücken
usage-breakdown-provider = Aufschlüsselung nach Anbieter
usage-breakdown-model = Aufschlüsselung nach Modell
usage-breakdown-session = Aufschlüsselung nach Sitzung
usage-breakdown-daily = Tägliche Aufschlüsselung
usage-by-provider = Nach Anbieter
usage-by-model = Nach Modell
usage-daily-activity = Tägliche Aktivität
usage-column-name = Name
usage-column-cost = Kosten
usage-column-tokens = Tokens
usage-column-requests = Anfragen
usage-column-cache = Cache

# Remaining chrome keys for localized interface surfaces.
activities-status-pending = ausstehend
activities-status-running = läuft
activities-status-waiting = wartet
activities-status-paused = pausiert
activities-status-succeeded = erfolgreich
activities-status-failed = fehlgeschlagen
activities-status-cancelled = abgebrochen
activities-status-stopped = gestoppt
activities-control-stop = stoppen
activities-control-pause = pausieren
activities-control-resume = fortsetzen
activities-control-delete = löschen
activities-control-dismiss = ausblenden
activities-kind-shell = Shell
activities-kind-monitor = Überwachung
activities-kind-task = Aufgabe
activities-kind-cron = Geplante Aufgabe
activities-kind-runtime = Laufzeit
activities-kind-browser = Browser
activities-kind-activity = Aktivität
activity-detail-details = Details
activity-detail-language = Sprache
activity-detail-notice = Hinweis
activity-detail-request = Anfrage
activity-detail-source = Quelle
activity-section-error = Fehler
activity-section-input = Eingabe
activity-section-output = Ausgabe
activity-title-plan-review = Planprüfung
composer-chip-file = [Datei: {$name}]
composer-chip-folder = [Ordner: {$name}]
composer-clipboard-text-empty = Text in der Zwischenablage ist leer
composer-error-attachment-budget = Anhang überschreitet das Limit von 20 MiB pro Datei oder 40 MiB pro Nachricht
composer-error-attachment-outside-workspace = Der Anhang muss sich im aktiven Arbeitsbereich befinden.
composer-error-clipboard-attachments-budget = Anhänge aus der Zwischenablage überschreiten das gesamte Upload-Limit von 40 MiB
composer-error-clipboard-file-changed = Datei aus der Zwischenablage wurde geändert oder überschreitet das Leselimit
composer-error-empty-message = Die Nachricht ist leer.
composer-error-paste-limit = Eingefügter Text überschreitet das Limit von 1 MiB. Zwischenablage und aktueller Entwurf bleiben unverändert.
composer-notice-paste-as-text = Langer eingefügter Text wurde als Text beibehalten, da das Anhangslimit erreicht ist.
context-help-context-activities = Hintergrundaktivitäten
flash-attachment-prepare-failed = Ein Anhang konnte nicht vorbereitet werden; den Entwurf prüfen und erneut senden.
flash-attachment-preparing = Anhänge werden vorbereitet; die Nachricht wird gesendet, sobald sie bereit sind.
flash-clipboard-osc52-sent = Zwischenablage-Anfrage an das Terminal gesendet (OSC 52).
flash-command-no-file-view = Befehlsreferenzen haben keine Dateiansicht.
flash-download-outside-workspace = Download-Pfade dürfen den aktuellen Arbeitsbereich nicht verlassen.
flash-download-relative-path = Einen Pfad relativ zum aktuellen Arbeitsbereich verwenden.
flash-download-usage = Verwendung: /download <Arbeitsbereichspfad>
flash-ui-backend-queue-full = Befehlswarteschlange des UI-Backends ist voll; auf laufende Vorgänge warten
flash-ui-backend-unavailable = Befehlsdienst des UI-Backends ist nicht verfügbar
help-diagnostics-kitty-helper = Kitty-Helfer
message-role-tool = Werkzeug
permission-studio-command-pattern-suffix = + Befehlsmuster
search-picker-preview-title = Vorschau
search-picker-search-failed = Suche fehlgeschlagen
search-picker-no-preview = Keine Vorschau verfügbar
search-picker-page = Seite {$current}/{$total}
search-picker-searching = Suche läuft…
search-picker-loading-more = weitere werden geladen…
search-picker-error = Fehler
search-picker-results = {$count} Ergebnisse
search-picker-footer-space = Space umschalten · {$count} ausgewählt
search-picker-footer-enter = Enter bestätigen
search-picker-footer-navigate-page = ↑/↓ navigieren · ←/→ Seite
search-picker-footer-page = ←/→ Seite
search-picker-footer-navigate = ↑/↓ navigieren
search-picker-footer-enter-select = Enter auswählen
search-picker-footer-close = Esc schließen
settings-choice-log-level-detail = Protokollstufe
terminal-diagnostics-state-confirmed = bestätigt
terminal-diagnostics-state-forced = erzwungen
terminal-diagnostics-state-profiled = aus Profil abgeleitet
terminal-diagnostics-state-unsupported = nicht unterstützt
terminal-diagnostics-state-unknown = unbekannt
transcript-alert-note = Hinweis
transcript-alert-tip = Tipp
transcript-alert-important = Wichtig
transcript-alert-warning = Warnung
transcript-alert-caution = Vorsicht
transcript-code-block-empty = (leer)
transcript-diff-changes = Änderungen
transcript-image-placeholder = Bild
usage-view-overview = Übersicht
usage-view-daily = Täglich
usage-view-models = Modelle
usage-view-sessions = Sitzungen
usage-sort-cost = Kosten
usage-sort-requests = Anfragen
usage-filter-subagents-included = eingeschlossen
usage-filter-subagents-excluded = ausgeschlossen
search-picker-custom-value-label = Eingegebenen Wert verwenden
search-picker-custom-value-detail = Genau {$value} anwenden
search-picker-custom-path-label = Eingegebenen Pfad verwenden

command-activities-summary = Hintergrundaktivitäten öffnen

command-allow-always-summary = Erste ausstehende Berechtigungsanfrage immer zulassen

command-allow-summary = Erste ausstehende Berechtigungsanfrage einmalig zulassen

command-attach-summary = Datei über die interaktive Auswahl anhängen

command-btw-summary = Separat fragen, während das Hauptgespräch weiterläuft. Nur lesende Tools; keine Änderungen am Hauptgespräch.

command-children-summary = Auswahl untergeordneter Sitzungen öffnen

command-commands-summary = Befehlspalette öffnen

command-commit-summary = Commit aus vorgemerkten Änderungen erstellen

command-compact-summary = Sitzungskontext zu einer Zusammenfassung verdichten

command-continue-summary = Aktuelle blockierte oder ausstehende Sitzung fortsetzen

command-copy-message-summary = Letzte Assistentennachricht kopieren

command-copy-summary = Geladenes Transkript kopieren

command-copy-visible-summary = Sichtbaren Transkriptbereich kopieren

command-deny-always-summary = Erste ausstehende Berechtigungsanfrage immer ablehnen

command-deny-summary = Erste ausstehende Berechtigungsanfrage einmalig ablehnen

command-diagnostics-summary = Bereinigte Diagnosezusammenfassung für Feedback anzeigen

command-download-summary = Arbeitsbereichsdatei über den aktiven Terminal-Übertragungsanbieter herunterladen

command-editor-summary = Externen Editor für das Eingabefeld öffnen

command-export-summary = Geladenes Transkript als Markdown exportieren und im Editor öffnen

command-favorite-summary = Aktuelle oder ausgewählte Sitzung zu den Favoriten hinzufügen oder daraus entfernen

command-fork-summary = Abzweigung der aktuellen Sitzung mit vollständigem Verlauf öffnen

command-help-summary = Hilfe zur aktuellen Oberfläche anzeigen

command-image-summary = Bild über die interaktive Auswahl anhängen

command-lineage-summary = Auswahl des Sitzungsverlaufs für die aktuelle Sitzung öffnen

command-model-summary = Modellauswahl der Sitzung für Anbieter/Adapter/Modell öffnen

command-new-summary = Neue Sitzung erstellen

command-pager-summary = Geladenes Transkript in einem Terminal-Pager öffnen

command-parent-summary = Zur übergeordneten Sitzung springen

command-paste-summary = Dateien, Bilder oder Text aus der Zwischenablage in das Eingabefeld einfügen

command-plan-summary = Planansicht für die aktuelle Sitzung öffnen

command-pr-summary = Pull Request mit minimalen gh-Optionen erstellen

command-rename-summary = Aktuelle oder ausgewählte Sitzung umbenennen

command-rewind-summary = Benutzernachricht und alle folgenden Nachrichten in einen neuen Zweig zurücknehmen

command-sessions-summary = Sitzungsumschalter öffnen

command-side-summary = Abgezweigtes Nebengespräch öffnen, ohne den aktuellen Lauf zu unterbrechen

command-status-summary = Aktuellen Sitzungs-, Ausführungs-, Arbeitsbereichs- und Oberflächenstatus anzeigen

command-timeline-summary = Durchsuchbare Ereigniszeitleiste für die aktuelle oder ausgewählte Sitzung öffnen

command-usage-summary = Interaktive Token- und Kostenanalyse öffnen

command-user-input-summary = Erste ausstehende Benutzereingabe beantworten

context-help-summary-sessions = Geladene Sitzungen durchsuchen und die aktive Unterhaltung auswählen.

context-help-summary-transcript = Die aktuelle Unterhaltung lesen, durchsuchen, erweitern, durchgehen und kopieren.

context-help-summary-composer = Nachricht verfassen, vorgemerkte Elemente verwalten und an die aktive Sitzung senden.

context-help-summary-composer-items = Anhänge und eingefügte Elemente prüfen, öffnen, fokussieren, neu anordnen oder entfernen.

context-help-summary-history = Frühere Eingaben von neu nach alt durchsuchen, ohne den Eingabebereich zu verlassen.

context-help-summary-suggestions = Einen passenden Befehl oder Dateiverweis für die aktuelle Suche auswählen.

context-help-summary-editor = Einen Wert bearbeiten und speichern oder abbrechen.

context-help-summary-editor-multiline = Mehrzeiligen Text bearbeiten und anschließend vollständig speichern.

context-help-summary-search-picker = Verfügbare Optionen durchsuchen und ein passendes Ergebnis auswählen.

context-help-summary-choice-list = Eine sichtbare Option auswählen, ohne den Suchtext zu ändern.

context-help-summary-timeline = Sitzungsereignisse durchsuchen und zur zugehörigen Nachricht springen.

context-help-summary-permission = Anfrage prüfen, Details ansehen und eine klare Entscheidung treffen.

context-help-summary-details = Aktuelle Details lesen und zur vorherigen Entscheidungsseite zurückkehren.

context-help-summary-user-input = Die aktuelle Frage beantworten und mit den übrigen Punkten fortfahren.

context-help-summary-user-input-editor = Eine eigene Antwort eingeben und übernehmen oder zu den Optionen zurückkehren.

context-help-summary-user-input-review = Alle Antworten prüfen, bevor die vollständige Antwort gesendet wird.

context-help-summary-user-input-decision-review = Abschließende Entscheidung treffen und Begründungsdetails unabhängig davon durchgehen.

context-help-summary-confirm = Angezeigten Vorgang bestätigen oder unverändert lassen.

context-help-summary-usage = Nutzungsdaten prüfen und Dashboard-Filter sowie Aktualisierung direkt aufrufen.

context-help-summary-model-catalog = Modellkatalog durchsuchen und Suche, Aktualisierung sowie Seitenwechsel direkt aufrufen.

context-help-summary-list = Sichtbare Zeilen durchgehen und das ausgewählte Element öffnen.

context-help-summary-panes = Zwischen sichtbaren Bereichen wechseln und die ausgewählte Zeile oder Steuerung bedienen.

context-help-summary-action-pane = Berechtigungsinhalte durchgehen und Regelaktionen direkt aufrufen.

context-help-summary-provider = Anbieterfelder, Adapter, Modelle und sichtbare Speicheraktionen konfigurieren.

context-help-summary-plugin-list = Plugins filtern, sichtbare Filter anpassen und ein Plugin öffnen.

context-help-summary-plugin-detail = Zwischen Detailreitern wechseln und den ausgewählten Reiter durchgehen.

context-help-summary-plugin-config = Im Konfigurationsbereich navigieren und seitenweite Aktionen direkt aufrufen.

context-help-summary-plugin-actions = Eine Aktion für den ausgewählten Konfigurationswert auswählen.

context-help-summary-plugin-selection = Einen oder mehrere zulässige Werte auswählen und übernehmen.

context-help-summary-plugin-drilldown = Verschachtelte Konfigurationszeilen durchgehen und die ausgewählte Zelle öffnen.

context-help-summary-plugin-diff = Ausstehende Konfigurationsänderungen prüfen und zurückkehren.

context-help-summary-session-work = Dateien, Hintergrundaufgaben und Wiederholungsstatus dieser Sitzung.

context-help-key-toggle-favorite = Ausgewählte Sitzung zu den Favoriten hinzufügen oder daraus entfernen.

context-help-section-actions = Aktionen

context-help-section-editing = Bearbeiten

context-help-section-navigation = Navigation

context-help-section-search = Suche und Auswahl

context-help-section-selection = Auswahl

context-help-section-workflow = Arbeitsablauf

context-help-tip-ctrl-h = Ctrl+H öffnet immer die Hilfe für die gerade verwendete Oberfläche.

message-input-activity-attachment = Anhang

message-input-activity-skill = Befehl

message-activity-response-running = Antwort wird generiert

message-activity-response-completed = Antwort abgeschlossen

message-activity-response-failed = Antwort fehlgeschlagen

message-activity-response-cancelled = Antwort abgebrochen

message-activity-continue-running = Antwort wird fortgesetzt

message-activity-continue-completed = Fortsetzung abgeschlossen

message-activity-continue-failed = Fortsetzung fehlgeschlagen

message-activity-continue-cancelled = Fortsetzung abgebrochen

message-activity-compact-running = Kontext wird komprimiert

message-activity-compact-completed = Kontext komprimiert

message-activity-compact-failed = Kontextkomprimierung fehlgeschlagen

message-activity-compact-cancelled = Kontextkomprimierung abgebrochen

message-activity-permission-running = Berechtigungsantwort wird angewendet

message-activity-permission-completed = Berechtigungsantwort angewendet

message-activity-permission-failed = Berechtigungsantwort fehlgeschlagen

message-activity-permission-cancelled = Berechtigungsantwort abgebrochen

message-activity-input-running = Benutzereingabe wird übermittelt

message-activity-input-completed = Benutzereingabe übermittelt

message-activity-input-failed = Übermittlung der Benutzereingabe fehlgeschlagen

message-activity-input-cancelled = Übermittlung der Benutzereingabe abgebrochen

message-compaction-unchanged = Der Kontext ist bereits kompakt. Der bisherige Verlauf wurde beibehalten.
message-compaction-title-manual = Kontext komprimiert

message-compaction-title-auto = Kontext automatisch komprimiert

message-compaction-title-reactive = Kontext nach Überschreitung des Kontextlimits komprimiert

message-compaction-token-summary = {$before} → {$after} Tokens · {$percent}% Reduktion

message-compaction-strategy-local = lokale Zusammenfassung

message-compaction-strategy-native = nativer Checkpoint des Anbieters

message-compaction-strategy-line = Strategie: {$strategy}

message-compaction-generation-line = Generation {$generation}

transcript-footer-pending = wird gesendet, sobald der aktive Durchlauf beendet ist

transcript-footer-pending-preview = als Nächstes gesendet: {$preview}

transcript-footer-plugin-block = {$label}: {$body}

transcript-export-path-is-directory = Der Exportpfad ist ein Verzeichnis: {$path}

transcript-export-title = Agena-Transkript-Export

transcript-export-session-id = - Sitzungs-ID: {$id}

transcript-export-exported-at = - Exportiert am: {$time}

transcript-export-messages-loaded = - Geladene Nachrichten: {$count}

transcript-export-older-messages-omitted = - Ausgelassene ältere Nachrichten: {$value}

transcript-export-parent-session = - Übergeordnete Sitzung: #{$id}

transcript-export-child-sessions = - Untergeordnete Sitzungen: {$count}

transcript-export-empty = _In dieser Sitzung wurden keine Nachrichten geladen._

transcript-node-kind-message = Nachricht

transcript-node-kind-markdown = Markdown-Block

transcript-node-kind-heading = Überschrift

transcript-node-kind-quote = Zitat

transcript-node-kind-alert = Hinweis

transcript-node-kind-code = Codeblock

transcript-node-kind-list = Liste

transcript-node-kind-table = Tabelle

transcript-node-kind-math = Formel

transcript-node-kind-image = Bild

transcript-node-kind-footnote = Fußnote

transcript-node-kind-diagram = Diagramm

transcript-node-kind-activity = Aktivitätsblöcke

transcript-footer-side-active = Nebengespräch · mit /parent oder über die Navigation verlassen

transcript-footer-side-return = Nebengespräch geöffnet · zum Zurückkehren wechseln

transcript-history-more = ↑ Ältere Nachrichten laden · nach oben scrollen oder hier klicken

transcript-history-loading = … Ältere Nachrichten werden geladen

transcript-history-start = Anfang des Gesprächs

transcript-history-error = ↑ Erneut versuchen, den Verlauf zu laden · { $error }

transcript-history-stalled = Der Verlaufscursor ist unverändert. Erneut laden.

transcript-fold-loading = … Frühere Antwortteile werden geladen

transcript-fold-error = Laden fehlgeschlagen · zum Wiederholen Enter drücken · { $error }

overlay-snapshot-title = Momentaufnahme
overlay-rename-title = Sitzung umbenennen
overlay-rename-prompt = Sitzungstitel bearbeiten
overlay-choice-footer = Enter auswählen · Esc schließen
overlay-choice-footer-select = Enter auswählen · Esc schließen
overlay-help-footer = Pfeiltasten blättern · Esc schließen
overlay-attach-image-title = Bild anhängen
overlay-attach-image-prompt = Bildpfad oder Suchbegriff eingeben. Enter hängt das ausgewählte Bild an.
overlay-attach-image-no-match = Keine passenden Bilder
overlay-attach-browser-prompt = Aktuellen Pfad durchsuchen. Datei- oder Ordnernamen zum Filtern eingeben oder den Pfad bearbeiten, um das Verzeichnis zu wechseln.
overlay-attach-image-browser-prompt = Bilder im aktuellen Pfad durchsuchen. Bild- oder Ordnernamen zum Filtern eingeben oder den Pfad bearbeiten, um das Verzeichnis zu wechseln.
overlay-attach-browser-footer = ↓ auswählen · Rechts Ordner öffnen · Links übergeordneten Ordner · Enter anhängen · Esc schließen
overlay-path-browser-list-title = Dateibrowser
overlay-plan-approval-title = Planfreigabe
overlay-plan-approval-summary = Aktiven Plan prüfen, bevor die Umsetzung fortgesetzt wird.
overlay-plan-approval-footer = a einmalig genehmigen | s Sitzung genehmigen | d ablehnen
overlay-plan-slug = Plan={$slug}
overlay-plan-path = Pfad: {$path}
overlay-plan-steps = Schritte={$steps}
overlay-plan-preview-heading = Vorschau
overlay-permission-request-id = Anforderungs-ID: {$request_id}
overlay-permission-reason = Grund: {$reason}
overlay-permission-explanation = Erläuterung: {$explanation}
overlay-permission-session = Sitzung: #{$session}
overlay-permission-requested-actions = Angeforderte Aktionen
overlay-permission-related-actions = Zugehörige Aktionen
overlay-permission-fact-source = Quelle={$value}
overlay-permission-fact-scope = Geltungsbereich={$value}
overlay-permission-fact-operator = Angefordert von={$value}
overlay-permission-footer = a einmalig erlauben | s für Sitzung erlauben | d ablehnen
overlay-permission-footer-edit-rule = {$footer} | e Regel bearbeiten
overlay-permission-footer-action = ↑/↓ auswählen | Enter ausführen | Esc schließen
overlay-permission-footer-scope = ↑/↓ auswählen | Enter ausführen | Esc zurück
overlay-permission-footer-details = Esc zurück
overlay-permission-decision-heading = Wie soll mit dieser Anfrage verfahren werden?
overlay-permission-scope-heading = Diese Entscheidung gilt für
overlay-permission-choice-allow = Erlauben…
overlay-permission-choice-deny = Ablehnen…
overlay-permission-choice-edit-rule = Präzise Regel erstellen…
overlay-permission-choice-auto-approve-busy = Automatische Freigabe läuft…
overlay-permission-choice-once = Nur dieses Mal
overlay-permission-choice-session = Aktuelle Sitzung
overlay-permission-choice-workspace = Aktueller Arbeitsbereich
overlay-permission-choice-global = Global
overlay-permission-choice-allow-always-session = Immer erlauben (Sitzung)
overlay-permission-choice-allow-always-workspace = Immer erlauben (Arbeitsbereich)
overlay-permission-choice-allow-always-global = Immer erlauben (global)
overlay-permission-choice-deny-always-session = Immer ablehnen (Sitzung)
overlay-permission-choice-deny-always-workspace = Immer ablehnen (Arbeitsbereich)
overlay-permission-choice-deny-always-global = Immer ablehnen (global)
overlay-permission-rule-fields = Regelfelder
overlay-permission-rule-empty-detail = Zum Bearbeiten ein Berechtigungsfeld auswählen.
overlay-permission-rule-item-browse-target-path = Zielpfad durchsuchen
overlay-permission-rule-item-browse-target-path-detail = Datei oder Ordner im Arbeitsbereich oder Dateisystem auswählen.
overlay-permission-rule-item-browse-workspace-root = Arbeitsbereichsverzeichnis durchsuchen
overlay-permission-rule-item-browse-workspace-root-detail = Verzeichnis für workspace_root auswählen.
overlay-permission-rule-item-browser-value = Browser öffnen
overlay-permission-rule-item-save = Regel speichern
overlay-permission-rule-item-save-detail = Den aktuellen Entwurf prüfen und speichern.
overlay-permission-rule-item-revoke = Regel widerrufen
overlay-permission-rule-item-revoke-detail = Die gespeicherte Regel widerrufen.
overlay-permission-rule-detail-browse-workspace-root = Verzeichnisbrowser öffnen und einen Ordner für workspace_root auswählen.
overlay-permission-rule-detail-browse-target-path = Dateisystembrowser öffnen und eine Datei oder einen Ordner als Vorbelegung für target_path auswählen.
overlay-permission-rule-detail-revoke = Beim Widerruf wird die gespeicherte Regel deaktiviert, ihr Verlauf bleibt erhalten.
overlay-permission-rule-preview-heading = Vorschau
overlay-permission-rule-preview-label = Bezeichnung: {$label}
overlay-permission-rule-preview-mode = Modus: {$mode}
overlay-permission-rule-preview-scope = Geltungsbereich: {$scope}
overlay-permission-rule-preview-subject-tool = Regelziel: Werkzeug ({$tool})
overlay-permission-rule-preview-subject-path = Regelziel: Dateizugriff ({$access})
overlay-permission-rule-preview-subject-network = Regelziel: Netzwerkzugriff
overlay-permission-rule-preview-qualifier = Einschränkung: {$qualifier}
overlay-permission-rule-preview-target = Ziel: {$target}
overlay-permission-rule-preview-workspace-root = Arbeitsbereichsverzeichnis: {$path}
overlay-permission-rule-preview-session-id = Sitzungs-ID: {$session}
overlay-permission-rule-preview-invalid = Ungültige Regel: {$error}
overlay-workbench-overview = Übersicht
overlay-permission-studio-footer = Links/Rechts oder Tab/Umschalt+Tab wechselt Bereiche · Pfeile wählen aus · Enter bearbeitet · Esc zurück
overlay-permission-studio-sections = Berechtigungsbereiche
overlay-permission-studio-nav = Navigation
overlay-detail-current-value = Aktueller Wert
overlay-choice-current-unavailable-detail = Aktueller Wert; nicht unter den verfügbaren Optionen
overlay-choice-clear-runtime-detail = Überschreibung der aktuellen Sitzung für {$field} entfernen.
overlay-choice-clear-provider-detail = {$field} im Entwurf leeren.
overlay-choice-clear-permission-subject = Regelziel auf die Standard-Werkzeugregel zurücksetzen.
overlay-choice-clear-permission-access-kind = Überschreibung der Dateizugriffsart entfernen.
overlay-choice-clear-permission-scope = Regelgeltungsbereich auf Arbeitsbereich zurücksetzen.
overlay-choice-clear-permission-mode = Regelmodus auf Nachfragen zurücksetzen.
overlay-choice-clear-permission-override-detail = Aktuelle Überschreibung für {$field} entfernen.
overlay-client-versions-title = Client-Versionen der Anbieter
overlay-insert-content-title = Inhalt einfügen
overlay-insert-content-prompt = Inhalt zum Einfügen an der Cursorposition im Eingabefeld auswählen
overlay-resume-title = Sitzung fortsetzen
overlay-resume-prompt = Sitzungen nach Titel, ID oder Verlauf filtern
overlay-resume-empty = Keine passenden Sitzungen
overlay-session-search-scope-all = alle
overlay-session-search-scope-roots = Wurzeln
overlay-session-search-scope-subtree = Teilbaum
overlay-session-search-tail-more = weitere
overlay-session-search-tail-end = Ende
overlay-session-search-footer-local = Bereich {$scope} · {$total} Ergebnisse
overlay-session-search-footer-remote = Bereich {$scope} · {$loaded} geladen · {$tail}
overlay-lineage-empty = Keine zugehörigen Sitzungszweige gefunden
overlay-rewind-empty = Keine abgeschlossenen Benutzernachrichten zum Zurücknehmen
overlay-rewind-confirm-title = Zurücksetzen bestätigen
overlay-rewind-confirm-keep = {$target} und alle folgenden Nachrichten zurücknehmen
overlay-rewind-confirm-warning = Ein neuer Zweig enthält nur den Verlauf vor dieser Benutzernachricht.
overlay-rewind-confirm-draft = Die zurückgenommene Benutzernachricht wird im Eingabefeld des neuen Zweigs wiederhergestellt; der aktuelle Entwurf bleibt in der ursprünglichen Sitzung.
overlay-confirm-footer = Y bestätigen
overlay-snapshot-remove-title = Entfernen der Momentaufnahme bestätigen
overlay-snapshot-remove-body = Aktive Momentaufnahme entfernen und aktuelle Sitzung verlassen?
overlay-snapshot-remove-force = Beim erzwungenen Entfernen gehen nicht übernommene Änderungen der Momentaufnahme verloren.
overlay-model-providers-title = Modellanbieter
overlay-model-providers-prompt = Vor der Modellauswahl einen Anbieter auswählen
overlay-models-title = Modelle [{$provider}]
overlay-models-prompt = Modell für Anbieter {$provider} auswählen
overlay-session-model-title = Sitzungsmodell
overlay-session-model-prompt = Modelle suchen
overlay-session-model-list-title = Modelle
overlay-session-model-footer = Zum Filtern tippen
overlay-model-catalog-title = Modellkatalog
overlay-model-catalog-footer = Ctrl+F suchen · Ctrl+R aktualisieren · Links/Rechts vorherige/nächste Seite · Oben/Unten auswählen · Esc schließen
overlay-model-catalog-search-title = Modellkatalog durchsuchen
overlay-model-catalog-search-prompt = Modellkatalog nach ID, Anzeigename, Herkunft oder Beschreibung filtern
overlay-model-catalog-entries = Modelle
overlay-model-catalog-detail = Details
overlay-model-catalog-query-all = <alle>
overlay-model-catalog-summary = Abfrage {$query} · Seite {$start}-{$end} / {$total} · {$count} Modelle
overlay-permission-rule-overview-draft = Entwurf
overlay-permission-rule-overview-mode = Modus
overlay-permission-rule-overview-scope = Geltungsbereich
overlay-permission-rule-overview-session = Sitzung
overlay-permission-rule-overview-workspace-root = Arbeitsbereichsverzeichnis
overlay-children-title = Untergeordnete Sitzungen [#{$session}]
overlay-children-prompt = Untergeordnete Sitzung zum Öffnen auswählen
overlay-children-empty = Keine untergeordneten Sitzungen gefunden
overlay-timeline-title = Zeitleiste [#{$session}]
overlay-timeline-prompt = Nach Ereignistyp, Befehl oder Nachrichtenmetadaten filtern
overlay-timeline-empty = Keine passenden Ereignisse
overlay-timeline-footer = Zum Filtern tippen · Pfeiltasten wählen aus · Enter öffnet verknüpfte Nachricht · Esc schließen
overlay-timeline-detail = Details

context-help-key-move = Aktuelle Auswahl um ein Element verschieben.
context-help-key-page = Um eine sichtbare Seite weiterschalten.
context-help-key-half-page = Um eine halbe sichtbare Transkriptseite weiterschalten.
context-help-key-scroll-line = Ansichtsbereich um eine Zeile scrollen, ohne den Cursor zu bewegen.
context-help-key-jump-history = Zwischen den letzten Cursorpositionen vor- und zurückspringen.
context-help-key-first-last = Zum ersten oder letzten Element springen.
context-help-key-open = Ausgewähltes Element öffnen.
context-help-key-session-scope = Sitzungsbereich „Alle“, „Wurzeln“ oder „Teilbaum“ auswählen.
context-help-key-session-cycle = Sitzungsbereich durchschalten.
context-help-key-transcript-vertical = Eine gerenderte Zeile bewegen und dabei die Zielspalte beibehalten.
context-help-key-transcript-horizontal = Zwischen vollständigen Nachrichten springen.
context-help-key-visual-select = Zeichen-, Zeilen- oder Blockauswahl im Visual-Modus starten; o/O wechseln die Endpunkte, gv stellt die Auswahl wieder her, y kopiert sie und Esc bricht ab.
context-help-key-count = Die folgende Vim-Bewegung wiederholen.
context-help-key-insert-mode = Eingabefeld im INSERT-Modus öffnen.
context-help-key-toggle = Ausgewählten Wert oder aufklappbaren Block umschalten.
context-help-key-expand-all = Alle Transkriptteile laden, ohne deren Details zu ändern.
context-help-key-collapse-parts = Diese Antwort (zc) oder alle Antworten (zC) auf die neuesten 5 Teile und ausstehende Interaktionen reduzieren; einzelne Detailauswahlen beibehalten.
context-help-key-copy = Visuelle Auswahl, Vim-Bewegung, aktuelle Zeile, Markdown-Block oder vollständige Nachricht kopieren.
context-help-key-search-transcript = Transkript durchsuchen und Treffer durchgehen.
context-help-key-word = Wort-, WORD- oder zeilenweise zum Anfang, ersten Nicht-Leerzeichen oder Ende bewegen.
context-help-key-find = Zum nächsten oder vorherigen Vorkommen eines eingegebenen Zeichens (f/F/t/T) springen; mit ; oder , wiederholen.
context-help-key-text-object = Wort, Absatz, Markdown-Block oder vollständige Nachricht als Textobjekt auswählen.
context-help-key-mouse = Scrollen, klicken, doppelklicken oder ziehen, um Transkriptinhalt auszuwählen; Sitzungs-Kopfzeile, Modellstatuszeile oder Eingabefeld ziehen, um Text auszuwählen, dann mit Ctrl+Y (oder y im Ansichtsmodus) kopieren.
context-help-key-line-start-end = Zum ersten, ersten Nicht-Leerzeichen oder letzten Zeichen der gerenderten Zeile springen.
context-help-key-count-prefix = Vor einer Vim-Bewegung eine Zahl eingeben, um sie mehrfach anzuwenden (z. B. 5j, 3w, 10G).
context-help-key-viewport = Zur obersten, mittleren oder untersten sichtbaren Zeile springen.
context-help-key-scroll-viewport = Ansichtsbereich so scrollen, dass der Cursor oben, mittig oder unten steht.
context-help-key-visual-endpoints = Beim Erweitern einer visuellen Auswahl den aktiven Endpunkt wechseln.
context-help-key-visual-restore = Letzte visuelle Auswahl wiederherstellen.
context-help-key-cancel-selection = Visuelle Auswahl oder Maus-Textauswahl abbrechen.
context-help-key-copy-line = Aktuelle Zeile kopieren.
context-help-key-copy-motion = Vom Cursor bis zum Zeilenende oder entlang einer Vim-Bewegung kopieren.
context-help-key-copy-text-object = Wort, Absatz, Markdown-Block oder vollständige Nachricht kopieren.
context-help-key-send = Aktuelle Nachricht aus dem Eingabefeld senden.
context-help-key-send-now = Lauf unterbrechen und diese Nachricht sofort senden.
context-help-key-newline = Zeilenumbruch einfügen, ohne zu senden.
context-help-key-view-mode = Zum VIEW-Modus des Transkripts zurückkehren.
context-help-key-editor-move = Textcursor zeichen-, zeilen- oder wortweise bewegen.
context-help-key-editor-edit = Text mit Shell-ähnlichen Bearbeitungsfunktionen löschen, entfernen oder wiederherstellen.
context-help-key-history = Durchsuchbaren Verlauf des Eingabefelds öffnen.
context-help-key-recover = Ausstehende Nachricht im Eingabefeld bearbeiten.
context-help-key-cancel-pending = Noch nicht zugestellte Nachricht abbrechen.
context-help-key-clear-composer = Eingabefeld und alle vorgemerkten Eingaben leeren.
context-help-key-items = Vorgemerkte Elemente des Eingabefelds öffnen.
context-help-key-commands = Slash-Befehle öffnen oder eingeben.
context-help-key-attach = Datei anhängen.
context-help-key-insert-content = Bereich zum Einfügen lokaler Pfade und Anhänge öffnen.
context-help-key-external-editor = Eingabefeld im konfigurierten externen Editor bearbeiten.
context-help-key-image = Dateien, Bilder oder Text aus der Zwischenablage einfügen.
context-help-key-pending-requests = Ausstehende Benutzereingabe- oder Berechtigungsanfrage öffnen.
context-help-key-next = Zum nächsten Element wechseln.
context-help-key-previous = Zum vorherigen Element wechseln.
context-help-key-close = Diese Oberfläche schließen.
context-help-key-older = Älteren Verlaufseintrag auswählen.
context-help-key-newer = Neueren Verlaufseintrag auswählen.
context-help-key-newer-stay = Neueren Eintrag auswählen, ohne den Verlauf am neuesten Eintrag zu schließen.
context-help-key-accept = Aktuelle Auswahl übernehmen.
context-help-key-filter = Tippen, um sichtbare Ergebnisse zu filtern.
context-help-key-fill = Ausgewählten Vorschlag in das Eingabefeld übernehmen.
context-help-key-edit-text = Aktuellen Wert eingeben und bearbeiten.
context-help-key-submit = Aktuellen Wert absenden.
context-help-key-cancel = Abbrechen, ohne Änderungen anzuwenden.
context-help-key-save = Vollständig bearbeiteten Wert speichern.
context-help-key-refresh = Aktuelle Daten aktualisieren.
context-help-key-provider-add-model = Dem ausgewählten Anbieteradapter ein Modell hinzufügen.
context-help-key-provider-save-adapter = Ausgewählten Anbieteradapter speichern.
context-help-key-usage-period = Berichtszeitraum durchschalten.
context-help-key-usage-view = Dashboard-Ansicht durchschalten.
context-help-key-usage-provider = Anbieterfilter durchschalten.
context-help-key-usage-model = Modellfilter durchschalten.
context-help-key-usage-subagents = Subagenten ein- oder ausschließen.
context-help-key-usage-sort = Sortierreihenfolge durchschalten.
context-help-key-model-catalog-search = Suchanfrage im Modellkatalog bearbeiten.
context-help-key-model-catalog-page = Vorherige oder nächste Katalogseite laden.
context-help-key-plugin-transport = Transportfilter durchschalten.
context-help-key-plugin-config-filter = Filter für den Konfigurationsstatus durchschalten.
context-help-key-plugin-validate = Aktuelle Plugin-Konfiguration validieren.
context-help-key-plugin-reset = Aktuelle Plugin-Konfiguration auf Standardwerte zurücksetzen.
context-help-key-plugin-diff = Ausstehende Konfigurationsunterschiede ein- oder ausblenden.
context-help-key-permission-add = Im aktuellen Berechtigungsbereich eine Regel hinzufügen.
context-help-key-permission-rename = Ausgewählte Berechtigungsregel umbenennen.
context-help-key-permission-browse = Ausgewähltes Pfadfeld durchsuchen.
context-help-key-activate = Ausgewählte Zeile, Steuerung oder Aktion aktivieren.
context-help-key-jump-message = Zur zugehörigen Nachricht des Ereignisses springen.
context-help-key-back = Zur vorherigen Ebene zurückkehren.
context-help-key-next-question = Zwischen Fragen und Übersichtsseite vor- oder zurückgehen.
context-help-key-custom-answer = Eigene Antwort bearbeiten.
context-help-key-clear = Ausgewählte Überschreibung oder Antwort leeren.
context-help-key-cancel-request = Gesamte Anfrage schließen oder abbrechen.
context-help-key-edit-answer = Ausgewählte Antwort erneut bearbeiten.
context-help-key-scroll-body = Unabhängigen Übersichtsbereich scrollen.
context-help-key-submit-all = Alle geprüften Antworten absenden.
context-help-key-confirm = Angezeigten Vorgang bestätigen.
context-help-key-focus-next = Fokus vor- oder rückwärts durch sichtbare Bereiche und Steuerelemente bewegen.
context-help-key-pane-horizontal = Direkt zum linken oder rechten Bereich wechseln.
context-help-key-horizontal = Zwischen sichtbaren Spalten, Zellen oder Aktionen wechseln.
context-help-key-next-tab = Zwischen Detailreitern vor- oder zurückwechseln.
context-help-key-scroll = Aktuellen Detailinhalt scrollen.
context-help-key-work-toggle = Arbeitsbereichsbereich ein- oder ausklappen.
context-help-key-work-tabs = Zwischen Dateien / Aufgaben / Status wechseln.
context-help-key-work-staged = Zwischen vorgemerkten und Arbeitsänderungen wechseln.
context-help-key-work-more = Weitere Diff-Inhalte laden.
context-help-key-work-control = Verfügbare Aktion der ausgewählten Aufgabe ausführen.
context-help-key-work-leave = Zum Eingabefeld zurückkehren.
context-help-key-answer-next = Antwort auswählen und zur nächsten Seite wechseln.
context-help-key-summary-action = Antwort bearbeiten, zurückgehen, absenden oder ausgewählte Übersichtszeile abbrechen.



timeline-summary-permission-requested = Berechtigung angefordert [{$risk}]: {$reason}
timeline-summary-permission-replied = Berechtigung beantwortet: {$kind}
timeline-summary-permission-rule-created = Berechtigungsregel #{$id} erstellt
timeline-summary-permission-rule-updated = Berechtigungsregel #{$id} aktualisiert
timeline-summary-permission-rule-revoked = Berechtigungsregel #{$id} widerrufen
timeline-detail-permission-session-id = session_id: {$id}
timeline-detail-permission-request-id = request_id: {$id}
timeline-detail-permission-reason = Grund: {$reason}
timeline-detail-permission-risk = Risiko: {$risk}
timeline-detail-permission-explanation = Erläuterung: {$explanation}
timeline-detail-permission-source = Quelle: {$source}
timeline-detail-permission-scope = Geltungsbereich: {$scope}
timeline-detail-permission-operator = Anwender: {$operator}
timeline-detail-permission-reply-kind = Antwortart: {$kind}
timeline-detail-permission-rule-id = rule_id: {$id}
timeline-detail-permission-rule-action-key = action_key: {$action}
timeline-detail-permission-rule-mode = Modus: {$mode}
timeline-detail-permission-rule-scope = Geltungsbereich: {$scope}
timeline-detail-permission-rule-source = Quelle: {$source}
timeline-type-execution-started = Ausführung gestartet
timeline-type-execution-completed = Ausführung abgeschlossen
timeline-type-execution-cancelled = Ausführung abgebrochen
timeline-type-execution-failed = Ausführung fehlgeschlagen
timeline-type-compaction-completed = Kontextkomprimierung abgeschlossen
timeline-type-subtask-status-changed = Status der Teilaufgabe geändert
timeline-type-stream-error = Datenstromfehler
timeline-type-message-part-checkpointed = Nachrichtenteil gespeichert
timeline-type-message-part-delta = Nachrichtenteil geändert
timeline-type-command-begin = Befehlsstart
timeline-type-command-output-delta = Befehlsausgabe geändert
timeline-type-command-end = Befehlsende
timeline-type-permission-requested = Berechtigung angefordert
timeline-type-permission-replied = Berechtigung beantwortet
timeline-type-permission-rule-created = Berechtigungsregel erstellt
timeline-type-permission-rule-updated = Berechtigungsregel aktualisiert
timeline-type-permission-rule-revoked = Berechtigungsregel widerrufen
timeline-type-session-goal-updated = Sitzungsziel aktualisiert
timeline-type-run-started = Durchlauf gestartet
timeline-type-run-completed = Durchlauf abgeschlossen
timeline-type-run-aborted = Durchlauf abgebrochen
timeline-type-user-message-appended = Benutzernachricht hinzugefügt
timeline-type-assistant-message-completed = Assistentennachricht abgeschlossen
timeline-type-tool-call-issued = Werkzeugaufruf ausgelöst
timeline-type-tool-call-completed = Werkzeugaufruf abgeschlossen
timeline-type-plugin-event = Plugin-Ereignis
timeline-summary-execution-started = Sitzung #{$id}
timeline-summary-message-part-checkpointed = Nachricht #{$message_id}, Teil #{$part_id} gespeichert ({$kind})
timeline-summary-message-part-delta = Nachricht #{$message_id}, Teil #{$part_id}: {$field} (+{$count} Zeichen)
timeline-summary-command-output-bytes = {$count} Byte
timeline-summary-command-output-delta = {$stream} {$preview}
timeline-summary-command-end = {$status}, Exit-Code {$exit_code} ({$duration_ms} ms)
timeline-summary-session-goal-updated = Ziel aktualisiert: {$status} · {$objective}
timeline-summary-run-started = Durchlauf {$id}
timeline-summary-run-completed = Durchlauf {$id} ({$finish})
timeline-summary-run-aborted = Durchlauf {$id} abgebrochen ({$reason})
timeline-summary-user-message-appended = Nutzer #{$id}
timeline-summary-assistant-message-completed = Assistent #{$id} ({$finish})
timeline-summary-tool-call-issued = Werkzeug {$name}, Aufruf={$call_id}
timeline-summary-tool-call-completed = Werkzeugaufruf {$call_id} abgeschlossen
timeline-summary-system-notice-appended = System #{$message_id}: {$kind}
timeline-summary-plugin-event = Plugin {$plugin_id}/{$kind_label}
timeline-label-seq = Sequenz
timeline-label-created = Erstellt
timeline-label-type = Typ
timeline-label-event-id = Ereignis-ID
timeline-label-causation-id = Ursachen-ID
timeline-label-correlation-id = Korrelations-ID
timeline-label-session-id = Sitzungs-ID
timeline-label-error-code = Fehlercode
timeline-label-outcome = Ergebnis
timeline-label-error-message = Fehlermeldung
timeline-label-message-id = Nachrichten-ID
timeline-label-part-id = Teil-ID
timeline-label-part-kind = Teiltyp
timeline-label-status = Status
timeline-label-summary = Zusammenfassung
timeline-label-field = Feld
timeline-label-delta = Änderung
timeline-label-call-id = Aufruf-ID
timeline-label-command = Befehl
timeline-label-cwd = Arbeitsverzeichnis
timeline-label-stream = Datenstrom
timeline-label-bytes = Byte
timeline-label-preview = Vorschau
timeline-label-exit-code = Exit-Code
timeline-label-duration-ms = Dauer (ms)
timeline-label-request-id = Anfrage-ID
timeline-label-reason = Grund
timeline-label-risk = Risiko
timeline-label-explanation = Erläuterung
timeline-label-source = Quelle
timeline-label-scope = Geltungsbereich
timeline-label-operator = Anwender
timeline-label-requested-actions = Angeforderte Aktionen
timeline-label-related-actions = Zugehörige Aktionen
timeline-label-reply-kind = Antwortart
timeline-label-rule-id = Regel-ID
timeline-label-action-key = Aktionsschlüssel
timeline-label-mode = Modus
timeline-label-goal-id = Ziel-ID
timeline-label-objective = Ziel
timeline-label-completed-at = Abgeschlossen am
timeline-label-run-id = Durchlauf-ID
timeline-label-model = Modell
timeline-label-finish = Abschluss
timeline-label-message = Nachricht
timeline-label-name = Name
timeline-label-kind = Art
timeline-label-text = Text
timeline-label-plugin-id = Plugin-ID
timeline-label-kind-label = Artbezeichnung
timeline-label-payload = Nutzdaten
timeline-label-trace = Trace
timeline-label-role = Rolle
timeline-label-state = Status
timeline-label-revision = Revision
timeline-label-run = Durchlauf
timeline-label-parent-part = Übergeordneter Teil

flash-session-view-mode = Sitzungsansicht: {$mode}
flash-session-title-empty = Der Sitzungstitel darf nicht leer sein.
flash-session-renamed = Sitzung in „{$title}“ umbenannt
flash-session-favorited = „{$title}“ zu den Favoriten hinzugefügt
flash-session-unfavorited = „{$title}“ aus den Favoriten entfernt
flash-session-compacted = Sitzungskontext zusammengefasst
flash-session-rewound = Zweig erstellt, nachdem {$target} zurückgenommen wurde; Nachricht im Eingabefeld wiederhergestellt
flash-large-paste-no-file-view = Große eingefügte Textblöcke können entfernt werden, haben aber keine Dateiansicht.
flash-quit-confirm = Zum Beenden Ctrl+C erneut drücken
flash-run-cancelling-quit = Lauf wird abgebrochen; zum Beenden Ctrl+C erneut drücken
flash-config-dir-prepare-failed = Konfigurationsverzeichnis {$path} konnte nicht vorbereitet werden: {$error}
flash-config-file-create-failed = Konfigurationsdatei {$path} konnte nicht erstellt werden: {$error}
flash-clipboard-paste-failed = Einfügen aus der Zwischenablage fehlgeschlagen: {$error}
flash-clipboard-attachments-truncated = Die Zwischenablage enthält mehr Ressourcen, als diese Nachricht aufnehmen kann; die ersten {$count} wurden angehängt.
flash-attachment-count-limit = Eine Nachricht kann höchstens {$count} Ressourcen anhängen.
flash-attach-images-only = Über die Bildauswahl können nur Bilddateien angehängt werden.
flash-composer-drafts-load-failed = Entwürfe des Eingabefelds konnten nicht geladen werden: {$error}
flash-composer-drafts-save-failed = Entwürfe des Eingabefelds konnten nicht gespeichert werden: {$error}
flash-prompt-history-load-failed = Verlauf der Eingaben konnte nicht geladen werden: {$error}
flash-prompt-history-save-failed = Verlauf der Eingaben konnte nicht gespeichert werden: {$error}
flash-copied-mouse-selection = Mausauswahl in die Zwischenablage kopiert
flash-transcript-line-copied = Ausgewählte Transkriptzeile in die Zwischenablage kopiert
flash-transcript-no-selection = Wählen Sie zuerst sichtbaren Transkriptinhalt durch Klicken oder mit einer Navigationstaste aus.
flash-transcript-exported = Transkript nach {$path} exportiert
flash-transcript-export-failed = Transkript konnte nicht exportiert werden: {$error}
flash-transcript-pager-failed = Transkript-Pager fehlgeschlagen: {$error}
flash-timeline-event-copied = Ausgewähltes Zeitleistenereignis kopiert
flash-transcript-node-expanded = {$kind} aufgeklappt
flash-transcript-node-collapsed = {$kind} eingeklappt
flash-transcript-parts-revealed = Bis zu {$count} weitere Teile werden geladen
flash-transcript-node-copied = Aktuellen {$kind} kopiert
flash-plan-viewer-requires-session = Für die Planansicht muss eine Sitzung geöffnet sein.
flash-plan-viewer-no-plan = Für diese Sitzung ist kein Plan geladen.
flash-snapshot-ready = Momentaufnahme bereit: {$path} ({$branch})
flash-snapshot-attached = Momentaufnahme angehängt: {$path} ({$branch})
flash-snapshot-exited = Momentaufnahme {$action}: {$path}
flash-commit-created = Commit erstellt: {$sha} {$summary}
flash-pr-created = Pull Request erstellt: {$url}
flash-attachment-inspect-failed = Anhang {$path} konnte nicht geprüft werden: {$error}
flash-session-awaiting-approval = Sitzung wartet auf eine Freigabe
flash-session-awaiting-plan-approval = Sitzung wartet auf die Planfreigabe
flash-session-awaiting-user-input = Sitzung wartet auf eine Benutzereingabe
flash-no-permission-request = Keine ausstehende Berechtigungsanfrage
flash-no-user-input-request = Keine ausstehende Benutzereingabe
flash-plugins-enabled = Plugins aktiviert
flash-plugins-disabled = Plugins deaktiviert
flash-plugin-config-not-object = Die Plugin-Konfiguration {$plugin_id} ist kein JSON-Objekt und kann nicht neu geschrieben werden.
flash-plugin-config-enabled = Plugin {$plugin_id} aktiviert; Konfiguration beibehalten und Laufzeit neu geladen
flash-plugin-config-disabled = Plugin {$plugin_id} deaktiviert; Konfiguration beibehalten und Laufzeit neu geladen
flash-tool-description-mode-set = Beschreibungsmodus der Werkzeuge auf {$mode} gesetzt
flash-plugin-ui-display-mode-set = Anzeigemodus der Plugin-Oberfläche auf {$mode} gesetzt
flash-model-selected = Modell ausgewählt: {$model}
flash-model-cleared = Modellüberschreibung entfernt
flash-model-provider-required = Anbieter erforderlich; wählen Sie zuerst einen Anbieter oder verwenden Sie anbieter/modell.
flash-provider-studio-catalog-template-applied = Modellfelder aus {$model} übernommen; zum Beibehalten der Werte speichern
flash-thinking-mode-set = Denkmodus auf {$mode} gesetzt
flash-thinking-mode-cleared = Überschreibung des Denkmodus entfernt
flash-thinking-mode-required = Denkmodus muss ausgewählt werden
flash-speed-mode-set = Geschwindigkeitsmodus auf {$mode} gesetzt
flash-speed-mode-cleared = Überschreibung des Geschwindigkeitsmodus entfernt
flash-speed-mode-required = Name des Geschwindigkeitsmodus erforderlich
flash-parent-session-missing = Die aktuelle Sitzung hat keine übergeordnete Sitzung.
flash-command-usage = Verwendung: {$usage}
flash-command-does-not-support-attachments = Slash-Befehle unterstützen keine vorgemerkten Anhänge.
flash-send-in-flight = Die vorherige Nachricht wird noch gesendet; die neue Nachricht bleibt im Eingabefeld.
flash-answer-pending-request = Beantworten Sie zuerst die ausstehende Anfrage; die Nachricht bleibt im Eingabefeld.
flash-message-queued = Nachricht vorgemerkt — wird gesendet, sobald der aktuelle Lauf beendet ist
flash-message-replaced = Es war bereits eine Nachricht vorgemerkt — sie wurde ersetzt
flash-steer-sent = Nachricht in den laufenden Vorgang eingebracht
flash-steer-failed-fallback-queue = Der laufende Vorgang ließ sich nicht steuern; die Nachricht bleibt vorgemerkt.
flash-run-cancelled = Lauf abgebrochen
flash-cancel-failed = Abbrechen fehlgeschlagen
flash-user-command-empty = Der Inhalt des Benutzerbefehls ist leer.
flash-side-requires-session = Öffnen Sie eine Sitzung, bevor Sie ein Nebengespräch starten.
flash-side-already-open = Es ist bereits ein Nebengespräch geöffnet.
flash-memory-forgotten = Erinnerung entfernt: {$name}
flash-no-pending-message = Keine ausstehende Nachricht
flash-pending-cancelled = Ausstehende Nachricht abgebrochen
flash-command-prefill-requires-empty-composer = Leeren Sie das Eingabefeld, bevor Sie einen Befehl vorab einfügen.
flash-temperature-set = Temperatur auf {$value} gesetzt
flash-temperature-cleared = Temperaturüberschreibung entfernt
flash-temperature-invalid = Ungültiger Temperaturwert
flash-verbosity-set = Ausführlichkeit auf {$value} gesetzt
flash-verbosity-cleared = Überschreibung der Ausführlichkeit entfernt
flash-parallel-tool-calls-set = Parallele Werkzeugaufrufe auf {$value} gesetzt
flash-parallel-tool-calls-cleared = Überschreibung paralleler Werkzeugaufrufe entfernt
flash-max-output-set = Maximale Ausgabetokenzahl auf {$value} gesetzt
flash-max-output-cleared = Überschreibung der maximalen Ausgabetokenzahl entfernt
flash-max-output-invalid = Ungültige maximale Ausgabetokenzahl
flash-system-set = System-Prompt-Überschreibung aktualisiert
flash-system-cleared = System-Prompt-Überschreibung entfernt
flash-runtime-status = Sitzungsstatus: {$summary}
flash-diagnostics-summary = Diagnose: cwd={$cwd} Sitzung={$session} Warteschlange={$queue} Laufzeit={$runtime}
flash-session-permission-updated = Sitzungsberechtigung aktualisiert.
session-view-all = alle
session-view-roots = Wurzeln
session-view-subtree = Teilbaum
session-view-subtree-root = Teilbaum #{$id}
session-awaiting-approval = wartet auf Freigabe
session-awaiting-plan-approval = wartet auf Planfreigabe
session-awaiting-user-input = wartet auf Eingabe
session-awaiting-model = wartet auf Modell
session-idle = inaktiv
session-tag-parent = übergeordnet
session-tag-child = untergeordnet
session-tag-current = aktuell
session-tag-ancestor = Vorfahr
session-tag-sibling = Geschwister
session-tag-leaf = Blatt
session-summary-root = Wurzel #{$id}
session-summary-depth = Tiefe {$depth}
session-summary-side-branches = {$count} Zweige
session-summary-descendants = {$count} Nachfolger
session-summary-parent = übergeordnet #{$id}
session-summary-children = {$count} untergeordnete Sitzung
session-status-plan = Plan {$slug}
session-status-plan-with-steps = Plan {$slug} · {$steps} Schritte
session-status-thinking = {$value}
session-status-speed = {$value}
session-work-tasks = Aufgaben
session-work-files = Dateien
session-work-requests = Anfragen
session-work-waiting = Wartet auf Werkzeuge
session-work-status = Ausführungsstatus
session-work-workspace = In der Sitzung aufgezeichnete Änderungen · wörtliche Werkzeugpfade
session-work-ready = Bereit
session-work-back = Zurück
session-work-switch = Wechseln →
session-work-staged = Vorgemerkt ⇄
session-work-working = Arbeitsbaum ⇄
session-work-stop = Stoppen
session-work-pause = Pausieren
session-work-resume = Fortsetzen
session-work-delete = Löschen
session-work-dismiss = Schließen
session-work-log-tail = Letzte 200 Zeilen / 128 KiB
session-work-empty = Keine Inhalte in dieser Ansicht
session-work-footer = Zur Eingabe zurückkehren
session-work-more-diff = Weitere Diff-Inhalte laden
session-work-status-pending = In Warteschlange
session-work-status-running = Läuft
session-work-status-waiting = Wartet
session-work-status-paused = Pausiert
session-work-status-succeeded = Abgeschlossen
session-work-status-failed = Fehlgeschlagen
session-work-status-cancelled = Abgebrochen
session-work-status-stopped = Angehalten
session-work-next-wake = Nächstes Aufwachen:
session-work-title = Sitzung
session-work-loading = Dateien werden geladen
session-work-previous = ‹ Zurück
session-work-next = Weiter ›
session-work-recording-incomplete = Aufzeichnung möglicherweise unvollständig (Shell-Schreibvorgänge sind Deklarationen)
session-work-operation-history = Vorgangsverlauf, kein Netto-Diff der Sitzung (keine vollständige Basis).
session-work-operation-diff = Gesamter Diff des Patch-Vorgangs (kann andere Dateien enthalten).
session-work-recorded-truncated = Diff gekürzt; nur gespeicherte Inhalte können geladen werden.
session-work-no-recorded-diff = Kein aufgezeichneter Text-Diff.

workbench-action-add-field = Feld hinzufügen
workbench-action-add-field-desc = Neues Feld in { $title } hinzufügen.
workbench-action-add-item = Element hinzufügen
workbench-action-add-item-desc = Neues Standardelement an { $title } anhängen.
workbench-action-duplicate-item = Element duplizieren
workbench-action-duplicate-item-desc = { $title } innerhalb dieses Arrays duplizieren.
workbench-action-field = Feldaktionen
workbench-action-insert-after = Danach einfügen
workbench-action-insert-after-desc = Nach diesem Array-Element ein neues Standardelement einfügen.
workbench-action-insert-before = Davor einfügen
workbench-action-insert-before-desc = Vor diesem Array-Element ein neues Standardelement einfügen.
workbench-action-more = Weitere Aktionen
workbench-action-move-down = Nach unten verschieben
workbench-action-move-down-desc = Dieses Array-Element um eine Position nach hinten verschieben.
workbench-action-move-up = Nach oben verschieben
workbench-action-move-up-desc = Dieses Array-Element um eine Position nach vorne verschieben.
workbench-action-remove-item = Element entfernen
workbench-action-remove-item-desc = { $title } aus diesem Array entfernen.
workbench-action-rename-field = Feld umbenennen
workbench-action-rename-field-desc = Schlüssel für { $title } umbenennen.
workbench-action-reset-field = Feld zurücksetzen
workbench-action-reset-field-desc = { $title } auf den Plugin-Standardwert zurücksetzen.
workbench-action-reset-group = Gruppe zurücksetzen
workbench-action-reset-group-desc = Alle Felder in { $title } auf die Plugin-Standardwerte zurücksetzen.
workbench-action-switch-shape-desc = Aktive Form für { $title } wechseln.
workbench-action-switch-type-desc = Aktiven Typ für { $title } wechseln.
workbench-editor-placeholder-type-to-edit = Zum Bearbeiten tippen
workbench-editor-prompt-add-field = Feldnamen für { $path } eingeben. Erlaubt das Schema mehrere Werttypen oder Formen, fragt der Editor nach dem Erstellen danach.
workbench-editor-prompt-new-field-name = Neuen Feldnamen eingeben.
workbench-editor-title-add-field = Feld hinzufügen
workbench-editor-title-edit = { $title } bearbeiten
workbench-editor-title-edit-with-label = { $title } bearbeiten · { $label }
workbench-editor-title-rename = { $title } umbenennen
workbench-error-append-array-item = Array-Element konnte nicht angehängt werden
workbench-error-args-invalid-json = Die Tool-Argumente sind kein gültiges JSON.
workbench-error-args-not-object = Plugin-Tool-Argumente müssen ein JSON-Objekt sein
workbench-error-args-schema-mismatch = Die Tool-Argumente entsprechen nicht dem Plugin-Schema.
workbench-error-config-update = Die Plugin-Konfiguration konnte nicht aktualisiert werden.
workbench-error-tool-input-prepare = Die Plugin-Tool-Eingabe konnte nicht vorbereitet werden. Diagnoseprotokoll prüfen und erneut versuchen.
workbench-error-tool-requires-session = Das Plugin-Tool benötigt eine aktive Sitzung.
workbench-error-tool-unavailable = Das Plugin-Tool ist nicht mehr verfügbar.
workbench-picker-choose-many = Einen oder mehrere Werte auswählen
workbench-picker-choose-one = Einen Wert auswählen
workbench-picker-no-actions = Keine Aktionen verfügbar
workbench-picker-no-choices = Keine Auswahlmöglichkeiten verfügbar
workbench-picker-space-toggle = Leertaste: umschalten
workbench-picker-title-select = { $title } auswählen
workbench-select-branch-prompt = Schemaform auswählen
workbench-select-branch-title = Zweig auswählen
workbench-select-type-prompt = JSON-Typ für { $path } auswählen
workbench-select-type-title = Typ auswählen
workbench-tool-completed = Plugin-Tool abgeschlossen
workbench-tool-completed-no-output = Tool erfolgreich ohne Ausgabe abgeschlossen.
workbench-tool-default-run = Dieses Plugin-Tool ausführen.
workbench-tool-editor-description = { $summary } Tags: { $tags }. JSON-Argumente unten bearbeiten. Das Absenden genehmigt einmalig genau diesen Tool-Aufruf; gespeicherte Ablehnungsregeln gelten weiterhin.
workbench-tool-editor-placeholder = Ctrl+S: prüfen und ausführen · Esc: abbrechen · Enter: Zeilenumbruch
workbench-tool-editor-title = { $plugin } / { $tool } ausführen
workbench-tool-tags-none = keine angegeben
workbench-warning-add-requires-object-or-array = „Hinzufügen“ ist für Objekt- und Array-Knoten verfügbar
workbench-warning-cannot-add-item = An dieser Array-Position kann kein weiteres Element hinzugefügt werden
workbench-warning-cannot-duplicate-item = Dieses Array-Element kann nicht dupliziert werden
workbench-warning-cannot-insert-item = An dieser Array-Position kann kein Element eingefügt werden
workbench-warning-cannot-remove-item = Dieses Array-Element kann nicht entfernt werden
workbench-warning-no-tools = Dieses Plugin stellt keine Tools bereit
workbench-warning-root-config-not-deletable = Die Stammkonfiguration kann nicht gelöscht werden
workbench-warning-row-not-object-field = Die ausgewählte Zeile verweist nicht auf ein Objektfeld

hub-action-favorite = Als Favorit markieren
hub-action-unfavorite = Aus Favoriten entfernen
hub-section-favorites = Favoriten
composer-status-pending-user-input = {$count} Eingaben ausstehend (Ctrl+R)
composer-status-pending-approval = {$count} Genehmigungen ausstehend (Ctrl+L)
composer-status-hidden-pending-dialog = Dialog mit ausstehender Eingabe während des Schreibens ausgeblendet
composer-status-selected-item = Element {$current}/{$total} {$label}
composer-status-history = Verlauf {$current}/{$total}
composer-status-history-query = Verlauf {$current}/{$total} Suche={$query}
composer-status-mention = Erwähnung {$query}
composer-status-slash = Befehl {$query}
composer-status-files = ±{$count} Dateien
composer-status-tasks = ●{$count} Aufgaben
composer-status-btw = BTW {$count}
surface-mode-insert = EINFÜGEN
surface-mode-navigate = NAVIGIEREN
surface-mode-select = AUSWÄHLEN
runtime-status-keys = Tasten Warteschlange={$queue} Senden={$send}
runtime-status-display = Anzeige {$value}
runtime-status-display-default = Standard
runtime-status-display-plugin = Plugin
runtime-status-tui-blocks = TUI-Blöcke {$count}
runtime-status-theme = Design {$value}
run-options-summary-thinking = Denken {$value}
run-options-summary-speed = Geschwindigkeit {$value}
run-options-summary-verbosity = Ausführlichkeit {$value}
run-options-summary-parallel-tools = Parallele Werkzeuge {$value}
help-sessions-line-4 = /rename, /timeline und /lineage verwalten die ausgewählte Sitzung
help-sessions-line-5 = Mit den Steuerelementen des Sitzungswechslers zwischen den Bereichen Alle, Stamm und Teilbaum wechseln
help-transcript-line-8 = /rename, /timeline, /lineage, /settings, /export oder /pager für Sitzungsaktionen und Einstellungen verwenden
help-composer-line-11 = Ctrl+G öffnet verfügbare Inline-Elemente; Schrägstrichbefehle wie /model oder /rewind lassen sich im Eingabefeld ausführen
help-section-user-commands = Benutzerbefehle
plan-viewer-refresh = Aktualisieren
plan-viewer-title = Plan
plan-viewer-empty = Noch kein Plan vorhanden.
plan-viewer-loading = Plan wird geladen
plan-viewer-footer = ↑/↓/PgUp/PgDn blättern · r aktualisieren · a automatisch ausführen · q schließen
plan-viewer-autorun-on = automatische Ausführung: an
plan-viewer-autorun-off = automatische Ausführung: aus
runtime-setting-choice-parallel-detail = Eine Überschreibung für parallele Werkzeugaufrufe anwenden
runtime-setting-summary-override-value = Überschreibung {$value}
runtime-setting-apply-cleared = {$field} zurückgesetzt
runtime-setting-apply-updated = {$field} aktualisiert
runtime-setting-error-bool = {$field} erwartet true/false, on/off, yes/no oder 1/0
runtime-setting-error-number = {$field} erwartet einen Zahlenwert
runtime-setting-error-finite = {$field} erwartet eine endliche Zahl
runtime-setting-error-positive-int = {$field} erwartet eine positive Ganzzahl
composer-prompt-history-label = Verlauf>
composer-prompt-history-no-matches =   Kein passender Eingabeverlauf
insert-content-file-label = Datei
insert-content-file-detail = Eine Datei oder einen Ordner aus dem Arbeitsbereich anhängen.
provider-field-auth-status = Authentifizierungsstatus
overlay-timeline-events = Ereignisse
status-part-workspace = Arbeitsbereich {$value}
status-part-state = Status={$value}
status-part-agent = Agent={$value}
status-part-task = Aufgabe={$value}
status-part-model = Modell={$value}
status-part-thinking = Denken={$value}
status-part-speed = Geschwindigkeit={$value}
status-part-verbosity = Ausführlichkeit={$value}
status-part-parallel-tools = parallele_Werkzeuge={$value}
status-part-cwd = Arbeitsverzeichnis={$value}
status-part-tools = Werkzeuge={$count}
status-part-permission = Berechtigung={$value}
status-part-permissions = Berechtigungen={$count}
status-part-user-input = Eingaben={$count}
inline-fact-provider = Anbieter
inline-fact-adapter = Adapter
inline-fact-model = Modell
inline-fact-thinking = Denken
inline-fact-speed = Geschwindigkeit
inline-fact-verbosity = Ausführlichkeit
inline-fact-parallel-tools = parallele Werkzeuge
permission-rule-error-session-token-required = Der Sitzungsbereich erfordert session=<id>
permission-rule-error-invalid-shell-args = Ungültige Argumente im Shell-Format
permission-rule-error-expected-structured = Eine strukturierte Regel erwartet, die mit tool/path beginnt und mit allow|ask|deny endet
permission-rule-error-unknown-option = Unbekannte Berechtigungsregeloption: {$key}
permission-rule-error-invalid-subject = Das Regelziel muss mit `tool`, `path` oder `network` beginnen
permission-rule-error-invalid-mode = Der Berechtigungsmodus muss allow, ask oder deny sein
permission-rule-error-invalid-scope = Der Geltungsbereich muss session, workspace oder global sein
permission-rule-error-invalid-option-format = Schlüssel=Wert-Option erwartet, erhalten: `{$token}`
operation-search-heading = Suche: {$query}
operation-search-results-heading = Suchergebnisse
operation-command-exit-code = Beendigungscode {$code}
operation-diff-summary = Diff: {$count} Zeilen
operation-diff-summary-detailed-one = Diff: 1 Datei, +{$added} -{$deleted}
operation-diff-summary-detailed-one-rename = Diff: 1 Datei, +{$added} -{$deleted}, {$renamed} umbenannt
operation-diff-summary-detailed = Diff: {$files} Dateien, +{$added} -{$deleted}
operation-diff-summary-detailed-renames = Diff: {$files} Dateien, +{$added} -{$deleted}, {$renamed} umbenannt
operation-nested-task-summary = {$title} ({$status})
snapshot-action-keep = behalten
snapshot-action-remove = entfernt
snapshot-action-exit = beendet
composer-placeholder-range-invalid = Ungültiger Platzhalterbereich im Eingabefeld
composer-placeholder-out-of-sync = Platzhalterstatus im Eingabefeld ist nicht synchron
composer-missing-staged-item = Bereitgestelltes Element für {$placeholder} fehlt
runtime-status-default = Sitzungsvorgaben
file-change-moved = verschoben
permission-label-auto-approve = automatisch genehmigen
failure-detail-section-title = Fehlerdetails
terminal-title-working = Wird ausgeführt
terminal-title-permission = Genehmigung ausstehend
terminal-title-user-input = Eingabe ausstehend
terminal-title-interrupted = Unterbrochen
terminal-notification-working = Wird ausgeführt
terminal-notification-permission = Genehmigung ausstehend
terminal-notification-user-input = Eingabe ausstehend
terminal-notification-interrupted = Unterbrochen
hub-section-directories = Verzeichnisse
hub-section-pinned = Angeheftet
hub-action-directory = Verzeichnis einklappen
hub-action-pin = Anheften/ lösen
hub-directory-more = + Weitere Sitzungen anzeigen ({ $count } verbleibend)
tool-technical-details = Technische Details
tool-presentation-data = Darstellungsdaten
tool-detail-input = Eingabe
tool-detail-output = Ausgabe
tool-detail-metadata = Metadaten
hub-section-search = Suchergebnisse
hub-page-next = → Nächste Seite ({ $page })
hub-page-previous = ← Vorherige Seite ({ $page })
btw-title = BTW · separate Frage ohne Änderungen
btw-description = Stelle eine separate Frage, während das Hauptgespräch weiterläuft. Nur lesende Werkzeuge; keine Änderungen am Hauptgespräch.
btw-send = Fragen
btw-stop = Anhalten
btw-loading = Antwort wird erstellt…
side-return = Nebenansicht · zum übergeordneten Gespräch zurückkehren
btw-input = BTW-Frage · Enter senden · Shift+Enter neue Zeile
btw-clear = Leeren
btw-copy = Kopieren
btw-copied = BTW-Antwort kopiert
btw-leave = Zurück zum Gespräch
btw-stopped = Angehalten · Teilantwort bleibt erhalten

transcript-rich-fallback = Vorschau nicht verfügbar: {$reason}. Der Quelltext folgt.
transcript-interactive-app = Öffnen Sie diese App in Agena Web, um Oberfläche und Dienste zu verwenden.

overlay-user-input-defer = Später bearbeiten

# Durable error Part headline and live retry progress.
error-part-waiting = Warten auf Wiederholung
error-part-retrying = Erneuter Versuch
error-part-recovered = Wiederhergestellt
error-part-failed = Wiederholung fehlgeschlagen
error-part-cancelled = Wiederholung abgebrochen
error-part-retry-count = Wiederholung { $attempt }/{ $max }
error-part-retry-number = Wiederholung { $attempt }
error-part-in-duration = Wiederholung in { $duration }
error-part-scheduled-at = um { $time }
error-part-awaiting-start = Warten auf den Start der Wiederholung
error-part-requesting = Warten auf Antwort
error-part-streaming = Antwort wird empfangen
error-part-elapsed = { $duration } vergangen
error-part-recovered-after = Nach { $attempt } Wiederholungen wiederhergestellt
error-part-exhausted = Alle { $max } Wiederholungen verbraucht
error-part-stopped = Automatische Wiederholung beendet
error-part-seconds = { $count } s
error-part-minutes = { $minutes } min { $seconds } s
error-part-hours = { $hours } h { $minutes } min

# Settings page navigation and search
settings-search-placeholder = Einstellungen suchen…
settings-search-empty = Keine passenden Einstellungen
settings-search-empty-hint = Anderen Namen oder Konfigurationspfad versuchen. Esc löscht die Suche.
settings-key-results = Ergebnisse
settings-key-clear = Löschen
settings-key-sections = Bereiche
settings-key-items = Einträge
settings-key-open = Öffnen
settings-key-search = Suchen
settings-key-details = Details
settings-key-refresh = Aktualisieren
settings-key-back = Zurück
settings-details-footer = ↑/↓ scrollen · PgUp/PgDn blättern · Esc zurück

## Compact session browsing
session-action-pin = anheften
session-action-unpin = lösen
session-action-page = Seite
session-action-scroll = scrollen
session-page-previous = zurück
session-page-next = weiter
session-hub-guide = Enter öffnen/ausklappen · p anheften · f Favorit · Ctrl+R laden
session-directory-guide = Enter Verzeichnis auf-/zuklappen
session-search-guide = / oder Ctrl+F: Titel oder ID suchen
session-search-input-hints = ←→ Cursor · Tab Liste · Alt+P anheften · Alt+F Favorit · Esc Liste
session-search-results-hints = ↑↓ Auswahl · ←→ / PgUp/Dn Seite · / Suche · Enter öffnen · Esc zurück
session-page-status = Seite { $page }/{ $pages }{ $more } · { $count } geladen
session-bucket-page = Seite { $page }
session-search-compact-hints = ↑↓ · ←→ Seite · Enter öffnen · Esc
