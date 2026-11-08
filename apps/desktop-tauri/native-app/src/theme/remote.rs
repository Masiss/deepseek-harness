//! Remote theme import: source resolution, fetching, validation, cache, and
//! provenance.
//!
//! A GitHub source is either an `https` URL to one JSON document or an
//! `owner/repo[@ref]` reference resolved through
//! `https://raw.githubusercontent.com`. Fetched documents are parsed and must
//! pass the same untrusted schema and contrast checks as a local user file
//! before they reach the cache. Nothing here runs at startup; callers fetch only
//! after an explicit Check, Import, or Check for updates action.

use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::atomic;
use super::pack::ThemePack;
use super::store::{resolve_pack_document, PackTrust, CUSTOM_PACK_ID};
use super::tokens::Scheme;
use crate::ui_preferences::AccentScheme;

/// Largest accepted response body.
pub(crate) const MAX_BODY_BYTES: usize = 64 * 1024;

/// Timeout applied to one fetch, including its body read.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Root used for `owner/repo` sources.
const RAW_GITHUB_ROOT: &str = "https://raw.githubusercontent.com/";

/// Default branch when a repository source names none.
const DEFAULT_REF: &str = "main";

/// Default document path when a repository source names none.
const DEFAULT_PATH: &str = "theme.json";

/// One resolved location for a theme document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceSpec {
    /// Absolute `https` URL to fetch.
    pub(crate) url: String,
    /// Git ref used to build a GitHub URL; `None` for a raw URL.
    pub(crate) ref_name: Option<String>,
}

/// Cache provenance stored beside one downloaded document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ThemeProvenance {
    /// URL the document was fetched from.
    pub(crate) source_url: String,
    /// Git ref used for a GitHub repository source; empty for a raw URL.
    #[serde(rename = "ref")]
    pub(crate) ref_name: String,
    /// HTTP ETag captured from the last changed response.
    pub(crate) etag: Option<String>,
    /// Fetch time as seconds since the Unix epoch.
    pub(crate) fetched_at: u64,
    /// Lowercase SHA-256 of the cached document bytes.
    pub(crate) sha256: String,
}

/// One validated remote document that has not been cached yet.
#[derive(Clone, Debug)]
pub(crate) struct CheckedTheme {
    /// Pack id in the fetched document.
    pub(crate) id: String,
    /// Display name in the fetched document.
    pub(crate) name: String,
    /// URL the document was fetched from.
    pub(crate) source_url: String,
    /// Git ref used for a repository source; empty for a raw URL.
    pub(crate) ref_name: String,
}

/// One validated cached document and its provenance.
#[derive(Clone, Debug)]
pub(crate) struct CachedTheme {
    /// Parsed pack document.
    pub(crate) pack: ThemePack,
    /// Provenance stored in `<id>.meta.json`.
    pub(crate) meta: ThemeProvenance,
    /// Cached JSON document path.
    pub(crate) path: PathBuf,
}

/// One cache file discovery could not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CacheFileError {
    /// File name shown beside the failure.
    pub(crate) file_name: String,
    /// Concrete reason the cache entry was rejected.
    pub(crate) reason: String,
}

/// Cached documents plus cache files discovery rejected.
#[derive(Clone, Debug, Default)]
pub(crate) struct CacheDiscovery {
    /// Valid cached documents in file-name order.
    pub(crate) themes: Vec<CachedTheme>,
    /// Cache files that could not be used.
    pub(crate) errors: Vec<CacheFileError>,
}

/// Result of an explicit update request.
#[derive(Clone, Debug)]
pub(crate) enum RefreshOutcome {
    /// The server answered 304; cache and provenance are untouched.
    Unchanged,
    /// A validated replacement replaced the cached document.
    Updated(ThemeProvenance),
}

/// One HTTP request the fetch boundary receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FetchRequest {
    /// Absolute `https` document URL.
    pub(crate) url: String,
    /// Stored ETag to send as `If-None-Match`.
    pub(crate) etag: Option<String>,
}

/// One HTTP response the fetch boundary returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FetchResponse {
    /// HTTP status code.
    pub(crate) status: u16,
    /// Response ETag, when the server sent one.
    pub(crate) etag: Option<String>,
    /// URL after any redirects.
    pub(crate) final_url: String,
    /// Response body bytes, capped by the caller before parsing.
    pub(crate) body: Vec<u8>,
}

