//! Full-pane surfaces that replace the conversation: Preferences, Plugins,
//! Skills, Marketplace, Goals, and the Document preview.

use gpui::{div, prelude::*, StatefulInteractiveElement, px, AnyElement, Context};

use crate::layout;
use crate::transcript::RowKind;
use crate::ui_preferences::{AccentScheme, KeymapPreset, ThemeMode, UiDensity};
use crate::shell::app::{ActiveView, DesktopApp, GoalAction};
use crate::shell::theme::Theme;
use super::widgets;

/// Returns the active page surface, or `None` while the conversation is shown.
pub(crate) fn active_page(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> Option<AnyElement> {
    let page = if app.show_settings {
        settings(app, theme, cx)
    } else if app.show_goals {
        goals(app, theme, cx)
    } else if app.show_plugins {
        plugins(app, theme, cx)
    } else if app.show_market {
        marketplace(app, theme, cx)
    } else if app.show_skills {
        skills(app, theme, cx)
    } else if app.show_document {
        document(app, theme, cx)
    } else {
        return None;
    };
    Some(
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.))
            .child(page_header(app, theme, cx))
            .child(page)
            .into_any_element(),
    )
}

fn page_header(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let (title, subtitle) = match app.active_view {
        ActiveView::Settings => (
            "Preferences",
            if theme.dark { "Dark interface preferences" } else { "Light interface preferences" },
        ),
        ActiveView::Plugins => (
            "Plugins",
            "Extensions configured by the active profile and managed through the Host.",
        ),
        ActiveView::Skills => ("Skills", "Instructions available to this Session."),
        ActiveView::Marketplace => (
            "Marketplace",
            "Search configured indexes and install profile extensions.",
        ),
        ActiveView::Goals => (
            "Session goal",
            "Keep the task, completion state, and recent plan activity together.",
        ),
        ActiveView::Document => (
            "Document preview",
            "Files scoped to this Session’s workspace; text documents can be edited and saved back to the Host.",
        ),
        ActiveView::Sessions | ActiveView::Explorer => ("Workspace", ""),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(theme.sp(16.))
        .py(theme.sp(10.))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.surface)
        .child(
            div()
                .flex()
                .flex_col()
                .min_w(px(0.))
                .child(
                    div()
                        .text_size(theme.fs(15.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.muted)
                        .child(subtitle),
                ),
        )
        .child(
            widgets::Button::new(theme, "Close")
                .build()
                .id("page-close")
                .on_click(cx.listener(|app, _, _, cx| {
                    app.switch_view(ActiveView::Sessions);
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn body(theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id("page-body")
        .flex()
        .flex_col()
        .gap(theme.sp(8.))
        .flex_1()
        .min_h(px(0.))
        .w_full()
        .px(theme.sp(16.))
        .py(theme.sp(12.))
        .overflow_y_scroll()
}

fn section(theme: &Theme, title: &str) -> AnyElement {
    div()
        .pt(theme.sp(4.))
        .text_size(theme.fs(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(title.to_owned())
        .into_any_element()
}

fn line(theme: &Theme, text: impl Into<String>) -> AnyElement {
    div()
        .text_size(theme.fs(11.))
        .text_color(theme.muted)
        .child(text.into())
        .into_any_element()
}

fn settings(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let dark = app.ui_preferences.theme == ThemeMode::Dark;
    let compact = app.ui_preferences.density == UiDensity::Compact;
    let scale = app.ui_preferences.ui_scale;
    let explorer = app.show_explorer;
    let accent = app.ui_preferences.accent_scheme;
    let restore_last_session = app.ui_preferences.restore_last_session;
    let message = app.preferences_message.clone();
    let update_status = app.update_status.clone();
    let checking = app.update_open_receiver.is_some();
    let connected = app.transport.is_some();
    let namespaces = app
        .settings_description
        .as_ref()
        .map(|describe| {
            describe
                .namespaces
                .iter()
                .map(|namespace| (namespace.ns.clone(), namespace.applies.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let settings_error = app.settings_error.clone();
    let settings_loading = app.settings_loading;

    let mut content = body(theme)
        .child(section(theme, "Appearance"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Theme"))
                .child(
                    widgets::Button::new(theme, "Light")
                        .enabled(dark)
                        .build()
                        .id("pref-light")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.theme = ThemeMode::Light;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Dark")
                        .enabled(!dark)
                        .build()
                        .id("pref-dark")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.theme = ThemeMode::Dark;
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Accent"))
                .child(
                    widgets::Button::new(theme, "Harness")
                        .enabled(accent != AccentScheme::Harness)
                        .build()
                        .id("pref-accent-harness")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.accent_scheme = AccentScheme::Harness;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Blue")
                        .enabled(accent != AccentScheme::Blue)
                        .build()
                        .id("pref-accent-blue")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.accent_scheme = AccentScheme::Blue;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Teal")
                        .enabled(accent != AccentScheme::Teal)
                        .build()
                        .id("pref-accent-teal")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.accent_scheme = AccentScheme::Teal;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Violet")
                        .enabled(accent != AccentScheme::Violet)
                        .build()
                        .id("pref-accent-violet")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.accent_scheme = AccentScheme::Violet;
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Keymap preset"))
                .child(
                    widgets::Button::new(theme, "DSH")
                        .enabled(app.ui_preferences.keymap_preset != KeymapPreset::Dsh)
                        .build()
                        .id("pref-keymap-dsh")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.keymap_preset = KeymapPreset::Dsh;
                                preferences.keybindings.apply_preset(KeymapPreset::Dsh);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Zed")
                        .enabled(app.ui_preferences.keymap_preset != KeymapPreset::Zed)
                        .build()
                        .id("pref-keymap-zed")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.keymap_preset = KeymapPreset::Zed;
                                preferences.keybindings.apply_preset(KeymapPreset::Zed);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "VS Code")
                        .enabled(app.ui_preferences.keymap_preset != KeymapPreset::Vscode)
                        .build()
                        .id("pref-keymap-vscode")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.keymap_preset = KeymapPreset::Vscode;
                                preferences.keybindings.apply_preset(KeymapPreset::Vscode);
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Density"))
                .child(
                    widgets::Button::new(theme, "Comfortable")
                        .enabled(compact)
                        .build()
                        .id("pref-comfortable")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.density = UiDensity::Comfortable;
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Compact")
                        .enabled(!compact)
                        .build()
                        .id("pref-compact")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.density = UiDensity::Compact;
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, format!("Interface scale · {scale:.2}×")))
                .child(
                    widgets::Button::new(theme, "−")
                        .build()
                        .id("pref-scale-down")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.ui_scale =
                                    (preferences.ui_scale - 0.05).clamp(0.8, 1.6);
                            });
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "＋")
                        .build()
                        .id("pref-scale-up")
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.ui_scale =
                                    (preferences.ui_scale + 0.05).clamp(0.8, 1.6);
                            });
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Workspace Explorer"))
                .child(
                    widgets::Button::new(theme, if explorer { "Shown" } else { "Hidden" })
                        .primary()
                        .build()
                        .id("pref-explorer")
                        .on_click(cx.listener(move |app, _, window, cx| {
                            app.update_preferences(window, |preferences| {
                                preferences.show_workspace_explorer = !explorer;
                            });
                            app.show_explorer = !explorer;
                            app.ui_preferences.show_workspace_explorer = !explorer;
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(line(theme, "Restore last Session"))
                .child(
                    widgets::Button::new(
                        theme,
                        if restore_last_session { "On" } else { "Off" },
                    )
                    .build()
                    .id("pref-restore-session")
                    .on_click(cx.listener(move |app, _, window, cx| {
                        app.update_preferences(window, |preferences| {
                            preferences.restore_last_session = !restore_last_session;
                        });
                        cx.notify();
                    })),
                ),
        )
        .child(line(
            theme,
            "Unsent composer text is kept per Session and saved with this client.",
        ))
        .when_some(message, |element, message| {
            element.child(
                div()
                    .text_size(theme.fs(10.5))
                    .text_color(theme.muted)
                    .child(message),
            )
        })
        .child(widgets::separator(theme))
        .child(section(theme, "Desktop updates"))
        .child(line(
            theme,
            "Check the signed feed and confirm any download or installation here.",
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Check for updates")
                        .enabled(connected && !checking)
                        .build()
                        .id("pref-updates")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.open_updates();
                            cx.notify();
                        })),
                )
                .when(checking, |element| element.child(line(theme, "Checking…")))
                .when_some(update_status, |element, status| {
                    element.child(line(theme, status))
                }),
        )
        .child(widgets::separator(theme))
        .child(section(theme, "Host settings"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Refresh")
                        .enabled(connected && !settings_loading)
                        .build()
                        .id("pref-refresh-settings")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_settings();
                            cx.notify();
                        })),
                )
                .when(settings_loading, |element| element.child(line(theme, "Loading…")))
                .when_some(settings_error, |element, error| {
                    element.child(
                        div()
                            .text_size(theme.fs(10.5))
                            .text_color(theme.error)
                            .child(error),
                    )
                }),
        );
    if namespaces.is_empty() {
        content = content.child(line(
            theme,
            if connected {
                "No settings namespaces are reported yet."
            } else {
                "Connect to the Host to view or change agent and plugin settings."
            },
        ));
    }
    for (name, applies) in namespaces {
        content = content.child(
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(theme.fs(11.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(line(theme, applies)),
        );
    }
    content.into_any_element()
}

fn plugins(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.plugins_loading;
    let error = app.plugins_error.clone();
    let status = app.plugin_write_status.clone();
    let busy = app.plugin_write_receiver.is_some();
    let snapshot = app.plugins_snapshot.clone();
    let managed = app.managed_plugins.clone();
    let bundles = app.plugin_bundles.clone();

    let mut content = body(theme).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(6.))
            .child(
                widgets::Button::new(theme, "Refresh")
                    .enabled(!loading)
                    .build()
                    .id("plugins-refresh")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.refresh_plugins();
                        cx.notify();
                    })),
            )
            .when(loading, |element| element.child(line(theme, "Loading plugins…")))
            .when_some(error.clone(), |element, error| {
                element.child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.error)
                        .child(error),
                )
            })
            .when_some(status, |element, status| element.child(line(theme, status))),
    );

    let Some(snapshot) = snapshot else {
        return content
            .child(line(theme, "Open this tab to load the plugin inventory."))
            .into_any_element();
    };
    if snapshot.management_available == Some(false) {
        content = content.child(line(theme, "Plugin management is unavailable for this Host."));
    }
    content = content.child(section(theme, "CONFIGURED PLUGINS"));
    if snapshot.entries.is_empty() {
        content = content.child(line(theme, "No configured plugins."));
    }
    for entry in &snapshot.entries {
        let phase = entry.fiber_phase.map_or_else(
            || "unknown".to_owned(),
            |phase| format!("{phase:?}").to_lowercase(),
        );
        let state = if entry.enabled { "enabled" } else { "disabled" };
        let entry_id = entry.entry_id.clone();
        let enabled = entry.enabled;
        let patchable = managed
            .iter()
            .find(|plugin| plugin.entry_id == entry.entry_id)
            .is_some_and(|plugin| plugin.patch_id.is_some());
        let read_only = managed
            .iter()
            .find(|plugin| plugin.entry_id == entry.entry_id)
            .and_then(|plugin| plugin.read_only_reason.clone());
        content = content.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .flex_1()
                        .text_size(theme.fs(11.))
                        .child(format!("{} · {state} · {phase}", entry.module_name)),
                )
                .when(patchable, |element| {
                    element.child(
                        widgets::Button::new(theme, if enabled { "Disable" } else { "Enable" })
                            .enabled(!busy)
                            .build()
                            .id(gpui::SharedString::from(format!("plugin-toggle-{entry_id}")))
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.set_plugin_enabled(entry_id.clone(), !enabled);
                                cx.notify();
                            })),
                    )
                })
                .when_some(read_only, |element, reason| {
                    element.child(line(theme, format!("read-only: {reason}")))
                }),
        );
    }
    if !bundles.is_empty() {
        content = content
            .child(widgets::separator(theme))
            .child(section(theme, "BUNDLES"));
        for bundle in &bundles {
            let name = bundle.name.clone();
            let remove_name = bundle.name.clone();
            let enabled = bundle.enabled;
            let state = if enabled { "enabled" } else { "disabled" };
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(theme.sp(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(theme.fs(11.))
                                    .child(format!(
                                        "{} · {state} · {}",
                                        bundle.name, bundle.availability
                                    )),
                            )
                            .child(
                                widgets::Button::new(
                                    theme,
                                    if enabled { "Disable" } else { "Enable" },
                                )
                                .enabled(!busy)
                                .build()
                                .id(gpui::SharedString::from(format!("bundle-toggle-{name}")))
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.set_bundle_enabled(name.clone(), !enabled);
                                    cx.notify();
                                })),
                            )
                            .child(
                                widgets::Button::new(theme, "Remove")
                                    .danger()
                                    .enabled(!busy)
                                    .build()
                                    .id(gpui::SharedString::from(format!("bundle-remove-{remove_name}")))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.remove_bundle(remove_name.clone());
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when_some(bundle.description.clone(), |element, description| {
                        element.child(line(theme, description))
                    }),
            );
        }
    }
    if let Some(presets) = &snapshot.agent_presets {
        content = content
            .child(widgets::separator(theme))
            .child(section(theme, "AGENT PRESETS"));
        for preset in presets {
            let name = preset.name.clone().unwrap_or_else(|| preset.id.clone());
            content = content.child(line(theme, format!("{name} · {} row(s)", preset.rows.len())));
        }
    }
    content.into_any_element()
}

