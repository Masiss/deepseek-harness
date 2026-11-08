//! Shared command identifiers and routing for the native desktop shell.
//!
//! One command id is consumed by the menu, the keyboard dispatcher, and the
//! command palette. Keeping labels, groupings, and default chords here means a
//! shortcut cannot be registered twice under different names, and the palette,
//! menu, and bindings always describe the same set.

/// Stable command identifiers shared by menu, shortcuts, and palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CommandId {
    NewSession,
    ToggleExplorer,
    ToggleChanges,
    ShowChangesDiff,
    ToggleTerminal,
    ToggleSettings,
    TogglePlugins,
    ToggleSkills,
    ToggleMarketplace,
    ToggleGoals,
    OpenFile,
    About,
    ShowPalette,
    Dismiss,
    Quit,
}

impl CommandId {
    /// Every command the shell understands, in palette order.
    pub const ALL: [CommandId; 15] = [
        Self::ShowPalette,
        Self::NewSession,
        Self::ToggleExplorer,
        Self::ToggleChanges,
        Self::ShowChangesDiff,
        Self::ToggleTerminal,
        Self::OpenFile,
        Self::ToggleSettings,
        Self::TogglePlugins,
        Self::ToggleSkills,
        Self::ToggleMarketplace,
        Self::ToggleGoals,
        Self::About,
        Self::Dismiss,
        Self::Quit,
    ];

    /// Commands a user can run from the menu or the palette.
    pub const INVOKABLE: [CommandId; 13] = [
        Self::NewSession,
        Self::ToggleExplorer,
        Self::ToggleChanges,
        Self::ShowChangesDiff,
        Self::ToggleTerminal,
        Self::OpenFile,
        Self::ToggleSettings,
        Self::TogglePlugins,
        Self::ToggleSkills,
        Self::ToggleMarketplace,
        Self::ToggleGoals,
        Self::About,
        Self::Quit,
    ];

    /// Hamburger entries in menu order.
    ///
    /// The Terminal belongs to the bottom bar, so the menu omits it even though
    /// the palette still offers it.
    pub const MENU: [CommandId; 13] = [
        Self::ShowPalette,
        Self::ToggleExplorer,
        Self::NewSession,
        Self::ToggleChanges,
        Self::ShowChangesDiff,
        Self::OpenFile,
        Self::ToggleGoals,
        Self::ToggleSettings,
        Self::TogglePlugins,
        Self::ToggleSkills,
        Self::ToggleMarketplace,
        Self::About,
        Self::Quit,
    ];

