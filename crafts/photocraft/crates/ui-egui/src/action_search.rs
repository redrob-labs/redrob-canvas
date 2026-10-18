//! Action search (Window › Search Actions, ⌘⇧↩): type to find any command or recorded action and
//! run it. The engine ranks the matches (`actions.search`); this is the floating field, the
//! result list and the keyboard: ↑/↓ move, ↩ runs the highlighted match, Esc closes.

use egui::{Key, RichText, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

/// The open search: its query, the matches, the highlighted row.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ActionSearch {
    pub query: String,
    #[serde(skip)]
    pub results: Vec<Value>,
    #[serde(skip)]
    pub selected: usize,
    #[serde(skip)]
    pub searched: String,
}

/// The UI command that opens the search.
pub const COMMAND: &str = "window.actionSearch";

pub fn open(app: &mut PhotocraftApp) {
    app.ui.action_search = Some(ActionSearch::default());
}

/// Draw the search when it is open; run once a frame from the document area.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut st) = app.ui.action_search.clone() else { return };
    if st.searched != st.query {
        st.results = match app.session.execute("actions.search", json!({ "query": st.query, "limit": 12 })) {
            Ok(r) if !st.query.trim().is_empty() => r["results"].as_array().cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        st.selected = 0;
        st.searched = st.query.clone();
    }
    let (esc, up, down, enter) =
        ctx.input(|i| (i.key_pressed(Key::Escape), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter)));
    if esc {
        app.ui.action_search = None;
        return;
    }
    if down && !st.results.is_empty() {
        st.selected = (st.selected + 1).min(st.results.len() - 1);
    }
    if up {
        st.selected = st.selected.saturating_sub(1);
    }
    let screen = ctx.content_rect();
    let width = 420.0f32.min(screen.width() - 32.0);
    let pos = egui::pos2(screen.center().x - width / 2.0, screen.top() + 80.0);
    let mut run: Option<Value> = None;
    let mut close = false;
    let id = egui::Id::new("photocraft-action-search");
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        let t = Tokens::get(ui.ctx());
        egui::Frame::NONE
            .fill(t.dock)
            .stroke(egui::Stroke::new(1.0, t.field_border))
            .corner_radius(egui::CornerRadius::same(t.radius_sm as u8))
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(width);
                let field = widgets::text_field(ui, id.with("query"), &mut st.query, tl!("Search actions and commands"), width - 20.0, false);
                if st.results.is_empty() && !st.query.trim().is_empty() && st.searched == st.query {
                    field.request_focus();
                } else if ui.memory(|m| m.focused().is_none()) {
                    field.request_focus();
                }
                if !st.results.is_empty() {
                    ui.add_space(6.0);
                    for (i, r) in st.results.iter().enumerate() {
                        let label = r["label"].as_str().unwrap_or("");
                        let menu: Vec<&str> = r["menu"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                        let enabled = r["enabled"].as_bool().unwrap_or(true);
                        let (rect, resp) = ui.allocate_exact_size(vec2(width - 20.0, 24.0), egui::Sense::click());
                        if i == st.selected {
                            ui.painter().rect_filled(rect, t.radius_sm, t.accent_soft);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, t.radius_sm, t.hover);
                        }
                        let color = if enabled { t.text } else { t.text_faint };
                        let mut text = label.to_string();
                        if !menu.is_empty() {
                            text = format!("{text}  ·  {}", menu.join(" › "));
                        }
                        if r["kind"] == "action" {
                            text = format!("{text}  ·  {}", tl!("Action"));
                        }
                        ui.painter().text(rect.left_center() + vec2(8.0, 0.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(12.5), color);
                        if let Some(sc) = r["shortcut"].as_str() {
                            ui.painter().text(
                                rect.right_center() - vec2(8.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                sc,
                                egui::FontId::proportional(11.0),
                                t.text_dim,
                            );
                        }
                        if resp.clicked() {
                            run = Some(r.clone());
                        }
                    }
                } else if !st.query.trim().is_empty() {
                    ui.label(RichText::new(tl!("No matching action")).color(t.text_faint));
                }
            });
    });
    if enter && run.is_none() {
        run = st.results.get(st.selected).cloned();
    }
    if crate::canvas_popup::outside_pressed(ctx, id, area.response.rect) {
        close = true;
    }
    if let Some(r) = run {
        let id = r["id"].as_str().unwrap_or("").to_string();
        let params = if r["kind"] == "action" { r["params"].clone() } else { json!({}) };
        app.ui.action_search = None;
        if let Err(e) = crate::menus::invoke(app, ctx, &id, params) {
            app.ui.status = e;
        }
        return;
    }
    app.ui.action_search = if close { None } else { Some(st) };
}
