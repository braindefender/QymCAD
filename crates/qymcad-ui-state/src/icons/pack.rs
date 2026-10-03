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

/// A loaded icon pack ready for icon retrieval.
#[derive(Debug, Clone)]
pub struct IconPack {
    pub manifest: IconManifest,
    pub source: PackSource,
}

impl IconPack {
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

        let mut manifest_file = zip
            .by_name("manifest.ron")
            .map_err(|_| "missing manifest.ron in archive".to_string())?;
        let mut content = String::new();
        manifest_file.read_to_string(&mut content).map_err(|e| e.to_string())?;
        let manifest = IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?;

        Ok(Self { manifest, source: PackSource::Archive(file_path.to_path_buf()) })
    }

    /// Load an icon pack from in-memory ZIP archive bytes (e.g. from `include_bytes!`).
    pub fn from_zip_bytes(bytes: &[u8]) -> Result<Self, String> {
        let cursor = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(cursor).map_err(|e| e.to_string())?;

        let manifest = {
            let mut manifest_file = zip
                .by_name("manifest.ron")
                .map_err(|_| "missing manifest.ron in archive".to_string())?;
            let mut content = String::new();
            manifest_file.read_to_string(&mut content).map_err(|e| e.to_string())?;
            IconManifest::parse_ron(&content).map_err(|e| format!("parse error: {e}"))?
        };

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
                std::fs::read(p).ok()
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
}
