//! Pack discovery, SVG validation, diagnostic inspection, and .qicons bundle packaging.

use std::path::Path;

use super::id::{IconId, ALL_ICONS};
use super::manifest::IconManifest;
use super::pack::IconPack;

/// Detailed diagnostic report of an icon theme directory or archive.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ValidationReport {
    /// Valid icons matching a known `IconId` that passed all SVG checks and will be included in the bundle.
    pub included: Vec<IconId>,
    /// Known CAD icons from `ALL_ICONS` that are not provided by this pack (will fall back to default theme).
    pub missing: Vec<IconId>,
    /// Files that failed SVG validation, with relative path and error description.
    pub rejected: Vec<(String, String)>,
    /// Extraneous files found (non-SVG files or unrecognized icon names).
    pub extraneous: Vec<String>,
}

impl ValidationReport {
    /// Total included icons and total known CAD icons (e.g. (45, 107)).
    pub fn coverage(&self) -> (usize, usize) {
        (self.included.len(), ALL_ICONS.len())
    }

    /// Total percentage coverage (0 to 100).
    pub fn coverage_percent(&self) -> usize {
        let (cov, total) = self.coverage();
        if total == 0 { 0 } else { (cov * 100) / total }
    }

    /// Whether there are any issues (rejected or extraneous files).
    pub fn has_issues(&self) -> bool {
        !self.rejected.is_empty() || !self.extraneous.is_empty()
    }

    /// Counts of included icons per category: [("sketch", 20, 32), ("constraint", 12, 12), ...]
    pub fn category_breakdown(&self) -> Vec<(&'static str, usize, usize)> {
        const CATEGORIES: &[&str] = &["sketch", "constraint", "part", "assembly", "datum", "common"];
        let mut breakdown = Vec::new();
        for &cat in CATEGORIES {
            let total = ALL_ICONS.iter().filter(|id| id.relative_path().starts_with(cat)).count();
            let inc = self.included.iter().filter(|id| id.relative_path().starts_with(cat)).count();
            breakdown.push((cat, inc, total));
        }
        breakdown
    }
}

/// Discover icon packs from a directory (subdirectories with `manifest.ron` and `.qicons`/`.zip` archives).
pub fn discover_packs_in(dir: &Path) -> Vec<IconPack> {
    let mut packs = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return packs };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.join("manifest.ron").exists() {
                if let Ok(pack) = IconPack::from_directory(&path) {
                    packs.push(pack);
                }
            }
        } else if path.extension().is_some_and(|ext| ext == "qicons" || ext == "zip") {
            if let Ok(pack) = IconPack::from_archive(&path) {
                packs.push(pack);
            }
        }
    }
    packs.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    packs
}

/// Parse width and height from an SVG viewBox attribute.
fn parse_viewbox_dimensions(text: &str) -> Option<(f32, f32)> {
    let idx = text.find("viewBox")?;
    let rest = &text[idx + 7..];
    let quote_start = rest.find(['"', '\''])?;
    let quote_char = rest.as_bytes()[quote_start] as char;
    let content = &rest[quote_start + 1..];
    let quote_end = content.find(quote_char)?;
    let val_str = &content[..quote_end];

    let parts: Vec<&str> = val_str
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() == 4 {
        let w: f32 = parts[2].parse().ok()?;
        let h: f32 = parts[3].parse().ok()?;
        Some((w, h))
    } else {
        None
    }
}

/// Inspect SVG text for extraneous editor metadata, dangerous executable elements, or junk tags.
/// Returns a list of human-readable issues detected.
pub fn find_svg_junk_issues(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut issues = Vec::new();

    // Dangerous / executable tags
    if lower.contains("<script") {
        issues.push("prohibited <script> tag".to_string());
    }
    if lower.contains("<foreignobject") {
        issues.push("prohibited <foreignObject> tag".to_string());
    }
    for tag in ["<applet", "<object", "<embed", "<iframe", "<audio", "<video"] {
        if lower.contains(tag) {
            issues.push(format!("prohibited {tag}> tag"));
        }
    }

    // Inline event handlers (onload=, onclick=, onerror=, etc.)
    if let Some(idx) = lower.find(" on") {
        let rest = &lower[idx..];
        for word in rest.split_whitespace() {
            if word.starts_with("on") && word.contains('=') {
                let attr = word.split('=').next().unwrap_or(word);
                if attr.chars().skip(2).all(|c| c.is_alphabetic()) {
                    issues.push(format!("prohibited event handler attribute ({attr})"));
                    break;
                }
            }
        }
    }

    // Editor metadata & proprietary elements
    if lower.contains("<sodipodi:") {
        issues.push("editor metadata (<sodipodi:*>)".to_string());
    }
    if lower.contains("<inkscape:") {
        issues.push("editor metadata (<inkscape:*>)".to_string());
    }
    if lower.contains("<metadata") {
        issues.push("extraneous <metadata> block".to_string());
    }
    if lower.contains("<rdf:rdf") {
        issues.push("extraneous <rdf:RDF> block".to_string());
    }
    if lower.contains("<i:pgf") {
        issues.push("proprietary Illustrator metadata (<i:pgf>)".to_string());
    }
    if lower.contains("<adobe:") || lower.contains("<x:xmpmeta") {
        issues.push("proprietary Adobe/XMP metadata".to_string());
    }
    if lower.contains("<sketch:") {
        issues.push("proprietary Sketch metadata".to_string());
    }
    if lower.contains("<figma:") {
        issues.push("proprietary Figma metadata".to_string());
    }
    if lower.contains("<!entity") {
        issues.push("prohibited <!ENTITY> declaration".to_string());
    }

    issues
}

