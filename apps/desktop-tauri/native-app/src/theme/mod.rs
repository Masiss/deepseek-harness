//! Theme packs: semantic colour data resolved into one frame's snapshot.
//!
//! A pack names colour roles for both schemes. Metrics, fonts, density, and
//! layout stay in the app, so a pack can restyle the shell but cannot break its
//! geometry. The built-in packs are JSON compiled into the binary, which keeps
//! resolution free of filesystem and network access.

mod contrast;
mod pack;
mod store;
mod tokens;

pub(crate) use pack::ThemeColor;
pub(crate) use store::{pack_choices, resolve, ThemeRequest, ThemeStatus, DEFAULT_PACK_ID};
pub(crate) use tokens::Rgba8;
