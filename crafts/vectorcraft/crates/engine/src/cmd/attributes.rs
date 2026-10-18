//! Window → Attributes: overprint, centre point display, image map, URL and note
//! (`attributes.set`), the fill rule (`path.setFillRule`) and, through `path.reverse`, the path
//! direction. The panel reads everything back with `attributes.info` ([`super::overprint`]).

use std::collections::BTreeSet;

use redrob_vector_doc::{Document, ImageMap, Node, NodeKind, ObjectAttributes};
use redrob_vector_geom::FillRule;
use serde_json::{Value, json};

use super::edit::selected_roots;
use super::overprint::{AttributesInfo, apply_overprint, edit_counted, overprint_jobs};
use super::path::paths_in;
use super::*;

/// How many URLs the Attributes panel's recent list keeps.
const RECENT_URLS: usize = 10;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "attributes.set",
            "Attributes",
            [],
            None,
            "{overprintFill?: bool, overprintStroke?: bool (aimed as object.setOverprint's fill and stroke, item? too), showCenter?: bool (the canvas shows the selected object's centre point), imageMap?: \"none\"|\"rectangle\"|\"polygon\", url?: string (\"\" removes it; SVG export links the object), urlTarget?: string (the link's target: _blank, _self…; \"\" none), urlTitle?: string (the link's tooltip), events?: {onclick?: script, onmouseover?: …} (Object Properties › Interactivity: SVG event attributes; a key given \"\" is removed, others are kept), highlight?: \"#rrggbb\"|null (the colour the object's outline shows in while selected; null: the layer colour), dpi?: number|null (images: the pixels per inch they show at), preserveAspectRatio?: \"xMidYMid meet\"|\"none\"|…|null (images), note?: string, title?: string, description?: string (Object Properties; SVG export writes <title> and <desc>; \"\" removes them), ids?|id?, item?: appearance item index|null, target?: \"object\"|\"contents\"} set the Attributes panel's values on ids or the selection, as one undo step; a URL given joins the recent URLs (attributes.info) → {changed: objects}",
            has_doc,
            set
        ),
        cmd!(
            "object.createAnchor",
            "Create Anchor",
            ["Object"],
            None,
            "{url?: string (the link; default: the recent URL, else empty, to be set in Object Properties), target?: _blank|_self|…, title?: string, ids?} wrap the selected objects in an anchor (a group named Anchor that links to the URL: the SVG <a> element; Object Properties edits its href, target and title) → {id}",
            has_selection,
            create_anchor
        ),
        cmd!(
            query "object.followLink",
            "Follow Link",
            ["Object"],
            None,
            "{id?} → {url, id} the link the selected object (or the anchor it is inside) points to; an error when none does. The app opens it in the browser",
            has_selection,
            follow_link
        ),
        cmd!(
            "path.setFillRule",
            "Fill Rule",
            [],
            None,
            "{rule: \"nonZero\"|\"evenOdd\", ids?|id?} the fill rule of the paths and compound paths in ids or the selection (groups: those inside), as one undo step → {changed}",
            has_doc,
            set_fill_rule
        ),
    ]
}

/// A fill rule as the commands name it.
pub(super) fn rule_name(r: FillRule) -> &'static str {
    match r {
        FillRule::NonZero => "nonZero",
        FillRule::EvenOdd => "evenOdd",
    }
}

fn parse_rule(s: &str) -> Option<FillRule> {
    match s.to_ascii_lowercase().replace(['-', ' ', '_'], "").as_str() {
        "nonzero" | "nonzerowinding" => Some(FillRule::NonZero),
        "evenodd" => Some(FillRule::EvenOdd),
        _ => None,
    }
}

/// The one value every item of `it` shares, if any.
fn same<T: PartialEq>(mut it: impl Iterator<Item = T>) -> Option<T> {
    let first = it.next()?;
    it.all(|v| v == first).then_some(first)
}

