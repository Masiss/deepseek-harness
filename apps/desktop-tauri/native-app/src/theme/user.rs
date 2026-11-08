//! User theme files: discovery, installation, editing, and export.
//!
//! A user pack lives at `<themes-dir>/<id>.json` and is untrusted. Discovery
//! validates every file independently, keeps built-ins when no user file shadows
//! them, and records unreadable or invalid files as list entries instead of
//! failing startup. Resolution applies the contrast gate to a loaded user pack.

use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::contrast;
use super::pack::ThemePack;
use super::remote::{self, ThemeProvenance};
use super::store::{builtin_packs, CUSTOM_PACK_ID};
use super::tokens::Scheme;
use crate::ui_preferences::AccentScheme;

/// File name suffix discovery accepts.
const THEME_FILE_EXTENSION: &str = "json";

/// Longest theme id, matching the pack schema.
const ID_MAX_CHARS: usize = 64;

/// Where one installed pack lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ThemeSource {
    /// Compiled into the app.
    Builtin,
    /// Read from `<themes-dir>/<id>.json`.
    User,
    /// Downloaded from GitHub into `<themes-dir>/cache/<id>.json`.
    Github,
}

impl ThemeSource {
    /// Source badge text shown in the Preferences list.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::User => "user",
            Self::Github => "github",
        }
    }
}

/// One pack the Preferences list can select.
#[derive(Clone, Debug)]
pub(crate) struct InstalledTheme {
    /// Parsed pack document.
    pub(crate) pack: ThemePack,
    /// Whether the pack ships with the app or came from a user file.
    pub(crate) source: ThemeSource,
    /// File the pack was read from; `None` for a built-in.
    pub(crate) path: Option<PathBuf>,
    /// Fetch provenance for a cached GitHub pack.
    pub(crate) provenance: Option<ThemeProvenance>,
}

/// One theme file that discovery could not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThemeFileError {
    /// File name shown in the Preferences list.
    pub(crate) file_name: String,
    /// Concrete reason the file was rejected.
    pub(crate) reason: String,
}

impl fmt::Display for ThemeFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.file_name, self.reason)
    }
}

/// Contract outcome for one pack in the active scheme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContrastStatus {
    /// Every required pair meets its ratio.
    Pass,
    /// Required pairs pass; the listed warning-only pairs do not.
    Warnings(usize),
    /// An untrusted pack fails a required pair and cannot render.
    Rejected(Vec<String>),
}

/// Discovered packs plus the files discovery rejected.
#[derive(Clone, Debug)]
pub(crate) struct ThemeCatalog {
    dir: PathBuf,
    themes: Vec<InstalledTheme>,
    errors: Vec<ThemeFileError>,
}

impl ThemeCatalog {
    /// The compiled-in packs without reading the filesystem.
    #[cfg(test)]
    pub(crate) fn builtins() -> Self {
        Self {
            dir: PathBuf::new(),
            themes: builtin_themes(),
            errors: Vec::new(),
        }
    }

    /// Loads built-ins and every `*.json` file in `dir`.
    ///
    /// Built-ins load first, then cached GitHub documents, then local user files.
    /// A user file shadows a cached or built-in pack with the same id, and a
    /// cached pack shadows a built-in. Files are visited in file-name order; a
    /// second user file with an id already installed becomes an error entry so
    /// the first file stays deterministic. A missing directory is an empty
    /// catalogue, not a failure.
    ///
    /// @param dir — directory that holds user theme files.
    /// @returns the catalogue and every rejected file.
    pub(crate) fn load(dir: &Path) -> Self {
        let mut catalog = Self {
            dir: dir.to_owned(),
            themes: builtin_themes(),
            errors: Vec::new(),
        };
        let cache = remote::discover_cache(&remote::cache_dir(dir));
        for cached in cache.themes {
            catalog.themes.retain(|theme| theme.pack.id != cached.pack.id);
            catalog.themes.push(InstalledTheme {
                pack: cached.pack,
                source: ThemeSource::Github,
                path: Some(cached.path),
                provenance: Some(cached.meta),
            });
        }
        catalog.errors.extend(cache.errors.into_iter().map(|error| ThemeFileError {
            file_name: error.file_name,
            reason: error.reason,
        }));
        match fs::read_dir(dir) {
            Ok(entries) => {
                let mut files: Vec<PathBuf> = entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry
                            .file_type()
                            .map(|kind| kind.is_file())
                            .unwrap_or(false)
                    })
                    .map(|entry| entry.path())
                    .filter(|path| {
                        path.extension().and_then(|extension| extension.to_str())
                            == Some(THEME_FILE_EXTENSION)
                    })
                    .collect();
                files.sort();
                for path in files {
                    catalog.load_file(&path);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => catalog.errors.push(ThemeFileError {
                file_name: dir.display().to_string(),
                reason: format!("cannot read the theme directory: {error}"),
            }),
        }
        catalog
    }

