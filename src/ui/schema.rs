//! JSON Schemas the Profiles page builds its forms from, generated from the Rust option structs.

use serde_json::{Map, Value, json};

use crate::profile::Profile;
use crate::widgets::gamepad::GamepadOptions;
use crate::widgets::keyboard::KeyboardOptions;
use crate::widgets::mouse::MouseOptions;
use crate::widgets::mouse_flick::FlickOptions;
use crate::widgets::mouse_movement::MovementOptions;
use crate::widgets::mouse_speed::SpeedOptions;

/// `{ profile, widgets: { <name>: schema } }`; widget names match `widgets::build`.
pub fn all() -> Value {
    json!({
        "profile": schemars::schema_for!(Profile),
        "widgets": {
            "keyboard": schemars::schema_for!(KeyboardOptions),
            "mouse": schemars::schema_for!(MouseOptions),
            "mouse-movement": schemars::schema_for!(MovementOptions),
            "mouse-speed": schemars::schema_for!(SpeedOptions),
            "mouse-flick": schemars::schema_for!(FlickOptions),
            "gamepad": schemars::schema_for!(GamepadOptions),
        },
    })
}

/// What control an option gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Number,
    Integer,
    Boolean,
    Enum,
    Color,
    Pair,
    String,
    Gradient,
    Keys,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct EnumOption {
    pub value: String,
    pub label: String,
    pub help: Option<String>,
}

/// One option of a profile or widget, ready for a form.
#[derive(Debug, Clone)]
pub struct FieldSpec {
    pub key: String,
    pub label: String,
    pub help: Option<String>,
    pub kind: FieldKind,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// May be left unset (Rust `Option`).
    pub optional: bool,
    pub default: Option<Value>,
    pub options: Vec<EnumOption>,
}

/// Field specs for the profile settings and every widget, in schema order.
pub struct Forms {
    /// Profile settings except `anchor` and `monitor` (they get their own controls).
    pub profile: Vec<FieldSpec>,
    pub widgets: Vec<(String, Vec<FieldSpec>)>,
}

impl Forms {
    pub fn build() -> Self {
        let schema = all();
        let widgets = schema["widgets"]
            .as_object()
            .map(|w| {
                w.iter()
                    .map(|(name, s)| (name.clone(), fields(s, &[])))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            profile: fields(&schema["profile"], &["anchor", "monitor"]),
            widgets,
        }
    }

    pub fn widget(&self, name: &str) -> Option<&[FieldSpec]> {
        self.widgets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, f)| f.as_slice())
    }

    pub fn profile_field(&self, key: &str) -> Option<&FieldSpec> {
        self.profile.iter().find(|f| f.key == key)
    }
}

/// Every property of an object schema, in schema order.
pub fn fields(schema: &Value, skip: &[&str]) -> Vec<FieldSpec> {
    let Some(props) = schema["properties"].as_object() else {
        return Vec::new();
    };
    props
        .iter()
        .filter(|(key, _)| !skip.contains(&key.as_str()))
        .map(|(key, prop)| describe(key, prop, schema))
        .collect()
}

/// `snake_case` or `kebab-case` → `Snake case`.
pub fn humanize(key: &str) -> String {
    let words = key.replace(['_', '-'], " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Rust f32 values arrive as e.g. 1.0049999952; shows them as typed (6 significant digits).
pub fn tidy(n: f64) -> f64 {
    if n == 0.0 || !n.is_finite() {
        return n;
    }
    let digits = 5 - n.abs().log10().floor() as i32;
    let k = 10f64.powi(digits);
    (n * k).round() / k
}

/// Follows `$ref: "#/$defs/Name"` within `root`; the node's own keys win.
fn resolve(node: &Value, root: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(name) = node["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#/$defs/"))
        && let Some(def) = root["$defs"][name].as_object()
    {
        out.extend(def.clone());
    }
    if let Some(own) = node.as_object() {
        out.extend(
            own.iter()
                .filter(|(k, _)| *k != "$ref")
                .map(|(k, v)| (k.clone(), v.clone())),
        );
    }
    out
}

fn types(node: &Map<String, Value>) -> Vec<&str> {
    match node.get("type") {
        Some(Value::Array(t)) => t.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(t)) => vec![t.as_str()],
        _ => Vec::new(),
    }
}

