//! Type › Spelling › Check Spelling… and Edit Custom Dictionary…: the misspellings
//! `text.spellCheck` finds, walked one at a time with Ignore, Ignore All, Change, Change All and
//! Add (to the custom dictionary) and a list of suggestions; the custom dictionary's words with
//! Add and Remove.
//!
//! Check Spelling fields: `language`, `languages` (codes installed), `misspellings` (the list from
//! the engine), `index` (the one shown), `replacement` (the word to change to), `checked`,
//! `done` (true once the list is used up). Custom Dictionary fields: `words`, `new`.

use serde_json::{Value, json};

use super::swatch_options::{grid, label};
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Check Spelling.
pub const KIND: &str = "checkSpelling";
/// The dialog kind of Edit Custom Dictionary.
pub const DICTIONARY_KIND: &str = "customDictionary";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Check Spelling").into(), body, confirm, ok: None, min_width: 420.0, ..DialogSpec::FORM };

pub(super) const DICTIONARY_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Edit Custom Dictionary").into(),
    body: dictionary_body,
    confirm: dictionary_confirm,
    ok: None,
    min_width: 360.0,
    ..DialogSpec::FORM
};

/// `ui.checkSpelling {language?}`: run the check and open the dialog on its first misspelling.
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let language = p.get("language").and_then(Value::as_str).map(str::to_string);
    let mut fields = json!({ "language": language.clone().unwrap_or_default(), "index": 0, "replacement": "", "done": false });
    // Known dictionaries, for the language menu.
    let langs = app.run("text.spellLanguages", json!({})).ok();
    let codes: Vec<Value> = langs
        .as_ref()
        .and_then(|l| l.get("languages"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|l| l.get("code").cloned()).collect())
        .unwrap_or_default();
    fields["languages"] = Value::Array(codes);
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    run_check(app)?;
    Ok(Value::Null)
}

/// Run `text.spellCheck` with the dialog's language and store the result in the dialog.
fn run_check(app: &mut VectorcraftApp) -> Result<(), String> {
    let Some(d) = app.ui.dialog.as_ref().filter(|d| d.kind == KIND) else { return Ok(()) };
    let language = d.str("language");
    let mut q = json!({});
    if !language.is_empty() {
        q["language"] = json!(language);
    }
    let r = match app.run("text.spellCheck", q) {
        Ok(r) => r,
        Err(e) => {
            app.ui.dialog = None;
            return Err(e);
        }
    };
    if let Some(d) = app.ui.dialog.as_mut() {
        d.fields.insert("misspellings".into(), r.get("misspellings").cloned().unwrap_or(json!([])));
        d.fields.insert("checked".into(), r.get("checked").cloned().unwrap_or(json!(0)));
        d.fields.insert("language".into(), r.get("language").cloned().unwrap_or(json!("")));
        d.fields.insert("index".into(), json!(0));
        let first = r.get("misspellings").and_then(Value::as_array).and_then(|a| a.first()).cloned();
        d.fields.insert(
            "replacement".into(),
            first.as_ref().and_then(|m| m.get("suggestions")).and_then(Value::as_array).and_then(|s| s.first()).cloned().unwrap_or(json!("")),
        );
        d.fields.insert("done".into(), json!(first.is_none()));
    }
    Ok(())
}

/// The misspelling shown.
fn current(d: &Dialog) -> Option<Value> {
    let i = d.f64("index", 0.0) as usize;
    d.fields.get("misspellings").and_then(Value::as_array).and_then(|a| a.get(i)).cloned()
}

