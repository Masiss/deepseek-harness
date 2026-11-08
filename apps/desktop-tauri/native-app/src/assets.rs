//! Outline icon assets embedded in the binary.
//!
//! Every shell icon comes from one drawn family: an SVG on a 24px viewBox whose
//! strokes are `currentColor`, so a `gpui::Svg` element paints the icon in the
//! colour its container passes. The files live in `src/assets/icons` and are
//! embedded with `include_bytes!`, so the shell needs no filesystem or network
//! access to draw them.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// Directory every embedded icon sits under, as an asset path prefix.
const ICON_DIRECTORY: &str = "icons";

/// Declares the icon set once: the enum, its asset paths, and its bytes.
///
/// The generated `match` arms are exhaustive, so a variant without an embedded
/// file is a compile error rather than an icon that silently draws nothing.
macro_rules! icon_set {
    ($($variant:ident => $name:literal,)+) => {
        /// One icon of the shell's outline family.
        ///
        /// A variant's file stem is the name `widgets::icon` exposes as the
        /// `icon-<name>` test selector; its geometry is the matching file under
        /// `src/assets/icons`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum Icon {
            $(
                #[doc = concat!("The `", $name, "` icon.")]
                $variant,
            )+
        }

        impl Icon {
            /// Every icon the embedded set defines.
            pub(crate) const ALL: &'static [Icon] = &[$(Icon::$variant),+];

            /// Asset path of the icon's SVG.
            pub(crate) fn path(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!("icons/", $name, ".svg"),)+
                }
            }

            /// File stem of the icon, without directory or extension.
            pub(crate) fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            /// The SVG document embedded in the binary.
            fn bytes(self) -> &'static [u8] {
                match self {
                    $(Self::$variant => include_bytes!(concat!("assets/icons/", $name, ".svg")),)+
                }
            }
        }
    };
}

icon_set! {
    Menu => "menu",
    Search => "search",
    PanelRight => "panel-right",
    Terminal => "terminal",
    Warning => "warning",
    Ellipsis => "ellipsis",
    Branch => "branch",
    Sync => "sync",
    Plus => "plus",
    Minus => "minus",
    X => "x",
    ChevronDown => "chevron-down",
    ChevronUp => "chevron-up",
    ChevronRight => "chevron-right",
    File => "file",
    Folder => "folder",
    FolderOpen => "folder-open",
    Git => "git",
    Check => "check",
    Clock => "clock",
    Calendar => "calendar",
    Send => "send",
    Stop => "stop",
    Paperclip => "paperclip",
    AtSign => "at-sign",
    Settings => "settings",
    Sun => "sun",
    Moon => "moon",
    Keyboard => "keyboard",
    Puzzle => "puzzle",
    Sparkles => "sparkles",
    Store => "store",
    Info => "info",
    Sliders => "sliders",
}

impl Icon {
    /// The icon at an asset path, or `None` when the path is not in the set.
    ///
    /// @param path — asset path, as passed to [`Icon::path`].
    /// @returns the matching icon, if the set defines it.
    pub(crate) fn from_path(path: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|icon| icon.path() == path)
    }
}

/// The icon set, exposed to gpui as the application's asset source.
pub(crate) struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::from_path(path).map(|icon| Cow::Borrowed(icon.bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path != ICON_DIRECTORY {
            return Ok(Vec::new());
        }
        Ok(Icon::ALL
            .iter()
            .map(|icon| SharedString::from(icon.path()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_resolves_through_the_asset_source() {
        for icon in Icon::ALL {
            let bytes = IconAssets
                .load(icon.path())
                .expect("asset source returns no error")
                .unwrap_or_else(|| panic!("{} is not embedded", icon.path()));
            let document = std::str::from_utf8(&bytes).expect("icon is UTF-8");
            assert!(
                document.contains(r#"viewBox="0 0 24 24""#),
                "{} is not on the 24px viewBox",
                icon.name()
            );
            assert!(
                document.contains(r#"fill="none""#),
                "{} is not an outline icon",
                icon.name()
            );
            assert!(
                document.contains(r#"stroke="currentColor""#),
                "{} does not take the text colour",
                icon.name()
            );
            assert!(
                document.contains(r#"stroke-width="1.8""#),
                "{} is not on the shared stroke weight",
                icon.name()
            );
        }
    }

    #[test]
    fn an_unknown_asset_path_loads_nothing() {
        assert!(IconAssets
            .load("icons/missing.svg")
            .expect("asset source returns no error")
            .is_none());
        assert!(IconAssets
            .load(ICON_DIRECTORY)
            .expect("asset source returns no error")
            .is_none());
        assert_eq!(
            IconAssets
                .list(ICON_DIRECTORY)
                .expect("asset source returns no error")
                .len(),
            Icon::ALL.len()
        );
        assert!(IconAssets
            .list("images")
            .expect("asset source returns no error")
            .is_empty());
    }

    #[test]
    fn icon_names_are_unique() {
        let mut names: Vec<_> = Icon::ALL.iter().map(|icon| icon.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Icon::ALL.len());
    }
}
