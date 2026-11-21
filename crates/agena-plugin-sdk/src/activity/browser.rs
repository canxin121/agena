//! Operator transport for live browser activities.

use std::net::SocketAddr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

#[async_trait]
pub trait BrowserNetworkAuthority: Send + Sync + 'static {
    async fn require(&self, target: &str, addresses: &[SocketAddr]) -> Result<()>;
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserControlCommand {
    List,
    Frame {
        page_id: String,
        control_id: Option<String>,
    },
    TakeControl {
        page_id: String,
    },
    Input {
        page_id: String,
        control_id: String,
        events: Vec<BrowserInput>,
    },
    ReturnControl {
        control_id: String,
        page_id: Option<String>,
    },
    Close {
        control_id: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrowserInput {
    Pointer {
        action: PointerAction,
        x: f64,
        y: f64,
        button: PointerButton,
        buttons: u8,
        clicks: u8,
        modifiers: u8,
    },
    Wheel {
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        modifiers: u8,
    },
    Key {
        down: bool,
        key: String,
        code: String,
        key_code: u32,
        modifiers: u8,
        repeat: bool,
    },
    Text {
        text: String,
    },
    Dialog {
        accept: bool,
        text: Option<String>,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerAction {
    Down,
    Up,
    Move,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    None,
    Left,
    Middle,
    Right,
}

#[derive(Serialize)]
pub struct BrowserTab {
    pub page_id: String,
    pub opener_page_id: Option<String>,
    pub title: String,
}

#[derive(Serialize)]
pub struct BrowserHelpRequest {
    pub page_id: String,
    pub reason: String,
    pub ready_to_resume: bool,
}

#[derive(Clone, Serialize)]
pub struct BrowserDialog {
    pub kind: String,
    pub message: String,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrowserControlResult {
    State {
        tabs: Vec<BrowserTab>,
        human_control: bool,
        help: Option<BrowserHelpRequest>,
    },
    Frame {
        jpeg: String,
        origin: String,
        width: u32,
        height: u32,
        dialog: Option<BrowserDialog>,
    },
    Control {
        control_id: String,
    },
    Done,
}
