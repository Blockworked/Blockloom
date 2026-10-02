//! What a package contributes, and checking a payload against it.
//!
//! A contribution is data: a component, a project resource, a block, a
//! command. A schema says which fields a payload has and their types; the
//! host validates every payload against it before it reaches a document, a
//! run or a build, so plugin code never sees a malformed record.

use crate::id::validate_type_id;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// The type of one field, and the bounds a value must be inside.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldType {
    Bool,
    Int {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Number {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<f64>,
    },
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_len: Option<usize>,
    },
    /// `#RRGGBB` or `#RRGGBBAA`.
    Color,
    /// Three numbers.
    Vec3,
    Choice {
        options: Vec<String>,
    },
    /// A project-relative asset path (`assets/...`) of the given kind
    /// (`image`, `model`, `sound`, `any`, ...). Enumerated as a reference.
    Asset {
        #[serde(default = "any_kind")]
        kind: String,
    },
    /// An actor id in the same project.
    Actor,
    List {
        item: Box<FieldType>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_len: Option<usize>,
    },
}

fn any_kind() -> String {
    "any".to_string()
}

/// One named, typed value slot: a component field, a block slot or a command
/// argument.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSchema {
    pub name: String,
    #[serde(flatten)]
    pub ty: FieldType,
    /// What an absent value reads as. A field with none must be given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// How the inspector draws it. Nothing here changes what a payload holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui: Option<FieldUi>,
}

/// Hints for drawing one field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldUi {
    /// Shown instead of the field's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget: Option<Widget>,
    /// Shown after the value, such as `m` or `%`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// A slider's step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<f64>,
    /// The field is drawn only while this holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Widget {
    /// A bounded `int` or `number` as a slider.
    Slider,
    /// A `text` field as a box of several lines.
    Multiline,
}

/// A test on another field of the same record. With neither `equals` nor
/// `not_equals` it holds while the field is truthy (true, a non-zero number
/// or a non-empty text or list).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_equals: Option<Value>,
}

/// Fields drawn together under one heading.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectorGroup {
    #[serde(default)]
    pub label: String,
    pub fields: Vec<String>,
    /// Starts folded away.
    #[serde(default)]
    pub collapsed: bool,
}

/// How a component's inspector card is laid out. A field no group names is
/// drawn first, ungrouped, in schema order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InspectorLayout {
    #[serde(default)]
    pub groups: Vec<InspectorGroup>,
}

/// Something in a payload that points outside it, so the host can find what a
/// record keeps alive (unused-asset pruning, build closure, rename follow).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Reference {
    pub kind: ReferenceKind,
    pub value: String,
    /// The field path the reference sits at, e.g. `targets[1]`.
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Asset,
    Actor,
}

/// One thing wrong with a payload, located by field path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaError {
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.at, self.message)
        }
    }
}

fn err(at: &str, message: impl Into<String>) -> SchemaError {
    SchemaError {
        at: at.to_string(),
        message: message.into(),
    }
}

impl FieldType {
    /// The value a field of this type reads as when nothing says otherwise.
    pub fn zero(&self) -> Value {
        match self {
            FieldType::Bool => json!(false),
            FieldType::Int { min, max } => json!(clamp_zero(*min, *max)),
            FieldType::Number { min, max } => json!(clamp_zero(*min, *max)),
            FieldType::Text { .. } => json!(""),
            FieldType::Color => json!("#FFFFFF"),
            FieldType::Vec3 => json!([0.0, 0.0, 0.0]),
            FieldType::Choice { options } => json!(options.first().cloned().unwrap_or_default()),
            FieldType::Asset { .. } | FieldType::Actor => json!(""),
            FieldType::List { .. } => json!([]),
        }
    }

    fn check(&self, at: &str, value: &Value, errors: &mut Vec<SchemaError>) {
        match self {
            FieldType::Bool => {
                if !value.is_boolean() {
                    errors.push(err(at, "expected true or false"));
                }
            }
            FieldType::Int { min, max } => match value.as_i64() {
                Some(n) => {
                    if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                        errors.push(err(at, format!("{n} is outside {}", range(min, max))));
                    }
                }
                None => errors.push(err(at, "expected a whole number")),
            },
            FieldType::Number { min, max } => match value.as_f64() {
                Some(n) if n.is_finite() => {
                    if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                        errors.push(err(at, format!("{n} is outside {}", range(min, max))));
                    }
                }
                _ => errors.push(err(at, "expected a finite number")),
            },
            FieldType::Text { max_len } => match value.as_str() {
                Some(s) => {
                    if max_len.is_some_and(|m| s.chars().count() > m) {
                        errors.push(err(at, "text is too long"));
                    }
                }
                None => errors.push(err(at, "expected text")),
            },
            FieldType::Color => match value.as_str() {
                Some(s) if is_color(s) => {}
                _ => errors.push(err(at, "expected a color like #RRGGBB")),
            },
            FieldType::Vec3 => match value.as_array() {
                Some(a)
                    if a.len() == 3 && a.iter().all(|n| n.as_f64().is_some_and(f64::is_finite)) => {
                }
                _ => errors.push(err(at, "expected three numbers")),
            },
            FieldType::Choice { options } => match value.as_str() {
                Some(s) if options.iter().any(|o| o == s) => {}
                _ => errors.push(err(at, format!("expected one of {}", options.join(", ")))),
            },
            FieldType::Asset { .. } => match value.as_str() {
                Some("") => {}
                Some(s) if crate::id::validate_package_path(s).is_ok() => {}
                _ => errors.push(err(at, "expected a project-relative asset path")),
            },
            FieldType::Actor => {
                if !value.is_string() {
                    errors.push(err(at, "expected an actor id"));
                }
            }
            FieldType::List { item, max_len } => match value.as_array() {
                Some(items) => {
                    if max_len.is_some_and(|m| items.len() > m) {
                        errors.push(err(at, "too many items"));
                    }
                    for (i, v) in items.iter().enumerate() {
                        item.check(&format!("{at}[{i}]"), v, errors);
                    }
                }
                None => errors.push(err(at, "expected a list")),
            },
        }
    }

    fn collect(&self, at: &str, value: &Value, out: &mut BTreeSet<Reference>) {
        match (self, value) {
            (FieldType::Asset { .. }, Value::String(s)) if !s.is_empty() => {
                out.insert(Reference {
                    kind: ReferenceKind::Asset,
                    value: s.clone(),
                    at: at.to_string(),
                });
            }
            (FieldType::Actor, Value::String(s)) if !s.is_empty() => {
                out.insert(Reference {
                    kind: ReferenceKind::Actor,
                    value: s.clone(),
                    at: at.to_string(),
                });
            }
            (FieldType::List { item, .. }, Value::Array(items)) => {
                for (i, v) in items.iter().enumerate() {
                    item.collect(&format!("{at}[{i}]"), v, out);
                }
            }
            _ => {}
        }
    }

    fn check_definition(&self, at: &str) -> Result<(), String> {
        match self {
            FieldType::Int {
                min: Some(a),
                max: Some(b),
            } if a > b => Err(format!("{at}: min is above max")),
            FieldType::Number {
                min: Some(a),
                max: Some(b),
            } if a > b => Err(format!("{at}: min is above max")),
            FieldType::Choice { options } if options.is_empty() => {
                Err(format!("{at}: a choice needs options"))
            }
            FieldType::List { item, .. } => item.check_definition(at),
            _ => Ok(()),
        }
    }
}

