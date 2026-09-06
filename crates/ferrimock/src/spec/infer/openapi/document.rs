//! Reading an OpenAPI document into something inference can work on.
//!
//! The document is read off a `serde_json::Value` rather than deserialized into
//! typed structs. Two reasons, both of which cost real time when ignored:
//!
//! - Typed OpenAPI crates model 3.0 and 3.1 as separate type systems, and their
//!   `Either`-shaped fields are untagged enums. `serde_json/arbitrary_precision`
//!   is force-enabled workspace-wide, and untagged buffering under it turns a
//!   number into a private one-key map. Walking a `Value` never meets either
//!   problem.
//! - The divergences between 3.0 and 3.1 are few and local (`nullable`,
//!   `exclusiveMinimum`, `example`), so one reader that names them beats two
//!   type systems that share nothing.
//!
//! `$ref` is *kept*, not inlined. A `$ref` from one schema into another is the
//! strongest relation signal an OpenAPI document carries, and resolving it at
//! read time would erase it.

use lean_string::LeanString;
use rustc_hash::FxHashMap;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::fmt;
use std::sync::Arc;

use crate::core::world::model::Constraints;

/// How deep `allOf` composition is followed before it is treated as a cycle.
const MAX_COMPOSITION_DEPTH: usize = 8;

/// Which version of the specification a document declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenApiVersion {
    /// 3.0.x — `nullable`, boolean `exclusiveMinimum`, no `$ref` siblings.
    V30,
    /// 3.1.x — JSON Schema 2020-12: `type` may be a list, `exclusiveMinimum`
    /// is a number, `$ref` may carry siblings.
    V31,
}

impl fmt::Display for OpenApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OpenApiVersion::V30 => "3.0",
            OpenApiVersion::V31 => "3.1",
        })
    }
}

/// Something the reader could not read, kept rather than thrown.
///
/// A 500-operation document with three external `$ref`s is still worth serving;
/// silently dropping the three is what makes a generated backend untrustworthy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecDefect {
    pub location: LeanString,
    pub kind: DefectKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefectKind {
    /// A `$ref` pointing outside this document.
    ExternalRef(LeanString),
    /// A `$ref` this document does not contain.
    DanglingRef(LeanString),
    /// An operation whose method the HTTP crate does not know.
    UnknownMethod(LeanString),
    /// Two operations that would mount at the same method and path.
    DuplicateOperation(LeanString),
}

impl fmt::Display for SpecDefect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            DefectKind::ExternalRef(target) => write!(
                f,
                "{}: `{target}` points outside this document; that schema is answered \
                 as an untyped value",
                self.location
            ),
            DefectKind::DanglingRef(target) => write!(
                f,
                "{}: `{target}` is not defined in this document",
                self.location
            ),
            DefectKind::UnknownMethod(method) => {
                write!(f, "{}: `{method}` is not an HTTP method", self.location)
            }
            DefectKind::DuplicateOperation(id) => write!(
                f,
                "{}: a second operation would mount as `{id}`; the first one wins",
                self.location
            ),
        }
    }
}

/// Every operation a document declares, plus the schemas they refer to.
///
/// This is what a `serve: rest` mock expands from, so it is kept per schema
/// file: entity graphs merge across documents, operation tables do not.
pub struct OperationTable {
    pub version: OpenApiVersion,
    pub title: LeanString,
    /// `servers[].url`, reported but never mounted from. A document says what
    /// it is; a mock says where it answers.
    pub servers: Vec<LeanString>,
    pub operations: Vec<Operation>,
    pub schemas: SchemaBook,
}

impl OperationTable {
    #[must_use]
    pub fn operation(&self, id: &str) -> Option<&Operation> {
        self.operations.iter().find(|op| op.id == id)
    }
}

impl fmt::Debug for OperationTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationTable")
            .field("version", &self.version)
            .field("title", &self.title)
            .field("operations", &self.operations.len())
            .field("schemas", &self.schemas.len())
            .finish_non_exhaustive()
    }
}

/// One method on one path.
#[derive(Debug, Clone)]
pub struct Operation {
    /// `operationId`, or `{method}-{path}` when the document omits one.
    pub id: LeanString,
    /// Whether the document named the operation itself.
    pub declared_id: bool,
    pub method: http::Method,
    /// The path template, exactly as written (`/folders/{folder_id}`).
    pub path: LeanString,
    pub segments: Vec<Segment>,
    pub summary: Option<LeanString>,
    pub description: Option<LeanString>,
    pub deprecated: bool,
    pub parameters: Vec<Parameter>,
    pub request_body: Option<SchemaRef>,
    pub responses: Vec<ResponseSpec>,
    pub extensions: JsonMap<String, JsonValue>,
}

impl Operation {
    /// The 2xx response a client is meant to get, preferring the most specific
    /// success code the document names.
    #[must_use]
    pub fn success(&self) -> Option<&ResponseSpec> {
        self.responses
            .iter()
            .filter(|r| r.status.is_success_range())
            .min_by_key(|r| match r.status {
                StatusPattern::Exact(code) => (0, code),
                StatusPattern::Range(class) => (1, u16::from(class) * 100),
                StatusPattern::Default => (2, 0),
            })
    }

