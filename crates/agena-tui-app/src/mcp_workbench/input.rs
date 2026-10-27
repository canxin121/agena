use agena_api::mcp_connections::McpConnectionScope;
use agena_tui_components::{ConfirmDialogState, EditorDialogKeyResult, drive_editor_dialog_key};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

use super::state::{
    CapabilityKind, ConfirmAction, EditAction, FormField, MapEditor, McpDraft, McpPane,
    PENDING_RELOAD_HINT, display_value, next_scope, pointer_segment, scope_label,
};
use super::{App, McpOperation, McpWorkbench};

impl App {
    pub(crate) fn handle_mcp_workbench_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut McpWorkbench,
    ) -> bool {
        if dialog.busy {
            return false;
        }
        let code = match key.code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };
        if let Some(confirm) = dialog.confirm.as_ref() {
            match code {
                KeyCode::Esc | KeyCode::Char('n') => dialog.confirm = None,
                KeyCode::Enter | KeyCode::Char('y') => {
                    let action = confirm.action.clone();
                    dialog.confirm = None;
                    self.confirm_mcp_action(dialog, action);
                }
                _ => {}
            }
            return false;
        }
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    if editor.input.text() == dialog.editor_original {
                        dialog.editor = None;
                        dialog.editor_original.clear();
                        dialog.error = None;
                    } else {
                        confirm(
                            dialog,
                            "Discard input?",
                            "The unsaved input will be discarded.",
                            ConfirmAction::DiscardEditor,
                        );
                    }
                }
                EditorDialogKeyResult::Submit(action, text) => {
                    self.submit_mcp_editor(dialog, action, text)
                }
            }
            return false;
        }
        if dialog.authorization.is_some() {
            self.handle_mcp_authorization_key(dialog, code);
            return false;
        }
        if dialog.import.is_some() {
            self.handle_mcp_import_key(dialog, code);
            return false;
        }
        if dialog.map_editor.is_some() {
            self.handle_mcp_map_key(dialog, code);
            return false;
        }
        if dialog.draft.is_some() {
            self.handle_mcp_form_key(dialog, key, code);
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if code == KeyCode::Char('r') {
                self.request_mcp_catalog(dialog, None);
            }
            return false;
        }
        match code {
            KeyCode::Esc=>return true,
            KeyCode::Tab|KeyCode::BackTab=>dialog.pane=match dialog.pane{McpPane::Connections=>McpPane::Capabilities,McpPane::Capabilities=>McpPane::Connections},
            KeyCode::Up|KeyCode::Char('k')=>self.move_mcp_selection(dialog,-1),
            KeyCode::Down|KeyCode::Char('j')=>self.move_mcp_selection(dialog,1),
            KeyCode::Home=>self.move_mcp_selection(dialog,isize::MIN),KeyCode::End=>self.move_mcp_selection(dialog,isize::MAX),
            KeyCode::PageUp=>dialog.scroll=dialog.scroll.saturating_sub(10),KeyCode::PageDown=>dialog.scroll=dialog.scroll.saturating_add(10),
            KeyCode::Char('/')=>dialog.open_editor("Search connections","Filter connection names, transport, and status.",false,dialog.query.clone(),EditAction::Search),
            KeyCode::Char('g')=>{
                dialog.scope_filter=match dialog.scope_filter{None=>Some(McpConnectionScope::Workspace),Some(McpConnectionScope::Workspace)=>Some(McpConnectionScope::Global),Some(McpConnectionScope::Global)=>None};
                dialog.selected=0;self.request_mcp_detail(dialog);
            }
            KeyCode::Char('s')=>dialog.target_scope=next_scope(&dialog.target_scope),
            KeyCode::Char('n')=>{dialog.draft=Some(McpDraft::new(dialog.target_scope));dialog.error=None;}
            KeyCode::Char('e')=>if let Some(detail)=&dialog.detail{dialog.draft=Some(McpDraft::from_detail(detail));dialog.error=None;},
            KeyCode::Char('i')=>dialog.open_editor("Import MCP connections",
                "Paste a mcpServers or native servers JSON object. Preview hides credential values; installation requires a separate confirmation.",true,
                "{\n  \"mcpServers\": {\n  }\n}".to_owned(),EditAction::Import),
            KeyCode::Char('r')=>self.request_mcp_catalog(dialog,None),
            KeyCode::Char('b')=>if let Some(catalog)=&dialog.catalog{
                let operation=McpOperation::Bridge{enabled:!catalog.bridge_enabled,scope:dialog.target_scope};
                self.submit_mcp_operation(dialog,operation);
            },
            KeyCode::Char(' ')=>if let Some(detail)=&dialog.detail{
                let operation=McpOperation::Enabled{id:detail.summary.id.clone(),enabled:!detail.summary.enabled,revision:detail.summary.revision.clone()};
                self.submit_mcp_operation(dialog,operation);
            },
            KeyCode::Char('d')|KeyCode::Delete=>if let Some(detail)=&dialog.detail{
                let action=ConfirmAction::Delete{id:detail.summary.id.clone(),revision:detail.summary.revision.clone()};
                let message=format!("Remove {} from {} configuration? Another same-name scope may become effective.",detail.summary.name,scope_label(&detail.summary.scope));
                confirm(dialog,"Remove MCP connection?",&message,action);
            },
            KeyCode::Char('c')=>if let Some(detail)=&dialog.detail{
                if detail.summary.state=="pending_reload"{dialog.notice=Some(PENDING_RELOAD_HINT.to_owned());}
                else{let operation=McpOperation::Reconnect{id:detail.summary.id.clone()};self.submit_mcp_operation(dialog,operation);}
            },
            KeyCode::Char('t')=>if let Some(detail)=&dialog.detail{
                let operation=McpOperation::Test{id:detail.summary.id.clone()};self.submit_mcp_operation(dialog,operation);
            },
            KeyCode::Char('m')=>{dialog.kind=dialog.kind.next();dialog.cursor=None;dialog.cursor_history.clear();self.request_mcp_capabilities(dialog);},
            KeyCode::Char('>')=>if let Some(cursor)=dialog.capabilities.as_ref().and_then(|p|p.next_cursor.clone()){
                dialog.cursor_history.push(dialog.cursor.clone());dialog.cursor=Some(cursor);self.request_mcp_capabilities(dialog);
            },
            KeyCode::Char('<')=>if let Some(cursor)=dialog.cursor_history.pop(){dialog.cursor=cursor;self.request_mcp_capabilities(dialog);},
            KeyCode::Enter=>if dialog.pane==McpPane::Connections{dialog.pane=McpPane::Capabilities;}else{self.open_mcp_capability(dialog);},
            KeyCode::Char('a')=>self.start_mcp_authorization_input(dialog),
            KeyCode::Char('v')=>if let Some(detail)=&dialog.detail{
                if detail.config.pointer("/auth/kind").and_then(Value::as_str)!=Some("bearer_from_store"){
                    dialog.error=Some("Choose 'bearer from store' in the connection configuration before setting its stored bearer credential.".to_owned());
                }else{
                    let id=detail.summary.id.clone();dialog.open_editor("Set stored bearer token","Enter the replacement token. It is stored by the Agena server and is not returned to this screen.",false,String::new(),EditAction::StoredToken{id});
                }
            },
            KeyCode::Char('x')=>self.confirm_mcp_credential_removal(dialog),
            _=>{}
        }
        false
    }

    fn move_mcp_selection(&mut self, dialog: &mut McpWorkbench, delta: isize) {
        if dialog.pane == McpPane::Connections {
            let next = move_index(dialog.selected, delta, dialog.visible().len());
            if next != dialog.selected {
                dialog.selected = next;
                self.request_mcp_detail(dialog);
            }
        } else {
            dialog.capability_index = move_index(
                dialog.capability_index,
                delta,
                dialog.capabilities.as_ref().map_or(0, |p| p.items.len()),
            );
            dialog.preview = None;
            dialog.scroll = 0;
        }
    }

    fn handle_mcp_form_key(&mut self, dialog: &mut McpWorkbench, key: KeyEvent, code: KeyCode) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            if code == KeyCode::Char('s') {
                self.save_mcp_draft(dialog);
            }
            return;
        }
        let Some(draft) = dialog.draft.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                if draft.dirty {
                    confirm(
                        dialog,
                        "Discard connection changes?",
                        "The unsaved connection draft will be discarded.",
                        ConfirmAction::DiscardDraft,
                    );
                } else {
                    dialog.draft = None;
                    dialog.error = None;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                draft.selected = move_index(draft.selected, -1, draft.fields().len())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                draft.selected = move_index(draft.selected, 1, draft.fields().len())
            }
            KeyCode::Home => draft.selected = 0,
            KeyCode::End => draft.selected = draft.fields().len().saturating_sub(1),
            KeyCode::Char('s') => self.save_mcp_draft(dialog),
            KeyCode::Char('o') => {
                let value =
                    serde_json::to_string_pretty(&draft.redacted_config()).unwrap_or_default();
                dialog.open_editor("Edit connection JSON","Existing secrets are redacted. Keep [REDACTED] to preserve them; remove a field to clear it. The server validates the complete configuration.",true,value,EditAction::RawConfig);
            }
            KeyCode::Delete => match draft.field() {
                FormField::Token => draft.set("/auth", Value::Null),
                field @ (FormField::ClientId
                | FormField::ClientSecret
                | FormField::ClientMetadataUrl) => draft.remove(field.path()),
                _ => {}
            },
            KeyCode::Enter | KeyCode::Char('e') => {
                let field = draft.field();
                match field{
                    FormField::Scope=>if draft.editing.is_none(){draft.scope=next_scope(&draft.scope);draft.dirty=true;}else{dialog.notice=Some("The scope of an existing connection is immutable. Create or import another copy to change it.".to_owned());},
                    FormField::Transport=>draft.change_transport(),FormField::Enabled=>draft.set("/enabled",json!(!draft.enabled())),
                    FormField::Auth=>draft.change_auth(),
                    FormField::Name if draft.editing.is_some()=>dialog.notice=Some("Existing connection names and scopes are immutable. Create or import a new connection to use another identity.".to_owned()),
                    _ if field.is_map()=>dialog.map_editor=Some(MapEditor{field,selected:0}),
                    _=>{
                        let value=match field{
                            FormField::Name=>draft.name.clone(),FormField::Token|FormField::ClientSecret=>String::new(),
                            FormField::Scopes|FormField::Include|FormField::Exclude=>draft.config.pointer(field.path()).and_then(Value::as_array).map(|values|values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
                            _=>draft.config.pointer(field.path()).map(display_value).unwrap_or_default(),
                        };
                        dialog.open_editor(field.label(),field.hint(),field.multiline(),value,EditAction::Field(field));
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_mcp_map_key(&mut self, dialog: &mut McpWorkbench, code: KeyCode) {
        let keys = dialog.map_keys();
        let Some(map) = dialog.map_editor.as_mut() else {
            return;
        };
        let field = map.field;
        match code{
            KeyCode::Esc=>{dialog.map_editor=None;dialog.error=None;}
            KeyCode::Up|KeyCode::Char('k')=>map.selected=move_index(map.selected,-1,keys.len()+1),
            KeyCode::Down|KeyCode::Char('j')=>map.selected=move_index(map.selected,1,keys.len()+1),
            KeyCode::Home=>map.selected=0,KeyCode::End=>map.selected=keys.len(),
            KeyCode::Char('n')=>dialog.open_editor("New key","Enter an environment or header name. Its value is entered separately in masked input.",false,String::new(),EditAction::MapKey(field)),
            KeyCode::Enter|KeyCode::Char('e')=>{
                if let Some(key)=keys.get(map.selected){
                    let key=key.clone();dialog.open_editor("Replace value",&format!("Set a new value for {key}. Cancelling preserves the current value; an empty value is stored explicitly."),false,String::new(),EditAction::MapValue{field,key});
                }else{dialog.open_editor("New key","Enter an environment or header name.",false,String::new(),EditAction::MapKey(field));}
            }
            KeyCode::Char('d')|KeyCode::Delete=>if let Some(key)=keys.get(map.selected){
                let path=format!("{}/{}",field.path(),pointer_segment(key));
                confirm(dialog,"Remove configured value?",&format!("Remove {key} from the draft? It will be removed from the server only when the connection is saved."),ConfirmAction::RemoveMapKey{path});
            },
            _=>{}
        }
    }

    fn submit_mcp_editor(&mut self, dialog: &mut McpWorkbench, action: EditAction, text: String) {
        dialog.error = None;
        match action {
            EditAction::Search => {
                dialog.query = text.trim().to_owned();
                dialog.selected = 0;
                dialog.editor = None;
                self.request_mcp_detail(dialog);
            }
            EditAction::Field(field) => {
                let Some(draft) = &mut dialog.draft else {
                    return;
                };
                if field == FormField::Name {
                    draft.name = text.trim().to_owned();
                    draft.dirty = true;
                } else {
                    match parse_form_value(field, &text) {
                        Ok(value) => draft.set_field(field, value),
                        Err(error) => {
                            dialog.error = Some(error);
                            return;
                        }
                    }
                }
                dialog.editor = None;
                dialog.editor_original.clear();
            }
            EditAction::MapKey(field) => {
                let key = text.trim().to_owned();
                if !valid_map_key(field, &key) {
                    dialog.error=Some("Enter a non-empty valid key without control characters. Header names must be HTTP tokens; environment names cannot contain '='.".to_owned());
                    return;
                }
                dialog.open_editor("Set value",&format!("Enter the value for {key}. Existing values are kept unless you submit a replacement."),false,String::new(),EditAction::MapValue{field,key});
            }
            EditAction::MapValue { field, key } => {
                if let Some(draft) = &mut dialog.draft {
                    draft.set(
                        &format!("{}/{}", field.path(), pointer_segment(&key)),
                        json!(text),
                    );
                }
                dialog.editor = None;
                dialog.editor_original.clear();
            }
            EditAction::RawConfig => match parse_object(&text) {
                Ok(value) => {
                    if let Some(draft) = &mut dialog.draft {
                        draft.replace_redacted_config(value);
                    }
                    dialog.editor = None;
                    dialog.editor_original.clear();
                }
                Err(error) => dialog.error = Some(error),
            },
            EditAction::Import => match parse_object(&text) {
                Ok(value) => self.preview_mcp_import(dialog, value),
                Err(error) => dialog.error = Some(error),
            },
            EditAction::StoredToken { id } => {
                if text.trim().is_empty() {
                    dialog.error = Some(
                        "Enter a token, or use the separate remove-credential action to clear it."
                            .to_owned(),
                    );
                    return;
                }
                self.submit_mcp_operation(dialog, McpOperation::Credential { id, token: text });
            }
            EditAction::OAuthRedirect { id } => self.submit_mcp_operation(
                dialog,
                McpOperation::StartAuthorization {
                    id,
                    redirect_uri: text.trim().to_owned(),
                },
            ),
            EditAction::OAuthCallback { id, flow_id } => self.submit_mcp_operation(
                dialog,
                McpOperation::CompleteAuthorization {
                    id,
                    flow_id,
                    callback_url: text.trim().to_owned(),
                },
            ),
            EditAction::ResourceUri { id } => self.submit_mcp_operation(
                dialog,
                McpOperation::Resource {
                    id,
                    uri: text.trim().to_owned(),
                },
            ),
            EditAction::PromptArguments { id, name } => match parse_object(&text) {
                Ok(arguments) => self.submit_mcp_operation(
                    dialog,
                    McpOperation::Prompt {
                        id,
                        name,
                        arguments,
                    },
                ),
                Err(error) => dialog.error = Some(error),
            },
        }
    }

    fn save_mcp_draft(&mut self, dialog: &mut McpWorkbench) {
        let Some(draft) = &dialog.draft else {
            return;
        };
        if draft.name.trim().is_empty() {
            dialog.error = Some("A connection name is required.".to_owned());
            return;
        }
        let operation = McpOperation::Save {
            name: draft.name.clone(),
            scope: draft.scope,
            config: draft.config.clone(),
            preserve_secrets: draft.preserve_secrets.iter().cloned().collect(),
            editing: draft.editing.clone(),
        };
        self.submit_mcp_operation(dialog, operation);
    }

    fn confirm_mcp_action(&mut self, dialog: &mut McpWorkbench, action: ConfirmAction) {
        let operation = match action {
            ConfirmAction::DiscardEditor => {
                dialog.editor = None;
                dialog.editor_original.clear();
                dialog.error = None;
                return;
            }
            ConfirmAction::DiscardDraft => {
                dialog.draft = None;
                dialog.map_editor = None;
                dialog.error = None;
                return;
            }
            ConfirmAction::RemoveMapKey { path } => {
                if let Some(draft) = &mut dialog.draft {
                    draft.remove(&path);
                }
                return;
            }
            ConfirmAction::Delete { id, revision } => McpOperation::Delete { id, revision },
            ConfirmAction::DeleteCredential { id } => McpOperation::DeleteCredential { id },
            ConfirmAction::Logout { id } => McpOperation::CancelAuthorization { id, flow_id: None },
            ConfirmAction::CancelAuthorization { id, flow_id } => {
                McpOperation::CancelAuthorization {
                    id,
                    flow_id: Some(flow_id),
                }
            }
            ConfirmAction::Import => {
                let Some(import) = &dialog.import else {
                    return;
                };
                McpOperation::Import {
                    scope: import.scope,
                    document: import.document.clone(),
                    selected: import.selected.iter().cloned().collect(),
                    replace: import.replace,
                    revision: import.preview.revision.clone(),
                }
            }
        };
        self.submit_mcp_operation(dialog, operation);
    }

    fn handle_mcp_import_key(&mut self, dialog: &mut McpWorkbench, code: KeyCode) {
        let Some(import) = dialog.import.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                dialog.import = None;
                dialog.error = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                import.cursor = move_index(import.cursor, -1, import.preview.candidates.len())
            }
            KeyCode::Down | KeyCode::Char('j') => {
                import.cursor = move_index(import.cursor, 1, import.preview.candidates.len())
            }
            KeyCode::Char(' ') => {
                if let Some(candidate) = import.preview.candidates.get(import.cursor)
                    && candidate.valid
                    && !import.selected.remove(&candidate.name)
                {
                    import.selected.insert(candidate.name.clone());
                }
            }
            KeyCode::Char('a') => {
                let candidates: Vec<_> = import
                    .preview
                    .candidates
                    .iter()
                    .filter(|c| c.valid && (import.replace || !c.conflict))
                    .map(|c| c.name.clone())
                    .collect();
                if candidates.iter().all(|name| import.selected.contains(name)) {
                    import.selected.clear();
                } else {
                    import.selected = candidates.into_iter().collect();
                }
            }
            KeyCode::Char('o') => import.replace = !import.replace,
            KeyCode::Enter => {
                if import.selected.is_empty() {
                    dialog.error = Some("Select at least one valid connection.".to_owned());
                    return;
                }
                if !import.preview.errors.is_empty() {
                    dialog.error = Some(import.preview.errors.join("\n"));
                    return;
                }
                if !import.replace
                    && import
                        .preview
                        .candidates
                        .iter()
                        .any(|c| c.conflict && import.selected.contains(&c.name))
                {
                    dialog.error=Some("A selected connection already exists. Press O to allow replacement, or deselect that connection.".to_owned());
                    return;
                }
                let message = format!(
                    "Import {} connection(s) into {}{}?",
                    import.selected.len(),
                    scope_label(&import.scope),
                    if import.replace {
                        " and replace selected existing records"
                    } else {
                        ""
                    }
                );
                confirm(
                    dialog,
                    "Import MCP connections?",
                    &message,
                    ConfirmAction::Import,
                );
            }
            _ => {}
        }
    }

    fn start_mcp_authorization_input(&mut self, dialog: &mut McpWorkbench) {
        let Some(detail) = &dialog.detail else {
            return;
        };
        if !matches!(
            detail.config.pointer("/auth/kind").and_then(Value::as_str),
            Some("o_auth" | "oauth")
        ) {
            dialog.error = Some(
                "Choose OAuth in the connection configuration and save before authorizing."
                    .to_owned(),
            );
            return;
        }
        let id = detail.summary.id.clone();
        let redirect = self.application.client().mcp_authorization_redirect_uri();
        dialog.open_editor("Start OAuth authorization","Confirm the redirect URL. After signing in, copy the complete browser callback URL and paste it into this workbench.",false,redirect,EditAction::OAuthRedirect{id});
    }

    fn handle_mcp_authorization_key(&mut self, dialog: &mut McpWorkbench, code: KeyCode) {
        let Some(flow) = &dialog.authorization else {
            return;
        };
        match code {
            KeyCode::Char('c') => self.request_clipboard_copy(
                flow.authorization_url.clone(),
                "Authorization URL copied".to_owned(),
            ),
            KeyCode::Char('a') | KeyCode::Enter => {
                let action = EditAction::OAuthCallback {
                    id: flow.id.clone(),
                    flow_id: flow.flow_id.clone(),
                };
                dialog.open_editor("Complete OAuth authorization","Paste the complete URL from the browser after authorization. Agena verifies the flow and callback state.",false,String::new(),action);
            }
            KeyCode::Char('x') | KeyCode::Esc => {
                let action = ConfirmAction::CancelAuthorization {
                    id: flow.id.clone(),
                    flow_id: flow.flow_id.clone(),
                };
                confirm(
                    dialog,
                    "Cancel pending authorization?",
                    "This cancels only the pending sign-in flow; existing stored credentials are preserved.",
                    action,
                );
            }
            KeyCode::PageUp => dialog.scroll = dialog.scroll.saturating_sub(10),
            KeyCode::PageDown => dialog.scroll = dialog.scroll.saturating_add(10),
            _ => {}
        }
    }

    fn confirm_mcp_credential_removal(&mut self, dialog: &mut McpWorkbench) {
        let Some(detail) = &dialog.detail else {
            return;
        };
        let id = detail.summary.id.clone();
        match detail.config.pointer("/auth/kind").and_then(Value::as_str){
            Some("o_auth"|"oauth")=>confirm(dialog,"Remove OAuth credentials?","The connection configuration is kept. You will need to authorize again to use this service.",ConfirmAction::Logout{id}),
            Some("bearer_from_store")=>confirm(dialog,"Remove stored bearer token?","This removes the bearer token from the secret store and keeps the connection configuration.",ConfirmAction::DeleteCredential{id}),
            _=>dialog.error=Some("This connection has no separately managed bearer or OAuth credential. Edit its configuration to change inline or environment-based authentication.".to_owned()),
        }
    }

    fn open_mcp_capability(&mut self, dialog: &mut McpWorkbench) {
        let Some(detail) = &dialog.detail else {
            return;
        };
        if detail.summary.state != "connected" {
            dialog.notice = Some(
                if detail.summary.state == "pending_reload" {
                    PENDING_RELOAD_HINT
                } else {
                    "Connect and enable this service before inspecting its live capabilities."
                }
                .to_owned(),
            );
            return;
        }
        let Some(item) = dialog
            .capabilities
            .as_ref()
            .and_then(|page| page.items.get(dialog.capability_index))
            .cloned()
        else {
            return;
        };
        let id = detail.summary.id.clone();
        match dialog.kind {
            CapabilityKind::Tools => {
                dialog.preview = Some(("Tool definition".to_owned(), item));
                dialog.scroll = 0;
            }
            CapabilityKind::Resources | CapabilityKind::Templates => {
                let uri = item
                    .get("uri")
                    .or_else(|| item.get("uriTemplate"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                dialog.open_editor(
                    "Read resource",
                    "Confirm the resource URI. Replace any template variables before reading.",
                    false,
                    uri,
                    EditAction::ResourceUri { id },
                );
            }
            CapabilityKind::Prompts => {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let mut arguments = serde_json::Map::new();
                for argument in item
                    .get("arguments")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(name) = argument.get("name").and_then(Value::as_str) {
                        arguments.insert(name.to_owned(), json!(""));
                    }
                }
                dialog.open_editor("Get prompt","Provide prompt arguments as a JSON object. The result is shown for inspection.",true,serde_json::to_string_pretty(&arguments).unwrap_or_else(|_|"{}".to_owned()),EditAction::PromptArguments{id,name});
            }
        }
    }
}

fn confirm(dialog: &mut McpWorkbench, title: &str, message: &str, action: ConfirmAction) {
    dialog.confirm = Some(ConfirmDialogState::new(
        title.to_owned(),
        vec![message.to_owned()],
        "Enter / Y confirm · Esc cancel".to_owned(),
        action,
    ));
}
fn move_index(index: usize, delta: isize, count: usize) -> usize {
    if count == 0 || delta == isize::MIN {
        0
    } else if delta == isize::MAX {
        count - 1
    } else {
        index.saturating_add_signed(delta).min(count - 1)
    }
}
fn parse_object(text: &str) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| format!("Invalid JSON: {error}"))?;
    if !value.is_object() {
        return Err("Provide a JSON object.".to_owned());
    }
    Ok(value)
}
fn parse_form_value(field: FormField, text: &str) -> Result<Value, String> {
    match field {
        FormField::Arguments => {
            let value: Value = serde_json::from_str(text)
                .map_err(|error| format!("Arguments must be a JSON array of strings: {error}"))?;
            if !value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
            {
                return Err("Arguments must be a JSON array containing only strings.".to_owned());
            }
            Ok(value)
        }
        FormField::Scopes | FormField::Include | FormField::Exclude => Ok(json!(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        )),
        FormField::ConnectTimeout | FormField::RequestTimeout => {
            let value = text
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| "Enter a positive number of milliseconds.".to_owned())?;
            Ok(json!(value))
        }
        FormField::Cwd
        | FormField::ClientId
        | FormField::ClientSecret
        | FormField::ClientMetadataUrl
            if text.trim().is_empty() =>
        {
            Ok(Value::Null)
        }
        FormField::Token | FormField::ClientSecret => Ok(json!(text)),
        _ => Ok(json!(text.trim())),
    }
}
fn valid_map_key(field: FormField, key: &str) -> bool {
    if key.is_empty() || key.chars().any(char::is_control) {
        return false;
    }
    if field == FormField::Environment {
        return !key.contains('=');
    }
    key.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_arguments_keep_spaces_empty_values_and_literal_shell_syntax() {
        let input = r#"["two words","","$(literal)","--flag=value"]"#;
        assert_eq!(
            parse_form_value(FormField::Arguments, input).unwrap(),
            json!(["two words", "", "$(literal)", "--flag=value"])
        );
        assert!(parse_form_value(FormField::Arguments, r#"["valid",4]"#).is_err());
    }
    #[test]
    fn validates_timeout_map_keys_and_import_shape_before_dispatch() {
        assert!(parse_form_value(FormField::RequestTimeout, "0").is_err());
        assert!(valid_map_key(FormField::Environment, "API_KEY"));
        assert!(!valid_map_key(FormField::Environment, "KEY=value"));
        assert!(!valid_map_key(FormField::Headers, "X-Header\r\nInjected"));
        assert!(parse_object("[]").is_err());
    }
    #[test]
    fn optional_oauth_fields_can_be_cleared_and_client_secrets_keep_literal_contents() {
        for field in [
            FormField::ClientId,
            FormField::ClientSecret,
            FormField::ClientMetadataUrl,
        ] {
            assert_eq!(parse_form_value(field, "").unwrap(), Value::Null);
        }
        assert_eq!(
            parse_form_value(FormField::ClientSecret, " client secret ").unwrap(),
            json!(" client secret ")
        );
        assert_eq!(
            parse_form_value(FormField::ClientId, " client-id ").unwrap(),
            json!("client-id")
        );
    }
}