/// HTTP boundary used by remote theme operations.
///
/// Tests implement this trait with fixtures; the shipped implementation uses
/// `reqwest`'s blocking client.
pub(crate) trait ThemeFetcher: Send + Sync {
    /// Fetches one document.
    ///
    /// @param request — URL and optional conditional ETag.
    /// @returns the response, or a transport failure.
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, RemoteError>;
}

/// Blocking `reqwest` fetch implementation.
pub(crate) struct ReqwestFetcher;

impl ThemeFetcher for ReqwestFetcher {
    fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, RemoteError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 8 {
                    attempt.error("too many theme redirects")
                } else if attempt.url().scheme() != "https" {
                    attempt.error("theme redirect must stay on https")
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|error| RemoteError::Http(error.to_string()))?;
        let mut outgoing = client.get(&request.url);
        if let Some(etag) = &request.etag {
            outgoing = outgoing.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        let response = outgoing
            .send()
            .map_err(|error| RemoteError::Http(error.to_string()))?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        response
            .take(MAX_BODY_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| RemoteError::Http(error.to_string()))?;
        if body.len() > MAX_BODY_BYTES {
            return Err(RemoteError::TooLarge {
                limit: MAX_BODY_BYTES,
            });
        }
        Ok(FetchResponse {
            status,
            etag,
            final_url,
            body,
        })
    }
}

/// One rejected remote operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteError {
    /// The source text or resolved URL is not acceptable.
    InvalidSource(String),
    /// The fetch failed before a usable response arrived.
    Http(String),
    /// The server returned a non-success status.
    Status(u16),
    /// The response body exceeded [`MAX_BODY_BYTES`].
    TooLarge { limit: usize },
    /// The document failed schema, id, colour, or contrast validation.
    InvalidDocument(String),
    /// A cache file or operation failed.
    Cache(String),
}

impl fmt::Display for RemoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSource(reason) => write!(formatter, "invalid theme source: {reason}"),
            Self::Http(reason) => write!(formatter, "theme fetch failed: {reason}"),
            Self::Status(status) => {
                write!(formatter, "theme server returned HTTP {status}")
            }
            Self::TooLarge { limit } => {
                write!(formatter, "theme document exceeds the {limit}-byte limit")
            }
            Self::InvalidDocument(reason) => {
                write!(formatter, "theme document is not usable: {reason}")
            }
            Self::Cache(reason) => write!(formatter, "theme cache failed: {reason}"),
        }
    }
}

impl StdError for RemoteError {}

