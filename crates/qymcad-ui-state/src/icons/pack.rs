//! Icon pack loading from disk directories, archives, or memory.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::id::{IconId, ALL_ICONS};
use super::manifest::IconManifest;

/// Source storage for an icon pack.
#[derive(Debug, Clone)]
pub enum PackSource {
    /// Folder on disk containing `manifest.ron` and `icons/`.
    Directory(PathBuf),
    /// Zip archive containing `manifest.ron` and `icons/` strictly at root level.
    Archive(PathBuf),
    /// In-memory map (used for tests and baked-in virtual bundles).
    Memory(HashMap<String, Vec<u8>>),
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
            Self::VerifiedArchive => "Verified Archive",
            Self::Embedded => "Built-in",
        }
    }
}

/// A loaded icon pack ready for icon retrieval.
#[derive(Debug, Clone)]
pub struct IconPack {
    pub manifest: IconManifest,
    pub source: PackSource,
}

impl IconPack {
    /// The provenance and format of this icon pack.
    pub fn format(&self) -> BundleFormat {
        if self.manifest.id == "default" || self.manifest.id == "freecad-classic" {
            return BundleFormat::Embedded;
        }
        match &self.source {
            PackSource::Directory(_) => BundleFormat::Directory,
            PackSource::Archive(_) => {
                if self.manifest.verified {
                    BundleFormat::VerifiedArchive
                } else {
                    BundleFormat::Archive
                }
            }
            PackSource::Memory(_) => {
                if self.manifest.verified {
                    BundleFormat::VerifiedArchive
                } else {
                    BundleFormat::Archive
                }
            }
        }
    }

