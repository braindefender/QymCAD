//! Manifest metadata and configuration for icon theme bundles.

use std::collections::BTreeMap;

/// Localized text displayed for an icon theme.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LocalizedThemeText {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// The requested language followed by its primary language, with duplicates removed.
pub(crate) fn locale_fallbacks(locale: &str) -> Vec<String> {
    let Ok(tag) = locale.parse::<unic_langid::LanguageIdentifier>() else {
        return Vec::new();
    };
    let exact = tag.to_string();
    let primary = tag.language.to_string();
    if exact == primary {
        vec![exact]
    } else {
        vec![exact, primary]
    }
}

/// The package type tag for future plugin system compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PackageType {
    #[default]
    IconTheme,
}

/// Colour rendering mode of the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ColorMode {
    /// Full-colour icons rendered as-is across all UI themes.
    #[default]
    Universal,
    /// Single-colour icons dynamically tinted by the active theme foreground.
    Monochrome,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, LocalizedThemeText>,
    #[serde(default)]
    pub color_mode: ColorMode,
    /// Whether the package is verified by QymCAD packager.
    #[serde(default)]
    pub verified: bool,
}

impl IconManifest {
    pub fn name_for_locale(&self, locale: &str) -> &str {
        for tag in locale_fallbacks(locale) {
            if let Some(text) = self.translations.get(&tag).map(|translation| translation.name.as_str()).filter(|text| !text.trim().is_empty()) {
                return text;
            }
        }
        &self.name
    }

    pub fn description_for_locale(&self, locale: &str) -> &str {
        for tag in locale_fallbacks(locale) {
            if let Some(text) = self.translations.get(&tag).map(|translation| translation.description.as_str()).filter(|text| !text.trim().is_empty()) {
                return text;
            }
        }
        &self.description
    }

    pub fn parse_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }
}
