#![cfg(feature = "schema")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The generated JSON Schema is a shipped artifact, so something has to look at it.
//!
//! An editor extension generates `mock.schema.json` from `MockCollectionConfig`
//! for completion, and a type annotated `schemars(with = "Value")`
//! describes nothing — no completion, no check that a field exists, a typo caught
//! only when the world is built. `machines:` shipped that way for exactly one
//! commit because the annotation was copied from a neighbour without asking
//! whether it was still needed.

/// A definition with no properties and no other constraint describes nothing.
///
/// Some genuinely are opaque — a bag of user variables, a JSON body — and those
/// are named here rather than left to be discovered. Anything else that goes
/// shapeless fails, which is the point.
const DELIBERATELY_OPAQUE: [&str; 0] = [];

#[test]
fn nothing_in_the_published_schema_is_shapeless() {
    let schema = serde_json::to_value(schemars::schema_for!(
        ferrimock::config::MockCollectionConfig
    ))
    .expect("the schema serialises");

    let defs = schema
        .get("$defs")
        .and_then(serde_json::Value::as_object)
        .expect("a schema with definitions");

    let mut shapeless = Vec::new();
    for (name, def) in defs {
        if DELIBERATELY_OPAQUE.contains(&name.as_str()) {
            continue;
        }
        let Some(def) = def.as_object() else { continue };
        // Anything that says what it is, is fine: properties, a union, an
        // enumeration, a primitive type, a reference.
        let described = [
            "properties",
            "oneOf",
            "anyOf",
            "allOf",
            "enum",
            "const",
            "$ref",
            "items",
        ]
        .iter()
        .any(|key| def.contains_key(*key))
            || def.get("type").is_some_and(|held| held != "object");
        if !described {
            shapeless.push(name.clone());
        }
    }

    assert!(
        shapeless.is_empty(),
        "these describe nothing an editor can use: {shapeless:?}"
    );
}

/// The machine format specifically, because it is the newest and the reason
/// this file exists.
#[test]
fn the_machine_format_reaches_an_editor() {
    let schema = serde_json::to_value(schemars::schema_for!(
        ferrimock::config::MockCollectionConfig
    ))
    .expect("the schema serialises");
    let defs = schema.get("$defs").and_then(serde_json::Value::as_object);
    let defs = defs.expect("definitions");

    for named in ["MachineConfig", "StateConfig", "EdgeConfig", "StatesConfig"] {
        let def = defs
            .get(named)
            .unwrap_or_else(|| panic!("`{named}` is not in the published schema"));
        assert!(
            def.get("description").is_some(),
            "`{named}` reaches an editor with no description to hover"
        );
    }

    assert!(
        schema
            .get("properties")
            .and_then(|properties| properties.get("machines"))
            .is_some(),
        "`machines:` is not a documented top-level key"
    );
}
