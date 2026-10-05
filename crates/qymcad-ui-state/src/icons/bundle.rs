//! Pack discovery, SVG validation, diagnostic inspection, and .qicons bundle packaging.

use std::path::Path;

use super::id::{IconId, ALL_ICONS};
use super::manifest::{ColorMode, IconManifest};
use super::pack::IconPack;

fn localized_readme_tag(name: &str) -> Option<&str> {
    let tag = name.strip_prefix("README.")?.strip_suffix(".md")?;
    tag.parse::<unic_langid::LanguageIdentifier>().ok()?;
    Some(tag)
}

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
        (cov * 100).checked_div(total).unwrap_or(0)
    }

    /// Whether there are any issues (rejected or extraneous files).
    pub fn has_issues(&self) -> bool {
        !self.rejected.is_empty() || !self.extraneous.is_empty()
    }

    /// Counts of included icons per category: [("sketch", 20, 32), ("constraint", 12, 12), ...]
    pub fn category_breakdown(&self) -> Vec<(&'static str, usize, usize)> {
        const CATEGORIES: &[&str] = &["sketch", "constraint", "part", "assembly", "datum"];
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

    let parts: Vec<&str> = val_str.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).collect();
    if parts.len() == 4 {
        let x: f32 = parts[0].parse().ok()?;
        let y: f32 = parts[1].parse().ok()?;
        let w: f32 = parts[2].parse().ok()?;
        let h: f32 = parts[3].parse().ok()?;
        (x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()).then_some((w, h))
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

    let (w, h) = parse_viewbox_dimensions(text).ok_or_else(|| "invalid viewBox attribute (expected four finite numbers)".to_string())?;
    if w <= 0.0 || h <= 0.0 {
        return Err(format!("non-positive viewBox dimensions: {w}x{h}"));
    }
    let ratio = w / h;
    if ratio < 0.95 || ratio > 1.05 {
        return Err(format!("non-square viewBox: {w}x{h} (aspect ratio must be 1:1)"));
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

fn monochrome_paint(value: &str) -> bool {
    let value = value.trim();
    value == "currentColor" || matches!(value.to_ascii_lowercase().as_str(), "white" | "#fff" | "#ffffff" | "none" | "rgb(255,255,255)" | "rgb(255, 255, 255)")
}

fn validate_monochrome_svg(data: &[u8]) -> Result<(), String> {
    let mut reader = quick_xml::Reader::from_reader(data);
    let mut inherited_fill = vec![false];
    loop {
        use quick_xml::events::Event;
        let event = reader.read_event().map_err(|err| format!("cannot parse SVG colors: {err}"))?;
        let (element, is_empty) = match &event {
            Event::Start(element) => (element, false),
            Event::Empty(element) => (element, true),
            Event::End(_) => {
                inherited_fill.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = std::str::from_utf8(element.local_name().as_ref()).map_err(|err| format!("invalid SVG element name: {err}"))?.to_ascii_lowercase();
        if matches!(name.as_str(), "style" | "lineargradient" | "radialgradient" | "pattern" | "filter" | "use" | "animate" | "animatetransform" | "animatecolor" | "set") || name.starts_with("fe") {
            return Err(format!("monochrome icon cannot use <{name}> because its colors cannot be checked"));
        }
        let mut fill_safe = *inherited_fill.last().unwrap_or(&false);
        for attr in element.attributes() {
            let attr = attr.map_err(|err| format!("invalid SVG color attribute: {err}"))?;
            let key = std::str::from_utf8(attr.key.as_ref()).map_err(|err| format!("invalid SVG color attribute name: {err}"))?.to_ascii_lowercase();
            let value = attr.unescape_value().map_err(|err| format!("invalid SVG color attribute value: {err}"))?;
            if key == "class" || value.to_ascii_lowercase().contains("url(") {
                return Err(format!("monochrome icon cannot use {key}={value:?} because its colors cannot be checked"));
            }
            if key == "style" {
                for declaration in value.split(';').filter(|part| !part.trim().is_empty()) {
                    let (property, paint) = declaration.split_once(':').ok_or_else(|| format!("invalid monochrome style declaration: {declaration}"))?;
                    let property = property.trim().to_ascii_lowercase();
                    let paint = paint.trim();
                    if !matches!(
                        property.as_str(),
                        "fill"
                            | "stroke"
                            | "color"
                            | "opacity"
                            | "fill-opacity"
                            | "stroke-opacity"
                            | "stroke-width"
                            | "stroke-linecap"
                            | "stroke-linejoin"
                            | "stroke-miterlimit"
                            | "stroke-dasharray"
                            | "stroke-dashoffset"
                            | "fill-rule"
                            | "clip-rule"
                            | "transform"
                            | "display"
                            | "visibility"
                    ) {
                        return Err(format!("monochrome icon cannot use style property {property:?} because its colors cannot be checked"));
                    }
                    if matches!(property.as_str(), "fill" | "stroke" | "color" | "stop-color" | "flood-color" | "lighting-color") || property.ends_with("-color") {
                        if paint != "inherit" && !monochrome_paint(paint) {
                            return Err(format!("monochrome icon has unsupported {property}={paint:?}; use white, currentColor, or none"));
                        }
                        if property == "fill" && paint != "inherit" {
                            fill_safe = true;
                        }
                    }
                }
            } else if matches!(key.as_str(), "fill" | "stroke" | "color" | "stop-color" | "flood-color" | "lighting-color") || key.ends_with("-color") {
                if value != "inherit" && !monochrome_paint(&value) {
                    return Err(format!("monochrome icon has unsupported {key}={value:?}; use white, currentColor, or none"));
                }
                if key == "fill" && value != "inherit" {
                    fill_safe = true;
                }
            }
        }
        if matches!(name.as_str(), "path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline" | "text" | "tspan") && !fill_safe {
            return Err(format!("monochrome <{name}> uses the implicit black fill; set fill to white, currentColor, or none"));
        }
        if !is_empty {
            inherited_fill.push(fill_safe);
        }
    }
    Ok(())
}

pub fn validate_icon_svg(data: &[u8], color_mode: ColorMode) -> Result<(), String> {
    validate_svg(data)?;
    if color_mode == ColorMode::Monochrome {
        validate_monochrome_svg(data)?;
    }
    Ok(())
}

fn removable_svg_element(name: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(name).to_ascii_lowercase();
    let local = lower.rsplit(':').next().unwrap_or(&lower);
    matches!(local, "script" | "foreignobject" | "applet" | "object" | "embed" | "iframe" | "audio" | "video" | "metadata" | "image")
        || lower.starts_with("sodipodi:")
        || lower.starts_with("inkscape:")
        || lower.starts_with("adobe:")
        || lower.starts_with("sketch:")
        || lower.starts_with("figma:")
        || matches!(lower.as_str(), "rdf:rdf" | "i:pgf" | "x:xmpmeta")
}

fn cleaned_svg_start(start: &quick_xml::events::BytesStart<'_>, changed: &mut bool) -> Result<quick_xml::events::BytesStart<'static>, String> {
    let mut cleaned = start.to_owned();
    cleaned.clear_attributes();
    for attr in start.attributes() {
        let attr = attr.map_err(|err| format!("invalid SVG attribute: {err}"))?;
        let key = std::str::from_utf8(attr.key.as_ref()).map_err(|err| format!("invalid SVG attribute name: {err}"))?;
        let value = attr.unescape_value().map_err(|err| format!("invalid SVG attribute value: {err}"))?;
        let lower_key = key.to_ascii_lowercase();
        let lower_value = value.to_ascii_lowercase();
        let event_handler = lower_key.strip_prefix("on").is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_alphabetic()));
        let editor_attribute = ["sodipodi:", "inkscape:", "adobe:", "sketch:", "figma:", "i:", "x:"].iter().any(|prefix| lower_key.starts_with(prefix));
        let external_link = lower_key.ends_with("href") && ["http:", "https:", "file:", "javascript:", "//"].iter().any(|prefix| lower_value.trim_start().starts_with(prefix));
        if event_handler || editor_attribute || lower_value.contains("data:image/") || external_link {
            *changed = true;
        } else {
            cleaned.push_attribute((key, value.as_ref()));
        }
    }
    Ok(cleaned)
}

/// Remove executable tags, editor metadata, raster content, and event attributes from SVG.
/// Geometry and viewBox values are retained; validation refuses changes that need manual repair.
pub fn clean_svg(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() as u64 > super::pack::MAX_ICON_SVG_SIZE {
        return Err("SVG file exceeds the icon size limit".to_string());
    }
    let source = std::str::from_utf8(data).map_err(|err| format!("SVG data is not valid UTF-8: {err}"))?;
    let mut reader = quick_xml::Reader::from_str(source);
    let mut writer = quick_xml::Writer::new(Vec::with_capacity(data.len()));
    let mut skipped_depth = 0usize;
    let mut open_depth = 0usize;
    let mut saw_svg_root = false;
    let mut changed = false;
    loop {
        use quick_xml::events::Event;
        let event = reader.read_event().map_err(|err| format!("cannot parse SVG: {err}"))?;
        if open_depth == 0 {
            match &event {
                Event::Text(text) => {
                    let bytes: &[u8] = text.as_ref();
                    if !bytes.iter().all(u8::is_ascii_whitespace) {
                        return Err("SVG contains text outside its root element".to_string());
                    }
                }
                Event::CData(_) | Event::GeneralRef(_) => return Err("SVG contains content outside its root element".to_string()),
                _ => {}
            }
        }
        match &event {
            Event::Start(start) | Event::Empty(start) if open_depth == 0 => {
                if saw_svg_root || start.name().as_ref() != b"svg" {
                    return Err("SVG must have one <svg> root element".to_string());
                }
                saw_svg_root = true;
            }
            _ => {}
        }
        match &event {
            Event::Start(_) => open_depth += 1,
            Event::End(_) => open_depth = open_depth.checked_sub(1).ok_or_else(|| "SVG has an unmatched closing tag".to_string())?,
            _ => {}
        }
        match event {
            Event::Start(_) if skipped_depth > 0 => skipped_depth += 1,
            Event::Start(start) if removable_svg_element(start.name().as_ref()) => {
                skipped_depth = 1;
                changed = true;
            }
            Event::Start(start) => writer.write_event(Event::Start(cleaned_svg_start(&start, &mut changed)?)).map_err(|err| err.to_string())?,
            Event::Empty(_) if skipped_depth > 0 => {}
            Event::Empty(empty) if removable_svg_element(empty.name().as_ref()) => changed = true,
            Event::Empty(empty) => writer.write_event(Event::Empty(cleaned_svg_start(&empty, &mut changed)?)).map_err(|err| err.to_string())?,
            Event::End(_) if skipped_depth > 0 => skipped_depth -= 1,
            Event::End(end) => writer.write_event(Event::End(end)).map_err(|err| err.to_string())?,
            Event::DocType(_) | Event::PI(_) | Event::Comment(_) => changed = true,
            Event::Eof => break,
            _ if skipped_depth > 0 => {}
            Event::GeneralRef(reference) => {
                let name: &[u8] = reference.as_ref();
                if name.starts_with(b"#") || [b"amp".as_slice(), b"lt", b"gt", b"quot", b"apos"].contains(&name) {
                    writer.write_event(Event::GeneralRef(reference)).map_err(|err| err.to_string())?;
                } else {
                    changed = true;
                }
            }
            other => writer.write_event(other).map_err(|err| err.to_string())?,
        }
    }
    if !saw_svg_root || open_depth != 0 || skipped_depth != 0 {
        return Err("SVG has an unclosed or missing root element".to_string());
    }
    let cleaned = writer.into_inner();
    if cleaned.len() as u64 > super::pack::MAX_ICON_SVG_SIZE {
        return Err("cleaned SVG exceeds the icon size limit".to_string());
    }
    validate_svg(&cleaned)?;
    if !changed && validate_svg(data).is_err() {
        return Err("SVG needs manual repair".to_string());
    }
    Ok(cleaned)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanIconResult {
    Missing,
    Unchanged,
    Cleaned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanPackReport {
    pub cleaned: Vec<std::path::PathBuf>,
    pub failed: Vec<CleanFileFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanFileFailure {
    pub path: std::path::PathBuf,
    pub reason: String,
}

fn clean_svg_file(path: &Path) -> Result<CleanIconResult, String> {
    let original = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(CleanIconResult::Missing),
        Err(err) => return Err(format!("cannot read SVG: {err}")),
    };
    if original.len() as u64 > super::pack::MAX_ICON_SVG_SIZE {
        return Err("SVG file exceeds the icon size limit".to_string());
    }
    if validate_svg(&original).is_ok() {
        return Ok(CleanIconResult::Unchanged);
    }
    let cleaned = clean_svg(&original)?;
    if cleaned == original {
        return Err("SVG needs manual repair".to_string());
    }
    let temp = path.with_extension(format!("svg.qymcad-{}.tmp", std::process::id()));
    if let Err(err) = std::fs::write(&temp, &cleaned) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("cannot write cleaned SVG: {err}"));
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(temp);
        return Err(format!("cannot replace SVG: {err}"));
    }
    Ok(CleanIconResult::Cleaned)
}

pub fn clean_directory_icon(pack: &IconPack, id: IconId) -> Result<CleanIconResult, String> {
    if !pack.is_directory() {
        return Err("only editable directory packs can be cleaned".to_string());
    }
    let super::pack::PackSource::Directory(root) = &pack.source else {
        return Err("only directory packs can be cleaned".to_string());
    };
    clean_svg_file(&root.join("icons").join(format!("{}.svg", id.relative_path())))
}

fn collect_svg_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("cannot inspect icon directory: {err}")),
    };
    for entry in entries {
        let entry = entry.map_err(|err| format!("cannot inspect icon entry: {err}"))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|err| format!("cannot inspect icon file: {err}"))?;
        if kind.is_dir() {
            collect_svg_files(&path, files)?;
        } else if kind.is_file() && path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("svg")) {
            files.push(path);
        }
    }
    Ok(())
}

