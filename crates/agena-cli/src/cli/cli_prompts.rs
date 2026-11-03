//! Prompt-layer commands backed by the long-lived server.

use std::path::Path;

use agena_prompt_layers::{FragmentId, LayerMode, PromptCondition, PromptFragment, PromptLayer};
use serde::{Deserialize, Serialize};

use super::cli_server::ServerSessionClient;
use super::{
    AgenaCli, AppError, OutputFormat, PromptAddArgs, PromptClearArgs, PromptEditArgs,
    PromptListArgs, PromptModeArg, PromptMoveArgs, PromptPreviewArgs, PromptRemoveArgs,
    PromptRequiresToolsArg, PromptScopeArg, PromptSetArgs, PromptTargetArgs, PromptToggleArgs,
    PromptsCommand, PromptsSubcommand, render_serialized,
};

const PROMPTS_SETTING_PATH: &str = "prompts";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PromptScopeFile {
    mode: LayerMode,
    order: Vec<FragmentId>,
    disabled: Vec<FragmentId>,
    inline: Vec<PromptFragment>,
    models: Vec<serde_json::Value>,
}

impl PromptScopeFile {
    fn into_layer(self) -> PromptLayer {
        PromptLayer {
            mode: self.mode,
            order: self.order,
            disabled: self.disabled,
            inline: self.inline,
        }
    }

    fn set_layer(&mut self, layer: &PromptLayer) {
        self.mode = layer.mode;
        self.order.clone_from(&layer.order);
        self.disabled.clone_from(&layer.disabled);
        self.inline.clone_from(&layer.inline);
    }

    fn transform_model_references(
        &mut self,
        old_id: &FragmentId,
        new_id: Option<&FragmentId>,
    ) -> bool {
        let mut changed = false;
        for model in &mut self.models {
            let Some(model) = model.as_object_mut() else {
                continue;
            };
            // A model layer may define its own inline fragment with this id.
            // In that case its order refers to its own fragment, not ours.
            if model
                .get("inline")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|inline| {
                    inline.iter().any(|fragment| {
                        fragment.get("id").and_then(serde_json::Value::as_str)
                            == Some(old_id.as_str())
                    })
                })
            {
                continue;
            }
            for field in ["order", "disabled"] {
                let Some(ids) = model
                    .get_mut(field)
                    .and_then(serde_json::Value::as_array_mut)
                else {
                    continue;
                };
                ids.retain_mut(|value| {
                    if value.as_str() != Some(old_id.as_str()) {
                        return true;
                    }
                    changed = true;
                    match new_id {
                        Some(new_id) => {
                            *value = serde_json::Value::String(new_id.to_string());
                            true
                        }
                        None => false,
                    }
                });
            }
        }
        changed
    }
}

struct PromptScopeState {
    scope: PromptScopeArg,
    session_id: Option<i64>,
    layer: PromptLayer,
    file: Option<PromptScopeFile>,
    revision: Option<String>,
}

impl PromptScopeState {
    fn output(&self) -> serde_json::Value {
        let mut output = serde_json::Map::new();
        output.insert(
            "scope".to_owned(),
            serde_json::Value::String(self.scope.as_str().to_owned()),
        );
        if let Some(session_id) = self.session_id {
            output.insert("session_id".to_owned(), serde_json::json!(session_id));
        }
        output.insert(
            "prompts".to_owned(),
            serde_json::to_value(&self.layer).expect("prompt layer always serializes"),
        );
        if let Some(file) = &self.file
            && !file.models.is_empty()
        {
            output.insert(
                "models".to_owned(),
                serde_json::to_value(&file.models).expect("model overlays always serialize"),
            );
        }
        serde_json::Value::Object(output)
    }

