//! Named Bot navigation. UUID selection is client-local; the server validates
//! membership and resolves the ordinary durable session on every entry.
use crate::{App, AppMessage, KeyEvent, Route, UiFailure, UiResult};
use agena_api::bot::*;
use agena_tui::main_focus::Focus;
use agena_tui_components::Editor;
use crossterm::event::{KeyCode, KeyModifiers};
use uuid::Uuid;

mod render;
mod requests;

#[derive(Default)]
pub(crate) struct BotState {
    selected: Option<Uuid>,
    profiles: Vec<BotProfile>,
    index: usize,
    request_id: u64,
    task: Option<tokio::task::AbortHandle>,
    error: Option<String>,
    view: BotView,
    form: Option<BotForm>,
    scope_request: u64,
    scope_assistant_pending: bool,
    archive_request: Option<(Uuid, BotArchive)>,
    /// The selected Bot's conversations, grouped by entrance shape.
    ///
    /// A Bot owns many conversations and no longer names a single one, so the
    /// list view is the only place that can say which conversation Enter opens.
    conversations: Vec<BotConversationGroup>,
    conversation_index: usize,
    conversations_loading: bool,
    conversations_error: Option<String>,
    /// True when the cursor is on the conversation list rather than the Bot
    /// list. Tab moves between them.
    focus_conversations: bool,
    /// The conversation awaiting delete confirmation.
    delete_conversation: Option<BotConversationRow>,
}

/// One entrance shape's conversations, in sidebar order.
#[derive(Default, Clone)]
pub(crate) struct BotConversationGroup {
    pub(crate) surface: &'static str,
    pub(crate) rows: Vec<BotConversationRow>,
}

#[derive(Clone)]
pub(crate) struct BotConversationRow {
    pub(crate) session_id: i64,
    pub(crate) title: String,
    pub(crate) favorite: bool,
    pub(crate) message_count: u64,
    pub(crate) last_message_at_ms: Option<i64>,
    pub(crate) running: bool,
}

const BOT_SURFACES: [&str; 4] = ["local", "direct", "group", "broadcast"];
#[derive(Default, PartialEq)]
enum BotView {
    #[default]
    Opening,
    List,
    Form,
    Archive,
    Reload,
    /// Confirm deleting one conversation of the highlighted Bot.
    DeleteConversation,
}
struct BotForm {
    base: Option<BotProfile>,
    name: String,
    persona: BotPersona,
    index: usize,
    edit: Option<Editor>,
    request_key: String,
}
impl BotForm {
    fn new(base: Option<BotProfile>) -> Self {
        Self {
            name: base.as_ref().map_or_else(String::new, |p| p.name.clone()),
            persona: base
                .as_ref()
                .map_or_else(BotPersona::default, |p| p.persona.clone()),
            base,
            index: 0,
            edit: None,
            request_key: Uuid::new_v4().to_string(),
        }
    }
    fn value(&self, index: usize) -> String {
        match index {
            0 => self.name.clone(),
            _ => self.persona.instructions.clone(),
        }
    }
    fn commit_field(&mut self) {
        if let Some(editor) = self.edit.take() {
            let value = editor.text().to_owned();
            match self.index {
                0 => self.name = value,
                1 => self.persona.instructions = value,
                _ => {}
            }
            self.request_key = Uuid::new_v4().to_string();
        }
    }
}
pub(crate) enum BotReply {
    List(BotList),
    Opened {
        profile: BotProfile,
        list: BotList,
        session: agena_api::bot::BotSession,
    },
    Saved(BotProfile),
    Archived(BotProfile),
    Edit(BotProfile),
    /// One Bot's conversations, already grouped by entrance shape.
    Conversations {
        bot: Uuid,
        groups: Vec<BotConversationGroup>,
    },
    /// A conversation was deleted; the Bot's list must be reloaded.
    DeletedConversation {
        bot: Uuid,
    },
}
enum BotOperation {
    List,
    Open(Option<Uuid>),
    Save(BotCreate, Option<(Uuid, i64)>),
    Archive(Uuid, BotArchive),
    Edit(Uuid),
    Conversations(Uuid),
    NewConversation(Uuid),
    DeleteConversation { bot: Uuid, session_id: i64 },
}
impl BotState {
    /// Every conversation row across all groups, in display order.
    pub(crate) fn conversation_rows(&self) -> Vec<&BotConversationRow> {
        self.conversations
            .iter()
            .flat_map(|group| group.rows.iter())
            .collect()
    }

