//! Icon manager, priority cascade resolution, and global state.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::id::IconId;
use super::pack::IconPack;

/// The result of resolving an icon through the priority stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIcon {
    /// SVG file bytes.
    pub data: Arc<[u8]>,
    /// Identifier of the pack supplying the icon.
    pub pack_id: String,
    /// Monotonically increasing revision counter (invalidates texture cache on edits).
    pub revision: u64,
    /// Palette fingerprint used during token resolution.
    pub palette_fingerprint: u64,
}

/// Central manager for icon themes and priority cascade.
pub struct IconManager {
    /// Active icon packs in descending priority (index 0 is highest priority).
    active_stack: Vec<IconPack>,
    /// Active palette for CSS variable preprocessing.
    palette: qymcad_scheme::Palette,
    /// Cache of resolved icons by `IconId`.
    cache: HashMap<IconId, ResolvedIcon>,
    /// Whether file watch mode is toggled globally.
    pub dev_watch_enabled: bool,
    /// Specific pack IDs that have live folder watching enabled.
    pub watched_pack_ids: Vec<String>,
    /// Last seen snapshot of files for watched folder packs: pack_id -> (path -> (mtime, len)).
    last_seen_snapshots: HashMap<String, HashMap<std::path::PathBuf, (std::time::SystemTime, u64)>>,
    /// Last time the filesystem was polled.
    pub(crate) last_poll_time: Option<std::time::Instant>,
    /// Monotonically increasing revision counter for cache busting.
    pub revision: u64,
}

impl Default for IconManager {
    fn default() -> Self {
        Self {
            active_stack: Vec::new(),
            palette: qymcad_scheme::dark(),
            cache: HashMap::new(),
            dev_watch_enabled: false,
            watched_pack_ids: Vec::new(),
            last_seen_snapshots: HashMap::new(),
            last_poll_time: None,
            revision: 0,
        }
    }
}

pub use super::id::{BUILTIN_ICON_THEMES, DEFAULT_THEME_ID};

/// Load all built-in icon themes embedded into the binary.
pub fn load_builtin_packs() -> Vec<IconPack> {
    BUILTIN_ICON_THEMES.iter().filter_map(|theme| IconPack::from_embedded_zip_bytes(theme.archive).ok()).collect()
}

/// Load a specific built-in icon pack by its manifest ID.
pub fn load_builtin_pack(id: &str) -> Option<IconPack> {
    BUILTIN_ICON_THEMES.iter().find(|theme| theme.id == id).and_then(|theme| IconPack::from_embedded_zip_bytes(theme.archive).ok())
}

/// Load the built-in default icon pack embedded into the binary.
pub fn load_default_pack() -> Option<IconPack> {
    load_builtin_pack(DEFAULT_THEME_ID)
}

impl IconManager {
    /// Update the active palette and invalidate resolution cache if palette changed.
    pub fn set_palette(&mut self, palette: qymcad_scheme::Palette) {
        if self.palette.fingerprint() != palette.fingerprint() {
            self.palette = palette;
            self.clear_cache();
        }
    }

    /// Read-only reference to the active palette.
    pub fn palette(&self) -> &qymcad_scheme::Palette {
        &self.palette
    }

    /// Create a new icon manager with the built-in default pack at the base of the cascade.
    pub fn new() -> Self {
        let mut mgr = Self::default();
        if let Some(def) = load_default_pack() {
            mgr.active_stack.push(def);
        }
        mgr
    }

    /// Read-only slice of active packs in priority order.
    pub fn active_stack(&self) -> &[IconPack] {
        &self.active_stack
    }

    /// Replace the active stack of packs. The built-in default pack is automatically appended
    /// to ensure 100% icon coverage across the entire system.
    pub fn set_active_stack(&mut self, mut packs: Vec<IconPack>) {
        if !packs.iter().any(|p| p.manifest.id == DEFAULT_THEME_ID) {
            if let Some(def) = load_default_pack() {
                packs.push(def);
            }
        }
        self.active_stack = packs;
        for id in &self.watched_pack_ids {
            if let Some(pack) = self.active_stack.iter().find(|p| &p.manifest.id == id) {
                if let Some(snap) = pack.directory_snapshot() {
                    self.last_seen_snapshots.insert(id.clone(), snap);
                }
            }
        }
        self.clear_cache();
    }

