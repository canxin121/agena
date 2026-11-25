//! Cua Driver owns the platform operations. Agena authorizes exact windows and
//! uses a private MCP connection for each invocation.

use std::{io::Cursor, path::PathBuf, process::Stdio, time::Duration};

use agena_plugin_sdk::{
    CancellationToken, PluginError, PluginErrorKind, Result, ToolInvokeOutput,
    attachment::{AttachmentItem, AttachmentKind, AttachmentSource},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader};
use rmcp::{
    RoleClient, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ClientCapabilities, ClientInfo, ContentBlock,
        ImageContent, Implementation, JsonObject, ProtocolVersion,
    },
    service::RunningService,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{io::AsyncReadExt, process::Command, sync::Semaphore, time::timeout};

use super::{Action, DesktopSettings, Modifier, Window};

#[cfg(test)]
mod tests;

const MAX_WINDOWS: usize = 64;
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 1400;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
static DISPLAY_GATE: Semaphore = Semaphore::const_new(1);

pub(super) async fn request<T>(
    cancellation: &CancellationToken,
    operation: impl AsyncFnOnce(&Desktop) -> Result<T>,
) -> Result<T> {
    let _permit = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Err(cancelled()),
        permit = DISPLAY_GATE.acquire() => permit.map_err(PluginError::internal)?,
    };
    let executable = driver_executable()?;
    #[cfg(target_os = "linux")]
    check_linux_display()?;
    let mut command = Command::new(&executable);
    command
        .arg("mcp")
        .arg("--direct")
        .current_dir(
            executable
                .parent()
                .ok_or_else(|| unavailable("Invalid Cua Driver path."))?,
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .env("DO_NOT_TRACK", "1");
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "SystemRoot",
        "WINDIR",
        "TMP",
        "TEMP",
        "TMPDIR",
        "DISPLAY",
        "XAUTHORITY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "XDG_SESSION_TYPE",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_DIRS",
        "DBUS_SESSION_BUS_ADDRESS",
        "AT_SPI_BUS_ADDRESS",
        "SWAYSOCK",
        "HYPRLAND_INSTANCE_SIGNATURE",
        "NIRI_SOCKET",
        "LANG",
        "LC_ALL",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(target_os = "linux")]
    command.env("CUA_DRIVER_RS_ENABLE_WAYLAND", "1");

    let mut child = agena_process::spawn(command)
        .map_err(|error| unavailable(format!("Could not start Cua Driver ({error}).")))?;
    let stdin = child
        .stdin()
        .take()
        .ok_or_else(|| unavailable("Cua Driver stdin is unavailable."))?;
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| unavailable("Cua Driver stdout is unavailable."))?;
    let client = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("agena.desktop", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2025_06_18);
    let result = tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(cancelled()),
        result = timeout(REQUEST_TIMEOUT, async {
            let peer = client.serve((stdout.take(MAX_RESPONSE_BYTES), stdin)).await
                .map_err(|error| unavailable(format!("Could not connect to Cua Driver ({error}).")))?;
            let server = peer.peer_info();
            if !server.as_deref().and_then(|info| info.server_info.as_ref())
                .is_some_and(|info| info.name == "cua-driver")
            {
                return Err(unavailable("The desktop executable must identify itself as Cua Driver."));
            }
            let mut desktop = Desktop { peer };
            let result = operation(&desktop).await;
            let _ = desktop.peer.close_with_timeout(Duration::from_millis(500)).await;
            result
        }) => result.unwrap_or_else(|_| Err(unavailable("Cua Driver did not respond in time."))),
    };
    if result.is_ok() {
        // Closing MCP stdin lets the driver clean up its native session. The
        // managed child terminates the process tree if it fails to exit.
        let _ = timeout(Duration::from_millis(500), child.wait()).await;
    }
    result
}

fn driver_executable() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("AGENA_DESKTOP_DRIVER") {
        let path = PathBuf::from(path);
        if !path.is_absolute() || !path.is_file() {
            return Err(unavailable(
                "AGENA_DESKTOP_DRIVER must be an absolute path to the Cua Driver executable.",
            ));
        }
        return Ok(path);
    }
    which::which("cua-driver").map_err(|_| unavailable(
        "Install Cua Driver and add cua-driver to the Agena service PATH, or set AGENA_DESKTOP_DRIVER to its absolute path.",
    ))
}

