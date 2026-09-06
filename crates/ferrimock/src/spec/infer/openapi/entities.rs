//! An OpenAPI document does not declare an entity graph; this infers one.
//!
//! A GraphQL schema states which types have identity. An OpenAPI document
//! states only paths and payloads, so identity has to be read off the shape of
//! the surface: a path that addresses one thing, a `$ref` from one schema into
//! another, a nested path, a field named like somebody else's key, a `links`
//! object. Every fact carries the rule that produced it and how much to trust
//! it, because inference that cannot explain itself is not usable on a real
//! document.
//!
//! The rules, strongest first:
//!
//! 1. [`Rule::CollectionItemPair`] — `/folders` returning `[Folder]` beside
//!    `/folders/{id}` returning `Folder`. The pair says `Folder` has identity
//!    and says which path parameter addresses it.
//! 2. [`Rule::SchemaRef`] — a `$ref` from one entity's schema into another.
//! 3. [`Rule::PathNesting`] — `/folders/{id}/items`, so the child carries the
//!    parent key.
//! 4. [`Rule::SpecLink`] — a `links` object, which states the relation and the
//!    field carrying it outright.
//! 5. [`Rule::ForeignKeyName`] — `user_id` where `User` is an entity. A name
//!    match is a guess, and is reported as one. It matches names, never
//!    meanings: `owner_id` finds an `Owner`, and nothing teaches it that an
//!    owner is a `User` — that is what a profile is for.
//! 6. [`Rule::VendorExtension`] — whatever the profile reads out of `x-` keys.

use lean_string::LeanString;
use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::Arc;

use super::document::{
    Operation, OperationTable, Property, SchemaBook, SchemaKind, SchemaNode, SchemaRef, Segment,
};
use super::schema::{Lens, fields_of};
use crate::core::world::model::{
    Cardinality, Carrier, CompositeKey, Confidence, EntityGraph, EntityType, FieldDef, KeyPart,
    KeySource, Provenance, Relation, Rule, ValueSpec,
};
use crate::profile::{ConsolidationProfile, DefaultProfile, SpecFieldContext};

/// What a response ultimately hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub entity: LeanString,
    /// Concrete entities behind a polymorphic response. Empty when concrete.
    pub members: Vec<LeanString>,
    pub is_list: bool,
    /// The envelope field the entities were dug out of, when the payload wraps
    /// them (`{ entries: [Folder], total_count: 42 }`).
    pub payload_field: Option<LeanString>,
}

/// An entity graph, and what had to be left out to build it.
#[derive(Debug, Default)]
pub struct Inference {
    pub graph: EntityGraph,
    /// Schemas a path addressed as an item but which nothing could key.
    pub skipped: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub schema: LeanString,
    pub reason: LeanString,
}

impl std::fmt::Display for Skipped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is not an entity: {}", self.schema, self.reason)
    }
}

/// Compile a document into an entity graph, with the built-in rules only.
#[must_use]
pub fn to_entity_graph(table: &OperationTable) -> EntityGraph {
    to_entity_graph_with(table, &DefaultProfile).graph
}

/// [`to_entity_graph`] with a profile consulted ahead of the built-in rules.
#[must_use]
pub fn to_entity_graph_with(
    table: &OperationTable,
    profile: &dyn ConsolidationProfile,
) -> Inference {
    let sightings = sightings_of(table);
    let (names, keys, mut skipped) = decide_entities(table, &sightings);

    let lens = Lens {
        book: &table.schemas,
        entities: &names,
        profile,
    };

    let mut graph = EntityGraph::new();
    // Sorted so the graph is built in the same order every run regardless of
    // hash-set iteration order; seeding depends on it.
    let mut ordered: Vec<&LeanString> = names.iter().collect();
    ordered.sort();

    for name in ordered {
        let Some(node) = table.schemas.effective(&SchemaRef::Named(name.clone())) else {
            continue;
        };
        let Some(key) = keys.get(name) else { continue };

        let mut entity = EntityType::new(
            name.clone(),
            key.key.clone(),
            Provenance::new(key.rule, key.detail.clone()),
        );
        entity.typename = Some(name.clone());
        entity.fields = fields_of(&lens, &node, name.as_str());

        apply_foreign_keys(&mut entity, &names, &node, key);
        apply_vendor_extensions(&mut entity, &names, &node, &table.schemas, profile);

        graph.insert(entity);
    }

    apply_path_nesting(&mut graph, table, &names, &sightings);
    apply_spec_links(&mut graph, table, &names, &keys);

    skipped.sort_by(|a, b| a.schema.cmp(&b.schema));
    Inference { graph, skipped }
}

// ===== Reading a response =====

/// The entity a schema hands back, unwrapping one envelope level.
///
/// `is_entity` decides what counts, so discovery can ask "any named schema"
/// before the entity set exists and binding can ask the real question after.
#[must_use]
pub fn target_of(
    book: &SchemaBook,
    reference: &SchemaRef,
    is_entity: &dyn Fn(&str) -> bool,
) -> Option<Target> {
    if let Some(target) = direct_target(book, reference, is_entity) {
        return Some(target);
    }

    // One envelope level, and only one: a payload with a single entity-shaped
    // field is a wrapper, and a payload with two is ambiguous in a way that
    // guessing would get wrong half the time.
    let node = book.effective(reference)?;
    if node.effective_kind() != SchemaKind::Object {
        return None;
    }

    let mut found: Option<Target> = None;
    for property in &node.properties {
        let Some(mut target) = direct_target(book, &property.schema, is_entity) else {
            continue;
        };
        if found.is_some() {
            return None;
        }
        target.payload_field = Some(property.name.clone());
        found = Some(target);
    }
    found
}

