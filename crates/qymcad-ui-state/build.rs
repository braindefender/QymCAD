// Build script for qymcad-ui-state:
// 1. Reads the clean `IconId` enum variants from `src/icons/id.rs`.
// 2. Maps each variant to its canonical `icons/<category>/<name>.svg` path.
// 3. Validates all default icons in `assets/icon-themes/default/` (SVG validity, viewBox, no raster images).
// 4. Generates `OUT_DIR/icon_generated.rs` implementing `relative_path(&self)`, `from_id_str(s)`, and `ALL_ICONS`.
// 5. Packages the default icon theme into a compressed `.qicons` bundle written to `OUT_DIR/default.qicons`.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn variant_to_relative_path(var: &str) -> String {
    if var == "SketchPickPlane" {
        return "datum/sketch_pick_plane".to_string();
    }
    if var == "SketchCircle3Pt" {
        return "sketch/circle_3pt".to_string();
    }
    for cat in &["Sketch", "Constraint", "Datum", "Part", "Assembly", "Common"] {
        if let Some(rest) = var.strip_prefix(cat) {
            let mut s = String::new();
            for (i, c) in rest.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    s.push('_');
                }
                s.push(c.to_ascii_lowercase());
            }
            return format!("{}/{}", cat.to_ascii_lowercase(), s);
        }
    }
    panic!("Unknown icon variant category for: {}", var);
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo_root = manifest_dir.join("../..");
    let default_theme_dir = repo_root.join("assets/icon-themes/default");

    println!("cargo:rerun-if-changed={}", default_theme_dir.display());

    if !default_theme_dir.is_dir() {
        panic!(
            "Default icon theme directory not found at: {}",
            default_theme_dir.display()
        );
    }

    // 1. Read and validate manifest.ron
    let manifest_path = default_theme_dir.join("manifest.ron");
    if !manifest_path.is_file() {
        panic!("Missing manifest.ron in default icon theme: {}", manifest_path.display());
    }
    let manifest_content = fs::read_to_string(&manifest_path).expect("manifest.ron reads");

    // 2. Extract variants from `src/icons/id.rs`
    let id_rs_path = manifest_dir.join("src/icons/id.rs");
    println!("cargo:rerun-if-changed={}", id_rs_path.display());
    let id_rs_content = fs::read_to_string(&id_rs_path).expect("id.rs reads");

    let mut in_enum = false;
    let mut variants = Vec::new();
    for line in id_rs_content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub enum IconId") {
            in_enum = true;
            continue;
        }
        if in_enum {
            if trimmed.starts_with('}') {
                break;
            }
            if trimmed.starts_with("//") || trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let var = trimmed.trim_end_matches(',').trim();
            if !var.is_empty() && var.chars().next().unwrap().is_ascii_uppercase() {
                variants.push(var.to_string());
            }
        }
    }

    if variants.is_empty() {
        panic!("Failed to parse IconId enum variants from id.rs");
    }

    // 3. Map variants to relative paths and check uniqueness
    let mut var_to_path = Vec::new();
    let mut path_to_var = HashMap::new();
    let mut expected_icons = HashSet::new();

    for v in &variants {
        let rel_path = variant_to_relative_path(v);
        var_to_path.push((v.clone(), rel_path.clone()));
        path_to_var.insert(rel_path.clone(), v.clone());
        expected_icons.insert(format!("{rel_path}.svg"));
    }

    // 4. Verify all expected icons exist and pass SVG validation
    let icons_dir = default_theme_dir.join("icons");
    for rel_path in &expected_icons {
        let full_path = icons_dir.join(rel_path);
        if !full_path.is_file() {
            panic!(
                "COMPILE ERROR: Missing default vector SVG icon: {}\nExpected at: {}",
                rel_path,
                full_path.display()
            );
        }

        let svg_data = fs::read(&full_path).unwrap_or_else(|e| {
            panic!("Failed to read {}: {e}", full_path.display());
        });

        let text = match std::str::from_utf8(&svg_data) {
            Ok(t) => t,
            Err(_) => panic!("SVG data in {} is not valid UTF-8", full_path.display()),
        };

        if !text.contains("<svg") {
            panic!("Missing <svg> root element in {}", full_path.display());
        }
        if !text.contains("viewBox") {
            panic!("Missing viewBox attribute in {}", full_path.display());
        }
        if text.contains("<image") || text.contains("data:image/") {
            panic!("Prohibited raster image found in {}", full_path.display());
        }
    }

    // 5. Generate OUT_DIR/icon_generated.rs
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let mut gen = String::new();
    gen.push_str("// Generated by build.rs. Do not edit directly.\n\n");
    gen.push_str("impl IconId {\n");
    gen.push_str("    /// Relative path inside the `icons/` folder, without `.svg` extension.\n");
    gen.push_str("    pub fn relative_path(&self) -> &'static str {\n");
    gen.push_str("        match self {\n");
    for (v, p) in &var_to_path {
        gen.push_str(&format!("            Self::{v} => \"{p}\",\n"));
    }
    gen.push_str("        }\n");
    gen.push_str("    }\n\n");

    gen.push_str("    /// Resolve an `IconId` from a relative path or dotted identifier.\n");
    gen.push_str("    pub fn from_id_str(s: &str) -> Option<Self> {\n");
    gen.push_str("        let clean = s.trim().trim_end_matches(\".svg\").replace('.', \"/\");\n");
    gen.push_str("        match clean.as_str() {\n");
    for (v, p) in &var_to_path {
        gen.push_str(&format!("            \"{p}\" => Some(Self::{v}),\n"));
    }
    gen.push_str("            _ => None,\n");
    gen.push_str("        }\n");
    gen.push_str("    }\n");
    gen.push_str("}\n\n");

    gen.push_str("/// All known `IconId` values. Generated automatically by build script.\n");
    gen.push_str("pub const ALL_ICONS: &[IconId] = &[\n");
    for v in &variants {
        gen.push_str(&format!("    IconId::{v},\n"));
    }
    gen.push_str("];\n");

    let gen_path = out_dir.join("icon_generated.rs");
    fs::write(&gen_path, gen).expect("writes icon_generated.rs");

    // 6. Package default theme into OUT_DIR/default.qicons
    let out_archive = out_dir.join("default.qicons");
    let out_file = fs::File::create(&out_archive).expect("creates output zip in OUT_DIR");
    let mut zip = zip::ZipWriter::new(out_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Write manifest.ron
    zip.start_file("manifest.ron", options).expect("writes manifest into zip");
    zip.write_all(manifest_content.as_bytes()).expect("writes manifest bytes");

    let pack_icon = fs::read(default_theme_dir.join("icon.svg")).expect("default pack icon reads");
    zip.start_file("icon.svg", options).expect("writes pack icon into zip");
    zip.write_all(&pack_icon).expect("writes pack icon bytes");

    // Write README.md if present
    let readme_path = default_theme_dir.join("README.md");
    if readme_path.is_file() {
        let readme_content = fs::read(&readme_path).expect("readme reads");
        zip.start_file("README.md", options).expect("writes readme into zip");
        zip.write_all(&readme_content).expect("writes readme bytes");
    }

    // Walk and write all SVGs
    fn walk_dir(
        base: &Path,
        current: &Path,
        zip: &mut zip::ZipWriter<fs::File>,
        options: zip::write::SimpleFileOptions,
    ) {
        for entry in fs::read_dir(current).expect("reads dir").flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_dir(base, &p, zip, options);
            } else if p.extension().is_some_and(|e| e == "svg") {
                let rel = p.strip_prefix(base).expect("strip prefix");
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let data = fs::read(&p).expect("read file");
                zip.start_file(format!("icons/{rel_str}"), options).expect("zip start file");
                zip.write_all(&data).expect("zip write file");
            }
        }
    }

    walk_dir(&icons_dir, &icons_dir, &mut zip, options);
    zip.finish().expect("finishes zip archive");

    println!("cargo:info=Successfully validated {} icons and packaged default.qicons", variants.len());
}