/// Zero, pulled inside whichever bound excludes it.
fn clamp_zero<T: Default + PartialOrd + Copy>(min: Option<T>, max: Option<T>) -> T {
    let zero = T::default();
    match (min, max) {
        (Some(m), _) if zero < m => m,
        (_, Some(m)) if zero > m => m,
        _ => zero,
    }
}

fn range<T: std::fmt::Display>(min: &Option<T>, max: &Option<T>) -> String {
    match (min, max) {
        (Some(a), Some(b)) => format!("{a}..{b}"),
        (Some(a), None) => format!(">= {a}"),
        (None, Some(b)) => format!("<= {b}"),
        (None, None) => "any".to_string(),
    }
}

fn is_color(s: &str) -> bool {
    let hex = s.strip_prefix('#').unwrap_or("");
    (hex.len() == 6 || hex.len() == 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

impl FieldSchema {
    /// The value this field takes when a payload leaves it out.
    pub fn default_value(&self) -> Value {
        self.default.clone().unwrap_or_else(|| self.ty.zero())
    }
}

fn check_fields(fields: &[FieldSchema], what: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for field in fields {
        validate_type_id(&field.name).map_err(|e| format!("{what} field {e}"))?;
        if !seen.insert(field.name.as_str()) {
            return Err(format!("{what} has two fields named {}", field.name));
        }
        field
            .ty
            .check_definition(&format!("{what}.{}", field.name))?;
        if let Some(default) = &field.default {
            let mut errors = Vec::new();
            field.ty.check(&field.name, default, &mut errors);
            if let Some(e) = errors.first() {
                return Err(format!("{what} default is invalid: {e}"));
            }
        }
    }
    Ok(())
}

/// Checks the drawing hints name real fields and suit their types.
fn check_ui(
    fields: &[FieldSchema],
    layout: Option<&InspectorLayout>,
    what: &str,
) -> Result<(), String> {
    let names: BTreeSet<&str> = fields.iter().map(|f| f.name.as_str()).collect();
    for field in fields {
        let Some(ui) = &field.ui else { continue };
        let at = format!("{what}.{}", field.name);
        match (&ui.widget, &field.ty) {
            (None, _) => {}
            (
                Some(Widget::Slider),
                FieldType::Int {
                    min: Some(_),
                    max: Some(_),
                },
            ) => {}
            (
                Some(Widget::Slider),
                FieldType::Number {
                    min: Some(_),
                    max: Some(_),
                },
            ) => {}
            (Some(Widget::Slider), _) => {
                return Err(format!(
                    "{at}: a slider needs an int or number with min and max"
                ));
            }
            (Some(Widget::Multiline), FieldType::Text { .. }) => {}
            (Some(Widget::Multiline), _) => {
                return Err(format!("{at}: a multiline box needs a text field"));
            }
        }
        if let Some(step) = ui.step
            && !(step.is_finite() && step > 0.0)
        {
            return Err(format!("{at}: step must be above zero"));
        }
        if let Some(condition) = &ui.visible_when {
            if condition.field == field.name {
                return Err(format!("{at}: visible_when can't name the field itself"));
            }
            if !names.contains(condition.field.as_str()) {
                return Err(format!(
                    "{at}: visible_when names unknown field {}",
                    condition.field
                ));
            }
            if condition.equals.is_some() && condition.not_equals.is_some() {
                return Err(format!(
                    "{at}: visible_when takes equals or not_equals, not both"
                ));
            }
        }
    }
    if let Some(layout) = layout {
        let mut placed = BTreeSet::new();
        for group in &layout.groups {
            for name in &group.fields {
                if !names.contains(name.as_str()) {
                    return Err(format!(
                        "{what}: inspector group \"{}\" names unknown field {name}",
                        group.label
                    ));
                }
                if !placed.insert(name.as_str()) {
                    return Err(format!("{what}: inspector draws {name} twice"));
                }
            }
        }
    }
    Ok(())
}

/// Checks `payload` (an object) against `fields`. Unknown fields are
/// refused: a typo should not be stored silently.
fn validate_object(fields: &[FieldSchema], payload: &Value) -> Vec<SchemaError> {
    let mut errors = Vec::new();
    let Some(object) = payload.as_object() else {
        return vec![err("", "expected an object")];
    };
    for key in object.keys() {
        if !fields.iter().any(|f| &f.name == key) {
            errors.push(err(key, "unknown field"));
        }
    }
    for field in fields {
        match object.get(&field.name) {
            Some(value) => field.ty.check(&field.name, value, &mut errors),
            None if field.default.is_none() => errors.push(err(&field.name, "missing")),
            None => {}
        }
    }
    errors
}

fn fill_defaults(fields: &[FieldSchema], payload: &Value) -> Value {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    for field in fields {
        object
            .entry(field.name.clone())
            .or_insert_with(|| field.default_value());
    }
    Value::Object(object)
}

/// One step of an upgrade from a component's schema version `from` to
/// `from + 1`, written as data so it can run on a copy of the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MigrationStep {
    pub from: u32,
    pub ops: Vec<MigrationOp>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MigrationOp {
    RenameField {
        from: String,
        to: String,
    },
    RemoveField {
        name: String,
    },
    /// Adds `name` with `value` unless the payload already has it.
    SetDefault {
        name: String,
        value: Value,
    },
}

impl MigrationOp {
    fn apply(&self, object: &mut Map<String, Value>) {
        match self {
            MigrationOp::RenameField { from, to } => {
                if let Some(value) = object.remove(from) {
                    object.insert(to.clone(), value);
                }
            }
            MigrationOp::RemoveField { name } => {
                object.remove(name);
            }
            MigrationOp::SetDefault { name, value } => {
                object.entry(name.clone()).or_insert_with(|| value.clone());
            }
        }
    }
}

/// A component an actor can carry, or (with [`Contributions::resources`]) a
/// record the project itself holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentSchema {
    pub type_id: String,
    #[serde(default)]
    pub display_name: String,
    /// The version of this schema. Records carry the version they were
    /// written at; a newer one than the package knows is refused.
    #[serde(default = "one")]
    pub version: u32,
    pub fields: Vec<FieldSchema>,
    /// True when nothing at run time reads it: a missing plugin then does
    /// not stop Play or a build.
    #[serde(default)]
    pub editor_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub migrations: Vec<MigrationStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspector: Option<InspectorLayout>,
}

fn one() -> u32 {
    1
}

impl ComponentSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.type_id)?;
        if self.version == 0 {
            return Err(format!("{}: schema versions start at 1", self.type_id));
        }
        check_fields(&self.fields, &self.type_id)?;
        check_ui(&self.fields, self.inspector.as_ref(), &self.type_id)?;
        let mut from = BTreeSet::new();
        for step in &self.migrations {
            if step.from == 0 || step.from >= self.version {
                return Err(format!(
                    "{}: migration from {} does not lead to a version up to {}",
                    self.type_id, step.from, self.version
                ));
            }
            if !from.insert(step.from) {
                return Err(format!(
                    "{}: two migrations from {}",
                    self.type_id, step.from
                ));
            }
        }
        for v in 1..self.version {
            if !from.contains(&v) {
                // A gap is allowed only when nobody can hold that version:
                // authors declare every step they ever shipped.
                return Err(format!(
                    "{}: no migration from schema version {v} to {}",
                    self.type_id,
                    v + 1
                ));
            }
        }
        Ok(())
    }

    /// The payload a freshly added component starts with.
    pub fn defaults(&self) -> Value {
        fill_defaults(&self.fields, &json!({}))
    }

    pub fn validate(&self, payload: &Value) -> Result<(), Vec<SchemaError>> {
        let errors = validate_object(&self.fields, payload);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// `payload` with every absent defaulted field filled in.
    pub fn normalize(&self, payload: &Value) -> Value {
        fill_defaults(&self.fields, payload)
    }

    pub fn references(&self, payload: &Value) -> Vec<Reference> {
        let mut out = BTreeSet::new();
        if let Some(object) = payload.as_object() {
            for field in &self.fields {
                if let Some(value) = object.get(&field.name) {
                    field.ty.collect(&field.name, value, &mut out);
                }
            }
        }
        out.into_iter().collect()
    }

    /// Runs the declared steps that take a payload written at `from` up to
    /// this schema's version. The caller validates the result.
    pub fn migrate(&self, from: u32, payload: &Value) -> Result<Value, String> {
        if from > self.version {
            return Err(format!(
                "{} was written at schema version {from}, newer than this package's {}",
                self.type_id, self.version
            ));
        }
        let mut object = payload
            .as_object()
            .cloned()
            .ok_or_else(|| "payload is not an object".to_string())?;
        for version in from..self.version {
            let step = self
                .migrations
                .iter()
                .find(|s| s.from == version)
                .ok_or_else(|| format!("{}: no migration from {version}", self.type_id))?;
            for op in &step.ops {
                op.apply(&mut object);
            }
        }
        Ok(Value::Object(object))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Statement,
    Reporter,
    Hat,
}

/// A block a package adds to the palette. `label` names its slots in braces:
/// `"set {name} charge to {amount}"`. Statements and reporters run the
/// command named by `command`; the bridge into the VM is the host's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockSchema {
    pub type_id: String,
    pub kind: BlockKind,
    pub category: String,
    pub label: String,
    #[serde(default)]
    pub slots: Vec<FieldSchema>,
    /// What a reporter answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<FieldType>,
    /// The command a statement or reporter runs. A hat names the event it
    /// starts on instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
}