fn direct_target(
    book: &SchemaBook,
    reference: &SchemaRef,
    is_entity: &dyn Fn(&str) -> bool,
) -> Option<Target> {
    if let Some((entity, members)) = entity_ref(book, reference, is_entity) {
        return Some(Target {
            entity,
            members,
            is_list: false,
            payload_field: None,
        });
    }

    let node = book.effective(reference)?;
    if node.effective_kind() != SchemaKind::Array {
        return None;
    }
    let (entity, members) = entity_ref(book, node.items.as_ref()?, is_entity)?;
    Some(Target {
        entity,
        members,
        is_list: true,
        payload_field: None,
    })
}

/// The entity a reference names, directly or as a union of entities.
fn entity_ref(
    book: &SchemaBook,
    reference: &SchemaRef,
    is_entity: &dyn Fn(&str) -> bool,
) -> Option<(LeanString, Vec<LeanString>)> {
    if let Some(name) = reference.name()
        && is_entity(name.as_str())
    {
        return Some((name.clone(), Vec::new()));
    }

    let node = book.effective(reference)?;
    if node.one_of.is_empty() {
        return None;
    }
    let members: Vec<LeanString> = node
        .one_of
        .iter()
        .filter_map(|member| member.name())
        .filter(|name| is_entity(name.as_str()))
        .cloned()
        .collect();
    if members.len() != node.one_of.len() {
        return None;
    }
    members
        .first()
        .map(|first| (first.clone(), members.clone()))
}

// ===== Discovery =====

/// Where a schema was seen, which is what says whether it has identity.
#[derive(Debug, Default)]
struct Sighting {
    /// Operations addressing one instance, with the path parameter that does.
    items: Vec<ItemPath>,
    /// Operations handing back a list of them.
    collections: Vec<LeanString>,
    /// Operations handing back exactly one, at a path that addresses nothing.
    singletons: Vec<LeanString>,
}

#[derive(Debug, Clone)]
struct ItemPath {
    path: LeanString,
    param: LeanString,
    /// The literal segment naming the resource (`folders`).
    resource: Option<LeanString>,
    segments: Vec<Segment>,
}

fn sightings_of(table: &OperationTable) -> FxHashMap<LeanString, Sighting> {
    let any_named = |name: &str| table.schemas.contains(name);
    let mut sightings: FxHashMap<LeanString, Sighting> = FxHashMap::default();

    for operation in &table.operations {
        // A write says what a thing *is*, but only a read says how to address
        // it, and addressing is what identity means here.
        if operation.method != http::Method::GET {
            continue;
        }
        let Some(schema) = operation.success().and_then(|r| r.schema.as_ref()) else {
            continue;
        };
        let Some(target) = target_of(&table.schemas, schema, &any_named) else {
            continue;
        };

        let sighting = sightings.entry(target.entity).or_default();
        if target.is_list {
            sighting.collections.push(operation.path.clone());
        } else if let Some(Segment::Param(param)) = operation.segments.last() {
            sighting.items.push(ItemPath {
                path: operation.path.clone(),
                param: param.clone(),
                resource: operation.resource_segment().cloned(),
                segments: operation.segments.clone(),
            });
        } else {
            sighting.singletons.push(operation.path.clone());
        }
    }

    sightings
}

/// How an entity is addressed, and why we believe it is one.
#[derive(Debug, Clone)]
struct KeyChoice {
    key: CompositeKey,
    rule: Rule,
    detail: LeanString,
}

type Decision = (
    FxHashSet<LeanString>,
    FxHashMap<LeanString, KeyChoice>,
    Vec<Skipped>,
);

fn decide_entities(
    table: &OperationTable,
    sightings: &FxHashMap<LeanString, Sighting>,
) -> Decision {
    let mut names = FxHashSet::default();
    let mut keys = FxHashMap::default();
    let mut skipped = Vec::new();

    for (name, sighting) in sightings {
        let Some(node) = table.schemas.effective(&SchemaRef::Named(name.clone())) else {
            continue;
        };

        let choice = if let Some(item) = sighting.items.first() {
            let Some(field) = key_property(&node, &item.param, item.resource.as_deref()) else {
                skipped.push(Skipped {
                    schema: name.clone(),
                    reason: LeanString::from(format!(
                        "{} addresses one by `{}`, but the schema has no field holding it",
                        item.path, item.param
                    )),
                });
                continue;
            };
            KeyChoice {
                key: composite_key(&node, item, field),
                rule: Rule::CollectionItemPair,
                detail: LeanString::from(match sighting.collections.first() {
                    Some(collection) => format!("{collection} and {}", item.path),
                    None => item.path.to_string(),
                }),
            }
        } else {
            // Listed or returned but never addressed: still a thing worth
            // storing when it carries its own identifier, and the graph is what
            // makes `$ref`s into it resolve.
            let Some(field) = identifier_property(&node) else {
                continue;
            };
            let Some(seen) = sighting
                .collections
                .first()
                .or_else(|| sighting.singletons.first())
            else {
                continue;
            };
            KeyChoice {
                key: CompositeKey::single(field),
                rule: Rule::CollectionItemPair,
                detail: LeanString::from(format!("{seen} (no item path)")),
            }
        };

        names.insert(name.clone());
        keys.insert(name.clone(), choice);
    }

    (names, keys, skipped)
}