    /// Installed packs in display order.
    pub(crate) fn themes(&self) -> &[InstalledTheme] {
        &self.themes
    }

    /// Files discovery rejected.
    pub(crate) fn errors(&self) -> &[ThemeFileError] {
        &self.errors
    }

    /// The installed pack with this id, whichever source won.
    pub(crate) fn find(&self, id: &str) -> Option<&InstalledTheme> {
        self.themes.iter().find(|theme| theme.pack.id == id)
    }

    /// Contrast status for one installed pack in the active scheme and accent.
    ///
    /// @param id — installed pack id.
    /// @param accent — accent family preference applied before measuring.
    /// @param dark — whether the dark scheme is active.
    /// @returns the gate outcome, or `None` when the pack is not installed.
    pub(crate) fn contrast_status(
        &self,
        id: &str,
        accent: AccentScheme,
        dark: bool,
    ) -> Option<ContrastStatus> {
        let pack = &self.find(id)?.pack;
        let colors = match pack.colors_for(Scheme::of(dark), accent.pack_accent_key()) {
            Ok(colors) => colors,
            Err(_) => return Some(ContrastStatus::Rejected(Vec::new())),
        };
        let report = contrast::report(&colors);
        if !report.failures.is_empty() {
            Some(ContrastStatus::Rejected(
                report.failures.iter().map(ToString::to_string).collect(),
            ))
        } else if !report.warnings.is_empty() {
            Some(ContrastStatus::Warnings(report.warnings.len()))
        } else {
            Some(ContrastStatus::Pass)
        }
    }

    /// Reads one user file as editable text.
    ///
    /// @param id — installed user pack id.
    /// @returns the file text, or the reason it cannot be edited.
    pub(crate) fn edit_text(&self, id: &str) -> Result<String, ThemeCatalogError> {
        let theme = self.user_theme(id)?;
        let path = theme
            .path
            .as_ref()
            .ok_or_else(|| ThemeCatalogError::NotUser { id: id.to_owned() })?;
        fs::read_to_string(path).map_err(|source| ThemeCatalogError::Io {
            operation: "read theme file",
            path: path.clone(),
            source,
        })
    }

    /// Copies one source file into the catalogue as `<id>.json`.
    ///
    /// @param source — JSON file chosen by the user.
    /// @param replace — whether an existing file for the same id may be replaced.
    /// @returns the installed id.
    pub(crate) fn import_file(
        &mut self,
        source: &Path,
        replace: bool,
    ) -> Result<String, ThemeCatalogError> {
        let bytes = fs::read(source).map_err(|source_error| ThemeCatalogError::Io {
            operation: "read theme file",
            path: source.to_owned(),
            source: source_error,
        })?;
        let text = String::from_utf8(bytes).map_err(|_| ThemeCatalogError::Invalid {
            path: source.to_owned(),
            reason: "the theme file is not valid UTF-8".to_owned(),
        })?;
        let pack = ThemePack::parse(&text).map_err(|error| ThemeCatalogError::Invalid {
            path: source.to_owned(),
            reason: error.to_string(),
        })?;
        if pack.id == CUSTOM_PACK_ID {
            return Err(ThemeCatalogError::Reserved { id: pack.id });
        }
        if super::store::builtin_pack(&pack.id).is_some() {
            return Err(ThemeCatalogError::Builtin { id: pack.id });
        }
        let target = self.dir.join(format!("{}.json", pack.id));
        if target.exists() && !replace {
            return Err(ThemeCatalogError::AlreadyInstalled { id: pack.id });
        }
        write_atomic(&target, text.as_bytes())?;
        self.reload();
        Ok(pack.id)
    }