    /// Path parameters, in the order the path declares them.
    pub fn path_params(&self) -> impl Iterator<Item = &LeanString> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Param(name) => Some(name),
            Segment::Literal(_) => None,
        })
    }

    /// The last literal segment, which is what names the resource.
    #[must_use]
    pub fn resource_segment(&self) -> Option<&LeanString> {
        self.segments
            .iter()
            .rev()
            .find_map(|segment| match segment {
                Segment::Literal(name) => Some(name),
                Segment::Param(_) => None,
            })
    }

    /// Whether the path ends in a parameter, which is what addresses one item.
    #[must_use]
    pub fn addresses_item(&self) -> bool {
        matches!(self.segments.last(), Some(Segment::Param(_)))
    }

    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }
}

/// A path template split into what varies and what does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Literal(LeanString),
    Param(LeanString),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamIn {
    Path,
    Query,
    Header,
    Cookie,
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: LeanString,
    pub location: ParamIn,
    pub required: bool,
    pub description: Option<LeanString>,
    pub schema: Option<SchemaRef>,
}

/// A status code, or the class or default a document answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusPattern {
    Exact(u16),
    /// `2XX`, `4XX` — the leading digit.
    Range(u8),
    Default,
}

impl StatusPattern {
    #[must_use]
    pub fn is_success_range(self) -> bool {
        match self {
            StatusPattern::Exact(code) => (200..300).contains(&code),
            StatusPattern::Range(class) => class == 2,
            StatusPattern::Default => false,
        }
    }

