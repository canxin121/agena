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

transcript-header-lines = Zeilen {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = suche={$query} ({$current}/{$total})
transcript-header-tail = am Ende folgen
transcript-header-loading = laedt
transcript-header-loading-older = laedt aeltere Nachrichten
transcript-header-busy = beschaeftigt
transcript-loading-older = Aeltere Nachrichten werden geladen...
transcript-more-older = Aeltere Nachrichten sind verfuegbar. Nach oben scrollen oder PageUp druecken.
transcript-empty-session = In dieser Sitzung gibt es noch keine Nachrichten.

no-session-selected = Keine Sitzung ausgewaehlt.
no-session-selected-hint = Alt+S waehlt eine Sitzung, oder tippen Sie direkt in die Eingabe, um eine neue Sitzung zu erstellen.
composer-session-new = neue Sitzung
composer-placeholder = Nachricht an Agena. Alt+Up Verlauf. / Befehle. F3 Datei.

status-global = Alt+S Sitzungen | Alt+P Befehle | ? Hilfe | q/Ctrl+C beenden
status-sessions = Sitzungen: Alt+S Wechsler | /sessions [Suche] | /search [Suche]
status-transcript = Transkript: j/k scrollen | / suchen | c letzte kopieren | y kopieren | v Pager
status-composer = Eingabe: Ctrl+Enter jetzt senden | Ctrl+J neue Zeile | Alt+Up/Down Verlauf | / Befehle | Tab Chat

help-title = Hilfe
help-header = Agena TUI
help-section-sessions = Sitzungswechsler
help-sessions-line-1 = Alt+S oeffnet den durchsuchbaren Sitzungswechsler
help-sessions-line-2 = Up/Down, PageUp/PageDown bewegen die Auswahl
help-sessions-line-3 = Enter oeffnet die ausgewaehlte Sitzung
help-section-transcript = Transkriptfenster
help-transcript-line-1 = j/k oder Pfeile scrollen
help-transcript-line-2 = Space / Shift+Space / Ctrl+F / Ctrl+B blaettern
help-transcript-line-3 = Ctrl+D / Ctrl+U halbe Seite
help-transcript-line-4 = PageUp nahe dem oberen Rand laedt aeltere Nachrichten
help-transcript-line-5 = g/G springt zum Anfang oder Ende
help-transcript-line-6 = / oder Ctrl+F durchsucht das geladene Transkript, n/N springt zwischen Treffern
help-transcript-line-7 = c kopiert die letzte Assistant-Nachricht, y das geladene Transkript, Y den sichtbaren Bereich
help-section-composer = Eingabe
help-composer-line-1 = Enter sendet
help-composer-line-2 = Alt/Shift+Enter oder Ctrl+J fuegt einen Zeilenumbruch ein
help-composer-line-3 = Ctrl+A/E/B/F/P/N bewegen, Alt+B/F oder Alt/Ctrl+Left/Right springen wortweise
help-composer-line-4 = Ctrl+H/D/W/U/K/Y bearbeiten wie in Shell oder Editor
help-composer-line-5 = An Zeilengrenzen kann Ctrl+A/E zur vorherigen/naechsten Zeile weitergehen
help-composer-line-6 = F3, Ctrl+O oder Alt+O suchen Workspace-Dateien zum Anhaengen
help-composer-line-7 = F4 oder Alt+E oeffnen $VISUAL/$EDITOR fuer die Eingabe
help-composer-line-8 = F6 oder Alt+I haengen ein Zwischenablagebild an
help-composer-line-9 = Ein einzelner Dateipfad wird direkt angehaengt, grosse Einfuegungen werden zu Inline-Platzhaltern und Anhaenge bleiben atomar
help-composer-line-10 = Alt+Up/Down ruft gesendete Prompts ab; Alt+P oeffnet die Befehlspalette
help-section-actions = Aktionen
help-actions-line-1 = n erstellt eine Sitzung
help-actions-line-2 = r setzt eine blockierte oder wartende Sitzung fort
help-actions-line-3 = a/A/d/D antworten auf die erste offene Berechtigungsanfrage
help-actions-line-4 = u oeffnet die erste offene Benutzereingabeanfrage
help-actions-line-5 = Mouse Capture ist deaktiviert, damit normale Terminal-Auswahl und Kopieren weiter funktionieren
help-actions-line-6 = q oder Ctrl+C beendet

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
overlay-user-input-reply-format = Antwortformat: question_id=value;other_id=value1,value2
overlay-user-input-cancel-hint = Ctrl+D bricht die Anfrage ab
overlay-user-input-footer = Ctrl+D abbrechen

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

message-role-user = benutzer
message-role-assistant = assistent
message-role-system = system

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed

message-parts-not-loaded = {$count} Teile nicht geladen
message-usage = Nutzung: in={$input} out={$output} reasoning={$reasoning}
message-finish = finish: {$finish}
message-empty = (leere Nachricht)
message-thinking = Denken: {$summary}
message-command-status = Status: {$status}, exit={$exit}
message-file-changes = Dateiaenderungen
message-search = Suche: {$query}
message-todo-list = Aufgabenliste
message-error = Fehler [{$code}]: {$message}
message-attachments = Anhaenge
message-awaiting-user-input = Warten auf Benutzereingabe: {$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = Teildetails nicht verfuegbar
message-tool-pending = wartet: {$label}
message-tool-running = laeuft: {$label}
message-tool-done = fertig: {$label}
message-tool-failed = fehlgeschlagen: {$label}
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

permission-summary-pending = Warten auf Berechtigung: {$reason}
permission-summary-allow-once = Einmal erlaubt: {$reason}
permission-summary-allow-always = Immer erlaubt: {$reason}
permission-summary-deny-once = Einmal abgelehnt: {$reason}
permission-summary-deny-always = Immer abgelehnt: {$reason}
