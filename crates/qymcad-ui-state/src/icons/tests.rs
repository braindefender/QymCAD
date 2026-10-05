//! Unit tests for the icon theme system.

use std::collections::HashMap;

use super::bundle::*;
use super::id::*;
use super::manager::*;
use super::manifest::*;
use super::pack::*;

#[test]
fn every_icon_id_has_relative_path() {
    for id in ALL_ICONS {
        let path = id.relative_path();
        assert!(!path.is_empty(), "icon {:?} has empty relative path", id);
        assert!(!path.ends_with(".svg"), "relative path should not contain extension: {}", path);

        let roundtrip = IconId::from_id_str(path);
        assert_eq!(roundtrip, Some(*id), "from_id_str failed for {}", path);
    }
}

#[test]
fn generated_correct_break_path() {
    assert_eq!(IconId::SketchBreak.relative_path(), "sketch/break");
}

#[test]
fn manifest_ron_roundtrip() {
    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "freecad-classic".to_string(),
        name: "FreeCAD Classic".to_string(),
        version: "1.0.0".to_string(),
        author: "FreeCAD Contributors".to_string(),
        license: "LGPL-2.1-or-later".to_string(),
        description: "Classic colored tool icons".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };

    let ron_str = manifest.to_ron().expect("serialization succeeds");
    let parsed = IconManifest::parse_ron(&ron_str).expect("parsing succeeds");

    assert_eq!(parsed.id, "freecad-classic");
    assert_eq!(parsed.name, "FreeCAD Classic");
    assert_eq!(parsed.color_mode, ColorMode::Universal);
}

#[test]
fn cascade_fallback_chain() {
    let sample_svg = br#"<svg viewBox="0 0 64 64"><line x1="0" y1="0" x2="64" y2="64"/></svg>"#;

    // Pack A has sketch/line
    let mut map_a = HashMap::new();
    map_a.insert("icons/sketch/line.svg".to_string(), sample_svg.to_vec());
    let pack_a = IconPack {
        manifest: IconManifest {
            package_type: PackageType::IconTheme,
            id: "pack-a".to_string(),
            name: "Pack A".to_string(),
            version: "1.0".to_string(),
            author: "Test".to_string(),
            license: "MIT".to_string(),
            description: "".to_string(),
            color_mode: ColorMode::Monochrome,
            verified: false,
        },
        source: PackSource::Memory(map_a),
        is_tampered: false,
    };

    // Pack B has sketch/rect
    let mut map_b = HashMap::new();
    map_b.insert("icons/sketch/rect.svg".to_string(), sample_svg.to_vec());
    let pack_b = IconPack {
        manifest: IconManifest {
            package_type: PackageType::IconTheme,
            id: "pack-b".to_string(),
            name: "Pack B".to_string(),
            version: "1.0".to_string(),
            author: "Test".to_string(),
            license: "MIT".to_string(),
            description: "".to_string(),
            color_mode: ColorMode::Universal,
            verified: false,
        },
        source: PackSource::Memory(map_b),
        is_tampered: false,
    };

    let mut mgr = IconManager::new();
    // Priority: Pack A > Pack B
    mgr.set_active_stack(vec![pack_a, pack_b]);

    // 1. Line is resolved from Pack A
    let res_line = mgr.resolve(IconId::SketchLine);
    assert_eq!(res_line.pack_id, "pack-a");
    assert_eq!(res_line.color_mode, ColorMode::Monochrome);

    // 2. Rect is resolved from Pack B (falling through Pack A)
    let res_rect = mgr.resolve(IconId::SketchRect);
    assert_eq!(res_rect.pack_id, "pack-b");
    assert_eq!(res_rect.color_mode, ColorMode::Universal);

    // 3. Extrude is in neither Pack A nor Pack B, falls back to the embedded default SVG pack
    let res_extrude = mgr.resolve(IconId::PartExtrude);
    assert_eq!(res_extrude.pack_id, "default");
    assert_eq!(res_extrude.color_mode, ColorMode::Monochrome);
}

