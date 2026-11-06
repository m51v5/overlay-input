//! Layout builder: palette, tree of rows/columns/widgets with drag-and-drop, and widget options inline.

use egui::{Frame, Id, Margin, RichText, Sense, Stroke, Ui, vec2};
use serde_json::{Value, json};

use super::{BLUE, BORDER, DANGER, PANEL, PANEL_2, card, fields, muted};
use crate::ui::schema::{Forms, humanize};
use crate::ui::tree::{self, Kind};

/// Darker than controls, so slider rails and inputs stand out.
const WIDGET_BG: egui::Color32 = egui::Color32::from_rgb(0x16, 0x19, 0x1d);

#[derive(Default)]
pub struct LayoutState {
    /// Selected node: palette clicks add there; a selected widget shows its options.
    selected: Option<Vec<usize>>,
}

/// What is being dragged: a node already in the tree, or a new one from the palette.
#[derive(Clone)]
enum Drag {
    Existing(Vec<usize>),
    New(Value),
}

enum Action {
    Select(Vec<usize>),
    /// Click on a widget title: open its options, or close them again.
    Toggle(Vec<usize>),
    Drop {
        item: Drag,
        parent: Vec<usize>,
        index: usize,
    },
    Shift(Vec<usize>, isize),
    Remove(Vec<usize>),
    SetKind(Vec<usize>, Kind),
    SetAlign(Vec<usize>, Option<&'static str>),
    Add(Value),
    Wrap,
    Start,
}

struct Ctx<'a> {
    forms: &'a Forms,
    selected: Option<&'a [usize]>,
    dragging: bool,
    actions: Vec<Action>,
}

impl LayoutState {
    pub fn show(&mut self, ui: &mut Ui, profile: &mut Value, forms: &Forms) {
        let root = profile.get("layout");
        if let (Some(root), Some(sel)) = (root, &self.selected)
            && tree::get(root, sel).is_none()
        {
            self.selected = None;
        }
        let mut actions = Vec::new();
        card(ui, "Layout", |ui| {
            ui.label(muted(
                "Drag ⠿ to move. Rows put items side by side, columns stack them.",
            ));
            ui.add_space(4.0);
            actions.extend(palette(ui, forms));
            if let Some(root) = profile.get("layout") {
                ui.label(muted(format!("Click adds to {}.", self.add_target(root))).small());
            }
            ui.add_space(6.0);

            let Some(root) = profile.get_mut("layout") else {
                if ui.button("Start a layout").clicked() {
                    actions.push(Action::Start);
                }
                return;
            };
            if tree::kind(root) == Some(Kind::Widget) {
                ui.horizontal(|ui| {
                    ui.label(muted("This layout is a single widget."));
                    if ui.button("Wrap in a column").clicked() {
                        actions.push(Action::Wrap);
                    }
                });
            }
            let mut ctx = Ctx {
                forms,
                selected: self.selected.as_deref(),
                dragging: egui::DragAndDrop::has_payload_of_type::<Drag>(ui.ctx()),
                actions: Vec::new(),
            };
            node(ui, root, &[], 0, &mut ctx);
            actions.extend(ctx.actions);
            if tree::widget_count(root) == 0 {
                ui.label(
                    RichText::new(
                        "Add at least one widget: an empty layout can't be shown or saved.",
                    )
                    .color(DANGER),
                );
            }
        });
        drag_ghost(ui, profile.get("layout"));
        for action in actions {
            self.apply(profile, action);
        }
    }

    /// Where a palette click lands, in words.
    fn add_target(&self, root: &Value) -> String {
        let path = self.selected.as_deref().unwrap_or(&[]);
        let node = tree::get(root, path).unwrap_or(root);
        match tree::kind(node) {
            Some(Kind::Widget) if path.is_empty() => "a new column with it".into(),
            Some(Kind::Widget) => {
                format!("after {}", humanize(node["widget"].as_str().unwrap_or("")))
            }
            Some(k) if path.is_empty() => format!("the top {}", k.key()),
            Some(k) => format!("the selected {}", k.key()),
            None => "the layout".into(),
        }
    }