    /// Add an icon pack to the top of the priority stack (highest priority).
    pub fn push_top_pack(&mut self, pack: IconPack) {
        let pack_id = pack.manifest.id.clone();
        if self.watched_pack_ids.iter().any(|id| id == &pack_id) || self.dev_watch_enabled {
            if let Some(snap) = pack.directory_snapshot() {
                self.last_seen_snapshots.insert(pack_id.clone(), snap);
            }
        }
        self.active_stack.retain(|p| p.manifest.id != pack_id);
        self.active_stack.insert(0, pack);
        self.clear_cache();
    }

    /// Clear cached resolutions and bump revision counter.
    pub fn clear_cache(&mut self) {
        self.revision += 1;
        self.cache.clear();
    }

    /// Explicitly bump revision counter and clear cache.
    pub fn bump_revision(&mut self) {
        self.clear_cache();
    }

    /// Synchronize watched pack IDs.
    pub fn sync_watched_packs(&mut self, watched: &[String]) {
        self.watched_pack_ids = watched.to_vec();
        for id in watched {
            if !self.last_seen_snapshots.contains_key(id) {
                if let Some(pack) = self.active_stack.iter().find(|p| &p.manifest.id == id) {
                    if let Some(snap) = pack.directory_snapshot() {
                        self.last_seen_snapshots.insert(id.clone(), snap);
                    }
                }
            }
        }
    }

    /// Whether a specific pack ID is being watched.
    pub fn is_pack_watched(&self, pack_id: &str) -> bool {
        self.dev_watch_enabled || self.watched_pack_ids.iter().any(|id| id == pack_id)
    }

    /// Set watching status for a pack.
    pub fn set_pack_watching(&mut self, pack_id: &str, watch: bool) {
        if watch {
            if !self.watched_pack_ids.iter().any(|id| id == pack_id) {
                self.watched_pack_ids.push(pack_id.to_string());
            }
            if let Some(pack) = self.active_stack.iter().find(|p| p.manifest.id == pack_id) {
                if let Some(snap) = pack.directory_snapshot() {
                    self.last_seen_snapshots.insert(pack_id.to_string(), snap);
                }
            }
        } else {
            self.watched_pack_ids.retain(|id| id != pack_id);
            self.last_seen_snapshots.remove(pack_id);
        }
    }

    /// Check watched folder packs for file changes.
    /// If an SVG or manifest was edited, bumps revision, clears cache, and returns true.
    pub fn check_watched_directories(&mut self) -> bool {
        if !self.dev_watch_enabled && self.watched_pack_ids.is_empty() {
            return false;
        }

        let now = std::time::Instant::now();
        if let Some(last) = self.last_poll_time {
            if now.duration_since(last) < std::time::Duration::from_millis(250) {
                return false;
            }
        }
        self.last_poll_time = Some(now);

        let mut any_changed = false;
        for pack in &mut self.active_stack {
            if pack.is_directory() && (self.dev_watch_enabled || self.watched_pack_ids.iter().any(|id| id == &pack.manifest.id)) {
                if let Some(snap) = pack.directory_snapshot() {
                    if let Some(prev) = self.last_seen_snapshots.get(&pack.manifest.id) {
                        if prev != &snap {
                            any_changed = true;
                            self.last_seen_snapshots.insert(pack.manifest.id.clone(), snap);
                            let _ = pack.reload_manifest();
                        }
                    } else {
                        // First observation of this pack's directory
                        self.last_seen_snapshots.insert(pack.manifest.id.clone(), snap);
                    }
                }
            }
        }

        if any_changed {
            self.clear_cache();
        }
        any_changed
    }

    /// Resolve an icon through the cascade stack:
    ///
    /// 1. Searches each active pack from top to bottom.
    /// 2. If no custom pack provides the icon, returns from the built-in default SVG pack.
    pub fn resolve(&mut self, id: IconId) -> ResolvedIcon {
        if let Some(cached) = self.cache.get(&id) {
            return cached.clone();
        }

        let mut had_transient_read_failure = false;

        for pack in &self.active_stack {
            if let Some(data) = pack.get_svg_for_id(id) {
                let data = Arc::from(resolve_icon_tokens(&data, &self.palette));
                let res = ResolvedIcon { data, pack_id: pack.manifest.id.clone(), revision: self.revision, palette_fingerprint: self.palette.fingerprint() };
                if !had_transient_read_failure {
                    self.cache.insert(id, res.clone());
                }
                return res;
            } else if pack.has_icon_on_disk(id) {
                // The icon file exists on disk in this custom folder pack, but reading it failed
                // (e.g. transient Windows file sharing violation while an editor was saving).
                had_transient_read_failure = true;
            }
        }

        // The default pack embedded into the binary is always at the base of the active stack,
        // and contains all 106 icons. If somehow not found, provide a minimal fallback SVG.
        let fallback_raw = b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"none\" stroke=\"currentColor\"/></svg>";
        let data = Arc::from(resolve_icon_tokens(fallback_raw, &self.palette));
        let fallback = ResolvedIcon { data, pack_id: "builtin-fallback".to_string(), revision: self.revision, palette_fingerprint: self.palette.fingerprint() };
        // Avoid permanently poisoning cache if custom pack file on disk failed to read transiently
        if !had_transient_read_failure {
            self.cache.insert(id, fallback.clone());
        }
        fallback
    }
}

