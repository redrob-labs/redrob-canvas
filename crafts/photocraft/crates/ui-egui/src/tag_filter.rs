//! Tag filter for resource panels (Gradients, Patterns, Styles, Shapes, Tool Presets, Swatches,
//! Dynamics): a field at the top of the panel whose words filter the list by tag (or name) prefix,
//! with autocomplete of the tags in use, and a tag editor for the selected resource.
//!
//! Tags come and go through the `resource.*` commands (`resource_tags_cmds`); the filter text
//! itself is per-panel UI state kept in egui's temp data, so no panel struct changes.

use egui::RichText;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::Tokens;

fn filter_id(kind: &str) -> egui::Id {
    egui::Id::new(("resource-tag-filter", kind.to_string()))
}

fn new_tag_id(kind: &str) -> egui::Id {
    egui::Id::new(("resource-new-tag", kind.to_string()))
}

/// The filter text of `kind`'s panel.
pub fn query(ctx: &egui::Context, kind: &str) -> String {
    ctx.data(|d| d.get_temp::<String>(filter_id(kind))).unwrap_or_default()
}

/// Sets the filter text of `kind`'s panel (tests and scripts).
pub fn set_query(ctx: &egui::Context, kind: &str, q: &str) {
    ctx.data_mut(|d| d.insert_temp(filter_id(kind), q.to_string()));
}

/// Whether resource `name` of `kind` passes the panel's current filter.
pub fn visible(app: &PhotocraftApp, ctx: &egui::Context, kind: &str, name: &str) -> bool {
    let q = query(ctx, kind);
    q.trim().is_empty() || redrob_photo_engine::resource_tags_cmds::matches(&app.session, kind, name, &q)
}

/// Keeps only the items of `groups` that pass the filter (`name` picks the item's name).
pub fn filter_groups<T>(
    app: &PhotocraftApp,
    ctx: &egui::Context,
    kind: &str,
    groups: &mut [redrob_photo_engine::presets::Group<T>],
    name: impl Fn(&T) -> String,
) {
    let q = query(ctx, kind);
    if q.trim().is_empty() {
        return;
    }
    for g in groups.iter_mut() {
        g.items.retain(|i| redrob_photo_engine::resource_tags_cmds::matches(&app.session, kind, &name(i), &q));
    }
}

/// The filter field: typing filters the panel; the tags starting with the word being typed are
/// offered below it and a click completes the word. Returns true when the filter changed.
pub fn filter_row(app: &PhotocraftApp, ui: &mut egui::Ui, kind: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let mut q = query(&ctx, kind);
    let mut changed = false;
    let id = filter_id(kind);
    ui.horizontal(|ui| {
        crate::icons::paint(ui, ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::hover()).0, "search", 12.0, t.text_dim);
        let r = crate::widgets::text_field(ui, id.with("field"), &mut q, tl!("Filter by tag"), (ui.available_width() - 24.0).max(60.0), false);
        if r.changed() {
            changed = true;
        }
        if !q.is_empty() && crate::icons::button(ui, "x", 18.0, false, tl!("Clear filter")).clicked() {
            q.clear();
            changed = true;
        }
        // Autocomplete: the tags starting with the last word, while the field is edited.
        let last = q.rsplit(' ').next().unwrap_or("").to_string();
        if r.has_focus() && !last.is_empty() {
            let tags: Vec<String> = redrob_photo_engine::resource_tags_cmds::all_tags(&app.session, Some(kind))
                .into_iter()
                .filter(|x| x.to_lowercase().starts_with(&last.to_lowercase()) && !x.eq_ignore_ascii_case(&last))
                .take(6)
                .collect();
            if !tags.is_empty() {
                let below = r.rect.left_bottom();
                egui::Area::new(id.with("popup")).order(egui::Order::Foreground).fixed_pos(below + egui::vec2(0.0, 2.0)).show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        for tag in tags {
                            if ui.add(egui::Button::new(RichText::new(&tag).size(11.5)).frame(false)).clicked() {
                                let mut words: Vec<&str> = q.split(' ').collect();
                                words.pop();
                                words.push(&tag);
                                q = words.join(" ");
                                q.push(' ');
                                changed = true;
                            }
                        }
                    });
                });
            }
        }
    });
    if changed {
        set_query(&ctx, kind, &q);
    }
    changed
}

/// The tag editor of resource `name`: its tags as removable chips and a field to add one.
pub fn tag_editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, kind: &str, name: &str) {
    let t = Tokens::get(ui.ctx());
    let tags = redrob_photo_engine::resource_tags_cmds::tags_of(&app.session, kind, name);
    let id = new_tag_id(kind);
    let mut add: Option<String> = None;
    let mut remove: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tl!("Tags:")).color(t.text_dim).size(11.0));
        for tag in &tags {
            ui.label(RichText::new(tag).size(11.0).color(t.text));
            if crate::icons::button(ui, "x", 14.0, false, tl!("Remove tag")).clicked() {
                remove = Some(tag.clone());
            }
        }
        let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
        let r = crate::widgets::text_field(ui, id.with("field"), &mut text, tl!("Add tag"), 90.0, false);
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
            add = Some(text.trim().to_string());
            text.clear();
        }
        ui.data_mut(|d| d.insert_temp(id, text));
    });
    if let Some(tag) = add {
        let _ = app.run("resource.tag", json!({"kind": kind, "name": name, "tag": tag}));
    }
    if let Some(tag) = remove {
        let _ = app.run("resource.untag", json!({"kind": kind, "name": name, "tag": tag}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_hides_untagged_resources_and_clears() {
        let mut app = PhotocraftApp::new(redrob_photo_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let mut groups = app.session.presets.gradients.clone();
        let first = groups[0].items[0].name.clone();
        let total: usize = groups.iter().map(|g| g.items.len()).sum();
        app.run("resource.tag", json!({"kind": "gradient", "name": first, "tag": "Favourite"})).unwrap();
        assert!(visible(&app, &ctx, "gradient", &first));
        set_query(&ctx, "gradient", "fav");
        filter_groups(&app, &ctx, "gradient", &mut groups, |g| g.name.clone());
        assert_eq!(groups.iter().map(|g| g.items.len()).sum::<usize>(), 1);
        assert_eq!(groups[0].items[0].name, first);
        set_query(&ctx, "gradient", "");
        let mut all = app.session.presets.gradients.clone();
        filter_groups(&app, &ctx, "gradient", &mut all, |g| g.name.clone());
        assert_eq!(all.iter().map(|g| g.items.len()).sum::<usize>(), total);
    }
}