fn skills(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.skills_loading;
    let error = app.skills_error.clone();
    let skills = app.skills.clone();
    let selected = app.selected_skill.clone();
    let has_session = app.selected_session.is_some();

    let mut content = body(theme);
    if !has_session {
        return content
            .child(line(theme, "Open this tab with a session selected to load skills."))
            .into_any_element();
    }
    content = content.child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(theme.sp(6.))
            .child(
                widgets::Button::new(theme, "Refresh")
                    .enabled(!loading)
                    .build()
                    .id("skills-refresh")
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.refresh_skills();
                        cx.notify();
                    })),
            )
            .when(loading, |element| element.child(line(theme, "Loading skills…")))
            .when_some(error.clone(), |element, error| {
                element.child(
                    div()
                        .text_size(theme.fs(10.5))
                        .text_color(theme.error)
                        .child(error),
                )
            }),
    );
    let Some(skills) = skills else {
        return content
            .child(line(theme, "Skills load when this tab opens with a Session."))
            .into_any_element();
    };
    let chosen = skills
        .skills
        .iter()
        .find(|skill| selected.as_deref() == Some(skill.name.as_str()))
        .cloned();
    let mut list = div()
        .flex()
        .flex_row()
        .gap(theme.sp(12.))
        .flex_1()
        .min_h(px(0.));
    let mut names = div()
        .id("skills-list")
        .flex()
        .flex_col()
        .gap(px(1.))
        .w(theme.sp(260.))
        .overflow_y_scroll();
    if skills.skills.is_empty() {
        names = names.child(line(theme, "No skills are visible to this Session."));
    }
    for skill in &skills.skills {
        let name = skill.name.clone();
        let current = selected.as_deref() == Some(name.as_str());
        names = names.child(
            div()
                .px(theme.sp(8.))
                .py(theme.sp(5.))
                .rounded(theme.corner())
                .cursor_pointer()
                .when(current, |element| element.bg(theme.selection))
                .hover(|style| style.bg(theme.surface))
                .text_size(theme.fs(11.5))
                .id(gpui::SharedString::from(format!("skill-{name}")))
                .child(name.clone())
                .on_click(cx.listener(move |app, _, _, cx| {
                    app.selected_skill = Some(name.clone());
                    cx.notify();
                })),
        );
    }
    let mut detail = div().flex().flex_col().gap(theme.sp(6.)).flex_1().min_w(px(0.));
    if let Some(skill) = chosen {
        detail = detail
            .child(
                div()
                    .text_size(theme.fs(14.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(skill.name.clone()),
            )
            .child(line(theme, skill.description.clone()));
        if let Some(when) = skill.when_to_use.clone() {
            detail = detail
                .child(section(theme, "When it applies"))
                .child(line(theme, when));
        }
        if let Some(path) = skill.path.clone() {
            detail = detail.child(line(theme, path));
        }
    } else {
        detail = detail.child(line(theme, "Select a skill to read its description."));
    }
    list = list.child(names).child(detail);
    content.child(list).into_any_element()
}

fn marketplace(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.market_loading;
    let status = app.market_status.clone();
    let indexes = app.market_indexes.len();
    let results = app.market_results.clone();
    let connected = app.transport.is_some();
    let query = app.text_field("market-query", "Search text", false, cx);

    let mut content = body(theme)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Refresh indexes")
                        .enabled(connected && !loading)
                        .build()
                        .id("market-refresh")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_market_indexes();
                            cx.notify();
                        })),
                )
                .when(loading, |element| element.child(line(theme, "Loading…")))
                .when(indexes == 0 && !loading, |element| {
                    element.child(line(theme, "No market indexes configured."))
                })
                .when_some(status, |element, status| element.child(line(theme, status))),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .w(theme.sp(240.))
                        .h(theme.control_height())
                        .px(theme.sp(6.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .overflow_hidden()
                        .child(crate::shell::text_input::text_input_element(
                            &query,
                            theme,
                            theme.control_height() - px(2.),
                        )),
                )
                .child(
                    widgets::Button::new(theme, "Search")
                        .enabled(connected && !loading)
                        .build()
                        .id("market-search")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.search_market();
                            cx.notify();
                        })),
                ),
        );
    for entry in &results {
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("plugin")
            .to_owned();
        let description = entry
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let install = entry
            .get("versions")
            .and_then(serde_json::Value::as_array)
            .and_then(|versions| versions.first())
            .and_then(|version| version.get("install"))
            .cloned();
        let spec = install
            .as_ref()
            .and_then(|install| install.get("spec"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        content = content.child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(theme.sp(8.))
                .rounded(theme.corner())
                .bg(theme.surface)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(6.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(theme.fs(11.5))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(name.clone()),
                        )
                        .when_some(spec, |element, spec| {
                            element.child(
                                widgets::Button::new(theme, "Install")
                                    .enabled(connected && !loading)
                                    .build()
                                    .id(gpui::SharedString::from(format!("market-install-{name}")))
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        app.install_entry(spec.clone());
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .when(!description.is_empty(), |element| {
                    element.child(line(theme, description))
                }),
        );
    }
    content.into_any_element()
}

fn goals(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let Some(session_id) = app.selected_session.clone() else {
        return body(theme)
            .child(line(theme, "Select a Session to manage its goal."))
            .into_any_element();
    };
    let goal = app.goal.clone();
    let loading = app.goal_loading;
    let mutating = app.goal_mutation_loading;
    let status = app.goal_status.clone();
    let error = app.goal_error.clone();
    let plan_status = app.plan_command_status.clone();
    let plan_loading = app.plan_command_loading;
    let busy = app.chat_loading || app.assistant_streaming;
    let objective_input = app.text_field("goal-objective", "Goal objective", true, cx);
    let rounds_input = app.text_field("goal-rounds", "Maximum rounds", false, cx);
    app.goal_objective = objective_input.read(cx).text().to_owned();
    app.goal_max_rounds = rounds_input.read(cx).text().to_owned();

    let mut content = body(theme)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Refresh")
                        .enabled(!loading && !mutating)
                        .build()
                        .id("goal-refresh")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.refresh_goal();
                            cx.notify();
                        })),
                )
                .child(line(theme, format!("Session: {session_id}")))
                .when_some(status, |element, status| element.child(line(theme, status)))
                .when_some(error.clone(), |element, error| {
                    element.child(
                        div()
                            .text_size(theme.fs(10.5))
                            .text_color(theme.error)
                            .child(error),
                    )
                }),
        );

    if loading && goal.is_none() {
        return content.child(line(theme, "Loading goal…")).into_any_element();
    }

    let input_box = |input: &gpui::Entity<crate::shell::text_input::TextInput>, height: gpui::Pixels| {
        div()
            .w_full()
            .h(height)
            .px(theme.sp(6.))
            .rounded(theme.corner())
            .border_1()
            .border_color(theme.border)
            .bg(theme.code_background)
            .overflow_hidden()
            .child(crate::shell::text_input::text_input_element(
                input,
                theme,
                height - px(2.),
            ))
    };

    match goal.clone() {
        Some(goal) if goal.phase != dsh_client_core::goals::GoalPhase::Complete => {
            content = content
                .child(section(
                    theme,
                    &format!(
                        "{} · {:?} · {:?}",
                        goal.id, goal.phase, goal.activation
                    ),
                ))
                .child(line(
                    theme,
                    format!(
                        "{} of {} rounds used",
                        goal.rounds_started, goal.max_goal_rounds
                    ),
                ))
                .child(input_box(&objective_input, theme.sp(72.)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(6.))
                        .child(line(theme, "Maximum rounds"))
                        .child(
                            div()
                                .w(theme.sp(90.))
                                .h(theme.control_height())
                                .px(theme.sp(6.))
                                .rounded(theme.corner())
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.code_background)
                                .overflow_hidden()
                                .child(crate::shell::text_input::text_input_element(
                                    &rounds_input,
                                    theme,
                                    theme.control_height() - px(2.),
                                )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(theme.sp(6.))
                        .child(
                            widgets::Button::new(theme, "Save changes")
                                .enabled(!mutating)
                                .build()
                                .id("goal-save")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.save_goal_edits();
                                    cx.notify();
                                })),
                        )
                        .when(
                            goal.phase == dsh_client_core::goals::GoalPhase::Active
                                && goal.activation
                                    == dsh_client_core::goals::GoalActivation::Armed,
                            |element| {
                                element.child(
                                    widgets::Button::new(theme, "Pause")
                                        .enabled(!mutating)
                                        .build()
                                        .id("goal-pause")
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.change_goal(GoalAction::Pause);
                                            cx.notify();
                                        })),
                                )
                            },
                        )
                        .when(
                            goal.phase != dsh_client_core::goals::GoalPhase::Active
                                || goal.activation
                                    == dsh_client_core::goals::GoalActivation::Disarmed,
                            |element| {
                                element.child(
                                    widgets::Button::new(theme, "Resume")
                                        .enabled(!mutating)
                                        .build()
                                        .id("goal-resume")
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.change_goal(GoalAction::Resume);
                                            cx.notify();
                                        })),
                                )
                            },
                        )
                        .child(
                            widgets::Button::new(theme, "Complete")
                                .enabled(!mutating)
                                .build()
                                .id("goal-complete")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.change_goal(GoalAction::Complete);
                                    cx.notify();
                                })),
                        )
                        .child(
                            widgets::Button::new(theme, "Clear")
                                .danger()
                                .enabled(!mutating)
                                .build()
                                .id("goal-clear")
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.change_goal(GoalAction::Clear);
                                    cx.notify();
                                })),
                        ),
                );
        }
        _ => {
            if goal
                .as_ref()
                .is_some_and(|goal| goal.phase == dsh_client_core::goals::GoalPhase::Complete)
            {
                content = content
                    .child(line(theme, "The previous goal is complete. Create another goal for this Session."));
            } else {
                content = content.child(line(theme, "No goal is active for this Session."));
            }
            content = content
                .child(input_box(&objective_input, theme.sp(72.)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(theme.sp(6.))
                        .child(line(theme, "Maximum rounds (optional)"))
                        .child(
                            div()
                                .w(theme.sp(90.))
                                .h(theme.control_height())
                                .px(theme.sp(6.))
                                .rounded(theme.corner())
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.code_background)
                                .overflow_hidden()
                                .child(crate::shell::text_input::text_input_element(
                                    &rounds_input,
                                    theme,
                                    theme.control_height() - px(2.),
                                )),
                        ),
                )
                .child(
                    widgets::Button::new(theme, "Create goal")
                        .primary()
                        .enabled(!mutating)
                        .build()
                        .id("goal-create")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.create_goal();
                            cx.notify();
                        })),
                );
        }
    }

    let work_rows = app
        .transcript
        .rows()
        .iter()
        .rev()
        .filter(|row| matches!(row.kind, RowKind::Todo | RowKind::Goal | RowKind::Notice))
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    content = content
        .child(widgets::separator(theme))
        .child(section(theme, "RECENT TASKS AND PLAN CHANGES"));
    if work_rows.is_empty() {
        content = content.child(line(
            theme,
            "Task and plan updates will appear here when the assistant records them.",
        ));
    }
    for row in work_rows {
        content = content
            .child(
                div()
                    .text_size(theme.fs(11.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(format!("{} · sequence {}", row.title, row.seq)),
            )
            .child(line(theme, row.text));
    }
    content
        .child(widgets::separator(theme))
        .child(section(theme, "PLAN MODE"))
        .child(
            div()
                .flex()
                .flex_row()
                .gap(theme.sp(6.))
                .child(
                    widgets::Button::new(theme, "Enter plan mode")
                        .enabled(!plan_loading && !busy)
                        .build()
                        .id("plan-enter")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.run_plan_command(true);
                            cx.notify();
                        })),
                )
                .child(
                    widgets::Button::new(theme, "Leave plan mode")
                        .enabled(!plan_loading && !busy)
                        .build()
                        .id("plan-leave")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.run_plan_command(false);
                            cx.notify();
                        })),
                ),
        )
        .when_some(plan_status, |element, status| element.child(line(theme, status)))
        .into_any_element()
}