/// How an item path addresses one instance.
///
/// Usually one parameter does it. `/repos/{owner}/{repo}` does not: two owners
/// can each have a repo called `docs`, and keying on `repo` alone would make
/// them the same record. Every parameter of the path that names a field of the
/// schema is part of the key, and a path whose earlier parameters name nothing
/// keeps the ordinary single key.
fn composite_key(node: &SchemaNode, item: &ItemPath, last: LeanString) -> CompositeKey {
    let mut parts: Vec<KeyPart> = Vec::new();

    for segment in &item.segments {
        let Segment::Param(param) = segment else {
            continue;
        };
        if *param == item.param {
            break;
        }
        // Only a parameter the schema itself holds a field for: a path
        // parameter naming the *parent* of a nested resource is a link, which
        // path nesting already reads, not part of this thing's identity.
        let Some(field) = node
            .properties
            .iter()
            .find(|property| property.name.eq_ignore_ascii_case(param.as_str()))
            .map(|property| property.name.clone())
        else {
            continue;
        };
        parts.push(KeyPart {
            field,
            source: KeySource::PathParam(param.clone()),
        });
    }

    parts.push(KeyPart {
        field: last,
        source: KeySource::PathParam(item.param.clone()),
    });
    CompositeKey::parts(parts)
}

/// The property a path parameter addresses.
///
/// `/folders/{folder_id}` is keyed by `Folder.id`, `/users/{login}` by
/// `User.login`. The parameter names the property directly, or names it with
/// the resource in front of it, or the schema simply has an `id`.
fn key_property(node: &SchemaNode, param: &str, resource: Option<&str>) -> Option<LeanString> {
    if let Some(property) = node.property(param) {
        return Some(property.name.clone());
    }

    let mut candidates: Vec<String> = Vec::new();
    if let Some(tail) = param.rsplit(['_', '-']).next()
        && tail != param
    {
        candidates.push(tail.to_string());
    }
    if let Some(resource) = resource {
        let singular = singular_of(resource);
        for prefix in [format!("{singular}_"), singular] {
            if let Some(rest) = strip_prefix_ignore_case(param, &prefix)
                && !rest.is_empty()
            {
                candidates.push(rest.to_string());
            }
        }
    }
    candidates.push("id".to_string());

    candidates.into_iter().find_map(|candidate| {
        node.properties
            .iter()
            .find(|property| property.name.eq_ignore_ascii_case(&candidate))
            .map(|property| property.name.clone())
    })
}

/// The property that identifies an instance when no path parameter says.
fn identifier_property(node: &SchemaNode) -> Option<LeanString> {
    // A key addresses one instance, so an `id` the document declares as an
    // array is not one. Keying on it would write a single value where the
    // document promised a list.
    let addresses_one = |property: &&Property| {
        !matches!(&property.schema, SchemaRef::Inline(inner)
            if inner.kind == Some(SchemaKind::Array))
    };
    node.properties
        .iter()
        .filter(addresses_one)
        .find(|property| property.name == "id")
        .or_else(|| {
            node.properties
                .iter()
                .filter(addresses_one)
                .find(|property| property.name.eq_ignore_ascii_case("id"))
        })
        .map(|property| property.name.clone())
}

// ===== Relations =====

/// `user_id` where `User` is an entity.
///
/// The scalar *becomes* the relation rather than gaining a sibling: the store
/// writes a to-one link's value as the target's key, so a field that already
/// holds a key is the link, and adding a second field would put two answers to
/// one question in the payload.
fn apply_foreign_keys(
    entity: &mut EntityType,
    names: &FxHashSet<LeanString>,
    node: &SchemaNode,
    key: &KeyChoice,
) {
    let key_field = key.key.iter().next().map(|part| part.field.clone());
    let mut discovered: Vec<(LeanString, LeanString)> = Vec::new();

    for field in &mut entity.fields {
        if field.relation().is_some() || Some(&field.name) == key_field.as_ref() {
            continue;
        }
        let Some(stem) = key_suffixed(&field.name) else {
            continue;
        };
        let Some(target) = matching_entity(names, &stem) else {
            continue;
        };
        // A `$ref` in the schema already said what this is; only an untyped
        // scalar is a guess worth making.
        if !matches!(field.value, ValueSpec::Scalar(_)) {
            continue;
        }
        let _ = node;
        discovered.push((field.name.clone(), target));
    }

    for (name, target) in discovered {
        // A payload that carries `user_id` beside an embedded `customer` is
        // describing one link twice, not two links. Naming the scalar as that
        // link's carrier keeps the two spellings pointing at one instance —
        // left as separate relations they derive independently, and the object
        // and the key end up naming different users.
        if let Some(existing) = entity.fields.iter_mut().find(|field| {
            field
                .relation()
                .is_some_and(|relation| relation.target == target && !relation.is_carried())
        }) {
            if let Some(relation) = existing.value.relation_mut() {
                relation.carrier = Carrier::ForeignKey(name);
            }
            continue;
        }

        let Some(field) = entity.fields.iter_mut().find(|field| field.name == name) else {
            continue;
        };
        field.value = ValueSpec::Relation(Box::new(Relation::new(
            target,
            Cardinality::One,
            Carrier::ForeignKey(name.clone()),
            Confidence::HEURISTIC,
            Provenance::new(Rule::ForeignKeyName, format!("{}.{name}", entity.name)),
        )));
    }
}