    /// Human-readable label used by menus, the palette, and shortcut listings.
    pub fn label(self) -> &'static str {
        match self {
            Self::NewSession => "New session",
            Self::ToggleExplorer => "Toggle Explorer",
            Self::ToggleChanges => "Toggle Changes",
            Self::ShowChangesDiff => "Show changes (diff)",
            Self::ToggleTerminal => "Toggle Terminal",
            Self::ToggleSettings => "Preferences…",
            Self::TogglePlugins => "Extensions",
            Self::ToggleSkills => "Skills",
            Self::ToggleMarketplace => "Marketplace",
            Self::ToggleGoals => "Goals",
            Self::OpenFile => "Open file…",
            Self::About => "About DSH",
            Self::ShowPalette => "Quick open",
            Self::Dismiss => "Dismiss overlay",
            Self::Quit => "Quit",
        }
    }

    /// Menu and palette grouping, matching the shell's information architecture.
    pub fn group(self) -> &'static str {
        match self {
            Self::NewSession
            | Self::ToggleExplorer
            | Self::ToggleChanges
            | Self::ShowChangesDiff
            | Self::OpenFile
            | Self::ToggleGoals
            | Self::ShowPalette => "Workspace",
            Self::ToggleTerminal => "View",
            Self::ToggleSettings
            | Self::TogglePlugins
            | Self::ToggleSkills
            | Self::ToggleMarketplace => "Customize",
            Self::About => "Help",
            Self::Dismiss | Self::Quit => "Application",
        }
    }

    /// Default chord in `Ctrl+Shift+S` form, or `None` when the command has none.
    ///
    /// Shortcuts listed in [`crate::ui_preferences::Keybindings`] are overridable;
    /// the rest are fixed until the keymap editor covers them.
    pub fn default_chord(self) -> Option<&'static str> {
        match self {
            Self::NewSession => Some("Ctrl+N"),
            Self::ToggleExplorer => Some("Ctrl+Shift+E"),
            Self::ToggleChanges => Some("Ctrl+Shift+C"),
            // Goals keeps Ctrl+Shift+G; the diff chord is the free one that
            // leaves that shipped, user-overridable shortcut untouched.
            Self::ShowChangesDiff => Some("Ctrl+Shift+D"),
            Self::ToggleTerminal => Some("Ctrl+Shift+T"),
            Self::ToggleSettings => Some("Ctrl+Comma"),
            Self::TogglePlugins => Some("Ctrl+Shift+X"),
            Self::ToggleSkills => Some("Ctrl+Shift+K"),
            Self::ToggleMarketplace => Some("Ctrl+Shift+M"),
            Self::ToggleGoals => Some("Ctrl+Shift+G"),
            Self::OpenFile => Some("Ctrl+O"),
            Self::ShowPalette => Some("Ctrl+P"),
            Self::Dismiss => Some("Escape"),
            Self::About | Self::Quit => None,
        }
    }

    /// Commands whose label or group contains the query, in palette order.
    pub fn matching(query: &str) -> Vec<CommandId> {
        let query = query.trim().to_ascii_lowercase();
        Self::INVOKABLE
            .into_iter()
            .filter(|command| {
                query.is_empty()
                    || command.label().to_ascii_lowercase().contains(&query)
                    || command.group().to_ascii_lowercase().contains(&query)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::CommandId;

    #[test]
    fn every_command_has_a_unique_label_and_chord() {
        let labels: BTreeSet<&str> = CommandId::ALL.iter().map(|command| command.label()).collect();
        assert_eq!(labels.len(), CommandId::ALL.len());

        let chords: Vec<&str> = CommandId::ALL
            .iter()
            .filter_map(|command| command.default_chord())
            .collect();
        let unique: BTreeSet<&str> = chords.iter().copied().collect();
        assert_eq!(unique.len(), chords.len(), "default chords must not collide");
    }

    #[test]
    fn palette_matches_labels_groups_and_every_command() {
        assert_eq!(CommandId::matching("").len(), CommandId::INVOKABLE.len());
        // The palette cannot list itself, but Quick open still has a menu entry.
        assert!(CommandId::matching("quick open").is_empty());
        assert!(CommandId::MENU.contains(&CommandId::ShowPalette));
        assert_eq!(
            CommandId::matching("terminal"),
            vec![CommandId::ToggleTerminal]
        );
        assert!(CommandId::matching("customize").contains(&CommandId::ToggleMarketplace));
        assert!(CommandId::matching("zzz").is_empty());
    }

    #[test]
    fn menu_covers_every_invokable_command_except_the_terminal() {
        for command in CommandId::INVOKABLE {
            if command == CommandId::ToggleTerminal {
                assert!(!CommandId::MENU.contains(&command));
                continue;
            }
            assert!(CommandId::MENU.contains(&command), "{command:?} missing from menu");
        }
        assert!(!CommandId::MENU.contains(&CommandId::ToggleTerminal));
        assert!(!CommandId::MENU.contains(&CommandId::Dismiss));
    }

    #[test]
    fn menu_groups_are_contiguous() {
        let mut seen = Vec::new();
        for command in CommandId::MENU {
            let group = command.group();
            if seen.last() != Some(&group) {
                assert!(
                    !seen.contains(&group),
                    "menu group {group} appears in two sections"
                );
                seen.push(group);
            }
        }
        assert_eq!(seen, vec!["Workspace", "Customize", "Help", "Application"]);
    }

    #[test]
    fn the_changes_and_goals_chords_are_the_documented_ones() {
        // `USER_GUIDE.md` promises these three chords for three distinct jobs.
        for (command, chord) in [
            (CommandId::ToggleChanges, "Ctrl+Shift+C"),
            (CommandId::ShowChangesDiff, "Ctrl+Shift+D"),
            (CommandId::ToggleGoals, "Ctrl+Shift+G"),
        ] {
            assert_eq!(
                command.default_chord(),
                Some(chord),
                "{command:?} keeps its documented chord"
            );
        }
    }

    #[test]
    fn the_user_guide_documents_the_changes_shortcuts() {
        let guide = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/USER_GUIDE.md"
        ))
        .expect("USER_GUIDE.md is readable");
        for claim in [
            "**Ctrl+Shift+D**",
            "**Ctrl+Shift+C**",
            "**Alt+↑/↓**",
            "**Ctrl+Enter**",
            "**Commit message**",
        ] {
            assert!(guide.contains(claim), "USER_GUIDE.md documents {claim}");
        }
    }

    #[test]
    fn palette_orders_matches_like_the_registry() {
        let matches = CommandId::matching("e");
        let expected: Vec<CommandId> = CommandId::INVOKABLE
            .into_iter()
            .filter(|command| matches.contains(command))
            .collect();
        assert_eq!(matches, expected);
    }
}
