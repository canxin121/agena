use crate::ui_text;
use agena_tui::i18n::I18n;

#[test]
fn visible_shortcut_hints_track_the_central_keymap() {
    let english = I18n::english();
    let transcript = ui_text::t(&english, "status-transcript");
    let composer = ui_text::t(&english, "status-composer");
    let composer_help = ui_text::t(&english, "help-composer-line-10");
    let global = ui_text::t(&english, "status-global");
    let help_hint = ui_text::t(&english, "context-help-global-hint");
    let provider_footer = ui_text::t(&english, "overlay-provider-studio-footer");
    let provider_model_footer = ui_text::t(&english, "overlay-provider-studio-model-footer");
    let model_catalog_footer = ui_text::t(&english, "overlay-model-catalog-footer");
    let permission_footer = ui_text::t(&english, "overlay-permission-studio-footer-nested");
    let permission_rule_footer = ui_text::t(&english, "overlay-permission-rule-studio-footer");

    assert!(transcript.contains("i insert"));
    assert!(composer.contains("Esc view"));
    assert!(!composer.contains("Ctrl+Up"));
    assert!(!composer.contains("edit pending"));
    assert!(composer.contains("Up at start history"));
    assert!(!composer.contains("Ctrl+R/Alt+Up history"));
    assert!(composer.contains("Ctrl+G items"));
    assert!(composer.contains("Ctrl+R input"));
    assert!(composer.contains("Ctrl+L approval"));
    // The pending-message edit shortcut is documented only in the Ctrl+H
    // help window, not in the always-visible status line.
    assert!(composer_help.contains("Ctrl+P edits the pending message"));
    assert!(composer_help.contains("Ctrl+X cancels"));
    for removed in ["F2", "Alt+U", "Alt+A", "F3", "F4", "F6"] {
        assert!(
            !composer.contains(removed),
            "composer still references {removed}"
        );
    }
    assert!(global.contains("Tab/Shift+Tab"));
    assert!(!global.contains("Alt+Tab"));
    assert!(help_hint.contains("Ctrl+H"));
    for removed in ["Alt+S", "Alt+P", "q quit"] {
        assert!(!global.contains(removed));
    }
    for shortcut in ["Ctrl+D", "Ctrl+R", "Ctrl+N", "Ctrl+A", "Ctrl+S"] {
        assert!(provider_footer.contains(shortcut));
    }
    assert!(provider_footer.contains("Tab/Shift+Tab"));
    for removed in ["Delete", "Alt+", "F2", "F5", "Ctrl+X", "Ctrl+K"] {
        assert!(!provider_footer.contains(removed));
    }
    assert!(provider_model_footer.contains("Ctrl+S save"));
    assert!(provider_model_footer.contains("Ctrl+D remove"));
    assert!(!provider_model_footer.contains("field or action"));
    assert!(!provider_footer.contains("Enter edits or activates"));
    for shortcut in ["Ctrl+F", "Ctrl+R", "Left", "Right"] {
        assert!(model_catalog_footer.contains(shortcut));
    }
    assert!(!model_catalog_footer.contains("visible actions"));
    for shortcut in ["Ctrl+N", "Enter", "Ctrl+E", "Ctrl+D"] {
        assert!(permission_footer.contains(shortcut));
    }
    assert!(!permission_footer.contains("duplicate"));
    assert!(!permission_footer.contains("Delete"));
    assert!(!permission_footer.contains("F2"));
    assert!(!permission_footer.contains("Alt+"));
    assert!(!permission_footer.contains("action bar"));
    for shortcut in ["Enter", "Ctrl+O", "Ctrl+S", "Ctrl+D"] {
        assert!(permission_rule_footer.contains(shortcut));
    }
    assert!(!permission_rule_footer.contains("field or action"));
}
