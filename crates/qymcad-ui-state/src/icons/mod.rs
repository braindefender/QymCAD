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

#[cfg(test)]
mod tests;

pub use bundle::{
    discover_packs_in, inspect_pack_directory, package_bundle, package_bundle_to_bytes,
    package_bundle_to_writer, validate_svg, ValidationReport,
};
pub use id::{IconId, IconSource, ALL_ICONS};
pub use manager::{
    clear_global_icon_cache, load_default_pack, reload_active_icon_themes, resolve_global_icon,
    set_global_icon_manager, with_global_icon_manager, with_global_icon_manager_mut, IconManager,
    ResolvedIcon, DEFAULT_QICONS,
};
pub use manifest::{ColorMode, IconManifest, PackageType};
pub use pack::{IconPack, PackSource};
