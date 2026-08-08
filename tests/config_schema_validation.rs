use jsonschema::{Resource, Retrieve, Uri};
use serde_json::Value;
use std::{fmt, fs, path::Path, path::PathBuf, time::Duration};

/// Crate root, so fixture paths never depend on the process current
/// directory.
const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Original URI of the vendored models.dev document; the opencode schema
/// references it in four `$ref`s.
const MODELS_DEV_URI: &str = "https://models.dev/model-schema.json";

/// Path (relative to the crate root) of the vendored models.dev document.
const MODELS_DEV_SCHEMA_PATH: &str = "schemas/models-dev-model.schema.json";

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

/// Resolve a fixture path relative to the crate root.
fn fixture_path(rel: &str) -> PathBuf {
    Path::new(MANIFEST_DIR).join(rel)
}

/// Required JSON loader used by every schema check.
///
/// There is deliberately no skip path: a missing or malformed fixture is a
/// hard failure. Read errors and parse errors are distinct errors and both
/// carry the full path.
fn load_json_file(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|err| format!("failed to parse JSON {}: {err}", path.display()))
}

/// Retriever for hermetic local validation: any resource that was not
/// registered in memory is a hard failure. No network access, ever.
#[derive(Debug, Default, Clone, Copy)]
struct NoNetworkRetriever;

impl Retrieve for NoNetworkRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(Box::new(UnresolvedResourceError(format!(
            "local validation cannot fetch external resource {uri}"
        ))))
    }
}

#[derive(Debug)]
struct UnresolvedResourceError(String);

impl fmt::Display for UnresolvedResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UnresolvedResourceError {}

/// Compile a schema for local (offline) validation.
///
/// The vendored models.dev document is registered under its original URI so
/// the opencode schema's external `$ref`s resolve without network access;
/// any other external resource fails via [`NoNetworkRetriever`]. Claude Code
/// and Codex compile through the same helper (registering the unused models
/// resource is harmless and keeps one local path).
fn compile_local(schema: &Value) -> Result<jsonschema::Validator, String> {
    let models_dev = load_json_file(&fixture_path(MODELS_DEV_SCHEMA_PATH))?;
    // 0.49's `Resource::from_contents` is infallible; draft detection happens
    // during `Registry::prepare`.
    let resource = Resource::from_contents(models_dev);
    let registry = jsonschema::Registry::new()
        .add(MODELS_DEV_URI, resource)
        .map_err(|err| err.to_string())?
        .prepare()
        .map_err(|err| err.to_string())?;
    jsonschema::options()
        .with_registry(&registry)
        .with_retriever(NoNetworkRetriever)
        .build(schema)
        .map_err(|err| err.to_string())
}

/// Compile a schema with the hermetic retriever but WITHOUT the vendored
/// models.dev resource registered — proves the opencode schema fails closed
/// when its external resource is unavailable.
fn compile_local_without_models_resource(schema: &Value) -> Result<jsonschema::Validator, String> {
    jsonschema::options()
        .with_retriever(NoNetworkRetriever)
        .build(schema)
        .map_err(|err| err.to_string())
}

/// Bounded HTTPS-only schema fetcher for the ignored latest-upstream test.
///
/// Root schema fetches and transitive `$ref` retrieval share one configured
/// blocking client (30 s timeout, descriptive user agent). The retriever is
/// explicit rather than jsonschema's built-in HTTP resolver so jsonschema's
/// transport features stay off: enabling them would unify reqwest 0.13 TLS
/// features process-wide and break unrelated runtime clients (reqwest 0.13's
/// `rustls-no-provider` requires a crypto provider installed before building
/// a client). This path uses the project's own reqwest 0.12 blocking client,
/// which configures its own provider, so only this test touches the network.
const SCHEMA_FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const SCHEMA_FETCH_USER_AGENT: &str = "serial-mcp-schema-drift-check";

#[derive(Debug, Clone)]
struct SchemaFetchClient {
    client: reqwest::blocking::Client,
}

