//! Icon manager, priority cascade resolution, and global state.

use std::collections::HashMap;
use std::sync::RwLock;

use super::id::IconId;
use super::manifest::ColorMode;
use super::pack::IconPack;

/// The result of resolving an icon through the priority stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIcon {
    /// SVG file bytes.
    pub data: Vec<u8>,
    /// Color mode of the pack supplying the icon.
    pub color_mode: ColorMode,
    /// Identifier of the pack supplying the icon.
    pub pack_id: String,
    /// Monotonically increasing revision counter (invalidates texture cache on edits).
    pub revision: u64,
}

/// Central manager for icon themes and priority cascade.
#[derive(Default)]
pub struct IconManager {
    /// Active icon packs in descending priority (index 0 is highest priority).
    active_stack: Vec<IconPack>,
    /// Cache of resolved icons by `IconId`.
    cache: HashMap<IconId, ResolvedIcon>,
    /// Whether file watch mode is toggled globally.
    pub dev_watch_enabled: bool,
    /// Specific pack IDs that have live folder watching enabled.
    pub watched_pack_ids: Vec<String>,
    /// Last seen modification timestamp for watched folder packs: pack_id -> SystemTime.
    last_seen_mtimes: HashMap<String, std::time::SystemTime>,
    /// Last time the filesystem was polled.
    last_poll_time: Option<std::time::Instant>,
    /// Monotonically increasing revision counter for cache busting.
    pub revision: u64,
}

pub const DEFAULT_QICONS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/default.qicons"));

/// Load the built-in default icon pack embedded into the binary.
pub fn load_default_pack() -> Option<IconPack> {
    IconPack::from_zip_bytes(DEFAULT_QICONS).ok()
}

impl IconManager {
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
        if !packs.iter().any(|p| p.manifest.id == "default") {
            if let Some(def) = load_default_pack() {
                packs.push(def);
            }
        }
        self.active_stack = packs;
        self.clear_cache();
    }

    /// Add an icon pack to the top of the priority stack (highest priority).
    pub fn push_top_pack(&mut self, pack: IconPack) {
        self.active_stack.retain(|p| p.manifest.id != pack.manifest.id);
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
        } else {
            self.watched_pack_ids.retain(|id| id != pack_id);
            self.last_seen_mtimes.remove(pack_id);
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
        for pack in &self.active_stack {
            if pack.is_directory() && (self.dev_watch_enabled || self.watched_pack_ids.iter().any(|id| id == &pack.manifest.id)) {
                if let Some(mtime) = pack.latest_mtime() {
                    if let Some(prev) = self.last_seen_mtimes.get(&pack.manifest.id) {
                        if mtime > *prev {
                            any_changed = true;
                            self.last_seen_mtimes.insert(pack.manifest.id.clone(), mtime);
                        }
                    } else {
                        self.last_seen_mtimes.insert(pack.manifest.id.clone(), mtime);
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

        for pack in &self.active_stack {
            if let Some(mut data) = pack.get_svg_for_id(id) {
                if pack.manifest.color_mode == ColorMode::Monochrome {
                    match String::from_utf8(data) {
                        Ok(text) => {
                            if text.contains("currentColor") || text.contains("fill=\"#000000\"") || text.contains("fill=\"black\"") {
                                let replaced = text
                                    .replace("currentColor", "white")
                                    .replace("fill=\"#000000\"", "fill=\"white\"")
                                    .replace("fill=\"black\"", "fill=\"white\"");
                                data = replaced.into_bytes();
                            } else {
                                data = text.into_bytes();
                            }
                        }
                        Err(e) => {
                            data = e.into_bytes();
                        }
                    }
                }
                let res = ResolvedIcon {
                    data,
                    color_mode: pack.manifest.color_mode.clone(),
                    pack_id: pack.manifest.id.clone(),
                    revision: self.revision,
                };
                self.cache.insert(id, res.clone());
                return res;
            }
        }

        // The default pack embedded into the binary is always at the base of the active stack,
        // and contains all 106 icons. If somehow not found, provide a minimal fallback SVG.
        let fallback = ResolvedIcon {
            data: b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"none\" stroke=\"currentColor\"/></svg>".to_vec(),
            color_mode: ColorMode::Monochrome,
            pack_id: "builtin-fallback".to_string(),
            revision: self.revision,
        };
        self.cache.insert(id, fallback.clone());
        fallback
    }
}

static GLOBAL_ICON_MANAGER: RwLock<Option<IconManager>> = RwLock::new(None);

/// Set or replace the global icon manager instance.
pub fn set_global_icon_manager(mgr: IconManager) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        *g = Some(mgr);
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
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            return mgr.resolve(id);
        }
    }
    if let Some(def) = load_default_pack() {
        if let Some(data) = def.get_svg_for_id(id) {
            return ResolvedIcon {
                data,
                color_mode: ColorMode::Monochrome,
                pack_id: "default".to_string(),
                revision: 0,
            };
        }
    }
    ResolvedIcon {
        data: b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"none\" stroke=\"currentColor\"/></svg>".to_vec(),
        color_mode: ColorMode::Monochrome,
        pack_id: "builtin-fallback".to_string(),
        revision: 0,
    }
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
    let mut all_packs = Vec::new();
    for d in search_dirs {
        for p in super::bundle::discover_packs_in(d) {
            if !all_packs.iter().any(|existing: &IconPack| existing.manifest.id == p.manifest.id) {
                all_packs.push(p);
            }
        }
    }

    let mut stack = Vec::new();
    for id in active_ids {
        if let Some(p) = all_packs.iter().find(|p| &p.manifest.id == id) {
            stack.push(p.clone());
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
        }
    }
}

/// Set watch state for a specific pack in the global icon manager.
pub fn set_global_pack_watching(pack_id: &str, watch: bool) {
    if let Ok(mut g) = GLOBAL_ICON_MANAGER.write() {
        if let Some(mgr) = g.as_mut() {
            mgr.set_pack_watching(pack_id, watch);
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
