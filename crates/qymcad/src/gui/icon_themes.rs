//! ICON THEME CONFIGURATION AND MANAGEMENT.
//!
//! Provides discovery, priority-cascade configuration, and bundle packaging
//! for CAD icon packs according to RFC-icon-theme-system.
use egui::Color32;
use egui_phosphor::regular as ph;
use qymcad_ui_state::icons::{
    clean_directory_icon, clean_directory_icons, clear_global_icon_cache, directory_has_cleanable_icons, discover_packs_detailed, inspect_pack_directory_for_mode, load_default_pack, package_bundle,
    reload_active_icon_themes, BundleFormat, CleanIconResult, ColorMode, DiscoveryError, IconId, IconManifest, IconPack, PackSource, PackageType, ValidationReport, ALL_ICONS,
};
use qymcad_ui_state::{Settings, WinCtx};
use std::path::{Path, PathBuf};

/// The directory for user-installed icon themes in the application config folder.
pub(crate) fn user_themes_dir() -> Option<PathBuf> {
    qymcad_paths::config("icon_themes")
}

/// The directory for bundled icon themes.
pub(crate) fn bundled_themes_dir() -> PathBuf {
    let dev = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icon-themes"));
    if dev.exists() {
        return dev;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let beside = parent.join("assets/icon-themes");
            if beside.exists() {
                return beside;
            }
        }
    }
    PathBuf::from("assets/icon-themes")
}

/// All search directories for icon themes (user directories followed by bundled directory).
pub(crate) fn all_theme_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(user_dir) = user_themes_dir() {
        if !dirs.contains(&user_dir) {
            dirs.push(user_dir);
        }
    }
    if let Some(data_dir) = qymcad_paths::data("icon_themes") {
        if !dirs.contains(&data_dir) {
            dirs.push(data_dir);
        }
    }
    if let Some(d) = qymcad_paths::dirs() {
        let xdg_data = d.data_dir().join("icon_themes");
        if !dirs.contains(&xdg_data) {
            dirs.push(xdg_data);
        }
        let xdg_config = d.config_dir().join("icon_themes");
        if !dirs.contains(&xdg_config) {
            dirs.push(xdg_config);
        }
    }
    let bundled = bundled_themes_dir();
    if !dirs.contains(&bundled) {
        dirs.push(bundled);
    }
    dirs
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThemeEntrySignature {
    path: PathBuf,
    modified: Option<std::time::SystemTime>,
    len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ThemeDirSignature {
    dir: PathBuf,
    entries: Vec<ThemeEntrySignature>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DiscoveredThemes {
    pub packs: Vec<IconPack>,
    pub errors: Vec<DiscoveryError>,
}

#[derive(Clone, Debug)]
struct CachedDiscovery {
    signatures: Vec<ThemeDirSignature>,
    themes: DiscoveredThemes,
}

static DISCOVERY_CACHE: std::sync::RwLock<Option<std::collections::HashMap<Vec<PathBuf>, CachedDiscovery>>> = std::sync::RwLock::new(None);

#[cfg(test)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiscoveryCacheStats {
    pub hits: usize,
    pub misses: usize,
}

#[cfg(test)]
static DISCOVERY_HITS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static DISCOVERY_MISSES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn discovery_cache_stats() -> DiscoveryCacheStats {
    DiscoveryCacheStats { hits: DISCOVERY_HITS.load(std::sync::atomic::Ordering::Relaxed), misses: DISCOVERY_MISSES.load(std::sync::atomic::Ordering::Relaxed) }
}

#[cfg(test)]
pub(crate) fn clear_discovery_cache_for_test() {
    if let Ok(mut guard) = DISCOVERY_CACHE.write() {
        if let Some(map) = guard.as_mut() {
            map.clear();
        }
    }
    DISCOVERY_HITS.store(0, std::sync::atomic::Ordering::Relaxed);
    DISCOVERY_MISSES.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Invalidate cached discovery results across all searched theme directories.
pub(crate) fn invalidate_theme_discovery_cache() {
    if let Ok(mut guard) = DISCOVERY_CACHE.write() {
        if let Some(map) = guard.as_mut() {
            map.clear();
        }
    }
}

fn compute_dir_signature(dir: &Path) -> ThemeDirSignature {
    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("manifest.ron");
                if let Ok(meta) = std::fs::metadata(&manifest_path) {
                    entries.push(ThemeEntrySignature { path: manifest_path, modified: meta.modified().ok(), len: meta.len() });
                }
            } else if path.extension().is_some_and(|ext| ext == "qicons" || ext == "zip") {
                if let Ok(meta) = std::fs::metadata(&path) {
                    entries.push(ThemeEntrySignature { path, modified: meta.modified().ok(), len: meta.len() });
                }
            }
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    ThemeDirSignature { dir: dir.to_path_buf(), entries }
}

fn discover_theme_packs_in_dirs(dirs: &[PathBuf]) -> DiscoveredThemes {
    let current_signatures: Vec<ThemeDirSignature> = dirs.iter().map(|d| compute_dir_signature(d)).collect();

    if let Ok(guard) = DISCOVERY_CACHE.read() {
        if let Some(map) = guard.as_ref() {
            if let Some(cached) = map.get(dirs) {
                if cached.signatures == current_signatures {
                    #[cfg(test)]
                    DISCOVERY_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return cached.themes.clone();
                }
            }
        }
    }

    #[cfg(test)]
    DISCOVERY_MISSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let mut packs = Vec::new();
    let mut errors = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();
    for dir in dirs {
        let report = discover_packs_detailed(dir);
        for pack in report.packs {
            if seen_ids.insert(pack.manifest.id.clone()) {
                packs.push(pack);
            }
        }
        errors.extend(report.errors);
    }

    let themes = DiscoveredThemes { packs, errors };

    if let Ok(mut guard) = DISCOVERY_CACHE.write() {
        let map = guard.get_or_insert_with(std::collections::HashMap::new);
        map.insert(dirs.to_vec(), CachedDiscovery { signatures: current_signatures, themes: themes.clone() });
    }

    themes
}

/// Apply the active icon packs from settings into the global icon manager.
pub(crate) fn apply_icon_themes(set: &Settings) {
    let dirs = all_theme_dirs();
    reload_active_icon_themes(&set.active_icon_packs, &dirs);
}

/// Render a compact, colored visual badge indicating the format and provenance of an icon bundle.
pub(crate) fn draw_bundle_format_badge(ui: &mut egui::Ui, format: BundleFormat, is_tampered: bool) {
    let visuals = ui.visuals();
    let (icon, label_key, bg, fg) = if is_tampered {
        (ph::WARNING, "bundle-format-tampered", visuals.error_fg_color.linear_multiply(0.20), visuals.error_fg_color)
    } else {
        match format {
            BundleFormat::Directory => (ph::FOLDER_OPEN, "bundle-format-folder", visuals.warn_fg_color.linear_multiply(0.18), visuals.warn_fg_color),
            BundleFormat::Archive => (ph::PACKAGE, "bundle-format-archive", visuals.hyperlink_color.linear_multiply(0.18), visuals.hyperlink_color),
            BundleFormat::VerifiedArchive => (ph::SHIELD_CHECK, "bundle-format-verified", visuals.selection.bg_fill.linear_multiply(0.22), visuals.selection.bg_fill),
            BundleFormat::Embedded => (ph::GEAR, "bundle-format-embedded", visuals.faint_bg_color, visuals.weak_text_color()),
        }
    };

    egui::Frame::NONE.fill(bg).corner_radius(3.0).inner_margin(egui::Margin::symmetric(5, 2)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.label(egui::RichText::new(icon).color(fg).small());
            ui.label(egui::RichText::new(crate::i18n::tr(label_key)).color(fg).small().strong());
        });
    });
}

fn draw_pack_icon(ui: &mut egui::Ui, pack: &IconPack, size: f32) {
    draw_pack_icon_bytes(ui, pack, size, pack.get_pack_icon_svg().into(), 0);
}

fn draw_pack_icon_bytes(ui: &mut egui::Ui, pack: &IconPack, size: f32, bytes: egui::load::Bytes, generation: u64) {
    let uri = format!("bytes://pack-icon/{}/r{}-g{generation}.svg", pack.manifest.id, qymcad_ui_state::icons::get_global_icon_revision());
    let id_key = egui::Id::new("pack_icon_prev_uri").with(&pack.manifest.id);
    let to_forget = ui.data_mut(|d| {
        let prev = d.get_temp::<String>(id_key);
        if prev.as_ref() != Some(&uri) {
            d.insert_temp(id_key, uri.clone());
            prev
        } else {
            None
        }
    });
    if let Some(prev) = to_forget {
        ui.ctx().forget_image(&prev);
    }
    ui.add(egui::Image::from_bytes(uri, bytes).fit_to_exact_size(egui::vec2(size, size)));
}

type ManagerIconPreview = Result<Option<egui::load::Bytes>, String>;

struct GalleryRowResponse {
    rect: egui::Rect,
    clean_clicked: bool,
    path_copied: bool,
}

fn draw_gallery_icon_row(ui: &mut egui::Ui, pack: &IconPack, id: IconId, icon: &ManagerIconPreview, cleanable: bool, generation: u64, copied_path: Option<&str>) -> GalleryRowResponse {
    let relative_path = id.relative_path();
    let archive_path = format!("icons/{relative_path}.svg");
    let name = relative_path.rsplit('/').next().unwrap_or(relative_path);
    let row_width = ui.available_width();
    let mut clean_clicked = false;
    let mut path_copied = false;
    let is_copied = copied_path == Some(archive_path.as_str());
    let rect = egui::Frame::NONE
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            let inner_width = (row_width - 20.0).max(0.0);
            ui.set_width(inner_width);
            ui.horizontal(|ui| {
                let (preview, _) = ui.allocate_exact_size(egui::vec2(56.0, 56.0), egui::Sense::hover());
                ui.painter().rect_filled(preview, 4.0, ui.visuals().extreme_bg_color);
                ui.painter().rect_stroke(preview, 4.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
                match &icon {
                    Ok(Some(svg_data)) => {
                        let uri = format!("bytes://mgr/{}/r{}-g{generation}/{}.svg", pack.manifest.id, qymcad_ui_state::icons::get_global_icon_revision(), relative_path);
                        let id_key = egui::Id::new("gallery_icon_prev_uri").with((&pack.manifest.id, id));
                        let to_forget = ui.data_mut(|d| {
                            let prev = d.get_temp::<String>(id_key);
                            if prev.as_ref() != Some(&uri) {
                                d.insert_temp(id_key, uri.clone());
                                prev
                            } else {
                                None
                            }
                        });
                        if let Some(prev) = to_forget {
                            ui.ctx().forget_image(&prev);
                        }
                        let mut image = egui::Image::from_bytes(uri, svg_data.clone()).fit_to_exact_size(egui::vec2(48.0, 48.0));
                        if pack.manifest.color_mode == ColorMode::Monochrome {
                            image = image.tint(ui.visuals().text_color());
                        }
                        ui.put(preview.shrink(4.0), image);
                    }
                    Err(_) => {
                        ui.painter().text(preview.center(), egui::Align2::CENTER_CENTER, ph::WARNING, egui::FontId::proportional(22.0), ui.visuals().warn_fg_color);
                    }
                    Ok(None) => {}
                }

                let text_width = (inner_width - 56.0 - ui.spacing().item_spacing.x).max(0.0);
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    ui.label(egui::RichText::new(name).strong());

                    let path_text = egui::RichText::new(&archive_path).monospace().small();
                    let path_label = if is_copied { path_text.color(ui.visuals().hyperlink_color) } else { path_text.weak() };
                    ui.horizontal_wrapped(|ui| {
                        let resp = ui.add(egui::Label::new(path_label).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if is_copied {
                            crate::i18n::tr("icon-mgr-path-copied")
                        } else {
                            crate::i18n::tr("icon-mgr-copy-path")
                        });

                        if resp.clicked() {
                            ui.output_mut(|output| output.commands.push(egui::OutputCommand::CopyText(archive_path.clone())));
                            path_copied = true;
                        }

                        if is_copied {
                            egui::Frame::NONE
                                .fill(ui.visuals().window_fill())
                                .stroke(egui::Stroke::new(1.0, ui.visuals().hyperlink_color))
                                .corner_radius(4.0)
                                .inner_margin(egui::Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    ui.label(egui::RichText::new(format!("{} {}", ph::CHECK, crate::i18n::tr("icon-mgr-path-copied"))).small().color(ui.visuals().hyperlink_color));
                                });
                        }
                    });
                    match &icon {
                        Ok(Some(_)) => {
                            ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-gallery-present")).small().weak());
                        }
                        Ok(None) => {
                            ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-gallery-missing")).small().weak());
                        }
                        Err(reason) => {
                            let error = format!("{}: {reason}", crate::i18n::tr("icon-mgr-gallery-invalid"));
                            ui.add(egui::Label::new(egui::RichText::new(error).small().color(ui.visuals().warn_fg_color)).wrap());
                            if cleanable && !reason.starts_with("monochrome ") && ui.button(format!("{} {}", ph::BROOM, crate::i18n::tr("icon-mgr-clean-icon"))).clicked() {
                                clean_clicked = true;
                            }
                        }
                    }
                });
            });
        })
        .response
        .rect;
    GalleryRowResponse { rect, clean_clicked, path_copied }
}

