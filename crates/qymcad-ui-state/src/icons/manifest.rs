//! Manifest metadata and configuration for icon theme bundles.

/// The package type tag for future plugin system compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PackageType {
    IconTheme,
}

impl Default for PackageType {
    fn default() -> Self {
        Self::IconTheme
    }
}

/// Colour rendering mode of the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColorMode {
    /// Full-colour icons rendered as-is across all UI themes.
    Universal,
    /// Single-colour icons dynamically tinted by the active theme foreground.
    Monochrome,
}

impl Default for ColorMode {
    fn default() -> Self {
        Self::Universal
    }
}

/// Metadata describing an icon pack bundle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IconManifest {
    #[serde(default)]
    pub package_type: PackageType,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub color_mode: ColorMode,
    /// Whether the package is verified by QymCAD packager.
    #[serde(default)]
    pub verified: bool,
}

impl IconManifest {
    pub fn parse_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }
}
