//! Font resolution against whatever the system has installed.
//!
//! We bundle nothing, so the reading face is chosen by walking a preference
//! chain and taking the first family actually present. Where a face is missing
//! (no true italic, for instance) cosmic-text's synthetic styling covers it.

use cosmic_text::{FontSystem, fontdb};

/// Preferred body families, best first.
const SERIF_CHAIN: &[&str] = &[
    "Charis SIL",
    "Literata",
    "Source Serif 4",
    "EB Garamond",
    "Noto Serif",
    "DejaVu Serif",
    "Liberation Serif",
    "Times New Roman",
];

/// Preferred monospace families for `code`.
const MONO_CHAIN: &[&str] = &[
    "JetBrains Mono",
    "Iosevka",
    "Noto Sans Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Courier New",
];

/// The families this installation will actually render with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontStack {
    pub body: String,
    pub mono: String,
}

impl FontStack {
    /// Resolve the stack, honoring explicit user choices when those families
    /// exist and falling back through the preference chains otherwise.
    pub fn resolve(fonts: &FontSystem, body_pref: Option<&str>, mono_pref: Option<&str>) -> Self {
        let installed = installed_families(fonts);
        let pick = |pref: Option<&str>, chain: &[&str], generic: &str| -> String {
            if let Some(name) = pref
                && installed.iter().any(|f| f.eq_ignore_ascii_case(name))
            {
                return name.to_string();
            }
            chain
                .iter()
                .find(|candidate| installed.iter().any(|f| f.eq_ignore_ascii_case(candidate)))
                .map(|f| (*f).to_string())
                .unwrap_or_else(|| generic.to_string())
        };
        Self {
            body: pick(body_pref, SERIF_CHAIN, "serif"),
            mono: pick(mono_pref, MONO_CHAIN, "monospace"),
        }
    }
}

/// Every family name in the font database, sorted and deduplicated, for the
/// settings panel's font list.
pub fn installed_families(fonts: &FontSystem) -> Vec<String> {
    let mut families: Vec<String> = fonts
        .db()
        .faces()
        .flat_map(|face: &fontdb::FaceInfo| face.families.iter().map(|(name, _)| name.clone()))
        .collect();
    families.sort_unstable();
    families.dedup();
    families
}