/// Developer Packager modal state kept in UI context.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PackagerDialogState {
    pub is_open: bool,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub license: String,
    pub description: String,
    pub is_monochrome: bool,
    pub source_dir: String,
    pub output_file: String,
    pub message: Option<String>,
    pub is_error: bool,
    pub report: Option<ValidationReport>,
}

impl Default for PackagerDialogState {
    fn default() -> Self {
        Self {
            is_open: false,
            id: "my-cad-theme".into(),
            name: "My CAD Theme".into(),
            version: "1.0.0".into(),
            author: "".into(),
            license: "LGPL-2.1-or-later".into(),
            description: "Custom CAD vector icons".into(),
            is_monochrome: false,
            source_dir: "".into(),
            output_file: "".into(),
            message: None,
            is_error: false,
            report: None,
        }
    }
}

/// Draw the Icon Themes section in the Settings window (Appearance tab).
pub(crate) fn icon_theme_section(wc: &mut WinCtx, ui: &mut egui::Ui, ctx: &egui::Context) {
    ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-themes-title")).strong());
    ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-themes-desc")).small().weak());
    ui.add_space(4.0);

    let mut all_packs = discover_theme_packs_in_dirs(&all_theme_dirs()).packs;
    if !all_packs.iter().any(|p| p.manifest.id == "default") {
        if let Some(def) = load_default_pack() {
            all_packs.push(def);
        }
    }

    // Single-line active cascade representation
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-active-chain")).strong());

        for id in &wc.set.active_icon_packs {
            if let Some(pack) = all_packs.iter().find(|p| &p.manifest.id == id) {
                ui.label(egui::RichText::new(pack.manifest.name_for_locale(&crate::i18n::language())).strong());
                draw_bundle_format_badge(ui, pack.format(), pack.is_tampered);
                ui.label(egui::RichText::new(ph::ARROW_RIGHT).weak());
            } else {
                ui.label(egui::RichText::new(id).weak());
                ui.label(egui::RichText::new(ph::ARROW_RIGHT).weak());
            }
        }

        // Base fallback is always the built-in SVG bundle
        let base_name = all_packs
            .iter()
            .find(|pack| pack.manifest.id == "default")
            .map(|pack| pack.manifest.name_for_locale(&crate::i18n::language()).to_string())
            .unwrap_or_else(|| crate::i18n::tr("settings-icon-themes-base"));
        ui.label(egui::RichText::new(base_name).strong());
        draw_bundle_format_badge(ui, BundleFormat::Embedded, false);
    });

    ui.add_space(6.0);
    if ui.button(format!("{} {}", ph::PALETTE, crate::i18n::tr("settings-open-icon-manager"))).clicked() {
        open_icon_manager(ctx);
    }
}

/// In-app developer modal dialog for packaging an icon bundle into `.qicons`.
fn draw_packager_modal(ctx: &egui::Context, state: &mut PackagerDialogState) {
    let mut open = state.is_open;
    egui::Window::new(crate::i18n::tr("icon-packager-title")).open(&mut open).collapsible(false).resizable(true).default_width(450.0).show(ctx, |ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("icon-packager-desc")).small().weak());
        ui.add_space(4.0);

        egui::Grid::new("packager_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Theme ID:");
            ui.text_edit_singleline(&mut state.id);
            ui.end_row();

            ui.label("Theme Name:");
            ui.text_edit_singleline(&mut state.name);
            ui.end_row();

            ui.label("Version:");
            ui.text_edit_singleline(&mut state.version);
            ui.end_row();

            ui.label("Author:");
            ui.text_edit_singleline(&mut state.author);
            ui.end_row();

            ui.label("License:");
            ui.text_edit_singleline(&mut state.license);
            ui.end_row();

            ui.label("Description:");
            ui.text_edit_singleline(&mut state.description);
            ui.end_row();

            ui.label("Monochrome:");
            ui.checkbox(&mut state.is_monochrome, "Theme adapts to UI foreground");
            ui.end_row();

            ui.label("Source Folder:");
            ui.text_edit_singleline(&mut state.source_dir);
            ui.end_row();

            ui.label("Output .qicons Path:");
            ui.text_edit_singleline(&mut state.output_file);
            ui.end_row();
        });

        ui.add_space(8.0);

        if let Some(msg) = &state.message {
            let color = if state.is_error { Color32::RED } else { Color32::GREEN };
            ui.label(egui::RichText::new(msg).color(color));
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            if ui.button(crate::i18n::tr("icon-packager-inspect-btn")).clicked() {
                let source_path = PathBuf::from(state.source_dir.trim());
                if !source_path.exists() {
                    state.message = Some(format!("Source folder does not exist: {}", source_path.display()));
                    state.is_error = true;
                    state.report = None;
                } else {
                    let color_mode = if state.is_monochrome { ColorMode::Monochrome } else { ColorMode::Universal };
                    match inspect_pack_directory_for_mode(&source_path, color_mode) {
                        Ok(rep) => {
                            if rep.has_issues() {
                                state.message =
                                    Some(format!("Validation found {} issue(s): {} rejected, {} extraneous", rep.rejected.len() + rep.extraneous.len(), rep.rejected.len(), rep.extraneous.len()));
                                state.is_error = !rep.rejected.is_empty();
                            } else {
                                state.message = Some(crate::i18n::tr("icon-packager-all-valid"));
                                state.is_error = false;
                            }
                            state.report = Some(rep);
                        }
                        Err(e) => {
                            state.message = Some(format!("Inspection failed: {e}"));
                            state.is_error = true;
                            state.report = None;
                        }
                    }
                }
            }

            if ui.button(crate::i18n::tr("icon-packager-build-btn")).clicked() {
                let source_path = PathBuf::from(state.source_dir.trim());
                let output_path = PathBuf::from(state.output_file.trim());

                if !source_path.exists() {
                    state.message = Some(format!("Source folder does not exist: {}", source_path.display()));
                    state.is_error = true;
                } else if state.output_file.trim().is_empty() {
                    state.message = Some("Output .qicons path cannot be empty".into());
                    state.is_error = true;
                } else {
                    let result = (|| {
                        let translations = if source_path.join("manifest.ron").exists() { IconPack::from_directory(&source_path)?.manifest.translations } else { Default::default() };
                        let manifest = IconManifest {
                            package_type: PackageType::IconTheme,
                            id: state.id.trim().to_string(),
                            name: state.name.trim().to_string(),
                            version: state.version.trim().to_string(),
                            author: state.author.trim().to_string(),
                            license: state.license.trim().to_string(),
                            description: state.description.trim().to_string(),
                            color_mode: if state.is_monochrome { ColorMode::Monochrome } else { ColorMode::Universal },
                            translations,
                            verified: true,
                        };
                        manifest.validate()?;
                        package_bundle(&source_path, &manifest, &output_path)
                    })();

                    match result {
                        Ok(rep) => {
                            if rep.included.is_empty() {
                                state.message = Some(crate::i18n::tr("icon-packager-no-icons"));
                                state.is_error = true;
                            } else {
                                invalidate_theme_discovery_cache();
                                let cov_str = rep.included.len().to_string();
                                let path_str = output_path.display().to_string();
                                state.message = Some(crate::i18n::trn("icon-packager-success", &[("count", &cov_str), ("path", &path_str)]));
                                state.is_error = false;
                            }
                            state.report = Some(rep);
                        }
                        Err(e) => {
                            state.message = Some(format!("Packaging failed: {e}"));
                            state.is_error = true;
                        }
                    }
                }
            }

            if ui.button(crate::i18n::tr("nav-cancel")).clicked() {
                state.is_open = false;
            }
        });

        // Diagnostic validation report section
        if let Some(rep) = &state.report {
            ui.separator();
            let (cov, total) = rep.coverage();
            let pct = rep.coverage_percent();
            let cov_str = cov.to_string();
            let total_str = total.to_string();
            let pct_str = pct.to_string();

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(crate::i18n::trn("icon-packager-coverage", &[("included", &cov_str), ("total", &total_str), ("percent", &pct_str)])).strong());
            });

            // Category badges
            ui.horizontal_wrapped(|ui| {
                for (cat, inc, cat_total) in rep.category_breakdown() {
                    let text = format!("{cat}: {inc}/{cat_total}");
                    ui.label(egui::RichText::new(text).small().weak());
                }
            });

            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                // Rejected files (critical errors)
                if !rep.rejected.is_empty() {
                    ui.add_space(4.0);
                    let title = crate::i18n::tr1("icon-packager-rejected-title", "count", &rep.rejected.len().to_string());
                    ui.label(egui::RichText::new(title).strong().color(Color32::RED));
                    ui.label(egui::RichText::new(crate::i18n::tr("icon-packager-rejected-desc")).small().weak());
                    for (path, reason) in &rep.rejected {
                        ui.label(egui::RichText::new(format!("  - {path}: {reason}")).color(ui.visuals().error_fg_color).small());
                    }
                }

                // Extraneous files (warnings)
                if !rep.extraneous.is_empty() {
                    ui.add_space(4.0);
                    let title = crate::i18n::tr1("icon-packager-extraneous-title", "count", &rep.extraneous.len().to_string());
                    ui.label(egui::RichText::new(title).strong().color(ui.visuals().warn_fg_color));
                    ui.label(egui::RichText::new(crate::i18n::tr("icon-packager-extraneous-desc")).small().weak());
                    for file in &rep.extraneous {
                        ui.label(egui::RichText::new(format!("  - {file}")).color(ui.visuals().warn_fg_color).small());
                    }
                }

                // Included icons list (collapsible)
                if !rep.included.is_empty() {
                    ui.add_space(4.0);
                    let inc_title = crate::i18n::tr1("icon-packager-included-title", "count", &cov_str);
                    egui::CollapsingHeader::new(egui::RichText::new(inc_title).color(Color32::GREEN)).default_open(rep.rejected.is_empty() && rep.extraneous.is_empty()).show(ui, |ui| {
                        for id in &rep.included {
                            ui.label(egui::RichText::new(format!("  {} {}", ph::CHECK, id.relative_path())).small());
                        }
                    });
                }

                // Missing icons list (collapsible)
                if !rep.missing.is_empty() {
                    ui.add_space(4.0);
                    let miss_title = crate::i18n::tr1("icon-packager-missing-title", "count", &rep.missing.len().to_string());
                    egui::CollapsingHeader::new(egui::RichText::new(miss_title).weak()).default_open(false).show(ui, |ui| {
                        ui.label(egui::RichText::new(crate::i18n::tr("icon-packager-missing-desc")).small().weak());
                        for id in &rep.missing {
                            ui.label(egui::RichText::new(format!("  - {}", id.relative_path())).weak().small());
                        }
                    });
                }
            });
        }
    });
    state.is_open = open;
}