    /// The status to actually answer with.
    #[must_use]
    pub fn status_code(self) -> u16 {
        match self {
            StatusPattern::Exact(code) => code,
            StatusPattern::Range(class) => u16::from(class) * 100,
            StatusPattern::Default => 200,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResponseSpec {
    pub status: StatusPattern,
    pub content_type: Option<LeanString>,
    pub schema: Option<SchemaRef>,
    pub links: Vec<SpecLink>,
}

/// An entry in a response's `links` object: the document stating outright that
/// this response leads to that operation, and which field supplies the key.
#[derive(Debug, Clone)]
pub struct SpecLink {
    pub name: LeanString,
    /// The operation linked to, by `operationId`.
    pub operation_id: Option<LeanString>,
    /// The path the link says to follow (`operationRef`), when it names one.
    pub operation_ref: Option<LeanString>,
    /// Parameter name to the runtime expression that fills it.
    pub parameters: Vec<(LeanString, LeanString)>,
}

/// A schema, either by name or written out.
#[derive(Debug, Clone)]
pub enum SchemaRef {
    /// `#/components/schemas/Folder`.
    Named(LeanString),
    Inline(Arc<SchemaNode>),
}

impl SchemaRef {
    /// The component name, when this is a reference to one.
    #[must_use]
    pub fn name(&self) -> Option<&LeanString> {
        match self {
            SchemaRef::Named(name) => Some(name),
            SchemaRef::Inline(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaKind {
    Object,
    Array,
    String,
    Integer,
    Number,
    Boolean,
    /// No `type`, or one this reader does not model.
    Any,
}

/// A JSON Schema, in the subset a mocked backend can act on.
#[derive(Debug, Clone, Default)]
pub struct SchemaNode {
    pub kind: Option<SchemaKind>,
    pub format: Option<LeanString>,
    pub nullable: bool,
    pub title: Option<LeanString>,
    pub description: Option<LeanString>,
    pub properties: Vec<Property>,
    pub items: Option<SchemaRef>,
    pub enum_values: Vec<LeanString>,
    pub constraints: Constraints,
    /// Values the document itself wrote for this schema.
    ///
    /// A declared example is better evidence than any inference from a field
    /// name: `example: "usr_01H8XG..."` says what an id family is, and nothing
    /// in the word `id` could. 3.0 spells it `example`, 3.1 `examples`.
    pub examples: Vec<JsonValue>,
    /// Composition, kept unmerged so a `$ref` inside it stays visible.
    pub all_of: Vec<SchemaRef>,
    /// `oneOf`/`anyOf` together: both mean "one of these shapes" to a mock.
    pub one_of: Vec<SchemaRef>,
    pub extensions: JsonMap<String, JsonValue>,
}

impl SchemaNode {
    #[must_use]
    pub fn effective_kind(&self) -> SchemaKind {
        self.kind.unwrap_or(if self.properties.is_empty() {
            SchemaKind::Any
        } else {
            SchemaKind::Object
        })
    }

    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }
}

#[derive(Debug, Clone)]
pub struct Property {
    pub name: LeanString,
    pub schema: SchemaRef,
    pub required: bool,
}

/// The document's named schemas, in declaration order.
#[derive(Debug, Default)]
pub struct SchemaBook {
    by_name: FxHashMap<LeanString, Arc<SchemaNode>>,
    order: Vec<LeanString>,
}

impl SchemaBook {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Arc<SchemaNode>> {
        self.by_name.get(name)
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &LeanString> {
        self.order.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The node a reference points at.
    #[must_use]
    pub fn resolve(&self, reference: &SchemaRef) -> Option<Arc<SchemaNode>> {
        match reference {
            SchemaRef::Named(name) => self.get(name).map(Arc::clone),
            SchemaRef::Inline(node) => Some(Arc::clone(node)),
        }
    }

    /// A schema with its `allOf` composition merged in.
    ///
    /// Properties from later members win, which is what `allOf: [Base, {…}]`
    /// means in every document that uses it for extension.
    #[must_use]
    pub fn effective(&self, reference: &SchemaRef) -> Option<Arc<SchemaNode>> {
        let node = self.resolve(reference)?;
        if node.all_of.is_empty() {
            return Some(node);
        }
        let mut open = Vec::new();
        Some(Arc::new(self.merge_composition(&node, &mut open)))
    }

    fn merge_composition(&self, node: &SchemaNode, open: &mut Vec<LeanString>) -> SchemaNode {
        let mut merged = node.clone();
        merged.all_of = Vec::new();

        for member in &node.all_of {
            if let SchemaRef::Named(name) = member {
                if open.contains(name) || open.len() >= MAX_COMPOSITION_DEPTH {
                    continue;
                }
                open.push(name.clone());
            }
            let Some(part) = self.resolve(member) else {
                if let SchemaRef::Named(_) = member {
                    open.pop();
                }
                continue;
            };
            let part = self.merge_composition(&part, open);
            if let SchemaRef::Named(_) = member {
                open.pop();
            }

            merged.kind = merged.kind.or(part.kind);
            merged.format = merged.format.or(part.format);
            merged.items = merged.items.or(part.items);
            merged.description = merged.description.or(part.description);
            merged.nullable |= part.nullable;
            if merged.enum_values.is_empty() {
                merged.enum_values = part.enum_values;
            }
            if merged.examples.is_empty() {
                merged.examples = part.examples;
            }
            for property in part.properties {
                if merged.property(property.name.as_str()).is_none() {
                    merged.properties.push(property);
                }
            }
            for (key, value) in part.extensions {
                merged.extensions.entry(key).or_insert(value);
            }
        }

        merged
    }
}

/// Read an OpenAPI document.
///
/// Accepts JSON and YAML, distinguished by the first non-whitespace byte rather
/// than by extension — a `.yaml` holding JSON is ordinary, and YAML's own
/// parser is slower on the large documents this is aimed at.
pub fn parse_openapi(source: &str) -> crate::Result<(OperationTable, Vec<SpecDefect>)> {
    let root: JsonValue = if source.trim_start().starts_with('{') {
        serde_json::from_str(source)
            .map_err(|e| crate::mp_err!("not a readable OpenAPI document: {e}"))?
    } else {
        serde_yaml_ng::from_str(source)
            .map_err(|e| crate::mp_err!("not a readable OpenAPI document: {e}"))?
    };

    let JsonValue::Object(root) = root else {
        return Err(crate::mp_err!(
            "an OpenAPI document is a mapping; this file holds a {}",
            type_name_of(&root)
        ));
    };

    let version = read_version(&root)?;
    let mut defects = Vec::new();

    let title = root
        .get("info")
        .and_then(JsonValue::as_object)
        .and_then(|info| info.get("title"))
        .and_then(JsonValue::as_str)
        .map_or_else(|| LeanString::from("(untitled)"), LeanString::from);

    let servers = root
        .get("servers")
        .and_then(JsonValue::as_array)
        .map(|servers| {
            servers
                .iter()
                .filter_map(|server| server.get("url").and_then(JsonValue::as_str))
                .map(LeanString::from)
                .collect()
        })
        .unwrap_or_default();

    let components = root.get("components").and_then(JsonValue::as_object);
    let schemas = read_schema_book(components, &mut defects);
    let reader = Reader { components };
    let operations = read_paths(&root, &reader, &mut defects);

    Ok((
        OperationTable {
            version,
            title,
            servers,
            operations,
            schemas,
        },
        defects,
    ))
}

fn read_version(root: &JsonMap<String, JsonValue>) -> crate::Result<OpenApiVersion> {
    if let Some(swagger) = root.get("swagger").and_then(JsonValue::as_str) {
        return Err(crate::mp_err!(
            "this is a Swagger {swagger} document; ferrimock reads OpenAPI 3.0 and 3.1. \
             Convert it first (`swagger2openapi`), or point `world.schemas` at the 3.x form."
        ));
    }

    let declared = root
        .get("openapi")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            crate::mp_err!(
                "no `openapi:` version in this document; a mock collection is loaded from a \
                 plain .yaml/.json, and an OpenAPI document has to say which version it is"
            )
        })?;

    match declared.split('.').next() {
        Some("3") => match declared.split('.').nth(1) {
            Some("0") => Ok(OpenApiVersion::V30),
            Some("1") => Ok(OpenApiVersion::V31),
            _ => Err(crate::mp_err!(
                "OpenAPI {declared} is not a version this reads; 3.0 and 3.1 are"
            )),
        },
        _ => Err(crate::mp_err!(
            "OpenAPI {declared} is not a version this reads; 3.0 and 3.1 are"
        )),
    }
}

/// Where a `$ref` can be followed to.
struct Reader<'a> {
    components: Option<&'a JsonMap<String, JsonValue>>,
}

impl<'a> Reader<'a> {
    /// Follow a local `$ref` into `components`.
    fn follow(
        &self,
        section: &str,
        value: &'a JsonValue,
        location: &str,
        defects: &mut Vec<SpecDefect>,
    ) -> Option<&'a JsonValue> {
        let Some(pointer) = value.get("$ref").and_then(JsonValue::as_str) else {
            return Some(value);
        };
        let Some(name) = local_component(pointer, section) else {
            defects.push(SpecDefect {
                location: LeanString::from(location),
                kind: if pointer.starts_with('#') {
                    DefectKind::DanglingRef(LeanString::from(pointer))
                } else {
                    DefectKind::ExternalRef(LeanString::from(pointer))
                },
            });
            return None;
        };
        let resolved = self
            .components
            .and_then(|components| components.get(section))
            .and_then(JsonValue::as_object)
            .and_then(|section| section.get(name));
        if resolved.is_none() {
            defects.push(SpecDefect {
                location: LeanString::from(location),
                kind: DefectKind::DanglingRef(LeanString::from(pointer)),
            });
        }
        resolved
    }
}

/// The component name a local pointer into `section` names.
fn local_component<'a>(pointer: &'a str, section: &str) -> Option<&'a str> {
    let prefix = format!("#/components/{section}/");
    pointer
        .strip_prefix(&prefix)
        .filter(|name| !name.is_empty() && !name.contains('/'))
}

fn read_schema_book(
    components: Option<&JsonMap<String, JsonValue>>,
    defects: &mut Vec<SpecDefect>,
) -> SchemaBook {
    let mut book = SchemaBook::default();
    let Some(schemas) = components
        .and_then(|components| components.get("schemas"))
        .and_then(JsonValue::as_object)
    else {
        return book;
    };

    for (name, value) in schemas {
        let node = read_schema(value, &format!("components.schemas.{name}"), defects);
        let name = LeanString::from(name.as_str());
        book.order.push(name.clone());
        book.by_name.insert(name, Arc::new(node));
    }
    book
}

/// A schema position: either a reference to a named component, or a node.
fn read_schema_ref(
    value: &JsonValue,
    location: &str,
    defects: &mut Vec<SpecDefect>,
) -> Option<SchemaRef> {
    if let Some(pointer) = value.get("$ref").and_then(JsonValue::as_str) {
        let Some(name) = local_component(pointer, "schemas") else {
            defects.push(SpecDefect {
                location: LeanString::from(location),
                kind: DefectKind::ExternalRef(LeanString::from(pointer)),
            });
            return None;
        };
        return Some(SchemaRef::Named(LeanString::from(name)));
    }
    if !value.is_object() {
        return None;
    }
    Some(SchemaRef::Inline(Arc::new(read_schema(
        value, location, defects,
    ))))
}

fn read_schema(value: &JsonValue, location: &str, defects: &mut Vec<SpecDefect>) -> SchemaNode {
    let Some(object) = value.as_object() else {
        return SchemaNode::default();
    };

    let (kind, mut nullable) = read_type(object);

    // 3.0 spells absence as a sibling flag; 3.1 puts `"null"` in the type list,
    // which `read_type` already folded in.
    if object.get("nullable").and_then(JsonValue::as_bool) == Some(true) {
        nullable = true;
    }

    let required: Vec<&str> = object
        .get("required")
        .and_then(JsonValue::as_array)
        .map(|names| names.iter().filter_map(JsonValue::as_str).collect())
        .unwrap_or_default();

    let properties = object
        .get("properties")
        .and_then(JsonValue::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter_map(|(name, schema)| {
                    let reference =
                        read_schema_ref(schema, &format!("{location}.{name}"), defects)?;
                    Some(Property {
                        name: LeanString::from(name.as_str()),
                        schema: reference,
                        required: required.contains(&name.as_str()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let items = object
        .get("items")
        .and_then(|items| read_schema_ref(items, &format!("{location}[]"), defects));

    let enum_values = object
        .get("enum")
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| match value {
                    JsonValue::String(s) => Some(LeanString::from(s.as_str())),
                    JsonValue::Null => None,
                    other => Some(LeanString::from(other.to_string())),
                })
                .collect()
        })
        .unwrap_or_default();

    let all_of = read_composition(object, "allOf", location, defects);
    let mut one_of = read_composition(object, "oneOf", location, defects);
    one_of.extend(read_composition(object, "anyOf", location, defects));

    let mut examples: Vec<JsonValue> = object
        .get("examples")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(single) = object.get("example") {
        examples.push(single.clone());
    }
    examples.retain(|value| !value.is_null());

    SchemaNode {
        kind,
        examples,
        format: string_of(object, "format"),
        nullable,
        title: string_of(object, "title"),
        description: string_of(object, "description"),
        properties,
        items,
        enum_values,
        constraints: read_constraints(object),
        all_of,
        one_of,
        extensions: extensions_of(object),
    }
}

fn read_composition(
    object: &JsonMap<String, JsonValue>,
    key: &str,
    location: &str,
    defects: &mut Vec<SpecDefect>,
) -> Vec<SchemaRef> {
    let Some(members) = object.get(key).and_then(JsonValue::as_array) else {
        return Vec::new();
    };
    members
        .iter()
        .enumerate()
        .filter_map(|(i, member)| {
            read_schema_ref(member, &format!("{location}.{key}[{i}]"), defects)
        })
        .collect()
}

/// The declared type, and whether the declaration itself allowed null.
fn read_type(object: &JsonMap<String, JsonValue>) -> (Option<SchemaKind>, bool) {
    match object.get("type") {
        Some(JsonValue::String(name)) => (schema_kind(name), false),
        // 3.1: `type: [string, "null"]`.
        Some(JsonValue::Array(names)) => {
            let mut nullable = false;
            let mut kind = None;
            for name in names.iter().filter_map(JsonValue::as_str) {
                if name == "null" {
                    nullable = true;
                } else if kind.is_none() {
                    kind = schema_kind(name);
                }
            }
            (kind, nullable)
        }
        _ => (None, false),
    }
}

fn schema_kind(name: &str) -> Option<SchemaKind> {
    Some(match name {
        "object" => SchemaKind::Object,
        "array" => SchemaKind::Array,
        "string" => SchemaKind::String,
        "integer" => SchemaKind::Integer,
        "number" => SchemaKind::Number,
        "boolean" => SchemaKind::Boolean,
        _ => return None,
    })
}

fn read_constraints(object: &JsonMap<String, JsonValue>) -> Constraints {
    // 3.0 writes `exclusiveMinimum: true` beside `minimum`; 3.1 writes
    // `exclusiveMinimum: 5` instead of one. Both bound the same value, and a
    // generated sample being inclusive at the edge is not worth two shapes.
    let bound = |inclusive: &str, exclusive: &str| -> Option<f64> {
        object
            .get(inclusive)
            .and_then(JsonValue::as_f64)
            .or_else(|| object.get(exclusive).and_then(JsonValue::as_f64))
    };

    Constraints {
        min: bound("minimum", "exclusiveMinimum"),
        max: bound("maximum", "exclusiveMaximum"),
        min_length: usize_of(object, "minLength"),
        max_length: usize_of(object, "maxLength"),
        pattern: string_of(object, "pattern"),
        format: string_of(object, "format"),
    }
}

fn read_paths(
    root: &JsonMap<String, JsonValue>,
    reader: &Reader<'_>,
    defects: &mut Vec<SpecDefect>,
) -> Vec<Operation> {
    let Some(paths) = root.get("paths").and_then(JsonValue::as_object) else {
        return Vec::new();
    };

    let mut operations = Vec::new();
    let mut seen: rustc_hash::FxHashSet<(http::Method, LeanString)> =
        rustc_hash::FxHashSet::default();

    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        let segments = parse_path(path);

        // Parameters declared on the path item apply to every operation under
        // it, and real documents put the path parameters there rather than
        // repeating them per method.
        let shared = read_parameters(item, reader, path, defects);

        for (method_name, operation) in item {
            if method_name.starts_with("x-")
                || matches!(
                    method_name.as_str(),
                    "parameters" | "summary" | "description" | "servers" | "$ref"
                )
            {
                continue;
            }
            let Ok(method) = http::Method::from_bytes(method_name.to_ascii_uppercase().as_bytes())
            else {
                defects.push(SpecDefect {
                    location: LeanString::from(path.as_str()),
                    kind: DefectKind::UnknownMethod(LeanString::from(method_name.as_str())),
                });
                continue;
            };
            let Some(operation) = operation.as_object() else {
                continue;
            };

            let location = format!("{} {path}", method.as_str());
            let mut parameters = shared.clone();
            for parameter in read_parameters(operation, reader, &location, defects) {
                match parameters
                    .iter_mut()
                    .find(|p| p.name == parameter.name && p.location == parameter.location)
                {
                    Some(existing) => *existing = parameter,
                    None => parameters.push(parameter),
                }
            }

            let declared_id = operation
                .get("operationId")
                .and_then(JsonValue::as_str)
                .filter(|id| !id.is_empty());
            let id = declared_id.map_or_else(
                || LeanString::from(derived_id(&method, path)),
                LeanString::from,
            );

            if !seen.insert((method.clone(), id.clone())) {
                defects.push(SpecDefect {
                    location: LeanString::from(location.as_str()),
                    kind: DefectKind::DuplicateOperation(id),
                });
                continue;
            }

            operations.push(Operation {
                id,
                declared_id: declared_id.is_some(),
                method,
                path: LeanString::from(path.as_str()),
                segments: segments.clone(),
                summary: string_of(operation, "summary"),
                description: string_of(operation, "description"),
                deprecated: operation
                    .get("deprecated")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                parameters,
                request_body: read_request_body(operation, reader, &location, defects),
                responses: read_responses(operation, reader, &location, defects),
                extensions: extensions_of(operation),
            });
        }
    }

    // Paths arrive in document order, which is stable; sorting by mount point
    // keeps the emitted mocks in an order a reader can scan.
    operations.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.method.as_str().cmp(b.method.as_str()))
    });
    operations
}

/// The id an operation gets when the document did not name one.
fn derived_id(method: &http::Method, path: &str) -> String {
    let cleaned: String = path
        .trim_matches('/')
        .chars()
        .map(|c| match c {
            '/' => '-',
            '{' | '}' => '_',
            other => other,
        })
        .collect();
    format!("{}-{cleaned}", method.as_str().to_ascii_lowercase())
}

fn parse_path(path: &str) -> Vec<Segment> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(
            |segment| match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(name) => Segment::Param(LeanString::from(name)),
                None => Segment::Literal(LeanString::from(segment)),
            },
        )
        .collect()
}

