//! Theme packs: semantic colour data resolved into one frame's snapshot.
//!
//! A pack names colour roles for both schemes. Metrics, fonts, density, and
//! layout stay in the app, so a pack can restyle the shell but cannot break its
//! geometry. The built-in packs are JSON compiled into the binary; user packs are
//! untrusted files loaded from the client's theme directory, and cached GitHub
//! packs are untrusted documents loaded from the explicit import cache.
//! Resolution never reads the network.

mod atomic;
mod contrast;
mod pack;
pub(crate) mod remote;
mod store;
mod tokens;
mod user;

pub(crate) use pack::ThemeColor;
pub(crate) use store::{
    resolve, ThemeRequest, ThemeStatus, CUSTOM_PACK_ID, DEFAULT_PACK_ID,
};
pub(crate) use tokens::Rgba8;
pub(crate) use user::{ContrastStatus, ThemeCatalog, ThemeCatalogError, ThemeSource};

/// A contrast-passing pack document used by theme tests.
///
/// @param id — pack id.
/// @param name — display name.
/// @returns a complete JSON document for both schemes.
#[cfg(test)]
pub(crate) fn test_pack_document(id: &str, name: &str) -> String {
    let dark = serde_json::json!({
        "canvas": "#000000",
        "chrome": "#000000",
        "surface": "#000000",
        "surfaceDeep": "#000000",
        "surfaceRaised": "#101010",
        "border": "#888888",
        "borderFaint": "#777777",
        "text": "#ffffff",
        "textSecondary": "#ffffff",
        "textTertiary": "#ffffff",
        "accent": "#ffffff",
        "accentText": "#000000",
        "selection": "#333333",
        "success": "#ffffff",
        "danger": "#ffffff",
        "warning": "#ffffff"
    });
    let light = serde_json::json!({
        "canvas": "#ffffff",
        "chrome": "#ffffff",
        "surface": "#ffffff",
        "surfaceDeep": "#ffffff",
        "surfaceRaised": "#eeeeee",
        "border": "#777777",
        "borderFaint": "#888888",
        "text": "#000000",
        "textSecondary": "#000000",
        "textTertiary": "#000000",
        "accent": "#000000",
        "accentText": "#ffffff",
        "selection": "#dddddd",
        "success": "#000000",
        "danger": "#000000",
        "warning": "#000000"
    });
    serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "id": id,
        "name": name,
        "author": "test",
        "license": "MIT",
        "source": "github",
        "colors": { "dark": dark, "light": light }
    }))
    .expect("the test theme document serializes")
}