/// Resolves user input into one `https` document URL.
///
/// `source` is either an absolute `https` URL ending in `.json` or a
/// `owner/repo[@ref]` reference. A repository reference defaults to `main` and
/// `theme.json`; an empty `path` uses that default. Raw URLs do not accept a
/// separate ref or path.
///
/// @param source — URL or repository reference, as typed.
/// @param ref_name — optional ref for repository references.
/// @param path — optional document path for repository references.
/// @returns the resolved URL and repository ref, or a named rejection.
pub(crate) fn resolve_source(
    source: &str,
    ref_name: &str,
    path: &str,
) -> Result<SourceSpec, RemoteError> {
    let source = source.trim();
    if source.is_empty() {
        return Err(RemoteError::InvalidSource(
            "enter a URL or owner/repo".to_owned(),
        ));
    }
    let ref_name = ref_name.trim();
    let path = path.trim();
    if source.contains("://") || source.starts_with("http:") || source.starts_with("https:") {
        if !ref_name.is_empty() || !path.is_empty() {
            return Err(RemoteError::InvalidSource(
                "ref and path are only accepted with owner/repo".to_owned(),
            ));
        }
        let url = validate_source_url(source)?;
        if !url.path().to_ascii_lowercase().ends_with(".json") {
            return Err(RemoteError::InvalidSource(
                "the URL must point to a .json document".to_owned(),
            ));
        }
        return Ok(SourceSpec {
            url: url.to_string(),
            ref_name: None,
        });
    }

    let (repo, inline_ref) = match source.split_once('@') {
        Some((repo, inline)) => (repo, inline),
        None => (source, ""),
    };
    if repo.is_empty() || (inline_ref.trim().is_empty() && source.ends_with('@')) {
        return Err(RemoteError::InvalidSource(
            "expected owner/repo[@ref]".to_owned(),
        ));
    }
    let inline_ref = inline_ref.trim();
    if !inline_ref.is_empty() && !ref_name.is_empty() && inline_ref != ref_name {
        return Err(RemoteError::InvalidSource(
            "ref is specified twice with different values".to_owned(),
        ));
    }
    let ref_name = if !ref_name.is_empty() {
        ref_name
    } else if !inline_ref.is_empty() {
        inline_ref
    } else {
        DEFAULT_REF
    };
    let path = if path.is_empty() { DEFAULT_PATH } else { path };

    let Some((owner, repository)) = repo.split_once('/') else {
        return Err(RemoteError::InvalidSource(
            "expected owner/repo[@ref]".to_owned(),
        ));
    };
    if repository.contains('/') {
        return Err(RemoteError::InvalidSource(
            "expected owner/repo[@ref] without an extra path segment".to_owned(),
        ));
    }
    if !valid_segment(owner) || !valid_segment(repository) {
        return Err(RemoteError::InvalidSource(
            "owner and repository must use letters, digits, '.', '_', '-', or '~'"
                .to_owned(),
        ));
    }
    if !valid_segments(ref_name) {
        return Err(RemoteError::InvalidSource(
            "ref contains an unsupported path segment".to_owned(),
        ));
    }
    if !valid_document_path(path) {
        return Err(RemoteError::InvalidSource(
            "path contains an unsupported segment or does not end in .json".to_owned(),
        ));
    }

    let mut url = reqwest::Url::parse(RAW_GITHUB_ROOT)
        .expect("the raw GitHub root is a valid absolute URL");
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| RemoteError::InvalidSource("cannot build the GitHub URL".to_owned()))?;
        segments.pop_if_empty();
        segments.push(owner);
        segments.push(repository);
        for segment in ref_name.split('/') {
            segments.push(segment);
        }
        for segment in path.split('/') {
            segments.push(segment);
        }
    }
    let url = validate_source_url(url.as_str())?;
    if !url.path().to_ascii_lowercase().ends_with(".json") {
        return Err(RemoteError::InvalidSource(
            "the resolved document must end in .json".to_owned(),
        ));
    }
    Ok(SourceSpec {
        url: url.to_string(),
        ref_name: Some(ref_name.to_owned()),
    })
}

/// Fetches and validates one document without writing a cache.
///
/// @param spec — resolved source.
/// @param fetcher — HTTP boundary.
/// @returns the validated pack identity and provenance-ready fields.
pub(crate) fn check_source(
    spec: &SourceSpec,
    fetcher: &dyn ThemeFetcher,
) -> Result<CheckedTheme, RemoteError> {
    let response = fetcher.fetch(&FetchRequest {
        url: spec.url.clone(),
        etag: None,
    })?;
    let pack = validate_response(&response)?;
    Ok(CheckedTheme {
        id: pack.id.clone(),
        name: pack.name.clone(),
        source_url: spec.url.clone(),
        ref_name: spec.ref_name.clone().unwrap_or_default(),
    })
}

/// Fetches, validates, and caches one document.
///
/// @param themes_dir — client theme directory that owns the `cache/` child.
/// @param spec — resolved source.
/// @param fetcher — HTTP boundary.
/// @returns the cached document and its new provenance.
pub(crate) fn import_source(
    themes_dir: &Path,
    spec: &SourceSpec,
    fetcher: &dyn ThemeFetcher,
) -> Result<CachedTheme, RemoteError> {
    let response = fetcher.fetch(&FetchRequest {
        url: spec.url.clone(),
        etag: None,
    })?;
    let pack = validate_response(&response)?;
    write_cache(
        &cache_dir(themes_dir),
        spec,
        &pack,
        &response.body,
        response.etag,
    )
}