    /// Writes a complete copy of one installed pack under a fresh id.
    ///
    /// @param id — installed pack to copy.
    /// @returns the new pack id.
    pub(crate) fn duplicate(&mut self, id: &str) -> Result<String, ThemeCatalogError> {
        let source = self
            .find(id)
            .ok_or_else(|| ThemeCatalogError::NotFound { id: id.to_owned() })?;
        let copy_id = self.unique_copy_id(id);
        let mut copy = source.pack.clone();
        copy.id = copy_id.clone();
        copy.name = format!("{} copy", copy.name);
        copy.source = "user".to_owned();
        let text = serialize_pack(&copy)?;
        let target = self.dir.join(format!("{copy_id}.json"));
        write_atomic(&target, text.as_bytes())?;
        self.reload();
        Ok(copy_id)
    }

    /// Validates edited pack text and replaces its user file atomically.
    ///
    /// An id change renames the file. The new id must not name a built-in or an
    /// existing user file other than the one being edited.
    ///
    /// @param original_id — id of the user pack being edited.
    /// @param text — complete replacement document.
    /// @returns the id after saving, which differs when the document changed ids.
    pub(crate) fn save_edit(
        &mut self,
        original_id: &str,
        text: &str,
    ) -> Result<String, ThemeCatalogError> {
        let original = self.user_theme(original_id)?;
        let original_path = original
            .path
            .clone()
            .ok_or_else(|| ThemeCatalogError::NotUser {
                id: original_id.to_owned(),
            })?;
        let pack = ThemePack::parse(text).map_err(|error| ThemeCatalogError::Invalid {
            path: original_path.clone(),
            reason: error.to_string(),
        })?;
        if pack.id == CUSTOM_PACK_ID {
            return Err(ThemeCatalogError::Reserved { id: pack.id });
        }
        if super::store::builtin_pack(&pack.id).is_some() {
            return Err(ThemeCatalogError::Builtin { id: pack.id });
        }
        let target = self.dir.join(format!("{}.json", pack.id));
        if target != original_path && target.exists() {
            return Err(ThemeCatalogError::AlreadyInstalled { id: pack.id });
        }
        write_atomic(&target, text.as_bytes())?;
        if target != original_path {
            fs::remove_file(&original_path).map_err(|source| ThemeCatalogError::Io {
                operation: "remove the renamed theme file",
                path: original_path,
                source,
            })?;
        }
        self.reload();
        Ok(pack.id)
    }

    /// Removes one user or cached GitHub pack file.
    ///
    /// @param id — installed user or cached pack id.
    pub(crate) fn remove(&mut self, id: &str) -> Result<(), ThemeCatalogError> {
        let source = self
            .find(id)
            .ok_or_else(|| ThemeCatalogError::NotFound { id: id.to_owned() })?
            .source;
        match source {
            ThemeSource::Builtin => {
                return Err(ThemeCatalogError::NotUser { id: id.to_owned() })
            }
            ThemeSource::User => {
                let theme = self.user_theme(id)?;
                let path = theme
                    .path
                    .clone()
                    .ok_or_else(|| ThemeCatalogError::NotUser { id: id.to_owned() })?;
                fs::remove_file(&path).map_err(|source| ThemeCatalogError::Io {
                    operation: "remove theme file",
                    path: path.clone(),
                    source,
                })?;
            }
            ThemeSource::Github => remote::remove_cached(&remote::cache_dir(&self.dir), id)
                .map_err(|error| ThemeCatalogError::Remote(error.to_string()))?,
        }
        self.reload();
        Ok(())
    }

    /// Serializes one installed pack for export.
    ///
    /// @param id — installed pack id; built-ins export too.
    /// @returns the complete JSON payload, newline terminated.
    pub(crate) fn export_text(&self, id: &str) -> Result<String, ThemeCatalogError> {
        let theme = self
            .find(id)
            .ok_or_else(|| ThemeCatalogError::NotFound { id: id.to_owned() })?;
        serialize_pack(&theme.pack)
    }

    /// Writes one installed pack to a chosen destination.
    ///
    /// @param id — installed pack id.
    /// @param destination — file the save dialog returned.
    pub(crate) fn export_to(&self, id: &str, destination: &Path) -> Result<(), ThemeCatalogError> {
        let text = self.export_text(id)?;
        write_atomic(destination, text.as_bytes())
    }