#[test]
fn svg_validation_rules() {
    let valid = br#"<svg viewBox="0 0 64 64"><path d="M0,0 L64,64"/></svg>"#;
    assert!(validate_svg(valid).is_ok());

    let missing_viewbox = br#"<svg width="64" height="64"><path d="M0,0"/></svg>"#;
    assert!(validate_svg(missing_viewbox).is_err());

    let with_embedded_raster = br#"<svg viewBox="0 0 64 64"><image href="data:image/png;base64,123"/></svg>"#;
    assert!(validate_svg(with_embedded_raster).is_err());

    // Prohibited script and foreignObject
    let with_script = br#"<svg viewBox="0 0 64 64"><script>alert(1)</script></svg>"#;
    assert!(validate_svg(with_script).is_err());
    let with_foreign = br#"<svg viewBox="0 0 64 64"><foreignObject><div>test</div></foreignObject></svg>"#;
    assert!(validate_svg(with_foreign).is_err());

    // Inline event handlers
    let with_handler = br#"<svg viewBox="0 0 64 64" onload="run()"><circle cx="32" cy="32" r="10"/></svg>"#;
    assert!(validate_svg(with_handler).is_err());

    // Editor metadata and namespaces
    let with_sodipodi = br#"<svg viewBox="0 0 64 64"><sodipodi:namedview id="base"/></svg>"#;
    assert!(validate_svg(with_sodipodi).is_err());
    let with_inkscape = br#"<svg viewBox="0 0 64 64"><inkscape:grid id="grid1"/></svg>"#;
    assert!(validate_svg(with_inkscape).is_err());
    let with_metadata = br#"<svg viewBox="0 0 64 64"><metadata id="meta"><rdf:RDF/></metadata></svg>"#;
    assert!(validate_svg(with_metadata).is_err());
    let with_illustrator = br#"<svg viewBox="0 0 64 64"><i:pgf id="adobe_pgf"/></svg>"#;
    assert!(validate_svg(with_illustrator).is_err());
}