pub(crate) static GLOBAL_ICON_MANAGER: RwLock<Option<IconManager>> = RwLock::new(None);

/// Set or replace the global icon manager instance.
pub fn set_global_icon_manager(mgr: IconManager) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        *g = Some(mgr);
    }
}

/// Set or update the active palette on the global icon manager, clearing the cache on change.
pub fn set_global_icon_palette(palette: qymcad_scheme::Palette) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.set_palette(palette);
        } else {
            let mut mgr = IconManager::new();
            mgr.set_palette(palette);
            *g = Some(mgr);
        }
    }
}

/// Execute a closure with a read reference to the global icon manager if initialised.
pub fn with_global_icon_manager<R>(f: impl FnOnce(&IconManager) -> R) -> Option<R> {
    GLOBAL_ICON_MANAGER.read().ok().and_then(|g| g.as_ref().map(f))
}

/// Execute a closure with a mutable reference to the global icon manager if initialised.
pub fn with_global_icon_manager_mut<R>(f: impl FnOnce(&mut IconManager) -> R) -> Option<R> {
    GLOBAL_ICON_MANAGER.write().ok().and_then(|mut g| g.as_mut().map(f))
}

/// Resolve an `IconId` using the global priority stack.
pub fn resolve_global_icon(id: IconId) -> ResolvedIcon {
    if let Ok(g) = GLOBAL_ICON_MANAGER.read() {
        if let Some(mgr) = g.as_ref() {
            if let Some(cached) = mgr.cache.get(&id) {
                return cached.clone();
            }
        }
    }
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        let mgr = g.get_or_insert_with(IconManager::new);
        return mgr.resolve(id);
    }
    if let Some(def) = load_default_pack() {
        if let Some(data) = def.get_svg_for_id(id) {
            let pal = qymcad_scheme::dark();
            let data = Arc::from(resolve_icon_tokens(&data, &pal));
            return ResolvedIcon { data, pack_id: DEFAULT_THEME_ID.to_string(), revision: 0, palette_fingerprint: pal.fingerprint() };
        }
    }
    let pal = qymcad_scheme::dark();
    let data = Arc::from(resolve_icon_tokens(b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"none\" stroke=\"currentColor\"/></svg>", &pal));
    ResolvedIcon { data, pack_id: "builtin-fallback".to_string(), revision: 0, palette_fingerprint: pal.fingerprint() }
}

/// Invalidate all cached icon resolutions across active packs.
pub fn clear_global_icon_cache() {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.clear_cache();
        }
    }
}

/// Reload the global icon manager using the user's active theme IDs and known theme directories.
pub fn reload_active_icon_themes(active_ids: &[String], search_dirs: &[std::path::PathBuf]) {
    let mut all_packs = load_builtin_packs();
    for d in search_dirs {
        for p in super::bundle::discover_packs_in(d) {
            if !all_packs.iter().any(|existing: &IconPack| existing.manifest.id == p.manifest.id) {
                all_packs.push(p);
            }
        }
    }

    let mut stack = Vec::new();
    for id in active_ids {
        if id != DEFAULT_THEME_ID {
            if let Some(p) = all_packs.iter().find(|p| &p.manifest.id == id) {
                stack.push(p.clone());
            }
        }
    }

    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.set_active_stack(stack);
        } else {
            let mut mgr = IconManager::new();
            mgr.set_active_stack(stack);
            *g = Some(mgr);
        }
    }
}

/// Poll watched folder icon packs for filesystem changes.
/// Returns true if any files changed and icon cache was invalidated.
pub fn poll_watched_icon_packs() -> bool {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            return mgr.check_watched_directories();
        }
    }
    false
}

