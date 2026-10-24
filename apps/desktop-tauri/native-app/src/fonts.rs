//! Installed font discovery and the egui font stack built from the user's choices.
//!
//! egui bundles Latin-only faces, so the app has always appended a system face
//! for scripts those faces lack. Preferences now name a family per role; this
//! module resolves a family to a font file with fontconfig, caches the installed
//! families for the picker, and builds the egui chains with the bundled faces
//! kept behind the chosen family so missing scripts still fall through.

use crate::ui_preferences::FontSettings;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Faces appended after the bundled ones so uncommon scripts still render.
const PROPORTIONAL_FALLBACKS: [&str; 4] = [
    "/usr/share/fonts/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans.ttf",
];
const MONOSPACE_FALLBACKS: [&str; 4] = [
    "/usr/share/fonts/liberation/LiberationMono-Regular.ttf",
    "/usr/share/fonts/noto/NotoSansMono-Regular.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
];

/// One installed family and the file egui can load for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontFamily {
    /// Family name as fontconfig reports it.
    pub name: String,
    /// Path to a single-face `.ttf`/`.otf` file.
    pub file: PathBuf,
}

/// Families installed on this machine, ordered by name.
#[derive(Debug, Default, Clone)]
pub struct FontCatalog {
    families: Vec<FontFamily>,
}

impl FontCatalog {
    /// Parses `fc-list --format '%{family}\t%{file}\n'` output.
    ///
    /// Font collections (`.ttc`) are skipped because egui loads single faces,
    /// and a family that appears in several files keeps its first regular file.
    pub fn parse(text: &str) -> Self {
        let mut families: Vec<FontFamily> = Vec::new();
        for line in text.lines() {
            let Some((names, file)) = line.split_once('\t') else {
                continue;
            };
            let file = file.trim();
            if !is_single_face(file) {
                continue;
            }
            let Some(name) = names.split(',').map(str::trim).find(|name| !name.is_empty()) else {
                continue;
            };
            if families.iter().any(|known| known.name == name) {
                continue;
            }
            families.push(FontFamily {
                name: name.to_owned(),
                file: PathBuf::from(file),
            });
        }
        families.sort_by_key(|family| family.name.to_lowercase());
        Self { families }
    }

    /// Reads the installed families from fontconfig, falling back to the known
    /// system faces when `fc-list` is unavailable.
    pub fn scan() -> Self {
        let output = std::process::Command::new("fc-list")
            .args(["--format", "%{family}\t%{file}\n"])
            .output();
        match output {
            Ok(output) if output.status.success() => {
                let catalog = Self::parse(&String::from_utf8_lossy(&output.stdout));
                if !catalog.families.is_empty() {
                    return catalog;
                }
            }
            Ok(_) | Err(_) => {}
        }
        Self::from_known_paths()
    }

    /// The fallback catalog built from paths every Linux desktop ships.
    fn from_known_paths() -> Self {
        let mut catalog = Self::default();
        for path in PROPORTIONAL_FALLBACKS.into_iter().chain(MONOSPACE_FALLBACKS) {
            if !Path::new(path).is_file() {
                continue;
            }
            let Some(name) = Path::new(path).file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if catalog.families.iter().any(|known| known.name == name) {
                continue;
            }
            catalog.families.push(FontFamily {
                name: name.to_owned(),
                file: PathBuf::from(path),
            });
        }
        catalog.families.sort_by_key(|family| family.name.to_lowercase());
        catalog
    }

    /// Installed families in display order.
    pub fn families(&self) -> &[FontFamily] {
        &self.families
    }

    /// Resolves a configured family name to a loadable file.
    ///
    /// Exact names win over case-insensitive matches, which win over substrings,
    /// so a family keeps its face when the reported name carries a suffix.
    pub fn file_for(&self, family: &str) -> Option<&PathBuf> {
        if family.is_empty() {
            return None;
        }
        let wanted = family.to_lowercase();
        self.families
            .iter()
            .find(|known| known.name == family)
            .or_else(|| {
                self.families
                    .iter()
                    .find(|known| known.name.to_lowercase() == wanted)
            })
            .or_else(|| {
                self.families
                    .iter()
                    .find(|known| known.name.to_lowercase().contains(&wanted))
            })
            .map(|known| &known.file)
    }
}

fn is_single_face(file: &str) -> bool {
    let lower = file.to_lowercase();
    lower.ends_with(".ttf") || lower.ends_with(".otf")
}

/// Catalog shared between the startup scan and the settings editor.
pub type SharedCatalog = Arc<Mutex<Option<FontCatalog>>>;

