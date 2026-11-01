//! Shared command identifiers and routing for the native desktop shell.
//!
//! One command id is consumed by the menu, keyboard dispatcher, and command
//! palette. Keeping the identifiers here prevents the same shortcut from being
//! registered twice under different names.

/// Stable command identifiers shared by menu, shortcuts, and palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum CommandId {
    ToggleSettings,
    ToggleMarketplace,
    ToggleGoals,
    NewSession,
    ToggleExplorer,
    ToggleChanges,
    ToggleTerminal,
    TogglePlugins,
    ToggleSkills,
}

impl CommandId {
    /// Commands currently dispatched through the shared keyboard wiring.
    #[allow(dead_code)]
    pub const ACTIVE: [CommandId; 5] = [
        Self::ToggleSettings,
        Self::ToggleMarketplace,
        Self::ToggleGoals,
        Self::NewSession,
        Self::ToggleTerminal,
    ];

    /// Human-readable label used by menus and the command palette.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleSettings => "Preferences…",
            Self::ToggleMarketplace => "Marketplace",
            Self::ToggleGoals => "Goals",
            Self::NewSession => "New session",
            Self::ToggleExplorer => "Toggle Explorer",
            Self::ToggleChanges => "Toggle Changes",
            Self::ToggleTerminal => "Toggle Terminal",
            Self::TogglePlugins => "Extensions",
            Self::ToggleSkills => "Skills",
        }
    }
}