impl SchemaFetchClient {
    fn new() -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(SCHEMA_FETCH_USER_AGENT)
            .timeout(SCHEMA_FETCH_TIMEOUT)
            .build()
            .map_err(|err| format!("failed to build schema fetch client: {err}"))?;
        Ok(Self { client })
    }

    /// Enforce the HTTPS-only boundary before any network I/O: external
    /// schema documents are fetched over HTTPS or not at all.
    fn https_only(url: &str) -> Result<(), String> {
        if url.starts_with("https://") {
            Ok(())
        } else {
            Err(format!(
                "schema reference {url} is not HTTPS; external schema retrieval is HTTPS-only"
            ))
        }
    }

    /// Fetch and parse a schema document. Every failure keeps the URL plus
    /// transport, HTTP-status, or JSON context.
    fn fetch_value(&self, url: &str) -> Result<Value, String> {
        Self::https_only(url)?;
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|err| format!("failed to fetch schema {url}: {err}"))?;
        let status = response.status();
        let response = response
            .error_for_status()
            .map_err(|err| format!("schema URL returned HTTP error {url}: {err}"))?;
        let text = response
            .text()
            .map_err(|err| format!("failed to read schema body {url}: {err}"))?;
        serde_json::from_str(&text)
            .map_err(|err| format!("failed to parse schema JSON from {url} (HTTP {status}): {err}"))
    }
}

impl Retrieve for SchemaFetchClient {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        // The URI is resolved to an absolute reference by jsonschema before
        // retrieval; the HTTPS-only check still runs here so a non-HTTPS
        // external `$ref` fails before any request is attempted.
        let url = uri.as_str();
        Self::https_only(url).map_err(|err| Box::new(SchemaRetrievalError(err)))?;
        self.fetch_value(url).map_err(|err| {
            let boxed: Box<dyn std::error::Error + Send + Sync> =
                Box::new(SchemaRetrievalError(err));
            boxed
        })
    }
}

#[derive(Debug)]
struct SchemaRetrievalError(String);

impl fmt::Display for SchemaRetrievalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SchemaRetrievalError {}

fn validate_instance(
    name: &str,
    schema_ref: &str,
    compiled: &jsonschema::Validator,
    instance: &Value,
) {
    if compiled.is_valid(instance) {
        return;
    }

    let errors: Vec<String> = compiled
        .iter_errors(instance)
        .map(|e| format!("{e}"))
        .collect();

    let mut message =
        format!("{name} config is invalid\nschema: {schema_ref}\ninstance: {instance}\nerrors:\n");
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
    let cases = local_cases();
    // Exactly three vendored cases exist; a missing fixture must fail the
    // whole run, never skip.
    assert_eq!(cases.len(), 3, "vendored schema case list changed");
    for case in cases {
        let schema =
            load_json_file(&fixture_path(case.schema_path)).unwrap_or_else(|err| panic!("{err}"));
        let instance =
            load_json_file(&fixture_path(case.instance_path)).unwrap_or_else(|err| panic!("{err}"));
        let compiled = compile_local(&schema).unwrap_or_else(|err| {
            panic!(
                "invalid schema for {name} ({path}): {err}",
                name = case.name,
                path = case.schema_path
            )
        });
        validate_instance(case.name, case.schema_path, &compiled, &instance);
    }
}

#[test]
fn missing_required_schema_path_fails() {
    let path = "schemas/does-not-exist.schema.json";
    let err = load_json_file(&fixture_path(path)).expect_err("missing schema must fail");
    assert!(err.contains(path), "error must name the schema path: {err}");
    assert!(
        err.contains("failed to read"),
        "error must say read failed: {err}"
    );
}

#[test]
fn missing_required_instance_path_fails() {
    let path = "example-configs/does-not-exist.json";
    let err = load_json_file(&fixture_path(path)).expect_err("missing instance must fail");
    assert!(
        err.contains(path),
        "error must name the instance path: {err}"
    );
    assert!(
        err.contains("failed to read"),
        "error must say read failed: {err}"
    );
}

