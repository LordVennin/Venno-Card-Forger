//! Reads every SVG under a folder and reports what could not be read or drawn:
//! `cargo run --release -p cardsmith-engine --example svg_check -- <dir>`.
//! For checking the reader against a symbols package.

use std::collections::BTreeMap;

fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files);
        } else if path.extension().is_some_and(|e| e == "svg") {
            files.push(path);
        }
    }
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: svg_check <dir>");
    let mut files = Vec::new();
    walk(std::path::Path::new(&dir), &mut files);
    let (mut failed, mut skipped) = (Vec::new(), BTreeMap::<String, usize>::new());
    for file in &files {
        match std::fs::read_to_string(file).map_err(|e| e.to_string()).and_then(|text| cardsmith_engine::svg::parse_svg(&text)) {
            Ok(art) => {
                let layers = art.layers([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], "s");
                if layers.is_empty() {
                    failed.push(format!("{}: no layers", file.display()));
                }
                for what in art.unsupported {
                    *skipped.entry(what).or_default() += 1;
                }
            }
            Err(e) => failed.push(format!("{}: {e}", file.display())),
        }
    }
    println!("{} files, {} could not be read", files.len(), failed.len());
    for line in failed.iter().take(20) {
        println!("  {line}");
    }
    println!("skipped features (files): {skipped:?}");
}