/// Creates the empty shared catalog and starts scanning in the background.
///
/// Scanning shells out to fontconfig, so it must not block the first frame.
pub fn shared_catalog() -> SharedCatalog {
    let shared: SharedCatalog = Arc::new(Mutex::new(None));
    let worker = Arc::clone(&shared);
    std::thread::spawn(move || {
        let catalog = FontCatalog::scan();
        if let Ok(mut slot) = worker.lock() {
            *slot = Some(catalog);
        }
    });
    shared
}

/// Copies the scanned catalog for the current frame, if the scan finished.
pub fn catalog_snapshot(shared: &SharedCatalog) -> Option<FontCatalog> {
    shared.lock().ok().and_then(|slot| slot.clone())
}

/// Reads one face, reporting why it could not be used.
fn read_face(path: &Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) if !bytes.is_empty() => Some(bytes),
        Ok(_) | Err(_) => None,
    }
}

/// Builds the egui font chains from the configured families.
///
/// The chosen family leads its chain, the bundled faces follow so uncovered
/// glyphs still resolve, and the system faces stay last. A configured family
/// that cannot be loaded is reported and skipped rather than failing startup.
///
/// @param context - Context whose fonts are replaced.
/// @param settings - Configured family names; empty means bundled defaults.
/// @param catalog - Installed families used to resolve a name to a file.
/// @returns One message per configured family that could not be loaded.
pub fn install(
    context: &egui::Context,
    settings: &FontSettings,
    catalog: &FontCatalog,
) -> Vec<String> {
    let mut definitions = egui::FontDefinitions::default();
    let mut warnings = Vec::new();
    for (family, chosen, fallbacks) in [
        (
            egui::FontFamily::Proportional,
            settings.proportional.as_str(),
            PROPORTIONAL_FALLBACKS,
        ),
        (
            egui::FontFamily::Monospace,
            settings.monospace.as_str(),
            MONOSPACE_FALLBACKS,
        ),
    ] {
        let preferred = if chosen.is_empty() {
            None
        } else {
            match catalog.file_for(chosen).and_then(|path| read_face(path)) {
                Some(bytes) => {
                    let name = format!("preferred-{}", chosen.replace(' ', "-"));
                    definitions.font_data.insert(
                        name.clone(),
                        Arc::new(egui::FontData::from_owned(bytes)),
                    );
                    Some(name)
                }
                None => {
                    warnings.push(format!(
                        "font family \"{chosen}\" is not installed; the bundled face is used"
                    ));
                    None
                }
            }
        };
        let Some(chain) = definitions.families.get_mut(&family) else {
            continue;
        };
        if let Some(preferred) = preferred {
            chain.insert(0, preferred);
        }
        for path in fallbacks {
            let Some(path) = Path::new(path).to_str() else {
                continue;
            };
            let Some(bytes) = read_face(Path::new(path)) else {
                continue;
            };
            let name = format!(
                "system-{}",
                path.rsplit('/').next().unwrap_or("font").replace('.', "-")
            );
            definitions
                .font_data
                .entry(name.clone())
                .or_insert_with(|| Arc::new(egui::FontData::from_owned(bytes)));
            if !chain.contains(&name) {
                chain.push(name);
            }
        }
    }
    context.set_fonts(definitions);
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "JetBrains Mono,JetBrains Mono NL\t/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf\n\
Noto Sans CJK JP\t/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc\n\
Liberation Sans\t/usr/share/fonts/liberation/LiberationSans-Regular.ttf\n\
Liberation Sans\t/usr/share/fonts/liberation/LiberationSans-Bold.ttf\n";

    #[test]
    fn parses_single_faces_and_skips_collections() {
        let catalog = FontCatalog::parse(SAMPLE);
        let names: Vec<&str> = catalog
            .families()
            .iter()
            .map(|family| family.name.as_str())
            .collect();
        assert_eq!(names, ["JetBrains Mono", "Liberation Sans"]);
        assert_eq!(
            catalog.file_for("Liberation Sans"),
            Some(&PathBuf::from(
                "/usr/share/fonts/liberation/LiberationSans-Regular.ttf"
            ))
        );
    }

    #[test]
    fn resolves_names_exactly_then_case_insensitively_then_by_suffix() {
        let catalog = FontCatalog::parse(SAMPLE);
        assert!(catalog.file_for("JetBrains Mono").is_some());
        assert!(catalog.file_for("jetbrains mono").is_some());
        assert!(catalog.file_for("Mono").is_some());
        assert!(catalog.file_for("Missing Face").is_none());
        assert!(catalog.file_for("").is_none());
    }
}