/// The paths and compound paths in `n`'s subtree that have a fill rule of their own with it (not a
/// compound path's members, which fill with its rule).
fn ruled<'a>(n: &'a Node, out: &mut Vec<(&'a Node, FillRule)>) {
    match &n.kind {
        NodeKind::Path { rule, guide: false, .. } | NodeKind::Compound { rule, .. } => out.push((n, *rule)),
        _ => n.children().into_iter().flatten().for_each(|c| ruled(c, out)),
    }
}

/// A directly selected compound member fills with its compound's rule. Its other attributes
/// and path direction still belong to the member itself.
fn fill_rule_owner(d: &Document, id: NodeId) -> Option<&Node> {
    let n = d.node(id)?;
    match d.parent_of(id).and_then(|parent| d.node(parent)) {
        Some(parent) if matches!(parent.kind, NodeKind::Compound { .. }) => Some(parent),
        _ => Some(n),
    }
}

/// The Attributes panel's values of `info.ids` beyond overprinting.
pub(super) fn fill_info(d: &Document, info: &mut AttributesInfo) {
    let nodes: Vec<&Node> = info.ids.iter().filter_map(|id| d.node(*id)).collect();
    let attrs = || nodes.iter().map(|n| n.extras().attrs.as_deref().cloned().unwrap_or_default());
    info.show_center = same(nodes.iter().map(|n| n.shows_center()));
    info.image_map = same(attrs().map(|a| a.image_map));
    info.url = same(attrs().map(|a| a.url));
    info.note = same(attrs().map(|a| a.note));
    info.title = same(attrs().map(|a| a.title));
    info.description = same(attrs().map(|a| a.description));
    info.url_target = same(attrs().map(|a| a.url_target));
    info.url_title = same(attrs().map(|a| a.url_title));
    info.events = same(attrs().map(|a| a.events));
    info.highlight = same(attrs().map(|a| a.highlight));
    info.dpi = same(attrs().map(|a| a.dpi));
    info.preserve_aspect_ratio = same(attrs().map(|a| a.preserve_aspect_ratio));
    let mut all = vec![];
    nodes.iter().filter_map(|n| fill_rule_owner(d, n.id)).for_each(|n| ruled(n, &mut all));
    info.fill_rule = same(all.into_iter().map(|(_, r)| r));
    let paths = paths_in(d, &info.ids);
    let subpaths = paths.iter().filter_map(|id| d.node(*id)?.path_data()).flat_map(|p| &p.subpaths);
    info.reversed = same(subpaths.map(|sp| sp.signed_area() < 0.0));
}

/// The attribute edits `p` asks for, applied to `a`.
#[derive(Default)]
struct Edits {
    show_center: Option<bool>,
    image_map: Option<ImageMap>,
    url: Option<String>,
    note: Option<String>,
    title: Option<String>,
    description: Option<String>,
    url_target: Option<String>,
    url_title: Option<String>,
    /// Event attributes to set (an empty script removes the event).
    events: Option<Vec<(String, String)>>,
    /// `Some(None)` clears the highlight.
    highlight: Option<Option<String>>,
    dpi: Option<Option<f64>>,
    preserve_aspect_ratio: Option<Option<String>>,
}