fn read_parameters(
    owner: &JsonMap<String, JsonValue>,
    reader: &Reader<'_>,
    location: &str,
    defects: &mut Vec<SpecDefect>,
) -> Vec<Parameter> {
    let Some(parameters) = owner.get("parameters").and_then(JsonValue::as_array) else {
        return Vec::new();
    };

    parameters
        .iter()
        .filter_map(|parameter| {
            let parameter = reader.follow("parameters", parameter, location, defects)?;
            let object = parameter.as_object()?;
            let name = object.get("name").and_then(JsonValue::as_str)?;
            let location_in = match object.get("in").and_then(JsonValue::as_str)? {
                "path" => ParamIn::Path,
                "query" => ParamIn::Query,
                "header" => ParamIn::Header,
                "cookie" => ParamIn::Cookie,
                _ => return None,
            };
            Some(Parameter {
                name: LeanString::from(name),
                location: location_in,
                // A path parameter is required by the specification whether or
                // not the document bothered to say so.
                required: location_in == ParamIn::Path
                    || object
                        .get("required")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false),
                description: string_of(object, "description"),
                schema: object.get("schema").and_then(|schema| {
                    read_schema_ref(schema, &format!("{location}.{name}"), defects)
                }),
            })
        })
        .collect()
}

fn read_request_body(
    operation: &JsonMap<String, JsonValue>,
    reader: &Reader<'_>,
    location: &str,
    defects: &mut Vec<SpecDefect>,
) -> Option<SchemaRef> {
    let body = operation.get("requestBody")?;
    let body = reader.follow("requestBodies", body, location, defects)?;
    let content = body.get("content")?.as_object()?;
    let (_, media) = preferred_media(content)?;
    read_schema_ref(
        media.get("schema")?,
        &format!("{location}.requestBody"),
        defects,
    )
}