/// Active tab in the Icon Theme Manager window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconManagerTab {
    Readme,
    Gallery,
}

/// State for the dedicated Icon Theme Manager window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct IconManagerState {
    pub is_open: bool,
    pub selected_pack_id: String,
    pub active_tab: IconManagerTab,
    pub search_query: String,
    pub category_filter: String,
    clean_notice: Option<CleanNotice>,
    copied_path: Option<(String, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
struct CleanNotice {
    pack_id: String,
    text: String,
    is_error: bool,
    details: Vec<String>,
}

impl Default for IconManagerState {
    fn default() -> Self {
        Self {
            is_open: false,
            selected_pack_id: "freecad-classic".into(),
            active_tab: IconManagerTab::Readme,
            search_query: String::new(),
            category_filter: "all".into(),
            clean_notice: None,
            copied_path: None,
        }
    }
}

#[derive(Clone)]
struct ManagerArchiveCache {
    path: PathBuf,
    file_size: u64,
    modified: Option<std::time::SystemTime>,
    revision: u64,
    preview: std::sync::Arc<ManagerPackPreview>,
}

#[derive(Clone)]
struct ManagerDirectoryCache {
    path: PathBuf,
    snapshot: std::sync::Arc<std::collections::HashMap<PathBuf, (std::time::SystemTime, u64)>>,
    revision: u64,
    preview: std::sync::Arc<ManagerPackPreview>,
}

#[derive(Clone)]
struct ManagerPreviewImage {
    bytes: egui::load::Bytes,
    extension: &'static str,
}

struct ManagerPackPreview {
    coverage: usize,
    invalid_icons: usize,
    has_cleanable_icons: bool,
    image_generation: u64,
    readmes: std::collections::HashMap<String, String>,
    preview_image: Option<ManagerPreviewImage>,
    pack_icon: egui::load::Bytes,
    icons: std::collections::HashMap<IconId, ManagerIconPreview>,
}

fn manager_archive_preview(ctx: &egui::Context, pack: &IconPack) -> Option<std::sync::Arc<ManagerPackPreview>> {
    let PackSource::Archive(path) = &pack.source else {
        return None;
    };
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok();
    let revision = qymcad_ui_state::icons::get_global_icon_revision();
    let cache_id = egui::Id::new("icon_manager_archive_cache");
    if let Some(cached) = ctx.data(|data| data.get_temp::<ManagerArchiveCache>(cache_id)) {
        if cached.path == *path && cached.file_size == metadata.len() && cached.modified == modified && cached.revision == revision {
            return Some(cached.preview);
        }
    }
    let snapshot = pack.archive_snapshot().ok()?;
    let mut coverage = 0;
    let mut invalid_icons = 0;
    let mut icons = std::collections::HashMap::new();
    for &id in ALL_ICONS {
        let available = snapshot.get_svg_for_id(id).is_some();
        if available {
            coverage += 1;
        }
        let inspected = snapshot.inspect_svg_for_id(id).map(|data| data.map(egui::load::Bytes::from));
        if inspected.is_err() {
            invalid_icons += 1;
        }
        icons.insert(id, inspected);
    }
    let preview = std::sync::Arc::new(ManagerPackPreview {
        coverage,
        invalid_icons,
        has_cleanable_icons: false,
        image_generation: 0,
        readmes: crate::i18n::available().into_iter().map(|(locale, _)| (locale.clone(), snapshot.get_readme_for_locale(&locale))).collect(),
        preview_image: snapshot.get_preview_image().map(|(bytes, extension)| ManagerPreviewImage { bytes: bytes.into(), extension }),
        pack_icon: snapshot.get_pack_icon_svg().into(),
        icons,
    });
    ctx.data_mut(|data| {
        data.insert_temp(cache_id, ManagerArchiveCache { path: path.clone(), file_size: metadata.len(), modified, revision, preview: preview.clone() });
    });
    Some(preview)
}

fn manager_directory_preview(ctx: &egui::Context, pack: &IconPack) -> Option<std::sync::Arc<ManagerPackPreview>> {
    let PackSource::Directory(path) = &pack.source else {
        return None;
    };
    let mut snapshot = pack.directory_snapshot()?;
    for name in ["README.md", "readme.md", "README.txt", "description.md", "preview.svg", "preview.png", "preview.webp"] {
        let file = path.join(name);
        if let Ok(metadata) = std::fs::metadata(file) {
            snapshot.insert(PathBuf::from(name), (metadata.modified().unwrap_or(std::time::UNIX_EPOCH), metadata.len()));
        }
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("README.") && name.ends_with(".md") {
                if let Ok(metadata) = entry.metadata() {
                    snapshot.insert(PathBuf::from(name.as_ref()), (metadata.modified().unwrap_or(std::time::UNIX_EPOCH), metadata.len()));
                }
            }
        }
    }
    let revision = qymcad_ui_state::icons::get_global_icon_revision();
    let cache_id = egui::Id::new("icon_manager_directory_cache");
    let cached = ctx.data(|data| data.get_temp::<ManagerDirectoryCache>(cache_id));
    if let Some(cache) = &cached {
        if cache.path == *path && *cache.snapshot == snapshot && cache.revision == revision {
            return Some(cache.preview.clone());
        }
    }

    let mut coverage = 0;
    let mut invalid_icons = 0;
    let mut icons = std::collections::HashMap::new();
    for &id in ALL_ICONS {
        let icon_path = PathBuf::from(format!("icons/{}.svg", id.relative_path()));
        if snapshot.get(&icon_path).is_some_and(|(_, len)| *len > 0) {
            coverage += 1;
        }
        let inspected = pack.inspect_svg_for_id(id).map(|data| data.map(egui::load::Bytes::from));
        if inspected.is_err() {
            invalid_icons += 1;
        }
        icons.insert(id, inspected);
    }
    let image_generation = cached.map_or(1, |cache| cache.preview.image_generation.wrapping_add(1));
    let has_cleanable_icons = directory_has_cleanable_icons(pack).unwrap_or(false);
    let preview = std::sync::Arc::new(ManagerPackPreview {
        coverage,
        invalid_icons,
        has_cleanable_icons,
        image_generation,
        readmes: crate::i18n::available().into_iter().map(|(locale, _)| (locale.clone(), pack.get_readme_for_locale(&locale))).collect(),
        preview_image: pack.get_preview_image().map(|(bytes, extension)| ManagerPreviewImage { bytes: bytes.into(), extension }),
        pack_icon: pack.get_pack_icon_svg().into(),
        icons,
    });
    ctx.data_mut(|data| {
        data.insert_temp(cache_id, ManagerDirectoryCache { path: path.clone(), snapshot: std::sync::Arc::new(snapshot), revision, preview: preview.clone() });
    });
    Some(preview)
}

/// Request to open the Icon Theme Manager window.
pub(crate) fn open_icon_manager(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<IconManagerState>(egui::Id::new("icon_manager_window"));
        state.is_open = true;
    });
}

