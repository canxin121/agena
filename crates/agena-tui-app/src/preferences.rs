/// TUI-local preferences decoded from the generic settings document.
#[derive(Debug, Clone, Default)]
pub struct TuiPreferencesResource {
    pub locale: Option<String>,
    pub theme: Option<String>,
    pub color_scheme: TuiColorSchemeResource,
    pub graphics: TuiGraphicsModeResource,
    /// Read-only rendering projection of the shared `ui.transcript` settings.
    pub transcript_detail_defaults: agena_tui_transcript::TranscriptDetailDefaults,
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
