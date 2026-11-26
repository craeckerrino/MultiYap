//! Fenster „Anzeigen“ eines Hervorhebungen-Tabs in Leptos (Nachbau von show-dialog.tsx; „Anzeigen“, damit es nicht mit
//! den Filter-Kategorien verwechselt wird). Änderungen gelten sofort und werden mit dem Tab gespeichert
//! (crates/web/src/workspace.rs).

use multiyap_core::i18n::tr;
use leptos::prelude::*;
use serde_json::{json, Value};

use crate::bridge::use_setting;
use crate::components::checkbox::Checkbox;
use crate::components::dialog::Dialog;
use crate::settings::model::{term_swatch, TermCategory};

/// Gewählt: „alle“ oder die Liste enthält den Eintrag.
/// Anzeige eines Kanals in der Auswahl: Twitch-Login, bei YouTube der Kanalname (sonst der Schlüssel) mit Plattform.
fn channel_name(login: &str) -> String {
    if !multiyap_core::youtube::is_channel_key(login) {
        return login.to_string();
    }
    let title = multiyap_web::youtube::view(login).and_then(|v| v.title).unwrap_or_else(|| login.to_string());
    format!("{title} · YouTube")
}

fn contains(selection: &Value, id: &str) -> bool {
    selection.as_str() == Some("all") || selection.as_array().is_some_and(|list| list.iter().any(|v| v.as_str() == Some(id)))
}

/// Auswahl „alle“ oder Liste: einen Eintrag umschalten (wie toggleIn in show-dialog.tsx). Aus „alle“ wird dabei eine
/// Liste aller bekannten Einträge; Einträge, die es nicht mehr gibt, fallen weg.
pub fn toggle_in(current: &Value, all: &[String], id: &str, checked: bool) -> Vec<String> {
    let mut list: Vec<String> = if current.as_str() == Some("all") {
        all.to_vec()
    } else {
        current.as_array().map(|l| l.iter().filter_map(|v| v.as_str()).filter(|v| all.iter().any(|a| a == v)).map(String::from).collect()).unwrap_or_default()
    };
    if checked {
        if !list.iter().any(|e| e == id) {
            list.push(id.to_string());
        }
    } else {
        list.retain(|e| e != id);
    }
    list
}

fn check_row(id: String, checked: Signal<bool>, indent: bool, on_change: Callback<bool>, label: AnyView) -> impl IntoView {
    let for_id = id.clone();
    view! {
        <div class=if indent { "flex items-center gap-2 py-1 pl-6" } else { "flex items-center gap-2 py-1" }>
            <Checkbox id=id checked=checked on_change=on_change />
            <label for=for_id class="flex items-center gap-2 text-sm">{label}</label>
        </div>
    }
}

