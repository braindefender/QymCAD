//! Icon pack loading from disk directories, archives, or memory.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use super::id::{IconId, ALL_ICONS};
use super::manifest::{locale_fallbacks, ColorMode, IconManifest, PackageType};

const DEFAULT_PACK_ICON_SVG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icon-themes/default/icon.svg"));

/// Source storage for an icon pack.
#[derive(Debug, Clone)]
pub enum PackSource {
    /// Folder on disk containing `manifest.ron` and `icons/`.
    Directory(PathBuf),
    /// Zip archive containing `manifest.ron` and `icons/` strictly at root level.
    Archive(PathBuf),
    /// In-memory map (used for tests and virtual bundles).
    Memory(HashMap<String, Vec<u8>>),
    /// Embedded in the application executable binary.
    Embedded(HashMap<String, Vec<u8>>),
}

/// The underlying format and provenance of an icon bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleFormat {
    /// Folder containing unpacked SVG files. Live editing and live watch supported.
    Directory,
    /// Standard zip archive (.qicons).
    Archive,
    /// Verified package created or certified by QymCAD packager.
    VerifiedArchive,
    /// Embedded in the application executable binary.
    Embedded,
}

impl BundleFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Directory => "Folder",
            Self::Archive => "Archive",
            Self::VerifiedArchive => "Bundle",
            Self::Embedded => "Built-in",
        }
    }
}

/// Maximum file size permitted for an icon archive file on disk (16 MB).
pub const MAX_ARCHIVE_FILE_SIZE: u64 = 16 * 1024 * 1024;
/// Maximum number of file entries permitted in an icon archive (prevents zip bomb exhaustion).
pub const MAX_ARCHIVE_ENTRIES: usize = 1_000;
/// Maximum cumulative uncompressed size of all files across an entire icon archive (32 MB).
pub const MAX_TOTAL_UNCOMPRESSED_SIZE: u64 = 32 * 1024 * 1024;
/// Maximum uncompressed size permitted for any single file inside an icon archive (2 MB).
pub const MAX_SINGLE_FILE_UNCOMPRESSED_SIZE: u64 = 2 * 1024 * 1024;
/// Maximum uncompressed size permitted for an individual SVG icon (512 KB).
pub const MAX_ICON_SVG_SIZE: u64 = 512 * 1024;
/// Maximum allowed size for manifest.ron (64 KB).
pub const MAX_MANIFEST_SIZE: u64 = 64 * 1024;
/// Maximum allowed size for documentation text files (512 KB).
pub const MAX_TEXT_FILE_SIZE: u64 = 512 * 1024;
/// Maximum compression ratio before triggering decompression bomb rejection for files > 64 KB.
pub const MAX_COMPRESSION_RATIO: u64 = 250;

/// Validate a ZIP archive against decompression bombs (Zip Bombs), zip slip, and resource exhaustion.
pub fn validate_archive_safety<R: std::io::Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<(), String> {
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!("archive contains too many files: {} (limit is {})", zip.len(), MAX_ARCHIVE_ENTRIES));
    }

    let mut total_uncompressed: u64 = 0;

    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name();

        // 1. Path traversal / Zip Slip protection
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') || name.contains(':') {
            return Err(format!("insecure file path inside archive: {name}"));
        }

        let uncompressed = file.size();
        let compressed = file.compressed_size();

        // 2. Single file size ceiling (e.g. rejects gigantic multi-gigabyte files)
        if uncompressed > MAX_SINGLE_FILE_UNCOMPRESSED_SIZE {
            return Err(format!("file {name} exceeds maximum uncompressed size: {uncompressed} bytes (limit is {MAX_SINGLE_FILE_UNCOMPRESSED_SIZE})"));
        }

        // 3. Compression ratio check (detects Deflate bomb payloads)
        if compressed > 0 && uncompressed > 64 * 1024 {
            let ratio = uncompressed / compressed;
            if ratio > MAX_COMPRESSION_RATIO {
                return Err(format!("suspicious compression ratio in {name} ({ratio}:1, possible decompression bomb)"));
            }
        }

        total_uncompressed = total_uncompressed.saturating_add(uncompressed);
        if total_uncompressed > MAX_TOTAL_UNCOMPRESSED_SIZE {
            return Err(format!("archive total uncompressed size exceeds limit: {total_uncompressed} bytes (limit is {MAX_TOTAL_UNCOMPRESSED_SIZE})"));
        }
    }

    Ok(())
}