/// The stem of a key-shaped field name: `owner_id` and `ownerId` both give
/// `owner`. `id` itself has no stem, which is what keeps a key from being read
/// as a link to something.
fn key_suffixed(name: &str) -> Option<String> {
    for suffix in ["_id", "_ids", "Id", "Ids", "ID", "IDs", "_key", "Key"] {
        if let Some(stem) = name.strip_suffix(suffix)
            && !stem.is_empty()
            && stem != "_"
        {
            return Some(stem.trim_end_matches('_').to_string());
        }
    }
    None
}

/// The entity a stem names, comparing the way people write names rather than
/// the way they are stored: `owner_id` finds `Owner`, `parent_folder_id` finds
/// `ParentFolder` or `Folder`.
fn matching_entity(names: &FxHashSet<LeanString>, stem: &str) -> Option<LeanString> {
    let flattened = flatten(stem);
    let singular = flatten(&singular_of(stem));

    let exact = names
        .iter()
        .find(|name| {
            let candidate = flatten(name.as_str());
            candidate == flattened || candidate == singular
        })
        .cloned();
    if exact.is_some() {
        return exact;
    }

    // `parent_folder_id` is a `Folder`; the qualifier says which one, not what.
    let tail = stem.rsplit(['_', '-']).next().unwrap_or(stem);
    if tail == stem {
        return None;
    }
    let tail = flatten(&singular_of(tail));
    names
        .iter()
        .find(|name| flatten(name.as_str()) == tail)
        .cloned()
}

