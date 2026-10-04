//! ICON THEME CONFIGURATION AND MANAGEMENT.
//!
//! Provides discovery, priority-cascade configuration, and bundle packaging
//! for CAD icon packs according to RFC-icon-theme-system.
use egui::Color32;
use egui_phosphor::regular as ph;
use qymcad_ui_state::icons::{
    clear_global_icon_cache, discover_packs_in, inspect_pack_directory, load_default_pack,
    package_bundle, reload_active_icon_themes, BundleFormat, ColorMode, IconManifest, PackageType,
    ValidationReport, ALL_ICONS,
};
use qymcad_ui_state::{Settings, WinCtx};
use std::path::PathBuf;

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

/// Apply the active icon packs from settings into the global icon manager.
pub(crate) fn apply_icon_themes(set: &Settings) {
    let dirs = all_theme_dirs();
    reload_active_icon_themes(&set.active_icon_packs, &dirs);
}

/// Render a compact, colored visual badge indicating the format and provenance of an icon bundle.
pub(crate) fn draw_bundle_format_badge(ui: &mut egui::Ui, format: BundleFormat) {
    let visuals = ui.visuals();
    let (icon, label_key, bg, fg) = match format {
        BundleFormat::Directory => (
            ph::FOLDER_OPEN,
            "bundle-format-folder",
            visuals.warn_fg_color.linear_multiply(0.18),
            visuals.warn_fg_color,
        ),
        BundleFormat::Archive => (
            ph::PACKAGE,
            "bundle-format-archive",
            visuals.hyperlink_color.linear_multiply(0.18),
            visuals.hyperlink_color,
        ),
        BundleFormat::VerifiedArchive => (
            ph::CHECK,
            "bundle-format-verified",
            visuals.selection.bg_fill.linear_multiply(0.22),
            visuals.selection.bg_fill,
        ),
        BundleFormat::Embedded => (
            ph::GEAR,
            "bundle-format-embedded",
            visuals.faint_bg_color,
            visuals.weak_text_color(),
        ),
    };

    egui::Frame::NONE
        .fill(bg)
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(5, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.label(egui::RichText::new(icon).color(fg).small());
                ui.label(egui::RichText::new(crate::i18n::tr(label_key)).color(fg).small().strong());
            });
        });
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

    let dirs = all_theme_dirs();
    let mut all_packs = Vec::new();
    for d in &dirs {
        all_packs.extend(discover_packs_in(d));
    }
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
                ui.label(egui::RichText::new(&pack.manifest.name).strong());
                draw_bundle_format_badge(ui, pack.format());
                ui.label(egui::RichText::new(ph::ARROW_RIGHT).weak());
            } else {
                ui.label(egui::RichText::new(id).weak());
                ui.label(egui::RichText::new(ph::ARROW_RIGHT).weak());
            }
        }

        // Base fallback is always the built-in SVG bundle
        ui.label(egui::RichText::new(crate::i18n::tr("settings-icon-themes-base")).strong());
        draw_bundle_format_badge(ui, BundleFormat::Embedded);
    });

    ui.add_space(6.0);
    if ui.button(format!("{} {}", ph::PALETTE, crate::i18n::tr("settings-open-icon-manager"))).clicked() {
        open_icon_manager(ctx);
    }
}