#[test]
fn package_bundle_and_load_from_archive() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_icon_test_{}", std::process::id()));
    let icons_dir = temp_dir.join("icons").join("sketch");
    std::fs::create_dir_all(&icons_dir).expect("creates test dirs");

    let svg_content = br#"<svg viewBox="0 0 64 64"><line x1="0" y1="0" x2="64" y2="64"/></svg>"#;
    std::fs::write(icons_dir.join("line.svg"), svg_content).expect("writes svg");
    let pack_icon = br#"<svg viewBox="0 0 64 64"><circle cx="32" cy="32" r="20"/></svg>"#;
    std::fs::write(temp_dir.join("icon.svg"), pack_icon).expect("writes pack icon");

    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "test-pack".to_string(),
        name: "Test Pack".to_string(),
        version: "1.0.0".to_string(),
        author: "Tester".to_string(),
        license: "MIT".to_string(),
        description: "Test".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(temp_dir.join("manifest.ron"), manifest.to_ron().expect("manifest serializes")).expect("writes manifest");

    let archive_path = temp_dir.join("test-pack.qicons");
    let report = package_bundle(&temp_dir, &manifest, &archive_path).expect("packaging succeeds");
    assert_eq!(report.included.len(), 1);
    assert_eq!(report.rejected.len(), 0);
    assert_eq!(report.extraneous.len(), 0);
    assert_eq!(report.missing.len(), ALL_ICONS.len() - 1);
    assert_eq!(IconPack::from_directory(&temp_dir).expect("folder loads").get_pack_icon_svg(), pack_icon);

    let pack = IconPack::from_archive(&archive_path).expect("loading archive succeeds");
    assert_eq!(pack.manifest.id, "test-pack");
    assert_eq!(pack.coverage().0, 1);
    assert_eq!(pack.get_pack_icon_svg(), pack_icon, "packaging must retain the icon beside manifest.ron");

    let embedded = IconPack::from_zip_bytes(&std::fs::read(&archive_path).expect("archive reads")).expect("embedded archive loads");
    assert_eq!(embedded.get_pack_icon_svg(), pack_icon);

    let data = pack.get_svg_for_id(IconId::SketchLine).expect("line icon exists in archive");
    assert_eq!(data, svg_content);

    let missing = pack.get_svg_for_id(IconId::SketchCircle);
    assert!(missing.is_none());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn missing_or_invalid_pack_icon_uses_default() {
    let dir = std::env::temp_dir().join(format!("qymcad_pack_icon_fallback_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("creates pack directory");
    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "icon-fallback".into(),
        name: "Icon fallback".into(),
        version: "1.0.0".into(),
        author: String::new(),
        license: "MIT".into(),
        description: String::new(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(dir.join("manifest.ron"), manifest.to_ron().expect("manifest serializes")).expect("writes manifest");
    let pack = IconPack::from_directory(&dir).expect("folder loads");
    let default_icon = load_default_pack().expect("default loads").get_pack_icon_svg();
    assert_eq!(pack.get_pack_icon_svg(), default_icon);

    std::fs::write(dir.join("icon.svg"), b"<svg><script/></svg>").expect("writes invalid icon");
    assert_eq!(pack.get_pack_icon_svg(), default_icon, "unsafe icons must not reach the UI");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn validation_viewbox_square_and_invalid() {
    // 1:1 square viewbox passes
    let valid_svg = br#"<svg viewBox="0 0 64 64"><rect width="64" height="64"/></svg>"#;
    assert!(validate_svg(valid_svg).is_ok());

    // Non-square viewbox (e.g. 100 x 50) fails
    let non_square_svg = br#"<svg viewBox="0 0 100 50"><rect width="100" height="50"/></svg>"#;
    let err = validate_svg(non_square_svg).unwrap_err();
    assert!(err.contains("aspect ratio must be 1:1"), "Expected non-square error, got: {}", err);

    // Missing viewBox fails
    let no_viewbox_svg = br#"<svg width="64" height="64"><circle r="10"/></svg>"#;
    let err = validate_svg(no_viewbox_svg).unwrap_err();
    assert!(err.contains("missing viewBox"), "Expected missing viewBox error, got: {}", err);

    // Raster image tag fails
    let raster_svg = br#"<svg viewBox="0 0 64 64"><image href="photo.png"/></svg>"#;
    let err = validate_svg(raster_svg).unwrap_err();
    assert!(err.contains("raster"), "Expected raster image error, got: {}", err);
}

#[test]
fn inspect_and_package_excludes_problematic_files() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_val_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let icons_dir = temp_dir.join("icons").join("sketch");
    std::fs::create_dir_all(&icons_dir).expect("creates test dirs");

    // Valid icon: sketch/line.svg
    let valid_svg = br#"<svg viewBox="0 0 64 64"><line x1="0" y1="0" x2="64" y2="64"/></svg>"#;
    std::fs::write(icons_dir.join("line.svg"), valid_svg).expect("writes valid svg");

    // Invalid icon (non-square): sketch/circle.svg
    let non_square_svg = br#"<svg viewBox="0 0 100 50"><circle cx="50" cy="25" r="20"/></svg>"#;
    std::fs::write(icons_dir.join("circle.svg"), non_square_svg).expect("writes non-square svg");

    // Extraneous file inside icons: sketch/notes.txt
    std::fs::write(icons_dir.join("notes.txt"), b"some notes").expect("writes notes");

    // Extraneous unknown icon: icons/sketch/unknown_extra_icon.svg
    std::fs::write(icons_dir.join("unknown_extra_icon.svg"), valid_svg).expect("writes unknown svg");

    // Extraneous file in pack root: temp_dir/notes.md
    std::fs::write(temp_dir.join("notes.md"), b"pack notes").expect("writes root notes");

    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "validation-test-pack".to_string(),
        name: "Validation Test Pack".to_string(),
        version: "1.0.0".to_string(),
        author: "Tester".to_string(),
        license: "MIT".to_string(),
        description: "Test".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };

    // Test inspect_pack_directory first
    let report = inspect_pack_directory(&temp_dir).expect("inspect succeeds");
    assert_eq!(report.included.len(), 1, "Only 1 valid icon should be included");
    assert_eq!(report.included[0], IconId::SketchLine);

    assert_eq!(report.rejected.len(), 1, "1 icon should be rejected");
    assert_eq!(report.rejected[0].0, "sketch/circle.svg");
    assert!(report.rejected[0].1.contains("aspect ratio must be 1:1"));

    assert_eq!(report.extraneous.len(), 3, "3 extraneous files should be flagged");

    // Package bundle: archive should only contain valid icon and manifest
    let archive_path = temp_dir.join("val-pack.qicons");
    let pack_report = package_bundle(&temp_dir, &manifest, &archive_path).expect("package succeeds");
    assert_eq!(pack_report.included.len(), 1);
    assert_eq!(pack_report.rejected.len(), 1);
    assert_eq!(pack_report.extraneous.len(), 3);

    // Verify loaded pack from archive
    let pack = IconPack::from_archive(&archive_path).expect("loads archive");
    assert_eq!(pack.coverage().0, 1);
    assert!(pack.get_svg_for_id(IconId::SketchLine).is_some());
    assert!(pack.get_svg_for_id(IconId::SketchCircle).is_none(), "Rejected icon must NOT be in archive");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn default_embedded_pack_is_valid_and_complete() {
    let pack = load_default_pack().expect("embedded default.qicons must load cleanly");
    assert_eq!(pack.manifest.id, "default");
    assert_eq!(pack.manifest.color_mode, ColorMode::Monochrome);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(DEFAULT_QICONS)).expect("embedded bundle reads");
    assert!(archive.by_name("icon.svg").is_ok(), "default icon must be stored beside manifest.ron");
    validate_svg(&pack.get_pack_icon_svg()).expect("default pack icon is a valid SVG");
    
    // Check coverage of all known IconIds
    let (cov, total) = pack.coverage();
    assert_eq!(cov, total, "embedded default.qicons must cover 100% of icons (got {}/{})", cov, total);

    // Verify SVG data is valid for every single icon
    for id in ALL_ICONS {
        let svg = pack.get_svg_for_id(*id).expect("must have SVG");
        validate_svg(&svg).expect("SVG must pass validation");
    }

    // Verify pack README is loaded
    let readme = pack.get_readme();
    assert!(readme.contains("Default Vector Icon Theme"), "embedded README should be available");
}

#[test]
fn global_icon_manager_cascade() {
    clear_global_icon_cache();
    let mgr = IconManager::new();
    set_global_icon_manager(mgr);

    // Initial resolution with default pack resolves to default embedded SVG
    let initial = resolve_global_icon(IconId::SketchLine);
    assert_eq!(initial.pack_id, "default");
    assert_eq!(initial.color_mode, ColorMode::Monochrome);

    // Now push custom pack on top
    let mut map = HashMap::new();
    let custom_svg = br#"<svg viewBox="0 0 64 64"><line x1="0" y1="0" x2="64" y2="64"/></svg>"#.to_vec();
    map.insert("icons/sketch/line.svg".to_string(), custom_svg.clone());
    let custom_pack = IconPack {
        manifest: IconManifest {
            package_type: PackageType::IconTheme,
            id: "pack-custom".to_string(),
            name: "Pack Custom".to_string(),
            version: "1.0.0".to_string(),
            author: "Test".to_string(),
            license: "MIT".to_string(),
            description: "Test".to_string(),
            color_mode: ColorMode::Universal,
            verified: false,
        },
        source: PackSource::Memory(map),
        is_tampered: false,
    };

    with_global_icon_manager_mut(|m| {
        m.push_top_pack(custom_pack);
    });

    let resolved = resolve_global_icon(IconId::SketchLine);
    assert_eq!(resolved.data, custom_svg);
    assert_eq!(resolved.color_mode, ColorMode::Universal);
    assert_eq!(resolved.pack_id, "pack-custom");
}

#[test]
fn icon_tool_renders_with_icon_id() {
    let ctx = egui::Context::default();
    let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
        let _ = crate::icon_tool(ui, IconId::SketchLine, "Line tool", true);
    });
}