fn document(
    app: &mut DesktopApp,
    theme: &Theme,
    cx: &mut Context<DesktopApp>,
) -> AnyElement {
    let loading = app.preview_loading;
    let error = app.preview_error.clone();
    let text = app.preview_document.clone();
    let image = app.preview_image.clone();
    let path = app.preview_path.clone();
    let office = DesktopApp::is_office_path_now(&path);
    let input = app.text_field("document-path", "Host file path", false, cx);

    let mut content = body(theme)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(theme.sp(6.))
                .child(
                    div()
                        .flex_1()
                        .h(theme.control_height())
                        .px(theme.sp(6.))
                        .rounded(theme.corner())
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.code_background)
                        .overflow_hidden()
                        .child(crate::shell::text_input::text_input_element(
                            &input,
                            theme,
                            theme.control_height() - px(2.),
                        )),
                )
                .child(
                    widgets::Button::new(theme, "Preview")
                        .enabled(!loading)
                        .build()
                        .id("document-preview")
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.read_document();
                            cx.notify();
                        })),
                )
                .when(office, |element| {
                    element.child(
                        widgets::Button::new(theme, "Render Office preview")
                            .enabled(!loading)
                            .build()
                            .id("document-office")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.read_office_document();
                                cx.notify();
                            })),
                    )
                }),
        )
        .when(loading, |element| element.child(line(theme, "Loading document…")))
        .when_some(error.clone(), |element, error| {
            element.child(
                div()
                    .text_size(theme.fs(11.))
                    .text_color(theme.error)
                    .child(error),
            )
        });

    if let Some(text) = text {
        if let Some(pending) = app.pending_preview_text.take() {
            app.set_input("document-body", &pending, cx);
        }
        let body_input = app.text_field("document-body", "Document text", true, cx);
        content = content
            .child(
                div()
                    .text_size(theme.fs(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(text.name.clone()),
            )
            .child(line(theme, text.host_path.clone()))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(theme.sp(6.))
                    .child(
                        widgets::Button::new(theme, "Save to Host")
                            .enabled(app.transport.is_some() && app.preview_save_receiver.is_none())
                            .build()
                            .id("document-save-host")
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.save_preview_document();
                                cx.notify();
                            })),
                    )
                    .when_some(app.preview_save_status.clone(), |element, status| {
                        element.child(line(theme, status))
                    }),
            )
            .child(
                div()
                    .id("document-text")
                    .flex()
                    .flex_col()
                    .w_full()
                    .min_h(theme.sp(200.))
                    .p(theme.sp(8.))
                    .rounded(theme.corner())
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.code_background)
                    .font(theme.mono_font())
                    .overflow_hidden()
                    .child(crate::shell::text_input::text_input_element(
                        &body_input,
                        theme,
                        theme.sp(200.),
                    )),
            );
    } else if let Some(image) = image {
    } else if let Some(image) = image {
        content = content
            .child(
                div()
                    .text_size(theme.fs(13.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(image.name.clone()),
            )
            .child(line(theme, image.host_path.clone()))
            .child(line(
                theme,
                format!("{} × {} pixels", image.size[0], image.size[1]),
            ))
            .child(line(
                theme,
                "The decoded image is ready; scaled rendering is pending the gpui image pipeline.",
            ));
    } else if !loading && error.is_none() {
        content = content.child(line(theme, "No document is loaded."));
    }
    content.into_any_element()
}

/// Keeps the layout module's constants referenced from one place.
#[allow(dead_code)]
fn explorer_min() -> f32 {
    layout::EXPLORER_MIN
}