/// `show`: Anzeige-Auswahl des Tabs; `channels`: gerade offene Kanäle.
#[component]
pub fn ShowDialog(tab_id: String, open: RwSignal<bool>, #[prop(into)] show: Signal<Value>, #[prop(into)] channels: Signal<Vec<String>>) -> impl IntoView {
    let tab_id = StoredValue::new(tab_id);
    let open_channels = channels;
    let categories = use_setting("highlights.categories", Vec::<TermCategory>::new());
    let save = move |key: &'static str, value: Value| {
        let mut next = show.get_untracked();
        if let Some(object) = next.as_object_mut() {
            object.insert(key.to_string(), value);
        }
        let id = tab_id.get_value();
        crate::layout::run(async move { multiyap_web::workspace::update_highlights_tab(&id, json!({ "show": next })).await });
    };
    // Auch Kanäle zeigen, die gewählt, aber gerade nicht offen sind.
    let listed_channels = Memo::new(move |_| {
        let mut list = open_channels.get();
        if let Some(chosen) = show.with(|s| s.get("channels").and_then(|c| c.as_array().cloned())) {
            for login in chosen.iter().filter_map(|v| v.as_str()) {
                if !list.iter().any(|l| l == login) {
                    list.push(login.to_string());
                }
            }
        }
        list
    });
    let selection = move |key: &'static str| show.with(|s| s.get(key).cloned().unwrap_or(Value::Null));
    let flag = move |key: &'static str| Signal::derive(move || show.with(|s| s.get(key).and_then(Value::as_bool).unwrap_or(false)));
    let toggle_flag = move |key: &'static str| Callback::new(move |value: bool| save(key, Value::Bool(value)));

    let content = move || {
        let id = tab_id.get_value();
        let category_ids: Vec<String> = categories.with(|c| c.iter().map(|c| c.id.clone()).collect());
        let channel_list = listed_channels.get();
        view! {
            <div class="flex flex-col gap-4">
                <section class="flex flex-col">
                    <h3 class="mb-1 text-sm font-semibold">"Filter"</h3>
                    {check_row(format!("{id}-terms-all"), Signal::derive(move || selection("terms").as_str() == Some("all")), false,
                        Callback::new(move |checked: bool| save("terms", if checked { json!("all") } else { json!([]) })), tr("Alle Kategorien", "All categories").into_any())}
                    {categories.get().into_iter().map(|category| {
                        let (cid, ids) = (category.id.clone(), category_ids.clone());
                        let check = cid.clone();
                        check_row(format!("{id}-term-{}", category.id), Signal::derive(move || contains(&selection("terms"), &check)), true,
                            Callback::new(move |checked: bool| save("terms", json!(toggle_in(&selection("terms"), &ids, &cid, checked)))),
                            view! { <span class=format!("size-3 rounded-full {}", term_swatch(&category.color)) aria-hidden="true"></span>{category.name.clone()} }.into_any())
                    }).collect_view()}
                </section>
                <section class="flex flex-col">
                    <h3 class="mb-1 text-sm font-semibold">{tr("Weitere", "More")}</h3>
                    {check_row(format!("{id}-mentions"), flag("mentions"), false, toggle_flag("mentions"), tr("Erwähnungen", "Mentions").into_any())}
                    {check_row(format!("{id}-events"), flag("events"), false, toggle_flag("events"), tr("Chat-Ereignisse", "Chat events").into_any())}
                    {check_row(format!("{id}-mod"), flag("modActions"), false, toggle_flag("modActions"), tr("Mod-Aktionen", "Mod actions").into_any())}
                    {check_row(format!("{id}-automod"), flag("automod"), false, toggle_flag("automod"), "AutoMod".into_any())}
                </section>
                <section class="flex flex-col">
                    <h3 class="mb-1 text-sm font-semibold">{tr("Kanäle", "Channels")}</h3>
                    {check_row(format!("{id}-channels-all"), Signal::derive(move || selection("channels").as_str() == Some("all")), false,
                        Callback::new(move |checked: bool| save("channels", if checked { json!("all") } else { json!([]) })), tr("Alle offenen Kanäle", "All open channels").into_any())}
                    {channel_list.iter().map(|login| {
                        let (all, check, toggle) = (channel_list.clone(), login.clone(), login.clone());
                        check_row(format!("{id}-channel-{login}"), Signal::derive(move || contains(&selection("channels"), &check)), true,
                            Callback::new(move |checked: bool| save("channels", json!(toggle_in(&selection("channels"), &all, &toggle, checked)))),
                            channel_name(login).into_any())
                    }).collect_view()}
                </section>
            </div>
        }
    };

    view! {
        <Dialog open=open title=tr("Anzeigen", "Show") description=Signal::derive(|| tr("Was dieser Tab sammelt. Änderungen gelten sofort.", "What this tab collects. Changes apply immediately.").to_string()) max_width="sm:max-w-md">
            {content}
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auswahl_umschalten() {
        let all = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(toggle_in(&json!("all"), &all, "b", false), ["a", "c"]);
        assert_eq!(toggle_in(&json!(["a", "weg"]), &all, "c", true), ["a", "c"]);
        assert_eq!(toggle_in(&json!(["a"]), &all, "a", true), ["a"]);
        assert!(contains(&json!("all"), "x"));
        assert!(!contains(&json!(["a"]), "b"));
    }
}
