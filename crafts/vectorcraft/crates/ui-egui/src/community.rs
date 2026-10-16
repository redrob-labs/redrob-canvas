//! Project links: the "Report an Issue" button (About, Home) and the link list. Icons are
//! Lucide's (ISC).

use egui::{Color32, CornerRadius, Sense, Ui, vec2};

use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, icons};

/// The issues button's label, height and width.
fn issues_layout(ui: &Ui) -> (std::sync::Arc<egui::Galley>, f32, f32) {
    let h = 36.0;
    let galley = ui.painter().layout_no_wrap(tl!("Report an Issue").to_string(), theme::semibold(14.0), Color32::WHITE);
    let w = galley.size().x + h * 0.55 + h * 0.9;
    (galley, h, w)
}

/// A pill button that opens the GitHub issue tracker (About / Home).
pub fn issues_button(app: &mut VectorcraftApp, ui: &mut Ui) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (galley, h, w) = issues_layout(ui);
    let icon = h * 0.55;
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let fill = if resp.hovered() { t.accent.gamma_multiply(0.85) } else { t.accent };
    ui.painter().rect_filled(r, CornerRadius::same((h / 2.0) as u8), fill);
    let ir = egui::Rect::from_center_size(r.left_center() + vec2(h * 0.35 + icon / 2.0, 0.0), vec2(icon, icon));
    icons::paint(ui, "message-circle", ir, Color32::WHITE);
    ui.painter().galley(egui::pos2(ir.right() + h * 0.2, r.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
    let resp = resp.on_hover_text(tl!("Bugs, feedback and feature requests on GitHub"));
    if resp.clicked() {
        app.open_link("help.issues");
    }
    resp
}

/// One link row: icon, label, opens the command's URL.
fn link(app: &mut VectorcraftApp, ui: &mut Ui, icon: &str, label: &str, cmd: &str, url: &str) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        icons::icon(ui, icon, 16.0, t.icon);
        let r = ui.add(egui::Button::new(egui::RichText::new(label).color(t.accent).size(13.0)).frame(false)).on_hover_text(url);
        if r.clicked() {
            app.open_link(cmd);
        }
    });
}

/// The issues button and the website / releases / GitHub links.
pub fn links(app: &mut VectorcraftApp, ui: &mut Ui) {
    issues_button(app, ui);
    ui.add_space(8.0);
    let l = app.session.execute("help.links", &serde_json::json!({})).unwrap_or_default();
    let s = |k: &str| l[k].as_str().unwrap_or("").to_string();
    link(app, ui, "globe", tl!("Redrob website"), "help.website", &s("website"));
    link(app, ui, "external-link", tl!("Redrob Canvas releases"), "help.appPage", &s("appPage"));
    link(app, ui, "git-branch", tl!("Source code on GitHub"), "help.github", &s("github"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_clicks_open_the_right_urls() {
        use std::sync::{Arc, Mutex};
        let opened = Arc::new(Mutex::new(vec![]));
        let o = opened.clone();
        let services = crate::Services { open_url: Some(Box::new(move |u: &str| o.lock().unwrap().push(u.to_string()))), ..Default::default() };
        let mut app = VectorcraftApp::new(redrob_vector_engine::Session::new(), services);
        for id in ["help.issues", "help.website", "help.appPage", "help.github"] {
            crate::menus::invoke(&mut app, id, serde_json::json!({}));
        }
        assert_eq!(
            *opened.lock().unwrap(),
            [
                "https://github.com/redrob-labs/redrob-canvas/issues",
                "https://redrob.ai",
                "https://github.com/redrob-labs/redrob-canvas/releases",
                "https://github.com/redrob-labs/redrob-canvas"
            ]
        );
        // The control channel / MCP path returns the URL without opening a browser.
        let v = app.run("help.issues", serde_json::json!({})).unwrap();
        assert_eq!(v["url"], "https://github.com/redrob-labs/redrob-canvas/issues");
        assert_eq!(opened.lock().unwrap().len(), 4);
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| links(&mut app, ui));
        out.textures_delta.clear();
    }
}