    fn load_file(&mut self, path: &Path) {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                self.errors.push(ThemeFileError {
                    file_name,
                    reason: format!("cannot read the file: {error}"),
                });
                return;
            }
        };
        let pack = match ThemePack::parse(&text) {
            Ok(pack) => pack,
            Err(error) => {
                self.errors.push(ThemeFileError {
                    file_name,
                    reason: error.to_string(),
                });
                return;
            }
        };
        if pack.id == CUSTOM_PACK_ID {
            self.errors.push(ThemeFileError {
                file_name,
                reason: format!(
                    "theme id `{CUSTOM_PACK_ID}` is reserved for the legacy custom palette"
                ),
            });
            return;
        }
        if self
            .themes
            .iter()
            .any(|theme| theme.source == ThemeSource::User && theme.pack.id == pack.id)
        {
            self.errors.push(ThemeFileError {
                file_name,
                reason: format!(
                    "theme id `{}` is already installed by an earlier file",
                    pack.id
                ),
            });
            return;
        }
        self.themes.retain(|theme| theme.pack.id != pack.id);
        self.themes.push(InstalledTheme {
            pack,
            source: ThemeSource::User,
            path: Some(path.to_owned()),
            provenance: None,
        });
    }

    fn user_theme(&self, id: &str) -> Result<&InstalledTheme, ThemeCatalogError> {
        let theme = self
            .find(id)
            .ok_or_else(|| ThemeCatalogError::NotFound { id: id.to_owned() })?;
        if theme.source != ThemeSource::User {
            return Err(ThemeCatalogError::NotUser { id: id.to_owned() });
        }
        Ok(theme)
    }

    fn unique_copy_id(&self, id: &str) -> String {
        for sequence in 1.. {
            let suffix = if sequence == 1 {
                "-copy".to_owned()
            } else {
                format!("-copy-{sequence}")
            };
            let base_budget = ID_MAX_CHARS.saturating_sub(suffix.len());
            let base: String = id.chars().take(base_budget).collect();
            let candidate = format!("{base}{suffix}");
            if self.find(&candidate).is_none() {
                return candidate;
            }
        }
        unreachable!("the copy suffix sequence is unbounded")
    }

    fn reload(&mut self) {
        *self = Self::load(&self.dir);
    }
}

/// One rejected theme operation with an actionable reason.
#[derive(Debug)]
pub(crate) enum ThemeCatalogError {
    /// A filesystem operation failed.
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    /// A theme document failed schema, id, or colour validation.
    Invalid { path: PathBuf, reason: String },
    /// A user file already provides this id.
    AlreadyInstalled { id: String },
    /// The id belongs to a pack compiled into the app.
    Builtin { id: String },
    /// The id names the legacy custom palette rather than a file pack.
    Reserved { id: String },
    /// The pack is installed but is not a user file.
    NotUser { id: String },
    /// No pack has this id.
    NotFound { id: String },
    /// A remote cache operation failed.
    Remote(String),
}

impl fmt::Display for ThemeCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
            Self::Invalid { path, reason } => {
                write!(
                    formatter,
                    "{} is not a usable theme: {reason}",
                    path.display()
                )
            }
            Self::AlreadyInstalled { id } => write!(
                formatter,
                "a user theme with id `{id}` already exists; replace it explicitly"
            ),
            Self::Builtin { id } => write!(
                formatter,
                "theme id `{id}` belongs to a built-in theme and cannot be replaced"
            ),
            Self::Reserved { id } => write!(
                formatter,
                "theme id `{id}` is reserved for the legacy custom palette"
            ),
            Self::NotUser { id } => write!(formatter, "theme `{id}` is not a user theme file"),
            Self::NotFound { id } => write!(formatter, "theme `{id}` is not installed"),
            Self::Remote(reason) => formatter.write_str(reason),
        }
    }
}

impl StdError for ThemeCatalogError {}

fn builtin_themes() -> Vec<InstalledTheme> {
    builtin_packs()
        .iter()
        .cloned()
        .map(|pack| InstalledTheme {
            pack,
            source: ThemeSource::Builtin,
            path: None,
            provenance: None,
        })
        .collect()
}

fn serialize_pack(pack: &ThemePack) -> Result<String, ThemeCatalogError> {
    let mut text =
        serde_json::to_string_pretty(pack).map_err(|error| ThemeCatalogError::Invalid {
            path: PathBuf::new(),
            reason: format!("serializing the theme failed: {error}"),
        })?;
    text.push('\n');
    Ok(text)
}

