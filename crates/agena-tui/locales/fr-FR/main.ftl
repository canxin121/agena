cli-about = Application de chat terminal Agena

pane-sessions = Sessions
pane-sessions-search = Sessions [{$query}]
pane-transcript = Transcript
pane-messages = Messages
pane-composer = Saisie [{$session}]

session-view-all = toutes
session-view-roots = racines
session-view-subtree = sous-arbre
session-view-subtree-root = sous-arbre #{$id}
session-meta = #{$id}  {$message_count} msg  {$updated}
session-running = en cours
session-awaiting-approval = en attente d'autorisation
session-awaiting-plan-approval = en attente d'approbation du plan
session-awaiting-user-input = en attente de saisie
session-awaiting-model = en attente du modèle
session-idle = inactive
session-tag-parent = parent
session-tag-child = enfant
session-tag-current = actuelle
sessions-empty = Aucune session trouvee
sessions-loading-more = Chargement de sessions supplementaires...
sessions-more = D'autres sessions sont disponibles
hub-title = Hub de session
hub-action-create = nouvelle session
hub-action-list = liste des sessions
hub-action-refresh = actualiser
hub-hint-move = déplacer
hub-hint-focus = focus
hub-hint-section = section
hub-hint-open = ouvrir
hub-hint-back = retour
hub-section-attention = Nécessite une attention
hub-section-running = En cours
hub-section-recent = Récentes
hub-empty-attention = Aucune session ne nécessite une attention
hub-empty-running = Aucune session en cours
hub-empty-recent = Aucune session récente
hub-section-new = Nouvelle session
hub-empty-new = Aucune session à créer
hub-item-new = + Nouvelle session
hub-item-new-detail = Entrée pour créer une session
hub-action-search = rechercher
hub-action-clear-search = effacer la recherche
hub-search-placeholder = Tapez pour filtrer les sessions…
hub-search-active-empty = Tapez pour filtrer…
hub-search-active = Filtre : {$query}
command-hub-summary = Ouvrir le hub de session
command-background-summary = Revenir au hub;la session continue
hub-empty = Aucune session pour l'instant. Créez-en une avec Ctrl+N.
context-help-context-hub = Hub de session
context-help-summary-hub = Consultez les sessions nécessitant une attention, en cours et récentes, et créez une nouvelle session.
context-help-key-create-session = Créer une nouvelle session.
context-help-key-session-list = Ouvrir la liste complète des sessions.

transcript-header-lines = lignes {$first}-{$last}/{$total} ({$percent}%)
transcript-header-find = recherche={$query} ({$current}/{$total})
transcript-header-tail = suivi fin
transcript-header-loading = chargement
transcript-header-loading-older = chargement des anciens messages
transcript-header-busy = occupe
transcript-loading-older = Chargement des anciens messages...
transcript-more-older = D'anciens messages sont disponibles. Faites defiler vers le haut ou appuyez sur PageUp.
transcript-empty-session = Aucun message dans cette session pour le moment.

session-state-creating = creation
session-state-ready = terminee recemment
session-state-running = en cours
session-state-awaiting-interaction = en attente de votre reponse
session-state-interrupted = interrompue
session-state-failed = echouee
session-tag-ancestor = ancêtre
session-tag-sibling = sœur
session-tag-leaf = feuille
session-summary-root = racine #{$id}
session-summary-depth = profondeur {$depth}
session-summary-side-branches = {$count} branches
session-summary-descendants = {$count} descendants
session-summary-parent = parent #{$id}
session-summary-children = {$count} enfant(s)

no-session-selected = Aucune session selectionnee.
no-session-selected-hint = Utilisez /sessions pour choisir une session, ou commencez a saisir dans la zone de composition pour en creer une.
composer-session-new = nouvelle session
composer-placeholder = Message pour Agena. Haut au debut ouvre l'historique. / commandes. Ctrl+O fichier.

status-global = / cherche en bas | ? cherche en haut | Ctrl+C deux fois quitte
status-sessions = Sessions: /sessions
status-transcript = VIEW: i saisie | j/k defile | / cherche | c copie dernier | y copie
status-composer = INSERT: Esc retour | Ctrl+Enter envoie maintenant | Ctrl+J nouvelle ligne | Haut au debut historique | / commandes | Ctrl+G items | Ctrl+R entree | Ctrl+L approbation

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
help-composer-line-10 = Haut ouvre l'historique quand le curseur est au debut ; Ctrl+P edite le message en attente et Ctrl+X l'annule
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
overlay-user-input-questions = Questions
overlay-user-input-summary = Résumé
overlay-user-input-detail = Détails
overlay-user-input-prompt-panel = Demande
overlay-user-input-choices = Choix
overlay-user-input-auto-resolve = Continuation automatique dans {$remaining}
overlay-user-input-choice-single-short = Choix unique
overlay-user-input-choice-multiple-short = Choix multiples
overlay-user-input-custom-input = Saisie personnalisée
overlay-user-input-custom-input-hint = Saisie personnalisée (e / coller)
overlay-user-input-submit = Envoyer
overlay-user-input-review-intro = Vérifiez vos réponses avant de les envoyer.
overlay-user-input-no-questions = Aucune question.
overlay-user-input-other = Autre
overlay-user-input-unanswered = sans réponse
overlay-user-input-custom-empty = Appuyez sur Espace pour saisir une réponse personnalisée
overlay-user-input-review-feedback = Commentaire pour l’agent
overlay-user-input-review-feedback-empty = Appuyez sur Entrée ou e pour saisir un commentaire, ou choisissez une décision
overlay-user-input-review-footer-hint = Entrée sur une option = envoyer · Entrée sur le titre = réduire/développer
overlay-user-input-missing-answer = Réponse manquante pour {$label}
overlay-user-input-footer-review = j/k ↑/↓ déplacer | Entrée choisir | e modifier | Ctrl+Y/Ctrl+E défiler par ligne | PgUp/PgDn pages | Ctrl+X annuler
overlay-user-input-footer-question = Tab/Shift+Tab pages | Espace basculer | Ctrl+D effacer | Ctrl+Y/Ctrl+E défiler par ligne | PgUp/PgDn pages | Ctrl+X annuler
overlay-user-input-wizard-keys = ←→ pages · Espace sélectionner · Entrée suivant / confirmer le résumé
overlay-user-input-page = Question { $current } / { $total }
overlay-user-input-previous = Précédent
overlay-user-input-next = Suivant
overlay-user-input-cancel = Annuler
overlay-user-input-request-id = request_id: {$request_id}
overlay-user-input-custom-allowed = valeur personnalisee autorisee
overlay-user-input-reply-format = Format de reponse : 0=value;1=value1,value2
overlay-user-input-cancel-hint = Ctrl+X annule la demande
overlay-user-input-footer = Ctrl+X annuler

flash-terminal-event-error = erreur d'evenement terminal : {$error}
flash-created-session = session creee {$title}
flash-session-view-mode = affichage des sessions : {$mode}
flash-session-title-empty = le titre de la session ne peut pas être vide
flash-session-renamed = session renommée en {$title}
flash-session-favorited = {$title} ajouté aux favoris
flash-session-unfavorited = {$title} retiré des favoris
flash-session-compacted = contexte de session condensé
flash-session-rewound = branche créée après le retrait de {$target} ; message restauré dans la zone de saisie
flash-permission-reply-sent = reponse d'autorisation envoyee : {$label}
flash-user-input-reply-sent = reponse utilisateur envoyee
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
flash-message-interrupting = interruption de l'execution en cours - le message sera envoye ensuite

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
message-user-input-replied = saisie utilisateur répondue : {$request_id}
message-question-line = - {$question}
message-part-detail-unavailable = detail de partie indisponible
message-tool-pending = attente : {$label}
message-tool-running = en cours : {$label}
message-tool-done = termine : {$label}
message-tool-failed = echec : {$label}
message-tool-cancelled = annule : {$label}
message-tool-result-blocks = {$count} blocs de résultat
message-input-activity-attachment = Pièce jointe
message-input-activity-skill = Commande
message-activity-response-running = Génération de la réponse en cours
message-activity-response-completed = Réponse terminée
message-activity-response-failed = Échec de la réponse
message-activity-response-cancelled = Réponse annulée
message-activity-continue-running = Poursuite de la réponse en cours
message-activity-continue-completed = Suite terminée
message-activity-continue-failed = Échec de la suite
message-activity-continue-cancelled = Suite annulée
message-activity-compact-running = Condensation du contexte en cours
message-activity-compact-completed = Contexte condensé
message-activity-compact-failed = Échec de la condensation du contexte
message-activity-compact-cancelled = Condensation du contexte annulée
message-activity-permission-running = Application de la réponse d’autorisation en cours
message-activity-permission-completed = Réponse d’autorisation appliquée
message-activity-permission-failed = Échec de la réponse d’autorisation
message-activity-permission-cancelled = Réponse d’autorisation annulée
message-activity-input-running = Envoi de la saisie utilisateur en cours
message-activity-input-completed = Saisie utilisateur envoyée
message-activity-input-failed = Échec de l’envoi de la saisie utilisateur
message-activity-input-cancelled = Envoi de la saisie utilisateur annulé
message-permission = autorisation
message-tool-summary = [outil] {$status} | {$label}
message-tool-output-collapsed =     … {$lines} de plus
message-activity-run-collapsed = Entrée : charger 5 · n Entrée · Ctrl+Maj+Entrée : tout charger · … {$count} anciennes parties masquées
message-compaction-title-manual = Contexte condensé
message-compaction-title-auto = Contexte condensé automatiquement
message-compaction-title-reactive = Contexte condensé après dépassement de la fenêtre
message-compaction-token-summary = {$before} → {$after} jetons · réduction de {$percent} %
message-compaction-strategy-local = résumé local
message-compaction-strategy-native = point de contrôle natif du fournisseur
message-compaction-strategy-line = Stratégie : {$strategy}
message-compaction-generation-line = Génération {$generation}

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

## Settings Studio core locale coverage
## Long policy descriptions intentionally continue to use the verified English fallback.

permission-studio-new-rule-label = + Nouvelle règle

permission-studio-new-rule-value = Créer

permission-studio-catalog-tags-title = Ajouter des règles d'étiquette d'outil

permission-studio-catalog-names-title = Ajouter des règles d'accès aux outils

permission-studio-catalog-footer = Vers le bas vers les résultats · L'espace bascule · Entrez le mode de choix · L'Esc annule

permission-studio-catalog-tag-detail = Utilisé par {$count} outil(s) enregistré(s)

permission-studio-catalog-custom-label = + Règle personnalisée...

permission-studio-catalog-custom-search = nom de l'outil de nouvelle balise manuelle personnalisée

overlay-settings-title = Paramètres

overlay-settings-footer = Ctrl+R rafraîchissement · ←/→ panneaux de commutation · Tab/Shift+ Panneaux de cycle de l'onglet · ↑/↓ select · Enter open · Esc close

overlay-settings-sections = Sections

overlay-settings-options = Options

overlay-settings-group-core = Principal

overlay-settings-group-application = Application

overlay-settings-group-session = Session

overlay-settings-group-system = Système

overlay-settings-default-section-title = Chapitre

overlay-settings-empty-section = Aucune section sélectionnée.

overlay-settings-empty-items = Pas de paramètres dans cette section.

overlay-settings-empty-detail = Sélectionnez une section et une option pour l'inspecter ou la modifier.

overlay-settings-detail-current = Valeur actuelle : {$value}

overlay-settings-detail-path = Voie : {$path}

overlay-settings-detail-action = Ouvrez ou modifiez ce paramètre.

settings-detail-action-screen = Ouvrez cet écran.

overlay-settings-edit-title = Modifier {$field}

overlay-settings-edit-file-value = Redéfinition du fichier : {$value}

overlay-settings-edit-effective-value = Valeur effective : {$value}

overlay-choice-clear-settings-detail = Supprimez le rebord du fichier pour {$field}.

overlay-settings-section-plugins-label = Plugins et outils

overlay-settings-section-plugins-summary = Plugins, outils et diagnostics

overlay-settings-section-providers-label = Modèles et fournisseurs

overlay-settings-section-providers-summary = {$count} Fournisseurs configurés

overlay-settings-section-model-catalog-label = Catalogue de modèles

overlay-settings-section-model-catalog-summary = {$count} entrées

overlay-settings-section-permissions-label = Autorisations

overlay-settings-section-permissions-summary = {$count} règle de permission persistante

overlay-settings-section-tracing-summary = Log filtres et diagnostics

overlay-settings-section-ui-label = Apparence

overlay-settings-section-ui-summary = Préférences locales et d'interface

overlay-settings-section-ui-description = Langue persistante, couleur, graphiques et paramètres de thème.

overlay-settings-section-runtime-session-label = Runtime et session

overlay-settings-section-runtime-session-summary = Identités des clients fournisseurs et rapprochement du contexte

settings-permission-global-label = Autorisation mondiale

settings-permission-global-detail = Point de référence pour toutes les sessions.

settings-permission-workspace-label = Permission d'espace de travail

settings-permission-workspace-detail = Couche de dépassement pour le projet actuel.

settings-permission-current-label = Autorisation de la session en cours

settings-permission-current-detail = S'applique uniquement à la session en cours.

settings-permission-effective-label = Autorisation effective

settings-permission-layer-global = Mondial

settings-permission-layer-workspace = Espace de travail

settings-permission-layer-session = Séance

settings-permission-layer-effective = Efficace

settings-runtime-thinking-label = Mode de réflexion

settings-runtime-thinking-description = redéfinition du mode think de session actuelle

settings-runtime-speed-label = Mode vitesse

settings-runtime-speed-description = mode vitesse de session courante

settings-runtime-verbosity-label = Verbosité

settings-runtime-verbosity-description = verbosité de session actuelle

settings-field-permission-approval-model-label = Modèle d’approbation automatique global

settings-field-ui-locale-label = Langue

settings-field-ui-locale-description = Langue de l'interface

settings-field-tui-color-scheme-label = Jeu de couleurs du terminal

settings-field-tui-theme-label = Thème de plugin TUI

settings-field-tui-theme-description = palette de couleurs sémantiques fournies par le plugin en option

settings-choice-tui-color-scheme-auto = Détecter l'arrière-plan du terminal automatiquement

settings-choice-tui-color-scheme-dark = Optimiser les couleurs pour un fond terminal sombre

settings-choice-tui-color-scheme-light = Optimiser les couleurs pour un fond terminal léger

settings-field-tui-graphics-label = Graphiques enrichis du terminal

settings-choice-tui-graphics-auto = Négocier automatiquement les graphiques natifs et revenir en toute sécurité à Unicode (recommandé)

settings-choice-tui-graphics-native = Forcer la négociation graphique native pour un chemin terminal configuré par un expert

settings-choice-tui-graphics-unicode = Désactiver les graphiques natifs et utiliser le rendu Unicode/texte déterministe

settings-field-activity-default-expanded-label = Développer les activités par défaut

settings-field-activity-kind-description = État d'expansion par défaut pour ce type d'activité.

settings-field-activity-tool-label = Extension par défaut de l'outil

settings-field-activity-tool-description = État d'extension par défaut pour cet outil exact.

settings-activity-kind-reasoning-label = Motifs

settings-activity-kind-operation-label = Opérations d'outils

settings-activity-kind-operation-description = Les appels d'outils et leurs résultats.

settings-activity-kind-resource-label = Ressources

settings-activity-kind-resource-description = Pièces jointes et autres ressources.

settings-activity-kind-skill_reference-label = Références de commande

settings-activity-kind-skill_reference-description = Références aux commandes sélectionnées dans la réponse.

settings-activity-kind-interaction-label = Interactions

settings-activity-kind-interaction-description = Demandes d'entrée des utilisateurs et invites interactives.

settings-activity-kind-hook-label = Crochets

settings-activity-kind-hook-description = Session hook runs et événements du cycle de vie.

settings-activity-kind-error-label = Erreurs

settings-activity-kind-error-description = Les opérations et les défaillances terminales ont échoué.

settings-activity-kind-notice-label = Avis

settings-activity-kind-notice-description = Informations générales et lignes informatives.

settings-activity-kind-text-label = Texte

settings-activity-kind-text-description = Contenu de texte simple et de texte artefact.

settings-field-tracing-filter-label = Niveau de journal de l’application

settings-field-tracing-filter-description = Niveau du journal de repérage par défaut

settings-field-tracing-database-label = Niveau de journal de la base de données

settings-field-tracing-database-description = Niveau du journal de recherche de la base de données

settings-field-tracing-adapter-label = Niveau de journal de l’adaptateur

settings-field-tracing-adapter-description = Niveau du journal de localisation de l'adaptateur fournisseur

settings-config-open-file-detail = Ouvrir agena.json pour ce chemin

settings-source-unset = Non défini

settings-source-configured = Configuration : {$value}

settings-source-effective = En vigueur : {$value}

settings-source-file-effective = Dossier : {$file} / En vigueur : {$effective}

settings-source-file-found = {$path} (trouvé)

settings-source-file-missing = {$path} (sera créé)

settings-source-row-config-file = Configurer le fichier

settings-source-row-workspace-config-file = Fichier de configuration de l'espace de travail

settings-source-row-file-value = Valeur du fichier

settings-source-row-workspace-value = Valeur de l'espace de travail

settings-source-row-effective-value = Valeur effective

settings-source-row-write-target = Écrit à

settings-source-row-layers = Couches actives

settings-source-current-session = données d'exécution en cours de session

settings-source-current-session-runtime = options d'exécution en cours de session

settings-detail-values-heading = Valeurs

settings-detail-sources-heading = Sources

settings-detail-action-readonly = Ouvrez la vue en lecture seule.

settings-detail-action-file = Ouvrez le fichier de configuration du support.




settings-field-parse-bool = {$field} s'attend à un booléen comme vrai/faux ou on/off

settings-field-parse-integer = {$field} s'attend à une valeur entière non signée

settings-field-parse-float = {$field} s'attend à une valeur numérique

settings-choice-adapter-fallback = adaptateur

settings-plugin-workbench-label = Atelier de configuration des plugins

settings-mcp-server-label = Serveur MCP Agena

settings-mcp-server-value = basculer activé/désactivé

settings-mcp-server-enabled = activé

settings-mcp-server-disabled = handicapés

settings-mcp-status-unavailable = statut non disponible

settings-mcp-ready = Prêt

settings-mcp-needs-attention = nécessitant une attention particulière

settings-mcp-auth-label = Authentification MCP

settings-mcp-auth-none = anonyme : chaque outil exposé

settings-mcp-auth-oauth = complet

settings-mcp-auth-mixed = mixte: découverte publique, par outil OAuth

settings-mcp-anonymous-access-label = Accès anonyme aux outils avec authentification mixte

settings-mcp-anonymous-access-none = Aucun (recommandé)

settings-mcp-anonymous-access-read-only = outils en lecture seule pour l'autorisation-contrat

settings-mcp-registration-label = enregistrement

settings-mcp-pkce-label = PKCE

settings-mcp-public-url-label = URL publique MCP

settings-mcp-public-url-value = modifier

settings-mcp-public-url-auto = écho-auditeur local

settings-mcp-oauth-issuer-label = URL de l’émetteur OAuth

settings-mcp-oauth-issuer-derived = d'origine des ressources du PCM

settings-mcp-oauth-password-label = Mot de passe OAuth MCP

settings-mcp-oauth-password-value = définir ou remplacer

settings-mcp-oauth-password-configured = Mot de passe spécifique au MCP configuré

settings-mcp-oauth-password-ui-fallback = utilisant le retour du mot de passe de l'interface utilisateur

settings-mcp-oauth-password-not-configured = non configuré

settings-mcp-oauth-password-clear-label = Effacer le mot de passe de MCP OAuth

settings-field-runtime-codex-version-label = Version client Codex

settings-field-runtime-claude-version-label = Version du code Claude

settings-field-runtime-gemini-version-label = Version Gemini CLI

settings-field-session-compaction-auto-label = Compactage automatique

settings-field-session-compaction-reserved-tokens-label = Tokens réservés au compactage

settings-client-versions-refresh-label = Actualiser les versions client

settings-client-versions-refresh-value = récupérer la dernière

settings-client-versions-entry-label = Versions du client fournisseur

settings-client-versions-entry-value = Codex · claude · gemini

