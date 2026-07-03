use std::fs;
use std::io;
use std::path::Path;

fn prune_appledouble(path: &Path) -> io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let file_type = entry.file_type()?;
        let entry_path = entry.path();

        if file_name.starts_with("._") {
            if file_type.is_dir() {
                fs::remove_dir_all(&entry_path)?;
            } else {
                fs::remove_file(&entry_path)?;
            }
            continue;
        }

        if file_type.is_dir() && file_name != "target" {
            prune_appledouble(&entry_path)?;
        }
    }

    Ok(())
}

fn main() {
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set");
    prune_appledouble(Path::new(&manifest_dir)).expect("failed to prune AppleDouble files");
    tauri_build::build()
}
