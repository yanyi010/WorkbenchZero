//! Command registry. Everything actionable in Workbench Zero is a Command
//! (spec §15). Commands are metadata: execution is routed by the shell to the
//! owning plugin (via the plugin RPC bridge) or handled by the app itself.
//! Command IDs are public API and MUST remain stable.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("command `{0}` is already registered")]
    Duplicate(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CommandDef {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    /// Context expression, e.g. `workspace.open && terminal.active`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Owning plugin, `None` for app/core commands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Default keybinding, e.g. `Ctrl+Alt+M`. User overrides win.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_keybinding: Option<String>,
    /// Whether the command accepts free-text arguments (palette input line).
    #[serde(default)]
    pub takes_args: bool,
    /// Hidden commands do not show in the palette but remain invocable.
    #[serde(default)]
    pub hidden: bool,
    /// Declarative behavior: opening the named view of the owning plugin.
    /// The shell resolves this without loading the plugin iframe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opens_view: Option<String>,
}

#[derive(Default)]
pub struct CommandRegistry {
    commands: std::sync::RwLock<HashMap<String, CommandDef>>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, def: CommandDef) -> Result<(), RegistryError> {
        let mut map = self.commands.write().unwrap();
        if map.contains_key(&def.id) {
            return Err(RegistryError::Duplicate(def.id));
        }
        map.insert(def.id.clone(), def);
        Ok(())
    }

    /// Replace a registration (used when a plugin re-registers after reload).
    pub fn upsert(&self, def: CommandDef) {
        self.commands.write().unwrap().insert(def.id.clone(), def);
    }

    pub fn unregister(&self, id: &str) -> bool {
        self.commands.write().unwrap().remove(id).is_some()
    }

    pub fn unregister_by_plugin(&self, plugin_id: &str) -> Vec<String> {
        let mut map = self.commands.write().unwrap();
        let removed: Vec<String> = map
            .iter()
            .filter(|(_, d)| d.plugin_id.as_deref() == Some(plugin_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &removed {
            map.remove(id);
        }
        removed
    }

    pub fn get(&self, id: &str) -> Option<CommandDef> {
        self.commands.read().unwrap().get(id).cloned()
    }

    pub fn list(&self) -> Vec<CommandDef> {
        let mut all: Vec<CommandDef> = self.commands.read().unwrap().values().cloned().collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str) -> CommandDef {
        CommandDef {
            id: id.into(),
            title: format!("Title {id}"),
            category: None,
            keywords: vec![],
            when: None,
            plugin_id: Some("test.plugin".into()),
            default_keybinding: None,
            takes_args: false,
            hidden: false,
            opens_view: None,
        }
    }

    #[test]
    fn register_get_list() {
        let r = CommandRegistry::new();
        r.register(def("memo.new")).unwrap();
        r.register(def("memo.open")).unwrap();
        assert!(r.get("memo.new").is_some());
        assert_eq!(r.list().len(), 2);
    }

    #[test]
    fn duplicate_rejected() {
        let r = CommandRegistry::new();
        r.register(def("a.b")).unwrap();
        assert!(matches!(
            r.register(def("a.b")),
            Err(RegistryError::Duplicate(_))
        ));
    }

    #[test]
    fn unregister_by_plugin_cleans_up() {
        let r = CommandRegistry::new();
        r.register(def("p.one")).unwrap();
        r.register(def("p.two")).unwrap();
        let mut core = def("core.x");
        core.plugin_id = None;
        r.register(core).unwrap();
        let removed = r.unregister_by_plugin("test.plugin");
        assert_eq!(removed.len(), 2);
        assert_eq!(r.list().len(), 1);
    }
}