settings-client-versions-section-label = Versions des clients

settings-client-versions-section-summary = Versions d'identité d'exécution

settings-provider-workbench-label = Liste des fournisseurs

settings-provider-workbench-value = {$count} fournisseur(s)

settings-global-default-model-label = Modèle par défaut global

settings-global-default-model-description = Modèle utilisé lorsqu'une session n'en spécifie aucun ; un modèle de session explicite est toujours prioritaire.
settings-workspace-default-model-label = Modèle par défaut de l’espace de travail
settings-workspace-default-model-description = Modèle par défaut stocké dans la configuration du projet de cet espace de travail ; il remplace ici le défaut global.

settings-model-default-mode-inherit-detail = Utilisez le mode natif par défaut du modèle sélectionné.

settings-provider-new-label = + Nouveau fournisseur

settings-provider-existing-detail = {$count} adaptateurs configurés

settings-model-catalog-open-label = Ouvrir le catalogue des modèles

settings-files-open-config-label = Ouvrir agena.json

settings-files-open-config-present = Présent

settings-files-open-config-create = créer en ouvert

permission-studio-field-path-workspace = Par défaut de l'espace de travail

permission-studio-field-path-external = Par défaut de chemin externe

permission-studio-field-path-rules = Règles de trajectoire

permission-studio-field-network-defaults = Par défaut réseau

permission-studio-field-network-rules = Règles de réseau

permission-studio-field-tool-names = Noms des outils

permission-studio-field-tool-rules = Règles d'outil

permission-studio-field-prompt-json = Entrez JSON pour {$field}. Laissez l'éditeur vide pour effacer cette surcharge.

permission-studio-detail-override = Dépassement

permission-studio-detail-effective = Efficace

permission-studio-detail-override-inline = Surpasser {$value}

permission-studio-detail-effective-inline = Efficace {$value}

permission-studio-detail-read-only = Ce document de permission est en lecture seule ici.

permission-studio-detail-mode-editable = Entrée ouvre le mode de sélection pour ce seul champ.

permission-studio-detail-text-editable = Saisissez les modifications de cette seule clé ou modèle.

permission-studio-detail-remove-hint = Entrée supprime cet élément immédiatement.

permission-studio-detail-navigate-hint = Entrée ouvre cette section.

permission-studio-overview-target = Objectif

permission-studio-overview-source = Source

permission-studio-overview-scope = Portée

permission-studio-overview-override = Dépassement

permission-studio-overview-effective = Efficace

permission-studio-section-workspace = Espace de travail

permission-studio-section-external = Extérieur

permission-studio-section-rules = Règles

permission-studio-section-defaults = Par défaut

permission-studio-source-global = mondial

permission-studio-source-workspace = espace de travail

permission-studio-source-session = session

permission-studio-source-effective = efficace

permission-studio-settings-override = remplacer {$value}

permission-studio-settings-effective = efficace {$value}

permission-studio-mode-read = lire {$value}

permission-studio-mode-write = écrire {$value}

permission-studio-network-default = {$label} {$value}

permission-studio-page-overview = Aperçu général

permission-studio-page-path = Voie

permission-studio-page-path-defaults = Système de fichiers / Zones par défaut

permission-studio-page-path-rules = Système de fichiers / Règles de chemin

permission-studio-page-network = Réseau

permission-studio-page-network-zones = Réseau / Zones réseau

permission-studio-page-network-rules = Réseau / Règles de domaine

permission-studio-page-tools = Outils

permission-studio-page-tool-tags = Accès à l'outil / Règles d'étiquette

permission-studio-page-tool-names = Accès aux outils / Règles de nom

permission-studio-page-tool-command-rules = Accès aux outils / Règles de commande

permission-studio-page-names = Noms

permission-studio-page-tool-rules = Règles d'outil

permission-studio-nav-overview = Aperçu général

permission-studio-nav-filesystem = Système de fichiers

permission-studio-nav-default-zones = Zones par défaut

permission-studio-nav-path-rules = Règles de chemin

permission-studio-nav-network = Réseau

permission-studio-nav-network-zones = Zones réseau

permission-studio-nav-domain-rules = Règles de domaine

permission-studio-nav-tool-access = Accès aux outils

permission-studio-nav-name-rules = Règles de nom

permission-studio-nav-command-rules = Règles de commande

permission-studio-path-workspace-read = Lecture de l’espace de travail

permission-studio-path-workspace-write = Écriture dans l’espace de travail

permission-studio-path-external-read = Lecture externe

permission-studio-path-external-write = Écriture externe

permission-studio-path-rule-read = Mode de lecture

permission-studio-path-rule-write = Mode d'écriture

permission-studio-network-internet = Internet

permission-studio-network-private = Privé

permission-studio-network-loopback = Loopback

permission-studio-tool-default = Valeur par défaut des outils

permission-studio-tool-default-summary = par défaut {$value}

permission-studio-add-path-rule = Ajouter la règle de chemin

permission-studio-add-network-rule = Ajouter une cible réseau

permission-studio-add-name = Ajouter un nom

permission-studio-add-tool-rule = Ajouter une règle d'outil

permission-studio-rule-key = Clé

permission-studio-rule-pattern = Modèle

permission-studio-rule-target = Objectif

permission-studio-rule-mode = Mode

permission-studio-tool-rule-fallback = Mode de recul

permission-studio-error-empty-value = {$field} ne peut pas être vide.

overlay-providers-title = Fournisseurs

overlay-providers-prompt = Choisir un fournisseur à configurer

overlay-provider-list-title = Liste des fournisseurs

overlay-provider-list-prompt = Recherche de fournisseurs configurés

overlay-provider-list-footer = Sélectionnez Créer un fournisseur ou un fournisseur existant, puis appuyez sur Entrée

overlay-provider-list-create-label = + Nouveau fournisseur

overlay-provider-list-row-detail-no-model = {$adapter} · {$count} adaptateurs configurés

overlay-provider-studio-title = Config du fournisseur

overlay-provider-studio-header = Config du fournisseur

overlay-provider-studio-footer = Tab/Shift+Plaques d'onglets · Flèches sélectionnez · Espace toggle · Entrer la modification · Ctrl+D supprimer sélectionné · Ctrl+R rafraîchir · Ctrl+N ajouter le modèle · Ctrl+ Un adaptateur de sauvegarde · Ctrl+S save provider · Esc close

overlay-provider-studio-providers = Fournisseurs

overlay-provider-studio-draft = Projet

overlay-provider-studio-adapters = Adaptateurs

overlay-provider-studio-models = Modèles

overlay-provider-studio-catalog = Catalogue des modèles
overlay-provider-studio-catalog-picker-title = Choisir un modèle de catalogue

overlay-provider-studio-catalog-picker-footer = Haut/Bas sélectionner · Ctrl+F rechercher · Gauche/Droite paginer · Entrée remplit les champs du modèle · Échap retour

overlay-provider-studio-catalog-match = Catalogue : {$model}

overlay-provider-studio-catalog-selected = Catalogue sélectionné : {$model}

overlay-provider-studio-catalog-loading = Recherche d’une correspondance dans le catalogue…

overlay-provider-studio-catalog-no-match = Aucune correspondance dans le catalogue · choisissez dans les réglages du modèle

overlay-provider-studio-detail = Détail

overlay-provider-studio-adapter-models-empty = Sélectionnez les adaptateurs, puis listez leurs modèles en direct

overlay-provider-studio-models-empty = Aucun modèle d’adaptateur disponible

overlay-provider-studio-catalog-empty = Aucune entrée de catalogue ne correspond à cette requête

overlay-provider-studio-new-provider-detail = Projet de fournisseur vide

overlay-provider-studio-provider-row-detail-no-model = {$adapter} · {$count} adaptateurs configurés

overlay-provider-studio-model-count = Modèles {$count}

overlay-provider-studio-loaded = chargé

overlay-provider-studio-error = erreur

overlay-provider-studio-configured = configuré

overlay-provider-studio-live-list = liste en direct

overlay-provider-studio-not-listed = non énumérés

overlay-provider-studio-not-supported = non soutenu par le contrat actuel

overlay-provider-studio-edit-title = Modifier le champ

overlay-provider-studio-edit-prompt = Mettre à jour {$field}

overlay-provider-studio-edit-footer = Type à modifier

overlay-provider-studio-model-edit-footer = Ctrl+S sauvegarde la configuration du modèle

overlay-provider-studio-model-json-title = Configuration du modèle · {$adapter}/{$model}

overlay-provider-studio-model-json-prompt = Modifier le modèle de fournisseur persistant JSON.

overlay-provider-studio-model-title = Modèle · {$adapter}/{$model}

overlay-provider-studio-model-footer = Flèches sélectionner · Entrer l'édition · Ctrl+S enregistrer · Ctrl+D supprimer · Esc retour

overlay-provider-delete-title = Supprimer le fournisseur

overlay-provider-delete-adapter-title = Supprimer l'adaptateur

overlay-provider-delete-model-title = Supprimer le modèle

overlay-provider-studio-model-edit-title = Modifier le champ de modèle

overlay-provider-studio-model-field-prompt = Mettre à jour {$field}

overlay-provider-studio-new-model-title = Ajouter un modèle

overlay-provider-studio-edit-auth-mode-prompt = Mettre à jour le mode auth (none)

overlay-provider-studio-edit-auth-subtype-prompt = Mettre à jour le sous-type auth (api: cline api= gitlab api=1 bedrock sigv4 · credential: openai chatgpt=1 github copilot=1 gitlab=1 google adc=1 sap ai core)

overlay-provider-studio-edit-auth-login-method-prompt = Mettre à jour la méthode de connexion auth (navigateur périphérique)

provider-studio-auth-status-pending = en attente

provider-studio-auth-status-unset = non réglé

provider-studio-auth-status-none = aucune

provider-studio-auth-status-select-subtype = sélectionner le sous-type

provider-studio-auth-status-select-issuer = sélectionner le sous-type

provider-studio-auth-status-configured = configuré

provider-studio-auth-status-partial = partielle

provider-studio-summary-env = env

provider-studio-summary-callback = rappel

provider-studio-summary-redirect = redirection

provider-studio-summary-account = compte

provider-studio-summary-name = nom

provider-studio-summary-user = utilisateur

provider-studio-summary-email = Courriel

provider-studio-summary-profile = profil

provider-studio-summary-region = région

provider-studio-summary-code = code

provider-studio-summary-state = État {$state}

provider-studio-summary-tokens-set = jeu de jetons

provider-studio-summary-keys-set = jeu de clés

provider-studio-summary-set-field = ensemble {$field}

provider-studio-summary-review-fields = revoir les champs

provider-studio-summary-start-browser = Démarrer le navigateur OAuth

provider-studio-summary-restart-browser = redémarrer le navigateur OAuth

provider-studio-summary-open-authorize = ouvrir l'URL autorisée

provider-studio-summary-start-device = démarrer la connexion du périphérique

provider-studio-summary-restart-device = Connexion du périphérique de redémarrage

provider-studio-summary-open-verify = ouvrir l'URL de vérification

provider-studio-summary-finish-callback = fin de l'échange de rappel

provider-studio-summary-poll-every = sondage chaque {$seconds}s

provider-studio-summary-paste-callback = coller l'URL de rappel

provider-studio-summary-poll-now = sondage maintenant

provider-studio-summary-start-auth-first = commencer d'abord

provider-studio-summary-poll-browser = résultat du navigateur de sondage

provider-studio-auth-openai-ready = Le navigateur OAuth est prêt. Ouvrez l'URL d'autorisation ci-dessous.

provider-studio-auth-openai-device-ready = La connexion du périphérique OpenAI est prête. Ouvrez l'URL de vérification ci-dessous et entrez {$code}

provider-studio-auth-authorize = autoriser {$url}

provider-studio-auth-redirect = rediriger {$url}

provider-studio-auth-paste-callback = coller l'URL redirigée dans l'URL Callback, puis appuyez sur p · état {$state}

provider-studio-auth-copilot-ready = La connexion du périphérique est prête. Ouvrez l'URL de vérification ci-dessous et entrez {$code}

provider-studio-auth-verify = vérifier {$url}

provider-studio-auth-poll = appuyez maintenant sur p to poll · tous les {$seconds}s

provider-studio-auth-gitlab-ready = Le navigateur GitLab OAuth est prêt. Ouvrez l'URL d'autorisation ci-dessous.

provider-studio-auth-atomgit-ready = AtomGit session de navigateur prêt · l'URL autorisée est affiché ci-dessous

provider-studio-auth-finish-browser = terminer le flux du navigateur, puis appuyez sur p · état {$state}

flash-settings-updated = mise à jour {$path}

flash-settings-cleared = nettoyée {$path}

flash-provider-save-error-settings-object = les paramètres existants du fournisseur doivent être un objet JSON

command-settings-summary = Ouvrir le workbench de paramètres unifiés pour les modèles, permissions, plugins, runtime, sessions, interface et diagnostics
command-side-summary = Ouvrir une conversation parallèle sans interrompre l’exécution en cours
command-commit-summary = Créer un commit à partir des modifications préparées
command-pr-summary = Créer une demande de fusion avec les options gh essentielles
command-help-summary = Afficher l’aide de l’interface actuelle
command-commands-summary = Ouvrir la palette de commandes
command-new-summary = Créer une nouvelle session
command-sessions-summary = Ouvrir le sélecteur de sessions
command-lineage-summary = Ouvrir l’historique des branches de la session actuelle
command-rewind-summary = Retirer un message utilisateur et tous les suivants dans une nouvelle branche
command-rename-summary = Renommer la session actuelle ou sélectionnée
command-favorite-summary = Ajouter la session actuelle ou sélectionnée aux favoris, ou l’en retirer
command-timeline-summary = Ouvrir la chronologie des événements consultable de la session actuelle ou sélectionnée
command-export-summary = Exporter le transcript chargé en Markdown et l’ouvrir dans l’éditeur
command-pager-summary = Ouvrir le transcript chargé dans un afficheur paginé du terminal
command-continue-summary = Reprendre la session actuelle bloquée ou en attente
command-compact-summary = Résumer le contexte de la session actuelle
command-user-input-summary = Répondre à la première demande de saisie utilisateur en attente
command-allow-summary = Autoriser une fois la première demande d’autorisation en attente
command-allow-always-summary = Toujours autoriser la première demande d’autorisation en attente
command-deny-summary = Refuser une fois la première demande d’autorisation en attente
command-deny-always-summary = Toujours refuser la première demande d’autorisation en attente
command-attach-summary = Joindre un fichier avec le sélecteur interactif
command-download-summary = Télécharger un fichier de l’espace de travail avec le fournisseur de transfert du terminal actif
command-editor-summary = Ouvrir l’éditeur externe pour la zone de rédaction
command-image-summary = Joindre une image avec le sélecteur interactif
command-paste-summary = Coller des fichiers, images ou du texte du presse-papiers dans la zone de rédaction
command-copy-summary = Copier le transcript chargé
command-copy-message-summary = Copier le dernier message de l’assistant
command-copy-visible-summary = Copier la partie visible du transcript
command-model-summary = Ouvrir le sélecteur de modèle de session pour choisir le fournisseur, l’adaptateur et le modèle
command-fork-summary = Ouvrir une branche de la session actuelle avec son historique complet
command-children-summary = Ouvrir le sélecteur de sessions enfants
command-parent-summary = Accéder à la session parente
command-diagnostics-summary = Afficher un résumé de diagnostic expurgé pour le signalement
command-status-summary = Afficher l’état actuel de la session, de l’exécution, de l’espace de travail et de l’interface
command-usage-summary = Ouvrir les statistiques interactives de jetons et de coûts
command-activities-summary = Ouvrir le panneau des activités en arrière-plan
command-plan-summary = Ouvrir l’afficheur de plan de la session actuelle
command-btw-summary = Poser une question à part pendant que la conversation principale continue. Outils en lecture seule ; aucune modification de la conversation principale.

settings-mcp-public-url-updated = URL publique de MCP d'Agena mise à jour

settings-mcp-oauth-issuer-updated = Mise à jour de l'URL de l'émetteur de l'OAuth

settings-mcp-oauth-password-updated = MCP d'Agena Mot de passe OAuth mis à jour

settings-mcp-server-enabled-flash = Serveur MCP d'Agena activé

settings-mcp-server-disabled-flash = serveur MCP d'Agena désactivé

settings-mcp-auth-mode-updated = Mode d'authentification MCP d'Agena défini à {$mode}

settings-mcp-anonymous-access-updated = Accès anonyme à l'outil Agena MCP défini à {$policy}

settings-mcp-oauth-password-cleared = MCP d'Agena Mot de passe OAuth effacé

permission-studio-command-pattern-title = Modèle de commande {$tool_name}

settings-tool-api-list-description = Énumérer les outils d'exécution.

settings-tool-api-search-description = Outils d'exécution de recherche.

settings-tool-api-help-description = Inspecter les contrats d'exécution-outils.

settings-tool-api-tags-description = Liste les balises d'exécution-outil.

settings-tool-api-call-description = Invoquez un outil d'exécution.

settings-tool-api-plugins-list-description = Énumérer les plugins d'outils.

settings-tool-api-plugins-search-description = Greffons d'outils de recherche.

settings-tool-api-plugins-tags-description = Lister les balises outil-plugin.

permission-studio-command-pattern-help = Saisissez un motif glob de commande shell, par exemple `git status` ou `git push *`.

permission-studio-rename-unsupported = Cette entrée ne peut pas être renommée ; supprimez-la puis recréez-la.