impl BlockSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.type_id)?;
        check_fields(&self.slots, &self.type_id)?;
        let mut rest = self.label.as_str();
        let mut named = BTreeSet::new();
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                return Err(format!("{}: unclosed '{{' in label", self.type_id));
            };
            let name = &rest[open + 1..open + close];
            if !self.slots.iter().any(|s| s.name == name) {
                return Err(format!("{}: label names unknown slot {name}", self.type_id));
            }
            named.insert(name.to_string());
            rest = &rest[open + close + 1..];
        }
        for slot in &self.slots {
            if !named.contains(&slot.name) {
                return Err(format!(
                    "{}: slot {} is not in the label",
                    self.type_id, slot.name
                ));
            }
        }
        match self.kind {
            BlockKind::Reporter if self.returns.is_none() => Err(format!(
                "{}: a reporter must say what it returns",
                self.type_id
            )),
            BlockKind::Hat if self.event.is_none() => {
                Err(format!("{}: a hat must name its event", self.type_id))
            }
            BlockKind::Statement | BlockKind::Reporter if self.command.is_none() => {
                Err(format!("{}: needs a command to run", self.type_id))
            }
            _ => Ok(()),
        }
    }
}

/// What a declared command does when the host runs it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum CommandAction {
    /// Adds (or replaces) a component on the actor named by the `actor`
    /// argument, its payload the remaining arguments.
    AddComponent { component: String },
    /// Sets one field of a component on the actor named by `actor`, from the
    /// `value` argument.
    SetField { component: String, field: String },
    /// Replaces a project resource with the arguments as its payload.
    SetResource { resource: String },
    /// Sets one field of a project resource from the `value` argument, or
    /// with `append` adds `value` to the end of that field's list. With a
    /// `template` the value is that text with each `{arg}` replaced by the
    /// argument of that name, and no `value` argument is needed.
    SetResourceField {
        resource: String,
        field: String,
        #[serde(default)]
        append: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        template: Option<String>,
    },
    /// Calls `op` on the package's native or portable module.
    Module { op: String },
}

/// The `{name}` placeholders in a command template, in order.
pub fn template_names(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        names.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    names
}