fn flatten(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn singular_of(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        return format!("{stem}y");
    }
    for (suffix, kept) in [
        ("ses", "s"),
        ("xes", "x"),
        ("zes", "z"),
        ("ches", "ch"),
        ("shes", "sh"),
    ] {
        if let Some(stem) = name.strip_suffix(suffix) {
            return format!("{stem}{kept}");
        }
    }
    match name.strip_suffix('s') {
        Some(stem) if !stem.ends_with('s') && !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

fn strip_prefix_ignore_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
}

/// `/folders/{folder_id}/items` — the child carries the parent key.
fn apply_path_nesting(
    graph: &mut EntityGraph,
    table: &OperationTable,
    names: &FxHashSet<LeanString>,
    sightings: &FxHashMap<LeanString, Sighting>,
) {
    // Every item path that belongs to an entity, so a nested path can find its
    // parent by prefix.
    let mut parents: Vec<(&[Segment], &LeanString)> = Vec::new();
    for (name, sighting) in sightings {
        if !names.contains(name) {
            continue;
        }
        for item in &sighting.items {
            parents.push((item.segments.as_slice(), name));
        }
    }
    parents.sort_by(|a, b| a.1.cmp(b.1));

    let is_entity = |name: &str| names.contains(name);

    for operation in &table.operations {
        if operation.method != http::Method::GET || operation.segments.len() < 3 {
            continue;
        }
        let Some(Segment::Literal(field_name)) = operation.segments.last() else {
            continue;
        };
        let prefix = operation
            .segments
            .get(..operation.segments.len() - 1)
            .unwrap_or_default();
        let Some(Segment::Param(parent_param)) = prefix.last() else {
            continue;
        };
        let Some((_, parent)) = parents.iter().find(|(segments, _)| *segments == prefix) else {
            continue;
        };

        let Some(schema) = operation.success().and_then(|r| r.schema.as_ref()) else {
            continue;
        };
        let Some(target) = target_of(&table.schemas, schema, &is_entity) else {
            continue;
        };

        let Some(owner) = graph.get_mut(parent.as_str()) else {
            continue;
        };
        if owner.field(field_name.as_str()).is_some() {
            continue;
        }

        let relation = Relation::new(
            target.entity.clone(),
            if target.is_list {
                Cardinality::Many
            } else {
                Cardinality::One
            },
            Carrier::Subresource(parent_param.clone()),
            Confidence::STRUCTURAL,
            Provenance::new(Rule::PathNesting, operation.path.clone()),
        )
        .abstract_target(target.members.clone());

        let value = if target.is_list {
            ValueSpec::List(Box::new(ValueSpec::Relation(Box::new(relation))))
        } else {
            ValueSpec::Relation(Box::new(relation))
        };
        owner
            .fields
            .push(FieldDef::new(field_name.clone(), value, true));
    }
}

/// A `links` object states the relation and the field carrying it, which is
/// the only declared relation an OpenAPI document can hold.
fn apply_spec_links(
    graph: &mut EntityGraph,
    table: &OperationTable,
    names: &FxHashSet<LeanString>,
    keys: &FxHashMap<LeanString, KeyChoice>,
) {
    let is_entity = |name: &str| names.contains(name);

    for operation in &table.operations {
        let Some(response) = operation.success() else {
            continue;
        };
        if response.links.is_empty() {
            continue;
        }
        let Some(source) = response
            .schema
            .as_ref()
            .and_then(|schema| target_of(&table.schemas, schema, &is_entity))
            .filter(|target| !target.is_list)
        else {
            continue;
        };

        for link in &response.links {
            let Some(target_id) = link.operation_id.as_deref() else {
                continue;
            };
            let Some(target_op) = table.operation(target_id) else {
                continue;
            };
            let Some(target) = target_op
                .success()
                .and_then(|r| r.schema.as_ref())
                .and_then(|schema| target_of(&table.schemas, schema, &is_entity))
                .filter(|target| !target.is_list)
            else {
                continue;
            };

            // Only a link keyed off a *field* of this payload says something
            // about this entity. One keyed off its own id is a sub-resource,
            // which the path already said.
            let Some(carrier) = link
                .parameters
                .iter()
                .find_map(|(_, expression)| body_pointer(expression))
            else {
                continue;
            };
            if keys
                .get(&source.entity)
                .and_then(|key| key.key.iter().next())
                .is_some_and(|part| part.field == carrier)
            {
                continue;
            }

            let Some(entity) = graph.get_mut(source.entity.as_str()) else {
                continue;
            };
            let Some(field) = entity.fields.iter_mut().find(|f| f.name == carrier) else {
                continue;
            };

            field.value = ValueSpec::Relation(Box::new(
                Relation::new(
                    target.entity.clone(),
                    Cardinality::One,
                    Carrier::ForeignKey(LeanString::from(carrier.as_str())),
                    Confidence::DECLARED,
                    Provenance::new(
                        Rule::SpecLink,
                        format!("{} → {}", operation.path, link.name),
                    ),
                )
                .abstract_target(target.members),
            ));
        }
    }
}

/// The payload field a runtime expression reads: `$response.body#/owner_id`.
fn body_pointer(expression: &str) -> Option<String> {
    let pointer = expression.strip_prefix("$response.body#/")?;
    (!pointer.is_empty() && !pointer.contains('/')).then(|| pointer.to_string())
}

fn apply_vendor_extensions(
    entity: &mut EntityType,
    names: &FxHashSet<LeanString>,
    node: &SchemaNode,
    book: &SchemaBook,
    profile: &dyn ConsolidationProfile,
) {
    for property in &node.properties {
        let SchemaRef::Inline(property_node) = &property.schema else {
            continue;
        };
        if property_node.extensions.is_empty() {
            continue;
        }
        let Some(stated) = profile.spec_relation(&SpecFieldContext {
            owner: entity.name.as_str(),
            field: property.name.as_str(),
            extensions: &property_node.extensions,
        }) else {
            continue;
        };
        let Some(target) = matching_entity(names, &stated.target) else {
            continue;
        };
        let Some(field) = entity
            .fields
            .iter_mut()
            .find(|field| field.name == property.name)
        else {
            continue;
        };

        let carrier = stated.foreign_key.map_or(Carrier::Embedded, |key| {
            Carrier::ForeignKey(LeanString::from(key.as_str()))
        });
        let relation = Relation::new(
            target,
            if stated.many {
                Cardinality::Many
            } else {
                Cardinality::One
            },
            carrier,
            Confidence::DECLARED,
            Provenance::new(
                Rule::VendorExtension,
                format!("{}.{}", entity.name, property.name),
            ),
        );
        field.value = if stated.many {
            ValueSpec::List(Box::new(ValueSpec::Relation(Box::new(relation))))
        } else {
            ValueSpec::Relation(Box::new(relation))
        };
    }
    let _: &SchemaBook = book;
}

/// The entity an operation reads or writes, for a caller that already has the
/// graph. Shared with the REST binding so discovery and serving cannot drift.
#[must_use]
pub fn operation_target(
    table: &OperationTable,
    operation: &Operation,
    graph: &EntityGraph,
) -> Option<Target> {
    let is_entity = |name: &str| graph.contains(name);
    let response = operation.success()?;
    response
        .schema
        .as_ref()
        .and_then(|schema| target_of(&table.schemas, schema, &is_entity))
        .or_else(|| {
            operation
                .request_body
                .as_ref()
                .and_then(|schema| target_of(&table.schemas, schema, &is_entity))
        })
        // A write with no body at either end (`DELETE … -> 204 No Content`)
        // still acts on the entity its path addresses.
        .or_else(|| {
            entity_at_item_path(graph, operation).map(|entity| Target {
                entity,
                members: Vec::new(),
                is_list: false,
                payload_field: None,
            })
        })
}

/// The entity an item path addresses.
///
/// The key's source records which path parameter addresses it, so a path
/// ending in that parameter is addressing that entity — which is how an
/// operation with no payload at either end still knows what it acts on.
#[must_use]
pub fn entity_at_item_path(graph: &EntityGraph, operation: &Operation) -> Option<LeanString> {
    let Some(Segment::Param(param)) = operation.segments.last() else {
        return None;
    };
    let wanted = KeySource::PathParam(param.clone());
    let candidates: Vec<&EntityType> = graph
        .entities()
        .filter(|entity| entity.key.iter().any(|part| part.source == wanted))
        .collect();

    if let [only] = candidates.as_slice() {
        return Some(only.name.clone());
    }
    // Two resources can share a parameter name, and then the path's own
    // resource segment is what tells them apart.
    let resource = flatten(&singular_of(operation.resource_segment()?.as_str()));
    candidates
        .iter()
        .find(|entity| flatten(entity.name.as_str()) == resource)
        .map(|entity| entity.name.clone())
}

/// The parent a nested collection hangs off, when the graph says one does.
///
/// `/folders/{folder_id}/items` is a sub-collection because inference put an
/// `items` link carried by `folder_id` on `Folder`. `/folders/{folder_id}/copy`
/// has no such link, which is what makes it an action rather than a collection.
#[must_use]
pub fn subresource_parent<'a>(
    graph: &'a EntityGraph,
    operation: &Operation,
    child: Option<&LeanString>,
) -> Option<(&'a EntityType, LeanString, LeanString)> {
    let Some(Segment::Literal(field)) = operation.segments.last() else {
        return None;
    };
    let Some(Segment::Param(param)) = operation
        .segments
        .get(operation.segments.len().checked_sub(2)?)
    else {
        return None;
    };

    graph.entities().find_map(|entity| {
        let held = entity.field(field.as_str())?;
        let relation = held.relation()?;
        // Either inference put the link there because of this path, or the
        // schema declared the same collection inline. Both mean the sub-path
        // serves *that* link — a document that declares `files` on `Folder`
        // and also offers `/folders/{id}/files` is describing one relation
        // twice, and answering the path with every file in the world is the
        // one reading that is certainly wrong.
        let carried_here = relation.carrier == Carrier::Subresource(param.clone());
        let declared_here = relation.cardinality == Cardinality::Many && held.name == *field;
        if !carried_here && !declared_here {
            return None;
        }
        if child.is_some_and(|child| relation.target != *child) {
            return None;
        }
        Some((entity, param.clone(), field.clone()))
    })
}