fn manager_theme_card(ui: &mut egui::Ui, id: &str, selected: bool, content: impl FnOnce(&mut egui::Ui) -> Option<egui::Rect>) -> (bool, egui::Rect) {
    let fill = if selected { ui.visuals().selection.bg_fill.linear_multiply(0.22) } else { ui.visuals().faint_bg_color };
    let (rect, background) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 68.0), egui::Sense::click());
    ui.painter().rect_filled(rect, 6.0, fill);
    let inner = rect.shrink2(egui::vec2(8.0, 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
    let action_rect = content(&mut child);
    let mut selection_rect = rect;
    if let Some(action_rect) = action_rect {
        selection_rect.max.x = action_rect.left();
    }
    let foreground = ui.interact(selection_rect, ui.id().with(("theme_card", id)), egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    (foreground.clicked() || background.clicked(), rect)
}

fn manager_sidebar_shell(ui: &mut egui::Ui, actions: impl FnOnce(&mut egui::Ui)) -> egui::ScrollArea {
    egui::Panel::bottom("icon_manager_actions").resizable(false).exact_size(44.0).show(ui, actions);
    egui::ScrollArea::vertical().id_salt("mgr_sidebar_scroll").auto_shrink([false, false])
}

fn draw_icon_manager_actions(ui: &mut egui::Ui) {
    ui.add_space(6.0);
    if let Some(user_dir) = user_themes_dir() {
        if ui.add_sized([ui.available_width(), 28.0], egui::Button::new(format!("{} {}", ph::FOLDER_OPEN, crate::i18n::tr("settings-icon-open-folder")))).clicked() {
            let _ = std::fs::create_dir_all(&user_dir);
            let (bin, args) = crate::gui::reveal_command(ui.ctx().os(), &user_dir);
            let _ = crate::system::start(bin, &args);
        }
    }
}

fn open_packager_for_directory(ctx: &egui::Context, pack: &IconPack, source: &std::path::Path) {
    ctx.data_mut(|data| {
        let state = data.get_temp_mut_or_default::<PackagerDialogState>(egui::Id::new("icon_packager_dialog"));
        state.is_open = true;
        state.id = pack.manifest.id.clone();
        state.name = pack.manifest.name.clone();
        state.version = pack.manifest.version.clone();
        state.author = pack.manifest.author.clone();
        state.license = pack.manifest.license.clone();
        state.description = pack.manifest.description.clone();
        state.is_monochrome = pack.manifest.color_mode == ColorMode::Monochrome;
        state.source_dir = source.display().to_string();
        state.output_file = source.with_extension("qicons").display().to_string();
        state.message = None;
        state.report = None;
        state.is_error = false;
    });
}

/// Draw the dedicated Icon Theme Manager window.
pub(crate) fn draw_icon_manager_window(ctx: &egui::Context, wc: &mut WinCtx) {
    draw_icon_manager_window_in_dirs(ctx, wc, &all_theme_dirs());
}

fn draw_icon_manager_window_in_dirs(ctx: &egui::Context, wc: &mut WinCtx, dirs: &[PathBuf]) {
    let locale = crate::i18n::language();
    let mut state = ctx.data_mut(|d| d.get_temp_mut_or_default::<IconManagerState>(egui::Id::new("icon_manager_window")).clone());

    let mut packager_state = ctx.data_mut(|d| d.get_temp_mut_or_default::<PackagerDialogState>(egui::Id::new("icon_packager_dialog")).clone());

    if packager_state.is_open {
        draw_packager_modal(ctx, &mut packager_state);
    }

    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::new("icon_packager_dialog"), packager_state);
    });

    if !state.is_open {
        return;
    }

    let mut open = state.is_open;
    let mut changed = false;

    let discovered = discover_theme_packs_in_dirs(dirs);
    let mut all_packs = discovered.packs;

    // Ensure the built-in default pack is always represented if discovered or from embedded
    if !all_packs.iter().any(|p| p.manifest.id == "default") {
        if let Some(default_pack) = load_default_pack() {
            all_packs.push(default_pack);
        }
    }

    // Default selection if current is invalid
    if !all_packs.iter().any(|p| p.manifest.id == state.selected_pack_id) {
        if let Some(first) = all_packs.first() {
            state.selected_pack_id = first.manifest.id.clone();
        }
    }

    egui::Window::new(format!("{} {}", ph::PALETTE, crate::i18n::tr("icon-mgr-title")))
        .open(&mut open)
        .default_size(egui::vec2(1020.0, 700.0))
        .min_size(egui::vec2(760.0, 500.0))
        .resizable(true)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-desc")).weak());
            ui.add_space(8.0);
            ui.separator();

            egui::Panel::left("icon_manager_sidebar").resizable(true).default_size(330.0).size_range(300.0..=420.0).show(ui, |ui| {
                manager_sidebar_shell(ui, |ui| {
                    draw_icon_manager_actions(ui);
                })
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-active-cascade")).strong());
                    ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-themes-priority-hint")).small().weak());
                    ui.add_space(6.0);

                    let mut to_swap = None;
                    let mut to_remove = None;

                    for (idx, id) in wc.set.active_icon_packs.iter().enumerate() {
                        let pack_opt = all_packs.iter().find(|p| &p.manifest.id == id);
                        let is_selected = state.selected_pack_id == *id;

                        let (clicked, _) = manager_theme_card(ui, id, is_selected, |ui| {
                            let text_width = (ui.available_width() - 150.0).max(96.0);
                            ui.horizontal(|ui| {
                                if let Some(pack) = pack_opt {
                                    draw_pack_icon(ui, pack, 36.0);
                                } else {
                                    ui.add_sized([36.0, 36.0], egui::Label::new(ph::PACKAGE));
                                }
                                ui.vertical(|ui| {
                                    let name = pack_opt.map(|p| p.manifest.name_for_locale(&locale)).unwrap_or(id.as_str());
                                    let title = format!("{:02}  {name}", idx + 1);
                                    ui.add_sized([text_width, 26.0], egui::Label::new(title).truncate()).on_hover_text(name);
                                    if let Some(p) = pack_opt {
                                        ui.horizontal(|ui| {
                                            draw_bundle_format_badge(ui, p.format(), p.is_tampered);
                                            if p.is_directory() && wc.set.watched_icon_packs.contains(id) {
                                                ui.label(egui::RichText::new(ph::EYE).small().color(ui.visuals().warn_fg_color)).on_hover_text(crate::i18n::tr("icon-mgr-watch-this-pack"));
                                            }
                                        });
                                    }
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.add(egui::Button::new(ph::MINUS).small()).on_hover_text(crate::i18n::tr("icon-mgr-deactivate-btn")).clicked() {
                                        to_remove = Some(idx);
                                    }
                                    if ui
                                        .add_enabled(idx + 1 < wc.set.active_icon_packs.len(), egui::Button::new(ph::ARROW_DOWN).small())
                                        .on_hover_text(crate::i18n::tr("icon-mgr-move-down"))
                                        .clicked()
                                    {
                                        to_swap = Some((idx, idx + 1));
                                    }
                                    let leftmost_action = ui.add_enabled(idx > 0, egui::Button::new(ph::ARROW_UP).small()).on_hover_text(crate::i18n::tr("icon-mgr-move-up"));
                                    if leftmost_action.clicked() {
                                        to_swap = Some((idx, idx - 1));
                                    }
                                    Some(leftmost_action.rect)
                                })
                                .inner
                            })
                            .inner
                        });
                        if clicked {
                            state.selected_pack_id = id.clone();
                        }
                        ui.add_space(4.0);
                    }

                    // Base fallback
                    let (clicked, _) = manager_theme_card(ui, "default", state.selected_pack_id == "default", |ui| {
                        let text_width = (ui.available_width() - 44.0).max(110.0);
                        let base_pack = all_packs.iter().find(|pack| pack.manifest.id == "default");
                        ui.horizontal(|ui| {
                            if let Some(pack) = base_pack {
                                draw_pack_icon(ui, pack, 36.0);
                            }
                            ui.vertical(|ui| {
                                let name = base_pack.map(|pack| pack.manifest.name_for_locale(&locale).to_string()).unwrap_or_else(|| crate::i18n::tr("settings-icon-themes-base"));
                                let description = base_pack.map(|pack| pack.manifest.description_for_locale(&locale).to_string()).unwrap_or_else(|| crate::i18n::tr("settings-icon-themes-base-desc"));
                                ui.add_sized([text_width, 26.0], egui::Label::new(name).truncate()).on_hover_text(description);
                                draw_bundle_format_badge(ui, BundleFormat::Embedded, false);
                            });
                        });
                        None
                    });
                    if clicked {
                        state.selected_pack_id = "default".into();
                    }

                    if let Some((a, b)) = to_swap {
                        wc.set.active_icon_packs.swap(a, b);
                        changed = true;
                    }

                    ui.add_space(10.0);
                    ui.separator();
                    ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-available-themes")).strong());
                    ui.add_space(6.0);

                    // Available (inactive) themes
                    let mut to_activate = None;
                    for p in &all_packs {
                        if p.manifest.id == "default" || wc.set.active_icon_packs.contains(&p.manifest.id) {
                            continue;
                        }
                        let is_selected = state.selected_pack_id == p.manifest.id;
                        let (clicked, _) = manager_theme_card(ui, &p.manifest.id, is_selected, |ui| {
                            let text_width = (ui.available_width() - 168.0).max(90.0);
                            ui.horizontal(|ui| {
                                draw_pack_icon(ui, p, 36.0);
                                ui.vertical(|ui| {
                                    let name = p.manifest.name_for_locale(&locale);
                                    ui.add_sized([text_width, 26.0], egui::Label::new(name).truncate()).on_hover_text(name);
                                    ui.horizontal(|ui| {
                                        draw_bundle_format_badge(ui, p.format(), p.is_tampered);
                                        if p.is_directory() && wc.set.watched_icon_packs.contains(&p.manifest.id) {
                                            ui.label(egui::RichText::new(ph::EYE).small().color(ui.visuals().warn_fg_color)).on_hover_text(crate::i18n::tr("icon-mgr-watch-this-pack"));
                                        }
                                    });
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let label = format!("{} {}", ph::PLUS, crate::i18n::tr("icon-mgr-activate-btn"));
                                    let action = ui
                                        .add_sized(
                                            [112.0, 36.0],
                                            egui::Button::new(egui::RichText::new(label).strong().color(ui.visuals().selection.stroke.color)).fill(ui.visuals().selection.bg_fill).truncate(),
                                        )
                                        .on_hover_text(crate::i18n::tr("icon-mgr-activate-btn"));
                                    if action.clicked() {
                                        to_activate = Some(p.manifest.id.clone());
                                    }
                                    Some(action.rect)
                                })
                                .inner
                            })
                            .inner
                        });
                        if clicked {
                            state.selected_pack_id = p.manifest.id.clone();
                        }
                        ui.add_space(4.0);
                    }
                    if all_packs.iter().all(|p| p.manifest.id == "default" || wc.set.active_icon_packs.contains(&p.manifest.id)) {
                        ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-themes-none-available")).weak());
                    }

                    if !discovered.errors.is_empty() {
                        ui.add_space(10.0);
                        ui.separator();
                        ui.label(egui::RichText::new(format!("{} {}", ph::WARNING, crate::i18n::tr("icon-mgr-rejected-title"))).strong().color(ui.visuals().error_fg_color));
                        ui.add_space(4.0);
                        for err in &discovered.errors {
                            let file_name = err.path.file_name().and_then(|f| f.to_str()).unwrap_or("unknown");
                            egui::Frame::NONE.fill(ui.visuals().error_fg_color.linear_multiply(0.12)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(file_name).strong());
                                    ui.label(egui::RichText::new(&err.reason).small().weak());
                                });
                            });
                            ui.add_space(3.0);
                        }
                    }

                    if let Some(idx) = to_remove {
                        let removed = wc.set.active_icon_packs.remove(idx);
                        if !wc.set.inactive_icon_packs.contains(&removed) {
                            wc.set.inactive_icon_packs.push(removed);
                        }
                        changed = true;
                    }

                    if let Some(act) = to_activate {
                        wc.set.active_icon_packs.push(act.clone());
                        wc.set.inactive_icon_packs.retain(|x| x != &act);
                        state.selected_pack_id = act;
                        changed = true;
                    }
                });
            });

            egui::CentralPanel::default().show(ui, |ui| {
                let pack_opt = all_packs.iter().find(|p| p.manifest.id == state.selected_pack_id);
                let Some(pack) = pack_opt else {
                    ui.label(crate::i18n::tr("icon-mgr-no-pack-selected"));
                    return;
                };
                let pack_preview = manager_archive_preview(ctx, pack).or_else(|| manager_directory_preview(ctx, pack));
                let folder_source = pack.is_directory();

                egui::Frame::group(ui.style()).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if let Some(preview) = pack_preview.as_ref() {
                            draw_pack_icon_bytes(ui, pack, 48.0, preview.pack_icon.clone(), preview.image_generation);
                        } else {
                            draw_pack_icon(ui, pack, 48.0);
                        }
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(pack.manifest.name_for_locale(&locale)).heading().strong()).truncate());
                            let description = pack.manifest.description_for_locale(&locale);
                            if !description.is_empty() {
                                ui.add(egui::Label::new(egui::RichText::new(description).small().weak()).wrap());
                            }
                            ui.horizontal(|ui| {
                                draw_bundle_format_badge(ui, pack.format(), pack.is_tampered);
                                if let PackSource::Directory(source) = &pack.source {
                                    if wc.set.active_icon_packs.contains(&pack.manifest.id) && ui.button(format!("{} {}", ph::PACKAGE, crate::i18n::tr("settings-icon-package-btn"))).clicked() {
                                        open_packager_for_directory(ctx, pack, source);
                                    }
                                }
                                ui.label(egui::RichText::new(format!("v{}", pack.manifest.version)).small().weak());
                            });
                        });
                    });

                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(crate::i18n::tr1("icon-mgr-meta-id", "value", &pack.manifest.id)).small().monospace().weak());
                        ui.separator();
                        ui.label(egui::RichText::new(crate::i18n::tr1("icon-mgr-meta-license", "value", &pack.manifest.license)).small().weak());
                        if !pack.manifest.author.is_empty() {
                            ui.separator();
                            ui.label(egui::RichText::new(crate::i18n::tr1("icon-mgr-meta-author", "value", &pack.manifest.author)).small().weak());
                        }
                        ui.separator();
                        let mode_key = match &pack.manifest.color_mode {
                            ColorMode::Monochrome => "icon-mgr-color-monochrome",
                            ColorMode::Universal => "icon-mgr-color-universal",
                        };
                        ui.label(egui::RichText::new(crate::i18n::tr(mode_key)).small().weak());
                    });

                    let (cov, total) = pack_preview.as_ref().map_or_else(|| pack.coverage(), |preview| (preview.coverage, ALL_ICONS.len()));
                    let pct = (cov * 100).checked_div(total).unwrap_or(0);
                    let cov_msg = crate::i18n::trn("icon-mgr-total-icons", &[("count", &cov.to_string()), ("total", &total.to_string()), ("percent", &pct.to_string())]);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(cov_msg).small().strong());
                    });
                    ui.add(egui::ProgressBar::new(pct as f32 / 100.0).desired_width(ui.available_width()));

                    // Verified bundle / Archive / Tampered status and hygiene checks
                    if pack.format() == BundleFormat::VerifiedArchive {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{} {}", ph::SHIELD_CHECK, crate::i18n::tr("icon-mgr-verified-bundle-desc"))).color(ui.visuals().selection.bg_fill).small().strong());
                        });
                    } else if pack.is_tampered {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{} {}", ph::WARNING, crate::i18n::tr("icon-mgr-tampered-desc"))).color(ui.visuals().error_fg_color).small().strong());
                        });
                    } else if pack.format() == BundleFormat::Archive {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{} {}", ph::PACKAGE, crate::i18n::tr("icon-mgr-archive-desc"))).color(ui.visuals().hyperlink_color).small());
                        });
                    }

                    // For unverified packs (Archive, Directory), scan for SVG hygiene / validation issues
                    // Verified bundles intentionally skip runtime scans for maximum responsiveness
                    if folder_source || (pack.format() != BundleFormat::VerifiedArchive && pack.format() != BundleFormat::Embedded) {
                        let invalid_count = pack_preview.as_ref().map_or_else(|| ALL_ICONS.iter().filter(|id| pack.inspect_svg_for_id(**id).is_err()).count(), |preview| preview.invalid_icons);

                        ui.add_space(2.0);
                        if invalid_count > 0 {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{} {}", ph::WARNING, crate::i18n::trn("icon-mgr-hygiene-warning", &[("count", &invalid_count.to_string())])))
                                        .color(ui.visuals().warn_fg_color)
                                        .small()
                                        .strong(),
                                );
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(format!("{} {}", ph::CHECK_CIRCLE, crate::i18n::tr("icon-mgr-hygiene-clean"))).color(ui.visuals().selection.bg_fill).small());
                            });
                        }
                        if folder_source
                            && pack_preview.as_ref().is_some_and(|preview| preview.has_cleanable_icons)
                            && ui.button(format!("{} {}", ph::BROOM, crate::i18n::tr("icon-mgr-clean-all"))).clicked()
                        {
                            match clean_directory_icons(pack) {
                                Ok(report) => {
                                    let fixed = report.cleaned.len();
                                    let failed = report.failed.len();
                                    let text = if fixed == 0 && failed == 0 {
                                        crate::i18n::tr("icon-mgr-clean-none")
                                    } else {
                                        crate::i18n::trn("icon-mgr-clean-summary", &[("count", &fixed.to_string()), ("failed", &failed.to_string())])
                                    };
                                    let details = report.failed.iter().map(|failure| format!("{}: {}", failure.path.display(), failure.reason)).collect();
                                    state.clean_notice = Some(CleanNotice { pack_id: pack.manifest.id.clone(), text, is_error: failed > 0, details });
                                    if fixed > 0 {
                                        changed = true;
                                        ctx.request_repaint();
                                    }
                                }
                                Err(reason) => {
                                    state.clean_notice = Some(CleanNotice {
                                        pack_id: pack.manifest.id.clone(),
                                        text: crate::i18n::tr1("icon-mgr-clean-failed", "reason", &reason),
                                        is_error: true,
                                        details: Vec::new(),
                                    });
                                }
                            }
                        }
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    if pack.is_directory() {
                        ui.horizontal(|ui| {
                            let mut is_watched = wc.set.watched_icon_packs.contains(&pack.manifest.id);
                            if ui.checkbox(&mut is_watched, crate::i18n::tr("icon-mgr-watch-this-pack")).changed() {
                                if is_watched {
                                    if !wc.set.watched_icon_packs.contains(&pack.manifest.id) {
                                        wc.set.watched_icon_packs.push(pack.manifest.id.clone());
                                    }
                                } else {
                                    wc.set.watched_icon_packs.retain(|id| id != &pack.manifest.id);
                                }
                                qymcad_ui_state::icons::sync_global_watched_packs(&wc.set.watched_icon_packs);
                                changed = true;
                            }

                            if ui.button(format!("{} {}", ph::ARROW_CLOCKWISE, crate::i18n::tr("settings-icon-reload-now"))).clicked() {
                                clear_global_icon_cache();
                                apply_icon_themes(wc.set);
                                ctx.request_repaint();
                                *wc.status = crate::i18n::tr("icon-mgr-reloaded");
                            }
                        });
                    } else {
                        ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-watch-folder-only")).small().weak());
                    }
                });

                ui.add_space(10.0);

                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut state.active_tab, IconManagerTab::Readme, crate::i18n::tr("icon-mgr-tab-readme"));
                    ui.selectable_value(&mut state.active_tab, IconManagerTab::Gallery, crate::i18n::tr("icon-mgr-tab-gallery"));
                });
                ui.separator();
                if let Some(notice) = state.clean_notice.as_ref().filter(|notice| notice.pack_id == pack.manifest.id) {
                    let color = if notice.is_error { ui.visuals().warn_fg_color } else { ui.visuals().text_color() };
                    ui.label(egui::RichText::new(&notice.text).small().color(color));
                    if !notice.details.is_empty() {
                        egui::CollapsingHeader::new(crate::i18n::tr("icon-mgr-clean-details")).show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                                for detail in &notice.details {
                                    ui.add(egui::Label::new(egui::RichText::new(detail).small().monospace()).wrap());
                                }
                            });
                        });
                    }
                }

                match state.active_tab {
                    IconManagerTab::Readme => {
                        egui::ScrollArea::vertical().id_salt("mgr_readme_scroll").auto_shrink([false, false]).show(ui, |ui| {
                            let preview_image = if let Some(preview) = pack_preview.as_ref() {
                                preview.preview_image.clone()
                            } else {
                                pack.get_preview_image().map(|(bytes, extension)| ManagerPreviewImage { bytes: bytes.into(), extension })
                            };
                            if let Some(image) = preview_image {
                                let uri = format!("bytes://preview/{}/{}.{}", pack.manifest.id, image.extension, image.extension);
                                let img = egui::Image::from_bytes(uri, image.bytes).max_width(ui.available_width());
                                ui.add(img);
                                ui.add_space(8.0);
                                ui.separator();
                            }

                            let readme_md = pack_preview.as_ref().and_then(|preview| preview.readmes.get(&locale)).cloned().unwrap_or_else(|| pack.get_readme_for_locale(&locale));
                            crate::gui::help_window::markdown(&wc.scheme.pal, ui, &readme_md);
                        });
                    }
                    IconManagerTab::Gallery => {
                        // Filter & Search bar
                        ui.horizontal(|ui| {
                            ui.label(ph::MAGNIFYING_GLASS);
                            ui.add(egui::TextEdit::singleline(&mut state.search_query).hint_text(crate::i18n::tr("icon-mgr-search-icons")).desired_width((ui.available_width() - 8.0).max(120.0)));
                        });

                        ui.horizontal_wrapped(|ui| {
                            let cats = [
                                ("all", "icon-mgr-filter-all"),
                                ("sketch", "icon-mgr-filter-sketch"),
                                ("constraint", "icon-mgr-filter-constraint"),
                                ("part", "icon-mgr-filter-part"),
                                ("assembly", "icon-mgr-filter-assembly"),
                                ("datum", "icon-mgr-filter-datum"),
                            ];
                            for (val, key) in cats {
                                ui.selectable_value(&mut state.category_filter, val.to_string(), crate::i18n::tr(key));
                            }
                        });

                        ui.add_space(4.0);

                        let q = state.search_query.trim().to_lowercase();
                        let cat_filter = state.category_filter.as_str();

                        let now = ui.input(|i| i.time);
                        if let Some((_, copied_at)) = state.copied_path {
                            if now - copied_at >= 2.0 {
                                state.copied_path = None;
                            } else {
                                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                            }
                        }
                        let active_copied_path = state.copied_path.as_ref().map(|(p, _)| p.clone());

                        egui::ScrollArea::vertical().id_salt("mgr_gallery_scroll").auto_shrink([false, false]).show(ui, |ui| {
                            let mut shown = 0;
                            for &id in ALL_ICONS {
                                let relative_path = id.relative_path();
                                if cat_filter != "all" && !relative_path.starts_with(cat_filter) {
                                    continue;
                                }
                                if !q.is_empty() && !relative_path.to_lowercase().contains(&q) {
                                    continue;
                                }
                                shown += 1;
                                let row = if let Some(icon) = pack_preview.as_ref().and_then(|preview| preview.icons.get(&id)) {
                                    draw_gallery_icon_row(ui, pack, id, icon, folder_source, pack_preview.as_ref().map_or(0, |preview| preview.image_generation), active_copied_path.as_deref())
                                } else {
                                    let icon = pack.inspect_svg_for_id(id).map(|data| data.map(egui::load::Bytes::from));
                                    draw_gallery_icon_row(ui, pack, id, &icon, folder_source, 0, active_copied_path.as_deref())
                                };
                                if row.clean_clicked {
                                    ui.scroll_to_rect(row.rect, Some(egui::Align::Center));
                                    match clean_directory_icon(pack, id) {
                                        Ok(CleanIconResult::Cleaned) => {
                                            state.clean_notice =
                                                Some(CleanNotice { pack_id: pack.manifest.id.clone(), text: crate::i18n::tr("icon-mgr-cleaned-one"), is_error: false, details: Vec::new() });
                                            changed = true;
                                            ctx.request_repaint();
                                        }
                                        Ok(CleanIconResult::Unchanged | CleanIconResult::Missing) => {
                                            state.clean_notice =
                                                Some(CleanNotice { pack_id: pack.manifest.id.clone(), text: crate::i18n::tr("icon-mgr-clean-none"), is_error: false, details: Vec::new() });
                                        }
                                        Err(reason) => {
                                            state.clean_notice = Some(CleanNotice {
                                                pack_id: pack.manifest.id.clone(),
                                                text: crate::i18n::tr1("icon-mgr-clean-failed", "reason", &reason),
                                                is_error: true,
                                                details: Vec::new(),
                                            });
                                        }
                                    }
                                }
                                if row.path_copied {
                                    state.copied_path = Some((format!("icons/{}.svg", id.relative_path()), now));
                                    ctx.request_repaint();
                                }
                                ui.add_space(4.0);
                            }
                            if shown == 0 {
                                ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-no-icons-found")).weak());
                            }
                        });
                    }
                }
            });
        });

    if changed {
        clear_global_icon_cache();
        apply_icon_themes(wc.set);
        ctx.request_repaint();
    }

    state.is_open = open;
    ctx.data_mut(|d| {
        d.insert_temp(egui::Id::new("icon_manager_window"), state);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_directories_resolution() {
        let bundled = bundled_themes_dir();
        assert!(bundled.exists());

        let all = all_theme_dirs();
        assert!(!all.is_empty());
        assert!(all.contains(&bundled));
    }

    #[test]
    fn test_apply_icon_themes_cascade_logic() {
        let set = Settings { active_icon_packs: vec!["nonexistent-pack".into()], ..Default::default() };
        apply_icon_themes(&set);

        // Fallback works even when active packs are missing: falls back to built-in default SVG pack
        let icon = qymcad_ui_state::icons::resolve_global_icon(qymcad_ui_state::icons::IconId::SketchLine);
        assert_eq!(icon.pack_id, "default");
        assert_eq!(icon.color_mode, qymcad_ui_state::icons::ColorMode::Monochrome);
    }

    #[test]
    fn test_packager_dialog_state_defaults() {
        let state = PackagerDialogState::default();
        assert!(!state.is_open);
        assert_eq!(state.license, "LGPL-2.1-or-later");
        assert!(!state.is_monochrome);
    }

    #[test]
    fn default_theme_card_uses_the_bundle_translation() {
        use crate::gui::App;

        let previous_language = crate::i18n::language();
        crate::i18n::set_language("ru");
        let mut app = App::default();
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let dirs = [bundled_themes_dir()];
        let mut draw = || {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            ctx.run_ui(input, |ui| draw_icon_manager_window_in_dirs(ui.ctx(), &mut app.win_ctx(&mut Vec::new()), &dirs))
        };
        let _ = draw();
        let output = draw();
        fn has_sidebar_text(shape: &egui::epaint::Shape, expected: &str) -> bool {
            match shape {
                egui::epaint::Shape::Text(text) => text.pos.x < 360.0 && text.galley.text() == expected,
                egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|shape| has_sidebar_text(shape, expected)),
                _ => false,
            }
        }
        let default = load_default_pack().expect("embedded default theme");
        let translated = default.manifest.name_for_locale("ru");
        assert!(output.shapes.iter().any(|shape| has_sidebar_text(&shape.shape, translated)), "base card must show its bundle's Russian name");
        crate::i18n::set_language(&previous_language);
    }

    #[test]
    fn test_bundled_freecad_classic_pack_discovered() {
        let bundled = bundled_themes_dir();
        let packs = qymcad_ui_state::icons::discover_packs_in(&bundled);
        assert!(packs.iter().any(|p| p.manifest.id == "freecad-classic"), "bundled freecad-classic pack must be found");
        let classic = packs.iter().find(|p| p.manifest.id == "freecad-classic").unwrap();
        assert!(classic.coverage().0 >= 5);
        assert_eq!(classic.format(), BundleFormat::Directory);
        assert!(classic.is_directory());
        let readme = classic.get_readme();
        assert!(readme.contains("FreeCAD Classic Icon Theme"), "FreeCAD pack should have markdown description");
    }

    #[test]
    fn selected_bundled_freecad_redraw_stays_responsive() {
        use crate::gui::App;
        let mut app = App::default();
        app.set.active_icon_packs.clear();
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let dirs = [bundled_themes_dir()];
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut draw = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            ctx.run_ui(input, |ui| draw_icon_manager_window_in_dirs(ui.ctx(), &mut app.win_ctx(&mut Vec::new()), &dirs))
        };
        fn find_sidebar_label(shape: &egui::epaint::Shape, title: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == title && text.pos.x < 360.0 => Some(egui::Rect::from_min_size(text.pos, text.galley.size()).center()),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find_sidebar_label(shape, title)),
                _ => None,
            }
        }
        let _ = draw(vec![]);
        let output = draw(vec![]);
        let base_title = load_default_pack().expect("embedded default theme").manifest.name_for_locale(&crate::i18n::language()).to_string();
        let base = output.shapes.iter().find_map(|shape| find_sidebar_label(&shape.shape, &base_title)).expect("base theme card");
        let base_click = |pressed| egui::Event::PointerButton { pos: base, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let _ = draw(vec![egui::Event::PointerMoved(base)]);
        let _ = draw(vec![base_click(true)]);
        let output = draw(vec![base_click(false)]);
        let freecad = IconPack::from_directory(bundled_themes_dir().join("freecad")).expect("bundled FreeCAD theme");
        let name = freecad.manifest.name_for_locale(&crate::i18n::language());
        let at = output.shapes.iter().find_map(|shape| find_sidebar_label(&shape.shape, name)).expect("bundled FreeCAD card");
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let _ = draw(vec![egui::Event::PointerMoved(at)]);
        let _ = draw(vec![click(true)]);
        let _ = draw(vec![click(false)]);
        let selected = ctx.data(|data| data.get_temp::<IconManagerState>(egui::Id::new("icon_manager_window")).expect("manager state"));
        assert_eq!(selected.selected_pack_id, "freecad-classic");
        let start = std::time::Instant::now();
        for _ in 0..8 {
            let _ = draw(vec![]);
        }
        let elapsed = start.elapsed();
        eprintln!("selected bundled FreeCAD: eight redraws took {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(180), "eight redraws of the selected bundled theme took {elapsed:?}");
        fn find_gallery_tab(shape: &egui::epaint::Shape, title: &str) -> Option<egui::Pos2> {
            match shape {
                egui::epaint::Shape::Text(label) if label.galley.text() == title => Some(egui::Rect::from_min_size(label.pos, label.galley.size()).center()),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find_gallery_tab(shape, title)),
                _ => None,
            }
        }
        let output = draw(vec![]);
        let gallery_title = crate::i18n::tr("icon-mgr-tab-gallery");
        let gallery = output.shapes.iter().find_map(|shape| find_gallery_tab(&shape.shape, &gallery_title)).expect("gallery tab");
        let gallery_click = |pressed| egui::Event::PointerButton { pos: gallery, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let _ = draw(vec![egui::Event::PointerMoved(gallery)]);
        let _ = draw(vec![gallery_click(true)]);
        let _ = draw(vec![gallery_click(false)]);
        let start = std::time::Instant::now();
        for _ in 0..8 {
            let _ = draw(vec![]);
        }
        let gallery_elapsed = start.elapsed();
        eprintln!("selected bundled FreeCAD gallery: eight redraws took {gallery_elapsed:?}");
        assert!(gallery_elapsed < std::time::Duration::from_millis(180), "eight gallery redraws of the selected bundled theme took {gallery_elapsed:?}");
        assert!(app.set.active_icon_packs.is_empty(), "selection must not activate the theme");
    }

    #[test]
    fn directory_preview_tracks_svg_edits_without_reloading_the_theme() {
        let root = std::env::temp_dir().join(format!("qymcad_directory_preview_{}", std::process::id()));
        let icon_dir = root.join("icons/sketch");
        std::fs::create_dir_all(&icon_dir).expect("create icon directory");
        let pack = IconPack {
            manifest: IconManifest {
                package_type: PackageType::IconTheme,
                id: "preview-edits".into(),
                name: "Preview Edits".into(),
                version: "1.0".into(),
                author: String::new(),
                license: "MIT".into(),
                description: String::new(),
                color_mode: ColorMode::Universal,
                translations: Default::default(),
                verified: false,
            },
            source: PackSource::Directory(root.clone()),
            is_tampered: false,
        };
        let ctx = egui::Context::default();
        let empty = manager_directory_preview(&ctx, &pack).expect("initial preview");
        assert_eq!(empty.coverage, 0);
        assert!(empty.readmes.contains_key("en"), "preview must cache text for the interface");
        std::fs::write(root.join("README.en.md"), "# Custom English").expect("add localized README");
        let localized = manager_directory_preview(&ctx, &pack).expect("preview after adding localized README");
        assert_eq!(localized.readmes.get("en").map(String::as_str), Some("# Custom English"));
        assert!(!std::sync::Arc::ptr_eq(&empty, &localized), "new localized text must refresh the preview cache");
        let file = icon_dir.join("line.svg");
        std::fs::write(&file, br#"<svg viewBox="0 0 24 24"><path d="M0 0 L24 24"/></svg>"#).expect("add SVG");
        let added = manager_directory_preview(&ctx, &pack).expect("preview after adding SVG");
        assert_eq!(added.coverage, 1);
        assert!(added.image_generation > empty.image_generation, "added SVG must refresh the image key");
        std::fs::write(&file, br#"<svg viewBox="0 0 24 24"><script>bad()</script><path d="M0 0 L24 24"/></svg>"#).expect("edit SVG");
        let invalid = manager_directory_preview(&ctx, &pack).expect("preview after editing SVG");
        assert_eq!(invalid.invalid_icons, 1);
        std::fs::remove_file(&file).expect("remove SVG");
        let removed = manager_directory_preview(&ctx, &pack).expect("preview after removing SVG");
        assert_eq!(removed.coverage, 0);
        assert_eq!(removed.invalid_icons, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn icon_manager_discovers_each_pack_once() {
        let bundled = bundled_themes_dir();
        let temp_root = std::env::temp_dir().join(format!("qymcad_duplicate_theme_{}", std::process::id()));
        let custom_theme = temp_root.join("custom_freecad");
        std::fs::create_dir_all(&custom_theme).expect("create another theme directory");
        std::fs::copy(bundled.join("freecad/manifest.ron"), custom_theme.join("manifest.ron")).expect("copy the duplicate manifest");
        let dirs = [temp_root.clone(), bundled];
        let packs = discover_theme_packs_in_dirs(&dirs).packs;

        let mut app = crate::gui::App::default();
        app.set.active_icon_packs.clear();
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut painted = Vec::new();
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let output = ctx.run_ui(input, |ui| draw_icon_manager_window_in_dirs(ui.ctx(), &mut app.win_ctx(&mut Vec::new()), &dirs));
            painted.clear();
            for shape in &output.shapes {
                crate::gui::screen_keys::tests::collect_text(&shape.shape, &mut painted);
            }
        }
        assert!(painted.iter().all(|text| !text.contains("widget ID")), "egui paints duplicate-widget warnings: {painted:?}");
        let freecad_copies = packs.iter().filter(|pack| pack.manifest.id == "freecad-classic").count();
        assert_eq!(freecad_copies, 1, "two search paths show the same theme twice and reuse its widget ID");
        let chosen = packs.iter().find(|pack| pack.manifest.id == "freecad-classic").expect("the theme is available");
        assert!(matches!(&chosen.source, PackSource::Directory(path) if path == &custom_theme), "the first search directory must take precedence");
        let _ = std::fs::remove_dir_all(temp_root);
    }

    #[test]
    fn test_icon_manager_state_defaults() {
        let state = IconManagerState::default();
        assert!(!state.is_open);
        assert_eq!(state.active_tab, IconManagerTab::Readme);
        assert_eq!(state.category_filter, "all");
    }

    #[test]
    fn icon_manager_sidebar_scroll_stops_above_actions() {
        use std::cell::Cell;
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 320.0));
        let actions = Cell::new(egui::Rect::NOTHING);
        let scroll = Cell::new(egui::Rect::NOTHING);
        let content_height = Cell::new(0.0);
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let _ = ctx.run_ui(input, |ui| {
            egui::Panel::left("icon_test_sidebar").exact_size(300.0).show(ui, |ui| {
                let area = manager_sidebar_shell(ui, |ui| {
                    actions.set(ui.max_rect());
                    let _ = ui.button("Open folder");
                    let _ = ui.button("Reload themes");
                })
                .show(ui, |ui| {
                    for n in 0..30 {
                        ui.add_sized([ui.available_width(), 35.0], egui::Label::new(format!("Theme {n}")));
                    }
                });
                scroll.set(area.inner_rect);
                content_height.set(area.content_size.y);
            });
        });
        assert!(scroll.get().bottom() <= actions.get().top(), "the actions cover part of the list");
        assert!(content_height.get() > scroll.get().height(), "the list must remain scrollable when it is long");
    }

    #[test]
    fn clicking_a_theme_card_selects_it_without_activating_it() {
        use std::cell::Cell;
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 160.0));
        let card = Cell::new(egui::Rect::NOTHING);
        let icon = Cell::new(egui::Rect::NOTHING);
        let name = Cell::new(egui::Rect::NOTHING);
        let badge = Cell::new(egui::Rect::NOTHING);
        let button = Cell::new(egui::Rect::NOTHING);
        let selected = Cell::new(false);
        let activated = Cell::new(false);
        let draw = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let _ = ctx.run_ui(input, |ui| {
                let (clicked, rect) = manager_theme_card(ui, "test-pack", false, |ui| {
                    ui.horizontal(|ui| {
                        icon.set(
                            ui.add(
                                egui::Image::from_bytes("bytes://test-pack-icon.svg", b"<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'></svg>")
                                    .fit_to_exact_size(egui::vec2(20.0, 20.0)),
                            )
                            .rect,
                        );
                        name.set(ui.label("Theme").rect);
                        badge.set(ui.scope(|ui| draw_bundle_format_badge(ui, BundleFormat::Directory, false)).response.rect);
                        let response = ui.button("Activate");
                        button.set(response.rect);
                        if response.clicked() {
                            activated.set(true);
                        }
                        Some(response.rect)
                    })
                    .inner
                });
                card.set(rect);
                if clicked {
                    selected.set(true);
                }
            });
        };
        draw(vec![]);
        let at = egui::pos2(card.get().left() + 3.0, card.get().center().y);
        let click = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        draw(vec![egui::Event::PointerMoved(at)]);
        draw(vec![click(at, true)]);
        draw(vec![click(at, false)]);
        assert!(selected.get(), "clicking the card background must select the theme");
        assert!(!activated.get(), "selection must not activate the theme");

        for target in [icon.get(), name.get(), badge.get()] {
            selected.set(false);
            let at = target.center();
            draw(vec![egui::Event::PointerMoved(at)]);
            draw(vec![click(at, true)]);
            draw(vec![click(at, false)]);
            assert!(selected.get(), "clicking visible card content must select the theme: {target:?}");
            assert!(!activated.get(), "clicking card content must not activate the theme");
        }

        selected.set(false);
        let at = button.get().center();
        draw(vec![egui::Event::PointerMoved(at)]);
        draw(vec![click(at, true)]);
        draw(vec![click(at, false)]);
        assert!(activated.get(), "the activation button must keep its own action");
        assert!(!selected.get(), "the card must not steal the activation click");
    }

    #[test]
    fn deactivation_does_not_paint_the_theme_in_both_lists() {
        use crate::gui::App;

        struct PaintedLabel {
            text: String,
            rect: egui::Rect,
        }
        fn labels_in(shape: &egui::epaint::Shape, labels: &mut Vec<PaintedLabel>) {
            match shape {
                egui::epaint::Shape::Text(text) => labels.push(PaintedLabel { text: text.galley.text().to_string(), rect: egui::Rect::from_min_size(text.pos, text.galley.size()) }),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels_in(shape, labels)),
                _ => {}
            }
        }

        let mut app = App::default();
        app.set.active_icon_packs = vec!["freecad-classic".into()];
        app.set.inactive_icon_packs.retain(|id| id != "freecad-classic");
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut draw = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            ctx.run_ui(input, |ui| draw_icon_manager_window(ui.ctx(), &mut app.win_ctx(&mut Vec::new())))
        };
        let _ = draw(vec![]);
        let output = draw(vec![]);
        let mut labels = Vec::new();
        for shape in &output.shapes {
            labels_in(&shape.shape, &mut labels);
        }
        let minus = labels.iter().find(|label| label.text == ph::MINUS).expect("the active theme has a deactivate button").rect.center();
        let click = |pressed| egui::Event::PointerButton { pos: minus, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        draw(vec![egui::Event::PointerMoved(minus)]);
        draw(vec![click(true)]);
        let output = draw(vec![click(false)]);
        assert!(app.set.active_icon_packs.is_empty(), "the button did not deactivate the theme");
        let mut labels = Vec::new();
        for shape in &output.shapes {
            labels_in(&shape.shape, &mut labels);
        }
        let freecad = IconPack::from_directory(bundled_themes_dir().join("freecad")).expect("bundled FreeCAD theme");
        let name = freecad.manifest.name_for_locale(&crate::i18n::language());
        let available_copies = labels.iter().filter(|label| label.rect.left() < 300.0 && label.rect.top() > 300.0 && label.text == name).count();
        assert_eq!(available_copies, 0, "deactivation paints the new card before the next frame");
    }

    #[test]
    fn selecting_unverified_archive_keeps_redraw_responsive() {
        use crate::gui::App;
        let Some(sample_dir) = qymcad_paths::data("icon_themes") else {
            eprintln!("PASSED OVER: icon theme data directory is unavailable");
            return;
        };
        let sample = sample_dir.join("shapr-alike.qicons");
        if !sample.is_file() {
            eprintln!("PASSED OVER: shapr-alike.qicons is unavailable");
            return;
        }
        let sample_pack = IconPack::from_archive(&sample).expect("the reported bundle loads");
        let mut app = App::default();
        app.set.active_icon_packs.retain(|id| id != &sample_pack.manifest.id);
        let active_before = app.set.active_icon_packs.clone();
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut draw = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            ctx.run_ui(input, |ui| draw_icon_manager_window(ui.ctx(), &mut app.win_ctx(&mut Vec::new())))
        };
        let _ = draw(vec![]);
        let output = draw(vec![]);
        fn find_label(shape: &egui::epaint::Shape, name: &str) -> Option<egui::Rect> {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == name => Some(egui::Rect::from_min_size(text.pos, text.galley.size())),
                egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find_label(shape, name)),
                _ => None,
            }
        }
        let label = output.shapes.iter().find_map(|shape| find_label(&shape.shape, &sample_pack.manifest.name)).expect("the reported bundle appears in the sidebar");
        let at = label.center();
        let click = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let _ = draw(vec![egui::Event::PointerMoved(at)]);
        let _ = draw(vec![click(true)]);
        let _ = draw(vec![click(false)]);
        let selected = ctx.data(|data| data.get_temp::<IconManagerState>(egui::Id::new("icon_manager_window")).expect("manager state"));
        assert_eq!(selected.selected_pack_id, sample_pack.manifest.id, "the bundle was not selected");
        let output = draw(vec![]);
        let package_label = format!("{} {}", ph::PACKAGE, crate::i18n::tr("settings-icon-package-btn"));
        let clean_label = format!("{} {}", ph::BROOM, crate::i18n::tr("icon-mgr-clean-all"));
        assert!(output.shapes.iter().all(|shape| find_label(&shape.shape, &package_label).is_none()), "archive must not offer packaging");
        assert!(output.shapes.iter().all(|shape| find_label(&shape.shape, &clean_label).is_none()), "archive must not offer SVG cleaning");

        let start = std::time::Instant::now();
        for _ in 0..3 {
            let _ = draw(vec![]);
        }
        let elapsed = start.elapsed();
        eprintln!("selected unverified archive: three redraws took {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(450), "three redraws after selecting the bundle took {elapsed:?}");

        let output = draw(vec![]);
        let gallery_label = crate::i18n::tr("icon-mgr-tab-gallery");
        let gallery_tab = output.shapes.iter().find_map(|shape| find_label(&shape.shape, &gallery_label)).expect("the gallery tab is visible").center();
        let tab_click = |pressed| egui::Event::PointerButton { pos: gallery_tab, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let _ = draw(vec![egui::Event::PointerMoved(gallery_tab)]);
        let _ = draw(vec![tab_click(true)]);
        let _ = draw(vec![tab_click(false)]);
        let selected = ctx.data(|data| data.get_temp::<IconManagerState>(egui::Id::new("icon_manager_window")).expect("manager state"));
        assert_eq!(selected.active_tab, IconManagerTab::Gallery, "the gallery tab did not open");
        let start = std::time::Instant::now();
        for _ in 0..3 {
            let _ = draw(vec![]);
        }
        let elapsed = start.elapsed();
        eprintln!("unverified archive gallery: three redraws took {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(450), "three gallery redraws took {elapsed:?}");
        assert_eq!(app.set.active_icon_packs, active_before, "selection must not activate the bundle");
    }

    #[test]
    fn cleaning_a_gallery_icon_updates_its_file_and_preview() {
        use crate::gui::App;
        let root = std::env::temp_dir().join(format!("qymcad_gallery_clean_{}", std::process::id()));
        let theme = root.join("repairable");
        let icons = theme.join("icons/sketch");
        std::fs::create_dir_all(&icons).expect("create theme icons");
        let manifest = IconManifest {
            package_type: PackageType::IconTheme,
            id: "repairable".into(),
            name: "Repairable Theme".into(),
            version: "1.0".into(),
            author: "Test".into(),
            license: "MIT".into(),
            description: String::new(),
            color_mode: ColorMode::Universal,
            translations: Default::default(),
            verified: false,
        };
        std::fs::write(theme.join("manifest.ron"), manifest.to_ron().unwrap()).unwrap();
        let path = icons.join("line.svg");
        std::fs::write(&path, br#"<svg viewBox="0 0 24 24"><script>bad()</script><path d="M0 0 L24 24"/></svg>"#).unwrap();
        let second = icons.join("circle.svg");
        std::fs::write(&second, br#"<svg viewBox="0 0 24 24"><metadata>editor</metadata><circle cx="12" cy="12" r="8"/></svg>"#).unwrap();
        let unrepairable = icons.join("rect.svg");
        let bad_viewbox = br#"<svg viewBox="0 0 32 16"><rect width="32" height="16"/></svg>"#;
        std::fs::write(&unrepairable, bad_viewbox).unwrap();
        let dirs = [root.clone()];
        let mut app = App::default();
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut draw = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            ctx.run_ui(input, |ui| draw_icon_manager_window_in_dirs(ui.ctx(), &mut app.win_ctx(&mut Vec::new()), &dirs))
        };
        fn find_text(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Rect> {
            fn in_shape(shape: &egui::epaint::Shape, needle: &str) -> Option<egui::Rect> {
                match shape {
                    egui::epaint::Shape::Text(text) if text.galley.text().contains(needle) => Some(egui::Rect::from_min_size(text.pos, text.galley.size())),
                    egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| in_shape(shape, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|shape| in_shape(&shape.shape, needle))
        }
        let click = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let _ = draw(vec![]);
        let output = draw(vec![]);
        let tab = find_text(&output.shapes, &crate::i18n::tr("icon-mgr-tab-gallery")).expect("gallery tab").center();
        let _ = draw(vec![egui::Event::PointerMoved(tab)]);
        let _ = draw(vec![click(tab, true)]);
        let output = draw(vec![click(tab, false)]);
        assert!(find_text(&output.shapes, &crate::i18n::tr("settings-icon-package-btn")).is_none(), "inactive folder must not offer packaging");
        assert!(find_text(&output.shapes, &crate::i18n::tr("settings-icon-refresh")).is_none(), "the sidebar must not show a refresh button");
        let archive_path = "icons/sketch/line.svg";
        let path_at = find_text(&output.shapes, archive_path).expect("gallery icon path").center();
        let _ = draw(vec![egui::Event::PointerMoved(path_at)]);
        let _ = draw(vec![click(path_at, true)]);
        let copy_output = draw(vec![click(path_at, false)]);
        assert!(copy_output.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == archive_path)), "clicking the icon path must copy it");
        let output = draw(vec![]);
        assert!(find_text(&output.shapes, &crate::i18n::tr("icon-mgr-path-copied")).is_some(), "copying a path needs visible confirmation");
        let button = find_text(&output.shapes, &crate::i18n::tr("icon-mgr-clean-icon")).expect("clean button beside invalid SVG").center();
        let before = qymcad_ui_state::icons::get_global_icon_revision();
        let _ = draw(vec![egui::Event::PointerMoved(button)]);
        let _ = draw(vec![click(button, true)]);
        let _ = draw(vec![click(button, false)]);
        qymcad_ui_state::icons::validate_svg(&std::fs::read(&path).unwrap()).expect("button cleaned the SVG on disk");
        assert!(qymcad_ui_state::icons::get_global_icon_revision() > before, "cleaning did not invalidate rendered icon textures");
        let output = draw(vec![]);
        assert!(find_text(&output.shapes, &crate::i18n::tr("icon-mgr-gallery-present")).is_some(), "the refreshed gallery does not show the cleaned icon");

        let clean_all = find_text(&output.shapes, &crate::i18n::tr("icon-mgr-clean-all")).expect("bulk clean button").center();
        let before_bulk = qymcad_ui_state::icons::get_global_icon_revision();
        let _ = draw(vec![egui::Event::PointerMoved(clean_all)]);
        let _ = draw(vec![click(clean_all, true)]);
        let _ = draw(vec![click(clean_all, false)]);
        qymcad_ui_state::icons::validate_svg(&std::fs::read(&second).unwrap()).expect("bulk button cleaned another icon");
        assert_eq!(std::fs::read(&unrepairable).unwrap(), bad_viewbox, "bulk clean must leave geometry needing manual repair alone");
        assert!(qymcad_ui_state::icons::get_global_icon_revision() > before_bulk, "bulk cleaning did not refresh icon textures");
        let output = draw(vec![]);
        let activate = find_text(&output.shapes, &crate::i18n::tr("icon-mgr-activate-btn")).expect("folder activation button").center();
        let _ = draw(vec![egui::Event::PointerMoved(activate)]);
        let _ = draw(vec![click(activate, true)]);
        let _ = draw(vec![click(activate, false)]);
        let output = draw(vec![]);
        let package_rect = find_text(&output.shapes, &crate::i18n::tr("settings-icon-package-btn")).expect("active folder packaging button");
        let version_rect = find_text(&output.shapes, "v1.0").expect("selected folder version");
        assert!((package_rect.center().y - version_rect.center().y).abs() < 16.0, "packaging button must share the folder badge row");
        let meta_label = crate::i18n::tr1("icon-mgr-meta-id", "value", &manifest.id);
        let metadata_rect = find_text(&output.shapes, &meta_label).expect("folder metadata must be visible below the header");
        assert!(metadata_rect.top() - package_rect.bottom() < 100.0, "packaging button must not stretch the folder header");
        assert!(find_text(&output.shapes, archive_path).is_some(), "folder gallery must remain visible after activation");
        let package = package_rect.center();
        let _ = draw(vec![egui::Event::PointerMoved(package)]);
        let _ = draw(vec![click(package, true)]);
        let _ = draw(vec![click(package, false)]);
        let packager = ctx.data(|data| data.get_temp::<PackagerDialogState>(egui::Id::new("icon_packager_dialog")).expect("packager state"));
        assert!(packager.is_open, "packager did not open");
        assert_eq!(packager.source_dir, theme.display().to_string(), "packager must use the selected folder");
        assert_eq!(packager.id, manifest.id);
        assert_eq!(packager.output_file, theme.with_extension("qicons").display().to_string());
        assert!(app.set.active_icon_packs.contains(&manifest.id), "the folder was not activated");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn gallery_rows_stack_within_the_panel_width() {
        use std::cell::RefCell;
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 300.0));
        let mut icons = std::collections::HashMap::new();
        icons.insert("icons/sketch/line.svg".to_string(), br#"<svg viewBox="0 0 32 16"/>"#.to_vec());
        let pack = IconPack {
            manifest: IconManifest {
                package_type: PackageType::IconTheme,
                id: "gallery-test".into(),
                name: "Gallery Test".into(),
                version: "1.0".into(),
                author: "Test".into(),
                license: "MIT".into(),
                description: String::new(),
                color_mode: ColorMode::Universal,
                translations: Default::default(),
                verified: false,
            },
            source: qymcad_ui_state::icons::PackSource::Memory(icons),
            is_tampered: false,
        };
        let rows = RefCell::new(Vec::new());
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let output = ctx.run_ui(input, |ui| {
            ui.set_width(380.0);
            let line = qymcad_ui_state::icons::IconId::SketchLine;
            let line_icon = pack.inspect_svg_for_id(line).map(|data| data.map(egui::load::Bytes::from));
            rows.borrow_mut().push(draw_gallery_icon_row(ui, &pack, line, &line_icon, false, 0, None).rect);
            let longest_path = ALL_ICONS.iter().copied().max_by_key(|id| id.relative_path().len()).unwrap();
            let longest_icon = pack.inspect_svg_for_id(longest_path).map(|data| data.map(egui::load::Bytes::from));
            rows.borrow_mut().push(draw_gallery_icon_row(ui, &pack, longest_path, &longest_icon, false, 0, None).rect);
        });
        let rows = rows.borrow();
        assert!(rows[0].width() <= 380.0, "a gallery row expands the panel");
        assert!(rows[1].top() >= rows[0].bottom(), "gallery icons must form a vertical list");
        assert!(rows[1].width() <= 380.0, "long paths expand the panel");

        fn painted_in(shape: &egui::epaint::Shape, labels: &mut Vec<String>, previews: &mut usize) {
            match shape {
                egui::epaint::Shape::Text(text) => labels.push(text.galley.text().to_string()),
                egui::epaint::Shape::Rect(rect) if (rect.rect.width() - 56.0).abs() < 0.1 && (rect.rect.height() - 56.0).abs() < 0.1 && rect.stroke.width > 0.0 => {
                    *previews += 1;
                }
                egui::epaint::Shape::Vec(shapes) => shapes.iter().for_each(|shape| painted_in(shape, labels, previews)),
                _ => {}
            }
        }
        let mut labels = Vec::new();
        let mut preview_frames = 0;
        for shape in &output.shapes {
            painted_in(&shape.shape, &mut labels, &mut preview_frames);
        }
        let longest_path = ALL_ICONS.iter().max_by_key(|id| id.relative_path().len()).unwrap().relative_path();
        assert!(labels.iter().any(|text| text.contains("icons/sketch/line.svg")), "the archive path is not visible");
        assert!(labels.iter().any(|text| text.contains(&format!("icons/{longest_path}.svg"))), "a missing icon needs its expected archive path");
        assert!(labels.iter().any(|text| text.contains("non-square viewBox")), "the specific SVG error is not visible");
        assert!(!labels.iter().any(|text| text.contains(&crate::i18n::tr("icon-mgr-clean-icon"))), "archive icons must not show cleaning controls");
        assert!(labels.iter().any(|text| text.contains(&crate::i18n::tr("icon-mgr-gallery-missing"))), "missing icons need a neutral status");
        assert_eq!(preview_frames, 2, "every icon needs a visible 56 px preview frame");
    }

    #[test]
    fn discover_theme_packs_caches_results_until_directory_changes() {
        let temp_root = std::env::temp_dir().join(format!("qymcad_cache_test_{}", std::process::id()));
        let theme_dir = temp_root.join("test_theme");
        std::fs::create_dir_all(&theme_dir).expect("create test theme dir");
        let manifest_content = r#"(
            id: "test-cache",
            name: "Test Cache",
            version: "1.0.0",
            author: "Tester",
            license: "MIT",
            color_mode: Universal,
        )"#;
        std::fs::write(theme_dir.join("manifest.ron"), manifest_content).expect("write manifest");

        let dirs = vec![temp_root.clone()];
        clear_discovery_cache_for_test();

        let initial_stats = discovery_cache_stats();
        let first = discover_theme_packs_in_dirs(&dirs);
        assert_eq!(first.packs.len(), 1);
        let after_first = discovery_cache_stats();
        assert_eq!(after_first.misses, initial_stats.misses + 1);

        let second = discover_theme_packs_in_dirs(&dirs);
        assert_eq!(second.packs.len(), 1);
        let after_second = discovery_cache_stats();
        assert_eq!(after_second.hits, initial_stats.hits + 1);
        assert_eq!(after_second.misses, after_first.misses);

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn icon_manager_window_displays_rejected_archives_with_reason() {
        let temp_root = std::env::temp_dir().join(format!("qymcad_reject_ui_test_{}", std::process::id()));
        std::fs::create_dir_all(&temp_root).expect("create temp dir");
        let broken_archive = temp_root.join("broken_pack.qicons");
        std::fs::write(&broken_archive, b"corrupted data that cannot be parsed as zip").expect("write broken archive");

        let dirs = vec![temp_root.clone()];
        clear_discovery_cache_for_test();

        let mut app = crate::gui::App::default();
        let ctx = egui::Context::default();
        crate::gui::install_fonts(&ctx);
        open_icon_manager(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        let mut painted = Vec::new();
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let output = ctx.run_ui(input, |ui| draw_icon_manager_window_in_dirs(ui.ctx(), &mut app.win_ctx(&mut Vec::new()), &dirs));
            painted.clear();
            for shape in &output.shapes {
                crate::gui::screen_keys::tests::collect_text(&shape.shape, &mut painted);
            }
        }

        assert!(painted.iter().any(|text| text.contains("broken_pack.qicons")), "icon manager window must display rejected archive filename: {painted:?}");
        assert!(painted.iter().any(|text| text.contains(&crate::i18n::tr("icon-mgr-rejected-title"))), "icon manager window must display rejected section title: {painted:?}");

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