    fn validate(&self) -> Result<(), AppError> {
        self.layer
            .validate()
            .map_err(|error| AppError::Config(format!("invalid prompt layer: {error}")))
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PromptClearAction {
    DeleteSetting { scope: PromptScopeArg },
    SetSession { session_id: i64, layer: PromptLayer },
}

impl AgenaCli {
    pub(super) async fn run_prompts(self, command: PromptsCommand) -> Result<(), AppError> {
        let command = command.command.unwrap_or({
            PromptsSubcommand::List(PromptListArgs {
                target: PromptTargetArgs {
                    scope: PromptScopeArg::Global,
                    session_id: None,
                },
                format: OutputFormat::Json,
            })
        });
        let server = ServerSessionClient::connect(&self, None).await?;
        let (format, output) = run_prompt_command(&server.client, command).await?;
        println!("{}", render_serialized(format, &output)?);
        Ok(())
    }
}

async fn run_prompt_command(
    client: &agena_client::AgenaClient,
    command: PromptsSubcommand,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    match command {
        PromptsSubcommand::List(args) => {
            let state = load_scope(client, &args.target).await?;
            Ok((args.format, state.output()))
        }
        PromptsSubcommand::Preview(args) => preview(client, args).await,
        PromptsSubcommand::Add(args) => add(client, args).await,
        PromptsSubcommand::Edit(args) => edit(client, args).await,
        PromptsSubcommand::Remove(args) => remove(client, args).await,
        PromptsSubcommand::Move(args) => move_fragment(client, args).await,
        PromptsSubcommand::Disable(args) => set_disabled(client, args, true).await,
        PromptsSubcommand::Enable(args) => set_disabled(client, args, false).await,
        PromptsSubcommand::Mode(args) => {
            let mut state = load_scope(client, &args.target).await?;
            state.layer.mode = match args.mode {
                PromptModeArg::Append => LayerMode::Append,
                PromptModeArg::Replace => LayerMode::Replace,
            };
            save_scope(client, state, args.format).await
        }
        PromptsSubcommand::Set(args) => set_layer(client, args).await,
        PromptsSubcommand::Clear(args) => clear_scope(client, args).await,
    }
}

async fn preview(
    client: &agena_client::AgenaClient,
    args: PromptPreviewArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    let preview = client
        .session_prompt_preview(args.session_id)
        .await
        .map_err(|error| client_error("failed to preview the resolved prompt", error))?;
    Ok((args.format, serde_json::to_value(preview)?))
}

async fn add(
    client: &agena_client::AgenaClient,
    args: PromptAddArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    validate_target(&args.target)?;
    let id = FragmentId::parse(&args.id)
        .map_err(|error| AppError::Config(format!("invalid fragment id: {error}")))?;
    let mut state = load_scope(client, &args.target).await?;
    let body = read_body(args.body, args.file.as_deref()).await?;
    if state.layer.order.contains(&id)
        || state.layer.inline.iter().any(|fragment| fragment.id == id)
        || state.layer.disabled.contains(&id)
    {
        return Err(AppError::Config(format!(
            "fragment `{}` already exists or is referenced in this scope",
            args.id
        )));
    }
    state.layer.inline.push(PromptFragment {
        id: id.clone(),
        title: args.title.unwrap_or_default(),
        body: body.unwrap_or_default(),
        when: PromptCondition {
            models: args.models,
            requires_tools: requires_tools(args.requires_tools),
        },
    });
    state.layer.order.push(id);
    save_scope(client, state, args.format).await
}

async fn edit(
    client: &agena_client::AgenaClient,
    args: PromptEditArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    validate_target(&args.target)?;
    let id = FragmentId::parse(&args.id)
        .map_err(|error| AppError::Config(format!("invalid fragment id: {error}")))?;
    let new_id = args
        .new_id
        .as_deref()
        .map(FragmentId::parse)
        .transpose()
        .map_err(|error| AppError::Config(format!("invalid replacement fragment id: {error}")))?;
    let mut state = load_scope(client, &args.target).await?;
    let body = read_body(args.body, args.file.as_deref()).await?;
    let index = state
        .layer
        .inline
        .iter()
        .position(|fragment| fragment.id == id)
        .ok_or_else(|| {
            AppError::Config(format!(
                "fragment `{}` is not an inline fragment in this scope",
                args.id
            ))
        })?;
    if let Some(new_id) = &new_id
        && new_id != &id
        && (state.layer.order.contains(new_id)
            || state.layer.disabled.contains(new_id)
            || state
                .layer
                .inline
                .iter()
                .enumerate()
                .any(|(other, fragment)| other != index && &fragment.id == new_id))
    {
        return Err(AppError::Config(format!(
            "fragment id `{}` is already used in this scope",
            new_id
        )));
    }
    if let Some(new_id) = &new_id
        && new_id != &id
    {
        rename_fragment_references(&mut state, index, &id, new_id);
    }
    if let Some(title) = args.title {
        state.layer.inline[index].title = title;
    }
    if let Some(body) = body {
        state.layer.inline[index].body = body;
    }
    if args.clear_models || !args.models.is_empty() {
        state.layer.inline[index].when.models = args.models;
    }
    if args.requires_tools.is_some() {
        state.layer.inline[index].when.requires_tools = requires_tools(args.requires_tools);
    }
    save_scope(client, state, args.format).await
}

async fn remove(
    client: &agena_client::AgenaClient,
    args: PromptRemoveArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    let id = FragmentId::parse(&args.id)
        .map_err(|error| AppError::Config(format!("invalid fragment id: {error}")))?;
    let mut state = load_scope(client, &args.target).await?;
    if !remove_fragment_references(&mut state, &id) {
        return Err(AppError::Config(format!(
            "fragment `{}` is not referenced in this scope",
            args.id
        )));
    }
    save_scope(client, state, args.format).await
}

async fn move_fragment(
    client: &agena_client::AgenaClient,
    args: PromptMoveArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    let id = FragmentId::parse(&args.id)
        .map_err(|error| AppError::Config(format!("invalid fragment id: {error}")))?;
    let mut state = load_scope(client, &args.target).await?;
    let current = state
        .layer
        .order
        .iter()
        .position(|fragment_id| fragment_id == &id)
        .ok_or_else(|| {
            AppError::Config(format!(
                "fragment `{}` is not present in this scope's order",
                args.id
            ))
        })?;
    let destination = if args.up {
        current.saturating_sub(1)
    } else if args.down {
        (current + 1).min(state.layer.order.len().saturating_sub(1))
    } else if let Some(position) = args.position {
        if position == 0 || position > state.layer.order.len() {
            return Err(AppError::Config(format!(
                "--position must be between 1 and {}",
                state.layer.order.len()
            )));
        }
        position - 1
    } else {
        return Err(AppError::Config(
            "pass --up, --down, or --position".to_owned(),
        ));
    };
    let moved = state.layer.order.remove(current);
    state.layer.order.insert(destination, moved);
    save_scope(client, state, args.format).await
}

async fn set_disabled(
    client: &agena_client::AgenaClient,
    args: PromptToggleArgs,
    disabled: bool,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    let id = FragmentId::parse(&args.id)
        .map_err(|error| AppError::Config(format!("invalid fragment id: {error}")))?;
    let mut state = load_scope(client, &args.target).await?;
    if disabled {
        if state.layer.order.contains(&id)
            || state.layer.inline.iter().any(|fragment| fragment.id == id)
        {
            return Err(AppError::Config(format!(
                "fragment `{}` is defined in this scope; remove it from the order or delete the inline fragment instead",
                args.id
            )));
        }
        if !state.layer.disabled.contains(&id) {
            state.layer.disabled.push(id);
        }
    } else {
        state
            .layer
            .disabled
            .retain(|fragment_id| fragment_id != &id);
    }
    save_scope(client, state, args.format).await
}

async fn set_layer(
    client: &agena_client::AgenaClient,
    args: PromptSetArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    validate_target(&args.target)?;
    let text = read_file_or_stdin(args.file.as_path()).await?;
    let layer: PromptLayer = serde_json::from_str(&text)
        .map_err(|error| AppError::Config(format!("parse prompt layer JSON: {error}")))?;
    let mut state = load_scope(client, &args.target).await?;
    state.layer = layer;
    save_scope(client, state, args.format).await
}

async fn clear_scope(
    client: &agena_client::AgenaClient,
    args: PromptClearArgs,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    match clear_action(&args.target)? {
        PromptClearAction::DeleteSetting { scope } => {
            client
                .delete_settings_layer_value(layer_name(scope), PROMPTS_SETTING_PATH, false, true)
                .await
                .map_err(|error| client_error("failed to clear the prompt layer", error))?;
            let state = PromptScopeState {
                scope,
                session_id: None,
                layer: PromptLayer::default(),
                file: Some(PromptScopeFile::default()),
                revision: None,
            };
            Ok((args.format, state.output()))
        }
        PromptClearAction::SetSession { session_id, layer } => {
            client
                .set_session_prompts(session_id, Some(layer.clone()))
                .await
                .map_err(|error| client_error("failed to clear session prompt layers", error))?;
            let state = PromptScopeState {
                scope: PromptScopeArg::Session,
                session_id: Some(session_id),
                layer,
                file: None,
                revision: None,
            };
            Ok((args.format, state.output()))
        }
    }
}

async fn load_scope(
    client: &agena_client::AgenaClient,
    target: &PromptTargetArgs,
) -> Result<PromptScopeState, AppError> {
    let (scope, session_id) = validate_target(target)?;
    match scope {
        PromptScopeArg::Global | PromptScopeArg::Workspace => {
            let response = client
                .settings_layer_value(layer_name(scope), PROMPTS_SETTING_PATH)
                .await
                .map_err(|error| client_error("failed to read prompt layers", error))?;
            let revision = response
                .get("revision")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned);
            let value = response
                .get("value")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let file = if value.is_null() {
                PromptScopeFile::default()
            } else {
                serde_json::from_value::<PromptScopeFile>(value).map_err(|error| {
                    AppError::Config(format!("prompt layer setting is invalid: {error}"))
                })?
            };
            Ok(PromptScopeState {
                scope,
                session_id: None,
                layer: file.clone().into_layer(),
                file: Some(file),
                revision,
            })
        }
        PromptScopeArg::Session => {
            let session_id = session_id.expect("validated session target");
            let execution = client
                .get_session_state(session_id)
                .await
                .map_err(|error| client_error("failed to read session prompt layers", error))?;
            Ok(PromptScopeState {
                scope,
                session_id: Some(session_id),
                layer: execution.execution.prompts,
                file: None,
                revision: None,
            })
        }
    }
}

async fn save_scope(
    client: &agena_client::AgenaClient,
    mut state: PromptScopeState,
    format: OutputFormat,
) -> Result<(OutputFormat, serde_json::Value), AppError> {
    state.validate()?;
    match state.scope {
        PromptScopeArg::Global | PromptScopeArg::Workspace => {
            let file = state
                .file
                .as_mut()
                .ok_or_else(|| AppError::Internal("prompt scope file was not loaded".to_owned()))?;
            file.set_layer(&state.layer);
            let value = serde_json::to_value(&*file)?;
            client
                .set_settings_layer_value_if_revision(
                    layer_name(state.scope),
                    PROMPTS_SETTING_PATH,
                    value,
                    false,
                    true,
                    state.revision.as_deref(),
                )
                .await
                .map_err(|error| client_error("failed to save prompt layers", error))?;
        }
        PromptScopeArg::Session => {
            let session_id = state.session_id.ok_or_else(|| {
                AppError::Internal("session prompt scope is missing its session id".to_owned())
            })?;
            client
                .set_session_prompts(session_id, Some(state.layer.clone()))
                .await
                .map_err(|error| client_error("failed to save session prompt layers", error))?;
        }
    }
    Ok((format, state.output()))
}

fn validate_target(target: &PromptTargetArgs) -> Result<(PromptScopeArg, Option<i64>), AppError> {
    match target.scope {
        PromptScopeArg::Session => target
            .session_id
            .map(|session_id| (target.scope, Some(session_id)))
            .ok_or_else(|| {
                AppError::Config("--scope session requires --session-id <id>".to_owned())
            }),
        PromptScopeArg::Global | PromptScopeArg::Workspace => {
            if target.session_id.is_some() {
                return Err(AppError::Config(
                    "--session-id is only valid with --scope session".to_owned(),
                ));
            }
            Ok((target.scope, None))
        }
    }
}

fn clear_action(target: &PromptTargetArgs) -> Result<PromptClearAction, AppError> {
    let (scope, session_id) = validate_target(target)?;
    Ok(match scope {
        PromptScopeArg::Global | PromptScopeArg::Workspace => {
            PromptClearAction::DeleteSetting { scope }
        }
        PromptScopeArg::Session => PromptClearAction::SetSession {
            session_id: session_id.expect("validated session target"),
            layer: PromptLayer::default(),
        },
    })
}

fn rename_fragment_references(
    state: &mut PromptScopeState,
    index: usize,
    old_id: &FragmentId,
    new_id: &FragmentId,
) {
    state.layer.inline[index].id = new_id.clone();
    for ordered in &mut state.layer.order {
        if ordered == old_id {
            *ordered = new_id.clone();
        }
    }
    for disabled in &mut state.layer.disabled {
        if disabled == old_id {
            *disabled = new_id.clone();
        }
    }
    if let Some(file) = &mut state.file {
        file.transform_model_references(old_id, Some(new_id));
    }
}

fn remove_fragment_references(state: &mut PromptScopeState, id: &FragmentId) -> bool {
    let inline_before = state.layer.inline.len();
    let order_before = state.layer.order.len();
    let disabled_before = state.layer.disabled.len();
    state.layer.inline.retain(|fragment| fragment.id != *id);
    state.layer.order.retain(|fragment_id| fragment_id != id);
    state.layer.disabled.retain(|fragment_id| fragment_id != id);
    let layer_changed = state.layer.inline.len() != inline_before
        || state.layer.order.len() != order_before
        || state.layer.disabled.len() != disabled_before;
    let model_changed = state
        .file
        .as_mut()
        .is_some_and(|file| file.transform_model_references(id, None));
    layer_changed || model_changed
}

fn layer_name(scope: PromptScopeArg) -> &'static str {
    match scope {
        PromptScopeArg::Global => "global",
        PromptScopeArg::Workspace => "workspace",
        PromptScopeArg::Session => {
            unreachable!("session prompt layers are not settings-file layers")
        }
    }
}

