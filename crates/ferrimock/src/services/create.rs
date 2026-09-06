//! Mock creation service — generate new mock definition files.

/// Input for creating a new mock.
#[derive(Debug, Clone)]
pub struct CreateInput {
    /// URL pattern to match
    pub url: String,
    /// HTTP method
    pub method: String,
    /// Response status code
    pub status: u16,
    /// Response body (JSON string or content)
    pub body: Option<String>,
    /// Generate template with fake data
    pub template: bool,
    /// Custom mock ID (auto-generated if None)
    pub id: Option<String>,
    /// Mock priority
    pub priority: u32,
    /// Collection name
    pub collection: Option<String>,
    /// Output format: "json" or "yaml"
    pub format: String,
    /// Mock kind: "http" (default), "ws", or "sse"
    pub kind: MockKind,
}

/// What kind of mock to scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MockKind {
    #[default]
    Http,
    Ws,
    Sse,
}

impl std::str::FromStr for MockKind {
    type Err = crate::FerrimockError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "http" => Ok(Self::Http),
            "ws" | "websocket" => Ok(Self::Ws),
            "sse" => Ok(Self::Sse),
            other => Err(crate::mp_err!(
                "Unknown mock kind '{other}': expected http, ws, or sse"
            )),
        }
    }
}

impl Default for CreateInput {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: "GET".into(),
            status: 200,
            body: None,
            template: false,
            id: None,
            priority: 100,
            collection: None,
            format: "yaml".into(),
            kind: MockKind::Http,
        }
    }
}

/// Result of mock creation.
#[derive(Debug, Clone)]
pub struct CreateResult {
    /// The generated mock ID
    pub mock_id: String,
    /// Serialized mock content (JSON or YAML)
    pub content: String,
}

/// Generate a mock ID from method and URL.
fn generate_mock_id(method: &str, url: &str) -> String {
    let slug = url
        .trim_start_matches('/')
        .replace('/', "-")
        .replace(':', "")
        .to_lowercase();
    format!("{}-{slug}", method.to_lowercase())
}

/// Create a new mock definition.
pub fn create(input: CreateInput) -> Result<CreateResult, crate::FerrimockError> {
    let mock_id = input.id.clone().unwrap_or_else(|| match input.kind {
        MockKind::Http => generate_mock_id(&input.method, &input.url),
        MockKind::Ws => generate_mock_id("ws", &input.url),
        MockKind::Sse => generate_mock_id("sse", &input.url),
    });

    let mock = match input.kind {
        MockKind::Http => {
            // An explicit body always wins (e.g. the wizard supplies an
            // edited template); otherwise generate a template body or
            // fall back to a default.
            let body = match (input.template, input.body) {
                (_, Some(b)) => b,
                (true, None) => generate_template_body(&input.method, &input.url),
                (false, None) => r#"{"message": "Mock response"}"#.to_string(),
            };
            let body_key = if input.template { "template" } else { "body" };
            serde_json::json!({
                "id": mock_id,
                "priority": input.priority,
                "enabled": true,
                "match": {
                    "method": input.method.to_uppercase(),
                    "url": input.url,
                },
                "response": {
                    "status": input.status,
                    "headers": {
                        "content-type": "application/json",
                    },
                    body_key: body,
                },
            })
        }
        MockKind::Ws => serde_json::json!({
            "id": mock_id,
            "priority": input.priority,
            "enabled": true,
            "match": {
                "method": "GET",
                "url": input.url,
            },
            "ws": {
                "on_connect": [
                    { "send": "welcome" },
                ],
                "on_message": [
                    {
                        "match": { "exact": "ping" },
                        "actions": [ { "send": "pong" } ],
                    },
                    {
                        "match": { "any": true },
                        "actions": [ "echo" ],
                    },
                ],
            },
        }),
        MockKind::Sse => serde_json::json!({
            "id": mock_id,
            "priority": input.priority,
            "enabled": true,
            "match": {
                "method": "GET",
                "url": input.url,
            },
            "sse": {
                "retry": 3000,
                "events": [
                    { "data": "hello" },
                    { "delay": "1s", "event": "tick", "id": "1", "data": { "count": 1 } },
                    { "delay": "1s", "event": "done", "data": "bye" },
                ],
            },
        }),
    };

    let collection = serde_json::json!({
        "mocks": [mock],
        "name": input.collection.as_deref().unwrap_or(&mock_id),
        "enabled": true,
    });

    let content = if input.format == "json" {
        serde_json::to_string_pretty(&collection)?
    } else {
        crate::config::json_yaml::to_yaml(&collection)?
    };

    Ok(CreateResult { mock_id, content })
}

/// Generate a template body with fake data based on endpoint heuristics.
pub fn generate_template_body(method: &str, url: &str) -> String {
    let is_list = url.ends_with('s')
        && !url.contains("/:") // e.g. /users but not /users/:id
        || url.contains("/list")
        || url.contains("/search");

    if is_list && method.eq_ignore_ascii_case("GET") {
        // List endpoint
        r#"{
  "data": [
    {% for i in range(end=5) %}
    {
      "id": "{{ fake_uuid() }}",
      "name": "{{ fake_name() }}",
      "email": "{{ fake_email() }}",
      "createdAt": "{{ fake_timestamp() }}"
    }{% if not loop.last %},{% endif %}
    {% endfor %}
  ],
  "total": 5
}"#
        .into()
    } else if method.eq_ignore_ascii_case("POST") {
        // Create endpoint
        r#"{
  "id": "{{ fake_uuid() }}",
  "createdAt": "{{ now() }}",
  "message": "Created successfully"
}"#
        .into()
    } else {
        // Single resource GET or other
        r#"{
  "id": "{{ fake_uuid() }}",
  "name": "{{ fake_name() }}",
  "email": "{{ fake_email() }}",
  "createdAt": "{{ fake_timestamp() }}"
}"#
        .into()
    }
}

#[cfg(test)]
mod template_body_tests {
    use super::generate_template_body;
    use crate::template::render_template;
    use crate::types::RequestContext;

    /// Every body `--template` writes has to render, or the mock it was
    /// written into fails to load. A function name that is not registered
    /// (`fake_iso_datetime` once was) shows up here, not at the user's shell.
    #[test]
    fn every_generated_template_renders_to_json() {
        let cases = [
            ("GET", "/api/users"),
            ("GET", "/api/users/:id"),
            ("GET", "/api/search"),
            ("POST", "/api/users"),
            ("DELETE", "/api/users/:id"),
        ];
        let mut failures = Vec::new();
        for (method, url) in cases {
            let body = generate_template_body(method, url);
            let rendered = match render_template(&body, &RequestContext::default()) {
                Ok(rendered) => rendered,
                Err(e) => {
                    failures.push(format!("{method} {url} does not render: {e}"));
                    continue;
                }
            };
            match serde_json::from_str::<serde_json::Value>(&rendered) {
                Ok(json) if json.get("id").is_some() || json.get("data").is_some() => {}
                Ok(json) => {
                    failures.push(format!("{method} {url} has neither id nor data: {json}"));
                }
                Err(e) => {
                    failures.push(format!("{method} {url} is not JSON: {e}\n{rendered}"));
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