# Settings, provider, permission, catalog, MCP, and diagnostics completion
overlay-editor-footer-single-line = Tapez pour modifier
overlay-editor-footer-multiline = Ctrl+S sauvegarder
context-help-title = Aide contextuelle
context-help-eyebrow = Interface actuelle
context-help-footer = ↑/↓ scroll · Esc ou Ctrl+H fermer
context-help-global-hint = Ctrl+H aide
context-help-context-composer-items = Articles du compositeur
context-help-context-suggestions = Suggestions
context-help-context-usage = Tableau de bord d'utilisation
context-help-context-plan-viewer = Visionneuse de plans
context-help-context-user-input = Demande de saisie de l'utilisateur
context-help-context-plugin-list = Établi de plugins · Liste
context-help-context-plugin-detail = Établi de plugins · Détails
context-help-context-plugin-config = Plugin Workbench · Configuration
context-help-context-plugin-actions = Configuration du plugin · Actions
context-help-context-plugin-selection = Configuration du plugin · Sélection
context-help-context-plugin-drilldown = Configuration du plugin · Exploration
context-help-context-plugin-diff = Configuration du plugin · Diff
context-help-key-delete = Supprimez l'élément sélectionné.
context-help-key-plugin-restart = Redémarrez le plugin sélectionné lorsqu'il est pris en charge.
overlay-permission-title = Demande d'autorisation
overlay-permission-details-title = Détails
overlay-permission-action-tool = outil : { $tool }
overlay-permission-action-path = chemin { $access } : { $path }
overlay-permission-action-network = réseau : { $target }
overlay-permission-field-tool = Outil
overlay-permission-field-target = Commandement ou cible
overlay-permission-field-access = Accès
overlay-permission-field-path = Chemin
overlay-permission-field-workspace = Espace de travail
overlay-permission-field-network = URL ou cible réseau
overlay-permission-field-host = Hôte
overlay-permission-field-reason = Pourquoi l'approbation est nécessaire
overlay-permission-detail-request-id = Numéro de demande
overlay-permission-detail-source = Source de la politique
overlay-permission-detail-scope = Portée demandée
overlay-permission-detail-operator = Demandé par
overlay-permission-detail-trace = Trace de décision
overlay-permission-summary-more-approvals = Approbation également de { $count } action(s) supplémentaire(s) dans cet appel d'outil
overlay-permission-detail-requested-actions = Demande également l'approbation de
overlay-permission-detail-related-actions = Déjà autorisé dans cet appel
overlay-permission-choice-auto-approve = Approuver automatiquement…
overlay-permission-rule-workbench-title = Règle d'autorisation
overlay-permission-rule-studio-footer = Flèches sélectionner · Entrer modifier · Ctrl+O parcourir le chemin sélectionné · Ctrl+S sauvegarder · Ctrl+D révoquer · Esc fermer
overlay-permission-rule-studio-footer-return = Flèches sélectionner · Entrer modifier · Ctrl+O parcourir le chemin sélectionné · Ctrl+S sauvegarder · Ctrl+D révoquer · Esc revient à la demande d'autorisation
flash-permission-rule-browse-path-selection = Sélectionnez Chemin cible ou Racine de l'espace de travail avant de parcourir.
overlay-permission-rule-choice-subject-title = Choisir le type de sujet
overlay-permission-rule-choice-subject-prompt = Choisissez le type de sujet de règle.
overlay-permission-rule-choice-subject-tool-detail = faire correspondre un outil ou un outil d'exécution
overlay-permission-rule-choice-subject-path-access-detail = faire correspondre l'accès au système de fichiers
overlay-permission-rule-choice-subject-network-access-detail = correspondre à l'accès au réseau
overlay-permission-rule-choice-access-title = Choisir le type d'accès au chemin
overlay-permission-rule-choice-access-prompt = Choisissez le mode d'accès au système de fichiers.
overlay-permission-rule-choice-access-read-detail = autoriser les lectures de fichiers uniquement
overlay-permission-rule-choice-access-write-detail = autoriser uniquement les écritures de fichiers
overlay-permission-rule-choice-access-read-write-detail = autoriser les lectures et les écritures
overlay-permission-rule-choice-scope-title = Choisir la portée de la règle
overlay-permission-rule-choice-scope-prompt = Choisissez dans quelle mesure la règle doit persister.
overlay-permission-rule-choice-scope-session-detail = seulement cette séance
overlay-permission-rule-choice-scope-workspace-detail = toutes les sessions dans cet espace de travail
overlay-permission-rule-choice-scope-global-detail = tous les espaces de travail
overlay-permission-rule-choice-mode-title = Choisir le mode règle
overlay-permission-rule-choice-mode-prompt = Choisissez autoriser, demander ou refuser.
overlay-permission-rule-choice-mode-allow-detail = toujours autoriser les actions correspondantes
overlay-permission-rule-choice-mode-auto-detail = laissez le modèle d'approbation configuré décider ; revenir à une invite en cas d'indisponibilité
overlay-permission-rule-choice-mode-ask-detail = demander avant d'autoriser les actions correspondantes
overlay-permission-rule-choice-mode-deny-detail = toujours refuser les actions correspondantes
overlay-permission-rule-editor-footer = Tapez pour modifier
overlay-permission-rule-editor-tool-name-title = Modifier le nom de l'outil
overlay-permission-rule-editor-tool-name-prompt = Entrez le nom exact de l'outil.
overlay-permission-rule-editor-qualifier-title = Modifier le qualificatif
overlay-permission-rule-editor-qualifier-prompt = Entrez un qualificatif facultatif ou laissez vide.
overlay-permission-rule-editor-workspace-root-title = Modifier la racine de l'espace de travail
overlay-permission-rule-editor-workspace-root-prompt = Entrez un répertoire workspace_root facultatif.
overlay-permission-rule-editor-target-path-title = Modifier le chemin cible
overlay-permission-rule-editor-target-path-prompt = Entrez le chemin ou le modèle cible.
overlay-permission-rule-editor-network-target-title = Modifier la cible du réseau
overlay-permission-rule-editor-network-target-prompt = Entrez un hôte, un hôte:port ou une URL.
overlay-permission-rule-editor-session-id-title = Modifier l'ID de session
overlay-permission-rule-editor-session-id-prompt = Entrez l'ID de session cible.
overlay-permission-rule-browser-workspace-root-title = Choisissez la racine de l'espace de travail
overlay-permission-rule-browser-workspace-root-prompt = Parcourez les répertoires et appuyez sur Entrée pour en sélectionner un.
overlay-permission-rule-browser-target-path-title = Choisissez le chemin cible
overlay-permission-rule-browser-target-path-prompt = Parcourez les fichiers ou les répertoires et appuyez sur Entrée pour en sélectionner un.
overlay-permission-rule-browser-footer = Sélectionnez ../ ou un répertoire et appuyez sur Entrée pour parcourir · sélectionnez une valeur et appuyez sur Entrée pour accepter
overlay-permission-rule-browser-empty = Aucun fichier ou répertoire correspondant.
overlay-permission-rule-item-subject-kind = Type de sujet
overlay-permission-rule-item-subject-kind-detail = Choisissez si cette règle s'applique à un outil, un chemin ou une cible réseau.
overlay-permission-rule-item-mode = Mode
overlay-permission-rule-item-mode-detail = Choisissez si les actions correspondantes sont autorisées, demandées ou refusées.
overlay-permission-rule-item-scope = Portée
overlay-permission-rule-item-scope-detail = Conservez cette règle pour la session, l’espace de travail ou globalement.
overlay-permission-rule-item-session-id = ID de session
overlay-permission-rule-item-session-id-detail = ID de session cible utilisé lorsque scope=session.
overlay-permission-rule-item-tool-name = Nom de l'outil
overlay-permission-rule-item-tool-name-detail = Nom exact de l'outil correspondant.
overlay-permission-rule-item-qualifier = Qualificateur
overlay-permission-rule-item-qualifier-detail = Qualificateur facultatif pour des règles d'outil plus spécifiques.
overlay-permission-rule-item-access-kind = Type d'accès
overlay-permission-rule-item-access-kind-detail = Choisissez lire, écrire ou read_write.
overlay-permission-rule-item-target-path = Chemin cible
overlay-permission-rule-item-target-path-detail = Modèle de chemin ou chemin exact à protéger.
overlay-permission-rule-item-workspace-root = Racine de l'espace de travail
overlay-permission-rule-item-workspace-root-detail = Répertoire de base facultatif utilisé pour interpréter les chemins cibles relatifs.
overlay-permission-rule-item-network-target = Cible du réseau
overlay-permission-rule-item-network-target-detail = Hôte, hôte : port ou cible URL correspondant.
overlay-permission-rule-detail-subject-kind = Les règles d'outil correspondent par nom d'outil et qualificatif facultatif. Les règles de chemin correspondent à l'accès au système de fichiers. Les règles réseau correspondent à l'accès à l'hôte ou à l'URL.
overlay-permission-rule-detail-tool-name = Les règles d'outil nécessitent un nom d'outil exact, par exemple `shell`, `read` ou `web_search`.
overlay-permission-rule-detail-qualifier = Le qualificatif est facultatif. Laissez-le vide, sauf si l'outil ou l'action nécessite une correspondance plus étroite.
overlay-permission-rule-detail-path-access-kind = Utilisez `read`, `write` ou `read_write` en fonction de l'accès au système de fichiers que vous souhaitez faire correspondre.
overlay-permission-rule-detail-workspace-root = Laissez workspace_root vide pour hériter de la racine de l’espace de travail d’exécution. Définissez-le explicitement lorsque le chemin protégé réside ailleurs.
overlay-permission-rule-detail-target-path = Entrez un chemin ou un modèle. Les chemins relatifs sont interprétés par rapport à workspace_root lorsqu'il est défini.
overlay-permission-rule-detail-network-target = Saisissez un hôte, `host:port`, ou une URL complète, selon la spécificité de la règle.
overlay-permission-rule-detail-scope = La portée de la session est idéale pour les remplacements temporaires. L’espace de travail et les étendues globales persistent plus longtemps.
overlay-permission-rule-detail-session-id = Les règles liées à la session nécessitent un identifiant de session concret.
overlay-permission-rule-detail-mode = Autoriser laisse passer l'action, demander l'approbation et refuser la bloque.
overlay-workbench-details = Détails
overlay-permission-studio-title = Autorisation
overlay-permission-studio-footer-nested = Ctrl+N ajouter · Entrer modifier · Ctrl+E renommer · Ctrl+D supprimer · Echap retour
permission-studio-catalog-prompt = Recherchez dans le catalogue d'outils en direct. Sélectionnez une ou plusieurs entrées, ou choisissez Règle personnalisée pour une valeur non actuellement enregistrée.
permission-studio-catalog-custom-detail = Ajoutez une balise ou un nom d'outil qui ne figure pas dans le catalogue actif actuel.
flash-permission-studio-catalog-empty = Sélectionnez au moins une entrée avant d'ajouter des règles.
overlay-runtime-setting-current-value = Remplacement actuel : { $value }
overlay-settings-help-string = Saisissez du texte. Laissez vide ou tapez `clear` pour supprimer le remplacement du fichier.
overlay-settings-help-bool = Saisissez vrai/faux, activé/désactivé, oui/non ou 1/0. Laissez vide ou tapez `clear` pour supprimer le remplacement du fichier.
overlay-settings-help-integer = Entrez un nombre entier. Laissez vide ou tapez `clear` pour supprimer le remplacement du fichier.
overlay-settings-help-float = Entrez un numéro. Laissez vide ou tapez `clear` pour supprimer le remplacement.
overlay-choice-clear-value = Effacer la valeur
overlay-settings-section-plugins-description = Configurez les plugins et consultez leurs outils et diagnostics.
overlay-settings-section-providers-description = Configurez les fournisseurs et leur comportement réseau, et inspectez le catalogue de modèles.
overlay-settings-section-model-catalog-description = Parcourez le catalogue de modèles résolus, inspectez les métadonnées du modèle et actualisez le cache local.
overlay-settings-section-permissions-description = Modifiez séparément les autorisations globales, de l'espace de travail et de la session en cours.
overlay-settings-section-runtime-session-description = Configurez les versions client de compatibilité et le comportement de compactage automatique des sessions.
settings-permission-effective-detail = Lecture seule · fusionné à partir du global, de l'espace de travail et de la session.
settings-permission-effective-read-only = L'autorisation effective est en lecture seule ; modifiez plutôt la session, l'espace de travail ou la source globale.
settings-field-permission-approval-model-description = Variantes de modèle et de réflexion/vitesse utilisées pour les décisions d'autorisation automatiques ; les sélections indisponibles reviennent à Ask
settings-workspace-approval-model-label = Modèle d’approbation automatique de l’espace de travail
settings-workspace-approval-model-description = Modèle d’approbation stocké dans la configuration du projet ; il remplace ici la sélection globale.
settings-field-tui-color-scheme-description = Détecter automatiquement l'arrière-plan du terminal ou forcer une palette claire ou sombre
settings-field-tui-graphics-description = Affichez des images et des formules de composition avec Kitty, Sixel ou iTerm2 lorsqu'ils sont pris en charge ; les modifications prennent effet après le redémarrage du TUI
settings-field-activity-default-expanded-description = État d'expansion par défaut pour les activités sans remplacement spécifique au type. Le raisonnement reste étendu à moins que son type ne soit défini explicitement.
settings-activity-kind-reasoning-description = Le parcours de réflexion complet du modèle. La valeur par défaut est développée et peut être réduite par type.
runtime-setting-choice-supported-model = soutenu par le modèle actuel
settings-plugin-workbench-detail = Ouvrez l'atelier de plug-in structuré pour connaître l'état d'exécution, la configuration, les outils, les commandes, les journaux et les diagnostics.
settings-mcp-server-detail = Activez/désactivez la surface HTTP MCP en direct d'Agena. Le processus du serveur Agena connecté reste le moteur d'exécution réel.
settings-mcp-auth-detail = Cycle sans authentification, OAuth complet et authentification mixte ChatGPT. Le mode mixte maintient l'initialisation et la découverte d'outils publiques ; les appels d'outils restent protégés par OAuth à moins que l'accès anonyme ne soit explicitement activé.
settings-mcp-anonymous-access-none-detail = Valeur par défaut sûre : aucun appel d'outil n'est anonyme ; ChatGPT peut toujours initialiser et découvrir le catalogue avant de se connecter.
settings-mcp-anonymous-access-read-only-detail = Opt-in à haut risque : les outils en lecture seule peuvent s'exécuter de manière anonyme et peuvent révéler un espace de travail privé, un système de fichiers, une configuration ou des données de diagnostic.
settings-mcp-anonymous-access-inactive-detail = Cette stratégie s'applique uniquement en mode d'authentification mixte ; basculez l’authentification sur mixte pour l’utiliser.
settings-mcp-public-url-detail = Définissez l’URL canonique de la ressource HTTPS MCP. Les URL des tunnels MCP sécurisés peuvent inclure le chemin complet /v1/mcp/tunnel_id ; les en-têtes de requête transférés ne sont jamais considérés comme une identité OAuth.
settings-mcp-oauth-issuer-detail = Définissez l’émetteur du serveur d’autorisation public accessible au navigateur. OAuth géré par Agena nécessite une origine sans chemin, telle que https://auth.example.com ; laissez-le vide lorsque OAuth et MCP utilisent le même domaine.
settings-mcp-oauth-password-detail = Définissez le mot de passe affiché sur la page d'autorisation Agena OAuth. Il est stocké par le serveur sous forme de hachage Argon2.
settings-mcp-oauth-password-clear-detail = Supprimez le mot de passe spécifique à MCP et revenez au mot de passe de l'interface utilisateur du serveur, s'il est configuré.
settings-field-runtime-codex-version-description = Version exacte de compatibilité @openai/codex utilisée dans les en-têtes d'identité de demande du fournisseur.
settings-field-runtime-claude-version-description = Version exacte de compatibilité @anthropic-ai/claude-code utilisée dans les en-têtes d'identité de demande du fournisseur.
settings-field-runtime-gemini-version-description = Version exacte de compatibilité @google/gemini-cli utilisée dans les en-têtes d'identité de demande du fournisseur.
settings-field-session-compaction-auto-description = Compactez automatiquement les sessions à mesure qu'elles approchent de la limite de la fenêtre contextuelle.
settings-field-session-compaction-reserved-tokens-description = Jetons réservés depuis la fenêtre contextuelle lors du choix du moment de compactage ; clear pour utiliser la valeur par défaut calculée.
settings-client-versions-refresh-description = Récupérez les dernières versions de packages compatibles à partir de npm, conservez les trois valeurs exactes et rechargez le runtime.
settings-client-versions-entry-detail = Ouvrez les versions de compatibilité exactes utilisées dans les en-têtes d'identité de demande du fournisseur.
settings-client-versions-section-description = Versions de compatibilité exactes utilisées dans les en-têtes d'identité de demande du fournisseur. Modifiez chaque valeur ou appuyez sur Ctrl+R pour actualiser depuis npm.
settings-provider-workbench-detail = Ouvrez la liste des fournisseurs consultables avant de configurer l'authentification, les adaptateurs, le routage de modèle ou les nouveaux fournisseurs.
settings-provider-new-detail = Créez un nouveau fournisseur, répertoriez les modèles d'adaptateur en direct et modifiez la configuration de l'adaptateur de fournisseur ; choisissez le modèle séparément.
settings-model-catalog-open-detail = Inspectez les métadonnées du modèle résolu et actualisez le cache du catalogue de modèles local.
permission-studio-command-rules-shell-only = Les règles de commande s’appliquent à shell.exec, shell.spawn, shell.watch, shell.open et shell.write ; utilisez une règle de nom ou la règle par défaut pour les autres outils.
permission-studio-detail-editable = Enter ouvre un éditeur JSON multiligne pour cette tranche d’autorisation.
permission-studio-detail-add-hint = Enter crée cet élément et l'ouvre immédiatement.
permission-studio-detail-full-config-editable = Entrée ouvre l'éditeur JSON avancé pour le document complet.
overlay-permission-studio-delete-title = Supprimer la règle
overlay-permission-studio-delete-body = Supprimer { $kind } : { $value }
flash-permission-studio-no-add = Aucun élément ne peut être ajouté dans la section actuelle.
flash-permission-studio-no-delete = Aucun élément ne peut être supprimé dans la section actuelle.
flash-permission-studio-no-selection = Sélectionnez d'abord un élément.
flash-permission-studio-context-lost = Le contexte de l'éditeur d'autorisations a été perdu. Rouvrez le studio d'autorisation et réessayez.
value-default = par défaut
value-none = aucun
value-clear = clair
value-path = chemin
value-network = réseau
value-workspace = espace de travail
value-external = externe
value-permission-filesystem = Système de fichiers
value-permission-network = Réseau
value-permission-tools = Outils
value-rule-count = { $count } règle(s)
value-custom = personnalisé
value-internet = Internet
value-private = privé
value-loopback = bouclage
value-name-count = { $count } nom(s)
value-rule-set-count = { $count } ensemble(s) de règles
value-open = ouvert
composer-prompt-history-title = Historique rapide
overlay-commands-title = Palette de commandes
overlay-commands-prompt = Actions de recherche ; les commandes qui nécessitent du texte continuent dans le compositeur
overlay-lineage-title = Historique de la succursale [#{ $session }]
overlay-lineage-prompt = Explorez l'arborescence des branches actuelle et accédez à une session ancêtre, frère ou enfant.
overlay-rewind-title = Rembobinage de la session [#{ $session }]
overlay-rewind-prompt = Choisissez le message utilisateur à retirer, ainsi que tout ce qui suit
overlay-picker-loading = Chargement...
overlay-picker-empty = Aucun élément correspondant
overlay-picker-footer = L'onglet remplit l'étiquette sélectionnée
session-model-context-window = { $value } ctx
session-model-max-output = sortie { $value }
overlay-provider-studio-detail-footer = Les touches fléchées sélectionnent · Entrée, modification · Echap retour ; les actions d'authentification sont visibles sur la page principale du fournisseur
overlay-provider-studio-configured-disk = configuré sur disque ; ne fait pas partie du contrat d'autorisation actuel
overlay-provider-studio-new-model-prompt = Entrez l'identifiant du modèle à ajouter sous l'adaptateur sélectionné.
provider-field-provider-id = Identifiant du fournisseur
provider-field-auth-mode = Mode d'authentification
provider-field-auth-subtype = Sous-type d'authentification
provider-field-auth-login-method = Méthode de connexion authentifiée
provider-field-start-auth = Démarrer l'authentification
provider-field-continue-auth = Continuer l'authentification
provider-field-auth-details = Détails d'authentification
provider-field-base-url = URL de base
provider-field-instance-url = URL de l'instance
provider-field-api-key-source = Source de clé API
provider-field-api-key-value = Valeur de la clé API
provider-field-redirect-uri = URI de redirection
provider-field-callback-url = URL de rappel
provider-field-refresh-token = Actualiser le jeton
provider-field-access-token = Jeton d'accès
provider-field-expires-at-ms = Expire à (ms)
provider-field-account-id = Identifiant du compte
provider-field-enterprise-domain = Domaine d'entreprise
provider-field-region = Région
provider-field-profile = Profil
provider-field-access-key-id = ID de clé d'accès
provider-field-secret-access-key = Clé d'accès secrète
provider-field-session-token = Jeton de session
provider-field-service-key-env = Environnement de clé de service
provider-field-request-timeout = Délai d'expiration de la demande (secondes)
provider-field-connect-timeout = Délai d'expiration de la connexion (secondes)
provider-field-adapter-id = ID de l'adaptateur
provider-field-model-id = ID du modèle
provider-model-field-model-id = ID du modèle
provider-model-field-enabled = Activé
provider-model-field-native-compaction = Compactage natif
provider-model-field-manual-overrides = Remplacements manuels
provider-model-field-agena-tool-mode = Mode outil (agena_tools.mode)
agena-tool-mode-provider-protocol-label = protocole_fournisseur
agena-tool-mode-provider-protocol-detail = Transportez les définitions et les appels d'outils gérés par Agena via le protocole d'outil de l'API du fournisseur.
agena-tool-mode-disabled-label = désactivé
agena-tool-mode-disabled-detail = N'exposez pas les outils gérés par Agena ou natifs du fournisseur à ce modèle.
provider-model-field-display-name = Nom d'affichage
provider-model-field-lifecycle = Cycle de vie
provider-model-field-context-window = Fenêtre contextuelle
provider-model-field-max-input = Entrée maximale
provider-model-field-max-output = Sortie maximale
provider-model-field-features = Caractéristiques
provider-model-field-input-modalities = Modalités de saisie
provider-model-field-output-modalities = Modalités de sortie
provider-model-field-thinking-modes = Modes de réflexion
provider-model-field-speed-modes = Modes de vitesse
provider-model-field-description = Descriptif
provider-model-enabled-detail = Indique si cet itinéraire modèle est activé.
provider-model-native-compaction-detail = Essayez le point de terminaison de compactage de conversation natif de ce fournisseur avant de recourir au résumé de texte d'Agena.
provider-model-manual-overrides-detail = Active : cette route garde ses propres valeurs. Desactive : elle suit le catalogue et recoit ses mises a jour au prochain listage.
provider-model-lifecycle-detail = Valeur du cycle de vie du modèle.
provider-auth-mode-none-detail = désactiver les métadonnées d'authentification du fournisseur
provider-auth-mode-api-detail = Authentification de style API avec un sous-type de deuxième étape pour les points de terminaison HTTP personnalisés, l'API Cline, les jetons de passerelle GitLab ou Bedrock SigV4
provider-auth-mode-credential-detail = authentification basée sur des informations d'identification résolue à partir d'un émetteur local, sélectionné dans le champ de sous-type d'authentification
provider-auth-kind-unset = désarmé
provider-auth-kind-none = aucun
provider-auth-kind-api = API
provider-auth-kind-cline = cline_api
provider-auth-kind-gitlab = gitlab_api
provider-auth-kind-credential = informations d'identification
provider-auth-kind-credential-with-issuer = identifiant :{ $issuer }
provider-auth-kind-bedrock = substrat rocheux_sigv4
provider-auth-subtype-custom-label = personnalisé
provider-auth-subtype-custom-detail = Clé API générique + authentification URL de base pour les fournisseurs HTTP compatibles OpenAI, Anthropic ou Gemini
provider-auth-subtype-cline-api-detail = Correction du point de terminaison de l'API Cline ; seule la saisie de la clé API est nécessaire et la découverte de modèles utilise les modèles recommandés par Cline
provider-api-key-source-inline-detail = Stockez la clé API en ligne dans la configuration du fournisseur
provider-api-key-source-env-detail = Lire la clé API à partir d'une variable d'environnement
provider-auth-subtype-gitlab-api-detail = Authentification du jeton GitLab acheminée via des adaptateurs openai ou anthropiques
provider-auth-subtype-bedrock-detail = Signature AWS Bedrock SigV4
provider-auth-login-kind-browser-label = Navigateur OAuth
provider-auth-login-kind-device-label = Connexion par code d'appareil
provider-auth-login-kind-browser-detail = Ouvrez l'URL d'autorisation, puis terminez le rappel redirigé.
provider-auth-login-kind-device-detail = Ouvrez une courte URL de vérification, saisissez un code d'appareil, puis interrogez.
provider-issuer-openai-chatgpt-label = openai_chatgpt
provider-issuer-github-copilot-label = github_copilot
provider-issuer-gitlab-label = gitlab
provider-issuer-google-adc-label = google_adc
provider-issuer-sap-ai-core-label = sap_ai_core
provider-issuer-openai-chatgpt-detail = Identifiants OpenAI ChatGPT
provider-issuer-github-copilot-detail = Identifiants GitHub Copilot
provider-issuer-gitlab-detail = Identifiants GitLab OAuth
provider-issuer-google-adc-detail = Informations d'identification par défaut de l'application Google
provider-issuer-sap-ai-core-detail = Authentification de la clé du service SAP AI Core
provider-instance-url-gitlab-detail = Point de terminaison OAuth du navigateur GitLab.com
provider-redirect-local-copy-detail = URL de rappel localhost pour les redirections copier/coller OAuth
provider-region-choice-detail = Région AWS
provider-service-key-env-detail = variable d'environnement de clé de service SAP AI Core par défaut
overlay-model-catalog-field-model-id = ID du modèle
overlay-model-catalog-field-display = Affichage
overlay-model-catalog-field-origin = Origine
overlay-model-catalog-field-lifecycle = Cycle de vie
overlay-model-catalog-field-dates = Dates
overlay-model-catalog-field-limits = Limites
overlay-model-catalog-field-inputs = Entrées
overlay-model-catalog-field-output = Sortie
overlay-model-catalog-field-features = Caractéristiques
overlay-model-catalog-field-modes = Modes
overlay-model-catalog-field-defaults = Valeurs par défaut
overlay-model-catalog-field-runtime = Durée d'exécution
overlay-model-catalog-field-pricing = Tarifs
overlay-model-catalog-field-source = Source
overlay-model-catalog-limits = ctx { $context } · entrée { $input } · sortie { $output }
overlay-model-catalog-lifecycle-active = actif
overlay-model-catalog-lifecycle-preview = aperçu
overlay-model-catalog-lifecycle-beta = bêta
overlay-model-catalog-lifecycle-alpha = alpha
overlay-model-catalog-lifecycle-experimental = expérimental
overlay-model-catalog-lifecycle-deprecated = obsolète
overlay-model-catalog-date-release = version { $value }
overlay-model-catalog-date-updated = mis à jour { $value }
overlay-model-catalog-date-cutoff = seuil { $value }
overlay-model-catalog-default-thinking = pense
overlay-model-catalog-default-speed = vitesse
overlay-model-catalog-thinking-modes = modes de réflexion
overlay-model-catalog-speed-modes = modes de vitesse
overlay-model-catalog-default-verbosity = verbosité
overlay-model-catalog-default-temperature = temp.
overlay-model-catalog-default-top-p = top_p
overlay-model-catalog-default-top-k = top_k
overlay-model-catalog-parallel-tools = outils parallèles
overlay-model-catalog-supports-verbosity = verbosité
overlay-model-catalog-reasoning-interleaved = raisonnement entrelacé
overlay-model-catalog-reasoning-field = champ de raisonnement
overlay-model-catalog-open-weights = poids ouverts
overlay-model-catalog-price-input = dans { "$" }{ $value }/M
overlay-model-catalog-price-output = sortie { "$" }{ $value }/M
overlay-model-catalog-price-cache-read = lecture du cache { "$" }{ $value }/M
overlay-model-catalog-price-cache-write = cache écriture { "$" }{ $value }/M
overlay-model-catalog-tier-count = { $count } niveau(s)
permission-rule-label-path = { $access } · { $path }
permission-rule-label-network = réseau · { $target }
value-unset = désarmé
value-auto = automobile
value-allow = permettre
value-ask = demander
value-deny = nier
value-read = lire
value-write = écrire
value-read-write = lecture_écriture
value-yes = oui
value-no = non
value-session = séance
value-global = mondiale
value-add = Ajouter
value-runtime-default = valeur par défaut d'exécution
value-permission-rule-subject-tool = outil
value-permission-rule-subject-path-access = chemin_accès
value-permission-rule-subject-network-access = accès_réseau
inline-fact-source = source
inline-fact-scope = portée
inline-fact-operator = opérateur
flash-permission-rule-saved = règle d'autorisation enregistrée : { $name }
flash-permission-rule-revoked = règle d'autorisation révoquée : { $name }
flash-permission-rule-context-lost = le contexte du studio de règles d'autorisation a été perdu
flash-provider-studio-context-lost = le contexte de configuration du fournisseur a été perdu
permission-rule-error-session-id-integer = l'identifiant de session doit être un entier
permission-rule-error-tool-name-required = les règles d'outil nécessitent un nom d'outil
permission-rule-error-path-access-kind-required = les règles de chemin nécessitent path_access_kind
permission-rule-error-target-path-required = les règles de chemin nécessitent target_path
permission-rule-error-network-target-required = les règles réseau nécessitent une cible réseau
permission-rule-error-session-id-required = la portée de la session nécessite un identifiant de session
flash-server-config-edit-in-settings = Le fichier de configuration appartient au serveur. Modifiez ses valeurs dans Paramètres au lieu d'ouvrir un chemin client local.
flash-command-requires-session = cette action nécessite une session ouverte
flash-session-busy = la séance est occupée
flash-provider-not-found = fournisseur introuvable : { $provider }
flash-permission-approval-model-updated = modèle d'approbation automatique mis à jour : { $provider }/{ $model }
flash-global-default-model-updated = modèle global par défaut mis à jour : { $provider }/{ $model }
flash-workspace-default-model-updated = modèle par défaut de l’espace de travail mis à jour : {$provider}/{$model}
flash-workspace-approval-model-updated = modèle d’approbation automatique de l’espace de travail mis à jour : {$provider}/{$model}
flash-provider-studio-adapter-required = sélectionnez d'abord un adaptateur
flash-provider-studio-adapter-not-enabled = vérifiez l'adaptateur sélectionné avant d'ajouter un modèle
flash-provider-studio-adapter-unavailable = le mode d'authentification actuel ne permet pas de sélectionner cet adaptateur
flash-provider-studio-model-required = sélectionnez d'abord un modèle répertorié
flash-provider-studio-model-id-required = l'identifiant du modèle est requis
flash-provider-studio-no-auth-details = aucun détail d'authentification n'est disponible pour le mode d'authentification actuel
flash-provider-studio-catalog-refreshed = catalogue de modèles actualisé
flash-provider-studio-invalid-model-json = modèle JSON non valide : { $error }
flash-provider-studio-live-listing-unavailable = La liste des modèles vivants n'est pas disponible pour l'authentification { $auth }
flash-provider-studio-draft-listing-unsupported = La liste de modèles préliminaires prend uniquement en charge les adaptateurs avec découverte de modèles en direct. Non pris en charge : { $adapters }
flash-provider-studio-listing-auth-required = la liste des modèles d'adaptateur nécessite la découverte de modèles en direct pour la paire authentification/adaptateur actuelle ou pour un fournisseur enregistré existant ; l'authentification actuelle est { $auth }
flash-provider-studio-invalid-auth-login-method = méthode de connexion d'authentification invalide
flash-provider-auth-openai-browser-started = L'authentification du navigateur OpenAI a démarré. Ouvrez l'URL d'autorisation affichée dans la boîte de dialogue, puis collez l'URL redirigée dans l'URL de rappel et appuyez sur p.
flash-provider-auth-openai-device-started = La connexion à l'appareil OpenAI a démarré. Ouvrez l'URL de vérification affichée dans la boîte de dialogue, entrez le code { $code }, puis appuyez sur p.
flash-provider-auth-copilot-device-started = La connexion au périphérique Copilot a démarré. Ouvrez l'URL de vérification affichée dans la boîte de dialogue, entrez le code { $code }, puis appuyez sur p.
flash-provider-auth-gitlab-browser-started = L'authentification du navigateur GitLab a démarré. Ouvrez l'URL d'autorisation affichée dans la boîte de dialogue, puis collez l'URL redirigée dans l'URL de rappel et appuyez sur p.
flash-provider-auth-atomgit-browser-started = L'authentification du navigateur AtomGit a démarré. Ouvrez l'URL d'autorisation affichée dans la boîte de dialogue, terminez la connexion, puis appuyez sur p pour interroger.
flash-provider-auth-openai-captured = Informations d'identification OpenAI OAuth capturées dans le brouillon.
flash-provider-auth-openai-pending = La connexion à l'appareil OpenAI est toujours en attente. Terminez l’étape de vérification, puis appuyez à nouveau sur p.
flash-provider-auth-copilot-pending = La connexion au périphérique Copilot est toujours en attente. Terminez l'approbation du navigateur, puis appuyez à nouveau sur p.
flash-provider-auth-copilot-captured = Informations d'identification du copilote OAuth capturées dans le brouillon.
flash-provider-auth-gitlab-captured = Informations d'identification GitLab OAuth capturées dans le brouillon.
flash-provider-auth-atomgit-pending = La connexion au navigateur AtomGit est toujours en attente. Terminez le flux du navigateur, puis appuyez à nouveau sur p.
flash-provider-auth-atomgit-captured = Informations d'identification AtomGit OAuth capturées dans le brouillon.
flash-provider-auth-error-unsupported = le mode d'authentification actuel ne prend pas en charge la connexion OAuth interactive
flash-provider-auth-error-start-browser-first = démarrez d'abord l'authentification du navigateur avec Start Auth ou o
flash-provider-auth-error-start-device-first = démarrez d'abord l'authentification de l'appareil avec Start Auth ou o
flash-provider-auth-error-required-field = { $field } est requis
flash-provider-save-draft = Fournisseur enregistré { $provider } avec l'adaptateur { $adapter }.
flash-provider-save-adapter-matches = { $provider }/{ $adapter } enregistré avec { $listed } modèle(s) répertorié(s) ; Le catalogue { $matched } correspond.
flash-provider-save-model = { $provider }/{ $adapter }/{ $model } enregistré.
flash-provider-save-configured-model = Modèle configuré enregistré { $provider }/{ $adapter }/{ $model }.
flash-provider-delete-provider = Fournisseur supprimé { $provider }.
flash-provider-delete-adapter = Adaptateur configuré supprimé { $provider }/{ $adapter } et modèles { $count } supprimés.
flash-provider-delete-model = Modèle configuré supprimé { $provider }/{ $adapter }/{ $model }.
flash-provider-studio-adapter-delete-empty = Aucun paramètre d'adaptateur n'est sélectionné pour être supprimé.
flash-provider-save-error-required-field = { $field } est requis
flash-provider-save-error-unsupported-adapters = auth { $auth } ne prend pas en charge les adaptateurs : { $adapters } ; attendu le { $supported }
flash-provider-save-error-api-base-url = L'authentification API nécessite base_url lors de l'utilisation du protocole OpenAI, des adaptateurs Anthropic ou Gemini
flash-provider-save-error-gitlab-token = l'authentification gitlab_api nécessite une source de clé API
flash-provider-save-error-credential-base-url = l'émetteur des informations d'identification `{ $issuer }` nécessite base_url
flash-provider-save-error-credential-service-key-env = l'émetteur des informations d'identification `{ $issuer }` nécessite service_key_env
flash-provider-save-error-bedrock-key-pair = bedrock_sigv4 nécessite access_key_id et secret_access_key ensemble
flash-provider-save-error-select-model = sélectionner au moins un modèle avant de sauvegarder le fournisseur
flash-provider-save-error-adapter-object = l'adaptateur de fournisseur `{ $adapter }` doit être un objet JSON
flash-provider-save-error-model-object = La configuration du modèle de fournisseur doit être un objet JSON
flash-provider-save-error-configured-adapter-object = les paramètres de l'adaptateur de fournisseur configurés doivent être un objet JSON
flash-provider-save-error-configured-models-object = les modèles d'adaptateur de fournisseur configurés doivent être un objet JSON
flash-provider-client-versions-refreshed = Versions client mises à jour : Codex { $codex }, Claude { $claude }, Gemini { $gemini }
terminal-diagnostics-title = Diagnostic des terminaux
terminal-diagnostics-eyebrow = Preuve de compatibilité et de protocole
terminal-diagnostics-footer = ↑/↓ faire défiler · c/y copier le rapport · Esc fermer
terminal-diagnostics-tip = Les couches d’identité et d’environnement du produit sont fondées sur des preuves ; SSH générique ne peut pas prouver le véritable terminal de point de terminaison.
terminal-diagnostics-copied = Diagnostic du terminal copié
terminal-diagnostics-unavailable = Les diagnostics du terminal ne sont pas disponibles dans ce runtime.
terminal-diagnostics-summary = Rapport final étayé par des preuves · confiance du point final { $confidence }
terminal-diagnostics-none = aucun
terminal-diagnostics-unknown = inconnu
terminal-diagnostics-unavailable-value = indisponible
terminal-diagnostics-term-unset = TERME n'est pas défini
terminal-diagnostics-section-identity = Identité
terminal-diagnostics-section-layers = Couches d'environnement
terminal-diagnostics-section-color = Couleur et apparence
terminal-diagnostics-section-protocols = Protocoles actifs
terminal-diagnostics-section-providers = Fournisseurs et intégrations
terminal-diagnostics-section-warnings = Avertissements
terminal-diagnostics-field-product = Produit
terminal-diagnostics-field-version = Version
terminal-diagnostics-field-parsed-version = Version analysée
terminal-diagnostics-field-compatibility = Compatibilité
terminal-diagnostics-field-confidence = Confiance
terminal-diagnostics-field-source = Source sélectionnée
terminal-diagnostics-field-evidence = Preuve
terminal-diagnostics-field-conflicts = Conflits
terminal-diagnostics-color-configured = Mode configuré
terminal-diagnostics-color-detected-background = Arrière-plan détecté
terminal-diagnostics-color-detected-appearance = Apparence détectée
terminal-diagnostics-color-source = Source de détection
terminal-diagnostics-color-refresh = Actualisation automatique
terminal-diagnostics-color-generation = Génération d'apparence
terminal-diagnostics-color-effective-appearance = Palette de texte efficace
terminal-diagnostics-color-formula-foreground = Couleur du glyphe de formule
terminal-diagnostics-color-formula-background = Fond d'image de formule
terminal-diagnostics-color-background-images = Images d'arrière-plan
terminal-diagnostics-color-mode-auto = Automatique
terminal-diagnostics-color-mode-dark = Obscurité forcée
terminal-diagnostics-color-mode-light = Lumière forcée
terminal-diagnostics-color-appearance-dark = Sombre
terminal-diagnostics-color-appearance-light = Lumière
terminal-diagnostics-color-appearance-unknown = Inconnu
terminal-diagnostics-color-appearance-conservative = Couleurs conservatrices natives du terminal (arrière-plan inconnu)
terminal-diagnostics-color-source-osc11 = Réponse du terminal OSC 11
terminal-diagnostics-color-source-iterm-osc4 = Réponse du terminal iTerm2 OSC 4 ; -2
terminal-diagnostics-color-source-colorfgbg = Environnement de secours COLORFGBG
terminal-diagnostics-color-source-term-background = TERM_BACKGROUND environnement de secours
terminal-diagnostics-color-source-vscode-theme = Remplacement de l'environnement VSCODE_THEME_KIND
terminal-diagnostics-color-source-unavailable = Aucun terminal utilisable ni preuve d'environnement
terminal-diagnostics-color-refresh-live = Sur la récupération de la concentration et la reprise du terminal ; les actualisations échouées conservent la dernière couleur connue
terminal-diagnostics-color-refresh-startup-only = Démarrage uniquement ; le terminal n'a pas répondu à une requête de couleur actualisable
terminal-diagnostics-color-formula-background-transparent = Transparente ; seule la couleur du glyphe de formule suit l'apparence
terminal-diagnostics-color-background-images-not-sampled = Non échantillonné ; les pixels de formule transparents préservent l'arrière-plan du terminal ou l'image d'arrière-plan en dessous
terminal-diagnostics-direct = Direct
terminal-diagnostics-direct-description = Aucune preuve SSH, Mosh, multiplexeur ou WSL détectée.
terminal-diagnostics-layer-description = Détecté à partir de { $source }. L’ordre des couches et la profondeur de nidification sont inconnus.
terminal-diagnostics-capability-description = point final={ $status } · source={ $source } · chemin={ $path } · fournisseur={ $provider }
terminal-diagnostics-path-clear = clair
terminal-diagnostics-path-forced = forcé par dérogation
terminal-diagnostics-path-unverified = non vérifié
terminal-diagnostics-path-blocked = bloqué
terminal-diagnostics-provider-not-required = pas obligatoire
terminal-diagnostics-provider-ready = prêt
terminal-diagnostics-provider-missing = manquant ou non mis en œuvre
terminal-diagnostics-helper-missing = Introuvable ou non exécutable.
terminal-diagnostics-helper-not-probed = Non sondé car le point de terminaison n’est pas identifié comme Kitty.
terminal-diagnostics-no-warnings = Aucun avertissement de compatibilité n'a été détecté.
terminal-diagnostics-protocol-alternate-screen = Écran alternatif
terminal-diagnostics-protocol-bracketed-paste = Pâte entre parenthèses
terminal-diagnostics-protocol-focus = Rapports ciblés
terminal-diagnostics-protocol-mouse = Capture de souris
terminal-diagnostics-protocol-mouse-mode = Mode filaire de la souris
terminal-diagnostics-protocol-mouse-events = Événements de souris reçus
terminal-diagnostics-protocol-mouse-last = Dernier événement de souris
terminal-diagnostics-mouse-mode-button-sgr = Suivi des événements de bouton (DECSET 1002) avec coordonnées SGR (DECSET 1006)
terminal-diagnostics-mouse-events-none = Aucun. Le terminal de point de terminaison n'a transmis aucun événement de souris à Agena ; vérifiez ses paramètres de profil de rapport de souris et de rapport de roue.
terminal-diagnostics-mouse-events-seen = { $count } événement(s)
terminal-diagnostics-mouse-last-none = Aucun
terminal-diagnostics-protocol-keyboard = Désambiguïsation du clavier
terminal-diagnostics-protocol-key-events = Types d'événements de clavier
terminal-diagnostics-protocol-background = Requête en arrière-plan
terminal-diagnostics-protocol-native-clipboard = Presse-papiers natif
terminal-diagnostics-protocol-osc52-write = OSC 52 écrire
terminal-diagnostics-protocol-osc52-read = OSC 52 lire
terminal-diagnostics-protocol-progress = OSC 9;4 progrès
terminal-diagnostics-provider-kitty-clipboard = Presse-papiers Kitty
terminal-diagnostics-provider-kitty-transfer = Transfert de chat
terminal-diagnostics-provider-iterm-transfer = Transfert iTerm2
terminal-diagnostics-provider-inline-images = Images en ligne
terminal-diagnostics-provider-hyperlinks = Liens hypertextes
terminal-diagnostics-provider-sync-output = Sortie synchronisée
terminal-diagnostics-status-confirmed = confirmé
terminal-diagnostics-status-forced = forcé par dérogation
terminal-diagnostics-status-profiled = profilé
terminal-diagnostics-status-unsupported = non pris en charge
terminal-diagnostics-status-unknown = inconnu
terminal-diagnostics-source-user = remplacement de l'utilisateur
terminal-diagnostics-source-environment = environnement
terminal-diagnostics-source-helper = sonde auxiliaire
terminal-diagnostics-source-terminal-query = requête de terminal
terminal-diagnostics-source-profile = profil de terminal
terminal-diagnostics-source-platform = plate-forme par défaut
terminal-diagnostics-source-conservative = défaut conservateur
terminal-diagnostics-source-terminfo = compatibilité terminfo
terminal-diagnostics-source-unknown = inconnu
terminal-diagnostics-confidence-explicit = explicite
terminal-diagnostics-confidence-strong = fort
terminal-diagnostics-confidence-compatibility = compatibilité uniquement
terminal-diagnostics-confidence-unknown = inconnu

# Plugin Workbench i18n completion
plugin-workbench-action-diff = diff
plugin-workbench-action-refresh = actualiser
plugin-workbench-action-remove-selected = supprimer/réinitialiser la sélection
plugin-workbench-action-reset-all = tout réinitialiser
plugin-workbench-action-restart = redémarrer
plugin-workbench-action-save = enregistrer
plugin-workbench-action-validate = valider
plugin-workbench-actions = Actions
plugin-workbench-authority-unavailable = Les données d’autorité ne sont pas disponibles.
plugin-workbench-choices = Choix
plugin-workbench-close-footer = Échap fermer
plugin-workbench-column-after = Après
plugin-workbench-column-args = Args
plugin-workbench-column-arguments = Arguments
plugin-workbench-column-before = Avant
plugin-workbench-column-category = Catégorie
plugin-workbench-column-change = Modification
plugin-workbench-column-command = Commande
plugin-workbench-column-description = Description
plugin-workbench-column-field = Champ
plugin-workbench-column-inputs = Entrées
plugin-workbench-column-message = Message
plugin-workbench-column-plugin = Plugin
plugin-workbench-column-section = Section
plugin-workbench-column-severity = Gravité
plugin-workbench-column-source = Source
plugin-workbench-column-summary = Résumé
plugin-workbench-column-tool = Outil
plugin-workbench-column-version = Version
plugin-workbench-column-visible-tool = Outil visible
plugin-workbench-command-arguments = Arguments : {$command}
plugin-workbench-config = Configuration
plugin-workbench-config-action = Action
plugin-workbench-config-choose-shape = choisir la forme
plugin-workbench-config-choose-type = choisir le type
plugin-workbench-config-default = Par défaut
plugin-workbench-config-diff = Diff de configuration
plugin-workbench-config-dirty = modifié
plugin-workbench-config-drilldown-footer = Gauche/Droite cellule · Haut/Bas ligne · Entrée modifier · Ctrl+D supprimer/réinitialiser · Échap retour
plugin-workbench-config-saved = enregistré
plugin-workbench-config-setting = Paramètre
plugin-workbench-config-state = État
plugin-workbench-config-state-changed = modifié
plugin-workbench-config-state-default = par défaut
plugin-workbench-config-state-dirty = modifié
plugin-workbench-config-state-error = erreur
plugin-workbench-config-state-inactive = inactif
plugin-workbench-config-summary = {$status} · {$save_state}
plugin-workbench-config-title = {$plugin} / Configuration
plugin-workbench-config-type = Type
plugin-workbench-config-value = Valeur
plugin-workbench-config-view-summary = Configuration effective · {$changed} champs modifiés · cellule sélectionnée : {$cell}
plugin-workbench-detail-footer = Tab/Maj+Tab section · Haut/Bas défiler · Échap retour
plugin-workbench-detail-tools-footer = Tab/Maj+Tab section · Haut/Bas sélectionner · Entrée configurer et exécuter · Échap retour
plugin-workbench-filter-all = Tous
plugin-workbench-filter-other = autre
plugin-workbench-header-summary = Outils : {$tools}        Commandes : {$commands}        Configuration : {$config}
plugin-workbench-input-preview = Aperçu de l’entrée : {$tool}
plugin-workbench-last-result-failed = Dernier résultat · {$tool} · échec
plugin-workbench-last-result-success = Dernier résultat · {$tool} · réussi
plugin-workbench-list-footer = Saisir pour rechercher · Haut/Bas sélectionner · Entrée ouvrir · Échap fermer
plugin-workbench-list-summary = Rechercher des plugins… {$query}        Transport : {$transport}        Configuration : {$config}        {$shown}/{$total} affichés
plugin-workbench-loading-actions = Chargement des actions…
plugin-workbench-loading-choices = Chargement des choix…
plugin-workbench-no-changes = Aucune modification
plugin-workbench-no-commands = Aucune commande.
plugin-workbench-no-config-section = Aucune section de configuration.
plugin-workbench-no-editable-rows = Aucune ligne modifiable.
plugin-workbench-no-filter-matches = Aucun plugin ne correspond aux filtres actuels.
plugin-workbench-no-issues = Aucun problème
plugin-workbench-no-logs = Aucun journal.
plugin-workbench-no-selection = Aucun plugin sélectionné.
plugin-workbench-no-structured-arguments = Aucun argument structuré.
plugin-workbench-no-tools = Aucun outil.
plugin-workbench-none = aucun
plugin-workbench-none-declared = aucun déclaré
plugin-workbench-overview = Aperçu
plugin-workbench-package-summary = Paquet : {$package}
plugin-workbench-plugin = Plugin
plugin-workbench-plugin-capabilities = Capacités du plugin
plugin-workbench-plugins = Plugins
plugin-workbench-provenance = Provenance : {$provenance}
plugin-workbench-sections = Sections
plugin-workbench-severity-error = erreur
plugin-workbench-severity-warning = avertissement
plugin-workbench-status-invalid = Invalide
plugin-workbench-status-issues = Problèmes
plugin-workbench-status-missing = Manquant
plugin-workbench-status-needs-restart = Redémarrage requis
plugin-workbench-status-runtime-issue = Problème d’exécution
plugin-workbench-status-schema-missing = Schéma manquant
plugin-workbench-status-valid = Valide
plugin-workbench-status-warning = Avertissement
plugin-workbench-summary = Requête : {$query} · transport {$transport} · configuration {$config} · {$shown}/{$total} affichés
plugin-workbench-tab-capabilities = Capacités
plugin-workbench-tab-commands = Commandes
plugin-workbench-tab-config = Configuration
plugin-workbench-tab-diagnostics = Diagnostic
plugin-workbench-tab-logs = Journaux
plugin-workbench-tab-tools = Outils
plugin-workbench-tabs = Onglets
plugin-workbench-tags-summary = Étiquettes : {$tags}
plugin-workbench-tool-capabilities = Capacités des outils
plugin-workbench-tools-help = Haut/Bas sélectionne un outil. Entrée ouvre le formulaire de schéma géré par l’hôte ; Ctrl+S valide et exécute.
plugin-workbench-transport = Transport
plugin-workbench-trust-level = Niveau de confiance : {$level}
plugin-workbench-unavailable = indisponible

# Plugin Workbench structured editor i18n completion
plugin-workbench-editor-also-matches = correspond aussi à : {$matches}
plugin-workbench-editor-array-action-help = Entrée menu d’actions · Ctrl+D supprime la ligne sélectionnée
plugin-workbench-editor-array-preview = Configurer… ({$count} éléments)
plugin-workbench-editor-configure = Configurer…
plugin-workbench-editor-format = format : {$format}
plugin-workbench-editor-generic-object = Éditeur d’objet générique
plugin-workbench-editor-index = Indice
plugin-workbench-editor-item = Élément {$index}
plugin-workbench-editor-map = Éditeur de table
plugin-workbench-editor-no-fields = Aucun champ.
plugin-workbench-editor-no-items = Aucun élément.
plugin-workbench-editor-object = Éditeur d’objet
plugin-workbench-editor-object-action-help = Entrée menu d’actions · Ajouter un champ depuis la cellule Action
plugin-workbench-editor-object-array = Éditeur de tableau d’objets
plugin-workbench-editor-object-array-help = Modifier ouvre l’élément sélectionné dans le même éditeur structuré.
plugin-workbench-editor-object-preview = Configurer… ({$count} champs)
plugin-workbench-editor-preview = Aperçu
plugin-workbench-editor-primitive-array = Éditeur de tableau primitif
plugin-workbench-editor-readonly = lecture seule
plugin-workbench-editor-schema-missing = Schéma manquant        Éditeur structuré de base
plugin-workbench-editor-shape = Forme
plugin-workbench-editor-suggestions = Suggestions
plugin-workbench-editor-tuple = Éditeur de tuple
plugin-workbench-editor-type-summary = Type : {$type}        Éditeur de chemin : interface structurée
plugin-workbench-field-state-available = disponible
plugin-workbench-field-state-custom = personnalisé
plugin-workbench-field-state-map-key = clé de table
plugin-workbench-field-state-missing = manquant
plugin-workbench-field-state-optional = facultatif
plugin-workbench-field-state-required = obligatoire
plugin-workbench-kind-all-of = allOf
plugin-workbench-kind-any-of = anyOf
plugin-workbench-kind-array = tableau
plugin-workbench-kind-boolean = booléen
plugin-workbench-kind-integer = entier
plugin-workbench-kind-null = nul
plugin-workbench-kind-number = nombre
plugin-workbench-kind-object = objet
plugin-workbench-kind-one-of = oneOf
plugin-workbench-kind-string = chaîne
plugin-workbench-kind-value = valeur

overlay-provider-list-create-detail = Créez un brouillon de fournisseur, puis configurez l’authentification, les adaptateurs et les modèles.

overlay-provider-delete-body = Supprimer le fournisseur {$provider} ainsi que tous les adaptateurs/modèles configurés ?

overlay-provider-delete-adapter-body = Supprimer l’adaptateur configuré {$provider}/{$adapter} ?

overlay-provider-delete-adapter-last-body = Il s’agit du dernier adaptateur configuré. La confirmation supprimera le fournisseur.

overlay-provider-delete-model-body = Supprimer le modèle configuré {$provider}/{$adapter}/{$model} ?

permission-studio-page-tool-defaults = Accès aux outils / Valeurs par défaut

# Generic inline content resources
content-loading = Chargement de la sortie…
content-active = Sortie en direct
content-complete = Sortie terminée
content-interrupted = Sortie interrompue
content-gap = Une partie de la sortie source n’est plus conservée.
content-reconnecting = Reconnexion à la sortie…

content-window = Une partie de la sortie est hors de la fenêtre affichée.

# Chrome-only activity headlines (the stored English title is the fallback).
activity-title-answer = Réponse
activity-title-thinking = Réflexion
activity-title-text = Texte
activity-title-error = Erreur
activity-title-notice = Notification
activity-title-hook = Hook
activity-title-compaction = Compactage du contexte
activity-title-provider-retry = Nouvelle tentative du fournisseur
activity-title-turn-limit = Limite de tours
activity-title-user-input = Entrée utilisateur requise
activity-title-pasted-text = Texte collé

# Plugin Workbench, permission editor and transcript keys localised with the settings catalog.
permission-studio-access-read = {$pattern} · lecture
permission-studio-access-write = {$pattern} · écriture
plugin-workbench-editor-pair-prompt = Saisissez les deux valeurs pour {$title}.
plugin-workbench-error-field-exists = le champ `{$field}` existe déjà
plugin-workbench-error-field-name-empty = le nom du champ ne peut pas être vide
plugin-workbench-error-fixed-schema-type = {$title} a un type de schéma fixe
plugin-workbench-error-invalid-branch-selection = sélection de branche non valide
plugin-workbench-error-invalid-enum-selection = sélection d’énumération non valide
plugin-workbench-error-invalid-type-selection = sélection de type non valide
plugin-workbench-error-no-selectable-schema-type = {$title} n’a aucun type de schéma sélectionnable
plugin-workbench-error-no-selection = aucune sélection disponible
plugin-workbench-error-plugin-unavailable = le plugin `{$plugin}` n’est pas disponible
plugin-workbench-error-read-only = {$title} est en lecture seule
plugin-workbench-error-rename-failed = échec du renommage du champ
plugin-workbench-error-row-not-object-field = la ligne sélectionnée n’est pas un champ objet
plugin-workbench-error-schema-mismatch = la valeur de configuration du plugin ne correspond pas à son schéma.
plugin-workbench-error-unsupported-tab = le plugin `{$plugin}` a demandé un onglet Workbench non pris en charge `{$tab}` ; ouverture de Config
transcript-image-embedded = image intégrée

# Activities, composer, diagnostics and usage analytics.
activities-title = Activités en arrière-plan
activities-empty = Aucune activité en arrière-plan.
activities-empty-hint = Exécutez shell.spawn, shell.open ou tasks.run pour en créer une.
activities-section-active = En cours
activities-section-finished = Terminées
activities-footer-close = Esc fermer
activities-footer-move = ↑↓ déplacer
activities-footer-detail = Enter détails
activities-footer-stop = s {$control}
activities-footer-dismiss = d {$control}
activities-footer-clear = x effacer
activities-footer-filters = f/k/t filtres
activities-footer-refresh = r actualiser
activities-filter-active-only = en cours uniquement
activities-status-summary = {$active} en cours · {$finished} terminées
activities-status-unknown = inconnu
activities-row-message = {$message}
activities-detail-title = Détails / fin du journal
activities-detail-no-selection = Aucune sélection.
activities-detail-waiting = En attente de sortie…
activities-detail-no-output = Aucune sortie enregistrée.
activities-detail-tail-hint = Appuyez sur ↵ pour suivre le journal de cette activité.
activities-detail-command = $ {$command}
activities-detail-session = session #{$id}
activities-detail-source-part = partie source #{$id}
activities-detail-next-wake = prochain réveil {$at}
activities-detail-output-separator = suivi en direct @ {$seq}
activities-detail-lines-omitted = … {$count} lignes plus anciennes masquées
activity-command-reference-title = Commande : {$name}
activity-detail-type = Type : {$media_type}
activity-detail-size = Taille : {$size} octets
activity-detail-dimensions = Dimensions : {$width}×{$height}
activity-detail-preview-fields = {$count} champs
activity-detail-preview-items = {$count} éléments
composer-chip-command = [Commande : {$name}]
composer-command-line = Commande : {$name}
composer-clipboard-files = fichiers du presse-papiers : {$error}
composer-clipboard-image = image du presse-papiers : {$error}
composer-clipboard-text = texte du presse-papiers : {$error}
composer-error-attachment-path = le chemin de la pièce jointe n’est pas un fichier ordinaire ni un dossier de l’espace de travail du serveur : {$path}
composer-error-attachment-unavailable = le chemin de la pièce jointe n’est plus disponible sur le serveur : {$path}
composer-error-clipboard-attachment-empty = la pièce jointe du presse-papiers est vide : {$path}
composer-error-clipboard-attachment-not-file = la pièce jointe du presse-papiers n’est pas un fichier ordinaire : {$path}
composer-error-clipboard-attachment-filename = la pièce jointe du presse-papiers n’a pas de nom de fichier utilisable : {$path}
composer-error-clipboard-attachment-too-large = la pièce jointe du presse-papiers dépasse la limite d’envoi de 20 MiB : {$path}
flash-notification-copied = Copié depuis la notification.
flash-paste-limit-draft-unchanged = Le texte collé dépasse la limite de 1 MiB ; le brouillon n’a pas été modifié.
flash-ui-backend-task-failed = Échec de la tâche de commande du backend de l’interface : {$error}
help-diagnostics-kitty-line = Assistant Kitty {$version} · {$path} · presse-papiers {$clipboard} · transfert {$transfer}
permission-rule-label = {$tool} · {$qualifier}
rewind-turn-label = Tour {$sequence}
rewind-turn-detail = tour {$sequence} · {$time}
terminal-diagnostics-state-path-blocked = chemin bloqué
terminal-diagnostics-state-path-unverified = chemin non vérifié
terminal-diagnostics-state-provider-unavailable = fournisseur indisponible
transcript-diagram-title = ◇ Diagramme · {$label}
usage-title = Analyse de l’utilisation
usage-title-refreshing = Analyse de l’utilisation · actualisation…
usage-to-small = Le terminal est trop petit pour afficher l’analyse de l’utilisation
usage-view-providers = Fournisseurs
usage-sort-tokens = Tokens
usage-filter-all = Tous
usage-filter-period = période {$period}
usage-filter-view = vue {$view}
usage-filter-provider = fournisseur {$provider}
usage-filter-model = modèle {$model}
usage-filter-subagents = sous-agents {$state}
usage-filter-sort = tri {$sort}
usage-filter-refresh = actualiser
usage-footer = ↑/↓ lignes Esc fermer · Enter ouvrir la session
usage-footer-open-session = Enter ouvrir la session
usage-footer-rows = ↑/↓ lignes Esc fermer
usage-loading = Chargement des statistiques d’utilisation…
usage-total-cost = Coût total
usage-cost-line = enregistré {$recorded} · estimé {$estimated}
usage-tokens = Tokens
usage-tokens-line = entrée {$input} · sortie {$output} · raisonnement {$reasoning}
usage-sessions = Sessions
usage-active-days = {$count} jours actifs
usage-average-cost = {$cost} en moyenne/requête
usage-requests = Requêtes
usage-cache-hit = Accès au cache
usage-cache-line = lecture {$read} · écriture {$write} · 5 min {$write_5m} · 1 h {$write_1h}
usage-days-summary = {$days} jours actifs · {$average} en moyenne/jour · pic de {$peak} le {$date}
usage-unpriced-requests = ⚠ {$count} requêtes sans tarif
usage-empty = Aucune utilisation
usage-empty-filtered = Aucune utilisation pour les filtres sélectionnés
usage-collecting = Collecte de l’utilisation des messages…
usage-error-badge = Erreur
usage-error-body = Impossible de charger les statistiques d’utilisation
usage-error-hint = Appuyez sur Ctrl+R pour actualiser
usage-breakdown-provider = Répartition par fournisseur
usage-breakdown-model = Répartition par modèle
usage-breakdown-session = Répartition par session
usage-breakdown-daily = Répartition quotidienne
usage-by-provider = Par fournisseur
usage-by-model = Par modèle
usage-daily-activity = Activité quotidienne
usage-column-name = nom
usage-column-cost = coût
usage-column-tokens = tokens
usage-column-requests = requêtes
usage-column-cache = cache

# Remaining chrome keys for localized interface surfaces.
activities-status-pending = en attente
activities-status-running = en cours
activities-status-waiting = en attente
activities-status-paused = en pause
activities-status-succeeded = réussie
activities-status-failed = échouée
activities-status-cancelled = annulée
activities-status-stopped = arrêtée
activities-control-stop = arrêter
activities-control-pause = mettre en pause
activities-control-resume = reprendre
activities-control-delete = supprimer
activities-control-dismiss = retirer
activities-kind-shell = Shell
activities-kind-monitor = Surveillance
activities-kind-task = Tâche
activities-kind-cron = Tâche planifiée
activities-kind-runtime = Exécution
activities-kind-browser = Navigateur
activities-kind-activity = Activité
activity-detail-details = Détails
activity-detail-language = Langue
activity-detail-notice = Avis
activity-detail-request = Demande
activity-detail-source = Source
activity-section-error = Erreur
activity-section-input = Entrée
activity-section-output = Sortie
activity-title-plan-review = Révision du plan
composer-chip-file = [Fichier : {$name}]
composer-chip-folder = [Dossier : {$name}]
composer-clipboard-text-empty = le texte du presse-papiers est vide
composer-error-attachment-budget = la pièce jointe dépasse la limite de 20 MiB par fichier ou de 40 MiB par message
composer-error-attachment-outside-workspace = La pièce jointe doit se trouver dans l’espace de travail actif.
composer-error-clipboard-attachments-budget = les pièces jointes du presse-papiers dépassent la limite totale d’envoi de 40 MiB
composer-error-clipboard-file-changed = le fichier du presse-papiers a changé ou dépasse la limite de lecture
composer-error-empty-message = Le message est vide.
composer-error-paste-limit = Le texte collé dépasse la limite de 1 MiB. Le presse-papiers et le brouillon actuel restent inchangés.
composer-notice-paste-as-text = Le texte long a été conservé car la limite de pièces jointes est atteinte.
context-help-context-activities = Activités en arrière-plan
flash-attachment-prepare-failed = Une pièce jointe n’a pas pu être préparée ; vérifiez le brouillon et renvoyez-le.
flash-attachment-preparing = Préparation des pièces jointes ; le message sera envoyé dès qu’elles seront prêtes.
flash-clipboard-osc52-sent = Demande de presse-papiers envoyée au terminal (OSC 52).
flash-command-no-file-view = Les références de commandes n’ont pas de vue de fichier.
flash-download-outside-workspace = Les chemins de téléchargement doivent rester dans l’espace de travail actuel.
flash-download-relative-path = Utilisez un chemin relatif à l’espace de travail actuel.
flash-download-usage = Utilisation : /download <chemin-dans-l’espace-de-travail>
flash-ui-backend-queue-full = La file de commandes du backend de l’interface est pleine ; attendez la fin des opérations en cours
flash-ui-backend-unavailable = Le service de commandes du backend de l’interface est indisponible
help-diagnostics-kitty-helper = Assistant Kitty
message-role-tool = outil
permission-studio-command-pattern-suffix = + motif de commande
search-picker-preview-title = Aperçu
search-picker-search-failed = Échec de la recherche
search-picker-no-preview = Aucun aperçu disponible
search-picker-page = Page {$current}/{$total}
search-picker-searching = recherche…
search-picker-loading-more = chargement supplémentaire…
search-picker-error = erreur
search-picker-results = {$count} résultats
search-picker-footer-space = Space basculer · {$count} sélectionnés
search-picker-footer-enter = Enter confirmer
search-picker-footer-navigate-page = ↑/↓ naviguer · ←/→ page
search-picker-footer-page = ←/→ page
search-picker-footer-navigate = ↑/↓ naviguer
search-picker-footer-enter-select = Enter sélectionner
search-picker-footer-close = Esc fermer
settings-choice-log-level-detail = niveau de journalisation
terminal-diagnostics-state-confirmed = confirmé
terminal-diagnostics-state-forced = forcé
terminal-diagnostics-state-profiled = déduit du profil
terminal-diagnostics-state-unsupported = non pris en charge
terminal-diagnostics-state-unknown = inconnu
timeline-label-role = Rôle
timeline-label-state = État
timeline-label-revision = Révision
timeline-label-run = Exécution
timeline-label-parent-part = Partie parente
timeline-summary-permission-requested = autorisation demandée [{$risk}] : {$reason}
timeline-summary-permission-replied = réponse à l’autorisation : {$kind}
timeline-summary-permission-rule-created = règle d’autorisation n° {$id} créée
timeline-summary-permission-rule-updated = règle d’autorisation n° {$id} mise à jour
timeline-summary-permission-rule-revoked = règle d’autorisation n° {$id} révoquée
timeline-detail-permission-session-id = session_id : {$id}
timeline-detail-permission-request-id = request_id : {$id}
timeline-detail-permission-reason = motif : {$reason}
timeline-detail-permission-risk = risque : {$risk}
timeline-detail-permission-explanation = explication : {$explanation}
timeline-detail-permission-source = source : {$source}
timeline-detail-permission-scope = portée : {$scope}
timeline-detail-permission-operator = opérateur : {$operator}
timeline-detail-permission-reply-kind = type : {$kind}
timeline-detail-permission-rule-id = rule_id : {$id}
timeline-detail-permission-rule-action-key = action_key : {$action}
timeline-detail-permission-rule-mode = mode : {$mode}
timeline-detail-permission-rule-scope = portée : {$scope}
timeline-detail-permission-rule-source = source : {$source}
timeline-type-execution-started = exécution démarrée
timeline-type-execution-completed = exécution terminée
timeline-type-execution-cancelled = exécution annulée
timeline-type-execution-failed = échec de l’exécution
timeline-type-compaction-completed = condensation du contexte terminée
timeline-type-subtask-status-changed = état de la sous-tâche modifié
timeline-type-stream-error = erreur de flux
timeline-type-message-part-checkpointed = partie du message enregistrée comme point de contrôle
timeline-type-message-part-delta = variation d’une partie du message
timeline-type-command-begin = début de commande
timeline-type-command-output-delta = variation de la sortie de commande
timeline-type-command-end = fin de commande
timeline-type-permission-requested = autorisation demandée
timeline-type-permission-replied = réponse à l’autorisation
timeline-type-permission-rule-created = règle d’autorisation créée
timeline-type-permission-rule-updated = règle d’autorisation mise à jour
timeline-type-permission-rule-revoked = règle d’autorisation révoquée
timeline-type-session-goal-updated = objectif de session mis à jour
timeline-type-run-started = exécution lancée
timeline-type-run-completed = exécution terminée
timeline-type-run-aborted = exécution interrompue
timeline-type-user-message-appended = message utilisateur ajouté
timeline-type-assistant-message-completed = réponse de l’assistant terminée
timeline-type-tool-call-issued = appel d’outil lancé
timeline-type-tool-call-completed = appel d’outil terminé
timeline-type-plugin-event = événement du plugin
timeline-summary-execution-started = session n° {$id}
timeline-summary-message-part-checkpointed = message n° {$message_id}, partie n° {$part_id} enregistrée ({$kind})
timeline-summary-message-part-delta = message n° {$message_id}, partie n° {$part_id} {$field} (+{$count} caractères)
timeline-summary-command-output-bytes = {$count} octets
timeline-summary-command-output-delta = {$stream} {$preview}
timeline-summary-command-end = {$status} code de sortie={$exit_code} ({$duration_ms} ms)
timeline-summary-session-goal-updated = objectif mis à jour : {$status} · {$objective}
timeline-summary-run-started = exécution {$id}
timeline-summary-run-completed = exécution {$id} ({$finish})
timeline-summary-run-aborted = exécution {$id} interrompue ({$reason})
timeline-summary-user-message-appended = utilisateur n° {$id}
timeline-summary-assistant-message-completed = assistant n° {$id} ({$finish})
timeline-summary-tool-call-issued = outil {$name}, appel={$call_id}
timeline-summary-tool-call-completed = appel d’outil {$call_id} terminé
timeline-summary-system-notice-appended = système n° {$message_id} : {$kind}
timeline-summary-plugin-event = plugin {$plugin_id}/{$kind_label}
timeline-label-seq = séquence
timeline-label-created = créé le
timeline-label-type = type
timeline-label-event-id = identifiant de l’événement
timeline-label-causation-id = identifiant de la cause
timeline-label-correlation-id = identifiant de corrélation
timeline-label-session-id = identifiant de session
timeline-label-error-code = code d’erreur
timeline-label-outcome = résultat
timeline-label-error-message = message d’erreur
timeline-label-message-id = identifiant du message
timeline-label-part-id = identifiant de la partie
timeline-label-part-kind = type de partie
timeline-label-status = état
timeline-label-summary = résumé
timeline-label-field = champ
timeline-label-delta = variation
timeline-label-call-id = identifiant de l’appel
timeline-label-command = commande
timeline-label-cwd = répertoire courant (cwd)
timeline-label-stream = flux
timeline-label-bytes = octets
timeline-label-preview = aperçu
timeline-label-exit-code = code de sortie
timeline-label-duration-ms = durée (ms)
timeline-label-request-id = identifiant de la demande
timeline-label-reason = motif
timeline-label-risk = risque
timeline-label-explanation = explication
timeline-label-source = source
timeline-label-scope = portée
timeline-label-operator = opérateur
timeline-label-requested-actions = actions demandées
timeline-label-related-actions = actions associées
timeline-label-reply-kind = type de réponse
timeline-label-rule-id = identifiant de règle
timeline-label-action-key = clé d’action
timeline-label-mode = mode
timeline-label-goal-id = identifiant de l’objectif
timeline-label-objective = objectif
timeline-label-completed-at = terminé le
timeline-label-run-id = identifiant de l’exécution
timeline-label-model = modèle
timeline-label-finish = fin
timeline-label-message = message
timeline-label-name = nom
timeline-label-kind = type
timeline-label-text = texte
timeline-label-plugin-id = identifiant du plugin
timeline-label-kind-label = libellé du type
timeline-label-payload = données
timeline-label-trace = trace
transcript-alert-note = Note
transcript-alert-tip = Conseil
transcript-alert-important = Important
transcript-alert-warning = Avertissement
transcript-alert-caution = Attention
transcript-code-block-empty = (vide)
transcript-diff-changes = Modifications
transcript-image-placeholder = Image
usage-view-overview = Vue d’ensemble
usage-view-daily = Quotidien
usage-view-models = Modèles
usage-view-sessions = Sessions
usage-sort-cost = Coût
usage-sort-requests = Requêtes
usage-filter-subagents-included = inclus
usage-filter-subagents-excluded = exclus
search-picker-custom-value-label = Utiliser la valeur saisie
search-picker-custom-value-detail = Appliquer exactement {$value}
search-picker-custom-path-label = Utiliser le chemin saisi

context-help-key-toggle-favorite = Ajouter ou retirer des favoris la session sélectionnée.
context-help-tip-ctrl-h = Ctrl+H ouvre toujours l'aide de l'interface affichée.
context-help-section-navigation = Navigation
context-help-section-actions = Actions
context-help-section-editing = Modification
context-help-section-workflow = Flux de travail
context-help-section-search = Recherche et sélection
context-help-section-selection = Sélection
context-help-summary-sessions = Parcourez les sessions chargées et choisissez la conversation active.
context-help-summary-transcript = Consultez, parcourez, développez, recherchez et copiez la conversation actuelle.
context-help-summary-composer = Rédigez un message, gérez les éléments joints et envoyez-le à la session active.
context-help-summary-composer-items = Consultez, ouvrez, réorganisez ou supprimez les pièces jointes et éléments collés.
context-help-summary-history = Recherchez les demandes récentes, de la plus récente à la plus ancienne, sans quitter la rédaction.
context-help-summary-suggestions = Choisissez une commande ou une mention de fichier correspondant à la recherche.
context-help-summary-editor = Modifiez une valeur, puis validez-la ou annulez.
context-help-summary-editor-multiline = Modifiez le texte structuré, puis enregistrez la valeur complète.
context-help-summary-search-picker = Recherchez parmi les choix et activez un résultat.
context-help-summary-choice-list = Choisissez une option visible sans modifier le texte recherché.
context-help-summary-timeline = Recherchez des événements de session et accédez au message correspondant.
context-help-summary-permission = Examinez la demande et ses détails, puis choisissez explicitement.
context-help-summary-details = Consultez les détails, puis revenez à la page de décision précédente.
context-help-summary-user-input = Répondez à la question et parcourez la suite de la demande.
context-help-summary-user-input-editor = Saisissez une réponse personnalisée, puis validez-la ou revenez aux choix.
context-help-summary-user-input-review = Relisez toutes les réponses avant d'envoyer la réponse complète.
context-help-summary-user-input-decision-review = Choisissez la décision finale et faites défiler séparément ses détails.
context-help-summary-confirm = Confirmez l'opération affichée ou laissez-la inchangée.
context-help-summary-usage = Consultez l'utilisation et utilisez les raccourcis pour filtrer ou actualiser le tableau de bord.
context-help-summary-model-catalog = Parcourez le catalogue de modèles avec les raccourcis de recherche, d'actualisation et de pagination.
context-help-summary-list = Parcourez les lignes visibles et activez l'élément sélectionné.
context-help-summary-panes = Passez d'un volet visible à l'autre et utilisez la ligne ou le contrôle sélectionné.
context-help-summary-action-pane = Parcourez les permissions et utilisez les raccourcis d'action sur les règles.
context-help-summary-provider = Configurez les champs, adaptateurs et modèles du fournisseur, puis enregistrez.
context-help-summary-plugin-list = Filtrez les plugins, ajustez les filtres visibles et ouvrez un plugin.
context-help-summary-plugin-detail = Passez d'un onglet de détails à l'autre et faites défiler son contenu.
context-help-summary-plugin-config = Parcourez la configuration du plugin avec les raccourcis d'action de page.
context-help-summary-plugin-actions = Choisissez une action pour la valeur de configuration sélectionnée.
context-help-summary-plugin-selection = Sélectionnez une ou plusieurs valeurs autorisées, puis validez.
context-help-summary-plugin-drilldown = Parcourez les lignes de configuration imbriquées et activez la cellule sélectionnée.
context-help-summary-plugin-diff = Examinez les modifications de configuration en attente avant de revenir.
context-help-key-move = Déplace la sélection d'un élément.
context-help-key-page = Avance ou recule d'une page visible.
context-help-key-half-page = Déplace d'une demi-page du transcript.
context-help-key-scroll-line = Fait défiler d'une ligne sans déplacer le curseur.
context-help-key-jump-history = Parcourt les positions récentes du curseur.
context-help-key-first-last = Va au premier ou au dernier élément.
context-help-key-open = Ouvre l'élément sélectionné.
context-help-key-session-scope = Choisit la portée All, Roots ou Subtree des sessions.
context-help-key-session-cycle = Change la portée des sessions.
context-help-key-transcript-vertical = Déplace d'une ligne affichée en conservant la colonne cible.
context-help-key-transcript-horizontal = Passe d'un message complet à l'autre.
context-help-key-visual-select = Démarre une sélection Visual par caractère, ligne ou bloc ; o/O change l'extrémité, gv la restaure, y la copie et Esc l'annule.
context-help-key-count = Répète le mouvement Vim suivant.
context-help-key-insert-mode = Passe en mode INSERT dans la zone de rédaction.
context-help-key-toggle = Bascule la valeur sélectionnée ou le bloc dépliable.
context-help-key-expand-all = Charge toutes les parties du transcript sans modifier leurs détails.
context-help-key-collapse-parts = Réduit cette réponse (zc) ou toutes (zC) aux 5 dernières parties et interactions en attente ; conserve les choix de détail.
context-help-key-copy = Copie une sélection Visual, un mouvement Vim, la ligne, le bloc Markdown ou le message complet.
context-help-key-search-transcript = Recherche dans le transcript et parcourt les résultats.
context-help-key-word = Avance par mot, WORD, ou au début, premier caractère non blanc ou fin de ligne.
context-help-key-find = Va à l'occurrence suivante ou précédente d'un caractère (f/F/t/T) ; répète avec ; ou ,.
context-help-key-text-object = Sélectionne un mot, paragraphe, bloc Markdown ou message complet comme objet texte.
context-help-key-mouse = Faites défiler, cliquez, double-cliquez ou glissez pour placer le curseur et sélectionner le transcript ; glissez sur l'en-tête de session, le modèle ou la zone de rédaction pour sélectionner du texte, puis copiez avec Ctrl+Y (ou y en mode VIEW).
context-help-key-line-start-end = Va au premier caractère, au premier caractère non blanc ou au dernier de la ligne affichée.
context-help-key-count-prefix = Saisissez un nombre avant un mouvement Vim pour le répéter (par exemple 5j, 3w, 10G).
context-help-key-viewport = Va en haut, au milieu ou en bas de la zone visible.
context-help-key-scroll-viewport = Fait défiler pour placer le curseur en haut, au milieu ou en bas.
context-help-key-visual-endpoints = Inverse l'extrémité active de la sélection Visual.
context-help-key-visual-restore = Restaure la dernière sélection Visual.
context-help-key-cancel-selection = Annule la sélection Visual ou la plage de texte à la souris.
context-help-key-copy-line = Copie la ligne actuelle.
context-help-key-copy-motion = Copie jusqu'à la fin de la ligne ou selon un mouvement Vim.
context-help-key-copy-text-object = Copie un mot, paragraphe, bloc Markdown ou message complet.
context-help-key-send = Envoie le message en cours de rédaction.
context-help-key-send-now = Interrompt l'exécution et envoie le message immédiatement.
context-help-key-newline = Insère un saut de ligne sans envoyer.
context-help-key-view-mode = Revient au mode VIEW du transcript.
context-help-key-editor-move = Déplace le curseur de texte par caractère, ligne ou mot.
context-help-key-editor-edit = Supprime, coupe ou restaure du texte avec les raccourcis de type shell.
context-help-key-history = Ouvre l'historique de rédaction consultable.
context-help-key-recover = Modifie le message en attente dans la zone de rédaction.
context-help-key-cancel-pending = Annule un message dont l'envoi est en cours.
context-help-key-clear-composer = Efface le texte et tous les éléments préparés.
context-help-key-items = Ouvre les éléments préparés dans la zone de rédaction.
context-help-key-commands = Ouvre ou saisit des commandes slash.
context-help-key-attach = Joint un fichier.
context-help-key-insert-content = Ouvre le panneau d'insertion de chemins locaux et de pièces jointes.
context-help-key-external-editor = Modifie le texte dans l'éditeur externe configuré.
context-help-key-image = Colle des fichiers, images ou texte depuis le presse-papiers.
context-help-key-pending-requests = Ouvre la demande de saisie ou d'autorisation en attente.
context-help-key-next = Passe à l'élément suivant.
context-help-key-previous = Passe à l'élément précédent.
context-help-key-close = Ferme cette interface.
context-help-key-older = Sélectionne une entrée d'historique plus ancienne.
context-help-key-newer = Sélectionne une entrée d'historique plus récente.
context-help-key-newer-stay = Sélectionne une entrée plus récente sans fermer l'historique à l'entrée la plus récente.
context-help-key-accept = Valide la sélection actuelle.
context-help-key-filter = Saisissez du texte pour filtrer les résultats visibles.
context-help-key-fill = Insère la suggestion sélectionnée dans la zone de rédaction.
context-help-key-edit-text = Saisissez et modifiez la valeur actuelle.
context-help-key-submit = Valide la valeur actuelle.
context-help-key-cancel = Annule sans appliquer les modifications.
context-help-key-save = Enregistre la valeur modifiée complète.
context-help-key-refresh = Actualise les données actuelles.
context-help-key-provider-add-model = Ajoute un modèle à l'adaptateur du fournisseur sélectionné.
context-help-key-provider-save-adapter = Enregistre l'adaptateur du fournisseur sélectionné.
context-help-key-usage-period = Change la période du rapport.
context-help-key-usage-view = Change la vue du tableau de bord.
context-help-key-usage-provider = Change le filtre de fournisseur.
context-help-key-usage-model = Change le filtre de modèle.
context-help-key-usage-subagents = Inclut ou exclut les sous-agents.
context-help-key-usage-sort = Change l'ordre de tri.
context-help-key-model-catalog-search = Modifie la recherche dans le catalogue de modèles.
context-help-key-model-catalog-page = Charge la page précédente ou suivante du catalogue.
context-help-key-plugin-transport = Change le filtre de transport.
context-help-key-plugin-config-filter = Change le filtre d'état de configuration.
context-help-key-plugin-validate = Valide la configuration actuelle du plugin.
context-help-key-plugin-reset = Rétablit les valeurs par défaut de la configuration du plugin.
context-help-key-plugin-diff = Affiche ou masque les modifications de configuration en attente.
context-help-key-permission-add = Ajoute une règle à la section de permissions actuelle.
context-help-key-permission-rename = Renomme la règle sélectionnée.
context-help-key-permission-browse = Parcourt le champ de chemin sélectionné.
context-help-key-activate = Active la ligne, le contrôle ou l'action sélectionné.
context-help-key-jump-message = Va au message associé à l'événement.
context-help-key-back = Revient au niveau précédent.
context-help-key-next-question = Passe à la question suivante ou précédente, ou à la page de révision.
context-help-key-custom-answer = Modifie une réponse personnalisée.
context-help-key-clear = Efface la dérogation ou la réponse sélectionnée.
context-help-key-cancel-request = Ferme ou annule toute la demande.
context-help-key-edit-answer = Revient à la modification de la réponse sélectionnée.
context-help-key-scroll-body = Fait défiler séparément le contenu de la révision.
context-help-key-submit-all = Envoie toutes les réponses relues.
context-help-key-confirm = Confirme l'opération affichée.
context-help-key-focus-next = Parcourt les volets et contrôles visibles dans les deux sens.
context-help-key-pane-horizontal = Passe directement au volet gauche ou droit.
context-help-key-horizontal = Parcourt les colonnes, cellules ou actions visibles.
context-help-key-next-tab = Passe d'un onglet de détails à l'autre.
context-help-key-scroll = Fait défiler le contenu des détails.
context-help-summary-session-work = Fichiers, tâches en arrière-plan et état des nouvelles tentatives de cette session.
context-help-key-work-toggle = Développe ou réduit le panneau de l'espace de travail.
context-help-key-work-tabs = Passe entre fichiers / tâches / état.
context-help-key-work-staged = Passe entre modifications préparées / en cours.
context-help-key-work-more = Affiche la suite du diff.
context-help-key-work-control = Exécute l'action disponible pour la tâche sélectionnée.
context-help-key-work-leave = Revient à la zone de rédaction.
context-help-key-answer-next = Sélectionne la réponse et passe à la page suivante.
context-help-key-summary-action = Modifie une réponse, revient en arrière, envoie ou annule l'action de la ligne récapitulative sélectionnée.

overlay-snapshot-title = Instantané
overlay-rename-title = Renommer la session
overlay-rename-prompt = Modifiez le titre de la session
overlay-choice-footer = Entrée sélectionner · Échap fermer
overlay-choice-footer-select = Entrée sélectionner · Échap fermer
overlay-help-footer = Flèches faire défiler · Échap fermer
overlay-attach-image-title = Joindre une image
overlay-attach-image-prompt = Saisissez le chemin d’une image ou un terme de recherche. Entrée joint l’image sélectionnée.
overlay-attach-image-no-match = Aucune image correspondante
overlay-attach-browser-prompt = Parcourez le chemin actuel. Saisissez un nom de fichier ou de dossier pour filtrer, ou modifiez le chemin pour changer de répertoire.
overlay-attach-image-browser-prompt = Parcourez les images du chemin actuel. Saisissez le nom d’une image ou d’un dossier pour filtrer, ou modifiez le chemin pour changer de répertoire.
overlay-attach-browser-footer = ↓ sélectionner · Droite ouvrir le dossier · Gauche dossier parent · Entrée joindre · Échap fermer
overlay-path-browser-list-title = Explorateur
overlay-plan-approval-title = Approbation du plan
overlay-plan-approval-summary = Examinez le plan actif avant la reprise de l’implémentation.
overlay-plan-approval-footer = a approuver une fois | s approuver la session | d refuser
overlay-plan-slug = plan={$slug}
overlay-plan-path = chemin : {$path}
overlay-plan-steps = étapes={$steps}
overlay-plan-preview-heading = Aperçu
overlay-permission-request-id = request_id : {$request_id}
overlay-permission-reason = motif : {$reason}
overlay-permission-explanation = explication : {$explanation}
overlay-permission-session = session : #{$session}
overlay-permission-requested-actions = Actions demandées
overlay-permission-related-actions = Actions associées
overlay-permission-fact-source = source={$value}
overlay-permission-fact-scope = portée={$value}
overlay-permission-fact-operator = opérateur={$value}
overlay-permission-footer = a autoriser une fois | s autoriser la session | d refuser
overlay-permission-footer-edit-rule = {$footer} | e modifier la règle
overlay-permission-footer-action = ↑/↓ sélectionner · Entrée activer · Échap fermer
overlay-permission-footer-scope = ↑/↓ sélectionner · Entrée activer · Échap revenir
overlay-permission-footer-details = Échap revenir
overlay-permission-decision-heading = Choisissez comment traiter cette demande
overlay-permission-scope-heading = Appliquer cette décision à
overlay-permission-choice-allow = Autoriser…
overlay-permission-choice-deny = Refuser…
overlay-permission-choice-edit-rule = Créer une règle précise…
overlay-permission-choice-auto-approve-busy = Approbation automatique…
overlay-permission-choice-once = Cette fois uniquement
overlay-permission-choice-session = Session actuelle
overlay-permission-choice-workspace = Espace de travail actuel
overlay-permission-choice-global = Global
overlay-permission-choice-allow-always-session = Toujours autoriser (session)
overlay-permission-choice-allow-always-workspace = Toujours autoriser (espace de travail)
overlay-permission-choice-allow-always-global = Toujours autoriser (global)
overlay-permission-choice-deny-always-session = Toujours refuser (session)
overlay-permission-choice-deny-always-workspace = Toujours refuser (espace de travail)
overlay-permission-choice-deny-always-global = Toujours refuser (global)
overlay-permission-rule-fields = Champs de la règle
overlay-permission-rule-empty-detail = Sélectionnez un champ d’autorisation à modifier.
overlay-permission-rule-item-browse-target-path = Parcourir le chemin cible
overlay-permission-rule-item-browse-target-path-detail = Choisissez un fichier ou dossier dans l’espace de travail ou le système de fichiers.
overlay-permission-rule-item-browse-workspace-root = Parcourir la racine de l’espace de travail
overlay-permission-rule-item-browse-workspace-root-detail = Choisissez un répertoire pour workspace_root.
overlay-permission-rule-item-browser-value = ouvrir l’explorateur
overlay-permission-rule-item-save = Enregistrer la règle
overlay-permission-rule-item-save-detail = Valider et enregistrer le brouillon actuel.
overlay-permission-rule-item-revoke = Révoquer la règle
overlay-permission-rule-item-revoke-detail = Révoquer la règle existante enregistrée.
overlay-permission-rule-detail-browse-workspace-root = Ouvrir l’explorateur de répertoires et choisir un dossier pour workspace_root.
overlay-permission-rule-detail-browse-target-path = Ouvrir l’explorateur de fichiers et choisir un fichier ou dossier pour préremplir target_path.
overlay-permission-rule-detail-revoke = La révocation rend la règle enregistrée inactive sans supprimer son historique.
overlay-permission-rule-preview-heading = Aperçu
overlay-permission-rule-preview-label = libellé : {$label}
overlay-permission-rule-preview-mode = mode : {$mode}
overlay-permission-rule-preview-scope = portée : {$scope}
overlay-permission-rule-preview-subject-tool = sujet : outil ({$tool})
overlay-permission-rule-preview-subject-path = sujet : path_access ({$access})
overlay-permission-rule-preview-subject-network = sujet : network_access
overlay-permission-rule-preview-qualifier = qualificatif : {$qualifier}
overlay-permission-rule-preview-target = cible : {$target}
overlay-permission-rule-preview-workspace-root = workspace_root : {$path}
overlay-permission-rule-preview-session-id = session_id : {$session}
overlay-permission-rule-preview-invalid = règle invalide : {$error}
overlay-workbench-overview = Vue d’ensemble
overlay-permission-studio-footer = Gauche/Droite ou Tab/Maj+Tab changer de volet · Flèches sélectionner · Entrée modifier · Échap revenir
overlay-permission-studio-sections = Sections d’autorisations
overlay-permission-studio-nav = Navigation
overlay-detail-current-value = Valeur actuelle
overlay-choice-current-unavailable-detail = Valeur actuelle, absente des options disponibles
overlay-choice-clear-runtime-detail = Supprimer la dérogation de session actuelle pour {$field}.
overlay-choice-clear-provider-detail = Définir {$field} comme valeur de brouillon vide.
overlay-choice-clear-permission-subject = Réinitialiser le type de sujet à la règle d’outil par défaut.
overlay-choice-clear-permission-access-kind = Effacer la dérogation du type d’accès au chemin.
overlay-choice-clear-permission-scope = Réinitialiser la portée de la règle à l’espace de travail.
overlay-choice-clear-permission-mode = Réinitialiser le mode de la règle à « demander ».
overlay-choice-clear-permission-override-detail = Supprimer la dérogation actuelle pour {$field}.
overlay-client-versions-title = Versions des clients fournisseur
overlay-insert-content-title = Insérer du contenu
overlay-insert-content-prompt = Choisissez le contenu à insérer à l’emplacement du curseur dans la zone de rédaction
overlay-resume-title = Reprendre une session
overlay-resume-prompt = Saisissez du texte pour filtrer les sessions par titre, identifiant ou filiation
overlay-resume-empty = Aucune session correspondante
overlay-session-search-scope-all = toutes
overlay-session-search-scope-roots = racines
overlay-session-search-scope-subtree = sous-arbre
overlay-session-search-tail-more = suite
overlay-session-search-tail-end = fin
overlay-session-search-footer-local = portée {$scope} · {$total} résultats
overlay-session-search-footer-remote = portée {$scope} · {$loaded} chargés · {$tail}
overlay-lineage-empty = Aucune session de branche associée trouvée
overlay-rewind-empty = Aucun message utilisateur terminé ne peut être retiré
overlay-rewind-confirm-title = Confirmer le retour en arrière
overlay-rewind-confirm-keep = Retirer {$target} et tous les messages suivants
overlay-rewind-confirm-warning = Une nouvelle branche ne conservera que l’historique antérieur à ce message utilisateur.
overlay-rewind-confirm-draft = Le message utilisateur retiré sera restauré dans la zone de rédaction de la nouvelle branche ; le brouillon actuel restera dans la session d’origine.
overlay-confirm-footer = Y confirmer
overlay-snapshot-remove-title = Confirmer la suppression de l’instantané
overlay-snapshot-remove-body = Supprimer l’instantané actif et quitter la session actuelle ?
overlay-snapshot-remove-force = La suppression forcée abandonnera les modifications non validées de l’instantané.
overlay-model-providers-title = Fournisseurs de modèles
overlay-model-providers-prompt = Choisissez un fournisseur avant de sélectionner un modèle
overlay-models-title = Modèles [{$provider}]
overlay-models-prompt = Choisissez un modèle pour le fournisseur {$provider}
overlay-session-model-title = Modèle de la session
overlay-session-model-prompt = Rechercher des modèles
overlay-session-model-list-title = Modèles
overlay-session-model-footer = Saisissez du texte pour filtrer
overlay-model-catalog-title = Catalogue de modèles
overlay-model-catalog-footer = Ctrl+F rechercher · Ctrl+R actualiser · Gauche/Droite page précédente/suivante · Haut/Bas sélectionner · Échap fermer
overlay-model-catalog-search-title = Recherche dans le catalogue de modèles
overlay-model-catalog-search-prompt = Filtrer le catalogue de modèles par identifiant, nom affiché, origine ou description
overlay-model-catalog-entries = Modèles
overlay-model-catalog-detail = Détail
overlay-model-catalog-query-all = <tous>
overlay-model-catalog-summary = requête {$query} · page {$start}-{$end} / {$total} · {$count} modèles
overlay-permission-rule-overview-draft = Brouillon
overlay-permission-rule-overview-mode = Mode
overlay-permission-rule-overview-scope = Portée
overlay-permission-rule-overview-session = Session
overlay-permission-rule-overview-workspace-root = Racine de l’espace de travail
overlay-children-title = Sessions enfants [#{$session}]
overlay-children-prompt = Choisissez une session enfant à ouvrir
overlay-children-empty = Aucune session enfant trouvée
overlay-timeline-title = Chronologie [#{$session}]
overlay-timeline-prompt = Filtrer par type d’événement, commande ou métadonnées de message
overlay-timeline-empty = Aucun événement correspondant
overlay-timeline-footer = Saisir pour filtrer · Flèches sélectionner · Entrée ouvrir le message associé · Échap fermer
overlay-timeline-events = Événements
overlay-timeline-detail = Détail

session-status-plan = plan {$slug}
session-status-plan-with-steps = plan {$slug} · {$steps} étapes
session-status-thinking = {$value}
session-status-speed = {$value}
flash-large-paste-no-file-view = les extraits d’un grand collage peuvent être supprimés, mais ne disposent pas d’aperçu de fichier
flash-quit-confirm = Appuyez de nouveau sur Ctrl+C pour quitter
flash-run-cancelling-quit = Annulation de l’exécution en cours ; appuyez de nouveau sur Ctrl+C pour quitter
flash-config-dir-prepare-failed = Impossible de préparer le répertoire de configuration {$path} : {$error}
flash-config-file-create-failed = Impossible de créer le fichier de configuration {$path} : {$error}
flash-clipboard-paste-failed = Échec du collage depuis le presse-papiers : {$error}
flash-clipboard-attachments-truncated = Le presse-papiers contient trop de ressources pour ce message ; les {$count} premières ont été jointes
flash-attachment-count-limit = Un message peut joindre au maximum {$count} ressources
flash-attach-images-only = Le sélecteur d’images ne peut joindre que des fichiers image.
flash-composer-drafts-load-failed = Impossible de charger les brouillons de rédaction : {$error}
flash-composer-drafts-save-failed = Impossible d’enregistrer les brouillons de rédaction : {$error}
flash-prompt-history-load-failed = Impossible de charger l’historique des demandes : {$error}
flash-prompt-history-save-failed = Impossible d’enregistrer l’historique des demandes : {$error}
flash-copied-mouse-selection = Sélection à la souris copiée dans le presse-papiers
flash-transcript-line-copied = Ligne sélectionnée du transcript copiée dans le presse-papiers
flash-transcript-no-selection = Sélectionnez d’abord du contenu visible du transcript par un clic ou une touche de navigation
flash-transcript-exported = Transcript exporté vers {$path}
flash-transcript-export-failed = Échec de l’exportation du transcript : {$error}
flash-transcript-pager-failed = Échec de l’affichage paginé du transcript : {$error}
flash-timeline-event-copied = Événement sélectionné de la chronologie copié
flash-transcript-node-expanded = {$kind} développé
flash-transcript-node-collapsed = {$kind} réduit
flash-transcript-parts-revealed = Chargement de {$count} parties supplémentaires au maximum
flash-transcript-node-copied = {$kind} actuel copié
flash-plan-viewer-requires-session = L’afficheur de plan nécessite une session ouverte
flash-plan-viewer-no-plan = Aucun plan n’est chargé pour cette session
flash-snapshot-ready = Instantané prêt : {$path} ({$branch})
flash-snapshot-attached = Instantané rattaché : {$path} ({$branch})
flash-snapshot-exited = Instantané {$action} : {$path}
flash-commit-created = Commit créé : {$sha} {$summary}
flash-pr-created = Demande de fusion créée : {$url}
flash-attachment-inspect-failed = Impossible d’examiner la pièce jointe {$path} : {$error}
flash-session-awaiting-approval = La session attend une autorisation
flash-session-awaiting-plan-approval = La session attend l’approbation du plan
flash-session-awaiting-user-input = La session attend une réponse
flash-no-permission-request = Aucune demande d’autorisation en attente
flash-no-user-input-request = Aucune demande de saisie utilisateur en attente
flash-plugins-enabled = Plugins activés
flash-plugins-disabled = Plugins désactivés
flash-plugin-config-not-object = La configuration du plugin {$plugin_id} n’est pas un objet JSON et ne peut pas être réécrite
flash-plugin-config-enabled = Plugin {$plugin_id} activé ; configuration conservée et environnement d’exécution rechargé
flash-plugin-config-disabled = Plugin {$plugin_id} désactivé ; configuration conservée et environnement d’exécution rechargé
flash-tool-description-mode-set = Mode de description des outils défini sur {$mode}
flash-plugin-ui-display-mode-set = Mode d’affichage de l’interface des plugins défini sur {$mode}
flash-model-selected = Modèle sélectionné : {$model}
flash-model-cleared = Remplacement du modèle supprimé
flash-model-provider-required = Un fournisseur est requis ; sélectionnez-en un ou utilisez fournisseur/modèle
flash-provider-studio-catalog-template-applied = Champs du modèle remplis à partir de {$model} ; enregistrez pour conserver ces valeurs
flash-thinking-mode-set = Mode de réflexion défini sur {$mode}
flash-thinking-mode-cleared = Remplacement du mode de réflexion supprimé
flash-thinking-mode-required = Le sélecteur du mode de réflexion est requis
flash-speed-mode-set = Mode de vitesse défini sur {$mode}
flash-speed-mode-cleared = Remplacement du mode de vitesse supprimé
flash-speed-mode-required = Le nom du mode de vitesse est requis
flash-parent-session-missing = La session actuelle n’a pas de parent
flash-command-usage = Utilisation : {$usage}
flash-command-does-not-support-attachments = Les commandes slash ne prennent pas en charge les pièces jointes en attente
flash-send-in-flight = Le message précédent est en cours d’envoi ; le nouveau message reste dans la zone de rédaction
flash-answer-pending-request = Répondez d’abord à la demande en attente ; le message reste dans la zone de rédaction
flash-message-queued = Message en attente — il sera envoyé à la fin de l’exécution en cours
flash-message-replaced = Un message était déjà en attente — il a été remplacé
flash-steer-sent = Instruction transmise à l’exécution en cours
flash-steer-failed-fallback-queue = Impossible d’orienter l’exécution en cours ; le message a été conservé
flash-run-cancelled = Exécution annulée
flash-cancel-failed = Échec de l’annulation
flash-user-command-empty = Le contenu de la commande utilisateur est vide
flash-side-requires-session = Ouvrez une session avant de démarrer une conversation parallèle
flash-side-already-open = Une conversation parallèle est déjà ouverte
flash-memory-forgotten = Mémoire supprimée : {$name}
flash-no-pending-message = Aucun message en attente
flash-pending-cancelled = Message en attente annulé
flash-command-prefill-requires-empty-composer = Videz la zone de rédaction avant de préremplir une commande
flash-temperature-set = Température définie sur {$value}
flash-temperature-cleared = Remplacement de la température supprimé
flash-temperature-invalid = Valeur de température invalide
flash-verbosity-set = Niveau de verbosité défini sur {$value}
flash-verbosity-cleared = Remplacement du niveau de verbosité supprimé
flash-parallel-tool-calls-set = Nombre d’appels parallèles d’outils défini sur {$value}
flash-parallel-tool-calls-cleared = Remplacement du nombre d’appels parallèles d’outils supprimé
flash-max-output-set = Nombre maximal de jetons de sortie défini sur {$value}
flash-max-output-cleared = Remplacement du nombre maximal de jetons supprimé
flash-max-output-invalid = Nombre maximal de jetons de sortie invalide
flash-system-set = Remplacement de l’invite système mis à jour
flash-system-cleared = Remplacement de l’invite système supprimé
flash-runtime-status = État de la session : {$summary}
flash-diagnostics-summary = Diagnostic : cwd={$cwd} session={$session} queue={$queue} runtime={$runtime}
flash-session-permission-updated = Autorisations de session mises à jour.
session-work-tasks = Tâches
session-work-files = Fichiers
session-work-requests = Demandes
session-work-waiting = En attente des outils
session-work-status = État d’exécution
session-work-workspace = Modifications consignées de la session · chemins littéraux des outils
session-work-ready = Prêt
session-work-back = Retour
session-work-switch = Basculer →
session-work-staged = Préparé ⇄
session-work-working = Arbre de travail ⇄
session-work-stop = Arrêter
session-work-pause = Mettre en pause
session-work-resume = Reprendre
session-work-delete = Supprimer
session-work-dismiss = Ignorer
session-work-log-tail = 200 dernières lignes / 128 Kio
session-work-empty = Aucun contenu dans cette vue
session-work-footer = Retour à la saisie
session-work-more-diff = Charger davantage de différences
session-work-status-pending = En file d’attente
session-work-status-running = En cours
session-work-status-waiting = En attente
session-work-status-paused = En pause
session-work-status-succeeded = Terminé
session-work-status-failed = Échec
session-work-status-cancelled = Annulé
session-work-status-stopped = Arrêté
session-work-next-wake = Prochain réveil :
session-work-title = Session
session-work-loading = Chargement des fichiers
session-work-previous = ‹ Précédent
session-work-next = Suivant ›
session-work-recording-incomplete = L’enregistrement peut être incomplet (les écritures du shell sont des déclarations)
session-work-operation-history = Historique des opérations, pas un diff global de la session (aucune base complète).
session-work-operation-diff = Diff complet de l’opération de patch (peut inclure d’autres fichiers).
session-work-recorded-truncated = Diff tronqué ; seul le contenu enregistré peut être chargé.
session-work-no-recorded-diff = Aucun diff textuel enregistré.

workbench-action-add-field = Ajouter un champ
workbench-action-add-field-desc = Ajouter un champ dans { $title }.
workbench-action-add-item = Ajouter un élément
workbench-action-add-item-desc = Ajouter un nouvel élément par défaut à la fin de { $title }.
workbench-action-duplicate-item = Dupliquer l’élément
workbench-action-duplicate-item-desc = Dupliquer { $title } dans ce tableau.
workbench-action-field = Actions sur le champ
workbench-action-insert-after = Insérer après
workbench-action-insert-after-desc = Insérer un nouvel élément par défaut après cet élément du tableau.
workbench-action-insert-before = Insérer avant
workbench-action-insert-before-desc = Insérer un nouvel élément par défaut avant cet élément du tableau.
workbench-action-more = Autres actions
workbench-action-move-down = Descendre
workbench-action-move-down-desc = Déplacer cet élément du tableau d’une position vers le bas.
workbench-action-move-up = Monter
workbench-action-move-up-desc = Déplacer cet élément du tableau d’une position vers le haut.
workbench-action-remove-item = Supprimer l’élément
workbench-action-remove-item-desc = Supprimer { $title } de ce tableau.
workbench-action-rename-field = Renommer le champ
workbench-action-rename-field-desc = Renommer la clé de { $title }.
workbench-action-reset-field = Réinitialiser le champ
workbench-action-reset-field-desc = Rétablir la valeur par défaut du plugin pour { $title }.
workbench-action-reset-group = Réinitialiser le groupe
workbench-action-reset-group-desc = Rétablir les valeurs par défaut du plugin pour tous les champs de { $title }.
workbench-action-switch-shape-desc = Changer la forme active de { $title }.
workbench-action-switch-type-desc = Changer le type actif de { $title }.
workbench-editor-placeholder-type-to-edit = Saisir pour modifier
workbench-editor-prompt-add-field = Saisissez un nom de champ pour { $path }. Si le schéma autorise plusieurs types ou formes, l’éditeur vous demandera de choisir après la création.
workbench-editor-prompt-new-field-name = Saisissez le nouveau nom du champ.
workbench-editor-title-add-field = Ajouter un champ
workbench-editor-title-edit = Modifier { $title }
workbench-editor-title-edit-with-label = Modifier { $title } · { $label }
workbench-editor-title-rename = Renommer { $title }
workbench-error-append-array-item = impossible d’ajouter l’élément au tableau
workbench-error-args-invalid-json = Les arguments de l’outil ne sont pas au format JSON valide.
workbench-error-args-not-object = Les arguments de l’outil du plugin doivent être un objet JSON.
workbench-error-args-schema-mismatch = Les arguments de l’outil ne correspondent pas au schéma du plugin.
workbench-error-config-update = Impossible de mettre à jour la configuration du plugin.
workbench-error-tool-input-prepare = Impossible de préparer les entrées de l’outil du plugin. Consultez le journal de diagnostic et réessayez.
workbench-error-tool-requires-session = L’outil du plugin nécessite une session active.
workbench-error-tool-unavailable = L’outil du plugin n’est plus disponible.
workbench-picker-choose-many = Choisissez une ou plusieurs valeurs
workbench-picker-choose-one = Choisissez une valeur
workbench-picker-no-actions = Aucune action disponible
workbench-picker-no-choices = Aucun choix disponible
workbench-picker-space-toggle = Espace : basculer
workbench-picker-title-select = Sélectionner { $title }
workbench-select-branch-prompt = Choisissez la forme du schéma
workbench-select-branch-title = Sélectionner une branche
workbench-select-type-prompt = Choisissez le type JSON pour { $path }
workbench-select-type-title = Sélectionner un type
workbench-tool-completed = outil du plugin exécuté
workbench-tool-completed-no-output = L’outil a été exécuté avec succès sans produire de résultat.
workbench-tool-default-run = Exécuter cet outil du plugin.
workbench-tool-editor-description = { $summary } Étiquettes : { $tags }. Modifiez les arguments JSON ci-dessous. La validation autorise une seule fois cet appel précis ; les règles de refus enregistrées restent applicables.
workbench-tool-editor-placeholder = Ctrl+S valider et exécuter · Échap annuler · Entrée nouvelle ligne
workbench-tool-editor-title = Exécuter { $plugin } / { $tool }
workbench-tool-tags-none = aucune déclarée
workbench-warning-add-requires-object-or-array = l’ajout est possible sur les nœuds objet et tableau
workbench-warning-cannot-add-item = impossible d’ajouter un autre élément à cette position du tableau
workbench-warning-cannot-duplicate-item = impossible de dupliquer cet élément du tableau
workbench-warning-cannot-insert-item = impossible d’insérer un élément à cette position du tableau
workbench-warning-cannot-remove-item = impossible de supprimer cet élément du tableau
workbench-warning-no-tools = ce plugin ne propose aucun outil
workbench-warning-root-config-not-deletable = la configuration racine ne peut pas être supprimée
workbench-warning-row-not-object-field = la ligne sélectionnée ne correspond pas à un champ d’objet
value-built-in = intégré
value-built-in-defaults = valeurs intégrées par défaut
value-char-count = {$count} caractère(s)
value-config-backed = géré par la configuration
value-config-owned = détenu par la configuration
value-default-true = par défaut (vrai)
value-delete = Supprimer
value-disabled = désactivé
value-duplicate = Dupliquer
value-edit = Modifier
value-editable = modifiable
value-enabled = activé
value-file-backed = stocké dans un fichier
value-hidden = masqué
value-inactive = inactif
value-inherit = hériter
value-inherit-off = héritage désactivé : {$sections}
value-inherits-runtime-defaults = hérite des valeurs par défaut de l’exécution
value-item-count = {$count} élément(s)
value-markdown-backed = stocké en Markdown
value-network-summary = réseau : {$detail}
value-off = désactivé
value-on = activé
value-path-summary = chemin : {$detail}
value-plugin-override-count = {$count} remplacement(s) de plugin
value-plugin-tools = outils du plugin
value-read-only = en lecture seule
value-rename = Renommer
value-reset = Réinitialiser
value-runtime-config = configuration d’exécution
value-runtime-config-file = configuration d’exécution (agena.json)
value-runtime-registered = enregistré à l’exécution
value-selected = sélectionné
value-tools = outils
value-tools-summary = outils : {$detail}
value-unknown = inconnu
value-unrestricted = sans restriction
value-visible = visible

provider-field-auth-status = État de l’authentification
provider-field-api-key-env = Variable d’environnement de la clé API
provider-field-api-key = Clé API
provider-field-username = Nom d’utilisateur
provider-field-display-name = Nom affiché
provider-field-email = Adresse e-mail
provider-field-avatar-url = URL de l’avatar
provider-model-field-catalog-template = Modèle du catalogue
provider-model-catalog-selection-note = Appuyez sur Entrée pour choisir un modèle. L’enregistrement applique ses champs, pas son identifiant de catalogue.
provider-issuer-atomgit-label = atomgit
provider-issuer-atomgit-detail = Identifiants AtomGit
provider-adapter-rule-none-ollama-detail = Adaptateur Ollama ; les champs d’authentification restent vides et le point de terminaison se configure sur l’adaptateur.
provider-adapter-rule-api-openai-detail = Adaptateur OpenAI Responses ou Chat Completions explicite ; utilise base_url pour l’authentification du fournisseur et permet de répertorier les modèles en brouillon.
provider-adapter-rule-api-anthropic-detail = Adaptateur HTTP Anthropic ; utilise base_url pour l’authentification du fournisseur et permet de répertorier les modèles en brouillon.
provider-adapter-rule-api-gemini-detail = Adaptateur HTTP Gemini ; utilise base_url pour l’authentification du fournisseur et permet de répertorier les modèles en brouillon.
provider-adapter-rule-cline-api-openai-detail = API Cline via l’adaptateur openai_chat_completions ; base_url et la découverte des modèles restent fixés au point de terminaison hébergé de Cline.
provider-adapter-rule-gitlab-auth-openai-detail = Passerelle GitLab acheminée par l’adaptateur OpenAI Responses ou Chat Completions sélectionné.
provider-adapter-rule-gitlab-auth-anthropic-detail = Passerelle GitLab acheminée par l’adaptateur anthropic.
provider-adapter-rule-openai-chatgpt-openai-detail = Adaptateur openai_responses avec le backend officiel ChatGPT Codex ; les identifiants proviennent de la session OpenAI locale.
provider-adapter-rule-github-copilot-openai-detail = Jeton Copilot acheminé par l’adaptateur OpenAI Responses ou Chat Completions sélectionné.
provider-adapter-rule-github-copilot-anthropic-detail = Jeton Copilot acheminé par l’adaptateur anthropic.
provider-adapter-rule-gitlab-credential-openai-detail = Identifiant OAuth GitLab acheminé par l’adaptateur OpenAI Responses ou Chat Completions sélectionné.
provider-adapter-rule-gitlab-credential-anthropic-detail = Identifiant OAuth GitLab acheminé par l’adaptateur anthropic.
provider-adapter-rule-google-adc-openai-detail = Adaptateur openai_chat_completions de type Vertex avec capability_family=gemini ; nécessite base_url pour l’authentification du fournisseur.
provider-adapter-rule-sap-ai-core-openai-detail = Adaptateur openai_chat_completions de SAP AI Core ; nécessite base_url pour l’authentification du fournisseur et service_key_env.
provider-adapter-rule-bedrock-sigv4-amazon-bedrock-detail = Adaptateur Amazon Bedrock signé avec AWS SigV4.
provider-redirect-local-studio-detail = URL de rappel localhost correspondant à la route Web de Studio
provider-profile-choice-detail = Profil de configuration partagée AWS
provider-profile-default-detail = Profil AWS par défaut
provider-api-key-env-openai-detail = Variable d’environnement de clé API compatible avec OpenAI
provider-api-key-env-anthropic-detail = Variable d’environnement de clé API Anthropic
provider-api-key-env-gemini-detail = Variable d’environnement de clé API Gemini
provider-api-key-env-gitlab-detail = Variable d’environnement du jeton GitLab
provider-api-key-env-vertex-detail = Variable d’environnement du jeton d’accès Google Vertex
provider-api-key-env-shared-gateway-detail = Variable d’environnement de la clé API de la passerelle partagée
provider-api-key-env-opencode-detail = Variable d’environnement de la clé API Opencode

transcript-footer-pending = envoi après l’arrêt de l’exécution en cours
transcript-footer-pending-preview = prochain envoi : {$preview}
transcript-footer-plugin-block = {$label} : {$body}
transcript-export-path-is-directory = le chemin d’exportation est un répertoire : {$path}
transcript-export-title = Exportation du transcript Agena
transcript-export-session-id = - Identifiant de session : #{$id}
transcript-export-exported-at = - Exporté le : {$time}
transcript-export-messages-loaded = - Messages chargés : {$count}
transcript-export-older-messages-omitted = - Anciens messages omis : {$value}
transcript-export-parent-session = - Session parente : #{$id}
transcript-export-child-sessions = - Sessions enfants : {$count}
transcript-export-empty = _Aucun message chargé dans cette session._
transcript-node-kind-message = message
transcript-node-kind-markdown = bloc Markdown
transcript-node-kind-heading = titre
transcript-node-kind-quote = citation
transcript-node-kind-alert = alerte
transcript-node-kind-code = bloc de code
transcript-node-kind-list = liste
transcript-node-kind-table = tableau
transcript-node-kind-math = formule
transcript-node-kind-image = image
transcript-node-kind-footnote = note de bas de page
transcript-node-kind-diagram = diagramme
transcript-node-kind-activity = blocs d’activité
transcript-footer-side-active = conversation parallèle · revenir avec /parent ou la navigation
transcript-footer-side-return = conversation parallèle ouverte · revenir en basculant
transcript-history-more = ↑ Charger les messages précédents · faire défiler vers le haut ou cliquer ici
transcript-history-loading = … Chargement des messages précédents
transcript-history-start = Début de la conversation
transcript-history-error = ↑ Réessayer de charger l’historique · { $error }
transcript-history-stalled = Le curseur de l’historique n’a pas avancé. Réessayez le chargement.
transcript-fold-loading = … Chargement des parties précédentes de la réponse
transcript-fold-error = Échec du chargement · Entrée pour réessayer · { $error }

hub-action-favorite = Ajouter aux favoris
hub-action-unfavorite = Retirer des favoris
hub-section-favorites = Favoris
composer-status-pending-user-input = {$count} saisie(s) en attente (Ctrl+R)
composer-status-pending-approval = {$count} autorisation(s) en attente (Ctrl+L)
composer-status-hidden-pending-dialog = boîte de dialogue en attente masquée pendant la saisie
composer-status-selected-item = élément {$current}/{$total} {$label}
composer-status-history = historique {$current}/{$total}
composer-status-history-query = historique {$current}/{$total} recherche={$query}
composer-status-mention = mention {$query}
composer-status-slash = commande slash {$query}
surface-mode-insert = INSERT
surface-mode-navigate = NAV
surface-mode-select = SELECT
runtime-status-keys = touches q={$queue} envoi={$send}
runtime-status-display = affichage {$value}
runtime-status-display-default = par défaut
runtime-status-display-plugin = plugin
runtime-status-tui-blocks = tui_blocks {$count}
runtime-status-theme = thème {$value}
run-options-summary-thinking = réflexion {$value}
run-options-summary-speed = vitesse {$value}
run-options-summary-verbosity = verbosité {$value}
run-options-summary-parallel-tools = outils-parallèles {$value}
help-sessions-line-4 = /rename, /timeline et /lineage gèrent la session sélectionnée
help-sessions-line-5 = Utilisez les commandes du sélecteur pour choisir la portée All, Roots ou Subtree
help-transcript-line-8 = Utilisez /rename, /timeline, /lineage, /settings, /export ou /pager pour gérer la session et sa configuration
help-composer-line-11 = Ctrl+G ouvre les éléments intégrés si disponibles ; les commandes slash comme /model ou /rewind s’exécutent depuis la zone de rédaction
help-section-user-commands = Commandes utilisateur
plan-viewer-refresh = Actualiser
plan-viewer-title = Plan
plan-inline-footer = Tab/Esc saisie · j/k défiler · r actualiser · f développer
plan-viewer-empty = Aucun plan pour le moment.
plan-viewer-loading = Chargement du plan
plan-viewer-footer = ↑/↓/PgUp/PgDn défiler · r actualiser · a exécution automatique · q fermer
plan-viewer-autorun-on = exécution automatique : activée
plan-viewer-autorun-off = exécution automatique : désactivée
runtime-setting-choice-parallel-detail = appliquer une dérogation au nombre d’appels d’outils parallèles
runtime-setting-summary-override-value = dérogation {$value}
runtime-setting-apply-cleared = dérogation de {$field} supprimée
runtime-setting-apply-updated = {$field} mis à jour
runtime-setting-error-bool = {$field} attend true/false, on/off, yes/no ou 1/0
runtime-setting-error-number = {$field} attend une valeur numérique
runtime-setting-error-finite = {$field} attend un nombre fini
runtime-setting-error-positive-int = {$field} attend un entier positif
composer-prompt-history-label = historique>
composer-prompt-history-no-matches =   aucun élément ne correspond à l’historique des demandes
insert-content-file-label = Fichier
insert-content-file-detail = Joindre un fichier ou un dossier de l’espace de travail.
status-part-workspace = espace={$value}
status-part-state = état={$value}
status-part-agent = agent={$value}
status-part-task = tâche={$value}
status-part-model = modèle={$value}
status-part-thinking = réflexion={$value}
status-part-speed = vitesse={$value}
status-part-verbosity = verbosité={$value}
status-part-parallel-tools = parallel_tools={$value}
status-part-cwd = cwd={$value}
status-part-tools = outils={$count}
status-part-permission = autorisation={$value}
status-part-permissions = autorisations={$count}
status-part-user-input = saisies={$count}
inline-fact-provider = fournisseur
inline-fact-adapter = adaptateur
inline-fact-model = modèle
inline-fact-thinking = réflexion
inline-fact-speed = vitesse
inline-fact-verbosity = verbosité
inline-fact-parallel-tools = parallel_tools
permission-rule-error-session-token-required = la portée session nécessite session=<id>
permission-rule-error-invalid-shell-args = arguments au format shell invalides
permission-rule-error-expected-structured = règle structurée attendue, commençant par tool/path et se terminant par allow|ask|deny
permission-rule-error-unknown-option = option de règle d’autorisation inconnue : {$key}
permission-rule-error-invalid-subject = le sujet de la règle doit commencer par `tool`, `path` ou `network`
permission-rule-error-invalid-mode = le mode d’autorisation doit être allow, ask ou deny
permission-rule-error-invalid-scope = la portée doit être session, workspace ou global
permission-rule-error-invalid-option-format = option au format clé=valeur attendue, reçu `{$token}`
operation-search-heading = recherche : {$query}
operation-search-results-heading = résultats de recherche
operation-command-exit-code = code de sortie {$code}
operation-diff-summary = diff : {$count} lignes
operation-diff-summary-detailed-one = diff : 1 fichier, +{$added} -{$deleted}
operation-diff-summary-detailed-one-rename = diff : 1 fichier, +{$added} -{$deleted}, {$renamed} renommé(s)
operation-diff-summary-detailed = diff : {$files} fichiers, +{$added} -{$deleted}
operation-diff-summary-detailed-renames = diff : {$files} fichiers, +{$added} -{$deleted}, {$renamed} renommé(s)
operation-nested-task-summary = {$title} ({$status})
snapshot-action-keep = conservé
snapshot-action-remove = supprimé
snapshot-action-exit = quitté
composer-placeholder-range-invalid = plage d’espace réservé de la zone de rédaction invalide
composer-placeholder-out-of-sync = état des espaces réservés de la zone de rédaction désynchronisé
composer-missing-staged-item = élément préparé manquant pour {$placeholder}
runtime-status-default = valeurs par défaut de la session
file-change-moved = déplacé
permission-label-auto-approve = approuver automatiquement
failure-detail-section-title = Détails de l’erreur
terminal-title-working = en cours
terminal-title-permission = autorisation requise
terminal-title-user-input = saisie requise
terminal-title-interrupted = interrompu
terminal-notification-working = en cours
terminal-notification-permission = autorisation requise
terminal-notification-user-input = saisie requise
terminal-notification-interrupted = interrompu
hub-section-directories = Dossiers
hub-section-pinned = Épinglées
hub-action-directory = Réduire le dossier
hub-action-pin = Épingler/désépingler
hub-directory-more = + Afficher plus de sessions (il en reste {$count})
tool-technical-details = Détails techniques
tool-presentation-data = Données de présentation
tool-detail-input = Entrée
tool-detail-output = Sortie
tool-detail-metadata = Métadonnées
hub-section-search = Résultats de recherche
hub-page-next = → Page suivante ({$page})
hub-page-previous = ← Page précédente ({$page})
btw-title = BTW · question en lecture seule
btw-description = Posez une question à part pendant que la conversation principale continue. Outils en lecture seule ; aucune modification de la conversation principale.
btw-send = Poser la question
btw-stop = Arrêter
btw-loading = Réponse en cours…
side-return = Conversation parallèle · revenir à la session parente
btw-footer = Entrée poser la question · PgUp/PgDn défiler · Ctrl+C arrêter · Tab/Esc revenir à la conversation
btw-input = Question BTW · Entrée envoyer · Maj+Entrée nouvelle ligne
btw-clear = Effacer
btw-copy = Copier
btw-copied = Réponse BTW copiée
btw-leave = Retour à la conversation
btw-stopped = Arrêtée · réponse partielle conservée
