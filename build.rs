//! Embeds the icon packs in the binary: one table per pack, `(name, svg bytes)`.

use std::fmt::Write;
use std::path::Path;

fn pack(name: &str, dir: &Path, out: &mut String) {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "svg"))
        .collect();
    files.sort();
    writeln!(out, "pub static {name}: &[(&str, &[u8])] = &[").unwrap();
    for file in files {
        let stem = file.file_stem().unwrap().to_string_lossy();
        writeln!(out, "    ({stem:?}, include_bytes!({:?})),", file.canonicalize().unwrap()).unwrap();
    }
    writeln!(out, "];").unwrap();
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut out = String::new();
    pack("PHOSPHOR", &root.join("phosphor-fill"), &mut out);
    pack("MATERIAL", &root.join("material"), &mut out);
    pack("OUTLINE", &root.join("outline"), &mut out);
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("icons.rs");
    std::fs::write(dest, out).unwrap();
}