/// Fills a command template from its arguments: text as it is, whole numbers
/// without a fraction. An argument the template names but `args` lacks is an
/// error.
pub fn fill_template(template: &str, args: &Value) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let name = &after[..close];
        out.push_str(&rest[..open]);
        match &args[name] {
            Value::Null => return Err(format!("the template needs {name}")),
            Value::String(s) => out.push_str(s),
            Value::Number(n) => match n.as_f64() {
                Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => {
                    out.push_str(&(f as i64).to_string())
                }
                _ => out.push_str(&n.to_string()),
            },
            other => out.push_str(&other.to_string()),
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A backend command a package adds. QML, the shell and MCP all find it
/// through the registry as `plugin-id/name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandSchema {
    pub name: String,
    pub summary: String,
    #[serde(default)]
    pub args: Vec<FieldSchema>,
    pub action: CommandAction,
}

impl CommandSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        check_fields(&self.args, &self.name)?;
        let needs = |name: &str| self.args.iter().any(|a| a.name == name);
        match &self.action {
            CommandAction::AddComponent { .. } | CommandAction::SetField { .. }
                if !needs("actor") =>
            {
                Err(format!("{}: needs an actor argument", self.name))
            }
            CommandAction::SetResourceField {
                template: Some(template),
                ..
            } => {
                for name in template_names(template) {
                    if !needs(&name) {
                        return Err(format!(
                            "{}: the template names {name}, which is not an argument",
                            self.name
                        ));
                    }
                }
                Ok(())
            }
            CommandAction::SetField { .. } | CommandAction::SetResourceField { .. }
                if !needs("value") =>
            {
                Err(format!("{}: needs a value argument", self.name))
            }
            _ => Ok(()),
        }
    }
}

/// The named points in a frame where plugin code may run, in order. A hook
/// sees the snapshot its stage documents and submits effects; it never edits
/// authored data directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Input,
    PreSimulation,
    FixedSimulation,
    EffectApplication,
    PostPhysics,
    RenderExtraction,
    Presentation,
}

impl Stage {
    pub const ALL: [Stage; 7] = [
        Stage::Input,
        Stage::PreSimulation,
        Stage::FixedSimulation,
        Stage::EffectApplication,
        Stage::PostPhysics,
        Stage::RenderExtraction,
        Stage::Presentation,
    ];
}

/// A runtime hook a package registers, with ordering against others in the
/// same stage. `before`/`after` name hooks as `name` (this package's) or
/// `plugin/name`; naming a plugin that is not active is ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookSchema {
    pub name: String,
    pub stage: Stage,
    #[serde(default)]
    pub before: Vec<String>,
    #[serde(default)]
    pub after: Vec<String>,
}

fn default_limit_ms() -> u32 {
    crate::assets::DEFAULT_LIMIT_MS
}

/// An importer a package adds: files with one of its extensions become
/// other project files. The module answers op `importer.<name>` (see
/// [`crate::assets`]); the host does the reading and the writing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImporterSchema {
    pub name: String,
    pub summary: String,
    /// Lowercase, without the dot.
    pub extensions: Vec<String>,
    /// How long one import may work, in milliseconds (a portable module is
    /// stopped when it spends it).
    #[serde(default = "default_limit_ms")]
    pub limit_ms: u32,
}

/// A hook a package runs when a game is built, before it is packed: it may
/// refuse the build and may add files to the game. The module answers op
/// `build.<name>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuildHookSchema {
    pub name: String,
    pub summary: String,
    #[serde(default = "default_limit_ms")]
    pub limit_ms: u32,
}

fn check_limit(name: &str, limit_ms: u32) -> Result<(), String> {
    if limit_ms == 0 || limit_ms > crate::assets::MAX_LIMIT_MS {
        return Err(format!(
            "{name}: limit_ms must be between 1 and {}",
            crate::assets::MAX_LIMIT_MS
        ));
    }
    Ok(())
}

impl ImporterSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        check_limit(&self.name, self.limit_ms)?;
        if self.extensions.is_empty() {
            return Err(format!("{}: an importer needs an extension", self.name));
        }
        let mut seen = BTreeSet::new();
        for extension in &self.extensions {
            let ok = (1..=16).contains(&extension.len())
                && extension
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
            if !ok {
                return Err(format!(
                    "{}: \"{extension}\" must be 1-16 lowercase letters or digits, without the dot",
                    self.name
                ));
            }
            if !seen.insert(extension.as_str()) {
                return Err(format!("{}: lists .{extension} twice", self.name));
            }
        }
        Ok(())
    }

    pub fn handles(&self, extension: &str) -> bool {
        self.extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case(extension))
    }
}

impl BuildHookSchema {
    pub fn check_definition(&self) -> Result<(), String> {
        validate_type_id(&self.name)?;
        check_limit(&self.name, self.limit_ms)
    }
}

/// Everything one package contributes, as read from its `schemas/` files.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Contributions {
    #[serde(default)]
    pub hooks: Vec<HookSchema>,
    #[serde(default)]
    pub importers: Vec<ImporterSchema>,
    #[serde(default, rename = "build")]
    pub build_hooks: Vec<BuildHookSchema>,
    #[serde(default)]
    pub components: Vec<ComponentSchema>,
    #[serde(default)]
    pub resources: Vec<ComponentSchema>,
    #[serde(default)]
    pub blocks: Vec<BlockSchema>,
    #[serde(default)]
    pub commands: Vec<CommandSchema>,
    #[serde(default)]
    pub panels: Vec<PanelSchema>,
    #[serde(default)]
    pub tools: Vec<ToolSchema>,
}

/// A scene-view tool a package adds. A click casts the pointer's ray through
/// the module's `cast` op (arguments `x y z dx dy dz reach`, answer `hit`
/// plus whatever else it reports) and, on a hit, runs `command` with `args`
/// resolved against the answer and the tool's option values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The module op the pointer's ray goes to.
    pub cast: String,
    /// How far along the ray to look, in world units.
    #[serde(default = "default_reach")]
    pub reach: f64,
    /// The command a hit runs.
    pub command: String,
    /// The command's arguments: `$hit.path` reads the cast's answer
    /// (`$hit.cell.0` is a list's first item), `$option.name` an option, and
    /// anything else is a literal text.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
    /// What the tool asks for in the editor before it is used.
    #[serde(default)]
    pub options: Vec<FieldSchema>,
    /// Path into the cast's answer of the box the editor outlines under the
    /// pointer and along a stroke: six numbers, min then max, in world units.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outline: String,
    /// Holding the button and dragging keeps casting, and the whole stroke
    /// runs on release as one undo step.
    #[serde(default)]
    pub drag: bool,
}

fn default_reach() -> f64 {
    64.0
}

