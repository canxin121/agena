#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg_attr(feature = "cef", tauri::cef_entry_point)]
fn main() {
    agena_studio_desktop_cef::run()
}
