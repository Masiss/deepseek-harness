//! Theme packs: semantic colour data resolved into one frame's snapshot.
//!
//! A pack names colour roles for both schemes. Metrics, fonts, density, and
//! layout stay in the app, so a pack can restyle the shell but cannot break its
//! geometry. The built-in packs are JSON compiled into the binary; user packs are
//! untrusted files loaded from the client's theme directory. Resolution never
//! reads the network.

mod contrast;
mod pack;
mod store;
mod tokens;
mod user;

pub(crate) use pack::ThemeColor;
pub(crate) use store::{
    resolve, ThemeRequest, ThemeStatus, CUSTOM_PACK_ID, DEFAULT_PACK_ID,
};
pub(crate) use tokens::Rgba8;
pub(crate) use user::{ContrastStatus, ThemeCatalog, ThemeCatalogError, ThemeSource};