    /// The row the conversation cursor points at.
    pub(crate) fn highlighted_conversation(&self) -> Option<&BotConversationRow> {
        self.conversation_rows()
            .get(self.conversation_index)
            .copied()
    }

    /// Flat list index where the first conversation row is drawn.
    ///
    /// The Bot rows come first, then one heading per non-empty surface; a
    /// conversation row's flat position is this offset plus its own index.
    pub(crate) fn flat_conversation_offset(&self) -> usize {
        self.profiles.iter().filter(|p| !p.archived).count()
            + if self.conversations.is_empty() { 0 } else { 1 }
            + self.conversations.len()
    }

    /// Drop the cached conversation list when the Bot under the cursor changes.
    ///
    /// The rows belong to one Bot, so they must never be shown under another;
    /// Tab fetches them again for whichever Bot is then highlighted.
    pub(crate) fn forget_conversations(&mut self) {
        self.conversations.clear();
        self.conversation_index = 0;
        self.focus_conversations = false;
        self.conversations_loading = false;
        self.conversations_error = None;
    }

    pub(crate) fn cancel_scope(&mut self) {
        self.scope_request = self.scope_request.wrapping_add(1);
        self.scope_assistant_pending = false;
    }
    pub(crate) fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.request_id = self.request_id.wrapping_add(1);
    }
    pub(crate) fn accepts_text(&self) -> bool {
        self.task.is_none()
            && self.form.as_ref().is_some_and(|f| f.edit.is_some())
            && self.view == BotView::Form
    }
    pub(crate) fn paste(&mut self, text: &str) {
        if self.accepts_text()
            && let Some(editor) = self.form.as_mut().and_then(|f| f.edit.as_mut())
            && editor.text().len() + text.len() <= 16384
        {
            editor.insert_str(text);
        }
    }
    fn highlighted(&self) -> Option<&BotProfile> {
        self.profiles.iter().filter(|p| !p.archived).nth(self.index)
    }
}
impl App {
    pub(crate) fn select_known_bot_for_session(&mut self, _session_id: i64) {
        // The Bot id arrives on the session itself (`owner_bot_id`), so the
        // registry list cannot disagree with the conversation being shown.
        if let Some(bot) = self.transcript.owner_bot_id
            && self
                .bot
                .profiles
                .iter()
                .any(|profile| !profile.archived && profile.id == bot)
        {
            self.bot.selected = Some(bot);
        }
    }
    pub(crate) fn current_bot_profile(&self) -> Option<&BotProfile> {
        let bot = self.transcript.owner_bot_id?;
        self.bot
            .profiles
            .iter()
            .find(|p| !p.archived && p.id == bot)
    }
    pub(crate) fn refresh_bot_registry(&mut self, assistant: bool) {
        if !assistant && self.bot.scope_assistant_pending {
            return;
        }
        self.bot.scope_assistant_pending = assistant;
        let Some(source) = self.current_or_selected_session_id() else {
            return;
        };
        let generation = self.subscription_generation;
        self.bot.scope_request = self.bot.scope_request.wrapping_add(1);
        let request = self.bot.scope_request;
        self.dispatch_backend_operation(
            move |backend| async move {
                Ok::<_, anyhow::Error>(
                    tokio::time::timeout(
                        std::time::Duration::from_secs(30),
                        backend.client().list_bots(true),
                    )
                    .await??,
                )
            },
            move |app, result| {
                if app.bot.scope_request != request
                    || app.subscription_generation != generation
                    || app.current_or_selected_session_id() != Some(source)
                {
                    return;
                }
                app.bot.scope_assistant_pending = false;
                match result {
                    Ok(list) => {
                        app.bot.profiles = list.items;
                        app.select_known_bot_for_session(source);
                        if assistant
                            && (app.current_route_is_main()
                                || matches!(app.current_route, Route::Hub(_)))
                        {
                            if let Some(id) = app
                                .bot
                                .profiles
                                .iter()
                                .find(|p| !p.archived && Some(p.id) == app.transcript.owner_bot_id)
                                .map(|p| p.id)
                            {
                                app.open_bot_persona_id(id);
                            } else if app
                                .bot
                                .profiles
                                .iter()
                                .any(|p| p.archived && Some(p.id) == app.transcript.owner_bot_id)
                            {
                                app.open_bot_picker();
                            } else {
                                app.open_workspace_assistant();
                            }
                        }
                    }
                    Err(error) if assistant => app.flash_warning(error.to_string()),
                    Err(_) => {}
                }
            },
        );
    }
    pub(crate) fn is_bot_session(&self) -> bool {
        self.current_bot_profile().is_some()
    }
    pub(crate) fn open_bot(&mut self) {
        if matches!(self.current_route, Route::BotEntry(_))
            && self.bot.view == BotView::Opening
            && self.bot.task.is_some()
        {
            return;
        }
        self.enter_bot_surface();
        self.bot.view = BotView::Opening;
        self.request_bot(BotOperation::Open(self.bot.selected));
    }
    pub(crate) fn open_bot_picker(&mut self) {
        self.enter_bot_surface();
        self.bot.form = None;
        self.bot.view = BotView::List;
        self.request_bot(BotOperation::List);
    }
    fn enter_bot_surface(&mut self) {
        self.bot.cancel_scope();
        if !matches!(self.current_route, Route::BotEntry(_)) {
            let mut previous = std::mem::replace(&mut self.current_route, Route::Main);
            if let Route::Hub(state) = &mut previous
                && let Some(task) = state.request_task.take()
            {
                task.abort();
            }
            self.route_stack.push(previous);
        }
        self.bot.cancel();
        self.bot.error = None;
        self.current_route = Route::BotEntry(self.bot.request_id);
    }
    pub(crate) fn open_bot_persona(&mut self) {
        let Some(id) = self.current_bot_profile().map(|p| p.id) else {
            self.open_bot_picker();
            return;
        };
        self.open_bot_persona_id(id);
    }
    fn open_bot_persona_id(&mut self, id: Uuid) {
        self.enter_bot_surface();
        self.bot.view = BotView::Form;
        self.bot.form = None;
        self.request_bot(BotOperation::Edit(id));
    }
    pub(crate) fn execute_bot_command(&mut self, args: &str) {
        match args.trim() {
            "" => self.open_bot(),
            "list" => self.open_bot_picker(),
            "create" => {
                self.enter_bot_surface();
                self.new_bot_form();
            }
            "edit" => self.open_bot_persona(),
            id => match Uuid::parse_str(id) {
                Ok(id) => {
                    self.enter_bot_surface();
                    self.bot.selected = Some(id);
                    self.bot.view = BotView::Opening;
                    self.request_bot(BotOperation::Open(Some(id)));
                }
                Err(_) => self.flash_warning(self.i18n.text("bot-command-help")),
            },
        }
    }
    pub(crate) fn handle_bot_session_ready(&mut self, request_id: u64, result: UiResult<BotReply>) {
        if self.bot.request_id != request_id
            || !matches!(self.current_route, Route::BotEntry(id) if id == request_id)
        {
            return;
        }
        self.bot.task = None;
        match result {
            Ok(BotReply::List(list)) => {
                self.bot.profiles = list.items;
                self.bot.index = self
                    .bot
                    .profiles
                    .iter()
                    .filter(|p| !p.archived)
                    .position(|p| Some(p.id) == self.bot.selected)
                    .unwrap_or(0);
                self.bot.view = BotView::List;
                // Conversations load on Tab rather than with the list: the Bot
                // list must stay immediately interactive, and most opens use
                // the default conversation anyway.
                self.bot.conversations.clear();
                self.bot.conversation_index = 0;
                self.bot.focus_conversations = false;
            }
            Ok(BotReply::DeletedConversation { bot }) => {
                self.bot.delete_conversation = None;
                self.bot.view = BotView::List;
                if self.bot.highlighted().map(|p| p.id) == Some(bot) {
                    self.bot.conversations_loading = true;
                    self.request_bot(BotOperation::Conversations(bot));
                }
            }
            Ok(BotReply::Conversations { bot, groups }) => {
                // Ignore a response for a Bot the cursor has left: the sidebar
                // must never show one identity's conversations under another.
                if self.bot.highlighted().map(|p| p.id) == Some(bot) {
                    self.bot.conversations = groups;
                    self.bot.conversations_loading = false;
                    self.bot.conversations_error = None;
                    self.bot.conversation_index = 0;
                    self.bot.focus_conversations = !self.bot.conversation_rows().is_empty();
                }
                self.bot.view = BotView::List;
            }
            Ok(BotReply::Opened {
                profile,
                list,
                session,
            }) => {
                self.bot.profiles = list.items;
                self.remember_bot_profile(profile.clone());
                self.bot.selected = Some(profile.id);
                self.current_route = Route::Main;
                self.route_stack.clear();
                self.open_session(session.session_id, profile.name);
                // Opening from the Bot picker is itself proof of ownership; a
                // later state shell may confirm it but cannot remove it.
                self.transcript.owner_bot_id = Some(profile.id);
                self.focus = Focus::Composer;
            }
            Ok(BotReply::Saved(profile)) => {
                let created = self.bot.form.as_ref().is_some_and(|f| f.base.is_none());
                self.remember_bot_profile(profile.clone());
                self.bot.form = None;
                if created {
                    self.bot.selected = Some(profile.id);
                    self.bot.view = BotView::Opening;
                    self.request_bot(BotOperation::Open(Some(profile.id)));
                } else {
                    self.bot.view = BotView::List;
                    self.request_bot(BotOperation::List);
                }
            }
            Ok(BotReply::Edit(profile)) => {
                self.remember_bot_profile(profile.clone());
                self.bot.form = Some(BotForm::new(Some(profile)));
                self.bot.view = BotView::Form;
            }
            Ok(BotReply::Archived(profile)) => {
                if self.bot.selected == Some(profile.id) {
                    self.bot.selected = None;
                }
                self.remember_bot_profile(profile);
                self.bot.view = BotView::List;
                self.request_bot(BotOperation::List);
            }
            Err(error) => self.bot.error = Some(error.to_string()),
        }
    }
    fn remember_bot_profile(&mut self, profile: BotProfile) {
        self.bot.profiles.retain(|p| p.id != profile.id);
        self.bot.profiles.push(profile);
    }
    fn new_bot_form(&mut self) {
        self.bot.view = BotView::Form;
        self.bot.form = Some(BotForm::new(None));
        self.bot.error = None;
    }
    fn save_bot_form(&mut self) {
        let Some(form) = &self.bot.form else {
            return;
        };
        if form.name.trim().is_empty()
            || form.persona.instructions.len() > MAX_BOT_INSTRUCTIONS_BYTES
        {
            self.bot.error = Some(self.i18n.text("bot-form-invalid"));
            return;
        }
        self.request_bot(BotOperation::Save(
            BotCreate {
                request_key: form.request_key.clone(),
                name: form.name.trim().into(),
                persona: Some(form.persona.clone()),
            },
            form.base.as_ref().map(|p| (p.id, p.revision)),
        ));
    }
    pub(crate) fn handle_bot_entry_key(&mut self, key: KeyEvent) -> bool {
        if self.bot.accepts_text() {
            if key.code == KeyCode::Esc {
                self.bot.form.as_mut().unwrap().edit = None;
            } else if key.code == KeyCode::Enter
                && key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                self.bot.form.as_mut().unwrap().commit_field();
            } else {
                self.bot
                    .form
                    .as_mut()
                    .unwrap()
                    .edit
                    .as_mut()
                    .unwrap()
                    .handle_multiline_input_key(key);
            }
            return false;
        }
        if key.code == KeyCode::Esc {
            if self.bot.task.is_some()
                || self.bot.view == BotView::List
                || self.bot.view == BotView::Opening
            {
                self.bot.cancel();
                if matches!(self.route_stack.last(), Some(Route::Hub(_)))
                    && let Some(Route::Hub(mut state)) = self.route_stack.pop()
                {
                    self.spawn_hub_overview_request(&mut state);
                    self.current_route = Route::Hub(state);
                }
                return true;
            }
            if self.bot.view == BotView::DeleteConversation {
                self.bot.delete_conversation = None;
                self.bot.view = BotView::List;
                return false;
            }
            if self.bot.view == BotView::Reload {
                self.bot.view = BotView::Form;
            } else {
                self.bot.form = None;
                self.bot.view = BotView::List;
            }
            self.bot.error = None;
            return false;
        }
        if self.bot.task.is_some() {
            return false;
        }
        match self.bot.view {
            BotView::Opening => match key.code {
                KeyCode::Char('l') => {
                    self.bot.form = None;
                    self.bot.view = BotView::List;
                    self.request_bot(BotOperation::List);
                }
                KeyCode::Enter | KeyCode::Char('r') => {
                    self.bot.view = BotView::Opening;
                    self.request_bot(BotOperation::Open(self.bot.selected));
                }
                _ => {}
            },
            BotView::List => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.bot.focus_conversations {
                        self.bot.conversation_index = self.bot.conversation_index.saturating_sub(1);
                    } else {
                        self.bot.index = self.bot.index.saturating_sub(1);
                        self.bot.forget_conversations();
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.bot.focus_conversations {
                        self.bot.conversation_index = (self.bot.conversation_index + 1)
                            .min(self.bot.conversation_rows().len().saturating_sub(1));
                    } else {
                        self.bot.index = (self.bot.index + 1).min(
                            self.bot
                                .profiles
                                .iter()
                                .filter(|p| !p.archived)
                                .count()
                                .saturating_sub(1),
                        );
                        self.bot.forget_conversations();
                    }
                }
                // Conversations are a second list under the highlighted Bot.
                // Tab moves into it so a Bot with several rooms can be opened
                // on the one the user wants rather than the server default.
                KeyCode::Tab => {
                    if self.bot.focus_conversations {
                        self.bot.focus_conversations = false;
                    } else if !self.bot.conversations.is_empty() {
                        self.bot.focus_conversations = true;
                        self.bot.conversation_index = 0;
                    } else if let Some(id) = self.bot.highlighted().map(|p| p.id) {
                        // First Tab on a Bot fetches its rooms; the reply moves
                        // the cursor down into them.
                        self.bot.conversations_loading = true;
                        self.bot.conversations_error = None;
                        self.request_bot(BotOperation::Conversations(id));
                    }
                }
                KeyCode::Char('n') => {
                    if self.bot.focus_conversations {
                        // A new conversation is the Bot's, not a new Bot.
                        if let Some(id) = self.bot.highlighted().map(|p| p.id) {
                            self.request_bot(BotOperation::NewConversation(id));
                        }
                    } else {
                        self.new_bot_form();
                    }
                }
                KeyCode::Char('d') if self.bot.focus_conversations => {
                    if let Some(row) = self.bot.highlighted_conversation().cloned() {
                        self.bot.delete_conversation = Some(row);
                        self.bot.view = BotView::DeleteConversation;
                    }
                }
                KeyCode::Char('r') => self.request_bot(BotOperation::List),
                KeyCode::Enter => {
                    let conversation = self
                        .bot
                        .highlighted_conversation()
                        .map(|row| (row.session_id, row.title.clone()));
                    if self.bot.focus_conversations
                        && let Some((session_id, title)) = conversation
                    {
                        self.bot.view = BotView::Opening;
                        self.open_session(session_id, title);
                        // Ownership is already known: this row is one of the
                        // highlighted Bot's conversations.
                        if let Some(id) = self.bot.highlighted().map(|p| p.id) {
                            self.bot.selected = Some(id);
                            self.transcript.owner_bot_id = Some(id);
                        }
                        self.focus = Focus::Composer;
                        return false;
                    }
                    if let Some(id) = self.bot.highlighted().map(|p| p.id) {
                        self.bot.selected = Some(id);
                        self.bot.view = BotView::Opening;
                        self.request_bot(BotOperation::Open(Some(id)));
                    }
                }
                KeyCode::Char('e') => {
                    if let Some(id) = self.bot.highlighted().map(|p| p.id) {
                        self.request_bot(BotOperation::Edit(id));
                    }
                }
                KeyCode::Char('a') => {
                    if let Some(profile) = self.bot.highlighted() {
                        self.bot.archive_request = Some((
                            profile.id,
                            BotArchive {
                                request_key: Uuid::new_v4().to_string(),
                                expected_revision: profile.revision,
                            },
                        ));
                        self.bot.view = BotView::Archive;
                    }
                }
                KeyCode::Char('w') => self.open_settings_studio(),
                _ => {}
            },
            BotView::Form => match key.code {
                KeyCode::Up => {
                    if let Some(f) = &mut self.bot.form {
                        f.index = (f.index + 1) % 2;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if let Some(f) = &mut self.bot.form {
                        f.index = (f.index + 1) % 2;
                    }
                }
                KeyCode::Char('s') => self.save_bot_form(),
                KeyCode::Char('r') => {
                    if self.bot.form.as_ref().is_some_and(|f| f.base.is_some()) {
                        self.bot.view = BotView::Reload;
                    }
                }
                KeyCode::Enter | KeyCode::Char('e') => {
                    if let Some(f) = &mut self.bot.form {
                        let mut editor = Editor::default();
                        editor.set_text(f.value(f.index));
                        f.edit = Some(editor);
                    }
                }
                _ => {}
            },
            BotView::Archive => {
                if key.code == KeyCode::Enter
                    && let Some((id, body)) = self.bot.archive_request.clone()
                {
                    self.request_bot(BotOperation::Archive(id, body));
                }
            }
            BotView::DeleteConversation => match key.code {
                KeyCode::Enter => {
                    if let (Some(bot), Some(row)) = (
                        self.bot.highlighted().map(|p| p.id),
                        self.bot.delete_conversation.clone(),
                    ) {
                        self.request_bot(BotOperation::DeleteConversation {
                            bot,
                            session_id: row.session_id,
                        });
                    }
                }
                KeyCode::Esc => {
                    self.bot.delete_conversation = None;
                    self.bot.view = BotView::List;
                }
                _ => {}
            },
            BotView::Reload => {
                if key.code == KeyCode::Enter
                    && let Some(id) = self
                        .bot
                        .form
                        .as_ref()
                        .and_then(|f| f.base.as_ref())
                        .map(|p| p.id)
                {
                    self.request_bot(BotOperation::Edit(id));
                }
            }
        }
        false
    }
}
#[cfg(test)]
mod tests;