    /// Whether this pack is a folder on disk that supports live file editing.
    pub fn is_directory(&self) -> bool {
        self.format() == BundleFormat::Directory
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

    /// Load an icon pack from a directory on disk.
    pub fn from_directory(dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = dir.as_ref();
        let manifest_path = dir.join("manifest.ron");
        if !manifest_path.is_file() {
            return Err(format!("missing manifest.ron in {}", dir.display()));
        }
        let content = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
        let manifest = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;
        Ok(Self { manifest, source: PackSource::Directory(dir.to_path_buf()) })
    }

    /// Load an icon pack from a `.qicons` (zip) file.
    /// The archive must strictly contain `manifest.ron` and `icons/` at the root level.
    pub fn from_archive(file_path: impl AsRef<Path>) -> Result<Self, String> {
        let file_path = file_path.as_ref();
        let file = std::fs::File::open(file_path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

        let is_verified = zip.by_name(".verified").is_ok();
        let mut manifest_file = zip
            .by_name("manifest.ron")
            .map_err(|_| "missing manifest.ron in archive".to_string())?;
        let mut content = String::new();
        manifest_file.read_to_string(&mut content).map_err(|e| e.to_string())?;
        let mut manifest = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;
        if is_verified {
            manifest.verified = true;
        }

        Ok(Self { manifest, source: PackSource::Archive(file_path.to_path_buf()) })
    }

    /// Load an icon pack from in-memory ZIP archive bytes (e.g. from `include_bytes!`).
    pub fn from_zip_bytes(bytes: &[u8]) -> Result<Self, String> {
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;

        let is_verified = zip.by_name(".verified").is_ok();
        let mut manifest = {
            let mut manifest_file = zip
                .by_name("manifest.ron")
                .map_err(|_| "missing manifest.ron in archive".to_string())?;
            let mut content = String::new();
            manifest_file.read_to_string(&mut content).map_err(|e| e.to_string())?;
            IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?
        };
        if is_verified || manifest.id == "default" {
            manifest.verified = true;
        }

        let mut map = HashMap::new();
        for i in 0..zip.len() {
            let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
            if !file.is_dir() {
                let name = file.name().to_string();
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                map.insert(name, buf);
            }
        }

        Ok(Self { manifest, source: PackSource::Memory(map) })
    }

    /// Retrieve raw SVG bytes for a relative path inside `icons/` (e.g. `"sketch/line.svg"`).
    pub fn get_svg(&self, rel_path: &str) -> Option<Vec<u8>> {
        let clean = rel_path.trim_start_matches('/');
        let file_subpath = if clean.ends_with(".svg") {
            clean.to_string()
        } else {
            format!("{clean}.svg")
        };

        match &self.source {
            PackSource::Directory(base) => {
                let p = base.join("icons").join(&file_subpath);
                read_svg_with_retry(&p)
            }
            PackSource::Archive(archive_path) => {
                let file = std::fs::File::open(archive_path).ok()?;
                let mut zip = zip::ZipArchive::new(file).ok()?;
                let full_name = format!("icons/{file_subpath}");
                let index = zip
                    .index_for_name(&full_name)
                    .or_else(|| zip.index_for_name(&file_subpath))?;
                let mut entry = zip.by_index(index).ok()?;
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf).ok()?;
                Some(buf)
            }
            PackSource::Memory(map) => {
                let full_name = format!("icons/{file_subpath}");
                map.get(&full_name).cloned().or_else(|| map.get(&file_subpath).cloned())
            }
        }
    }

    /// Retrieve raw SVG bytes for an `IconId`.
    pub fn get_svg_for_id(&self, id: IconId) -> Option<Vec<u8>> {
        self.get_svg(id.relative_path())
    }

    /// List all known `IconId`s present in this pack.
    pub fn available_icons(&self) -> Vec<IconId> {
        ALL_ICONS
            .iter()
            .copied()
            .filter(|id| self.get_svg_for_id(*id).is_some())
            .collect()
    }

    /// Calculate coverage as (present_count, total_count).
    pub fn coverage(&self) -> (usize, usize) {
        let count = self.available_icons().len();
        (count, ALL_ICONS.len())
    }

    /// Retrieve README markdown text for this icon pack, or fallback to a formatted manifest description.
    pub fn get_readme(&self) -> String {
        let try_file = |name: &str| -> Option<String> {
            match &self.source {
                PackSource::Directory(base) => {
                    let p = base.join(name);
                    std::fs::read_to_string(p).ok()
                }
                PackSource::Archive(archive_path) => {
                    let file = std::fs::File::open(archive_path).ok()?;
                    let mut zip = zip::ZipArchive::new(file).ok()?;
                    let mut entry = zip.by_name(name).ok()?;
                    let mut s = String::new();
                    entry.read_to_string(&mut s).ok()?;
                    Some(s)
                }
                PackSource::Memory(map) => {
                    map.get(name).and_then(|bytes| String::from_utf8(bytes.clone()).ok())
                }
            }
        };

        for candidate in &["README.md", "readme.md", "README.txt", "description.md"] {
            if let Some(text) = try_file(candidate) {
                if !text.trim().is_empty() {
                    return text;
                }
            }
        }

        // Fallback: build markdown text from manifest
        let mut out = format!("# {}\n\n", self.manifest.name);
        if !self.manifest.description.is_empty() {
            out.push_str(&self.manifest.description);
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
                    let mut entry = zip.by_name(candidate).ok()?;
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf).ok()?;
                    Some(buf)
                }
                PackSource::Memory(map) => map.get(candidate).cloned(),
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
fn read_svg_with_retry(path: &Path) -> Option<Vec<u8>> {
    for attempt in 0..6 {
        if let Ok(data) = std::fs::read(path) {
            if !data.is_empty() {
                return Some(data);
            }
        } else if attempt >= 2 && !path.exists() {
            return None;
        }
        if attempt < 5 {
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }
    let data = std::fs::read(path).ok()?;
    if !data.is_empty() {
        Some(data)
    } else {
        None
    }
}

/// Recursively collect all .svg files in a directory, ignoring temporary and editor swap files.
fn collect_svgs_recursively(
    dir: &Path,
    base: &Path,
    map: &mut HashMap<PathBuf, (std::time::SystemTime, u64)>,
) {
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
            if name_lower.ends_with(".svg")
                && !name.starts_with('.')
                && !name.starts_with('~')
                && !name.ends_with(".tmp")
                && !name.ends_with(".bak")
                && !name.ends_with('~')
            {
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
