use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use agena_plugin_sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginError, PluginErrorKind,
    PluginManifest, PluginManifestTranslation, PluginSettings, Result, ToolContract,
    ToolDefinition, ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolTag, TransportKind,
    host_api::HostClient, macro_support,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

mod driver;

pub(crate) const DESKTOP_PLUGIN_ID: &str = "agena.desktop";
const WINDOW_HANDLE_LIFETIME: Duration = Duration::from_secs(15 * 60);
const MAX_TRACKED_SESSIONS: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(default, deny_unknown_fields)]
struct DesktopSettings {
    /// Exact application names as reported by the local Cua Driver.
    #[schemars(length(max = 32))]
    allowed_applications: Vec<String>,
    /// Enable clicks, scrolling and keyboard input in permitted windows.
    allow_input: bool,
}

#[derive(Default)]
pub(crate) struct DesktopPlugin {
    settings: PluginSettings<DesktopSettings>,
    handles: Arc<Mutex<HashMap<i64, WindowHandles>>>,
}

/// Window handles belong to the Agena runtime across plugin host rebuilds.
#[derive(Clone, Default)]
pub struct DesktopWindowPool(Arc<Mutex<HashMap<i64, WindowHandles>>>);

impl DesktopWindowPool {
    pub(crate) fn plugin(&self) -> DesktopPlugin {
        DesktopPlugin {
            settings: PluginSettings::default(),
            handles: self.0.clone(),
        }
    }
}

struct WindowHandles {
    expires: Instant,
    windows: HashMap<Uuid, Window>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Window {
    #[serde(skip)]
    id: u64,
    #[serde(skip)]
    pid: u32,
    application: String,
    title: String,
    width: u16,
    height: u16,
}

impl Window {
    fn same_target(&self, other: &Self) -> bool {
        self.id == other.id && self.pid == other.pid && self.application == other.application
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListInput {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WindowInput {
    /// An ID returned by desktop.windows in this conversation.
    window_id: String,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ActionInput {
    /// An ID returned by desktop.windows in this conversation.
    window_id: String,
    /// Input operation selected by its `kind` field.
    action: Action,
}

#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Click {
        /// Horizontal coordinate in the live window, starting at its left edge.
        x: u16,
        /// Vertical coordinate in the live window, starting at its top edge.
        y: u16,
        /// Mouse button to click.
        #[serde(default)]
        button: MouseButton,
    },
    Scroll {
        /// Horizontal coordinate in the live window, starting at its left edge.
        x: u16,
        /// Vertical coordinate in the live window, starting at its top edge.
        y: u16,
        /// Direction of the scroll.
        direction: ScrollDirection,
        /// Number of scroll steps at that location.
        #[schemars(range(min = 1, max = 10))]
        steps: u8,
    },
    Type {
        /// Text to enter in the target window.
        #[schemars(length(min = 1, max = 256))]
        text: String,
    },
    Key {
        /// Printable key or supported named key.
        key: String,
        /// Modifier keys to hold when pressing `key`.
        #[serde(default)]
        #[schemars(length(max = 3))]
        modifiers: Vec<Modifier>,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum MouseButton {
    #[default]
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Modifier {
    Control,
    Shift,
    Alt,
    /// Command on macOS, Windows key on Windows, and Super on Linux.
    Meta,
}

fn definition<T: JsonSchema>(
    name: &str,
    summary: &str,
    help: &str,
    effect: ToolTag,
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        contract: ToolContract {
            input_schema: macro_support::json_schema_for::<T>(),
            ..Default::default()
        },
        docs: ToolDocs {
            summary: Some(summary.into()),
            help: Some(help.into()),
            ..Default::default()
        },
        runtime: Default::default(),
        tags: vec![ToolTag::Interactive, effect],
        applies_to: Default::default(),
    }
}

#[async_trait::async_trait]
impl Plugin for DesktopPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "desktop", env!("CARGO_PKG_VERSION"));
        manifest.display_name = Some("Desktop".into());
        manifest.icon = Some(crate::plugin_metadata::icon(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/plugin-icons/desktop.svg"
        ))));
        manifest.authors = vec!["canxin121".into()];
        manifest.tags = vec![ToolTag::Interactive];
        manifest.summary = Some(
            "Inspect and control allowed application windows on Windows, Linux, and macOS with Cua Driver.".into(),
        );
        manifest.help = Some(
            "Install Cua Driver on the machine running Agena. Put `cua-driver` on the service's PATH or set `AGENA_DESKTOP_DRIVER` to its absolute path. Set `allowed_applications` to exact application names and enable `allow_input` to send mouse or keyboard events. Call `desktop.windows` to obtain a window ID for the current conversation.".into(),
        );
        manifest.translations.insert(
            "zh-CN".into(),
            PluginManifestTranslation {
                summary: Some(
                    "通过 Cua Driver 查看和操作本机 Windows、Linux 和 macOS 上获准访问的应用窗口。"
                        .into(),
                ),
                help: Some(
                    "在运行 Agena 的机器上安装 Cua Driver，并把 `cua-driver` 加到服务的 PATH，或将 `AGENA_DESKTOP_DRIVER` 设为可执行文件的绝对路径。在 `allowed_applications` 填写准确的应用名称。需要发送鼠标或键盘事件时启用 `allow_input`，通过 `desktop.windows` 获取当前会话的窗口 ID。".into(),
                ),
                ..Default::default()
            },
        );
        manifest.transports = vec![TransportKind::Static];
        manifest.hooks = HookSubscription::INIT | HookSubscription::TOOL_INVOKE;
        manifest.settings = Some(macro_support::settings_contract_for_default(
            DesktopSettings::default(),
        ));
        manifest.tools = vec![
            definition::<ListInput>(
                "windows",
                "List permitted desktop application windows.",
                "Requires a local Cua Driver and an operator-configured allowed_applications list. Use the returned window IDs within this conversation.",
                ToolTag::ReadOnly,
            ),
            definition::<WindowInput>(
                "screenshot",
                "Capture a permitted desktop window.",
                "Use an ID returned by desktop.windows. The image reports its dimensions and the window dimensions. Scale image coordinates to window coordinates before clicking.",
                ToolTag::ReadOnly,
            ),
            definition::<ActionInput>(
                "action",
                "Click, scroll, type text, or press a key in a permitted desktop window.",
                "Requires allow_input. Click and scroll coordinates are relative to the top-left of the live window. Cua Driver targets the selected window and may briefly bring it to the foreground. Supported named keys include Enter, Tab, Escape, Backspace, Delete, Home, End, PageUp, PageDown, Left, Right, Up, Down, and F1 through F12.",
                ToolTag::Mutate,
            ),
        ];
        manifest
    }