fn archive_entries<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut map = HashMap::new();
    for index in 0..zip.len() {
        let file = zip.by_index(index).map_err(|err| err.to_string())?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_string();
        let mut data = Vec::new();
        file.take(MAX_SINGLE_FILE_UNCOMPRESSED_SIZE + 1).read_to_end(&mut data).map_err(|err| err.to_string())?;
        if data.len() as u64 > MAX_SINGLE_FILE_UNCOMPRESSED_SIZE {
            return Err(format!("file {name} exceeds safe memory limit"));
        }
        map.insert(name, data);
    }
    Ok(map)
}

/// A loaded icon pack ready for icon retrieval.
#[derive(Debug, Clone)]
pub struct IconPack {
    pub manifest: IconManifest,
    pub source: PackSource,
    /// Whether this package had its verification trailer tampered with or corrupted.
    pub is_tampered: bool,
}

impl IconPack {
    /// The provenance and format of this icon pack.
    pub fn format(&self) -> BundleFormat {
        match &self.source {
            PackSource::Directory(_) => BundleFormat::Directory,
            PackSource::Archive(_) | PackSource::Memory(_) => {
                if self.manifest.verified && !self.is_tampered {
                    BundleFormat::VerifiedArchive
                } else {
                    BundleFormat::Archive
                }
            }
            PackSource::Embedded(_) => BundleFormat::Embedded,
        }
    }

    /// Whether this pack is a folder on disk that supports live file editing.
    pub fn is_directory(&self) -> bool {
        matches!(self.source, PackSource::Directory(_))
    }

    /// Whether an icon file exists on disk in a directory pack (even if temporarily locked).
    pub fn has_icon_on_disk(&self, id: IconId) -> bool {
        match &self.source {
            PackSource::Directory(base) => {
                let rel = id.relative_path();
                let sub = if rel.ends_with(".svg") { rel.to_string() } else { format!("{rel}.svg") };
                base.join("icons").join(sub).exists()
            }
            _ => false,
        }
    }

    /// Whether this pack is an archive (.qicons).
    pub fn is_archive(&self) -> bool {
        matches!(self.source, PackSource::Archive(_))
    }

    /// Whether this pack is verified.
    pub fn is_verified(&self) -> bool {
        self.manifest.verified || self.format() == BundleFormat::VerifiedArchive
    }