    fn apply(&mut self, profile: &mut Value, action: Action) {
        let Some(map) = profile.as_object_mut() else {
            return;
        };
        match action {
            Action::Start => {
                map.insert("layout".into(), json!({ "column": [] }));
                self.selected = Some(Vec::new());
                return;
            }
            Action::Wrap => {
                if let Some(root) = map.remove("layout") {
                    map.insert("layout".into(), json!({ "column": [root] }));
                    self.selected = Some(vec![0]);
                }
                return;
            }
            _ => {}
        }
        let Some(root) = map.get_mut("layout") else {
            return;
        };
        match action {
            Action::Select(path) => self.selected = Some(path),
            Action::Toggle(path) => {
                self.selected = if self.selected.as_ref() == Some(&path) {
                    Some(path[..path.len().saturating_sub(1)].to_vec())
                } else {
                    Some(path)
                };
            }
            Action::Drop {
                item,
                parent,
                index,
            } => {
                let placed = match item {
                    Drag::Existing(from) => tree::move_node(root, &from, &parent, index),
                    Drag::New(node) => tree::insert(root, &parent, index, node),
                };
                if placed.is_some() {
                    self.selected = placed;
                }
            }
            Action::Shift(path, delta) => {
                let (Some((&i, parent)), true) = (path.split_last(), delta != 0) else {
                    return;
                };
                let Some(list) = tree::get_mut(root, parent).and_then(tree::children_mut) else {
                    return;
                };
                let j = i as isize + delta;
                if j < 0 || j as usize >= list.len() {
                    return;
                }
                list.swap(i, j as usize);
                self.selected = Some([parent, &[j as usize]].concat());
            }
            Action::Remove(path) => {
                tree::remove(root, &path);
                if self
                    .selected
                    .as_deref()
                    .is_some_and(|s| tree::is_within(s, &path))
                {
                    self.selected = Some(Vec::new());
                }
            }
            Action::SetKind(path, kind) => {
                if let Some(node) = tree::get_mut(root, &path) {
                    tree::set_container_kind(node, kind);
                }
            }
            Action::SetAlign(path, align) => {
                if let Some(node) = tree::get_mut(root, &path).and_then(Value::as_object_mut) {
                    match align {
                        Some(a) => node.insert("align".into(), a.into()),
                        None => node.remove("align"),
                    };
                }
            }
            Action::Add(new) => {
                let path = self.selected.clone().unwrap_or_default();
                let path = if tree::get(root, &path).is_some() {
                    path
                } else {
                    Vec::new()
                };
                if path.is_empty() && tree::kind(root) == Some(Kind::Widget) {
                    let old = root.take();
                    *root = json!({ "column": [old, new] });
                    self.selected = Some(vec![1]);
                } else if tree::kind(tree::get(root, &path).unwrap_or(root)) == Some(Kind::Widget) {
                    let (&i, parent) = path.split_last().expect("non-root widget");
                    self.selected = tree::insert(root, parent, i + 1, new);
                } else {
                    let len = tree::get(root, &path)
                        .and_then(tree::children)
                        .map_or(0, Vec::len);
                    self.selected = tree::insert(root, &path, len, new);
                }
            }
            Action::Start | Action::Wrap => unreachable!("handled above"),
        }
    }
}

/// Widget chips, then Row and Column: drag into the tree or click to add.
fn palette(ui: &mut Ui, forms: &Forms) -> Option<Action> {
    let mut items: Vec<(String, Value)> = forms
        .widgets
        .iter()
        .map(|(name, _)| (humanize(name), tree::new_node(Kind::Widget, Some(name))))
        .collect();
    items.push(("⇢ Row".into(), tree::new_node(Kind::Row, None)));
    items.push(("⇣ Column".into(), tree::new_node(Kind::Column, None)));
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        // Chips wrap as a whole, never their text.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for (label, node) in items {
            let chip = ui
                .add(
                    egui::Button::new(&label)
                        .corner_radius(12)
                        .sense(Sense::click_and_drag()),
                )
                .on_hover_text("Drag into the layout, or click to add");
            chip.dnd_set_drag_payload(Drag::New(node.clone()));
            if chip.clicked() {
                action = Some(Action::Add(node));
            }
        }
    });
    action
}