#[test]
fn discover_packs_in_directory() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_discover_test_{}", std::process::id()));
    let theme_a = temp_dir.join("theme_a");
    std::fs::create_dir_all(theme_a.join("icons").join("sketch")).unwrap();
    let manifest_a = IconManifest {
        package_type: PackageType::IconTheme,
        id: "theme-a".to_string(),
        name: "Theme A".to_string(),
        version: "1.0.0".to_string(),
        author: "Tester".to_string(),
        license: "MIT".to_string(),
        description: "Test".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(theme_a.join("manifest.ron"), manifest_a.to_ron().unwrap()).unwrap();
    std::fs::write(theme_a.join("icons").join("sketch").join("line.svg"), br#"<svg viewBox="0 0 64 64"><line x1="0" y1="0" x2="64" y2="64"/></svg>"#).unwrap();

    let packs = discover_packs_in(&temp_dir);
    assert_eq!(packs.len(), 1);
    assert_eq!(packs[0].manifest.id, "theme-a");

    let mut mgr = IconManager::new();
    mgr.push_top_pack(packs.into_iter().next().unwrap());
    let resolved = mgr.resolve(IconId::SketchLine);
    assert_eq!(resolved.pack_id, "theme-a");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn freecad_theme_is_complete_and_valid() {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let freecad_dir = manifest_dir.join("../../assets/icon-themes/freecad");
    if !freecad_dir.is_dir() {
        return;
    }

    let pack = IconPack::from_directory(&freecad_dir).expect("FreeCAD theme must load from directory");
    assert_eq!(pack.manifest.id, "freecad-classic");
    assert_eq!(pack.manifest.color_mode, ColorMode::Universal);
    assert_eq!(pack.format(), BundleFormat::Embedded);
    assert!(!pack.is_directory());
    validate_svg(&pack.get_pack_icon_svg()).expect("FreeCAD pack icon is a valid SVG");
    assert_ne!(pack.get_pack_icon_svg(), load_default_pack().expect("default loads").get_pack_icon_svg());

    let (cov, total) = pack.coverage();
    assert_eq!(cov, total, "FreeCAD theme must cover 100% of icons (got {}/{})", cov, total);

    for id in ALL_ICONS {
        let svg = pack.get_svg_for_id(*id).expect("FreeCAD theme must have SVG for icon");
        validate_svg(&svg).expect("SVG must pass validation");
    }

    assert!(pack.get_readme().contains("FreeCAD Classic Icon Theme"), "FreeCAD README should be available");
}


#[test]
fn user_shapr_alike_pack_if_present_loads_and_has_icons() {
    let user_path = std::path::PathBuf::from("/home/pavver/.antigravity-profiles/tony.ai.new77/.local/share/qymcad/icon_themes/shapr-alike.qicons");
    if user_path.is_file() {
        let pack = IconPack::from_archive(&user_path).expect("shapr-alike.qicons must load cleanly");
        assert_eq!(pack.manifest.id, "shapr-alike");
        assert_eq!(pack.manifest.name, "Shapr-Alike");
        let (cov, _total) = pack.coverage();
        assert!(cov >= 105, "shapr-alike should cover almost all icons, got {cov}");
    }
}

#[test]
fn bundle_format_and_provenance_detection() {
    let default_pack = load_default_pack().expect("embedded default pack");
    assert_eq!(default_pack.format(), BundleFormat::Embedded);
    assert!(default_pack.is_verified());
    assert!(!default_pack.is_directory());

    let temp_dir = std::env::temp_dir().join(format!("qymcad_format_test_{}", std::process::id()));
    let folder_pack_dir = temp_dir.join("folder_theme");
    std::fs::create_dir_all(folder_pack_dir.join("icons").join("sketch")).unwrap();
    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "folder-theme".to_string(),
        name: "Folder Theme".to_string(),
        version: "1.0.0".to_string(),
        author: "Dev".to_string(),
        license: "MIT".to_string(),
        description: "Test".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(folder_pack_dir.join("manifest.ron"), manifest.to_ron().unwrap()).unwrap();
    std::fs::write(folder_pack_dir.join("icons").join("sketch").join("line.svg"), br#"<svg viewBox="0 0 24 24"><line x1="0" y1="0" x2="24" y2="24"/></svg>"#).unwrap();

    let folder_pack = IconPack::from_directory(&folder_pack_dir).expect("folder pack must load");
    assert_eq!(folder_pack.format(), BundleFormat::Directory);
    assert!(folder_pack.is_directory());
    assert!(!folder_pack.is_archive());

    // Package into .qicons via CAD packager
    let archive_path = temp_dir.join("packaged.qicons");
    package_bundle(&folder_pack_dir, &manifest, &archive_path).expect("packaging must succeed");

    let loaded_archive = IconPack::from_archive(&archive_path).expect("packaged archive must load");
    assert_eq!(loaded_archive.format(), BundleFormat::VerifiedArchive);
    assert!(loaded_archive.is_verified());
    assert!(loaded_archive.is_archive());
    assert!(!loaded_archive.is_directory());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn live_watch_folder_auto_reload_on_svg_change() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_watch_test_{}", std::process::id()));
    let pack_dir = temp_dir.join("watch_theme");
    std::fs::create_dir_all(pack_dir.join("icons").join("sketch")).unwrap();
    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "watch-theme".to_string(),
        name: "Watch Theme".to_string(),
        version: "1.0.0".to_string(),
        author: "Dev".to_string(),
        license: "MIT".to_string(),
        description: "Test".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(pack_dir.join("manifest.ron"), manifest.to_ron().unwrap()).unwrap();
    let initial_svg = br#"<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"/></svg>"#;
    let svg_path = pack_dir.join("icons").join("sketch").join("line.svg");
    std::fs::write(&svg_path, initial_svg).unwrap();

    let folder_pack = IconPack::from_directory(&pack_dir).expect("pack must load");
    let mut mgr = IconManager::new();
    mgr.push_top_pack(folder_pack);
    mgr.set_pack_watching("watch-theme", true);
    assert!(mgr.is_pack_watched("watch-theme"));

    // First resolve: gets initial SVG
    let res1 = mgr.resolve(IconId::SketchLine);
    assert_eq!(res1.data, initial_svg);
    let rev1 = res1.revision;

    // Polling without changes returns false
    assert!(!mgr.check_watched_directories());

    // Sleep briefly so file modification timestamp differs
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Now edit the SVG in the folder
    let updated_svg = br#"<svg viewBox="0 0 24 24"><rect width="24" height="24"/></svg>"#;
    std::fs::write(&svg_path, updated_svg).unwrap();

    // Reset poll timer throttle so test runs immediately
    std::thread::sleep(std::time::Duration::from_millis(260));

    // Polling detects the change
    let changed = mgr.check_watched_directories();
    assert!(changed, "check_watched_directories should detect edited file in watched directory");

    // Second resolve: gets updated SVG and updated revision
    let res2 = mgr.resolve(IconId::SketchLine);
    assert_eq!(res2.data, updated_svg, "resolved icon should have the updated SVG content");
    assert!(res2.revision > rev1, "revision counter should have incremented");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_successive_folder_live_reloads_do_not_stop_after_3_times() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_successive_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let pack_dir = temp_dir.join("continuous-theme");
    std::fs::create_dir_all(pack_dir.join("icons").join("sketch")).unwrap();

    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "continuous-theme".to_string(),
        name: "Continuous Theme".to_string(),
        version: "1.0.0".to_string(),
        author: "Tester".to_string(),
        license: "MIT".to_string(),
        description: "Testing continuous updates".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(pack_dir.join("manifest.ron"), manifest.to_ron().unwrap()).unwrap();
    let svg_line = pack_dir.join("icons").join("sketch").join("line.svg");
    let svg_circle = pack_dir.join("icons").join("sketch").join("circle.svg");
    std::fs::write(&svg_line, br#"<svg viewBox="0 0 24 24"><path d="M0 0 L10 10"/></svg>"#).unwrap();
    std::fs::write(&svg_circle, br#"<svg viewBox="0 0 24 24"><circle cx="1" cy="1" r="1"/></svg>"#).unwrap();

    let folder_pack = IconPack::from_directory(&pack_dir).expect("pack must load");
    let mut mgr = IconManager::new();
    mgr.push_top_pack(folder_pack);
    mgr.set_pack_watching("continuous-theme", true);

    // Initial check
    let _ = mgr.resolve(IconId::SketchLine);
    let _ = mgr.resolve(IconId::SketchCircle);

    // Perform 6 consecutive edits with polling checks
    for edit_num in 1..=6 {
        // Sleep past the 250ms throttle
        std::thread::sleep(std::time::Duration::from_millis(260));

        let new_content = format!(r#"<svg viewBox="0 0 24 24"><path d="M0 0 L{} {}"/></svg>"#, edit_num * 10, edit_num * 10);
        if edit_num % 2 == 1 {
            std::fs::write(&svg_line, new_content.as_bytes()).unwrap();
        } else {
            std::fs::write(&svg_circle, new_content.as_bytes()).unwrap();
        }

        let detected = mgr.check_watched_directories();
        assert!(detected, "Edit {} MUST be detected by check_watched_directories", edit_num);

        if edit_num % 2 == 1 {
            let res = mgr.resolve(IconId::SketchLine);
            assert_eq!(res.data, new_content.as_bytes(), "Edit {} data mismatch", edit_num);
        } else {
            let res = mgr.resolve(IconId::SketchCircle);
            assert_eq!(res.data, new_content.as_bytes(), "Edit {} data mismatch", edit_num);
        }
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn qicons_valid_trailer_and_tampered_downgrade() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_tamper_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let pack_dir = temp_dir.join("tamper_src");
    std::fs::create_dir_all(pack_dir.join("icons").join("sketch")).unwrap();

    let manifest = IconManifest {
        package_type: PackageType::IconTheme,
        id: "tamper-test".to_string(),
        name: "Tamper Test".to_string(),
        version: "1.0.0".to_string(),
        author: "Dev".to_string(),
        license: "MIT".to_string(),
        description: "Test".to_string(),
        color_mode: ColorMode::Universal,
        verified: false,
    };
    std::fs::write(pack_dir.join("manifest.ron"), manifest.to_ron().unwrap()).unwrap();
    let valid_svg = br#"<svg viewBox="0 0 24 24"><line x1="0" y1="0" x2="24" y2="24"/></svg>"#;
    std::fs::write(pack_dir.join("icons").join("sketch").join("line.svg"), valid_svg).unwrap();

    // 1. Package legitimate .qicons bundle with trailing verification record
    let bundle_path = temp_dir.join("official.qicons");
    let report = package_bundle(&pack_dir, &manifest, &bundle_path).expect("bundle packaging succeeds");
    assert_eq!(report.included.len(), 1);

    // 2. Load genuine bundle: should be VerifiedArchive with is_tampered == false
    let pack = IconPack::from_archive(&bundle_path).expect("genuine bundle loads cleanly");
    assert_eq!(pack.format(), BundleFormat::VerifiedArchive);
    assert!(!pack.is_tampered);
    assert!(pack.manifest.verified);
    assert_eq!(pack.get_svg_for_id(IconId::SketchLine).unwrap(), valid_svg);

    // 3. Tamper with the bundle bytes (e.g. external archiver or binary patch modification)
    let mut tampered_bytes = std::fs::read(&bundle_path).unwrap();
    assert!(tampered_bytes.len() > 40);
    // Alter a byte inside the zip payload (before the 40-byte trailer)
    tampered_bytes[10] ^= 0xFF;
    let tampered_path = temp_dir.join("tampered.qicons");
    std::fs::write(&tampered_path, &tampered_bytes).unwrap();

    // 4. Load tampered file: zip archive itself is broken or hash fails
    // Even if it parses as zip or if trailer hash doesn't match:
    // When SHA-256 doesn't match, verify_qicons_trailer returns Tampered
    let check = crate::icons::sha256::verify_qicons_trailer(&tampered_bytes);
    assert_eq!(check, crate::icons::sha256::TrailerCheck::Tampered);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn generic_zip_without_manifest_loads_as_archive_with_crash_guard() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_zip_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Create a generic .zip archive with NO manifest.ron (community zip)
    let zip_path = temp_dir.join("my-cool-pack.zip");
    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = zip::write::SimpleFileOptions::default();

    let valid_svg = br#"<svg viewBox="0 0 32 32"><circle cx="16" cy="16" r="10"/></svg>"#;
    zip.start_file("icons/sketch/circle.svg", options).unwrap();
    std::io::Write::write_all(&mut zip, valid_svg).unwrap();

    // Malicious XML entity bomb
    let entity_bomb = br#"<?xml version="1.0"?><!DOCTYPE lolz [<!ENTITY lol "lol"><!ELEMENT lolz (#PCDATA)>]><svg viewBox="0 0 10 10">&lol;</svg>"#;
    zip.start_file("icons/sketch/line.svg", options).unwrap();
    std::io::Write::write_all(&mut zip, entity_bomb).unwrap();

    zip.finish().unwrap();

    // Load generic .zip
    let pack = IconPack::from_archive(&zip_path).expect("generic zip must load");
    // Format must be Archive (NOT VerifiedArchive)
    assert_eq!(pack.format(), BundleFormat::Archive);
    assert!(!pack.manifest.verified);
    assert!(!pack.is_tampered);
    // Manifest synthesized from filename stem
    assert_eq!(pack.manifest.name, "my-cool-pack");

    // Valid SVG should be retrieved
    let circle_data = pack.get_svg_for_id(IconId::SketchCircle);
    assert_eq!(circle_data.unwrap(), valid_svg);

    // Dangerous XML entity bomb must be rejected by crash-guard (returns None instead of crashing)
    let line_data = pack.get_svg_for_id(IconId::SketchLine);
    assert!(line_data.is_none(), "Entity bomb should be filtered out by crash-guard");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn zip_bomb_excessive_compression_ratio_is_rejected() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_bomb_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    // Create a zip bomb: 500 KB of zeroes compresses to ~500 bytes (ratio ~1000:1)
    let bomb_path = temp_dir.join("bomb.zip");
    let file = std::fs::File::create(&bomb_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("icons/sketch/line.svg", options).unwrap();
    let zero_payload = vec![0u8; 500 * 1024];
    std::io::Write::write_all(&mut zip, &zero_payload).unwrap();
    zip.finish().unwrap();

    let res = IconPack::from_archive(&bomb_path);
    assert!(res.is_err(), "Zip bomb archive must be rejected");
    let err = res.unwrap_err();
    assert!(
        err.contains("possible decompression bomb") || err.contains("suspicious compression ratio"),
        "Expected decompression bomb error, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn zip_slip_path_traversal_is_rejected() {
    let temp_dir = std::env::temp_dir().join(format!("qymcad_slip_test_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let slip_path = temp_dir.join("slip.zip");
    let file = std::fs::File::create(&slip_path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    zip.start_file("../../etc/passwd", options).unwrap();
    std::io::Write::write_all(&mut zip, b"malicious").unwrap();
    zip.finish().unwrap();

    let res = IconPack::from_archive(&slip_path);
    assert!(res.is_err(), "Zip slip archive must be rejected");
    let err = res.unwrap_err();
    assert!(
        err.contains("insecure file path"),
        "Expected insecure file path error, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