fn first_line(text: Option<&Value>) -> Option<String> {
    text.and_then(Value::as_str)
        .and_then(|t| t.lines().next())
        .map(str::to_owned)
}

fn describe(key: &str, prop: &Value, root: &Value) -> FieldSpec {
    let mut node = resolve(prop, root);
    let mut optional = types(&node).contains(&"null");

    // Option<T> with a $ref comes as anyOf: [T, null].
    if let Some(any_of) = node.get("anyOf").and_then(Value::as_array).cloned() {
        let branches: Vec<Map<String, Value>> = any_of.iter().map(|b| resolve(b, root)).collect();
        optional |= branches.iter().any(|b| types(b).contains(&"null"));
        if let Some(inner) = branches.into_iter().find(|b| !types(b).contains(&"null")) {
            let (description, default) = (
                node.get("description").cloned(),
                node.get("default").cloned(),
            );
            node = inner;
            for (k, v) in [("description", description), ("default", default)] {
                if let Some(v) = v {
                    node.insert(k.to_owned(), v);
                }
            }
        }
    }

    let mut spec = FieldSpec {
        key: key.to_owned(),
        label: humanize(key),
        help: first_line(prop.get("description").or(node.get("description"))),
        kind: FieldKind::Unknown,
        min: None,
        max: None,
        step: None,
        optional,
        default: prop.get("default").or(node.get("default")).cloned(),
        options: Vec::new(),
    };
    let t: Vec<&str> = types(&node).into_iter().filter(|t| *t != "null").collect();
    let format = node.get("format").and_then(Value::as_str);
    let option = |value: &str, help: Option<String>| EnumOption {
        value: value.to_owned(),
        label: humanize(value),
        help,
    };

    spec.kind = if format == Some("color") {
        FieldKind::Color
    } else if format == Some("key-list") {
        FieldKind::Keys
    } else if let Some(values) = node.get("enum").and_then(Value::as_array) {
        spec.options = values
            .iter()
            .filter_map(Value::as_str)
            .map(|v| option(v, None))
            .collect();
        FieldKind::Enum
    } else if let Some(branches) = node.get("oneOf").and_then(Value::as_array)
        && branches
            .iter()
            .all(|b| b.get("const").is_some() || b["enum"].is_array())
    {
        spec.options = branches
            .iter()
            .flat_map(|b| {
                let help = first_line(b.get("description"));
                let values: Vec<&str> = match b.get("const").and_then(Value::as_str) {
                    Some(c) => vec![c],
                    None => b["enum"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect(),
                };
                values
                    .into_iter()
                    .map(move |v| option(v, help.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        FieldKind::Enum
    } else if t.contains(&"array") {
        let items = resolve(node.get("items").unwrap_or(&Value::Null), root);
        let item_props = items.get("properties").and_then(Value::as_object);
        if item_props.is_some_and(|p| p.contains_key("at") && p.contains_key("rgb")) {
            FieldKind::Gradient
        } else if node.get("minItems") == Some(&json!(2)) && node.get("maxItems") == Some(&json!(2))
        {
            FieldKind::Pair
        } else {
            FieldKind::Unknown
        }
    } else if t.contains(&"integer") {
        FieldKind::Integer
    } else if t.contains(&"number") {
        FieldKind::Number
    } else if t.contains(&"boolean") {
        FieldKind::Boolean
    } else if t.contains(&"string") {
        FieldKind::String
    } else {
        FieldKind::Unknown
    };

    if matches!(spec.kind, FieldKind::Number | FieldKind::Integer) {
        let num = |a: &str, b: &str| node.get(a).or(node.get(b)).and_then(Value::as_f64);
        spec.min = num("minimum", "exclusiveMinimum");
        spec.max = num("maximum", "exclusiveMaximum");
        spec.step = match (spec.kind, spec.min, spec.max) {
            (FieldKind::Integer, ..) => Some(1.0),
            // About 100–1000 slider positions across the range.
            (_, Some(lo), Some(hi)) if hi > lo => {
                Some(10f64.powi((hi - lo).log10().floor() as i32 - 2))
            }
            _ => None,
        };
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_kinds_follow_the_option_types() {
        let forms = Forms::build();
        let kind = |widget: &str, key: &str| {
            forms
                .widget(widget)
                .unwrap()
                .iter()
                .find(|f| f.key == key)
                .unwrap()
                .kind
        };
        assert_eq!(kind("keyboard", "preset"), FieldKind::Enum);
        assert_eq!(kind("keyboard", "hidden_keys"), FieldKind::Keys);
        assert_eq!(kind("mouse", "scroll_color"), FieldKind::Color);
        assert_eq!(kind("mouse-movement", "gradient"), FieldKind::Gradient);
        assert_eq!(kind("mouse-movement", "element_size"), FieldKind::Pair);
        assert_eq!(kind("mouse-movement", "elements"), FieldKind::Integer);
        assert_eq!(kind("mouse-flick", "text"), FieldKind::String);
        assert_eq!(kind("mouse", "scroll_highlight"), FieldKind::Boolean);

        let opacity = forms.profile_field("opacity").unwrap();
        assert_eq!(
            (opacity.kind, opacity.min, opacity.max),
            (FieldKind::Number, Some(0.0), Some(1.0))
        );
        assert_eq!(opacity.step, Some(0.01));
        assert!(forms.profile_field("anchor").is_none(), "own control");
        let highlight = forms.profile_field("highlight_opacity").unwrap();
        assert!(highlight.optional);
        assert_eq!(forms.profile_field("offset").unwrap().kind, FieldKind::Pair);

        let preset = forms
            .widget("keyboard")
            .unwrap()
            .iter()
            .find(|f| f.key == "preset")
            .unwrap();
        let values: Vec<&str> = preset.options.iter().map(|o| o.value.as_str()).collect();
        assert_eq!(values, ["default", "full"]);
        assert!(preset.options[0].help.is_some());
    }

    #[test]
    fn labels_and_numbers_read_well() {
        assert_eq!(humanize("scroll_color"), "Scroll color");
        assert_eq!(humanize("mouse-movement"), "Mouse movement");
        assert_eq!(tidy(1.004_999_995_2), 1.005);
        assert_eq!(tidy(0.600_000_023_8), 0.6);
        assert_eq!(tidy(1234.5), 1234.5);
    }

    #[test]
    fn every_buildable_widget_has_a_schema() {
        let schema = all();
        let widgets = schema["widgets"].as_object().unwrap();
        for name in crate::widgets::KNOWN_WIDGETS.split(", ") {
            assert!(widgets.contains_key(name), "{name} has no schema");
        }
        assert_eq!(
            widgets.len(),
            crate::widgets::KNOWN_WIDGETS.split(", ").count()
        );
    }

    #[test]
    fn every_option_has_a_default() {
        let schema = all();
        let widgets = schema["widgets"].as_object().unwrap();
        for (name, widget) in widgets {
            for (field, prop) in widget["properties"].as_object().unwrap() {
                let optional = prop["type"]
                    .as_array()
                    .is_some_and(|t| t.contains(&"null".into()));
                assert!(
                    optional || prop.get("default").is_some(),
                    "{name}.{field} has no default (missing Serialize?)"
                );
            }
        }
        assert!(
            schema["profile"]["properties"]["anchor"]
                .get("default")
                .is_some()
        );
    }

    #[test]
    fn schema_carries_docs_ranges_and_color_hints() {
        let schema = all();
        let opacity = &schema["profile"]["properties"]["opacity"];
        assert_eq!(opacity["maximum"], 1.0);
        assert!(opacity["description"].as_str().unwrap().contains("opacity"));
        assert!(
            schema["profile"]["properties"].get("layout").is_none(),
            "layout is edited separately"
        );
        let flick = &schema["widgets"]["mouse-flick"]["properties"];
        assert_eq!(flick["glow_color"]["format"], "color");
        assert_eq!(flick["threshold"]["default"], 1000.0);
    }
}