fn node(ui: &mut Ui, value: &mut Value, path: &[usize], siblings: usize, ctx: &mut Ctx) {
    match tree::kind(value) {
        Some(Kind::Widget) => widget(ui, value, path, siblings, ctx),
        Some(kind) => container(ui, value, kind, path, siblings, ctx),
        None => {
            ui.label(RichText::new(format!("Unrecognized layout item: {value}")).color(DANGER));
        }
    }
}

fn widget(ui: &mut Ui, value: &mut Value, path: &[usize], siblings: usize, ctx: &mut Ctx) {
    let open = ctx.selected == Some(path);
    let name = value["widget"].as_str().unwrap_or_default().to_owned();
    let set = value.as_object().map_or(0, |m| m.len().saturating_sub(1));
    Frame::new()
        .fill(WIDGET_BG)
        .stroke(Stroke::new(1.0, if open { BLUE } else { BORDER }))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                grip(ui, path);
                let title = format!("{} {}", if open { "▾" } else { "▸" }, humanize(&name));
                let summary = if set == 0 {
                    "defaults".to_owned()
                } else {
                    format!("{set} option{} set", if set == 1 { "" } else { "s" })
                };
                let clicked = ui
                    .add(egui::Button::new(RichText::new(title).strong()).frame(false))
                    .clicked();
                ui.label(muted(summary));
                if clicked {
                    ctx.actions.push(Action::Toggle(path.to_vec()));
                }
                tools(ui, path, siblings, ctx);
            });
            if open {
                ui.add_space(6.0);
                match (ctx.forms.widget(&name), value.as_object_mut()) {
                    (Some(specs), Some(map)) if !specs.is_empty() => {
                        fields::form(ui, ("widget", path), specs, map, |_| None);
                    }
                    (Some(_), _) => {
                        ui.label(muted("No options."));
                    }
                    (None, _) => {
                        ui.label(
                            RichText::new(format!("Unknown widget \"{name}\".")).color(DANGER),
                        );
                    }
                }
            }
        });
}

fn container(
    ui: &mut Ui,
    value: &mut Value,
    kind: Kind,
    path: &[usize],
    siblings: usize,
    ctx: &mut Ctx,
) {
    let selected = ctx.selected == Some(path);
    let align = value["align"].as_str().unwrap_or("start").to_owned();
    Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, if selected { BLUE } else { BORDER }))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                grip(ui, path);
                let (title, note) = match kind {
                    Kind::Row => ("⇢ Row", "side by side"),
                    _ => ("⇣ Column", "stacked"),
                };
                if ui
                    .add(egui::Button::new(RichText::new(title).strong()).frame(false))
                    .on_hover_text("Select: palette clicks add here")
                    .clicked()
                {
                    ctx.actions.push(Action::Select(path.to_vec()));
                }
                ui.label(muted(note));
                let other = if kind == Kind::Row {
                    Kind::Column
                } else {
                    Kind::Row
                };
                if ui
                    .small_button("⇄")
                    .on_hover_text(format!("Make it a {}", other.key()))
                    .clicked()
                {
                    ctx.actions.push(Action::SetKind(path.to_vec(), other));
                }
                // Cross axis: a row aligns heights, a column aligns widths.
                let labels = if kind == Kind::Row {
                    ["Top", "Middle", "Bottom"]
                } else {
                    ["Left", "Center", "Right"]
                };
                for (value, label) in ["start", "center", "end"].into_iter().zip(labels) {
                    if ui
                        .selectable_label(align == value, label)
                        .on_hover_text(format!("Align {}", label.to_lowercase()))
                        .clicked()
                    {
                        let align = (value != "start").then_some(value);
                        ctx.actions.push(Action::SetAlign(path.to_vec(), align));
                    }
                }
                tools(ui, path, siblings, ctx);
            });

            let Some(children) = tree::children_mut(value) else {
                return;
            };
            ui.add_space(4.0);
            ui.indent(("children", path), |ui| {
                let count = children.len();
                if count == 0 {
                    drop_zone(
                        ui,
                        path,
                        0,
                        Some("Empty: drag widgets here, or select it and click the palette"),
                        ctx,
                    );
                    return;
                }
                drop_zone(ui, path, 0, None, ctx);
                for (i, child) in children.iter_mut().enumerate() {
                    let child_path = [path, &[i]].concat();
                    node(ui, child, &child_path, count, ctx);
                    drop_zone(ui, path, i + 1, None, ctx);
                }
            });
        });
}