/// Whether bulk cleaning can change at least one SVG in an editable directory pack.
pub fn directory_has_cleanable_icons(pack: &IconPack) -> Result<bool, String> {
    if !pack.is_directory() {
        return Ok(false);
    }
    let super::pack::PackSource::Directory(root) = &pack.source else {
        return Ok(false);
    };
    let mut files = Vec::new();
    collect_svg_files(&root.join("icons"), &mut files)?;
    files.push(root.join("icon.svg"));
    for path in files {
        let data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(_) => continue,
        };
        if data.len() as u64 > super::pack::MAX_ICON_SVG_SIZE || validate_svg(&data).is_ok() {
            continue;
        }
        if clean_svg(&data).is_ok_and(|cleaned| cleaned != data) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn clean_directory_icons(pack: &IconPack) -> Result<CleanPackReport, String> {
    if !pack.is_directory() {
        return Err("only editable directory packs can be cleaned".to_string());
    }
    let super::pack::PackSource::Directory(root) = &pack.source else {
        return Err("only directory packs can be cleaned".to_string());
    };
    let mut files = Vec::new();
    collect_svg_files(&root.join("icons"), &mut files)?;
    files.push(root.join("icon.svg"));
    files.sort();
    let mut report = CleanPackReport { cleaned: Vec::new(), failed: Vec::new() };
    for path in files {
        let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        match clean_svg_file(&path) {
            Ok(CleanIconResult::Cleaned) => report.cleaned.push(relative),
            Ok(CleanIconResult::Missing | CleanIconResult::Unchanged) => {}
            Err(reason) => report.failed.push(CleanFileFailure { path: relative, reason }),
        }
    }
    Ok(report)
}

/// Inspect and validate an icon pack directory, returning a detailed `ValidationReport`.
/// Checks all SVG viewports, identifies extraneous/unknown files, and lists included vs missing icons.
pub fn inspect_pack_directory(source_dir: impl AsRef<Path>) -> Result<ValidationReport, String> {
    let source_dir = source_dir.as_ref();
    let color_mode = if source_dir.join("manifest.ron").exists() { IconPack::from_directory(source_dir)?.manifest.color_mode } else { ColorMode::Universal };
    inspect_pack_directory_for_mode(source_dir, color_mode)
}

/// Inspect a folder using the color mode that will be written to its bundle manifest.
pub fn inspect_pack_directory_for_mode(source_dir: impl AsRef<Path>, color_mode: ColorMode) -> Result<ValidationReport, String> {
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
                    || fname == "README.md"
                    || fname == "README.txt"
                    || localized_readme_tag(fname).is_some()
                    || fname.starts_with("preview.")
                    || fname.ends_with(".qicons")
                    || fname.ends_with(".zip");
                if !allowed {
                    extraneous.push(format!("extra root file: {fname}"));
                } else if localized_readme_tag(fname).is_some() {
                    if std::fs::metadata(&p).ok().is_some_and(|metadata| metadata.len() > super::pack::MAX_TEXT_FILE_SIZE) {
                        rejected.push((fname.to_string(), "localized README exceeds maximum text size".to_string()));
                    } else if let Err(err) = std::fs::read_to_string(&p) {
                        rejected.push((fname.to_string(), format!("localized README is not readable UTF-8: {err}")));
                    }
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
    fn walk_icons(base: &Path, current: &Path, color_mode: ColorMode, included: &mut Vec<IconId>, rejected: &mut Vec<(String, String)>, extraneous: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(current) else { return };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk_icons(base, &p, color_mode, included, rejected, extraneous);
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
                    Ok(data) => match validate_icon_svg(&data, color_mode) {
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

    walk_icons(&icons_dir, &icons_dir, color_mode, &mut included, &mut rejected, &mut extraneous);
    included.sort_by_key(|id| id.relative_path());

    // 3. Compute missing icons from standard catalog
    let missing: Vec<IconId> = ALL_ICONS.iter().copied().filter(|id| !included.contains(id)).collect();

    Ok(ValidationReport { included, missing, rejected, extraneous })
}

/// Package an icon folder into any writer (e.g. file or in-memory cursor).
/// Only valid, verified icons that match a known `IconId` are packaged into the archive.
/// Extraneous files and invalid SVGs are automatically excluded.
pub fn package_bundle_to_writer<W: std::io::Write + std::io::Seek>(source_dir: impl AsRef<Path>, manifest: &IconManifest, writer: W) -> Result<ValidationReport, String> {
    let source_dir = source_dir.as_ref();
    let report = inspect_pack_directory_for_mode(source_dir, manifest.color_mode)?;

    if report.included.is_empty() {
        return Err("cannot package bundle: 0 valid CAD icons found in icons/ directory".to_string());
    }

    let mut zip = zip::ZipWriter::new(writer);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

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

    let mut localized_readmes = std::fs::read_dir(source_dir)
        .map_err(|err| format!("cannot list localized READMEs: {err}"))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            (entry.path().is_file() && localized_readme_tag(&name).is_some()).then_some(name)
        })
        .collect::<Vec<_>>();
    localized_readmes.sort();
    for name in localized_readmes {
        let path = source_dir.join(&name);
        let metadata = std::fs::metadata(&path).map_err(|err| format!("cannot inspect {name}: {err}"))?;
        if metadata.len() > super::pack::MAX_TEXT_FILE_SIZE {
            return Err(format!("{name} exceeds maximum text size"));
        }
        let content = std::fs::read_to_string(&path).map_err(|err| format!("cannot read {name} as UTF-8: {err}"))?;
        zip.start_file(&name, options).map_err(|err| err.to_string())?;
        std::io::Write::write_all(&mut zip, content.as_bytes()).map_err(|err| err.to_string())?;
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
pub fn package_bundle_to_bytes(source_dir: impl AsRef<Path>, manifest: &IconManifest) -> Result<(Vec<u8>, ValidationReport), String> {
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
pub fn package_bundle(source_dir: impl AsRef<Path>, manifest: &IconManifest, output_archive: impl AsRef<Path>) -> Result<ValidationReport, String> {
    let (bytes, report) = package_bundle_to_bytes(source_dir, manifest)?;
    std::fs::write(output_archive.as_ref(), bytes).map_err(|e| e.to_string())?;
    Ok(report)
}