#[cfg(target_os = "linux")]
fn check_linux_display() -> Result<()> {
    if let Some(display) = std::env::var_os("DISPLAY") {
        let display = display.to_string_lossy();
        let local = display
            .strip_prefix(':')
            .or_else(|| display.strip_prefix("unix:"));
        if !display.is_empty() && local.is_none() {
            return Err(unavailable(
                "Desktop control requires a local X11 or Wayland session.",
            ));
        }
        if local.is_some_and(|address| {
            let (display, screen) = address.split_once('.').unwrap_or((address, "0"));
            [display, screen]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        }) {
            return Ok(());
        }
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|name| !name.is_empty()) {
        return Ok(());
    }
    Err(unavailable(
        "Run Agena in the local Linux graphical session with DISPLAY or WAYLAND_DISPLAY set.",
    ))
}

pub(super) struct Desktop {
    peer: RunningService<RoleClient, ClientInfo>,
}

impl Desktop {
    async fn call(&self, name: &'static str, args: Value) -> Result<CallToolResult> {
        let arguments: JsonObject = serde_json::from_value(args).map_err(PluginError::internal)?;
        let response = self
            .peer
            .call_tool(CallToolRequestParams::new(name).with_arguments(arguments))
            .await
            .map_err(|error| unavailable(format!("Cua Driver {name} failed ({error}).")))?;
        if response.is_error == Some(true) {
            return Err(driver_refusal(name, &response));
        }
        Ok(response)
    }

    async fn window_list(&self) -> Result<ListedWindows> {
        structured(
            self.call("list_windows", json!({"on_screen_only":true}))
                .await?,
        )
    }

    pub(super) async fn windows(&self, settings: &DesktopSettings) -> Result<Vec<Window>> {
        Ok(self
            .window_list()
            .await?
            .windows
            .into_iter()
            .filter_map(|window| window.permitted(settings))
            .take(MAX_WINDOWS)
            .collect())
    }

    async fn live_window(&self, settings: &DesktopSettings, target: &Window) -> Result<Window> {
        self.window_list()
            .await?
            .windows
            .into_iter()
            .filter_map(|window| window.permitted(settings))
            .find(|window| target.same_target(window))
            .ok_or_else(|| {
                PluginError::invalid_params(
                    "The selected window changed or closed. Run desktop.windows again.",
                )
            })
    }

    async fn capture(&self, window: &Window) -> Result<WindowImage> {
        let response = self
            .call(
                "get_window_state",
                json!({
                    "pid":window.pid, "window_id":window.id,
                    "include_accessibility_tree":false, "include_screenshot":true,
                    "max_image_dimension":MAX_IMAGE_EDGE,
                }),
            )
            .await?;
        window_image(window, response)
    }

    pub(super) async fn screenshot(
        &self,
        settings: &DesktopSettings,
        target: &Window,
    ) -> Result<ToolInvokeOutput> {
        let window = self.live_window(settings, target).await?;
        let image = self.capture(&window).await?;
        let payload = json!({
            "image_width":image.width, "image_height":image.height,
            "window_width":window.width, "window_height":window.height,
        });
        Ok(ToolInvokeOutput::from_parts(
            "Desktop screenshot",
            format!("Captured {}", window.application),
            payload.to_string(),
            Some(payload),
            Default::default(),
            vec![image.attachment()],
        ))
    }

    pub(super) async fn action(
        &self,
        settings: &DesktopSettings,
        target: &Window,
        action: Action,
    ) -> Result<ToolInvokeOutput> {
        let window = self.live_window(settings, target).await?;
        let image = match &action {
            Action::Click { x, y, .. } | Action::Scroll { x, y, .. } => {
                if *x >= window.width || *y >= window.height {
                    return Err(PluginError::invalid_params(
                        "Coordinates are outside the selected window.",
                    ));
                }
                Some(self.capture(&window).await?)
            }
            _ => None,
        };
        let (name, args) = action_request(&window, image.as_ref(), action)?;
        let response = self.call(name, args).await?;
        let effect = response
            .structured_content
            .as_ref()
            .and_then(|value| value["effect"].as_str());
        let summary = match effect {
            Some("confirmed") => "Desktop action confirmed.",
            Some("partial") => "Desktop action partially completed.",
            Some("unverifiable") | None => {
                "Desktop action sent. Verify the window state before repeating it."
            }
            _ => return Err(driver_refusal(name, &response)),
        };
        let payload = json!({"effect":effect.unwrap_or("unverifiable")});
        Ok(ToolInvokeOutput::from_parts(
            "Desktop action",
            summary,
            payload.to_string(),
            Some(payload),
            Default::default(),
            Vec::new(),
        ))
    }
}

