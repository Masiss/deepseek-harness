//! Copy for the desktop shell, generated from `apps/desktop/src/locale.ts`.
//!
//! Regenerate with `bun apps/desktop-tauri/scripts/generate-locale.ts`.

/// The English copy of the shell's dialogs.
pub const EN: &[(&str, &str)] = &[
    ("startupFailed", "DeepSeek Harness is unavailable"),
    ("fatalSummary", "The application could not start or stopped unexpectedly."),
    ("quitTitle", "Quit DeepSeek Harness?"),
    ("quitActiveTasks", "Running tasks will be interrupted."),
    ("quit", "Quit"),
    ("cancel", "Cancel"),
    ("updateTitle", "DeepSeek Harness Update"),
    ("updateAvailable", "New version available: {version}"),
    ("updateDetail", "Once the download is complete, you can install the update and restart the app."),
    ("updateDownloadedTitle", "Version {version} is ready to install"),
    ("updateDownloadedDetail", "The app will close during the update and reopen automatically when it is complete."),
    ("updateDownload", "Download update"),
    ("updateStopTasks", "Stop tasks and update"),
    ("updateLater", "Update later"),
    ("updateAcknowledge", "OK"),
];

/// The Chinese copy of the shell's dialogs.
pub const ZH: &[(&str, &str)] = &[
    ("startupFailed", "DeepSeek Harness 无法使用"),
    ("fatalSummary", "应用无法启动或已意外停止。"),
    ("quitTitle", "退出 DeepSeek Harness？"),
    ("quitActiveTasks", "当前正在运行的任务将会中断"),
    ("quit", "退出"),
    ("cancel", "取消"),
    ("updateTitle", "DeepSeek Harness 更新"),
    ("updateAvailable", "发现新版本 {version}"),
    ("updateDetail", "下载完成后，可安装并重启应用。"),
    ("updateDownloadedTitle", "新版本 {version} 已准备就绪"),
    ("updateDownloadedDetail", "更新期间应用将暂时关闭，完成后会自动打开。"),
    ("updateDownload", "下载更新"),
    ("updateStopTasks", "停止任务并更新"),
    ("updateLater", "稍后更新"),
    ("updateAcknowledge", "确定"),
];

/// Look a key up in one locale, falling back to English.
fn lookup(table: &'static [(&'static str, &'static str)], key: &str) -> &'static str {
    table
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| *value)
        .unwrap_or("")
}

/// The copy for one key in the session's language.
///
/// The session's language follows the environment; an unknown language reads
/// the English copy.
pub fn text(key: &str) -> &'static str {
    let language = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default()
        .to_lowercase();
    let chinese = language.starts_with("zh");
    let table = if chinese { ZH } else { EN };
    let value = lookup(table, key);
    if value.is_empty() {
        lookup(EN, key)
    } else {
        value
    }
}