/// The most a tool's reach may be.
pub const MAX_TOOL_REACH: f64 = 100_000.0;

impl ToolSchema {
    fn check_definition(&self, all: &Contributions) -> Result<(), String> {
        validate_type_id(&self.name)?;
        let name = &self.name;
        if self.title.trim().is_empty() {
            return Err(format!("tool {name}: needs a title"));
        }
        if self.cast.trim().is_empty() {
            return Err(format!("tool {name}: needs a cast op"));
        }
        if !(self.reach > 0.0 && self.reach <= MAX_TOOL_REACH) {
            return Err(format!("tool {name}: reach is 0 to {MAX_TOOL_REACH}"));
        }
        check_fields(&self.options, name)?;
        if self.outline.starts_with('$') {
            return Err(format!(
                "tool {name}: outline is a path into the hit, not a $ source"
            ));
        }
        let Some(command) = all.command(&self.command) else {
            return Err(format!("tool {name}: unknown command {}", self.command));
        };
        for (arg, source) in &self.args {
            if !command.args.iter().any(|a| &a.name == arg) {
                return Err(format!(
                    "tool {name}: {} has no argument {arg}",
                    self.command
                ));
            }
            if let Some(path) = source.strip_prefix("$option.")
                && !self.options.iter().any(|o| o.name == path)
            {
                return Err(format!("tool {name}: {source} names no option"));
            }
            if source.starts_with('$')
                && !source.starts_with("$hit.")
                && !source.starts_with("$option.")
            {
                return Err(format!(
                    "tool {name}: {source} reads neither hit nor option"
                ));
            }
        }
        Ok(())
    }

    /// The command's arguments for one hit: each `$` source read from the
    /// cast's answer or the option values, the rest passed as text. The
    /// options a caller leaves out take their defaults.
    pub fn resolve_args(&self, hit: &Value, options: &Value) -> Result<Value, String> {
        let mut out = serde_json::Map::new();
        for (arg, source) in &self.args {
            let value = if let Some(path) = source.strip_prefix("$hit.") {
                dig(hit, path).ok_or_else(|| format!("the hit has no {path}"))?
            } else if let Some(path) = source.strip_prefix("$option.") {
                match options.get(path).filter(|v| !v.is_null()) {
                    Some(v) => v.clone(),
                    None => self
                        .options
                        .iter()
                        .find(|o| o.name == path)
                        .and_then(|o| o.default.clone())
                        .ok_or_else(|| format!("the option {path} has no value"))?,
                }
            } else {
                Value::String(source.clone())
            };
            out.insert(arg.clone(), value);
        }
        Ok(Value::Object(out))
    }
}

/// The six numbers (min then max) a tool's `outline` path names in a hit.
pub fn box_at(hit: &Value, path: &str) -> Option<[f64; 6]> {
    if path.is_empty() {
        return None;
    }
    let list = dig(hit, path)?;
    let list = list.as_array()?;
    if list.len() != 6 {
        return None;
    }
    let mut out = [0.0; 6];
    for (slot, value) in out.iter_mut().zip(list) {
        let n = value.as_f64()?;
        if !n.is_finite() {
            return None;
        }
        *slot = n;
    }
    Some(out)
}

/// Reads `a.b.0` out of a value: a key of an object or an index of a list.
pub fn dig(value: &Value, path: &str) -> Option<Value> {
    let mut at = value;
    for step in path.split('.') {
        at = match at {
            Value::Array(list) => list.get(step.parse::<usize>().ok()?)?,
            other => other.get(step)?,
        };
    }
    Some(at.clone())
}

/// An editor panel a package adds, drawn by the editor from this list so a
/// plugin needs no UI code: text, a form for one of its resources, and
/// buttons that run its commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PanelSchema {
    pub name: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub items: Vec<PanelItem>,
}

/// The most items one panel may hold.
pub const MAX_PANEL_ITEMS: usize = 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PanelItem {
    /// A paragraph.
    Text { text: String },
    /// A form for one of the package's resources.
    Resource { resource: String },
    /// A form for a command's arguments with a button that runs it.
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
    },
}