/// The schemas a table declares, as `Arc`s the binding can hold.
#[must_use]
pub fn schema_arc(book: &SchemaBook, name: &str) -> Option<Arc<SchemaNode>> {
    book.effective(&SchemaRef::Named(LeanString::from(name)))
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
    use crate::spec::infer::openapi::document::parse_openapi;

    const FORGE: &str = r"
openapi: 3.0.3
info: { title: Forge }
paths:
  /repos:
    get:
      operationId: listRepos
      responses:
        '200':
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Repo' }
  /repos/{owner}/{repo}:
    parameters:
      - { name: owner, in: path, required: true, schema: { type: string } }
      - { name: repo, in: path, required: true, schema: { type: string } }
    get:
      operationId: getRepo
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Repo' }
components:
  schemas:
    Repo:
      type: object
      required: [owner, repo]
      properties:
        owner: { type: string }
        repo: { type: string }
";

    const ORDERS: &str = r"
openapi: 3.0.3
info: { title: Shop }
paths:
  /orders:
    get:
      operationId: listOrders
      responses:
        '200':
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Order' }
  /orders/{order_id}:
    parameters:
      - { name: order_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: getOrder
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Order' }
  /users/{user_id}:
    parameters:
      - { name: user_id, in: path, required: true, schema: { type: integer } }
    get:
      operationId: getUser
      responses:
        '200':
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
components:
  schemas:
    Order:
      type: object
      required: [id]
      properties:
        id: { type: integer }
        user_id: { type: integer }
        customer: { $ref: '#/components/schemas/User' }
    User:
      type: object
      properties:
        id: { type: integer }
";

    #[test]
    fn a_foreign_key_beside_an_embedded_object_is_one_link_not_two() {
        let table = parse_openapi(ORDERS).unwrap().0;
        let graph = to_entity_graph(&table);
        let order = graph.get("Order").expect("Order is an entity");

        let links: Vec<&str> = order
            .relations()
            .map(|(field, _)| field.name.as_str())
            .collect();
        assert_eq!(
            links,
            ["customer"],
            "`user_id` carries the link the `$ref` declared; it is not a second one"
        );

        let (_, relation) = order.relations().next().unwrap();
        assert_eq!(
            relation.carrier,
            Carrier::ForeignKey(LeanString::from("user_id")),
            "and the scalar is named as its carrier"
        );
        assert!(
            order.field("user_id").is_some(),
            "the declared scalar stays a field of the payload"
        );
    }

    #[test]
    fn a_path_addressing_one_thing_by_two_parameters_keys_on_both() {
        let table = parse_openapi(FORGE).unwrap().0;
        let graph = to_entity_graph(&table);
        let repo = graph.get("Repo").expect("Repo is an entity");
        let parts: Vec<&str> = repo.key.iter().map(|part| part.field.as_str()).collect();
        assert_eq!(
            parts,
            ["owner", "repo"],
            "two owners can each have a repo called `docs`"
        );
    }

    const FILESTORE: &str = r#"
openapi: 3.0.3
info: { title: Filestore }
paths:
  /folders:
    get:
      operationId: listFolders
      responses:
        "200":
          content:
            application/json:
              schema:
                type: object
                properties:
                  entries:
                    type: array
                    items: { $ref: '#/components/schemas/Folder' }
                  total_count: { type: integer }
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
      - { name: folder_id, in: path, required: true, schema: { type: string } }
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
              parameters: { user_id: "$response.body#/owned_by" }
    put:
      operationId: replaceFolder
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Folder' }
    delete:
      operationId: deleteFolder
      responses:
        "204": { description: gone }
  /folders/{folder_id}/items:
    parameters:
      - { name: folder_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: listFolderItems
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/File' }
  /users/{user_id}:
    parameters:
      - { name: user_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getUser
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/User' }
  /files/{file_id}:
    parameters:
      - { name: file_id, in: path, required: true, schema: { type: string } }
    get:
      operationId: getFile
      responses:
        "200":
          content:
            application/json:
              schema: { $ref: '#/components/schemas/File' }
  /events:
    get:
      operationId: listEvents
      responses:
        "200":
          content:
            application/json:
              schema:
                type: array
                items: { $ref: '#/components/schemas/Event' }
  /health:
    get:
      operationId: health
      responses:
        "200":
          content:
            application/json:
              schema: { type: object, properties: { ok: { type: boolean } } }
components:
  schemas:
    Folder:
      type: object
      required: [id, name]
      properties:
        id: { type: string }
        name: { type: string }
        user_id: { type: string }
        owned_by: { type: string }
        quota_id: { type: string }
        parent: { $ref: '#/components/schemas/Folder' }
        size: { type: integer }
    FolderInput:
      type: object
      properties:
        name: { type: string }
    User:
      type: object
      properties:
        id: { type: string }
        login: { type: string }
    File:
      type: object
      properties:
        id: { type: string }
        name: { type: string }
    Event:
      type: object
      properties:
        id: { type: string }
        kind: { type: string }
    Unaddressed:
      type: object
      properties:
        note: { type: string }
"#;

    fn filestore() -> (OperationTable, Inference) {
        let table = parse_openapi(FILESTORE).unwrap().0;
        let inference = to_entity_graph_with(&table, &DefaultProfile);
        (table, inference)
    }

    #[test]
    fn a_collection_and_an_item_path_make_an_entity() {
        let (_, inference) = filestore();
        let folder = inference.graph.get("Folder").expect("Folder is an entity");
        assert_eq!(folder.provenance.rule, Rule::CollectionItemPair);
        assert_eq!(
            folder.key.as_single().map(LeanString::as_str),
            Some("id"),
            "`folder_id` addresses the `id` field"
        );
    }

    #[test]
    fn a_path_parameter_is_recorded_as_the_key_source() {
        let (_, inference) = filestore();
        let folder = inference.graph.get("Folder").unwrap();
        let part = folder.key.iter().next().unwrap();
        assert_eq!(
            part.source,
            KeySource::PathParam(LeanString::from("folder_id"))
        );
    }

    #[test]
    fn an_input_only_schema_is_not_an_entity() {
        let (_, inference) = filestore();
        assert!(
            !inference.graph.contains("FolderInput"),
            "a request body is not a thing the world holds"
        );
        assert!(!inference.graph.contains("Unaddressed"));
    }

    #[test]
    fn a_list_only_schema_with_its_own_identifier_is_still_an_entity() {
        let (_, inference) = filestore();
        let event = inference.graph.get("Event").expect("Event is an entity");
        assert_eq!(event.key.as_single().map(LeanString::as_str), Some("id"));
    }

    #[test]
    fn an_envelope_is_unwrapped_to_find_the_collection() {
        let (table, inference) = filestore();
        let listing = table.operation("listFolders").unwrap();
        let target = operation_target(&table, listing, &inference.graph).unwrap();
        assert_eq!(target.entity.as_str(), "Folder");
        assert!(target.is_list);
        assert_eq!(target.payload_field.as_deref(), Some("entries"));
    }

    #[test]
    fn a_ref_between_entity_schemas_is_a_relation() {
        let (_, inference) = filestore();
        let parent = inference
            .graph
            .get("Folder")
            .unwrap()
            .field("parent")
            .unwrap();
        let relation = parent.relation().expect("parent should be a link");
        assert_eq!(relation.target.as_str(), "Folder");
        assert_eq!(relation.provenance.rule, Rule::SchemaRef);
    }

    #[test]
    fn a_field_named_like_another_entity_key_becomes_a_link() {
        let (_, inference) = filestore();
        let folder = inference.graph.get("Folder").unwrap();

        let relation = folder
            .field("user_id")
            .unwrap()
            .relation()
            .expect("user_id should be a link");
        assert_eq!(relation.target.as_str(), "User");
        assert_eq!(relation.provenance.rule, Rule::ForeignKeyName);
        assert_eq!(
            relation.carrier,
            Carrier::ForeignKey(LeanString::from("user_id")),
            "the field that already holds a key is the link"
        );
        assert!(
            relation.confidence < Confidence::STRUCTURAL,
            "a name match is a guess, and has to read as one"
        );

        assert!(
            folder.field("quota_id").unwrap().relation().is_none(),
            "a key-shaped name matching no entity stays an ordinary scalar"
        );
    }

    #[test]
    fn the_key_field_is_never_read_as_a_link_to_something() {
        let (_, inference) = filestore();
        assert!(
            inference
                .graph
                .get("Folder")
                .unwrap()
                .field("id")
                .unwrap()
                .relation()
                .is_none()
        );
    }

    #[test]
    fn a_nested_path_hangs_the_child_off_the_parent() {
        let (_, inference) = filestore();
        let items = inference
            .graph
            .get("Folder")
            .unwrap()
            .field("items")
            .expect("`/folders/{folder_id}/items` should reach Folder");
        let relation = items.relation().unwrap();
        assert_eq!(relation.target.as_str(), "File");
        assert_eq!(relation.cardinality, Cardinality::Many);
        assert_eq!(
            relation.carrier,
            Carrier::Subresource(LeanString::from("folder_id"))
        );
        assert_eq!(relation.provenance.rule, Rule::PathNesting);
    }

    #[test]
    fn a_links_object_beats_a_name_match() {
        let (_, inference) = filestore();
        let owned_by = inference
            .graph
            .get("Folder")
            .unwrap()
            .field("owned_by")
            .unwrap();
        let relation = owned_by.relation().expect("the link states a relation");
        assert_eq!(relation.target.as_str(), "User");
        assert_eq!(relation.provenance.rule, Rule::SpecLink);
        assert_eq!(relation.confidence, Confidence::DECLARED);
    }

    #[test]
    fn a_response_that_is_not_an_entity_yields_nothing() {
        let (table, inference) = filestore();
        let health = table.operation("health").unwrap();
        assert!(operation_target(&table, health, &inference.graph).is_none());
    }

    #[test]
    fn compiling_is_order_independent() {
        let a: Vec<String> = filestore()
            .1
            .graph
            .entities()
            .map(|e| e.name.to_string())
            .collect();
        let b: Vec<String> = filestore()
            .1
            .graph
            .entities()
            .map(|e| e.name.to_string())
            .collect();
        assert_eq!(a, b);
    }

    #[test]
    fn an_item_path_whose_schema_cannot_be_keyed_is_reported_not_dropped_silently() {
        let table = parse_openapi(
            "openapi: 3.0.0\ninfo: { title: t }\npaths:\n  /things/{thing_id}:\n    get:\n      operationId: getThing\n      parameters:\n        - { name: thing_id, in: path, required: true, schema: { type: string } }\n      responses:\n        \"200\":\n          content:\n            application/json:\n              schema: { $ref: '#/components/schemas/Thing' }\ncomponents:\n  schemas:\n    Thing:\n      type: object\n      properties:\n        label: { type: string }\n",
        )
        .unwrap()
        .0;
        let inference = to_entity_graph_with(&table, &DefaultProfile);

        assert!(!inference.graph.contains("Thing"));
        assert_eq!(inference.skipped.len(), 1);
        assert!(
            inference.skipped[0].to_string().contains("thing_id"),
            "unexpected: {}",
            inference.skipped[0]
        );
    }

    #[test]
    fn a_parameter_naming_the_property_outright_keys_the_entity() {
        let table = parse_openapi(
            "openapi: 3.0.0\ninfo: { title: t }\npaths:\n  /users/{login}:\n    get:\n      operationId: getUser\n      parameters:\n        - { name: login, in: path, required: true, schema: { type: string } }\n      responses:\n        \"200\":\n          content:\n            application/json:\n              schema: { $ref: '#/components/schemas/User' }\ncomponents:\n  schemas:\n    User:\n      type: object\n      properties:\n        login: { type: string }\n        name: { type: string }\n",
        )
        .unwrap()
        .0;
        let graph = to_entity_graph(&table);
        assert_eq!(
            graph
                .get("User")
                .unwrap()
                .key
                .as_single()
                .map(LeanString::as_str),
            Some("login")
        );
    }

    #[test]
    fn a_vendor_extension_the_profile_reads_states_a_relation() {
        struct FilestoreProfile;
        impl ConsolidationProfile for FilestoreProfile {
            fn name(&self) -> &'static str {
                "filestore"
            }
            fn spec_relation(
                &self,
                ctx: &SpecFieldContext<'_>,
            ) -> Option<crate::profile::SpecRelation> {
                ctx.extensions
                    .get("x-entity")
                    .and_then(serde_json::Value::as_str)
                    .map(|target| crate::profile::SpecRelation {
                        target: target.to_string(),
                        many: false,
                        foreign_key: Some(ctx.field.to_string()),
                    })
            }
        }

        let table = parse_openapi(
            "openapi: 3.0.0\ninfo: { title: t }\npaths:\n  /files/{file_id}:\n    get:\n      operationId: getFile\n      parameters:\n        - { name: file_id, in: path, required: true, schema: { type: string } }\n      responses:\n        \"200\":\n          content:\n            application/json:\n              schema: { $ref: '#/components/schemas/File' }\n  /users/{user_id}:\n    get:\n      operationId: getUser\n      parameters:\n        - { name: user_id, in: path, required: true, schema: { type: string } }\n      responses:\n        \"200\":\n          content:\n            application/json:\n              schema: { $ref: '#/components/schemas/User' }\ncomponents:\n  schemas:\n    File:\n      type: object\n      properties:\n        id: { type: string }\n        uploader: { type: string, x-entity: User }\n    User:\n      type: object\n      properties:\n        id: { type: string }\n",
        )
        .unwrap()
        .0;

        let plain = to_entity_graph(&table);
        assert!(
            plain
                .get("File")
                .unwrap()
                .field("uploader")
                .unwrap()
                .relation()
                .is_none(),
            "the engine must not read an extension it was never taught"
        );

        let informed = to_entity_graph_with(&table, &FilestoreProfile).graph;
        let uploader = informed.get("File").unwrap().field("uploader").unwrap();
        let relation = uploader.relation().expect("the profile stated a link");
        assert_eq!(relation.target.as_str(), "User");
        assert_eq!(relation.provenance.rule, Rule::VendorExtension);
    }

    #[test]
    fn plurals_and_case_conventions_both_find_the_entity() {
        let names: FxHashSet<LeanString> = ["User", "Folder"]
            .into_iter()
            .map(LeanString::from)
            .collect();
        assert_eq!(
            matching_entity(&names, "user").map(|n| n.to_string()),
            Some("User".to_string())
        );
        assert_eq!(
            matching_entity(&names, "users").map(|n| n.to_string()),
            Some("User".to_string())
        );
        assert_eq!(
            matching_entity(&names, "parent_folder").map(|n| n.to_string()),
            Some("Folder".to_string())
        );
        assert!(matching_entity(&names, "quota").is_none());
    }

    #[test]
    fn key_shaped_names_are_recognised_and_bare_ids_are_not() {
        assert_eq!(key_suffixed("owner_id").as_deref(), Some("owner"));
        assert_eq!(key_suffixed("ownerId").as_deref(), Some("owner"));
        assert!(key_suffixed("id").is_none());
        assert!(key_suffixed("valid").is_none());
    }
}
