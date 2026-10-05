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

/// Maximum length of a theme bundle ID (64 characters).
pub const MAX_MANIFEST_ID_LEN: usize = 64;

/// Maximum length of a theme name (64 characters).
pub const MAX_MANIFEST_NAME_LEN: usize = 64;

/// Maximum length of a version string (32 characters).
pub const MAX_MANIFEST_VERSION_LEN: usize = 32;

/// Maximum length of an author string (64 characters).
pub const MAX_MANIFEST_AUTHOR_LEN: usize = 64;

/// Maximum length of a license identifier (64 characters).
pub const MAX_MANIFEST_LICENSE_LEN: usize = 64;

/// Maximum length of a manifest description summary (512 characters).
pub const MAX_MANIFEST_DESCRIPTION_LEN: usize = 512;

/// Maximum number of translation entries permitted in a manifest.
pub const MAX_MANIFEST_TRANSLATIONS: usize = 32;

/// Maximum length of a translation locale tag (16 characters).
pub const MAX_MANIFEST_LOCALE_LEN: usize = 16;

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

    /// Validate manifest fields to prevent UI distortion, path traversal, and resource issues.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() {
            return Err("theme ID cannot be empty".to_string());
        }
        if self.id.chars().count() > MAX_MANIFEST_ID_LEN {
            return Err(format!("theme ID exceeds maximum length of {MAX_MANIFEST_ID_LEN} characters"));
        }
        if !self.id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            || !self.id.ends_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err(format!("theme ID must be lowercase ASCII alphanumeric with hyphens or underscores, and start and end with an alphanumeric character (got {:?})", self.id));
        }

        let trimmed_name = self.name.trim();
        if trimmed_name.is_empty() {
            return Err("theme name cannot be empty".to_string());
        }
        if self.name.chars().count() > MAX_MANIFEST_NAME_LEN {
            return Err(format!("theme name exceeds maximum length of {MAX_MANIFEST_NAME_LEN} characters"));
        }
        if self.name.chars().any(|c| c == '\n' || c == '\r' || c.is_control()) {
            return Err("theme name cannot contain newlines or control characters".to_string());
        }

        if self.version.chars().count() > MAX_MANIFEST_VERSION_LEN {
            return Err(format!("version exceeds maximum length of {MAX_MANIFEST_VERSION_LEN} characters"));
        }
        if self.version.chars().any(|c| c.is_control()) {
            return Err("version cannot contain control characters".to_string());
        }

        if self.author.chars().count() > MAX_MANIFEST_AUTHOR_LEN {
            return Err(format!("author exceeds maximum length of {MAX_MANIFEST_AUTHOR_LEN} characters"));
        }
        if self.author.chars().any(|c| c.is_control()) {
            return Err("author cannot contain control characters".to_string());
        }

        if self.license.chars().count() > MAX_MANIFEST_LICENSE_LEN {
            return Err(format!("license exceeds maximum length of {MAX_MANIFEST_LICENSE_LEN} characters"));
        }
        if self.license.chars().any(|c| c.is_control()) {
            return Err("license cannot contain control characters".to_string());
        }

        if self.description.chars().count() > MAX_MANIFEST_DESCRIPTION_LEN {
            return Err(format!("description exceeds maximum length of {MAX_MANIFEST_DESCRIPTION_LEN} characters"));
        }
        if self.description.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') {
            return Err("description cannot contain control characters".to_string());
        }

        if self.translations.len() > MAX_MANIFEST_TRANSLATIONS {
            return Err(format!("too many translation entries: {} (maximum is {MAX_MANIFEST_TRANSLATIONS})", self.translations.len()));
        }

        for (locale, text) in &self.translations {
            if locale.chars().count() > MAX_MANIFEST_LOCALE_LEN {
                return Err(format!("locale tag {locale:?} exceeds maximum length of {MAX_MANIFEST_LOCALE_LEN} characters"));
            }
            if locale.parse::<unic_langid::LanguageIdentifier>().is_err() {
                return Err(format!("invalid locale tag: {locale:?}"));
            }
            if text.name.chars().count() > MAX_MANIFEST_NAME_LEN {
                return Err(format!("translated name for {locale} exceeds maximum length of {MAX_MANIFEST_NAME_LEN} characters"));
            }
            if text.name.chars().any(|c| c == '\n' || c == '\r' || c.is_control()) {
                return Err(format!("translated name for {locale} cannot contain newlines or control characters"));
            }
            if text.description.chars().count() > MAX_MANIFEST_DESCRIPTION_LEN {
                return Err(format!("translated description for {locale} exceeds maximum length of {MAX_MANIFEST_DESCRIPTION_LEN} characters"));
            }
            if text.description.chars().any(|c| c.is_control() && c != '\n' && c != '\r' && c != '\t') {
                return Err(format!("translated description for {locale} cannot contain control characters"));
            }
        }

        Ok(())
    }

    pub fn parse_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }
}