fn requires_tools(value: Option<PromptRequiresToolsArg>) -> Option<bool> {
    match value {
        Some(PromptRequiresToolsArg::True) => Some(true),
        Some(PromptRequiresToolsArg::False) => Some(false),
        Some(PromptRequiresToolsArg::Inherit) | None => None,
    }
}

async fn read_body(body: Option<String>, file: Option<&Path>) -> Result<Option<String>, AppError> {
    match (body, file) {
        (Some(body), _) => Ok(Some(body)),
        (None, Some(file)) => read_file_or_stdin(file).await.map(Some),
        (None, None) => Ok(None),
    }
}

async fn read_file_or_stdin(path: &Path) -> Result<String, AppError> {
    use tokio::io::AsyncReadExt;

    if path == Path::new("-") {
        let mut text = String::new();
        tokio::io::stdin()
            .read_to_string(&mut text)
            .await
            .map_err(|error| AppError::Internal(format!("read prompt JSON from stdin: {error}")))?;
        return Ok(text);
    }
    tokio::fs::read_to_string(path).await.map_err(|error| {
        AppError::Internal(format!("read prompt file {}: {error}", path.display()))
    })
}

fn client_error(context: &str, error: agena_client::ClientError) -> AppError {
    AppError::Config(format!("{context}: {}", error.operator_diagnostic()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(value: &str) -> FragmentId {
        FragmentId::parse(value).unwrap()
    }

    fn target(scope: PromptScopeArg, session_id: Option<i64>) -> PromptTargetArgs {
        PromptTargetArgs { scope, session_id }
    }

    fn fragment(id: FragmentId, body: &str) -> PromptFragment {
        PromptFragment::new(id, body)
    }

    #[test]
    fn prompt_scope_requires_a_session_id_only_for_session_scope() {
        assert_eq!(
            validate_target(&target(PromptScopeArg::Global, None)).unwrap(),
            (PromptScopeArg::Global, None)
        );
        assert_eq!(
            validate_target(&target(PromptScopeArg::Workspace, None)).unwrap(),
            (PromptScopeArg::Workspace, None)
        );
        assert_eq!(
            validate_target(&target(PromptScopeArg::Session, Some(7))).unwrap(),
            (PromptScopeArg::Session, Some(7))
        );

        assert!(matches!(
            validate_target(&target(PromptScopeArg::Session, None)),
            Err(AppError::Config(message))
                if message == "--scope session requires --session-id <id>"
        ));
        assert!(matches!(
            validate_target(&target(PromptScopeArg::Global, Some(7))),
            Err(AppError::Config(message))
                if message == "--session-id is only valid with --scope session"
        ));
    }

    #[test]
    fn clear_uses_setting_deletion_for_file_scopes_and_an_empty_layer_for_sessions() {
        assert_eq!(
            clear_action(&target(PromptScopeArg::Global, None)).unwrap(),
            PromptClearAction::DeleteSetting {
                scope: PromptScopeArg::Global
            }
        );
        assert_eq!(
            clear_action(&target(PromptScopeArg::Workspace, None)).unwrap(),
            PromptClearAction::DeleteSetting {
                scope: PromptScopeArg::Workspace
            }
        );
        assert_eq!(
            clear_action(&target(PromptScopeArg::Session, Some(7))).unwrap(),
            PromptClearAction::SetSession {
                session_id: 7,
                layer: PromptLayer::default()
            }
        );
    }

    #[test]
    fn changing_the_layer_preserves_model_overlays_verbatim() {
        let models = vec![json!({
            "match": "anthropic/*",
            "order": ["base"],
            "custom": {"kept": true}
        })];
        let mut file = PromptScopeFile {
            models: models.clone(),
            ..PromptScopeFile::default()
        };
        let layer = PromptLayer {
            mode: LayerMode::Replace,
            order: vec![id("replacement")],
            disabled: vec![id("inherited")],
            inline: vec![fragment(id("replacement"), "replacement")],
        };

        file.set_layer(&layer);

        assert_eq!(file.models, models);
        assert_eq!(file.clone().into_layer(), layer);
        let output = PromptScopeState {
            scope: PromptScopeArg::Global,
            session_id: None,
            layer,
            file: Some(file),
            revision: Some("revision".to_owned()),
        }
        .output();
        assert_eq!(output["models"], json!(models));
    }

    #[test]
    fn output_uses_the_prompts_envelope_and_scope_fields() {
        let layer = PromptLayer {
            order: vec![id("base")],
            ..PromptLayer::default()
        };
        let output = PromptScopeState {
            scope: PromptScopeArg::Session,
            session_id: Some(7),
            layer: layer.clone(),
            file: None,
            revision: None,
        }
        .output();

        assert_eq!(output["scope"], "session");
        assert_eq!(output["session_id"], 7);
        assert_eq!(output["prompts"], serde_json::to_value(layer).unwrap());
        assert!(output.get("models").is_none());
    }

    #[test]
    fn renaming_an_inline_fragment_updates_scope_and_model_references() {
        let old = id("old");
        let renamed = id("new");
        let other = id("other");
        let old_name = old.to_string();
        let renamed_name = renamed.to_string();
        let other_name = other.to_string();
        let private_model = json!({
            "match": "private/*",
            "inline": [{"id": old_name, "body": "private fragment"}],
            "order": [old_name],
            "disabled": [old_name]
        });
        let inherited_model = json!({
            "match": "anthropic/*",
            "order": [old_name, other_name],
            "disabled": [old_name]
        });
        let mut state = PromptScopeState {
            scope: PromptScopeArg::Workspace,
            session_id: None,
            layer: PromptLayer {
                mode: LayerMode::Append,
                order: vec![old.clone(), other.clone()],
                disabled: vec![old.clone()],
                inline: vec![
                    fragment(old.clone(), "old fragment"),
                    fragment(other.clone(), "other fragment"),
                ],
            },
            file: Some(PromptScopeFile {
                models: vec![inherited_model, private_model.clone()],
                ..PromptScopeFile::default()
            }),
            revision: None,
        };

        rename_fragment_references(&mut state, 0, &old, &renamed);

        assert_eq!(state.layer.inline[0].id, renamed);
        assert_eq!(state.layer.order, vec![renamed.clone(), other.clone()]);
        assert_eq!(state.layer.disabled, vec![renamed]);
        let models = &state.file.as_ref().unwrap().models;
        assert_eq!(
            models[0],
            json!({
                "match": "anthropic/*",
                "order": [renamed_name, other_name],
                "disabled": [renamed_name]
            })
        );
        assert_eq!(models[1], private_model);
    }

    #[test]
    fn removing_a_fragment_also_removes_model_only_references() {
        let removed = id("removed");
        let kept = id("kept");
        let removed_name = removed.to_string();
        let kept_name = kept.to_string();
        let private_model = json!({
            "match": "private/*",
            "inline": [{"id": removed_name, "body": "private fragment"}],
            "order": [removed_name]
        });
        let inherited_model = json!({
            "match": "anthropic/*",
            "order": [removed_name, kept_name],
            "disabled": [removed_name]
        });
        let mut state = PromptScopeState {
            scope: PromptScopeArg::Workspace,
            session_id: None,
            layer: PromptLayer {
                order: vec![kept.clone()],
                ..PromptLayer::default()
            },
            file: Some(PromptScopeFile {
                models: vec![inherited_model, private_model.clone()],
                ..PromptScopeFile::default()
            }),
            revision: None,
        };

        assert!(remove_fragment_references(&mut state, &removed));
        assert_eq!(state.layer.order, vec![kept]);
        let models = &state.file.as_ref().unwrap().models;
        assert_eq!(
            models[0],
            json!({
                "match": "anthropic/*",
                "order": [kept_name],
                "disabled": []
            })
        );
        assert_eq!(models[1], private_model);
        assert!(!remove_fragment_references(&mut state, &removed));
    }
}