#[derive(Deserialize)]
struct ListedWindows {
    windows: Vec<DriverWindow>,
}

#[derive(Deserialize)]
struct DriverWindow {
    window_id: u64,
    pid: Option<u32>,
    app_name: String,
    title: Option<String>,
    bounds: Bounds,
    is_on_screen: bool,
}

#[derive(Deserialize)]
struct Bounds {
    width: f64,
    height: f64,
}

impl Bounds {
    fn dimensions(&self) -> Option<(u16, u16)> {
        let dimension = |value: f64| {
            let rounded = value.round();
            (1.0..=f64::from(u16::MAX))
                .contains(&rounded)
                .then_some(rounded as u16)
        };
        Some((dimension(self.width)?, dimension(self.height)?))
    }
}

impl DriverWindow {
    fn permitted(self, settings: &DesktopSettings) -> Option<Window> {
        let pid = self.pid.filter(|&pid| pid != 0)?;
        if self.window_id == 0
            || !self.is_on_screen
            || self.app_name.is_empty()
            || !settings.allowed_applications.contains(&self.app_name)
        {
            return None;
        }
        let (width, height) = self.bounds.dimensions()?;
        Some(Window {
            id: self.window_id,
            pid,
            application: self.app_name,
            title: self
                .title
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .take(256)
                .collect(),
            width,
            height,
        })
    }
}

fn structured<T: serde::de::DeserializeOwned>(response: CallToolResult) -> Result<T> {
    let content = response
        .structured_content
        .ok_or_else(|| unavailable("Cua Driver returned no structured data."))?;
    serde_json::from_value(content).map_err(|error| {
        unavailable(format!(
            "Cua Driver returned invalid desktop data ({error})."
        ))
    })
}

struct WindowImage {
    content: ImageContent,
    width: u32,
    height: u32,
    bytes: usize,
    extension: &'static str,
}

impl WindowImage {
    fn attachment(self) -> AttachmentItem {
        AttachmentItem {
            kind: AttachmentKind::Image,
            mime: self.content.mime_type,
            source: AttachmentSource::Base64 {
                data: self.content.data,
            },
            filename: Some(format!("desktop.{}", self.extension)),
            title: Some("Desktop screenshot".into()),
            size_bytes: Some(self.bytes as u64),
            sha256: None,
            width: Some(self.width),
            height: Some(self.height),
            duration_ms: None,
            page_count: None,
        }
    }
}

fn window_image(window: &Window, response: CallToolResult) -> Result<WindowImage> {
    let state = response
        .structured_content
        .as_ref()
        .ok_or_else(|| unavailable("Cua Driver returned no window state."))?;
    if state["window_id"].as_u64() != Some(window.id)
        || state["pid"].as_u64() != Some(u64::from(window.pid))
        || state["coordinate_frame"].as_str() == Some("desktop")
        || state["screenshot_frame_valid"].as_bool() == Some(false)
    {
        return Err(unavailable(
            "Cua Driver did not verify the selected window in the screenshot.",
        ));
    }
    if let Some(bounds) = state
        .get("window_bounds")
        .filter(|bounds| bounds.is_object())
    {
        let bounds: Bounds = serde_json::from_value(bounds.clone())
            .map_err(|_| unavailable("Cua Driver returned invalid window bounds."))?;
        if bounds.dimensions() != Some((window.width, window.height)) {
            return Err(PluginError::invalid_params(
                "The selected window was resized during capture. Take a new screenshot.",
            ));
        }
    }
    let mut images = response
        .content
        .into_iter()
        .filter_map(|content| match content {
            ContentBlock::Image(image) => Some(image),
            _ => None,
        });
    let content = images.next().ok_or_else(|| unavailable("Cua Driver could not capture the selected window. Check the desktop session and screen capture permissions."))?;
    if images.next().is_some() {
        return Err(unavailable(
            "Cua Driver returned more than one image for the selected window.",
        ));
    }
    let (format, extension) = match content.mime_type.as_str() {
        "image/png" => (ImageFormat::Png, "png"),
        "image/jpeg" => (ImageFormat::Jpeg, "jpg"),
        _ => {
            return Err(unavailable(
                "Cua Driver returned an unsupported screenshot format.",
            ));
        }
    };
    let bytes = STANDARD
        .decode(&content.data)
        .map_err(|_| unavailable("Cua Driver returned invalid screenshot data."))?;
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|_| unavailable("Cua Driver returned an unreadable screenshot."))?;
    if !(1..=MAX_IMAGE_EDGE).contains(&width) || !(1..=MAX_IMAGE_EDGE).contains(&height) {
        return Err(unavailable(
            "Cua Driver returned a screenshot outside the requested size limit.",
        ));
    }
    Ok(WindowImage {
        content,
        width,
        height,
        bytes: bytes.len(),
        extension,
    })
}

