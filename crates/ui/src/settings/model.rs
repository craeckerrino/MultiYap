//! Werte der Einstellungen, wie sie gespeichert sind (Schemas in crates/core/src/settings.rs).

use multiyap_core::i18n::tr;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use multiyap_core::shortcuts::ShortcutEntry;

pub const MAX_REASON_LENGTH: usize = 500;

/// Namen der Mod-Aktionen in den Einstellungen.
pub fn action_label(action: &str) -> &'static str {
    match action {
        "delete" => tr("Löschen", "Delete"),
        "warn" => tr("Verwarnen", "Warn"),
        "timeout" => "Timeout",
        "ban" => "Ban",
        "unban" => "Unban",
        _ => "",
    }
}

pub const ALL_ACTIONS: [&str; 5] = ["delete", "warn", "timeout", "ban", "unban"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickButton {
    pub id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasonDialog {
    pub timeout: bool,
    pub ban: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReasonTemplate {
    pub id: String,
    pub text: String,
    pub hotkey: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReasonTemplates {
    pub warn: Vec<ReasonTemplate>,
    pub timeout: Vec<ReasonTemplate>,
    pub ban: Vec<ReasonTemplate>,
}

impl ReasonTemplates {
    pub fn list(&self, action: &str) -> &Vec<ReasonTemplate> {
        match action {
            "warn" => &self.warn,
            "ban" => &self.ban,
            _ => &self.timeout,
        }
    }

    pub fn list_mut(&mut self, action: &str) -> &mut Vec<ReasonTemplate> {
        match action {
            "warn" => &mut self.warn,
            "ban" => &mut self.ban,
            _ => &mut self.timeout,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShortcutList {
    pub enabled: bool,
    pub entries: Vec<ShortcutEntry>,
}

/// Allgemeine Tastenkürzel: Aktion → Kombination oder null.
pub type Hotkeys = BTreeMap<String, Option<String>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermCategory {
    pub id: String,
    pub name: String,
    pub color: String,
    pub terms: Vec<String>,
    #[serde(default)]
    pub regex: bool,
    pub scroll_marker: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TermException {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub all_categories: bool,
    pub category_ids: Vec<String>,
}

pub fn term_colors() -> [(&'static str, &'static str); 6] {
    [("term-1", tr("Rot", "Red")), ("term-2", "Orange"), ("term-3", tr("Gelb", "Yellow")), ("term-4", tr("Grün", "Green")), ("term-5", tr("Blau", "Blue")), ("term-6", tr("Violett", "Violet"))]
}

/// Farbpunkt einer Kategorie (Tailwind braucht die Namen ausgeschrieben).
pub fn term_swatch(color: &str) -> &'static str {
    match color {
        "term-1" => "bg-term-1",
        "term-2" => "bg-term-2",
        "term-3" => "bg-term-3",
        "term-4" => "bg-term-4",
        "term-5" => "bg-term-5",
        _ => "bg-term-6",
    }
}