/// Validate SVG data according to the theme specification:
/// - Must contain a valid root `<svg>` element.
/// - Must contain a valid square `viewBox` attribute (1:1 aspect ratio, e.g. `viewBox="0 0 64 64"`).
/// - Must NOT contain embedded raster images (`<image>` or `data:image/`).
/// - Must NOT contain junk tags, editor metadata, or executable elements.
pub fn validate_svg(data: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(data).map_err(|_| "SVG data is not valid UTF-8".to_string())?;

    if !text.contains("<svg") {
        return Err("missing <svg> root element".to_string());
    }

    if !text.contains("viewBox") {
        return Err("missing viewBox attribute (expected 1:1, e.g. viewBox=\"0 0 64 64\")".to_string());
    }

    if let Some((w, h)) = parse_viewbox_dimensions(text) {
        if w <= 0.0 || h <= 0.0 {
            return Err(format!("non-positive viewBox dimensions: {w}x{h}"));
        }
        let ratio = w / h;
        if ratio < 0.95 || ratio > 1.05 {
            return Err(format!("non-square viewBox: {w}x{h} (aspect ratio must be 1:1)"));
        }
    }

    if text.contains("<image") || text.contains("data:image/") {
        return Err("embedded raster images (<image>) are prohibited".to_string());
    }

    let junk = find_svg_junk_issues(text);
    if !junk.is_empty() {
        return Err(format!("extraneous/junk tags detected: {}", junk.join(", ")));
    }

    Ok(())
}

/// Inspect and validate an icon pack directory, returning a detailed `ValidationReport`.
/// Checks all SVG viewports, identifies extraneous/unknown files, and lists included vs missing icons.
pub fn inspect_pack_directory(source_dir: impl AsRef<Path>) -> Result<ValidationReport, String> {
    let source_dir = source_dir.as_ref();
    let icons_dir = source_dir.join("icons");
    if !icons_dir.is_dir() {
        return Err(format!("missing icons/ directory in {}", source_dir.display()));
    }

    let mut included = Vec::new();
    let mut rejected = Vec::new();
    let mut extraneous = Vec::new();

    // 1. Inspect root files in source_dir (excluding icons/)
    if let Ok(entries) = std::fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let allowed = fname == "manifest.ron"
                    || fname == "icon.svg"
                    || fname.starts_with("LICENSE")
                    || fname.starts_with("README")
                    || fname.starts_with("preview.")
                    || fname.ends_with(".qicons")
                    || fname.ends_with(".zip");
                if !allowed {
                    extraneous.push(format!("extra root file: {fname}"));
                } else if fname == "icon.svg" {
                    if std::fs::metadata(&p).ok().is_some_and(|m| m.len() > super::pack::MAX_ICON_SVG_SIZE) {
                        rejected.push((fname.to_string(), "pack icon exceeds maximum SVG size".to_string()));
                    } else {
                        match std::fs::read(&p) {
                            Ok(data) => {
                                if let Err(err) = validate_svg(&data) {
                                    rejected.push((fname.to_string(), err));
                                }
                            }
                            Err(err) => rejected.push((fname.to_string(), format!("read error: {err}"))),
                        }
                    }
                }
            }
        }
    }

    // 2. Recursively walk icons/ directory
    fn walk_icons(
        base: &Path,
        current: &Path,
        included: &mut Vec<IconId>,
        rejected: &mut Vec<(String, String)>,
        extraneous: &mut Vec<String>,
    ) {
        let Ok(entries) = std::fs::read_dir(current) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_icons(base, &p, included, rejected, extraneous);
            } else if p.is_file() {
                let Ok(rel) = p.strip_prefix(base) else { continue };
                let rel_str = rel.to_string_lossy().replace('\\', "/");

                // Non-SVG files in icons/ are extraneous
                if !rel_str.ends_with(".svg") {
                    extraneous.push(format!("non-SVG file in icons/: {rel_str}"));
                    continue;
                }

                // Check if recognized CAD IconId
                let Some(id) = IconId::from_id_str(&rel_str) else {
                    extraneous.push(format!("unrecognized icon path: {rel_str} (not a known CAD icon)"));
                    continue;
                };

                // Validate SVG content and viewport
                match std::fs::read(&p) {
                    Ok(data) => match validate_svg(&data) {
                        Ok(()) => {
                            if !included.contains(&id) {
                                included.push(id);
                            }
                        }
                        Err(err) => {
                            rejected.push((rel_str, err));
                        }
                    },
                    Err(e) => {
                        rejected.push((rel_str, format!("read error: {e}")));
                    }
                }
            }
        }
    }

    walk_icons(&icons_dir, &icons_dir, &mut included, &mut rejected, &mut extraneous);
    included.sort_by_key(|id| id.relative_path());

    // 3. Compute missing icons from standard catalog
    let missing: Vec<IconId> = ALL_ICONS
        .iter()
        .copied()
        .filter(|id| !included.contains(id))
        .collect();

    Ok(ValidationReport {
        included,
        missing,
        rejected,
        extraneous,
    })
}

