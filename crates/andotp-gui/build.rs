//! Embeds the service logos from `thumbnails/` (see `tools/vector2svg.py`).

use std::fmt::Write;
use std::path::Path;

fn main() {
    // `cfg(camera)`: a QR scanner is available (webcam on desktop, camera on Android).
    println!("cargo::rustc-check-cfg=cfg(camera)");
    let android = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "android");
    if android || std::env::var_os("CARGO_FEATURE_WEBCAM").is_some() {
        println!("cargo::rustc-cfg=camera");
    }

    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("thumbnails");
    println!("cargo::rerun-if-changed={}", dir.display());

    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("thumbnails directory")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".svg").map(str::to_owned))
        .collect();
    // Sorted so the table can be binary searched.
    names.sort();

    let mut table =
        String::from("/// (thumbnail name, SVG), sorted by name.\npub static THUMBNAILS: &[(&str, &[u8])] = &[\n");
    for name in &names {
        let file = dir.join(format!("{name}.svg"));
        writeln!(table, "    ({name:?}, include_bytes!({:?})),", file.display().to_string()).unwrap();
    }
    table.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("thumbnails.rs");
    std::fs::write(out, table).unwrap();
}
