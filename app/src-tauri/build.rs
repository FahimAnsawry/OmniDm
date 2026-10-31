use std::path::Path;

fn main() {
    release_notes();
    tauri_build::build()
}

/// Builds docs/releases/<version>.md into the app as `RELEASE_NOTES`, so after an update
/// OmniDM can say what changed in the version it now is.
fn release_notes() {
    let dir = Path::new("../../docs/releases");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut table = String::from("pub const RELEASE_NOTES: &[(&str, &str)] = &[\n");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md") {
                let version = path.file_stem().unwrap().to_string_lossy().into_owned();
                let path = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(path);
                println!("cargo:rerun-if-changed={}", path.display());
                table.push_str(&format!("    ({version:?}, include_str!({:?})),\n", path));
            }
        }
    }
    table.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("release_notes.rs");
    std::fs::write(out, table).unwrap();
}