/// Writes a theme file through the shared atomic replacement helper.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ThemeCatalogError> {
    super::atomic::write_atomic(path, bytes).map_err(|source| ThemeCatalogError::Io {
        operation: "write",
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::store::{
        builtin_pack, resolve, ThemeRequest, DEFAULT_PACK_ID, FALLBACK_PACK_ID,
    };
    use crate::ui_preferences::CustomTheme;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..64 {
                let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("dsh-theme-files-{}-{sequence}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("unique theme test directory should be created: {error}"),
                }
            }
            panic!("could not allocate an isolated theme test directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!(
                        "theme test directory cleanup failed for {}: {error}",
                        self.0.display()
                    );
                }
            }
        }
    }

    fn pack_document(id: &str, name: &str) -> String {
        let mut pack = builtin_packs()[0].clone();
        pack.id = id.to_owned();
        pack.name = name.to_owned();
        pack.source = "user".to_owned();
        serde_json::to_string_pretty(&pack).unwrap()
    }

    fn write_pack(dir: &Path, file: &str, document: &str) -> PathBuf {
        let path = dir.join(file);
        fs::write(&path, document).unwrap();
        path
    }

    #[test]
    fn discovery_keeps_builtin_order_and_user_files_shadow_same_id() {
        let directory = TestDirectory::new();
        write_pack(
            directory.path(),
            "aaa.json",
            &pack_document("dsh", "Shadowed DSH"),
        );
        write_pack(
            directory.path(),
            "zzz.json",
            &pack_document("forest", "Forest"),
        );

        let catalog = ThemeCatalog::load(directory.path());
        let ids: Vec<&str> = catalog
            .themes()
            .iter()
            .map(|theme| theme.pack.id.as_str())
            .collect();
        assert_eq!(
            ids,
            ["harness", "high-contrast", "solarized", "dsh", "forest"]
        );
        assert_eq!(catalog.find("dsh").unwrap().source, ThemeSource::User);
        assert_eq!(catalog.find("dsh").unwrap().pack.name, "Shadowed DSH");
        assert_eq!(
            catalog.find("harness").unwrap().source,
            ThemeSource::Builtin
        );
        assert!(catalog.errors().is_empty());
    }

    #[test]
    fn invalid_file_is_isolated_from_valid_files_and_builtins() {
        let directory = TestDirectory::new();
        write_pack(directory.path(), "broken.json", "{ not json");
        write_pack(
            directory.path(),
            "forest.json",
            &pack_document("forest", "Forest"),
        );

        let catalog = ThemeCatalog::load(directory.path());
        assert_eq!(catalog.errors().len(), 1);
        assert_eq!(catalog.errors()[0].file_name, "broken.json");
        assert!(!catalog.errors()[0].reason.is_empty());
        assert!(catalog.find("forest").is_some());
        assert!(catalog.find(DEFAULT_PACK_ID).is_some());
    }

    #[test]
    fn a_user_pack_that_fails_contrast_is_rejected_with_its_reason() {
        let directory = TestDirectory::new();
        let mut pack = builtin_packs()[0].clone();
        pack.id = "low-contrast".to_owned();
        pack.name = "Low contrast".to_owned();
        pack.source = "user".to_owned();
        pack.colors.dark.text = super::super::pack::ThemeColor("#0d1721".to_owned());
        pack.colors.dark.text_secondary = super::super::pack::ThemeColor("#0d1721".to_owned());
        pack.colors.dark.accent = super::super::pack::ThemeColor("#0d1721".to_owned());
        pack.colors.dark.accent_text = super::super::pack::ThemeColor("#0d1721".to_owned());
        let document = serde_json::to_string_pretty(&pack).unwrap();
        write_pack(directory.path(), "low-contrast.json", &document);

        let catalog = ThemeCatalog::load(directory.path());
        let custom = CustomTheme::default();
        let resolved = resolve(&ThemeRequest {
            pack_id: "low-contrast",
            custom: &custom,
            catalog: &catalog,
            accent: AccentScheme::Harness,
            dark: true,
        });
        assert_eq!(resolved.status.pack_id, FALLBACK_PACK_ID);
        assert!(
            resolved.status.errors.iter().any(|error| {
                error.contains("theme `low-contrast` was rejected")
                    && error.contains("text on canvas")
            }),
            "{:?}",
            resolved.status.errors
        );
    }

    #[test]
    fn duplicate_copies_every_role_and_accent_family() {
        let directory = TestDirectory::new();
        let mut catalog = ThemeCatalog::load(directory.path());
        let id = catalog.duplicate(DEFAULT_PACK_ID).unwrap();
        assert_eq!(id, "dsh-copy");
        let copy = catalog.find(&id).unwrap();
        let source = builtin_pack(DEFAULT_PACK_ID).unwrap();
        assert_eq!(copy.source, ThemeSource::User);
        assert_eq!(copy.pack.colors, source.colors);
        assert_eq!(copy.pack.accents, source.accents);
        let expected = directory.path().join("dsh-copy.json");
        assert_eq!(copy.path.as_deref(), Some(expected.as_path()));

        let second = catalog.duplicate(DEFAULT_PACK_ID).unwrap();
        assert_eq!(second, "dsh-copy-2");
    }

    #[test]
    fn edit_save_validates_before_replacing_the_file() {
        let directory = TestDirectory::new();
        let mut catalog = ThemeCatalog::load(directory.path());
        let id = catalog.duplicate(DEFAULT_PACK_ID).unwrap();
        let original = catalog.edit_text(&id).unwrap();
        assert!(catalog.save_edit(&id, "{ not json").is_err());
        assert_eq!(catalog.edit_text(&id).unwrap(), original);

        let mut document: serde_json::Value = serde_json::from_str(&original).unwrap();
        document["colors"]["dark"]["canvas"] = serde_json::json!("#010203");
        let edited = serde_json::to_string_pretty(&document).unwrap();
        assert_eq!(catalog.save_edit(&id, &edited).unwrap(), id);
        assert_eq!(
            catalog.find(&id).unwrap().pack.colors.dark.canvas.0,
            "#010203"
        );

        document["id"] = serde_json::json!(DEFAULT_PACK_ID);
        let builtin = serde_json::to_string_pretty(&document).unwrap();
        assert!(matches!(
            catalog.save_edit(&id, &builtin),
            Err(ThemeCatalogError::Builtin { .. })
        ));
    }

    #[test]
    fn import_reports_an_existing_id_and_requires_explicit_replacement() {
        let directory = TestDirectory::new();
        let source_dir = TestDirectory::new();
        let mut catalog = ThemeCatalog::load(directory.path());
        let first = write_pack(
            source_dir.path(),
            "first.json",
            &pack_document("forest", "Forest"),
        );
        let second = write_pack(
            source_dir.path(),
            "second.json",
            &pack_document("forest", "Forest 2"),
        );

        assert_eq!(catalog.import_file(&first, false).unwrap(), "forest");
        assert!(matches!(
            catalog.import_file(&second, false),
            Err(ThemeCatalogError::AlreadyInstalled { .. })
        ));
        assert_eq!(catalog.import_file(&second, true).unwrap(), "forest");
        assert_eq!(catalog.find("forest").unwrap().pack.name, "Forest 2");

        let builtin = write_pack(
            source_dir.path(),
            "dsh.json",
            &pack_document("dsh", "Copy of DSH"),
        );
        assert!(matches!(
            catalog.import_file(&builtin, true),
            Err(ThemeCatalogError::Builtin { .. })
        ));
    }

    #[test]
    fn export_payload_round_trips_the_installed_pack() {
        let directory = TestDirectory::new();
        let catalog = ThemeCatalog::load(directory.path());
        let payload = catalog.export_text(DEFAULT_PACK_ID).unwrap();
        let parsed = ThemePack::parse(&payload).unwrap();
        assert_eq!(parsed.id, DEFAULT_PACK_ID);
        assert_eq!(parsed.colors, builtin_pack(DEFAULT_PACK_ID).unwrap().colors);

        let destination = directory.path().join("exported-dsh.json");
        catalog.export_to(DEFAULT_PACK_ID, &destination).unwrap();
        assert_eq!(fs::read_to_string(destination).unwrap(), payload);
    }

    #[test]
    fn remove_deletes_only_user_files() {
        let directory = TestDirectory::new();
        let mut catalog = ThemeCatalog::load(directory.path());
        let id = catalog.duplicate(DEFAULT_PACK_ID).unwrap();
        let path = catalog.find(&id).unwrap().path.clone().unwrap();
        assert!(path.exists());
        catalog.remove(&id).unwrap();
        assert!(!path.exists());
        assert!(catalog.find(&id).is_none());
        assert!(matches!(
            catalog.remove(DEFAULT_PACK_ID),
            Err(ThemeCatalogError::NotUser { .. })
        ));
    }
}