fn action_request(
    window: &Window,
    image: Option<&WindowImage>,
    action: Action,
) -> Result<(&'static str, Value)> {
    let mut args = json!({"pid":window.pid, "window_id":window.id, "scope":"window", "delivery_mode":"foreground"});
    let mut set_point = |x: u16, y: u16| -> Result<()> {
        let image = image.ok_or_else(|| {
            unavailable("The selected window has no current screenshot coordinates.")
        })?;
        args["x"] = json!(u32::from(x) * image.width / u32::from(window.width));
        args["y"] = json!(u32::from(y) * image.height / u32::from(window.height));
        args["coordinate_frame"] = json!("window");
        Ok(())
    };
    let name = match action {
        Action::Click { x, y, button } => {
            set_point(x, y)?;
            args["button"] = json!(button);
            "click"
        }
        Action::Scroll {
            x,
            y,
            direction,
            steps,
        } => {
            if !(1..=10).contains(&steps) {
                return Err(PluginError::invalid_params(
                    "Scroll steps must be between 1 and 10.",
                ));
            }
            set_point(x, y)?;
            args["direction"] = json!(direction);
            args["amount"] = json!(steps);
            args["by"] = json!("line");
            "scroll"
        }
        Action::Type { text } => {
            if text.is_empty() || text.chars().count() > 256 || text.chars().any(char::is_control) {
                return Err(PluginError::invalid_params(
                    "Type requires 1–256 printable characters.",
                ));
            }
            args["text"] = json!(text);
            "type_text"
        }
        Action::Key { key, modifiers } => {
            if key.is_empty()
                || key.len() > 32
                || key.chars().any(char::is_control)
                || modifiers.len() > 3
            {
                return Err(PluginError::invalid_params(
                    "Supply a key name with up to three modifiers.",
                ));
            }
            args["key"] = json!(key);
            args["modifiers"] = json!(modifiers.into_iter().map(modifier).collect::<Vec<_>>());
            "press_key"
        }
    };
    Ok((name, args))
}

fn modifier(modifier: Modifier) -> &'static str {
    match modifier {
        Modifier::Control => "ctrl",
        Modifier::Shift => "shift",
        Modifier::Alt => "alt",
        Modifier::Meta if cfg!(target_os = "macos") => "cmd",
        Modifier::Meta if cfg!(target_os = "windows") => "win",
        Modifier::Meta => "super",
    }
}

fn driver_refusal(name: &str, response: &CallToolResult) -> PluginError {
    let detail: String = response
        .structured_content
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/code")
                .or_else(|| value.get("code"))
                .and_then(Value::as_str)
        })
        .unwrap_or("refused")
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .take(80)
        .collect();
    unavailable(format!(
        "Cua Driver could not complete {name} ({detail}). Check the selected window and desktop permissions."
    ))
}

fn cancelled() -> PluginError {
    PluginError::from_kind(PluginErrorKind::Cancelled, "Desktop invocation cancelled.")
}

fn unavailable(message: impl Into<String>) -> PluginError {
    let message = message.into();
    PluginError::from_kind_with_public_detail(
        PluginErrorKind::CapabilityUnavailable,
        message.clone(),
        message,
    )
}