/// Check if any folder packs currently have watching enabled.
pub fn has_watched_icon_packs() -> bool {
    if let Ok(g) = GLOBAL_ICON_MANAGER.read() {
        if let Some(mgr) = g.as_ref() {
            return mgr.dev_watch_enabled || !mgr.watched_pack_ids.is_empty();
        }
    }
    false
}

/// Synchronize watched pack IDs to the global icon manager.
pub fn sync_global_watched_packs(watched: &[String]) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.sync_watched_packs(watched);
        } else {
            let mut mgr = IconManager::new();
            mgr.sync_watched_packs(watched);
            *g = Some(mgr);
        }
    }
}

/// Set watch state for a specific pack in the global icon manager.
pub fn set_global_pack_watching(pack_id: &str, watch: bool) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.set_pack_watching(pack_id, watch);
        } else {
            let mut mgr = IconManager::new();
            mgr.set_pack_watching(pack_id, watch);
            *g = Some(mgr);
        }
    }
}

/// Check if a specific pack is watched in the global icon manager.
pub fn is_global_pack_watched(pack_id: &str) -> bool {
    if let Ok(g) = GLOBAL_ICON_MANAGER.read() {
        if let Some(mgr) = g.as_ref() {
            return mgr.is_pack_watched(pack_id);
        }
    }
    false
}

/// Set global dev watch mode.
pub fn set_global_dev_watch(enabled: bool) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.dev_watch_enabled = enabled;
            if enabled {
                mgr.clear_cache();
            }
        } else {
            let mut mgr = IconManager::new();
            mgr.dev_watch_enabled = enabled;
            if enabled {
                mgr.clear_cache();
            }
            *g = Some(mgr);
        }
    }
}

/// Current global icon revision number.
pub fn get_global_icon_revision() -> u64 {
    if let Ok(g) = GLOBAL_ICON_MANAGER.read() {
        if let Some(mgr) = g.as_ref() {
            return mgr.revision;
        }
    }
    0
}

/// Preprocess SVG bytes by resolving CSS color variables (`var(--token, fallback)`)
/// and `currentColor` using the active palette.
pub fn resolve_icon_tokens(data: &[u8], palette: &qymcad_scheme::Palette) -> Vec<u8> {
    let has_var = data.windows(4).any(|w| w == b"var(");
    let has_current_color = data.windows(12).any(|w| w.eq_ignore_ascii_case(b"currentcolor"));

    if !has_var && !has_current_color {
        return data.to_vec();
    }

    let Ok(text) = std::str::from_utf8(data) else {
        return data.to_vec();
    };

    let stroke_hex = palette.format_icon_color("icon-stroke").unwrap_or_else(|| "#E0E0E0".to_string());

    let mut result = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(start_idx) = rest.find("var(") {
        result.push_str(&rest[..start_idx]);
        let after_var = &rest[start_idx + 4..];
        let mut depth = 0usize;
        let mut close_idx = None;
        for (idx, ch) in after_var.char_indices() {
            if ch == '(' {
                depth += 1;
            } else if ch == ')' {
                if depth == 0 {
                    close_idx = Some(idx);
                    break;
                }
                depth -= 1;
            }
        }

        if let Some(end_idx) = close_idx {
            let inner = &after_var[..end_idx];
            let replaced_color = if let Some((tok, fallback)) = inner.split_once(',') {
                let tok_trimmed = tok.trim();
                let fallback_trimmed = fallback.trim();
                palette.format_icon_color(tok_trimmed).unwrap_or_else(|| fallback_trimmed.to_string())
            } else {
                let tok_trimmed = inner.trim();
                palette.format_icon_color(tok_trimmed).unwrap_or_else(|| stroke_hex.clone())
            };
            result.push_str(&replaced_color);
            rest = &after_var[end_idx + 1..];
        } else {
            result.push_str("var(");
            rest = after_var;
        }
    }
    result.push_str(rest);

    if result.to_ascii_lowercase().contains("currentcolor") {
        let mut replaced = String::with_capacity(result.len());
        let mut search_from = 0;
        let lower = result.to_ascii_lowercase();
        while let Some(pos) = lower[search_from..].find("currentcolor") {
            let abs_pos = search_from + pos;
            replaced.push_str(&result[search_from..abs_pos]);
            replaced.push_str(&stroke_hex);
            search_from = abs_pos + 12;
        }
        replaced.push_str(&result[search_from..]);
        result = replaced;
    }

    result.into_bytes()
}
