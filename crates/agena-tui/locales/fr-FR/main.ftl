cli-about = Application de chat terminal Agena

pane-sessions = Sessions
pane-sessions-search = Sessions [{$query}]
pane-transcript = Transcript
pane-messages = Messages
pane-composer = Saisie [{$session}]

session-meta = #{$id}  {$message_count} msg  {$updated}
session-running = en cours
sessions-empty = Aucune session trouvee
sessions-loading-more = Chargement de sessions supplementaires...
sessions-more = D'autres sessions sont disponibles

transcript-header-lines = lignes {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = recherche={$query} ({$current}/{$total})
transcript-header-tail = suivi fin
transcript-header-loading = chargement
transcript-header-loading-older = chargement des anciens messages
transcript-header-busy = occupe
transcript-loading-older = Chargement des anciens messages...
transcript-more-older = D'anciens messages sont disponibles. Faites defiler vers le haut ou appuyez sur PageUp.
transcript-empty-session = Aucun message dans cette session pour le moment.

no-session-selected = Aucune session selectionnee.
no-session-selected-hint = Utilisez /sessions pour choisir une session, ou commencez a saisir dans la zone de composition pour en creer une.
composer-session-new = nouvelle session
composer-placeholder = Message pour Agena. Haut au debut ouvre l'historique. / commandes. Ctrl+O fichier.

status-global = / cherche en bas | ? cherche en haut | Ctrl+C deux fois quitte
status-sessions = Sessions: /sessions
status-transcript = VIEW: i saisie | j/k defile | / cherche | c copie dernier | y copie
status-composer = INSERT: Esc retour | Ctrl+Enter envoie maintenant | Ctrl+J nouvelle ligne | Haut au debut historique | Ctrl+Up file | / commandes | Ctrl+G items | Ctrl+R entree | Ctrl+L approbation

help-title = Aide
help-header = Agena TUI
help-section-sessions = Selecteur de sessions
help-sessions-line-1 = /sessions ouvre le selecteur de sessions avec recherche
help-sessions-line-2 = Up/Down, PageUp/PageDown deplacent la selection
help-sessions-line-3 = Enter ouvre la session selectionnee
help-section-transcript = Panneau du transcript
help-transcript-line-1 = i passe en INSERT ; j/k ou les fleches font defiler
help-transcript-line-2 = Space / Shift+Space / Ctrl+B paginent
help-transcript-line-3 = Ctrl+D / Ctrl+U demi-page
help-transcript-line-4 = PageUp pres du haut charge les anciens messages
help-transcript-line-5 = g/G saute au debut ou a la fin
help-transcript-line-6 = / recherche vers le bas et ? vers le haut ; n continue et N inverse le sens
help-transcript-line-7 = c copie le dernier message assistant, y copie le transcript charge, Y copie la zone visible
help-section-composer = Zone de composition
help-composer-line-1 = Esc revient en VIEW ; Enter envoie
help-composer-line-2 = Shift+Enter ou Ctrl+J insere une nouvelle ligne
help-composer-line-3 = Ctrl+A/E/B/F/P/N se deplacent, Ctrl+Left/Right sautent par mot
help-composer-line-4 = Ctrl+H/D/W/U/K/Y editent comme un shell ou un editeur
help-composer-line-5 = A une limite de ligne, Ctrl+A/E peut continuer vers la ligne precedente/suivante
help-composer-line-6 = Ctrl+O recherche des fichiers du workspace a joindre
help-composer-line-7 = Ctrl+E ouvre $VISUAL/$EDITOR pour la composition
help-composer-line-8 = Ctrl+T joint une image du presse-papiers
help-composer-line-9 = Le texte colle est insere directement ; un chemin de fichier unique est joint et les pieces jointes restent atomiques
help-composer-line-10 = Haut ouvre l'historique quand le curseur est au debut ; Ctrl+Up recupere un message en attente
help-section-actions = Actions
help-actions-line-1 = Ctrl+N cree une session ; n/N parcourt les resultats de recherche
help-actions-line-2 = r continue une session bloquee ou en attente ; U ouvre les statistiques d’utilisation
help-actions-line-3 = a/A/d/D repondent a la premiere demande d'autorisation en attente
help-actions-line-4 = Ctrl+R ouvre la premiere demande d'entree en attente depuis la composition
help-actions-line-5 = La capture de souris est desactivee pour conserver la selection/copie du terminal
help-actions-line-6 = Ctrl+C deux fois quitte

overlay-session-search-title = Recherche de session
overlay-session-search-prompt = Rechercher dans les titres de session
overlay-transcript-search-title = Recherche du transcript
overlay-transcript-search-prompt = Rechercher dans les messages charges
overlay-line-footer = Saisissez pour modifier

overlay-attach-title = Joindre un fichier
overlay-attach-prompt = Saisissez un chemin ou un terme de recherche. Enter joint le fichier selectionne.
overlay-attach-no-match = Aucun fichier correspondant
overlay-attach-matches = Correspondances
overlay-attach-footer = Tab remplit le chemin

overlay-user-input-title = Entree utilisateur en attente
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = valeur personnalisee autorisee
overlay-user-input-reply-format = Format de reponse : question_id=value;other_id=value1,value2
overlay-user-input-cancel-hint = Ctrl+X annule la demande
overlay-user-input-footer = Ctrl+X annuler

flash-terminal-event-error = erreur d'evenement terminal : {$error}
flash-created-session = session creee {$title}
flash-permission-reply-sent = reponse d'autorisation envoyee : {$label}
flash-user-input-reply-sent = reponse utilisateur envoyee
flash-large-paste-staged = grand collage place dans la composition
flash-attached = {$path} joint
flash-composer-updated = composition mise a jour depuis l'editeur externe
flash-prompt-history-empty = l'historique des prompts est vide
flash-prompt-history-items = retirez les pieces jointes ou collages prepares avant de rappeler l'historique des prompts
flash-external-editor-failed = echec de l'editeur externe : {$error}
flash-clipboard-image-attached = image du presse-papiers jointe : {$width}x{$height} {$format}
flash-clipboard-image-attach-failed = echec de la jointure de l'image du presse-papiers : {$error}
flash-no-loaded-transcript = aucun transcript charge a copier
flash-copied-loaded-transcript = transcript charge copie dans le presse-papiers
flash-no-assistant-message = aucun message assistant a copier
flash-no-assistant-message-text = le dernier message assistant n'a pas de texte charge a copier
flash-copied-assistant-message = dernier message assistant copie dans le presse-papiers
flash-no-visible-transcript = aucun texte visible a copier
flash-copied-visible-transcript = zone visible copiee dans le presse-papiers
flash-clipboard-copy-failed = echec de la copie vers le presse-papiers : {$error}

message-role-user = utilisateur
message-role-assistant = assistant
message-role-system = systeme

message-state-pending = pending
message-state-in-progress = in_progress
message-state-completed = completed
message-state-failed = failed
message-state-policy-denied = blocked by permission policy
message-state-user-declined = declined by user
message-state-capability-unavailable = capability unavailable
message-state-tool-unavailable = tool unavailable

message-parts-not-loaded = {$count} parties non chargees
message-usage = usage : in={$input} out={$output} reasoning={$reasoning}
message-finish = finish : {$finish}
message-empty = (message vide)
message-thinking = reflexion : {$summary}
message-command-status = statut : {$status}, exit={$exit}
message-file-changes = changements de fichiers
message-file-changes-preview-one = 1 fichier : {$paths}
message-file-changes-preview-many = {$count} fichiers : {$paths}
message-file-changes-more = +{$count} de plus
message-search = recherche : {$query}
message-todo-list = liste de taches
message-error = erreur [{$code}] : {$message}
message-attachments = pieces jointes
message-awaiting-user-input = en attente d'entree utilisateur : {$request_id}
message-question-line = - {$question} ({$id})
message-part-detail-unavailable = detail de partie indisponible
message-tool-pending = attente : {$label}
message-tool-running = en cours : {$label}
message-tool-done = termine : {$label}
message-tool-failed = echec : {$label}
message-tool-cancelled = annule : {$label}
message-tool-result-blocks = {$count} blocs de resultat

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

time-just-now = a l'instant
time-minutes-ago = il y a {$count} min
time-hours-ago = il y a {$count} h
time-days-ago = il y a {$count} j

session-default-title = Nouvelle session {$time}
session-default-base = Nouvelle session
session-fallback-title = Session {$id}

user-input-error-empty = la reponse ne peut pas etre vide
user-input-error-invalid-segment = segment de reponse invalide : {$segment}
user-input-error-unknown-question = identifiant de question inconnu : {$question_id}
user-input-error-missing-answer = la question {$question_id} doit avoir au moins une reponse
user-input-error-no-answers = la reponse ne contenait aucune valeur

attachment-kind-image = image
attachment-kind-audio = audio
attachment-kind-video = video
attachment-kind-pdf = pdf
attachment-kind-file = fichier
attachment-kind-directory = dossier
attachment-generic = piece jointe
attachment-chip-image = {$kind} : {$filename} ({$width}x{$height}, {$size})
attachment-chip-other = {$kind} : {$filename} ({$size})
attachment-placeholder = [{$kind} {$filename}]

bytes-gb = {$value} GB
bytes-mb = {$value} MB
bytes-kb = {$value} KB
bytes-b = {$value} B

paste-label = collage de {$count} caracteres
paste-label-append = collage de {$count} caracteres, ajoute a l'envoi
paste-placeholder = [collage de {$count} caracteres]

permission-label-allow-once = autoriser une fois
permission-label-allow-always = toujours autoriser
permission-label-deny-once = refuser une fois
permission-label-deny-always = toujours refuser

permission-summary-allow-once = autorisation accordee une fois : {$reason}
permission-summary-allow-always = autorisation toujours accordee : {$reason}
permission-summary-deny-once = autorisation refusee une fois : {$reason}
permission-summary-deny-always = autorisation toujours refusee : {$reason}

failure-detail-message = Message
failure-detail-code = Code d'erreur
failure-detail-category = Catégorie
failure-detail-responsibility = Responsabilité
failure-detail-impact = Impact
failure-detail-recovery = Récupération
failure-detail-retry = Nouvelle tentative
failure-detail-reference = Référence
failure-category-invalid-input = Entrée invalide
failure-category-not-found = Introuvable
failure-category-conflict = Conflit
failure-category-permission-required = Autorisation requise
failure-category-permission-denied = Autorisation refusée
failure-category-authentication-required = Authentification requise
failure-category-rate-limited = Limite de débit
failure-category-quota-exceeded = Quota dépassé
failure-category-timeout = Délai dépassé
failure-category-dependency-unavailable = Dépendance indisponible
failure-category-protocol-failure = Erreur de protocole
failure-category-data-corruption = Problème d’intégrité des données
failure-category-internal = Erreur interne
failure-responsibility-caller = La requête
failure-responsibility-policy = Politique
failure-responsibility-dependency = La dépendance
failure-responsibility-system = Le système
failure-impact-request-rejected = Requête rejetée
failure-impact-operation-failed = Opération échouée
failure-impact-operation-paused = Opération suspendue
failure-impact-partial-success = Succès partiel
failure-impact-background-task-failed = Tâche de fond échouée
failure-impact-runtime-degraded = Runtime dégradé
failure-impact-fatal-startup-failure = Échec de démarrage fatal
failure-recovery-none = Aucune récupération automatique
failure-recovery-refresh = Actualiser
failure-recovery-reauthenticate = Réauthentifier
failure-recovery-open-settings = Ouvrir les paramètres
failure-recovery-request-permission = Demander une autorisation
failure-recovery-ask-user = Demander à l’utilisateur
failure-recovery-retry = Réessayer
failure-recovery-choose-alternative = Choisir une alternative
failure-recovery-restart-plugin = Redémarrer le plugin
failure-recovery-restart-runtime = Redémarrer le runtime
failure-retry-never = Ne pas réessayer
failure-retry-correct-input = Corriger l’entrée et réessayer
failure-retry-after-user-action = Réessayer après action de l’utilisateur
failure-retry-after-refresh = Réessayer après actualisation
failure-retry-immediate-once = Réessayer une fois immédiatement
failure-retry-backoff = Réessayer avec backoff
failure-retry-use-alternative = Utiliser une alternative
failure-retry-unknown = Inconnu
