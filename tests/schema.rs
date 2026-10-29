//! Validates real ferl JSON output against the published JSON Schema artifacts in
//! `schemas/`. The validator implements the keywords those schemas use: type, const,
//! enum, required, properties, additionalProperties, items, bounds, pattern, minLength,
//! allOf, if/then, and local $ref.

mod common;

use std::path::PathBuf;

use common::{Harness, KEY, MockServer, Reply, account_json, error_json, link_json, page_json};
use serde_json::{Value, json};

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas")
}

fn load(name: &str) -> Value {
    let text = std::fs::read_to_string(schema_dir().join(name)).expect("schema file");
    serde_json::from_str(&text).expect("schema JSON")
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        _ => false,
    }
}

fn validate(schema: &Value, instance: &Value, path: &str) -> Result<(), String> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = load(reference);
        return validate(&target, instance, path);
    }
    if let Some(expected) = schema.get("type") {
        let ok = match expected {
            Value::String(t) => type_matches(t, instance),
            Value::Array(list) => list
                .iter()
                .any(|t| t.as_str().is_some_and(|t| type_matches(t, instance))),
            _ => true,
        };
        if !ok {
            return Err(format!(
                "{path}: expected type {expected}, found {instance}"
            ));
        }
    }
    if let Some(constant) = schema.get("const") {
        if constant != instance {
            return Err(format!(
                "{path}: expected constant {constant}, found {instance}"
            ));
        }
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array) {
        if !options.contains(instance) {
            return Err(format!("{path}: {instance} is not in the allowed set"));
        }
    }
    let exact = instance
        .as_i64()
        .map(i128::from)
        .or_else(|| instance.as_u64().map(i128::from));
    if let Some(number) = exact {
        if let Some(min) = schema.get("minimum").and_then(Value::as_i64) {
            if number < i128::from(min) {
                return Err(format!("{path}: {number} is below {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(Value::as_i64) {
            if number > i128::from(max) {
                return Err(format!("{path}: {number} is above {max}"));
            }
        }
    }
    if let Some(text) = instance.as_str() {
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
            let regex = regex::Regex::new(pattern).expect("schema pattern");
            if !regex.is_match(text) {
                return Err(format!("{path}: {text:?} does not match {pattern}"));
            }
        }
        if let Some(min) = schema.get("minLength").and_then(Value::as_u64) {
            if (text.len() as u64) < min {
                return Err(format!("{path}: string is shorter than {min}"));
            }
        }
    }
    if let Some(object) = instance.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required {
                let name = field.as_str().expect("required name");
                if !object.contains_key(name) {
                    return Err(format!("{path}: missing required member {name}"));
                }
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        if schema.get("additionalProperties") == Some(&json!(false)) {
            for key in object.keys() {
                if !properties.is_some_and(|p| p.contains_key(key)) {
                    return Err(format!("{path}: unexpected member {key}"));
                }
            }
        }
        if let Some(properties) = properties {
            for (key, child_schema) in properties {
                if let Some(child) = object.get(key) {
                    validate(child_schema, child, &format!("{path}.{key}"))?;
                }
            }
        }
    }
    if let Some(list) = instance.as_array() {
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in list.iter().enumerate() {
                validate(item_schema, item, &format!("{path}[{index}]"))?;
            }
        }
    }
    if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
        for sub in all {
            if let Some(condition) = sub.get("if") {
                if validate(condition, instance, path).is_ok() {
                    if let Some(then) = sub.get("then") {
                        validate(then, instance, path)?;
                    }
                }
            } else {
                validate(sub, instance, path)?;
            }
        }
    }
    Ok(())
}

fn assert_valid(schema_name: &str, instance: &Value) {
    let schema = load(schema_name);
    if let Err(problem) = validate(&schema, instance, "$") {
        panic!("{schema_name} rejected the output: {problem}\n{instance:#}");
    }
}

fn profile_for(server: &MockServer) -> Harness {
    let harness = Harness::new();
    assert_eq!(harness.add_profile("work", &server.url(), KEY).status, 0);
    harness
}

#[test]
fn list_output_validates_against_the_result_and_link_schemas() {
    let links = vec![
        link_json("sch00001", Some("Docs"), "9007199254740993"),
        link_json("sch00002", None, "0"),
    ];
    let body = page_json(&links, None);
    let server = MockServer::start(move |_| Reply::json(200, &body));
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_valid("ferl-result-v1.schema.json", &run.json_stdout());
}

#[test]
fn partial_list_output_validates_and_its_error_record_validates() {
    let server = MockServer::start(|request| {
        if request.query_values("cursor").is_empty() {
            Reply::json(
                200,
                &page_json(&[link_json("sch00003", None, "1")], Some("next")),
            )
        } else {
            Reply::json(500, &error_json("internal_error", "boom", "req-x"))
        }
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list", "--all"]);
    assert_eq!(run.status, 12);
    assert_valid("ferl-result-v1.schema.json", &run.json_stdout());
    let record: Value =
        serde_json::from_str(run.stderr.lines().last().expect("error record")).expect("json");
    assert_valid("ferl-error-v1.schema.json", &record);
}

#[test]
fn create_and_delete_outputs_validate() {
    let created = format!(r#"{{"link":{}}}"#, link_json("sch00004", Some("T"), "0"));
    let server = MockServer::start(move |request| {
        if request.method == "POST" {
            Reply::json(201, &created)
        } else {
            Reply::Respond {
                status: 204,
                headers: vec![],
                body: vec![],
            }
        }
    });
    let harness = profile_for(&server);
    let create = harness.run_json(
        "work",
        &["create", "https://example.com/schema", "--no-copy"],
    );
    assert_eq!(create.status, 0, "{}", create.stderr);
    assert_valid("ferl-result-v1.schema.json", &create.json_stdout());
    let delete = harness.run_json("work", &["delete", "sch00004", "--yes"]);
    assert_eq!(delete.status, 0, "{}", delete.stderr);
    assert_valid("ferl-result-v1.schema.json", &delete.json_stdout());
}

#[test]
fn profile_check_output_validates() {
    let server = MockServer::start(|_| Reply::json(200, &account_json("ferl_cli_admin", "admin")));
    let harness = profile_for(&server);
    let run = harness.run(&["--output", "json", "profile", "check", "work"]);
    assert_eq!(run.status, 0, "{}", run.stderr);
    assert_valid("ferl-result-v1.schema.json", &run.json_stdout());
}

#[test]
fn error_records_for_rejections_validate() {
    let server = MockServer::start(|_| {
        Reply::json(401, &error_json("invalid_credentials", "No.", "req-401"))
    });
    let harness = profile_for(&server);
    let run = harness.run_json("work", &["list"]);
    assert_eq!(run.status, 4);
    let record: Value = serde_json::from_str(run.stderr.trim()).expect("json");
    assert_valid("ferl-error-v1.schema.json", &record);
}

#[test]
fn the_link_schema_rejects_counts_outside_the_exact_integer_range() {
    let mut overflow =
        serde_json::from_str::<Value>(&link_json("sch00005", None, "0")).expect("json");
    overflow["visit_count"] = json!(9223372036854775808_u64);
    let schema = load("ferl-link-v1.schema.json");
    assert!(validate(&schema, &overflow, "$").is_err());
    let mut missing = overflow.clone();
    missing["visit_count"] = json!(3);
    missing.as_object_mut().expect("object").remove("tags");
    assert!(validate(&schema, &missing, "$").is_err());
}