fn read_responses(
    operation: &JsonMap<String, JsonValue>,
    reader: &Reader<'_>,
    location: &str,
    defects: &mut Vec<SpecDefect>,
) -> Vec<ResponseSpec> {
    let Some(responses) = operation.get("responses").and_then(JsonValue::as_object) else {
        return Vec::new();
    };

    let mut specs: Vec<ResponseSpec> = responses
        .iter()
        .filter_map(|(status, response)| {
            let pattern = status_pattern(status)?;
            let response = reader.follow("responses", response, location, defects)?;
            let content = response.get("content").and_then(JsonValue::as_object);
            let media = content.and_then(preferred_media);
            let schema = media.as_ref().and_then(|(_, media)| {
                read_schema_ref(
                    media.get("schema")?,
                    &format!("{location}.responses.{status}"),
                    defects,
                )
            });
            Some(ResponseSpec {
                status: pattern,
                content_type: media.map(|(name, _)| LeanString::from(name)),
                schema,
                links: read_links(response),
            })
        })
        .collect();

    specs.sort_by_key(|spec| match spec.status {
        StatusPattern::Exact(code) => (0, code),
        StatusPattern::Range(class) => (1, u16::from(class)),
        StatusPattern::Default => (2, 0),
    });
    specs
}

fn read_links(response: &JsonValue) -> Vec<SpecLink> {
    let Some(links) = response.get("links").and_then(JsonValue::as_object) else {
        return Vec::new();
    };

    links
        .iter()
        .map(|(name, link)| SpecLink {
            name: LeanString::from(name.as_str()),
            operation_id: link
                .get("operationId")
                .and_then(JsonValue::as_str)
                .map(LeanString::from),
            operation_ref: link
                .get("operationRef")
                .and_then(JsonValue::as_str)
                .map(LeanString::from),
            parameters: link
                .get("parameters")
                .and_then(JsonValue::as_object)
                .map(|parameters| {
                    parameters
                        .iter()
                        .filter_map(|(key, value)| {
                            Some((
                                LeanString::from(key.as_str()),
                                LeanString::from(value.as_str()?),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

/// The media type to model. JSON is what a mocked backend can answer; a
/// document offering both JSON and XML means the JSON one.
fn preferred_media(content: &JsonMap<String, JsonValue>) -> Option<(&str, &JsonValue)> {
    content
        .iter()
        .find(|(name, _)| name.contains("json"))
        .or_else(|| content.iter().next())
        .map(|(name, media)| (name.as_str(), media))
}

fn status_pattern(status: &str) -> Option<StatusPattern> {
    if status.eq_ignore_ascii_case("default") {
        return Some(StatusPattern::Default);
    }
    if let Ok(code) = status.parse::<u16>() {
        return Some(StatusPattern::Exact(code));
    }
    let mut chars = status.chars();
    let class = u8::try_from(chars.next()?.to_digit(10)?).ok()?;
    chars
        .all(|c| c == 'X' || c == 'x')
        .then_some(StatusPattern::Range(class))
}

fn extensions_of(object: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    object
        .iter()
        .filter(|(key, _)| key.starts_with("x-"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn string_of(object: &JsonMap<String, JsonValue>, key: &str) -> Option<LeanString> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .map(LeanString::from)
}

fn usize_of(object: &JsonMap<String, JsonValue>, key: &str) -> Option<usize> {
    object
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn type_name_of(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "list",
        JsonValue::Object(_) => "mapping",
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    const FILESTORE: &str = r#"
openapi: 3.0.3
info:
  title: Filestore
  version: "2.0"
servers:
  - url: https://api.example.com/2.0
paths:
  /folders:
    get:
      operationId: listFolders
      parameters:
        - name: limit
          in: query
          schema: { type: integer }
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Folder' }
    post:
      operationId: createFolder
      requestBody:
        content:
          application/json:
            schema: { $ref: '#/components/schemas/FolderInput' }
      responses:
        "201":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
  /folders/{folder_id}:
    parameters:
      - name: folder_id
        in: path
        required: true
        schema: { type: string }
    get:
      operationId: getFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
          links:
            owner:
              operationId: getUser
              parameters: { user_id: "$response.body#/owner_id" }
        "404":
          description: gone
    delete:
      operationId: deleteFolder
      responses:
        "204":
          description: deleted
  /users/{user_id}:
    get:
      operationId: getUser
      parameters:
        - name: user_id
          in: path
          schema: { type: string }
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
components:
  schemas:
    Folder:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string, maxLength: 255 }
        owner_id: { type: string }
        description: { type: string, nullable: true }
    FolderInput:
      type: object
      properties:
        name: { type: string }
    User:
      allOf:
        - $ref: '#/components/schemas/Principal'
        - type: object
          properties:
            login: { type: string }
    Principal:
      type: object
      properties:
        id: { type: string }
"#;

    fn filestore() -> OperationTable {
        parse_openapi(FILESTORE).unwrap().0
    }

    #[test]
    fn a_document_reads_its_version_title_and_servers() {
        let table = filestore();
        assert_eq!(table.version, OpenApiVersion::V30);
        assert_eq!(table.title.as_str(), "Filestore");
        assert_eq!(table.servers[0].as_str(), "https://api.example.com/2.0");
    }

    #[test]
    fn every_operation_is_read_once() {
        let table = filestore();
        let mut ids: Vec<&str> = table.operations.iter().map(|op| op.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            [
                "createFolder",
                "deleteFolder",
                "getFolder",
                "getUser",
                "listFolders"
            ]
        );
    }

    #[test]
    fn a_path_item_parameter_reaches_every_method_under_it() {
        let table = filestore();
        for id in ["getFolder", "deleteFolder"] {
            let operation = table.operation(id).unwrap();
            assert!(
                operation.parameter("folder_id").is_some(),
                "{id} should inherit the path item's parameter"
            );
        }
    }

    #[test]
    fn a_path_splits_into_literals_and_parameters() {
        let table = filestore();
        let operation = table.operation("getFolder").unwrap();
        assert_eq!(
            operation.segments,
            vec![
                Segment::Literal(LeanString::from("folders")),
                Segment::Param(LeanString::from("folder_id")),
            ]
        );
        assert!(operation.addresses_item());
        assert_eq!(operation.resource_segment().unwrap().as_str(), "folders");
    }

    #[test]
    fn a_ref_is_kept_rather_than_inlined() {
        let table = filestore();
        let response = table.operation("getFolder").unwrap().success().unwrap();
        assert_eq!(
            response
                .schema
                .as_ref()
                .unwrap()
                .name()
                .map(LeanString::as_str),
            Some("Folder"),
            "the reference is the relation signal; inlining it erases the target"
        );
    }

    #[test]
    fn the_most_specific_success_response_is_the_one_that_answers() {
        let table = filestore();
        assert_eq!(
            table
                .operation("createFolder")
                .unwrap()
                .success()
                .unwrap()
                .status,
            StatusPattern::Exact(201)
        );
        assert_eq!(
            table
                .operation("deleteFolder")
                .unwrap()
                .success()
                .unwrap()
                .status,
            StatusPattern::Exact(204)
        );
    }

    #[test]
    fn required_and_nullable_come_off_the_schema() {
        let table = filestore();
        let folder = table.schemas.get("Folder").unwrap();
        assert!(folder.property("id").unwrap().required);
        assert!(!folder.property("owner_id").unwrap().required);

        let SchemaRef::Inline(description) = &folder.property("description").unwrap().schema else {
            panic!("an inline schema")
        };
        assert!(description.nullable);
    }

    #[test]
    fn constraints_survive() {
        let table = filestore();
        let folder = table.schemas.get("Folder").unwrap();
        let SchemaRef::Inline(name) = &folder.property("name").unwrap().schema else {
            panic!("an inline schema")
        };
        assert_eq!(name.constraints.max_length, Some(255));
    }

    #[test]
    fn all_of_merges_when_it_is_asked_for() {
        let table = filestore();
        let user = table
            .schemas
            .effective(&SchemaRef::Named(LeanString::from("User")))
            .unwrap();
        assert!(user.property("id").is_some(), "the base's fields come in");
        assert!(user.property("login").is_some());
    }

    #[test]
    fn links_are_read_because_they_state_a_relation_outright() {
        let table = filestore();
        let response = table.operation("getFolder").unwrap().success().unwrap();
        let link = &response.links[0];
        assert_eq!(link.name.as_str(), "owner");
        assert_eq!(link.operation_id.as_deref(), Some("getUser"));
        assert_eq!(link.parameters[0].0.as_str(), "user_id");
    }

    #[test]
    fn an_operation_without_an_id_gets_one_from_its_mount_point() {
        let (table, _) = parse_openapi(
            "openapi: 3.1.0\ninfo: { title: t }\npaths:\n  /a/{b}/c:\n    get:\n      responses:\n        \"200\": { description: ok }\n",
        )
        .unwrap();
        let operation = &table.operations[0];
        assert!(!operation.declared_id);
        assert_eq!(operation.id.as_str(), "get-a-_b_-c");
    }

    #[test]
    fn a_3_1_null_type_reads_as_nullable() {
        let (table, _) = parse_openapi(
            r#"{"openapi":"3.1.0","info":{"title":"t"},"paths":{},
                "components":{"schemas":{"A":{"type":"object","properties":
                {"note":{"type":["string","null"]}}}}}}"#,
        )
        .unwrap();
        let a = table.schemas.get("A").unwrap();
        let SchemaRef::Inline(note) = &a.property("note").unwrap().schema else {
            panic!("an inline schema")
        };
        assert!(note.nullable);
        assert_eq!(note.kind, Some(SchemaKind::String));
    }

    #[test]
    fn json_and_yaml_read_the_same() {
        let (from_json, _) = parse_openapi(
            r#"{"openapi":"3.0.0","info":{"title":"t"},"paths":{"/a":{"get":{"operationId":"a","responses":{}}}}}"#,
        )
        .unwrap();
        assert_eq!(from_json.operations[0].id.as_str(), "a");
    }

    #[test]
    fn swagger_2_is_refused_by_name_rather_than_half_read() {
        let error = parse_openapi("swagger: \"2.0\"\ninfo: { title: t }\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Swagger 2.0"), "unexpected: {error}");
        assert!(error.contains("3.0 and 3.1"), "unexpected: {error}");
    }

    #[test]
    fn a_collection_without_an_openapi_key_says_so() {
        let error = parse_openapi("name: my mocks\nmocks: []\n")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no `openapi:` version"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn an_external_ref_is_reported_rather_than_silently_dropped() {
        let (_, defects) = parse_openapi(
            "openapi: 3.0.0\ninfo: { title: t }\npaths: {}\ncomponents:\n  schemas:\n    A:\n      type: object\n      properties:\n        b: { $ref: 'other.yaml#/B' }\n",
        )
        .unwrap();
        assert_eq!(defects.len(), 1);
        assert!(matches!(defects[0].kind, DefectKind::ExternalRef(_)));
        assert!(defects[0].to_string().contains("other.yaml"));
    }

    #[test]
    fn a_media_type_that_is_not_json_still_yields_the_operation() {
        let (table, _) = parse_openapi(
            "openapi: 3.0.0\ninfo: { title: t }\npaths:\n  /f:\n    get:\n      operationId: f\n      responses:\n        \"200\":\n          content:\n            application/octet-stream:\n              schema: { type: string, format: binary }\n",
        )
        .unwrap();
        let response = table.operations[0].success().unwrap();
        assert_eq!(
            response.content_type.as_deref(),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn reading_is_order_independent() {
        let a: Vec<String> = filestore()
            .operations
            .iter()
            .map(|op| op.id.to_string())
            .collect();
        let b: Vec<String> = filestore()
            .operations
            .iter()
            .map(|op| op.id.to_string())
            .collect();
        assert_eq!(a, b);
    }
}