/// Drag handle; the root has none.
fn grip(ui: &mut Ui, path: &[usize]) {
    if path.is_empty() {
        return;
    }
    ui.add(egui::Label::new(RichText::new("⠿").size(16.0)).sense(Sense::drag()))
        .on_hover_text("Drag to move")
        .dnd_set_drag_payload(Drag::Existing(path.to_vec()));
}

/// The dragged item's name next to the pointer.
fn drag_ghost(ui: &Ui, root: Option<&Value>) {
    let Some(drag) = egui::DragAndDrop::payload::<Drag>(ui.ctx()) else {
        return;
    };
    let Some(pointer) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let node = match &*drag {
        Drag::Existing(path) => root.and_then(|r| tree::get(r, path)),
        Drag::New(node) => Some(node),
    };
    let label = match node.and_then(tree::kind) {
        Some(Kind::Widget) => humanize(node.and_then(|n| n["widget"].as_str()).unwrap_or("")),
        Some(Kind::Row) => "⇢ Row".into(),
        Some(_) => "⇣ Column".into(),
        None => return,
    };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    egui::Area::new(Id::unique("drag-ghost"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + vec2(14.0, 10.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            Frame::new()
                .fill(PANEL_2)
                .stroke(Stroke::new(1.0, BLUE))
                .corner_radius(6)
                .inner_margin(Margin::symmetric(8, 3))
                .show(ui, |ui| ui.label(RichText::new(label).strong()));
        });
}

/// Move up / down / remove, right-aligned; the root has none.
fn tools(ui: &mut Ui, path: &[usize], siblings: usize, ctx: &mut Ctx) {
    let Some(&i) = path.last() else {
        return;
    };
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .small_button(RichText::new("✕").color(DANGER))
            .on_hover_text("Remove")
            .clicked()
        {
            ctx.actions.push(Action::Remove(path.to_vec()));
        }
        if ui
            .add_enabled(i + 1 < siblings, egui::Button::new("↓").small())
            .on_hover_text("Move down")
            .clicked()
        {
            ctx.actions.push(Action::Shift(path.to_vec(), 1));
        }
        if ui
            .add_enabled(i > 0, egui::Button::new("↑").small())
            .on_hover_text("Move up")
            .clicked()
        {
            ctx.actions.push(Action::Shift(path.to_vec(), -1));
        }
    });
}

/// Drop target between items (only while dragging) or in an empty container.
fn drop_zone(ui: &mut Ui, parent: &[usize], index: usize, hint: Option<&str>, ctx: &mut Ctx) {
    if !ctx.dragging && hint.is_none() {
        ui.add_space(4.0);
        return;
    }
    let frame = Frame::new()
        .corner_radius(4)
        .inner_margin(Margin::symmetric(6, 2));
    let (_, payload) = ui.dnd_drop_zone::<Drag, ()>(frame, |ui| {
        ui.set_min_size(vec2(
            ui.available_width(),
            if hint.is_some() { 22.0 } else { 6.0 },
        ));
        if let Some(hint) = hint {
            ui.label(muted(hint).small());
        }
    });
    if let Some(item) = payload {
        ctx.actions.push(Action::Drop {
            item: (*item).clone(),
            parent: parent.to_vec(),
            index,
        });
    }
}