/// In-app developer modal dialog for packaging an icon bundle into `.qicons`.
fn draw_packager_modal(ctx: &egui::Context, state: &mut PackagerDialogState) {
    let mut open = state.is_open;
    egui::Window::new(crate::i18n::tr("icon-packager-title"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(450.0)
        .show(ctx, |ui| {
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
                        match inspect_pack_directory(&source_path) {
                            Ok(rep) => {
                                if rep.has_issues() {
                                    state.message = Some(format!(
                                        "Validation found {} issue(s): {} rejected, {} extraneous",
                                        rep.rejected.len() + rep.extraneous.len(),
                                        rep.rejected.len(),
                                        rep.extraneous.len()
                                    ));
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

                    if state.id.trim().is_empty() || state.name.trim().is_empty() {
                        state.message = Some("Theme ID and Name cannot be empty".into());
                        state.is_error = true;
                    } else if !source_path.exists() {
                        state.message = Some(format!("Source folder does not exist: {}", source_path.display()));
                        state.is_error = true;
                    } else if state.output_file.trim().is_empty() {
                        state.message = Some("Output .qicons path cannot be empty".into());
                        state.is_error = true;
                    } else {
                        let manifest = IconManifest {
                            package_type: PackageType::IconTheme,
                            id: state.id.trim().to_string(),
                            name: state.name.trim().to_string(),
                            version: state.version.trim().to_string(),
                            author: state.author.trim().to_string(),
                            license: state.license.trim().to_string(),
                            description: state.description.trim().to_string(),
                            color_mode: if state.is_monochrome {
                                ColorMode::Monochrome
                            } else {
                                ColorMode::Universal
                            },
                            verified: true,
                        };

                        match package_bundle(&source_path, &manifest, &output_path) {
                            Ok(rep) => {
                                if rep.included.is_empty() {
                                    state.message = Some(crate::i18n::tr("icon-packager-no-icons"));
                                    state.is_error = true;
                                } else {
                                    let cov_str = rep.included.len().to_string();
                                    let path_str = output_path.display().to_string();
                                    state.message = Some(crate::i18n::trn(
                                        "icon-packager-success",
                                        &[("count", &cov_str), ("path", &path_str)],
                                    ));
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
                    ui.label(
                        egui::RichText::new(crate::i18n::trn(
                            "icon-packager-coverage",
                            &[("included", &cov_str), ("total", &total_str), ("percent", &pct_str)],
                        ))
                        .strong(),
                    );
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
                        egui::CollapsingHeader::new(egui::RichText::new(inc_title).color(Color32::GREEN))
                            .default_open(rep.rejected.is_empty() && rep.extraneous.is_empty())
                            .show(ui, |ui| {
                                for id in &rep.included {
                                    ui.label(egui::RichText::new(format!("  {} {}", ph::CHECK, id.relative_path())).small());
                                }
                            });
                    }

                    // Missing icons list (collapsible)
                    if !rep.missing.is_empty() {
                        ui.add_space(4.0);
                        let miss_title = crate::i18n::tr1("icon-packager-missing-title", "count", &rep.missing.len().to_string());
                        egui::CollapsingHeader::new(egui::RichText::new(miss_title).weak())
                            .default_open(false)
                            .show(ui, |ui| {
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
}

impl Default for IconManagerState {
    fn default() -> Self {
        Self {
            is_open: false,
            selected_pack_id: "freecad-classic".into(),
            active_tab: IconManagerTab::Readme,
            search_query: String::new(),
            category_filter: "all".into(),
        }
    }
}

/// Request to open the Icon Theme Manager window.
pub(crate) fn open_icon_manager(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        let state = d.get_temp_mut_or_default::<IconManagerState>(egui::Id::new("icon_manager_window"));
        state.is_open = true;
    });
}

/// Draw the dedicated Icon Theme Manager window.
pub(crate) fn draw_icon_manager_window(ctx: &egui::Context, wc: &mut WinCtx) {
    let mut state = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<IconManagerState>(egui::Id::new("icon_manager_window"))
            .clone()
    });

    let mut packager_state = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<PackagerDialogState>(egui::Id::new("icon_packager_dialog"))
            .clone()
    });

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

    let dirs = all_theme_dirs();
    let mut all_packs = Vec::new();
    for d in &dirs {
        all_packs.extend(discover_packs_in(d));
    }

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
        .default_size(egui::vec2(860.0, 580.0))
        .min_size(egui::vec2(680.0, 420.0))
        .resizable(true)
        .show(ctx, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-desc")).small().weak());
            ui.add_space(4.0);
            ui.separator();

            ui.columns(2, |cols| {
                // LEFT COLUMN: Packs & Cascade order
                cols[0].vertical(|ui| {
                    ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-active-cascade")).strong());
                    ui.add_space(2.0);

                    let mut to_swap = None;
                    let mut to_remove = None;

                    egui::ScrollArea::vertical().id_salt("mgr_active_scroll").max_height(200.0).show(ui, |ui| {
                        for (idx, id) in wc.set.active_icon_packs.iter().enumerate() {
                            let pack_opt = all_packs.iter().find(|p| &p.manifest.id == id);
                            let is_selected = state.selected_pack_id == *id;

                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(format!("#{}:", idx + 1)).weak());

                                if idx > 0 && ui.button(ph::ARROW_UP).on_hover_text(crate::i18n::tr("icon-mgr-move-up")).clicked() {
                                    to_swap = Some((idx, idx - 1));
                                }
                                if idx + 1 < wc.set.active_icon_packs.len()
                                    && ui.button(ph::ARROW_DOWN).on_hover_text(crate::i18n::tr("icon-mgr-move-down")).clicked()
                                {
                                    to_swap = Some((idx, idx + 1));
                                }
                                if ui.button(ph::MINUS).on_hover_text(crate::i18n::tr("icon-mgr-deactivate-btn")).clicked() {
                                    to_remove = Some(idx);
                                }

                                let name = pack_opt.map(|p| p.manifest.name.as_str()).unwrap_or(id.as_str());
                                let label = if is_selected {
                                    egui::RichText::new(name).strong().underline()
                                } else {
                                    egui::RichText::new(name)
                                };
                                if ui.selectable_label(is_selected, label).clicked() {
                                    state.selected_pack_id = id.clone();
                                }
                                if let Some(p) = pack_opt {
                                    draw_bundle_format_badge(ui, p.format());
                                    if p.is_directory() && wc.set.watched_icon_packs.contains(id) {
                                        ui.label(egui::RichText::new(ph::EYE).small().color(ui.visuals().warn_fg_color))
                                            .on_hover_text(crate::i18n::tr("icon-mgr-watch-this-pack"));
                                    }
                                }
                            });
                        }

                        // Base fallback
                        ui.horizontal(|ui| {
                            let is_selected = state.selected_pack_id == "default";
                            ui.label(egui::RichText::new("Base:").weak());
                            if ui.selectable_label(is_selected, crate::i18n::tr("settings-icon-themes-base")).clicked() {
                                state.selected_pack_id = "default".into();
                            }
                            draw_bundle_format_badge(ui, BundleFormat::Embedded);
                        });
                    });

                    if let Some((a, b)) = to_swap {
                        wc.set.active_icon_packs.swap(a, b);
                        changed = true;
                    }
                    if let Some(idx) = to_remove {
                        let removed = wc.set.active_icon_packs.remove(idx);
                        if !wc.set.inactive_icon_packs.contains(&removed) {
                            wc.set.inactive_icon_packs.push(removed);
                        }
                        changed = true;
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.label(egui::RichText::new(crate::i18n::tr("icon-mgr-available-themes")).strong());
                    ui.add_space(2.0);

                    // Available (inactive) themes
                    let mut to_activate = None;
                    egui::ScrollArea::vertical().id_salt("mgr_avail_scroll").max_height(160.0).show(ui, |ui| {
                        for p in &all_packs {
                            if p.manifest.id == "default" || wc.set.active_icon_packs.contains(&p.manifest.id) {
                                continue;
                            }
                            let is_selected = state.selected_pack_id == p.manifest.id;
                            ui.horizontal(|ui| {
                                if ui.button(ph::PLUS).on_hover_text(crate::i18n::tr("icon-mgr-activate-btn")).clicked() {
                                    to_activate = Some(p.manifest.id.clone());
                                }
                                let label = if is_selected {
                                    egui::RichText::new(&p.manifest.name).strong().underline()
                                } else {
                                    egui::RichText::new(&p.manifest.name)
                                };
                                if ui.selectable_label(is_selected, label).clicked() {
                                    state.selected_pack_id = p.manifest.id.clone();
                                }
                                draw_bundle_format_badge(ui, p.format());
                                if p.is_directory() && wc.set.watched_icon_packs.contains(&p.manifest.id) {
                                    ui.label(egui::RichText::new(ph::EYE).small().color(ui.visuals().warn_fg_color))
                                        .on_hover_text(crate::i18n::tr("icon-mgr-watch-this-pack"));
                                }
                            });
                        }
                    });

                    if let Some(act) = to_activate {
                        wc.set.active_icon_packs.push(act.clone());
                        wc.set.inactive_icon_packs.retain(|x| x != &act);
                        state.selected_pack_id = act;
                        changed = true;
                    }

                    ui.add_space(8.0);
                    ui.separator();

                    // Toolbar actions
                    ui.horizontal_wrapped(|ui| {
                        if let Some(user_dir) = user_themes_dir() {
                            if ui.button(format!("{} {}", ph::FOLDER_OPEN, crate::i18n::tr("settings-icon-open-folder"))).clicked() {
                                let _ = std::fs::create_dir_all(&user_dir);
                                let (bin, args) = crate::gui::reveal_command(ui.ctx().os(), &user_dir);
                                let _ = crate::system::start(bin, &args);
                            }
                        }
                        if ui.button(format!("{} {}", ph::ARROW_CLOCKWISE, crate::i18n::tr("settings-icon-refresh"))).clicked() {
                            clear_global_icon_cache();
                            apply_icon_themes(wc.set);
                            *wc.status = crate::i18n::tr("settings-icon-cache-cleared");
                        }
                        if ui.button(format!("{} {}", ph::PACKAGE, crate::i18n::tr("settings-icon-package-btn"))).clicked() {
                            ctx.data_mut(|d| {
                                let ps = d.get_temp_mut_or_default::<PackagerDialogState>(egui::Id::new("icon_packager_dialog"));
                                ps.is_open = true;
                            });
                        }
                    });
                });

                // RIGHT COLUMN: Selected Theme Details, Markdown README, and Icon Gallery Preview
                cols[1].vertical(|ui| {
                    let pack_opt = all_packs.iter().find(|p| p.manifest.id == state.selected_pack_id);
                    let Some(pack) = pack_opt else {
                        ui.label(crate::i18n::tr("icon-mgr-no-pack-selected"));
                        return;
                    };

                    // Pack Header Card
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(&pack.manifest.name).heading().strong());
                            ui.label(egui::RichText::new(format!("v{}", pack.manifest.version)).weak());
                            draw_bundle_format_badge(ui, pack.format());
                        });

                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(format!("ID: {}", pack.manifest.id)).small().monospace());
                            ui.label("-");
                            ui.label(egui::RichText::new(format!("License: {}", pack.manifest.license)).small());
                            if !pack.manifest.author.is_empty() {
                                ui.label("-");
                                ui.label(egui::RichText::new(format!("Author: {}", pack.manifest.author)).small().weak());
                            }
                            ui.label("-");
                            let mode_str = match &pack.manifest.color_mode {
                                ColorMode::Monochrome => "Monochrome",
                                ColorMode::Universal => "Universal Colors",
                                ColorMode::Specific(_) => "Specific Themes",
                            };
                            ui.label(egui::RichText::new(mode_str).small().color(ui.visuals().hyperlink_color));
                        });

                        let (cov, total) = pack.coverage();
                        let pct = if total == 0 { 0 } else { (cov * 100) / total };
                        let cov_msg = crate::i18n::trn(
                            "icon-mgr-total-icons",
                            &[("count", &cov.to_string()), ("total", &total.to_string()), ("percent", &pct.to_string())],
                        );
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(cov_msg).small().strong());
                        });

                        // Scan for SVG hygiene / validation issues
                        let mut invalid_icons = Vec::new();
                        for &id in ALL_ICONS {
                            if let Some(svg_bytes) = pack.get_svg_for_id(id) {
                                if let Err(e) = qymcad_ui_state::icons::validate_svg(&svg_bytes) {
                                    invalid_icons.push((id, e));
                                }
                            }
                        }

                        ui.add_space(2.0);
                        if !invalid_icons.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} {}",
                                        ph::WARNING,
                                        crate::i18n::trn("icon-mgr-hygiene-warning", &[("count", &invalid_icons.len().to_string())])
                                    ))
                                    .color(ui.visuals().warn_fg_color)
                                    .small()
                                    .strong(),
                                );
                            });
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("{} {}", ph::CHECK_CIRCLE, crate::i18n::tr("icon-mgr-hygiene-clean")))
                                        .color(ui.visuals().selection.bg_fill)
                                        .small(),
                                );
                            });
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

                    ui.add_space(4.0);

                    // Tab selector
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut state.active_tab, IconManagerTab::Readme, crate::i18n::tr("icon-mgr-tab-readme"));
                        ui.selectable_value(&mut state.active_tab, IconManagerTab::Gallery, crate::i18n::tr("icon-mgr-tab-gallery"));
                    });
                    ui.separator();

                    match state.active_tab {
                        IconManagerTab::Readme => {
                            egui::ScrollArea::vertical().id_salt("mgr_readme_scroll").max_height(380.0).show(ui, |ui| {
                                if let Some((img_bytes, ext)) = pack.get_preview_image() {
                                    let uri = format!("bytes://preview/{}/{}.{}", pack.manifest.id, ext, ext);
                                    let img = egui::Image::from_bytes(uri, img_bytes).fit_to_original_size(1.0);
                                    ui.add(img);
                                    ui.add_space(8.0);
                                    ui.separator();
                                }

                                let readme_md = pack.get_readme();
                                crate::gui::help_window::markdown(&wc.scheme.pal, ui, &readme_md);
                            });
                        }
                        IconManagerTab::Gallery => {
                            // Filter & Search bar
                            ui.horizontal(|ui| {
                                ui.label(ph::MAGNIFYING_GLASS);
                                ui.add(
                                    egui::TextEdit::singleline(&mut state.search_query)
                                        .hint_text(crate::i18n::tr("icon-mgr-search-icons"))
                                        .desired_width(140.0),
                                );

                                ui.separator();

                                let cats = [
                                    ("all", "icon-mgr-filter-all"),
                                    ("sketch", "icon-mgr-filter-sketch"),
                                    ("constraint", "icon-mgr-filter-constraint"),
                                    ("part", "icon-mgr-filter-part"),
                                    ("assembly", "icon-mgr-filter-assembly"),
                                    ("datum", "icon-mgr-filter-datum"),
                                    ("common", "icon-mgr-filter-common"),
                                ];
                                for (val, key) in cats {
                                    ui.selectable_value(&mut state.category_filter, val.to_string(), crate::i18n::tr(key));
                                }
                            });

                            ui.add_space(4.0);

                            // Icons Grid
                            let q = state.search_query.trim().to_lowercase();
                            let cat_filter = state.category_filter.as_str();

                            egui::ScrollArea::vertical().id_salt("mgr_gallery_scroll").max_height(340.0).show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for &id in ALL_ICONS {
                                        let rel_path = id.relative_path();

                                        if cat_filter != "all" && !rel_path.starts_with(cat_filter) {
                                            continue;
                                        }

                                        if !q.is_empty() && !rel_path.to_lowercase().contains(&q) {
                                            continue;
                                        }

                                        let icon_svg = pack.get_svg_for_id(id);
                                        let is_present = icon_svg.is_some();
                                        let short_name = rel_path.split('/').last().unwrap_or(rel_path);

                                        egui::Frame::NONE
                                            .fill(if is_present { ui.visuals().faint_bg_color } else { Color32::TRANSPARENT })
                                            .corner_radius(4.0)
                                            .inner_margin(4.0)
                                            .show(ui, |ui| {
                                                ui.vertical_centered(|ui| {
                                                    let val_err = icon_svg.as_ref().and_then(|data| qymcad_ui_state::icons::validate_svg(data).err());
                                                    if let Some(svg_data) = icon_svg {
                                                        let uri = format!("bytes://mgr/{}/r{}/{}.svg", pack.manifest.id, qymcad_ui_state::icons::get_global_icon_revision(), rel_path);
                                                        let mut img = egui::Image::from_bytes(uri, svg_data)
                                                            .fit_to_exact_size(egui::vec2(28.0, 28.0));
                                                        if pack.manifest.color_mode == ColorMode::Monochrome {
                                                            img = img.tint(ui.visuals().text_color());
                                                        }
                                                        let tip = if let Some(ref e) = val_err {
                                                            format!("{rel_path}\n(in pack)\n⚠️ Validation / Hygiene Issue:\n{e}")
                                                        } else {
                                                            format!("{rel_path}\n(in pack)")
                                                        };
                                                        ui.add(img).on_hover_text(tip);
                                                    } else {
                                                        let resolved = qymcad_ui_state::icons::resolve_global_icon(id);
                                                        let uri = format!("bytes://mgr/fallback/r{}/{}.svg", resolved.revision, rel_path);
                                                        let mut img = egui::Image::from_bytes(uri, resolved.data)
                                                            .fit_to_exact_size(egui::vec2(28.0, 28.0));
                                                        img = img.tint(ui.visuals().weak_text_color());
                                                        ui.add(img)
                                                            .on_hover_text(format!("{rel_path}\n(missing: falls back to default)"));
                                                    }

                                                    let label_text = if val_err.is_some() {
                                                        egui::RichText::new(format!("⚠️ {short_name}")).small().color(ui.visuals().warn_fg_color)
                                                    } else {
                                                        egui::RichText::new(short_name).small().weak()
                                                    };
                                                    ui.label(label_text);
                                                });
                                            });
                                    }
                                });
                            });
                        }
                    }
                });
            });
        });

    if changed {
        clear_global_icon_cache();
        apply_icon_themes(wc.set);
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
        let mut set = Settings::default();
        set.active_icon_packs = vec!["nonexistent-pack".into()];
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
    fn test_bundled_freecad_classic_pack_discovered() {
        let bundled = bundled_themes_dir();
        let packs = discover_packs_in(&bundled);
        assert!(packs.iter().any(|p| p.manifest.id == "freecad-classic"), "bundled freecad-classic pack must be found");
        let classic = packs.iter().find(|p| p.manifest.id == "freecad-classic").unwrap();
        assert!(classic.coverage().0 >= 5);
        assert_eq!(classic.format(), BundleFormat::Embedded);
        let readme = classic.get_readme();
        assert!(readme.contains("FreeCAD Classic Icon Theme"), "FreeCAD pack should have markdown description");
    }

    #[test]
    fn test_icon_manager_state_defaults() {
        let state = IconManagerState::default();
        assert!(!state.is_open);
        assert_eq!(state.active_tab, IconManagerTab::Readme);
        assert_eq!(state.category_filter, "all");
    }
}