    async fn init(&self, ctx: InitContext, _: Arc<dyn HostClient>) -> Result<InitOutcome> {
        self.settings.set_from_json(
            ctx.settings,
            "invalid desktop settings",
            "desktop already initialized",
        )?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let settings = self.settings_for(&input)?;
        let session = input.session_id;
        let cancellation = input.context().cancellation;
        match input.tool_name.as_str() {
            "windows" => {
                parse_input::<ListInput>(input.input)?;
                let windows = driver::request(&cancellation, async |driver| {
                    driver.windows(&settings).await
                })
                .await?;
                self.remember_windows(session, windows)
            }
            "screenshot" => {
                let input = parse_input::<WindowInput>(input.input)?;
                let target = self.resolve_window(session, &input.window_id)?;
                driver::request(&cancellation, async |driver| {
                    driver.screenshot(&settings, &target).await
                })
                .await
            }
            "action" => {
                if !settings.allow_input {
                    return Err(denied(
                        "desktop input requires allow_input in plugin settings",
                    ));
                }
                let input = parse_input::<ActionInput>(input.input)?;
                let target = self.resolve_window(session, &input.window_id)?;
                driver::request(&cancellation, async |driver| {
                    driver.action(&settings, &target, input.action).await
                })
                .await
            }
            _ => Err(PluginError::not_implemented(&input.tool_name)),
        }
    }
}

impl DesktopPlugin {
    fn settings_for(&self, input: &ToolInvokeInput) -> Result<DesktopSettings> {
        if input.caller.is_some() || input.session_id <= 0 {
            return Err(denied("desktop tools require a direct Agena conversation"));
        }
        let settings = self
            .settings
            .get()
            .ok_or_else(|| PluginError::internal("desktop is not initialized"))?;
        if settings.allowed_applications.is_empty() {
            return Err(denied(
                "set allowed_applications in the Desktop plugin settings to grant access",
            ));
        }
        Ok(settings.clone())
    }