impl Edits {
    fn parse(p: &Value) -> Result<Self> {
        const C: &str = "attributes.set";
        let text = |k: &str| -> Result<Option<String>> {
            p.get(k).map(|v| v.as_str().map(|s| s.trim().to_string()).ok_or_else(|| bad(C, format!("`{k}` must be a string")))).transpose()
        };
        Ok(Self {
            show_center: p.get("showCenter").map(|v| v.as_bool().ok_or_else(|| bad(C, "`showCenter` must be true or false"))).transpose()?,
            image_map: text("imageMap")?
                .map(|s| ImageMap::parse(&s).ok_or_else(|| bad(C, "`imageMap` must be none, rectangle or polygon")))
                .transpose()?,
            url: text("url")?,
            note: text("note")?,
            title: text("title")?,
            description: text("description")?,
            url_target: text("urlTarget")?,
            url_title: text("urlTitle")?,
            events: match p.get("events") {
                None => None,
                Some(Value::Object(o)) => Some(
                    o.iter()
                        .map(|(k, v)| {
                            let k = k.trim().to_ascii_lowercase();
                            if !k.starts_with("on") || k.len() < 3 || !k.chars().all(|c| c.is_ascii_alphanumeric()) {
                                return Err(bad(C, format!("`events` keys are SVG event attributes such as onclick, not {k:?}")));
                            }
                            Ok((k, v.as_str().ok_or_else(|| bad(C, "`events` values must be strings"))?.to_string()))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
                Some(_) => return Err(bad(C, "`events` must be an object of event attribute → script")),
            },
            highlight: match p.get("highlight") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => {
                    Some(Some(color_value(v).map(|c| c.to_hex()).ok_or_else(|| bad(C, "`highlight` must be a colour such as \"#ff8800\" or null"))?))
                }
            },
            dpi: match p.get("dpi") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => {
                    Some(Some(v.as_f64().filter(|d| d.is_finite() && *d > 0.0).ok_or_else(|| bad(C, "`dpi` must be a positive number or null"))?))
                }
            },
            preserve_aspect_ratio: match p.get("preserveAspectRatio") {
                None => None,
                Some(Value::Null) => Some(None),
                Some(v) => Some(Some(
                    v.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == ' '))
                        .ok_or_else(|| bad(C, "`preserveAspectRatio` must be an SVG value such as \"xMidYMid meet\" or null"))?
                        .to_string(),
                )),
            },
        })
    }

    fn is_empty(&self) -> bool {
        self.show_center.is_none()
            && self.image_map.is_none()
            && self.url.is_none()
            && self.note.is_none()
            && self.title.is_none()
            && self.description.is_none()
            && self.url_target.is_none()
            && self.url_title.is_none()
            && self.events.is_none()
            && self.highlight.is_none()
            && self.dpi.is_none()
            && self.preserve_aspect_ratio.is_none()
    }

    /// `n`'s attributes with the edits applied (Show Center kept only where it isn't the default).
    fn apply(&self, n: &Node) -> ObjectAttributes {
        let mut a = n.extras().attrs.as_deref().cloned().unwrap_or_default();
        if let Some(v) = self.show_center {
            a.show_center = (v != n.shows_center_by_default()).then_some(v);
        }
        if let Some(v) = self.image_map {
            a.image_map = v;
        }
        if let Some(v) = &self.url {
            a.url.clone_from(v);
        }
        if let Some(v) = &self.note {
            a.note.clone_from(v);
        }
        if let Some(v) = &self.title {
            a.title.clone_from(v);
        }
        if let Some(v) = &self.description {
            a.description.clone_from(v);
        }
        if let Some(v) = &self.url_target {
            a.url_target.clone_from(v);
        }
        if let Some(v) = &self.url_title {
            a.url_title.clone_from(v);
        }
        if let Some(events) = &self.events {
            for (k, script) in events {
                a.events.retain(|(e, _)| e != k);
                if !script.is_empty() {
                    a.events.push((k.clone(), script.clone()));
                }
            }
        }
        if let Some(v) = &self.highlight {
            a.highlight.clone_from(v);
        }
        if let Some(v) = self.dpi {
            a.dpi = v;
        }
        if let Some(v) = &self.preserve_aspect_ratio {
            a.preserve_aspect_ratio.clone_from(v);
        }
        a
    }
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "attributes.set";
    let jobs = overprint_jobs(s, p, C, ["overprintFill", "overprintStroke"])?;
    let edits = Edits::parse(p)?;
    if jobs.is_empty() && edits.is_empty() {
        return Err(bad(
            C,
            "give overprintFill, overprintStroke, showCenter, imageMap, url, urlTarget, urlTitle, events, highlight, dpi, preserveAspectRatio, note, title or description",
        ));
    }
    let ids = targets(s, p)?;
    let changed = edit_counted(s, "Attributes", |d| {
        let mut changed = BTreeSet::new();
        apply_overprint(d, &jobs, C, &mut changed)?;
        if !edits.is_empty() {
            for id in &ids {
                let Some(n) = d.node(*id) else { continue };
                let new = edits.apply(n);
                if new != n.extras().attrs.as_deref().cloned().unwrap_or_default()
                    && let Some(n) = d.node_mut(*id)
                {
                    n.edit_attrs(|a| *a = new);
                    changed.insert(*id);
                }
            }
        }
        Ok(changed.len())
    })?;
    if let Some(url) = edits.url.filter(|u| !u.is_empty() && !ids.is_empty()) {
        s.recent_urls.retain(|u| *u != url);
        s.recent_urls.insert(0, url);
        s.recent_urls.truncate(RECENT_URLS);
    }
    Ok(json!({ "changed": changed }))
}