/// Package an icon folder into any writer (e.g. file or in-memory cursor).
/// Only valid, verified icons that match a known `IconId` are packaged into the archive.
/// Extraneous files and invalid SVGs are automatically excluded.
pub fn package_bundle_to_writer<W: std::io::Write + std::io::Seek>(
    source_dir: impl AsRef<Path>,
    manifest: &IconManifest,
    writer: W,
) -> Result<ValidationReport, String> {
    let source_dir = source_dir.as_ref();
    let report = inspect_pack_directory(source_dir)?;

    if report.included.is_empty() {
        return Err("cannot package bundle: 0 valid CAD icons found in icons/ directory".to_string());
    }

    let mut zip = zip::ZipWriter::new(writer);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 1. Write manifest.ron (marked verified)
    let mut manifest = manifest.clone();
    manifest.verified = true;
    let ron_text = manifest.to_ron().map_err(|e| format!("manifest serialization error: {e}"))?;
    zip.start_file("manifest.ron", options).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut zip, ron_text.as_bytes()).map_err(|e| e.to_string())?;

    // 2. Write optional metadata and preview files from root if present
    for doc in &["LICENSE", "LICENSE.txt", "LICENSE.md", "README.md", "README.txt", "preview.svg", "preview.png", "preview.webp", "icon.svg"] {
        let doc_path = source_dir.join(doc);
        if doc_path.is_file() {
            if *doc == "icon.svg" && std::fs::metadata(&doc_path).ok().is_some_and(|m| m.len() > super::pack::MAX_ICON_SVG_SIZE) {
                continue;
            }
            if let Ok(content) = std::fs::read(&doc_path) {
                if *doc == "icon.svg" && (content.len() as u64 > super::pack::MAX_ICON_SVG_SIZE || validate_svg(&content).is_err()) {
                    continue;
                }
                let _ = zip.start_file(*doc, options);
                let _ = std::io::Write::write_all(&mut zip, &content);
            }
        }
    }

    // 3. Write ONLY the validated icons
    let icons_dir = source_dir.join("icons");
    for id in &report.included {
        let rel_path = format!("{}.svg", id.relative_path());
        let src_file = icons_dir.join(&rel_path);
        let data = std::fs::read(&src_file).map_err(|e| format!("failed to read {}: {e}", src_file.display()))?;
        zip.start_file(format!("icons/{rel_path}"), options).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut zip, &data).map_err(|e| e.to_string())?;
    }

    zip.finish().map_err(|e| e.to_string())?;

    Ok(report)
}

/// Package an icon folder into an in-memory byte buffer, signed with QymCAD verification trailer.
pub fn package_bundle_to_bytes(
    source_dir: impl AsRef<Path>,
    manifest: &IconManifest,
) -> Result<(Vec<u8>, ValidationReport), String> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = cursor;
    let report = package_bundle_to_writer(source_dir, manifest, &mut writer)?;
    let mut bytes = writer.into_inner();
    super::sha256::append_qicons_trailer(&mut bytes);
    Ok((bytes, report))
}

/// Package an icon folder into a `.qicons` bundle file on disk.
/// Automatically validates all icons, guarantees that only verified CAD icons enter the bundle,
/// and seals the bundle with a trailing cryptographic SHA-256 integrity record.
pub fn package_bundle(
    source_dir: impl AsRef<Path>,
    manifest: &IconManifest,
    output_archive: impl AsRef<Path>,
) -> Result<ValidationReport, String> {
    let (bytes, report) = package_bundle_to_bytes(source_dir, manifest)?;
    std::fs::write(output_archive.as_ref(), bytes).map_err(|e| e.to_string())?;
    Ok(report)
}