impl PanelSchema {
    fn check_definition(&self, all: &Contributions) -> Result<(), String> {
        validate_type_id(&self.name)?;
        if self.title.trim().is_empty() {
            return Err(format!("panel {}: needs a title", self.name));
        }
        if self.items.is_empty() || self.items.len() > MAX_PANEL_ITEMS {
            return Err(format!(
                "panel {}: holds 1 to {MAX_PANEL_ITEMS} items, not {}",
                self.name,
                self.items.len()
            ));
        }
        for item in &self.items {
            match item {
                PanelItem::Text { .. } => {}
                PanelItem::Resource { resource } if all.resource(resource).is_none() => {
                    return Err(format!("panel {}: unknown resource {resource}", self.name));
                }
                PanelItem::Command { command, .. } if all.command(command).is_none() => {
                    return Err(format!("panel {}: unknown command {command}", self.name));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Contributions {
    /// Folds another schema file into this one.
    pub fn merge(&mut self, other: Contributions) {
        self.components.extend(other.components);
        self.resources.extend(other.resources);
        self.blocks.extend(other.blocks);
        self.commands.extend(other.commands);
        self.panels.extend(other.panels);
        self.tools.extend(other.tools);
        self.hooks.extend(other.hooks);
        self.importers.extend(other.importers);
        self.build_hooks.extend(other.build_hooks);
    }

    pub fn component(&self, type_id: &str) -> Option<&ComponentSchema> {
        self.components.iter().find(|c| c.type_id == type_id)
    }

    pub fn resource(&self, type_id: &str) -> Option<&ComponentSchema> {
        self.resources.iter().find(|c| c.type_id == type_id)
    }

    /// Either kind of record schema.
    pub fn record(&self, type_id: &str) -> Option<&ComponentSchema> {
        self.component(type_id).or_else(|| self.resource(type_id))
    }

    pub fn command(&self, name: &str) -> Option<&CommandSchema> {
        self.commands.iter().find(|c| c.name == name)
    }

    pub fn check_definition(&self) -> Result<(), String> {
        let mut records = BTreeSet::new();
        for schema in self.components.iter().chain(&self.resources) {
            schema.check_definition()?;
            if !records.insert(schema.type_id.as_str()) {
                return Err(format!("two record types named {}", schema.type_id));
            }
        }
        let mut names = BTreeSet::new();
        for block in &self.blocks {
            block.check_definition()?;
            if !names.insert(block.type_id.as_str()) {
                return Err(format!("two blocks named {}", block.type_id));
            }
            if let Some(command) = &block.command
                && self.command(command).is_none()
            {
                return Err(format!("{}: unknown command {command}", block.type_id));
            }
        }
        let mut hooks = BTreeSet::new();
        for hook in &self.hooks {
            validate_type_id(&hook.name)?;
            if !hooks.insert(hook.name.as_str()) {
                return Err(format!("two hooks named {}", hook.name));
            }
        }
        for hook in &self.hooks {
            for other in hook.before.iter().chain(&hook.after) {
                if !other.contains('/') && !hooks.contains(other.as_str()) {
                    return Err(format!(
                        "hook {} is ordered against unknown hook {other}",
                        hook.name
                    ));
                }
                if other == &hook.name {
                    return Err(format!("hook {} is ordered against itself", hook.name));
                }
            }
        }
        let mut importers = BTreeSet::new();
        for importer in &self.importers {
            importer.check_definition()?;
            if !importers.insert(importer.name.as_str()) {
                return Err(format!("two importers named {}", importer.name));
            }
        }
        let mut builds = BTreeSet::new();
        for hook in &self.build_hooks {
            hook.check_definition()?;
            if !builds.insert(hook.name.as_str()) {
                return Err(format!("two build hooks named {}", hook.name));
            }
        }
        let mut panels = BTreeSet::new();
        for panel in &self.panels {
            panel.check_definition(self)?;
            if !panels.insert(panel.name.as_str()) {
                return Err(format!("two panels named {}", panel.name));
            }
        }
        let mut tools = BTreeSet::new();
        for tool in &self.tools {
            tool.check_definition(self)?;
            if !tools.insert(tool.name.as_str()) {
                return Err(format!("two tools named {}", tool.name));
            }
        }
        let mut commands = BTreeSet::new();
        for command in &self.commands {
            command.check_definition()?;
            if !commands.insert(command.name.as_str()) {
                return Err(format!("two commands named {}", command.name));
            }
            match &command.action {
                CommandAction::AddComponent { component }
                | CommandAction::SetField { component, .. }
                    if self.component(component).is_none() =>
                {
                    return Err(format!("{}: unknown component {component}", command.name));
                }
                CommandAction::SetResource { resource } if self.resource(resource).is_none() => {
                    return Err(format!("{}: unknown resource {resource}", command.name));
                }
                CommandAction::SetResourceField {
                    resource,
                    field,
                    append,
                    ..
                } => {
                    let schema = self
                        .resource(resource)
                        .ok_or_else(|| format!("{}: unknown resource {resource}", command.name))?;
                    let Some(target) = schema.fields.iter().find(|f| &f.name == field) else {
                        return Err(format!("{}: {resource} has no field {field}", command.name));
                    };
                    if *append && !matches!(target.ty, FieldType::List { .. }) {
                        return Err(format!(
                            "{}: {resource}.{field} is not a list, so nothing can be appended",
                            command.name
                        ));
                    }
                }
                CommandAction::SetField { component, field } => {
                    let schema = self.component(component).expect("checked above");
                    if !schema.fields.iter().any(|f| &f.name == field) {
                        return Err(format!(
                            "{}: {component} has no field {field}",
                            command.name
                        ));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn health() -> ComponentSchema {
        serde_json::from_value(json!({
            "type_id": "Health",
            "version": 2,
            "fields": [
                {"name": "hp", "type": "int", "min": 0, "max": 100, "default": 10},
                {"name": "label", "type": "text", "default": "", "max_len": 8},
                {"name": "tint", "type": "color", "default": "#FF0000"},
                {"name": "friends", "type": "list", "item": {"type": "actor"}, "default": []},
                {"name": "icon", "type": "asset", "kind": "image", "default": ""},
                {"name": "mode", "type": "choice", "options": ["a", "b"], "default": "a"}
            ],
            "migrations": [
                {"from": 1, "ops": [{"op": "rename_field", "from": "health", "to": "hp"}]}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn defaults_validate() {
        let schema = health();
        schema.check_definition().unwrap();
        let defaults = schema.defaults();
        assert_eq!(defaults["hp"], 10);
        schema.validate(&defaults).unwrap();
    }

    #[test]
    fn bad_payloads_are_located() {
        let schema = health();
        let errors = schema
            .validate(
                &json!({"hp": 500, "label": "far too long", "tint": "red", "nope": 1,
                "friends": [1], "mode": "z"}),
            )
            .unwrap_err();
        let at: Vec<&str> = errors.iter().map(|e| e.at.as_str()).collect();
        for expected in ["hp", "label", "tint", "nope", "friends[0]", "mode"] {
            assert!(at.contains(&expected), "{expected} in {at:?}");
        }
        assert!(schema.validate(&json!([1])).is_err());
    }

    #[test]
    fn missing_required_fields_are_errors_but_defaults_are_not() {
        let schema: ComponentSchema = serde_json::from_value(
            json!({"type_id": "T", "fields": [{"name": "a", "type": "bool"}]}),
        )
        .unwrap();
        assert!(schema.validate(&json!({})).is_err());
        assert!(schema.validate(&json!({"a": true})).is_ok());
        assert_eq!(schema.defaults(), json!({"a": false}));
    }

    #[test]
    fn references_are_enumerated_with_their_paths() {
        let schema = health();
        let refs = schema.references(&json!({"friends": ["a", "b"], "icon": "assets/x.png"}));
        assert_eq!(refs.len(), 3);
        assert!(
            refs.iter()
                .any(|r| r.kind == ReferenceKind::Asset && r.at == "icon")
        );
        assert!(
            refs.iter()
                .any(|r| r.kind == ReferenceKind::Actor && r.at == "friends[1]")
        );
    }

    #[test]
    fn migrations_chain_and_refuse_the_future() {
        let schema = health();
        let moved = schema.migrate(1, &json!({"health": 7})).unwrap();
        assert_eq!(moved["hp"], 7);
        assert!(schema.migrate(3, &json!({})).is_err());
        assert_eq!(
            schema.migrate(2, &json!({"hp": 1})).unwrap(),
            json!({"hp": 1})
        );
    }

    #[test]
    fn a_missing_migration_step_is_a_definition_error() {
        let mut schema = health();
        schema.migrations.clear();
        assert!(schema.check_definition().is_err());
    }

    #[test]
    fn blocks_must_match_their_slots_and_commands() {
        let mut contributions: Contributions = serde_json::from_value(json!({
            "components": [{"type_id": "Health", "fields": [{"name": "hp", "type": "int", "default": 1}]}],
            "commands": [{"name": "heal", "summary": "x",
                "args": [{"name": "actor", "type": "actor"}, {"name": "value", "type": "int"}],
                "action": {"do": "set_field", "component": "Health", "field": "hp"}}],
            "blocks": [{"type_id": "heal", "kind": "statement", "category": "Health",
                "label": "heal {actor} by {value}",
                "slots": [{"name": "actor", "type": "actor"}, {"name": "value", "type": "int"}],
                "command": "heal"}]
        }))
        .unwrap();
        contributions.check_definition().unwrap();
        contributions.blocks[0].label = "heal {actor}".to_string();
        assert!(contributions.check_definition().is_err());
        contributions.blocks[0].label = "heal {actor} by {value} {ghost}".to_string();
        assert!(contributions.check_definition().is_err());
    }

    #[test]
    fn hooks_must_order_against_real_hooks() {
        let mut c: Contributions = serde_json::from_value(json!({
            "hooks": [
                {"name": "a", "stage": "fixed_simulation", "after": ["b"]},
                {"name": "b", "stage": "fixed_simulation", "before": ["other.plugin/x"]}
            ]
        }))
        .unwrap();
        c.check_definition().unwrap();
        c.hooks[0].after = vec!["ghost".to_string()];
        assert!(c.check_definition().is_err());
        c.hooks[0].after = vec!["a".to_string()];
        assert!(c.check_definition().is_err());
    }

    #[test]
    fn commands_may_not_point_at_missing_fields() {
        let contributions: Contributions = serde_json::from_value(json!({
            "components": [{"type_id": "Health", "fields": [{"name": "hp", "type": "int", "default": 1}]}],
            "commands": [{"name": "x", "summary": "x",
                "args": [{"name": "actor", "type": "actor"}, {"name": "value", "type": "int"}],
                "action": {"do": "set_field", "component": "Health", "field": "mana"}}]
        }))
        .unwrap();
        assert!(contributions.check_definition().is_err());
    }

    #[test]
    fn resource_field_commands_name_a_real_field_and_list() {
        let with = |action: Value| -> Result<(), String> {
            let c: Contributions = serde_json::from_value(json!({
                "resources": [{"type_id": "Log", "fields": [
                    {"name": "title", "type": "text", "default": ""},
                    {"name": "lines", "type": "list", "item": {"type": "text"}, "default": []}
                ]}],
                "commands": [{"name": "x", "summary": "x",
                    "args": [{"name": "value", "type": "text"}], "action": action}]
            }))
            .unwrap();
            c.check_definition()
        };
        with(json!({"do": "set_resource_field", "resource": "Log", "field": "title"})).unwrap();
        with(json!({"do": "set_resource_field", "resource": "Log", "field": "lines", "append": true}))
            .unwrap();
        let e = with(json!({"do": "set_resource_field", "resource": "Log", "field": "nope"}));
        assert!(e.unwrap_err().contains("no field"));
        let e = with(json!({"do": "set_resource_field", "resource": "Nope", "field": "title"}));
        assert!(e.unwrap_err().contains("unknown resource"));
        let e = with(
            json!({"do": "set_resource_field", "resource": "Log", "field": "title", "append": true}),
        );
        assert!(e.unwrap_err().contains("not a list"));
    }

    #[test]
    fn panels_name_real_resources_and_commands() {
        let with = |items: Value| -> Result<(), String> {
            let c: Contributions = serde_json::from_value(json!({
                "resources": [{"type_id": "Log", "fields": [{"name": "t", "type": "text", "default": ""}]}],
                "commands": [{"name": "go", "summary": "go", "action": {"do": "module", "op": "go"}}],
                "panels": [{"name": "main", "title": "Main", "items": items}]
            }))
            .unwrap();
            c.check_definition()
        };
        with(json!([
            {"kind": "text", "text": "Hello"},
            {"kind": "resource", "resource": "Log"},
            {"kind": "command", "command": "go", "label": "Go"}
        ]))
        .unwrap();
        assert!(with(json!([])).is_err());
        let e = with(json!([{"kind": "resource", "resource": "Nope"}]));
        assert!(e.unwrap_err().contains("unknown resource"));
        let e = with(json!([{"kind": "command", "command": "nope"}]));
        assert!(e.unwrap_err().contains("unknown command"));
    }

    #[test]
    fn a_tool_names_a_command_and_fills_its_arguments_from_a_hit() {
        let schema = |args: Value| -> Contributions {
            serde_json::from_value(json!({
                "commands": [{"name": "paint", "summary": "x",
                    "args": [{"name": "x", "type": "int"}, {"name": "m", "type": "text", "default": "a"}],
                    "action": {"do": "module", "op": "paint"}}],
                "tools": [{"name": "brush", "title": "Brush", "cast": "cast", "command": "paint",
                    "args": args,
                    "options": [{"name": "material", "type": "text", "default": "stone"}]}]
            }))
            .unwrap()
        };
        let good = schema(json!({"x": "$hit.cell.0", "m": "$option.material"}));
        good.check_definition().unwrap();
        let tool = &good.tools[0];
        let hit = json!({"hit": true, "cell": [4, 5, 6]});
        assert_eq!(
            tool.resolve_args(&hit, &json!({})).unwrap(),
            json!({"x": 4, "m": "stone"})
        );
        assert_eq!(
            tool.resolve_args(&hit, &json!({"material": "wood"}))
                .unwrap()["m"],
            "wood"
        );
        assert!(
            tool.resolve_args(&json!({"hit": true}), &json!({}))
                .is_err()
        );
        for bad in [
            json!({"nope": "1"}),
            json!({"x": "$option.missing"}),
            json!({"x": "$other.cell"}),
        ] {
            assert!(schema(bad).check_definition().is_err());
        }
    }

    #[test]
    fn a_tool_outline_is_a_path_to_six_numbers() {
        let hit = json!({"hit": true, "before_box": [0, 1, 2, 3.5, 4, 5], "short": [1, 2]});
        assert_eq!(
            box_at(&hit, "before_box"),
            Some([0.0, 1.0, 2.0, 3.5, 4.0, 5.0])
        );
        assert_eq!(box_at(&hit, "short"), None);
        assert_eq!(box_at(&hit, "missing"), None);
        assert_eq!(box_at(&hit, ""), None);
        let bad: Contributions = serde_json::from_value(json!({
            "commands": [{"name": "paint", "summary": "x", "args": [],
                "action": {"do": "module", "op": "paint"}}],
            "tools": [{"name": "brush", "title": "Brush", "cast": "cast",
                "command": "paint", "outline": "$hit.box", "drag": true}]
        }))
        .unwrap();
        assert!(bad.check_definition().is_err());
    }

    #[test]
    fn templates_name_arguments_and_fill_from_them() {
        assert_eq!(template_names("fill {a} {b} x {a}"), ["a", "b", "a"]);
        let args = json!({"x": 3.0, "r": 2.5, "m": "wood"});
        assert_eq!(
            fill_template("sphere {x} {r} {m} {{", &args).unwrap(),
            "sphere 3 2.5 wood {{"
        );
        assert!(fill_template("go {nope}", &args).is_err());
        let c: Contributions = serde_json::from_value(json!({
            "resources": [{"type_id": "Log", "fields": [{"name": "lines", "type": "list", "item": {"type": "text"}, "default": []}]}],
            "commands": [{"name": "add", "summary": "x",
                "args": [{"name": "x", "type": "int"}],
                "action": {"do": "set_resource_field", "resource": "Log", "field": "lines", "append": true, "template": "at {x}"}}]
        }))
        .unwrap();
        c.check_definition().unwrap();
        let mut bad = c.clone();
        if let CommandAction::SetResourceField { template, .. } = &mut bad.commands[0].action {
            *template = Some("at {y}".to_string());
        }
        assert!(bad.check_definition().is_err());
    }

    #[test]
    fn importers_and_build_hooks_are_checked() {
        let mut c: Contributions = serde_json::from_value(json!({
            "importers": [{"name": "gpl", "summary": "palettes", "extensions": ["gpl", "pal"]}],
            "build": [{"name": "check", "summary": "checks", "limit_ms": 1000}]
        }))
        .unwrap();
        c.check_definition().unwrap();
        assert!(c.importers[0].handles("GPL"));
        assert!(!c.importers[0].handles("png"));
        assert_eq!(c.importers[0].limit_ms, crate::assets::DEFAULT_LIMIT_MS);

        c.importers[0].extensions = vec![".gpl".to_string()];
        assert!(c.check_definition().is_err());
        c.importers[0].extensions = vec!["gpl".to_string(), "gpl".to_string()];
        assert!(c.check_definition().is_err());
        c.importers[0].extensions = Vec::new();
        assert!(c.check_definition().is_err());
        c.importers[0].extensions = vec!["gpl".to_string()];
        c.importers.push(c.importers[0].clone());
        assert!(c.check_definition().is_err());
        c.importers.pop();
        c.build_hooks[0].limit_ms = 0;
        assert!(c.check_definition().is_err());
        c.build_hooks[0].limit_ms = crate::assets::MAX_LIMIT_MS + 1;
        assert!(c.check_definition().is_err());
    }
    fn with(fields: Value, inspector: Value) -> ComponentSchema {
        serde_json::from_value(json!({
            "type_id": "Ui", "fields": fields, "inspector": inspector
        }))
        .unwrap()
    }

    #[test]
    fn inspector_hints_round_trip_and_leave_payloads_alone() {
        let schema = with(
            json!([
                {"name": "on", "type": "bool", "default": true},
                {"name": "power", "type": "number", "min": 0, "max": 1, "default": 0.5,
                 "ui": {"label": "Power", "widget": "slider", "step": 0.1, "unit": "%",
                        "visible_when": {"field": "on"}}},
                {"name": "note", "type": "text", "default": "",
                 "ui": {"widget": "multiline", "visible_when": {"field": "on", "equals": true}}}
            ]),
            json!({"groups": [{"label": "Main", "fields": ["on", "power"], "collapsed": true}]}),
        );
        schema.check_definition().unwrap();
        schema.validate(&schema.defaults()).unwrap();
        let again: ComponentSchema =
            serde_json::from_value(serde_json::to_value(&schema).unwrap()).unwrap();
        assert_eq!(again, schema);
        assert_eq!(
            schema.fields[1].ui.as_ref().unwrap().widget,
            Some(Widget::Slider)
        );
        assert!(schema.inspector.as_ref().unwrap().groups[0].collapsed);
        assert!(health().inspector.is_none());
    }

    #[test]
    fn bad_inspector_hints_are_definition_errors() {
        let bad = |fields: Value, inspector: Value| {
            with(fields, inspector).check_definition().unwrap_err()
        };
        let none = json!(null);
        // A slider needs a bounded number.
        let e = bad(
            json!([{"name": "a", "type": "int", "default": 0, "ui": {"widget": "slider"}}]),
            none.clone(),
        );
        assert!(e.contains("slider"), "{e}");
        let e = bad(
            json!([{"name": "a", "type": "bool", "default": false, "ui": {"widget": "multiline"}}]),
            none.clone(),
        );
        assert!(e.contains("multiline"), "{e}");
        let e = bad(
            json!([{"name": "a", "type": "int", "min": 0, "max": 5, "default": 0,
                    "ui": {"widget": "slider", "step": 0}}]),
            none.clone(),
        );
        assert!(e.contains("step"), "{e}");
        // Conditions name another real field.
        let e = bad(
            json!([{"name": "a", "type": "bool", "default": false,
                    "ui": {"visible_when": {"field": "a"}}}]),
            none.clone(),
        );
        assert!(e.contains("itself"), "{e}");
        let e = bad(
            json!([{"name": "a", "type": "bool", "default": false,
                    "ui": {"visible_when": {"field": "zzz"}}}]),
            none.clone(),
        );
        assert!(e.contains("zzz"), "{e}");
        let e = bad(
            json!([{"name": "a", "type": "bool", "default": false},
                   {"name": "b", "type": "bool", "default": false,
                    "ui": {"visible_when": {"field": "a", "equals": true, "not_equals": false}}}]),
            none,
        );
        assert!(e.contains("not both"), "{e}");
        // Groups name each field once.
        let fields = json!([{"name": "a", "type": "bool", "default": false}]);
        let e = bad(
            fields.clone(),
            json!({"groups": [{"label": "G", "fields": ["nope"]}]}),
        );
        assert!(e.contains("nope"), "{e}");
        let e = bad(
            fields,
            json!({"groups": [{"label": "G", "fields": ["a"]}, {"label": "H", "fields": ["a"]}]}),
        );
        assert!(e.contains("twice"), "{e}");
    }
}
