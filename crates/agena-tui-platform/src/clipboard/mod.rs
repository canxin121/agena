//! Clipboard read/write integration.

mod image;
mod path;
mod text;

pub use image::{
    ClipboardFilesError, PastedImageInfo, clipboard_file_list, paste_image_to_temp_png,
    pasted_image_format,
};
pub use path::normalize_pasted_path;
pub use text::{ClipboardCopyMethod, ClipboardTextError, get_clipboard_text, set_clipboard_text};