/// Move the dialog on to the next misspelling (skipping ones whose word is `skip`).
fn advance(d: &mut Dialog, skip: Option<&str>) {
    let list = d.fields.get("misspellings").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut i = d.f64("index", 0.0) as usize + 1;
    while i < list.len() && skip.is_some_and(|w| list[i].get("word").and_then(Value::as_str) == Some(w)) {
        i += 1;
    }
    d.fields.insert("index".into(), json!(i));
    let next = list.get(i);
    let suggestion = next.and_then(|m| m.get("suggestions")).and_then(Value::as_array).and_then(|s| s.first()).cloned().unwrap_or(json!(""));
    d.fields.insert("replacement".into(), suggestion);
    d.fields.insert("done".into(), json!(next.is_none()));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    // Language.
    ui.horizontal(|ui| {
        label(ui, tl!("Language:"));
        let langs: Vec<String> = d
            .fields
            .get("languages")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let cur = d.str("language");
        let names: Vec<&str> = langs.iter().map(String::as_str).collect();
        if let Some(i) = widgets::dropdown_names(ui, "spell-language", &cur, &names, 160.0) {
            if let Some(code) = langs.get(i) {
                d.fields.insert("language".into(), json!(code));
                d.fields.insert("__recheck".into(), json!(true));
            }
        }
    });
    ui.add_space(6.0);
    let done = d.bool("done");
    if done {
        let checked = d.f64("checked", 0.0) as usize;
        ui.label(egui::RichText::new(tl!("The spell check is complete.")).color(t.text));
        ui.label(egui::RichText::new(format!("{}: {checked}", tl!("Words checked"))).color(t.text_dim));
        ui.add_space(6.0);
        if widgets::secondary_button(ui, tl!("Check Again")).clicked() {
            d.fields.insert("__recheck".into(), json!(true));
        }
        return false;
    }
    let Some(m) = current(d) else {
        d.fields.insert("done".into(), json!(true));
        return false;
    };
    let word = m.get("word").and_then(Value::as_str).unwrap_or("").to_string();
    let suggestions: Vec<String> =
        m.get("suggestions").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    grid(ui, |ui| {
        label(ui, tl!("Word Not Found:"));
        ui.label(egui::RichText::new(&word).strong().color(t.text));
        ui.end_row();
        label(ui, tl!("Change To:"));
        form::text(ui, d, "replacement", 220.0);
        ui.end_row();
        label(ui, tl!("Suggestions:"));
        ui.vertical(|ui| {
            if suggestions.is_empty() {
                ui.label(egui::RichText::new(tl!("No suggestions")).color(t.text_dim));
            }
            for s in &suggestions {
                let selected = d.str("replacement") == *s;
                if ui.selectable_label(selected, s.as_str()).clicked() {
                    d.fields.insert("replacement".into(), json!(s));
                }
            }
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    // Actions: each records what to do; `confirm` runs them after the frame.
    let mut action: Option<&str> = None;
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, tl!("Ignore")).clicked() {
            action = Some("ignore");
        }
        if widgets::secondary_button(ui, tl!("Ignore All")).clicked() {
            action = Some("ignoreAll");
        }
        if widgets::secondary_button(ui, tl!("Add")).clicked() {
            action = Some("add");
        }
    });
    ui.horizontal(|ui| {
        let can = !d.str("replacement").trim().is_empty();
        if ui.add_enabled(can, egui::Button::new(tl!("Change"))).clicked() {
            action = Some("change");
        }
        if ui.add_enabled(can, egui::Button::new(tl!("Change All"))).clicked() {
            action = Some("changeAll");
        }
    });
    if let Some(a) = action {
        d.fields.insert("__action".into(), json!(a));
    }
    // The actions run here, where the app is at hand (the body has it; the frame's confirm is Close).
    if let Some(a) = d.fields.remove("__action").and_then(|v| v.as_str().map(str::to_string)) {
        let _ = act(app, d, &a, &m, &word);
    }
    if d.fields.remove("__recheck").is_some() {
        d.fields.insert("__pending_recheck".into(), json!(true));
    }
    false
}

/// Apply one action to misspelling `m` and move on.
fn act(app: &mut VectorcraftApp, d: &mut Dialog, action: &str, m: &Value, word: &str) -> Result<(), String> {
    match action {
        "ignore" => advance(d, None),
        "ignoreAll" => {
            app.run("text.spellIgnore", json!({ "word": word }))?;
            advance(d, Some(word));
        }
        "add" => {
            app.run("text.spellAdd", json!({ "word": word }))?;
            advance(d, Some(word));
        }
        "change" | "changeAll" => {
            let with = d.str("replacement");
            let all = action == "changeAll";
            app.run(
                "text.spellReplace",
                json!({ "id": m.get("id"), "run": m.get("run"), "start": m.get("start"), "end": m.get("end"), "with": with, "all": all }),
            )?;
            // Offsets after the change have moved: check again, keeping the place in the list.
            let i = d.f64("index", 0.0) as usize;
            d.fields.insert("__pending_recheck".into(), json!(true));
            d.fields.insert("__resume".into(), json!(if all { 0 } else { i }));
        }
        _ => {}
    }
    Ok(())
}

/// Close (the frame's only button).
fn confirm(app: &mut VectorcraftApp, _: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    Ok(Value::Null)
}

/// Run a pending re-check (after a language change or a replacement) outside the body's borrow.
pub(crate) fn after_frame(app: &mut VectorcraftApp) {
    let pending = app.ui.dialog.as_mut().filter(|d| d.kind == KIND).and_then(|d| {
        let p = d.fields.remove("__pending_recheck").is_some();
        let resume = d.fields.remove("__resume").and_then(|v| v.as_u64()).map(|v| v as usize);
        p.then_some(resume)
    });
    if let Some(resume) = pending {
        let _ = run_check(app);
        if let (Some(i), Some(d)) = (resume, app.ui.dialog.as_mut()) {
            // Keep the place in the list.
            let n = d.fields.get("misspellings").and_then(Value::as_array).map_or(0, Vec::len);
            if i > 0 && i < n {
                d.fields.insert("index".into(), json!(i));
                let s = d
                    .fields
                    .get("misspellings")
                    .and_then(Value::as_array)
                    .and_then(|a| a.get(i))
                    .and_then(|m| m.get("suggestions"))
                    .and_then(Value::as_array)
                    .and_then(|s| s.first())
                    .cloned();
                d.fields.insert("replacement".into(), s.unwrap_or(json!("")));
            }
        }
    }
}

// ---------- Edit Custom Dictionary ----------

/// `ui.customDictionary {}`: open the custom dictionary editor.
pub fn open_dictionary(app: &mut VectorcraftApp, _: &Value) -> Result<Value, String> {
    let r = app.run("text.spellLanguages", json!({}))?;
    let words = r.get("custom").cloned().unwrap_or(json!([]));
    app.ui.dialog = Some(Dialog::new(DICTIONARY_KIND, json!({ "words": words, "new": "" })));
    Ok(Value::Null)
}

fn dictionary_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let words: Vec<String> =
        d.fields.get("words").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let mut remove: Option<String> = None;
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        if words.is_empty() {
            ui.label(egui::RichText::new(tl!("No words yet")).color(t.text_dim));
        }
        for w in &words {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(w).color(t.text));
                if ui.small_button("−").clicked() {
                    remove = Some(w.clone());
                }
            });
        }
    });
    ui.add_space(6.0);
    let mut add = false;
    ui.horizontal(|ui| {
        label(ui, tl!("Word:"));
        add |= form::text(ui, d, "new", 180.0) && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if widgets::secondary_button(ui, tl!("Add")).clicked() {
            add = true;
        }
    });
    if add {
        let w = d.str("new").trim().to_string();
        if !w.is_empty()
            && let Ok(r) = app.run("text.spellAdd", json!({ "word": w }))
        {
            d.fields.insert("words".into(), r.get("words").cloned().unwrap_or(json!([])));
            d.fields.insert("new".into(), json!(""));
        }
    }
    if let Some(w) = remove
        && let Ok(r) = app.run("text.spellRemove", json!({ "word": w }))
    {
        d.fields.insert("words".into(), r.get("words").cloned().unwrap_or(json!([])));
    }
    false
}

fn dictionary_confirm(app: &mut VectorcraftApp, _: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    Ok(Value::Null)
}
