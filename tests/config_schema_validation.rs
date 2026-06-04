use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug, Clone, Copy)]
struct LocalSchemaCase {
    name: &'static str,
    schema_path: &'static str,
    instance_path: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct RemoteSchemaCase {
    name: &'static str,
    schema_url: &'static str,
    instance_path: &'static str,
}

fn load_json_file(path: impl AsRef<Path>) -> Value {
    let path = path.as_ref();

    let text = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));

    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("failed to parse JSON {}: {err}", path.display()))
}

fn fetch_json(url: &str) -> Value {
    reqwest::blocking::get(url)
        .unwrap_or_else(|err| panic!("failed to fetch schema {url}: {err}"))
        .error_for_status()
        .unwrap_or_else(|err| panic!("schema URL returned HTTP error {url}: {err}"))
        .json()
        .unwrap_or_else(|err| panic!("failed to parse schema JSON from {url}: {err}"))
}

fn validate_json(name: &str, schema_ref: &str, schema: &Value, instance_path: &str) {
    let instance = load_json_file(instance_path);

    let compiled = jsonschema::validator_for(schema)
        .unwrap_or_else(|err| panic!("invalid schema for {name} ({schema_ref}): {err}"));

    if compiled.is_valid(&instance) {
        return;
    }

    let errors: Vec<String> = compiled
        .iter_errors(&instance)
        .map(|e| format!("{e}"))
        .collect();

    let mut message =
        format!("{name} config is invalid\nschema: {schema_ref}\nfile: {instance_path}\nerrors:\n");
    for error in &errors {
        message.push_str(&format!("  - {error}\n"));
    }
    panic!("{message}");
}

fn local_cases() -> [LocalSchemaCase; 3] {
    [
        LocalSchemaCase {
            name: "Claude Code",
            schema_path: "schemas/claude-code-settings.schema.json",
            instance_path: "example-configs/claude_code.json",
        },
        LocalSchemaCase {
            name: "OpenAI Codex",
            schema_path: "schemas/codex-config.schema.json",
            instance_path: "example-configs/codex.json",
        },
        LocalSchemaCase {
            name: "opencode",
            schema_path: "schemas/opencode.schema.json",
            instance_path: "example-configs/opencode.json",
        },
    ]
}

fn remote_cases() -> [RemoteSchemaCase; 3] {
    [
        RemoteSchemaCase {
            name: "Claude Code",
            schema_url: "https://json.schemastore.org/claude-code-settings.json",
            instance_path: "example-configs/claude_code.json",
        },
        RemoteSchemaCase {
            name: "OpenAI Codex",
            schema_url: "https://developers.openai.com/codex/config-schema.json",
            instance_path: "example-configs/codex.json",
        },
        RemoteSchemaCase {
            name: "opencode",
            schema_url: "https://opencode.ai/config.json",
            instance_path: "example-configs/opencode.json",
        },
    ]
}

#[test]
fn example_configs_match_vendored_schemas() {
    for case in local_cases() {
        let schema = load_json_file(case.schema_path);
        validate_json(case.name, case.schema_path, &schema, case.instance_path);
    }
}

#[test]
#[ignore = "requires network; checks latest upstream schemas"]
fn example_configs_match_latest_upstream_schemas() {
    for case in remote_cases() {
        let schema = fetch_json(case.schema_url);
        validate_json(case.name, case.schema_url, &schema, case.instance_path);
    }
}