    /// Reload the pack manifest from its source on disk if available.
    pub fn reload_manifest(&mut self) -> Result<(), String> {
        match &self.source {
            PackSource::Directory(dir) => {
                let manifest_path = dir.join("manifest.ron");
                if !manifest_path.is_file() {
                    return Err(format!("missing manifest.ron in {}", dir.display()));
                }
                let content = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
                let manifest = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;
                manifest.validate().map_err(|e| format!("invalid manifest in {}: {e}", dir.display()))?;
                self.manifest = manifest;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Load an icon pack from a directory on disk.
    pub fn from_directory(dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = dir.as_ref();
        let manifest_path = dir.join("manifest.ron");
        if !manifest_path.is_file() {
            return Err(format!("missing manifest.ron in {}", dir.display()));
        }
        let content = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
        let manifest = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;
        manifest.validate().map_err(|e| format!("invalid manifest in {}: {e}", dir.display()))?;
        Ok(Self { manifest, source: PackSource::Directory(dir.to_path_buf()), is_tampered: false })
    }

    /// Load an icon pack from a `.qicons` (bundle) or `.zip` (archive) file.
    /// - `.qicons`: Verified QymCAD bundle (checks cryptographic trailer; if tampered, downgraded to unverified archive).
    /// - `.zip`: Unverified community archive (can omit manifest.ron, crash-guarded).
    pub fn from_archive(file_path: impl AsRef<Path>) -> Result<Self, String> {
        let file_path = file_path.as_ref();
        let metadata = std::fs::metadata(file_path).map_err(|e| e.to_string())?;
        if metadata.len() > MAX_ARCHIVE_FILE_SIZE {
            return Err(format!("archive file size exceeds limit: {} bytes (limit is {MAX_ARCHIVE_FILE_SIZE})", metadata.len()));
        }

        let is_qicons = file_path.extension().is_some_and(|e| e == "qicons");
        let mut file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
        let trailer_check = super::sha256::verify_qicons_trailer_stream(&mut file, metadata.len()).map_err(|e| e.to_string())?;

        let (is_verified, is_tampered) = match trailer_check {
            super::sha256::TrailerCheck::Verified => (true, false),
            super::sha256::TrailerCheck::Tampered => (false, true),
            super::sha256::TrailerCheck::Unsigned => {
                // If it claims to be .qicons but has no valid trailer, it is an unverified/tampered archive
                (false, is_qicons)
            }
        };

        file.seek(std::io::SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

        // Guard against decompression bombs (Zip Bombs), excessive file counts, and zip-slip
        validate_archive_safety(&mut zip)?;

        let mut manifest = if let Ok(manifest_file) = zip.by_name("manifest.ron") {
            let mut content = String::new();
            manifest_file.take(MAX_MANIFEST_SIZE + 1).read_to_string(&mut content).map_err(|e| e.to_string())?;
            if content.len() as u64 > MAX_MANIFEST_SIZE {
                return Err("manifest.ron exceeds maximum allowed size".to_string());
            }
            let parsed = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;
            parsed.validate().map_err(|e| format!("invalid manifest in {}: {e}", file_path.display()))?;
            parsed
        } else {
            // Missing manifest: allowed for generic .zip community archives.
            // Synthesize a safe fallback manifest from the file name.
            let raw_stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("community-icons");
            let id: String = raw_stem.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).take(super::manifest::MAX_MANIFEST_ID_LEN).collect();
            let trimmed_id = id.trim_matches(|c| c == '-' || c == '_');
            let id = if trimmed_id.is_empty() { "community-pack".to_string() } else { trimmed_id.to_string() };
            let name: String = raw_stem.chars().take(super::manifest::MAX_MANIFEST_NAME_LEN).collect();
            let name = if name.trim().is_empty() { "Community Icons".to_string() } else { name };
            IconManifest {
                package_type: PackageType::IconTheme,
                id,
                name,
                version: "1.0.0".to_string(),
                author: "Community".to_string(),
                license: "Unknown".to_string(),
                description: "Imported ZIP archive (unverified)".to_string(),
                color_mode: ColorMode::Universal,
                translations: Default::default(),
                verified: false,
            }
        };

        manifest.verified = is_verified;

        Ok(Self { manifest, source: PackSource::Archive(file_path.to_path_buf()), is_tampered })
    }

    /// Read a validated archive once for manager previews; 106 separate icon reads took about 100 ms on a 248 KB archive.
    pub fn archive_snapshot(&self) -> Result<Self, String> {
        let PackSource::Archive(path) = &self.source else {
            return Ok(self.clone());
        };
        let file = std::fs::File::open(path).map_err(|err| err.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|err| err.to_string())?;
        validate_archive_safety(&mut zip)?;
        let map = archive_entries(&mut zip)?;
        Ok(Self { manifest: self.manifest.clone(), source: PackSource::Memory(map), is_tampered: self.is_tampered })
    }

    /// Load an icon pack from in-memory ZIP archive bytes (e.g. from `include_bytes!`).
    pub fn from_zip_bytes(bytes: &[u8]) -> Result<Self, String> {
        let trailer_check = super::sha256::verify_qicons_trailer(bytes);
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;

        // Guard against decompression bombs and unsafe archives
        validate_archive_safety(&mut zip)?;

        let is_verified = trailer_check == super::sha256::TrailerCheck::Verified;
        let mut manifest = {
            let manifest_file = zip.by_name("manifest.ron").map_err(|_| "missing manifest.ron in archive".to_string())?;
            let mut content = String::new();
            manifest_file.take(MAX_MANIFEST_SIZE + 1).read_to_string(&mut content).map_err(|e| e.to_string())?;
            if content.len() as u64 > MAX_MANIFEST_SIZE {
                return Err("manifest.ron exceeds maximum allowed size".to_string());
            }
            let parsed = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;
            parsed.validate().map_err(|e| format!("invalid manifest: {e}"))?;
            parsed
        };
        if is_verified || manifest.id == "default" {
            manifest.verified = true;
        }

        let map = archive_entries(&mut zip)?;

        Ok(Self { manifest, source: PackSource::Memory(map), is_tampered: trailer_check == super::sha256::TrailerCheck::Tampered })
    }

    /// Load an embedded icon pack from in-memory ZIP archive bytes (e.g. from `include_bytes!`).
    pub fn from_embedded_zip_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut pack = Self::from_zip_bytes(bytes)?;
        pack.manifest.verified = true;
        pack.is_tampered = false;
        if let PackSource::Memory(map) = pack.source {
            pack.source = PackSource::Embedded(map);
        }
        Ok(pack)
    }

    /// Retrieve raw SVG bytes for a relative path inside `icons/` (e.g. `"sketch/line.svg"`).
    pub fn get_svg(&self, rel_path: &str) -> Option<Vec<u8>> {
        let clean = rel_path.trim_start_matches('/');
        let file_subpath = if clean.ends_with(".svg") { clean.to_string() } else { format!("{clean}.svg") };

        let data = match &self.source {
            PackSource::Directory(base) => {
                let p = base.join("icons").join(&file_subpath);
                read_svg_with_retry(&p)?
            }
            PackSource::Archive(archive_path) => {
                let file = std::fs::File::open(archive_path).ok()?;
                let mut zip = zip::ZipArchive::new(file).ok()?;
                let full_name = format!("icons/{file_subpath}");
                let index = zip.index_for_name(&full_name).or_else(|| zip.index_for_name(&file_subpath))?;
                let entry = zip.by_index(index).ok()?;
                if entry.size() > MAX_ICON_SVG_SIZE {
                    return None;
                }
                let mut buf = Vec::new();
                entry.take(MAX_ICON_SVG_SIZE + 1).read_to_end(&mut buf).ok()?;
                if buf.len() as u64 > MAX_ICON_SVG_SIZE {
                    return None;
                }
                buf
            }
            PackSource::Memory(map) | PackSource::Embedded(map) => {
                let full_name = format!("icons/{file_subpath}");
                map.get(&full_name).or_else(|| map.get(&file_subpath))?.clone()
            }
        };

        if self.format() == BundleFormat::VerifiedArchive || self.format() == BundleFormat::Embedded {
            // Fast path: Verified QymCAD bundle or built-in embedded pack
            Some(data)
        } else {
            // Unverified pack (folder, community zip, or raw memory): must pass SVG validation to prevent corrupting UI or blocking fallback
            sanitize_svg_for_safety(data.clone())?;
            if super::bundle::validate_icon_svg(&data, self.manifest.color_mode).is_err() {
                return None;
            }
            Some(data)
        }
    }

    /// Retrieve raw SVG bytes for an `IconId`.
    pub fn get_svg_for_id(&self, id: IconId) -> Option<Vec<u8>> {
        self.get_svg(id.relative_path())
    }

    /// Read an icon for preview, distinguishing an absent file from a file that cannot be used.
    pub fn inspect_svg_for_id(&self, id: IconId) -> Result<Option<Vec<u8>>, String> {
        let file_subpath = format!("{}.svg", id.relative_path());
        let full_name = format!("icons/{file_subpath}");
        let data = match &self.source {
            PackSource::Directory(base) => {
                let path = base.join(&full_name);
                let metadata = match std::fs::metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(err) => return Err(format!("cannot inspect SVG file: {err}")),
                };
                if !metadata.is_file() {
                    return Err("SVG path is not a file".to_string());
                }
                if metadata.len() > MAX_ICON_SVG_SIZE {
                    return Err(format!("SVG file exceeds {MAX_ICON_SVG_SIZE} byte limit"));
                }
                if metadata.len() == 0 {
                    return Err("SVG file is empty".to_string());
                }
                read_svg_with_retry(&path).ok_or_else(|| "SVG file is empty or cannot be read".to_string())?
            }
            PackSource::Archive(archive_path) => {
                let file = std::fs::File::open(archive_path).map_err(|err| format!("cannot open archive: {err}"))?;
                let mut zip = zip::ZipArchive::new(file).map_err(|err| format!("cannot read archive: {err}"))?;
                let Some(index) = zip.index_for_name(&full_name).or_else(|| zip.index_for_name(&file_subpath)) else {
                    return Ok(None);
                };
                let entry = zip.by_index(index).map_err(|err| format!("cannot read SVG entry: {err}"))?;
                if entry.size() > MAX_ICON_SVG_SIZE {
                    return Err(format!("SVG file exceeds {MAX_ICON_SVG_SIZE} byte limit"));
                }
                let mut data = Vec::new();
                entry.take(MAX_ICON_SVG_SIZE + 1).read_to_end(&mut data).map_err(|err| format!("cannot read SVG entry: {err}"))?;
                data
            }
            PackSource::Memory(map) | PackSource::Embedded(map) => {
                let Some(data) = map.get(&full_name).or_else(|| map.get(&file_subpath)) else {
                    return Ok(None);
                };
                data.clone()
            }
        };

        if data.len() as u64 > MAX_ICON_SVG_SIZE {
            return Err(format!("SVG file exceeds {MAX_ICON_SVG_SIZE} byte limit"));
        }
        if data.is_empty() {
            return Err("SVG file is empty".to_string());
        }
        super::bundle::validate_icon_svg(&data, self.manifest.color_mode)?;
        if matches!(self.source, PackSource::Archive(_) | PackSource::Memory(_)) && self.format() != BundleFormat::VerifiedArchive && sanitize_svg_for_safety(data.clone()).is_none() {
            return Err("SVG failed archive safety checks (external reference or NUL byte)".to_string());
        }
        Ok(Some(data))
    }

    /// Return the pack's root icon.svg, falling back to the built-in pack image.
    pub fn get_pack_icon_svg(&self) -> Vec<u8> {
        let data = match &self.source {
            PackSource::Directory(base) => {
                let path = base.join("icon.svg");
                if std::fs::metadata(&path).ok().is_some_and(|m| m.len() <= MAX_ICON_SVG_SIZE) {
                    std::fs::read(path).ok()
                } else {
                    None
                }
            }
            PackSource::Archive(archive_path) => {
                let file = std::fs::File::open(archive_path).ok();
                file.and_then(|file| zip::ZipArchive::new(file).ok()).and_then(|mut zip| {
                    let entry = zip.by_name("icon.svg").ok()?;
                    if entry.size() > MAX_ICON_SVG_SIZE {
                        return None;
                    }
                    let mut data = Vec::new();
                    entry.take(MAX_ICON_SVG_SIZE + 1).read_to_end(&mut data).ok()?;
                    (data.len() as u64 <= MAX_ICON_SVG_SIZE).then_some(data)
                })
            }
            PackSource::Memory(map) | PackSource::Embedded(map) => map.get("icon.svg").cloned(),
        };
        data.and_then(sanitize_svg_for_safety).filter(|svg| super::bundle::validate_svg(svg).is_ok()).unwrap_or_else(|| DEFAULT_PACK_ICON_SVG.to_vec())
    }

    /// List all known `IconId`s present in this pack.
    pub fn available_icons(&self) -> Vec<IconId> {
        ALL_ICONS.iter().copied().filter(|id| self.get_svg_for_id(*id).is_some()).collect()
    }

    /// Calculate coverage as CoverageCount { present, total }.
    pub fn coverage(&self) -> super::bundle::CoverageCount {
        let count = self.available_icons().len();
        super::bundle::CoverageCount { present: count, total: ALL_ICONS.len() }
    }

    /// Retrieve README markdown text for this icon pack, or fallback to a formatted manifest description.
    pub fn get_readme(&self) -> String {
        self.get_readme_for_locale("")
    }

    /// Retrieve the requested language's README, then the base README or manifest text.
    pub fn get_readme_for_locale(&self, locale: &str) -> String {
        let try_file = |name: &str| -> Option<String> {
            match &self.source {
                PackSource::Directory(base) => {
                    let p = base.join(name);
                    if std::fs::metadata(&p).ok()?.len() > MAX_TEXT_FILE_SIZE {
                        return None;
                    }
                    std::fs::read_to_string(p).ok()
                }
                PackSource::Archive(archive_path) => {
                    let file = std::fs::File::open(archive_path).ok()?;
                    let mut zip = zip::ZipArchive::new(file).ok()?;
                    let entry = zip.by_name(name).ok()?;
                    if entry.size() > MAX_TEXT_FILE_SIZE {
                        return None;
                    }
                    let mut s = String::new();
                    entry.take(MAX_TEXT_FILE_SIZE + 1).read_to_string(&mut s).ok()?;
                    if s.len() as u64 > MAX_TEXT_FILE_SIZE {
                        return None;
                    }
                    Some(s)
                }
                PackSource::Memory(map) | PackSource::Embedded(map) => map.get(name).filter(|bytes| bytes.len() as u64 <= MAX_TEXT_FILE_SIZE).and_then(|bytes| String::from_utf8(bytes.clone()).ok()),
            }
        };

        for tag in locale_fallbacks(locale) {
            let candidate = format!("README.{tag}.md");
            if let Some(text) = try_file(&candidate).filter(|text| !text.trim().is_empty()) {
                return text;
            }
        }

        for candidate in &["README.md", "readme.md", "README.txt", "description.md"] {
            if let Some(text) = try_file(candidate) {
                if !text.trim().is_empty() {
                    return text;
                }
            }
        }

        // Fallback: build markdown text from manifest
        let mut out = format!("# {}\n\n", self.manifest.name_for_locale(locale));
        let description = self.manifest.description_for_locale(locale);
        if !description.is_empty() {
            out.push_str(description);
            out.push_str("\n\n");
        }
        out.push_str(&format!(
            "- **ID**: `{}`\n- **Version**: `{}`\n- **Author**: {}\n- **License**: `{}`\n- **Color Mode**: `{:?}`\n",
            self.manifest.id, self.manifest.version, self.manifest.author, self.manifest.license, self.manifest.color_mode
        ));
        out
    }

    /// Retrieve preview image data and extension if provided in the pack (e.g. preview.svg, preview.png).
    pub fn get_preview_image(&self) -> Option<(Vec<u8>, &'static str)> {
        let try_file = |candidate: &str| -> Option<Vec<u8>> {
            match &self.source {
                PackSource::Directory(base) => {
                    let p = base.join(candidate);
                    std::fs::read(p).ok()
                }
                PackSource::Archive(archive_path) => {
                    let file = std::fs::File::open(archive_path).ok()?;
                    let mut zip = zip::ZipArchive::new(file).ok()?;
                    let entry = zip.by_name(candidate).ok()?;
                    if entry.size() > MAX_SINGLE_FILE_UNCOMPRESSED_SIZE {
                        return None;
                    }
                    let mut buf = Vec::new();
                    entry.take(MAX_SINGLE_FILE_UNCOMPRESSED_SIZE + 1).read_to_end(&mut buf).ok()?;
                    if buf.len() as u64 > MAX_SINGLE_FILE_UNCOMPRESSED_SIZE {
                        return None;
                    }
                    Some(buf)
                }
                PackSource::Memory(map) | PackSource::Embedded(map) => map.get(candidate).cloned(),
            }
        };

        for (candidate, ext) in &[("preview.svg", "svg"), ("preview.png", "png"), ("preview.webp", "webp")] {
            if let Some(bytes) = try_file(candidate) {
                return Some((bytes, ext));
            }
        }
        None
    }

    /// Take a full snapshot of all SVG files and manifest.ron in this directory pack,
    /// mapping relative path -> (modification time, file size).
    pub fn directory_snapshot(&self) -> Option<HashMap<PathBuf, (std::time::SystemTime, u64)>> {
        let PackSource::Directory(base) = &self.source else {
            return None;
        };

        let mut snapshot = HashMap::new();
        let manifest_path = base.join("manifest.ron");
        if let Ok(m) = std::fs::metadata(&manifest_path) {
            let mtime = m.modified().unwrap_or(std::time::UNIX_EPOCH);
            snapshot.insert(PathBuf::from("manifest.ron"), (mtime, m.len()));
        }

        let icon_path = base.join("icon.svg");
        if let Ok(m) = std::fs::metadata(&icon_path) {
            let mtime = m.modified().unwrap_or(std::time::UNIX_EPOCH);
            snapshot.insert(PathBuf::from("icon.svg"), (mtime, m.len()));
        }

        let icons_dir = base.join("icons");
        collect_svgs_recursively(&icons_dir, base, &mut snapshot);

        Some(snapshot)
    }

    /// Returns the maximum modification timestamp among files in this directory pack,
    /// or None if this is not a directory.
    pub fn latest_mtime(&self) -> Option<std::time::SystemTime> {
        let snap = self.directory_snapshot()?;
        snap.values().map(|(t, _)| *t).max()
    }
}

/// Helper to read SVG file with retry backoff.
/// On Windows, graphic editors (Inkscape, Illustrator, VS Code) lock files exclusively
/// during save or perform atomic rename (delete + rename), and may temporarily truncate to 0 bytes.
/// Retrying with short delays (~75ms total) avoids ERROR_SHARING_VIOLATION and empty reads.
/// Missing paths return immediately; 106 absent icons previously took 3.2s to check.
fn read_svg_with_retry(path: &Path) -> Option<Vec<u8>> {
    for attempt in 0..6 {
        match std::fs::metadata(path) {
            Ok(m) if m.len() > MAX_ICON_SVG_SIZE => return None,
            Ok(m) if m.len() > 0 => {
                if let Ok(mut file) = std::fs::File::open(path) {
                    let mut buf = Vec::with_capacity(m.len() as usize);
                    if file.by_ref().take(MAX_ICON_SVG_SIZE + 1).read_to_end(&mut buf).is_ok() && buf.len() as u64 <= MAX_ICON_SVG_SIZE && !buf.is_empty() {
                        return Some(buf);
                    }
                }
            }
            Ok(m) if m.len() == 0 && attempt >= 2 => return None,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound || (attempt >= 2 && !path.exists()) => {
                return None;
            }
            _ => {}
        }
        if attempt < 5 {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
    None
}

/// Recursively collect all .svg files in a directory, ignoring temporary and editor swap files.
fn collect_svgs_recursively(dir: &Path, base: &Path, map: &mut HashMap<PathBuf, (std::time::SystemTime, u64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_svgs_recursively(&path, base, map);
        } else if let Ok(name) = entry.file_name().into_string() {
            let name_lower = name.to_lowercase();
            // Only track valid SVG files, ignoring temporary/editor swap files and OS artifacts
            if name_lower.ends_with(".svg") && !name.starts_with('.') && !name.starts_with('~') && !name.ends_with(".tmp") && !name.ends_with(".bak") && !name.ends_with('~') {
                if let Ok(m) = entry.metadata() {
                    let mtime = m.modified().unwrap_or(std::time::UNIX_EPOCH);
                    if let Ok(rel) = path.strip_prefix(base) {
                        map.insert(rel.to_path_buf(), (mtime, m.len()));
                    }
                }
            }
        }
    }
}

/// Crash-guard for unverified archives (.zip or corrupted .qicons):
/// Protects against XML entity bombs, gigantic payloads, null bytes, and non-SVG data.
fn sanitize_svg_for_safety(buf: Vec<u8>) -> Option<Vec<u8>> {
    // 1. Guard against memory exhaustion (icons should not exceed 512 KB)
    if buf.len() > 512 * 1024 {
        return None;
    }
    // 2. Must be valid UTF-8
    let text = std::str::from_utf8(&buf).ok()?;
    let lower = text.to_lowercase();
    // 3. Must contain root SVG element
    if !lower.contains("<svg") {
        return None;
    }
    // 4. Guard against XML entity expansion / billion laughs attack
    if lower.contains("<!entity") || lower.contains("system \"") || lower.contains("system '") {
        return None;
    }
    // 5. Guard against null bytes
    if buf.contains(&0) {
        return None;
    }
    Some(buf)
}