/// Explicitly refreshes one cached pack with its stored ETag.
///
/// @param themes_dir — client theme directory that owns the `cache/` child.
/// @param id — cached pack id.
/// @param fetcher — HTTP boundary.
/// @returns `Unchanged` for 304 or the replacement provenance.
pub(crate) fn refresh_cached(
    themes_dir: &Path,
    id: &str,
    fetcher: &dyn ThemeFetcher,
) -> Result<RefreshOutcome, RemoteError> {
    let cache = cache_dir(themes_dir);
    let document_path = cache.join(format!("{id}.json"));
    if !document_path.is_file() {
        return Err(RemoteError::Cache(format!(
            "cached theme `{id}` has no document"
        )));
    }
    let meta_path = cache.join(format!("{id}.meta.json"));
    let meta_text = fs::read_to_string(&meta_path)
        .map_err(|error| RemoteError::Cache(format!("{id}.meta.json: {error}")))?;
    let meta: ThemeProvenance = serde_json::from_str(&meta_text)
        .map_err(|error| RemoteError::Cache(format!("{id}.meta.json: {error}")))?;
    validate_source_url(&meta.source_url)?;
    let response = fetcher.fetch(&FetchRequest {
        url: meta.source_url.clone(),
        etag: meta.etag.clone(),
    })?;
    if response.status == 304 {
        return Ok(RefreshOutcome::Unchanged);
    }
    let pack = validate_response(&response)?;
    if pack.id != id {
        return Err(RemoteError::InvalidDocument(format!(
            "updated document id `{}` does not match cached id `{id}`",
            pack.id
        )));
    }
    let spec = SourceSpec {
        url: meta.source_url.clone(),
        ref_name: if meta.ref_name.is_empty() {
            None
        } else {
            Some(meta.ref_name.clone())
        },
    };
    let cached = write_cache(&cache, &spec, &pack, &response.body, response.etag)?;
    Ok(RefreshOutcome::Updated(cached.meta))
}

