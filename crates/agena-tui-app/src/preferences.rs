/// TUI-local preferences decoded from the generic settings document.
#[derive(Debug, Clone, Default)]
pub struct TuiPreferencesResource {
    pub locale: Option<String>,
    pub theme: Option<String>,
    pub color_scheme: TuiColorSchemeResource,
    pub graphics: TuiGraphicsModeResource,
    /// Default transcript expansion for activities without a kind override.
    pub transcript_activity_default_expanded: bool,
    /// Per-kind transcript expansion overrides keyed by activity kind id.
    pub transcript_activity_kinds: std::collections::BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Copy, Default)]
/// Color scheme preference for the TUI.
pub enum TuiColorSchemeResource {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default)]
/// Graphics mode preference for the TUI.
pub enum TuiGraphicsModeResource {
    #[default]
    Auto,
    Native,
    Unicode,
}