#[test]
fn malformed_json_fails_with_path_bearing_parse_error() {
    let path = std::env::temp_dir().join(format!(
        "serial-mcp-schema-test-malformed-{}.json",
        std::process::id()
    ));
    fs::write(&path, "{ not json").expect("write temp fixture");
    let err = load_json_file(&path).expect_err("malformed JSON must fail");
    let path_text = path.display().to_string();
    let _ = fs::remove_file(&path);
    assert!(
        err.contains(&path_text),
        "error must name the full path: {err}"
    );
    assert!(
        err.contains("failed to parse JSON"),
        "error must say parsing failed: {err}"
    );
}

#[test]
fn opencode_without_models_resource_fails_closed() {
    let schema = load_json_file(&fixture_path("schemas/opencode.schema.json"))
        .unwrap_or_else(|err| panic!("{err}"));
    let err = compile_local_without_models_resource(&schema)
        .expect_err("opencode must not compile without the models.dev resource registered");
    assert!(
        err.contains(MODELS_DEV_URI),
        "error must name the unresolved URI: {err}"
    );
}

#[test]
fn opencode_with_vendored_models_resource_compiles_and_validates() {
    let schema = load_json_file(&fixture_path("schemas/opencode.schema.json"))
        .unwrap_or_else(|err| panic!("{err}"));
    let instance = load_json_file(&fixture_path("example-configs/opencode.json"))
        .unwrap_or_else(|err| panic!("{err}"));
    let compiled = compile_local(&schema)
        .unwrap_or_else(|err| panic!("opencode must compile with vendored models.dev: {err}"));
    assert!(
        compiled.is_valid(&instance),
        "opencode example config must validate against the vendored schema"
    );
}

#[test]
#[ignore = "requires network; checks latest upstream schemas"]
fn example_configs_match_latest_upstream_schemas() {
    // Generic HTTPS transitive resolution: upstream schemas (e.g. opencode ->
    // models.dev) may `$ref` arbitrary remote documents, so the retriever
    // must fetch any transitive HTTPS URL. This uses an explicit bounded
    // retriever instead of enabling jsonschema's built-in transport features,
    // which would globally change reqwest's TLS setup and break runtime
    // clients; the local path stays fail-closed via `NoNetworkRetriever`.
    let client = SchemaFetchClient::new().unwrap_or_else(|err| panic!("{err}"));
    for case in remote_cases() {
        let schema = client
            .fetch_value(case.schema_url)
            .unwrap_or_else(|err| panic!("{err}"));
        // The local instance file is mandatory here too; only the schema is
        // allowed to come from the network.
        let instance =
            load_json_file(&fixture_path(case.instance_path)).unwrap_or_else(|err| panic!("{err}"));
        let compiled = jsonschema::options()
            .with_retriever(client.clone())
            .build(&schema)
            .unwrap_or_else(|err| {
                panic!(
                    "invalid schema for {} ({}): {err}",
                    case.name, case.schema_url
                )
            });
        validate_instance(case.name, case.schema_url, &compiled, &instance);
    }
}

#[test]
fn remote_retriever_rejects_non_https_references() {
    // Deterministic and network-free: the retriever must reject a plain-HTTP
    // external `$ref` before any request is attempted, proving the HTTPS-only
    // boundary of the network path.
    let client = SchemaFetchClient::new().unwrap_or_else(|err| panic!("{err}"));
    let schema = serde_json::json!({ "$ref": "http://example.invalid/transitive-schema.json" });
    let err = format!(
        "{}",
        jsonschema::options()
            .with_retriever(client)
            .build(&schema)
            .map(|_| ())
            .expect_err("non-HTTPS external $ref must fail schema compilation")
    );
    assert!(
        err.contains("http://example.invalid/transitive-schema.json"),
        "error must name the rejected URI: {err}"
    );
    assert!(
        err.contains("HTTPS-only"),
        "error must state the HTTPS-only policy: {err}"
    );
}