/// Discovers validated cached documents under one cache directory.
///
/// A cache entry needs both `<id>.json` and `<id>.meta.json`; the metadata
/// SHA-256 must match the document. Schema, id, colour, and untrusted contrast
/// validation runs again so a tampered or partial cache never renders.
///
/// @param cache — cache directory; a missing directory is empty.
/// @returns valid cached documents and rejected cache files.
pub(crate) fn discover_cache(cache: &Path) -> CacheDiscovery {
    let mut discovery = CacheDiscovery::default();
    let entries = match fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return discovery,
        Err(error) => {
            discovery.errors.push(CacheFileError {
                file_name: cache.display().to_string(),
                reason: format!("cannot read the theme cache: {error}"),
            });
            return discovery;
        }
    };
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
            path.extension().and_then(|extension| extension.to_str()) == Some("json")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".meta.json"))
        })
        .collect();
    files.sort();
    for path in files {
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            discovery.errors.push(CacheFileError {
                file_name,
                reason: "cache file has no UTF-8 id".to_owned(),
            });
            continue;
        };
        let meta_path = cache.join(format!("{id}.meta.json"));
        let meta_text = match fs::read_to_string(&meta_path) {
            Ok(text) => text,
            Err(error) => {
                discovery.errors.push(CacheFileError {
                    file_name,
                    reason: format!("cannot read {}: {error}", meta_path.display()),
                });
                continue;
            }
        };
        let meta: ThemeProvenance = match serde_json::from_str(&meta_text) {
            Ok(meta) => meta,
            Err(error) => {
                discovery.errors.push(CacheFileError {
                    file_name,
                    reason: format!("metadata is invalid: {error}"),
                });
                continue;
            }
        };
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                discovery.errors.push(CacheFileError {
                    file_name,
                    reason: format!("cannot read the document: {error}"),
                });
                continue;
            }
        };
        let actual_sha = sha256_hex(&bytes);
        if meta.sha256 != actual_sha {
            discovery.errors.push(CacheFileError {
                file_name,
                reason: "cached document does not match its metadata SHA-256".to_owned(),
            });
            continue;
        }
        let text = match std::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(_) => {
                discovery.errors.push(CacheFileError {
                    file_name,
                    reason: "cached document is not valid UTF-8".to_owned(),
                });
                continue;
            }
        };
        let pack = match ThemePack::parse(text) {
            Ok(pack) => pack,
            Err(error) => {
                discovery.errors.push(CacheFileError {
                    file_name,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        if pack.id != id {
            discovery.errors.push(CacheFileError {
                file_name,
                reason: format!("document id `{}` does not match its file name", pack.id),
            });
            continue;
        }
        if let Err(error) = validate_cached_pack(&pack) {
            discovery.errors.push(CacheFileError {
                file_name,
                reason: error.to_string(),
            });
            continue;
        }
        discovery.themes.push(CachedTheme {
            pack,
            meta,
            path,
        });
    }
    discovery
}

/// Removes one cached document and its metadata.
///
/// @param cache — cache directory.
/// @param id — cached pack id.
pub(crate) fn remove_cached(cache: &Path, id: &str) -> Result<(), RemoteError> {
    let document = cache.join(format!("{id}.json"));
    let meta = cache.join(format!("{id}.meta.json"));
    if !document.exists() && !meta.exists() {
        return Err(RemoteError::Cache(format!("theme `{id}` is not cached")));
    }
    if document.exists() {
        fs::remove_file(&document)
            .map_err(|error| RemoteError::Cache(format!("{}: {error}", document.display())))?;
    }
    if meta.exists() {
        fs::remove_file(&meta)
            .map_err(|error| RemoteError::Cache(format!("{}: {error}", meta.display())))?;
    }
    Ok(())
}

/// Cache directory that belongs to one client theme directory.
///
/// @param themes_dir — client theme directory.
/// @returns `<themes-dir>/cache`.
pub(crate) fn cache_dir(themes_dir: &Path) -> PathBuf {
    themes_dir.join("cache")
}

fn write_cache(
    cache: &Path,
    spec: &SourceSpec,
    pack: &ThemePack,
    body: &[u8],
    etag: Option<String>,
) -> Result<CachedTheme, RemoteError> {
    validate_source_url(&spec.url)?;
    let document = cache.join(format!("{}.json", pack.id));
    let meta_path = cache.join(format!("{}.meta.json", pack.id));
    let meta = ThemeProvenance {
        source_url: spec.url.clone(),
        ref_name: spec.ref_name.clone().unwrap_or_default(),
        etag,
        fetched_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        sha256: sha256_hex(body),
    };
    let mut meta_bytes = serde_json::to_vec_pretty(&meta)
        .map_err(|error| RemoteError::Cache(format!("metadata: {error}")))?;
    meta_bytes.push(b'\n');
    atomic::write_atomic(&document, body)
        .map_err(|error| RemoteError::Cache(format!("{}: {error}", document.display())))?;
    atomic::write_atomic(&meta_path, &meta_bytes)
        .map_err(|error| RemoteError::Cache(format!("{}: {error}", meta_path.display())))?;
    Ok(CachedTheme {
        pack: pack.clone(),
        meta,
        path: document,
    })
}

fn validate_response(response: &FetchResponse) -> Result<ThemePack, RemoteError> {
    if response.status == 304 {
        return Err(RemoteError::Http(
            "unexpected HTTP 304 for a non-conditional fetch".to_owned(),
        ));
    }
    if !(200..300).contains(&response.status) {
        return Err(RemoteError::Status(response.status));
    }
    validate_final_url(&response.final_url)?;
    if response.body.len() > MAX_BODY_BYTES {
        return Err(RemoteError::TooLarge {
            limit: MAX_BODY_BYTES,
        });
    }
    let text = std::str::from_utf8(&response.body)
        .map_err(|_| RemoteError::InvalidDocument("the document is not valid UTF-8".to_owned()))?;
    let pack = ThemePack::parse(text)
        .map_err(|error| RemoteError::InvalidDocument(error.to_string()))?;
    validate_cached_pack(&pack)
        .map_err(|error| RemoteError::InvalidDocument(error.to_string()))?;
    Ok(pack)
}

fn validate_cached_pack(pack: &ThemePack) -> Result<(), RemoteError> {
    if pack.id == CUSTOM_PACK_ID {
        return Err(RemoteError::InvalidDocument(format!(
            "theme id `{CUSTOM_PACK_ID}` is reserved for the legacy custom palette"
        )));
    }
    for scheme in [Scheme::Light, Scheme::Dark] {
        if let Err(failures) =
            resolve_pack_document(pack, PackTrust::Untrusted, AccentScheme::Harness, scheme)
        {
            return Err(RemoteError::InvalidDocument(format!(
                "theme `{}` was rejected: {}",
                pack.id,
                failures.join("; ")
            )));
        }
    }
    Ok(())
}

fn validate_source_url(value: &str) -> Result<reqwest::Url, RemoteError> {
    let url = reqwest::Url::parse(value).map_err(|error| {
        RemoteError::InvalidSource(format!("`{value}` is not a valid URL: {error}"))
    })?;
    if url.scheme() != "https" {
        return Err(RemoteError::InvalidSource(
            "theme sources must use https".to_owned(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RemoteError::InvalidSource(
            "theme sources must not embed credentials".to_owned(),
        ));
    }
    Ok(url)
}

fn validate_final_url(value: &str) -> Result<(), RemoteError> {
    let url = validate_source_url(value)?;
    if url.host_str().is_none() {
        return Err(RemoteError::InvalidSource(
            "the fetched document has no host".to_owned(),
        ));
    }
    Ok(())
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | '~')
        })
}

fn valid_segments(value: &str) -> bool {
    !value.is_empty() && value.split('/').all(valid_segment)
}

fn valid_document_path(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".json") && valid_segments(path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::store::{resolve, ThemeRequest};
    use crate::theme::user::{ThemeCatalog, ThemeSource};
    use crate::ui_preferences::CustomTheme;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..64 {
                let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "dsh-remote-theme-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("unique remote theme directory should be created: {error}"),
                }
            }
            panic!("could not allocate an isolated remote theme directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if let Err(error) = fs::remove_dir_all(&self.0) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "remote theme test directory cleanup failed for {}: {error}",
                        self.0.display()
                    );
                }
            }
        }
    }

    struct FixtureFetcher {
        response: Result<FetchResponse, RemoteError>,
        requests: Mutex<Vec<FetchRequest>>,
    }

    impl FixtureFetcher {
        fn response(response: FetchResponse) -> Self {
            Self {
                response: Ok(response),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn failure(error: RemoteError) -> Self {
            Self {
                response: Err(error),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<FetchRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl ThemeFetcher for FixtureFetcher {
        fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, RemoteError> {
            self.requests.lock().unwrap().push(request.clone());
            self.response.clone()
        }
    }

    fn pack_document(id: &str, name: &str) -> String {
        crate::theme::test_pack_document(id, name)
    }

    fn response(body: String, etag: &str) -> FetchResponse {
        FetchResponse {
            status: 200,
            etag: Some(etag.to_owned()),
            final_url: "https://raw.example/theme.json".to_owned(),
            body: body.into_bytes(),
        }
    }

    fn github_spec() -> SourceSpec {
        resolve_source("owner/repo@dev", "", "themes/forest.json").unwrap()
    }

    #[test]
    fn resolves_repository_and_raw_sources() {
        let default = resolve_source("owner/repo", "", "").unwrap();
        assert_eq!(
            default.url,
            "https://raw.githubusercontent.com/owner/repo/main/theme.json"
        );
        assert_eq!(default.ref_name.as_deref(), Some("main"));

        let explicit = resolve_source("owner/repo@dev", "", "themes/forest.json").unwrap();
        assert_eq!(
            explicit.url,
            "https://raw.githubusercontent.com/owner/repo/dev/themes/forest.json"
        );
        assert_eq!(explicit.ref_name.as_deref(), Some("dev"));

        let raw = resolve_source("https://example.com/themes/forest.json", "", "").unwrap();
        assert_eq!(raw.url, "https://example.com/themes/forest.json");
        assert!(raw.ref_name.is_none());
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_sources() {
        for (source, ref_name, path) in [
            ("http://example.com/theme.json", "", ""),
            ("https://user:pass@example.com/theme.json", "", ""),
            ("https://example.com/theme.txt", "", ""),
            ("owner/repo/extra", "", ""),
            ("owner/repo@", "", ""),
            ("owner/repo@main", "dev", ""),
            ("https://example.com/theme.json", "main", ""),
            ("owner/repo", "", "../theme.json"),
            ("owner/repo", "bad ref", ""),
        ] {
            assert!(
                resolve_source(source, ref_name, path).is_err(),
                "{source} {ref_name} {path}"
            );
        }
    }

    #[test]
    fn caps_the_body_before_parsing() {
        let directory = TestDirectory::new();
        let body = vec![b' '; MAX_BODY_BYTES + 1];
        let fetcher = FixtureFetcher::response(FetchResponse {
            status: 200,
            etag: None,
            final_url: "https://raw.example/theme.json".to_owned(),
            body,
        });
        let error = import_source(directory.path(), &github_spec(), &fetcher).unwrap_err();
        assert!(matches!(error, RemoteError::TooLarge { .. }), "{error}");
        assert!(!cache_dir(directory.path()).exists());
    }

    #[test]
    fn rejects_invalid_json_before_caching() {
        let directory = TestDirectory::new();
        let fetcher = FixtureFetcher::response(response("{ not json".to_owned(), "\"v1\""));
        let error = import_source(directory.path(), &github_spec(), &fetcher).unwrap_err();
        assert!(
            matches!(error, RemoteError::InvalidDocument(_)),
            "{error}"
        );
        assert!(!cache_dir(directory.path()).exists());
    }

    #[test]
    fn rejects_a_redirect_that_leaves_https() {
        let directory = TestDirectory::new();
        let fetcher = FixtureFetcher::response(FetchResponse {
            status: 200,
            etag: None,
            final_url: "http://raw.example/theme.json".to_owned(),
            body: pack_document("forest", "Forest").into_bytes(),
        });
        let error = import_source(directory.path(), &github_spec(), &fetcher).unwrap_err();
        assert!(matches!(error, RemoteError::InvalidSource(_)), "{error}");
    }

    #[test]
    fn rejects_a_pack_that_fails_the_contrast_gate() {
        let directory = TestDirectory::new();
        let mut document: serde_json::Value =
            serde_json::from_str(&pack_document("low-contrast", "Low contrast")).unwrap();
        document["colors"]["dark"]["text"] = serde_json::json!("#0d1721");
        document["colors"]["dark"]["textSecondary"] = serde_json::json!("#0d1721");
        document["colors"]["dark"]["accent"] = serde_json::json!("#0d1721");
        document["colors"]["dark"]["accentText"] = serde_json::json!("#0d1721");
        let body = serde_json::to_string(&document).unwrap();
        let fetcher = FixtureFetcher::response(response(body, "\"v1\""));
        let error = import_source(directory.path(), &github_spec(), &fetcher).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("text on canvas"), "{message}");
        assert!(!cache_dir(directory.path()).exists());
    }

    #[test]
    fn check_validates_without_writing_a_cache() {
        let directory = TestDirectory::new();
        let body = pack_document("forest", "Forest");
        let fetcher = FixtureFetcher::response(response(body, "\"v1\""));
        let checked = check_source(&github_spec(), &fetcher).unwrap();
        assert_eq!(checked.id, "forest");
        assert_eq!(checked.name, "Forest");
        assert!(!cache_dir(directory.path()).exists());
    }

    #[test]
    fn import_writes_provenance_and_ranks_above_a_builtin() {
        let directory = TestDirectory::new();
        let body = pack_document("dsh", "Remote DSH");
        let fetcher = FixtureFetcher::response(response(body.clone(), "\"v1\""));
        let cached = import_source(directory.path(), &github_spec(), &fetcher).unwrap();
        assert_eq!(cached.pack.id, "dsh");
        assert_eq!(cached.meta.source_url, github_spec().url);
        assert_eq!(cached.meta.ref_name, "dev");
        assert_eq!(cached.meta.etag.as_deref(), Some("\"v1\""));
        assert_eq!(cached.meta.sha256, sha256_hex(body.as_bytes()));
        assert!(cached.meta.fetched_at > 0);
        assert!(cached.path.is_file());
        assert!(cache_dir(directory.path())
            .join("dsh.meta.json")
            .is_file());

        let catalog = ThemeCatalog::load(directory.path());
        let installed = catalog.find("dsh").unwrap();
        assert_eq!(installed.source, ThemeSource::Github);
        assert_eq!(installed.pack.name, "Remote DSH");
        assert_eq!(installed.provenance.as_ref(), Some(&cached.meta));
    }

    #[test]
    fn a_local_user_file_shadows_the_cached_pack() {
        let directory = TestDirectory::new();
        let body = pack_document("forest", "Remote forest");
        let fetcher = FixtureFetcher::response(response(body, "\"v1\""));
        import_source(directory.path(), &github_spec(), &fetcher).unwrap();
        assert_eq!(
            ThemeCatalog::load(directory.path())
                .find("forest")
                .unwrap()
                .source,
            ThemeSource::Github
        );

        fs::write(
            directory.path().join("forest.json"),
            pack_document("forest", "Local forest"),
        )
        .unwrap();
        let catalog = ThemeCatalog::load(directory.path());
        let installed = catalog.find("forest").unwrap();
        assert_eq!(installed.source, ThemeSource::User);
        assert_eq!(installed.pack.name, "Local forest");
    }

    #[test]
    fn conditional_refresh_treats_304_as_unchanged() {
        let directory = TestDirectory::new();
        let body = pack_document("forest", "Remote forest");
        let fetcher = FixtureFetcher::response(response(body, "\"v1\""));
        let cached = import_source(directory.path(), &github_spec(), &fetcher).unwrap();
        let document = fs::read(&cached.path).unwrap();
        let meta = fs::read(cache_dir(directory.path()).join("forest.meta.json")).unwrap();

        let update = FixtureFetcher::response(FetchResponse {
            status: 304,
            etag: Some("\"v1\"".to_owned()),
            final_url: github_spec().url,
            body: Vec::new(),
        });
        assert!(matches!(
            refresh_cached(directory.path(), "forest", &update).unwrap(),
            RefreshOutcome::Unchanged
        ));
        assert_eq!(fs::read(&cached.path).unwrap(), document);
        assert_eq!(
            fs::read(cache_dir(directory.path()).join("forest.meta.json")).unwrap(),
            meta
        );
        assert_eq!(
            update.requests()[0].etag.as_deref(),
            Some("\"v1\""),
            "the stored ETag is sent with the update request"
        );
    }

    #[test]
    fn a_validated_refresh_replaces_the_cached_document() {
        let directory = TestDirectory::new();
        let fetcher = FixtureFetcher::response(response(
            pack_document("forest", "Forest one"),
            "\"v1\"",
        ));
        let cached = import_source(directory.path(), &github_spec(), &fetcher).unwrap();
        let update = FixtureFetcher::response(response(
            pack_document("forest", "Forest two"),
            "\"v2\"",
        ));
        let meta = match refresh_cached(directory.path(), "forest", &update).unwrap() {
            RefreshOutcome::Updated(meta) => meta,
            RefreshOutcome::Unchanged => panic!("a new document must replace the cache"),
        };
        assert_eq!(meta.etag.as_deref(), Some("\"v2\""));
        let catalog = ThemeCatalog::load(directory.path());
        assert_eq!(catalog.find("forest").unwrap().pack.name, "Forest two");
        assert_eq!(
            catalog.find("forest").unwrap().provenance.as_ref(),
            Some(&meta)
        );
        assert_eq!(
            fs::read(&cached.path).unwrap(),
            fs::read(cache_dir(directory.path()).join("forest.json")).unwrap()
        );
    }

    #[test]
    fn failed_refresh_keeps_the_cached_copy_usable_offline() {
        let directory = TestDirectory::new();
        let body = pack_document("forest", "Remote forest");
        let fetcher = FixtureFetcher::response(response(body, "\"v1\""));
        let cached = import_source(directory.path(), &github_spec(), &fetcher).unwrap();
        let document = fs::read(&cached.path).unwrap();
        let meta = fs::read(cache_dir(directory.path()).join("forest.meta.json")).unwrap();

        let invalid = FixtureFetcher::response(response("{ not json".to_owned(), "\"v2\""));
        assert!(refresh_cached(directory.path(), "forest", &invalid).is_err());
        let failed = FixtureFetcher::failure(RemoteError::Http("network down".to_owned()));
        assert!(refresh_cached(directory.path(), "forest", &failed).is_err());
        assert_eq!(fs::read(&cached.path).unwrap(), document);
        assert_eq!(
            fs::read(cache_dir(directory.path()).join("forest.meta.json")).unwrap(),
            meta
        );

        let catalog = ThemeCatalog::load(directory.path());
        let custom = CustomTheme::default();
        let resolved = resolve(&ThemeRequest {
            pack_id: "forest",
            custom: &custom,
            catalog: &catalog,
            accent: AccentScheme::Harness,
            dark: true,
        });
        assert_eq!(resolved.status.pack_id, "forest");
        assert!(resolved.status.errors.is_empty());
    }

    #[test]
    fn a_tampered_cache_document_is_reported_and_not_loaded() {
        let directory = TestDirectory::new();
        let body = pack_document("forest", "Remote forest");
        let fetcher = FixtureFetcher::response(response(body, "\"v1\""));
        let cached = import_source(directory.path(), &github_spec(), &fetcher).unwrap();
        fs::write(&cached.path, b"{\"schemaVersion\":1}").unwrap();

        let catalog = ThemeCatalog::load(directory.path());
        assert!(catalog.find("forest").is_none());
        assert!(catalog
            .errors()
            .iter()
            .any(|error| error.file_name == "forest.json"
                && error.reason.contains("SHA-256")));
    }
}