fn set_fill_rule(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.setFillRule";
    let rule = str_param(p, "rule").and_then(parse_rule).ok_or_else(|| bad(C, "`rule` must be nonZero or evenOdd"))?;
    let ids = targets(s, p)?;
    let changed = edit_counted(s, "Fill Rule", |d| {
        let mut all = vec![];
        ids.iter().filter_map(|id| fill_rule_owner(d, *id)).for_each(|n| ruled(n, &mut all));
        let todo: BTreeSet<_> = all.into_iter().filter(|(_, r)| *r != rule).map(|(n, _)| n.id).collect();
        for id in &todo {
            if let Some(NodeKind::Path { rule: r, .. } | NodeKind::Compound { rule: r, .. }) = d.node_mut(*id).map(|n| &mut n.kind) {
                *r = rule;
            }
        }
        Ok(todo.len())
    })?;
    Ok(json!({ "changed": changed }))
}

fn create_anchor(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.createAnchor";
    let url = match p.get("url") {
        None => s.recent_urls.first().cloned().unwrap_or_default(),
        Some(v) => v.as_str().map(|u| u.trim().to_string()).ok_or_else(|| bad(C, "`url` must be a string"))?,
    };
    let target = str_param(p, "target").unwrap_or("").trim().to_string();
    let title = str_param(p, "title").unwrap_or("").trim().to_string();
    let ids = if p.get("ids").is_some() { checked_ids_param(s, p, "ids", C)? } else { selected_roots(s)? };
    if ids.is_empty() {
        return Err(bad(C, "nothing selected"));
    }
    let gid = s.edit("Create Anchor", |d, sel| {
        let gid = d.alloc_id();
        let id = d.wrap(&ids, |children| {
            let mut g = Node::group(gid, children);
            g.name = Some("Anchor".into());
            g.edit_attrs(|a| {
                a.url = url.clone();
                a.url_target = target.clone();
                a.url_title = title.clone();
            });
            g
        })?;
        sel.set([id]);
        Ok(id)
    })?;
    if let Some(u) = s.recent_urls.iter().position(|u| *u == url) {
        s.recent_urls.remove(u);
    }
    if !url.is_empty() {
        s.recent_urls.insert(0, url);
        s.recent_urls.truncate(RECENT_URLS);
    }
    Ok(json!({ "id": gid.0 }))
}

fn follow_link(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.followLink";
    let st = s.doc()?;
    let start = match id_param(p, "id") {
        Some(id) => id,
        None => *st.selection.objects.first().ok_or_else(|| bad(C, "nothing selected"))?,
    };
    // The object's own link, else the nearest anchor it is inside.
    let mut cur = Some(start);
    while let Some(id) = cur {
        if let Some(url) = st.doc.node(id).and_then(|n| n.url()) {
            return Ok(json!({ "url": url, "id": id.0 }));
        }
        cur = st.doc.parent_of(id);
    }
    Err(bad(C, "the selected object has no link"))
}
