//! Dynamic vector icon themes and bundles.
//!
//! A bundle is a directory or a `.qicons` archive (ZIP) carrying an `icons/` folder and a `manifest.ron`.
//! Tools resolve their icons through a cascade stack: top active pack -> lower packs -> built-in
//! default SVG pack.
//!
//! Strict folder paths determine icon identity (`icons/sketch/line.svg` -> `IconId::SketchLine`),
//! eliminating redundant mapping tables.

pub mod bundle;
pub mod id;
pub mod manager;
pub mod manifest;
pub mod pack;
pub mod sha256;

#[cfg(test)]
mod tests;

pub use bundle::{
    clean_directory_icon, clean_directory_icons, clean_svg, directory_has_cleanable_icons, discover_packs_detailed, discover_packs_in, find_svg_junk_issues, inspect_pack_directory, package_bundle,
    package_bundle_to_bytes, package_bundle_to_writer, validate_icon_svg, validate_svg, CleanFileFailure, CleanIconResult, CleanPackReport, DiscoveryError, DiscoveryReport, ValidationReport,
    inspect_pack_directory_for_mode,
};
pub use id::{IconId, IconSource, ALL_ICONS};
pub use manager::{
    clear_global_icon_cache, get_global_icon_revision, has_watched_icon_packs, is_global_pack_watched, load_builtin_pack, load_builtin_packs, load_default_pack, poll_watched_icon_packs,
    prepare_monochrome_svg, reload_active_icon_themes, resolve_global_icon, set_global_dev_watch, set_global_icon_manager, set_global_pack_watching, sync_global_watched_packs,
    with_global_icon_manager, with_global_icon_manager_mut, IconManager, ResolvedIcon, BUILTIN_ICON_THEMES, DEFAULT_QICONS,
};
pub use manifest::{
    has_zalgo, is_combining_mark, ColorMode, IconManifest, LocalizedThemeText, PackageType, MAX_MANIFEST_AUTHOR_LEN, MAX_MANIFEST_DESCRIPTION_LEN, MAX_MANIFEST_ID_LEN, MAX_MANIFEST_LICENSE_LEN,
    MAX_MANIFEST_LOCALE_LEN, MAX_MANIFEST_NAME_LEN, MAX_MANIFEST_TRANSLATIONS, MAX_MANIFEST_VERSION_LEN,
};
pub use pack::{BundleFormat, IconPack, PackSource};
