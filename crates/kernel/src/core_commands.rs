//! Core commands registered at bootstrap. These are executed by the
//! application shell (main frame); plugin commands are routed to plugin
//! iframes. Default keybindings can be overridden by the user (spec §74).

use eigendesk_commands::{CommandDef, CommandRegistry};

pub fn register_core_commands(registry: &CommandRegistry) {
    let core = |id: &str, title: &str, category: &str, keybinding: Option<&str>| CommandDef {
        id: id.to_string(),
        title: title.to_string(),
        category: Some(category.to_string()),
        keywords: vec![],
        when: None,
        plugin_id: None,
        default_keybinding: keybinding.map(|s| s.to_string()),
        takes_args: false,
        hidden: false,
    };
    let commands = vec![
        core("core.showPalette", "Show Command Palette", "View", Some("Ctrl+K")),
        core("core.quickCapture", "Quick Capture", "Capture", Some("Alt+Space")),
        core("core.universalSearch", "Universal Search", "View", Some("Ctrl+P")),
        core("core.toggleSidebar", "Toggle Sidebar", "View", Some("Ctrl+B")),
        core("core.toggleBottomPanel", "Toggle Bottom Panel", "View", Some("Ctrl+J")),
        core("core.closeTab", "Close Tab", "View", Some("Ctrl+W")),
        core("core.nextTab", "Next Tab", "View", Some("Ctrl+Tab")),
        core("core.prevTab", "Previous Tab", "View", Some("Ctrl+Shift+Tab")),
        core("core.splitRight", "Split Editor Right", "View", Some("Ctrl+\\")),
        core("core.openDashboard", "Open Dashboard", "View", None),
        core("core.openSearch", "Open Search View", "View", None),
        core("core.openPlugins", "Open Plugin Store", "Plugins", Some("Ctrl+Shift+P")),
        core("core.openSettings", "Open Settings", "Settings", Some("Ctrl+,")),
        core("core.showKeyboardShortcuts", "Show Keyboard Shortcuts", "Help", Some("Ctrl+/")),
        core("core.toggleTheme", "Toggle Light/Dark Theme", "Appearance", None),
        core("core.newWorkspace", "New Workspace", "Workspace", None),
        core("core.openWorkspace", "Open Workspace", "Workspace", None),
        core("core.switchWorkspace", "Switch Workspace", "Workspace", None),
        core("core.closeWorkspace", "Close Workspace", "Workspace", None),
        core("core.openLogsFolder", "Open Logs Folder", "Diagnostics", None),
        core("core.exportDiagnostics", "Export Diagnostics Bundle", "Diagnostics", None),
        core("core.reloadWindow", "Reload Window", "Developer", Some("Ctrl+R")),
        core("core.quit", "Quit EigenDesk", "App", None),
    ];
    // App commands that receive arguments.
    let with_args = vec![CommandDef {
        id: "core.pluginRestart".to_string(),
        title: "Restart Plugin".to_string(),
        category: Some("Plugins".to_string()),
        keywords: vec![],
        when: None,
        plugin_id: None,
        default_keybinding: None,
        takes_args: true,
        hidden: true,
    }];
    for def in commands.into_iter().chain(with_args) {
        if let Err(e) = registry.register(def) {
            tracing::warn!(error = %e, "core command registration conflict");
        }
    }
}