    fn remember_windows(&self, session: i64, windows: Vec<Window>) -> Result<ToolInvokeOutput> {
        #[derive(Serialize)]
        struct ListedWindow<'a> {
            window_id: Uuid,
            #[serde(flatten)]
            window: &'a Window,
        }

        let mut handles = self.handles.lock().map_err(PluginError::internal)?;
        let now = Instant::now();
        handles.retain(|_, state| state.expires > now);
        if handles.len() >= MAX_TRACKED_SESSIONS && !handles.contains_key(&session) {
            if let Some(oldest) = handles
                .iter()
                .min_by_key(|(_, state)| state.expires)
                .map(|(id, _)| *id)
            {
                handles.remove(&oldest);
            }
        }
        let mut previous = handles
            .remove(&session)
            .map(|state| state.windows)
            .unwrap_or_default();
        let mut current = HashMap::new();
        let mut listing = Vec::with_capacity(windows.len());
        for window in windows {
            let known = previous
                .iter()
                .find(|(_, saved)| saved.same_target(&window))
                .map(|(id, _)| *id);
            let id = known.unwrap_or_else(Uuid::new_v4);
            previous.remove(&id);
            listing.push(id);
            current.insert(id, window);
        }
        let listed = listing
            .into_iter()
            .map(|id| ListedWindow {
                window_id: id,
                window: &current[&id],
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({ "windows": listed });
        let summary = format!("{} permitted desktop windows", listed.len());
        handles.insert(
            session,
            WindowHandles {
                expires: now + WINDOW_HANDLE_LIFETIME,
                windows: current,
            },
        );
        Ok(ToolInvokeOutput::from_parts(
            "Desktop windows",
            summary,
            payload.to_string(),
            Some(payload),
            Default::default(),
            Vec::new(),
        ))
    }

    fn resolve_window(&self, session: i64, id: &str) -> Result<Window> {
        let id = Uuid::parse_str(id)
            .map_err(|_| PluginError::invalid_params("use a window ID from desktop.windows"))?;
        self.handles
            .lock()
            .map_err(PluginError::internal)?
            .get(&session)
            .filter(|state| state.expires > Instant::now())
            .and_then(|state| state.windows.get(&id))
            .cloned()
            .ok_or_else(|| {
                PluginError::invalid_params(
                    "window ID expired or belongs to another conversation; run desktop.windows",
                )
            })
    }
}

fn parse_input<T: DeserializeOwned>(value: serde_json::Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| PluginError::invalid_params_error(&error))
}

fn denied(message: &str) -> PluginError {
    PluginError::from_kind_with_public_detail(PluginErrorKind::PolicyDenied, message, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocated_window_handles_are_conversation_local_and_expire() {
        let windows = DesktopWindowPool::default();
        let desktop = windows.plugin();
        let window = Window {
            id: 12,
            pid: 4,
            application: "Fixture".into(),
            title: "Fixture window".into(),
            width: 80,
            height: 80,
        };
        let list = desktop.remember_windows(1, vec![window.clone()]).unwrap();
        let id = list.payload.as_ref().unwrap()["windows"][0]["window_id"]
            .as_str()
            .unwrap();
        assert_eq!(desktop.resolve_window(1, id).unwrap(), window);
        let rebuilt = windows.plugin();
        assert_eq!(rebuilt.resolve_window(1, id).unwrap(), window);
        assert!(rebuilt.resolve_window(2, id).is_err());
        desktop.handles.lock().unwrap().get_mut(&1).unwrap().expires = Instant::now();
        assert!(desktop.resolve_window(1, id).is_err());
    }

    #[tokio::test]
    async fn disabled_and_channel_calls_do_not_open_a_desktop_connection() {
        let desktop = DesktopPlugin::default();
        desktop
            .settings
            .set(DesktopSettings::default(), "test")
            .unwrap();
        let mut input = ToolInvokeInput {
            tool_name: "windows".into(),
            session_id: 1,
            call_id: 10,
            workspace_root: String::new(),
            caller: None,
            input: serde_json::json!({}),
        };
        assert_eq!(
            desktop.tool_invoke(input.clone()).await.unwrap_err().kind,
            PluginErrorKind::PolicyDenied
        );
        input.caller = Some(Default::default());
        assert_eq!(
            desktop.tool_invoke(input).await.unwrap_err().kind,
            PluginErrorKind::PolicyDenied
        );
    }
}
