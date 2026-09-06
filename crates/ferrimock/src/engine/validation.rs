//! Mock validation module with comprehensive error reporting
//!
//! This module provides validation for mock collection configurations,
//! catching errors before runtime with helpful, compiler-style error messages.

use crate::config::MockCollectionConfig;
use crate::engine::types::MockDefinition;
use crate::template;
use http::Method;
use http::header::HeaderName;
use lean_string::LeanString;
use regex::Regex;
use rustc_hash::FxHashSet;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Validator for mock configurations
pub struct MockValidator {
    /// Configuration options
    #[cfg_attr(not(test), allow(dead_code))]
    config: ValidatorConfig,
}

/// Validator configuration
#[derive(Debug, Clone)]
struct ValidatorConfig {
    /// Check for file existence
    #[allow(dead_code)]
    check_files: bool,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self { check_files: true }
    }
}

impl MockValidator {
    /// Create a new validator with default configuration
    pub fn new() -> Self {
        Self {
            config: ValidatorConfig::default(),
        }
    }

    /// Validate a single file
    pub async fn validate_file(&self, path: &Path) -> ValidationResult {
        let file_path = Some(path.to_path_buf());

        // Read file content
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                return ValidationResult {
                    file_path,
                    errors: vec![ValidationError {
                        mock_id: None,
                        error_type: ErrorType::FileReadError,
                        message: format!("Failed to read file: {e}"),
                        snippet: None,
                        suggestion: None,
                        line_number: None,
                    }],
                    warnings: vec![],
                    handler_count: None,
                };
            }
        };

        // Determine format from extension
        let extension = path.extension().and_then(|e| e.to_str());
        let config_result = match extension {
            Some("json") => serde_json::from_str::<MockCollectionConfig>(&content)
                .map_err(|e| crate::mp_err!("JSON parse error: {e}")),
            Some("yaml" | "yml") => serde_yaml_ng::from_str::<MockCollectionConfig>(&content)
                .map_err(|e| crate::mp_err!("YAML parse error: {e}")),
            Some("js" | "mjs" | "ts" | "mts") => {
                return self.validate_script_file(path).await;
            }
            _ => {
                return ValidationResult {
                    file_path,
                    errors: vec![ValidationError {
                        mock_id: None,
                        error_type: ErrorType::UnsupportedFormat,
                        message: format!("Unsupported file format: {extension:?}"),
                        snippet: None,
                        suggestion: Some(
                            "Use .json, .yaml, .yml, .js, .mjs, .ts, or .mts extension".to_string(),
                        ),
                        line_number: None,
                    }],
                    warnings: vec![],
                    handler_count: None,
                };
            }
        };

        match config_result {
            Ok(config) => {
                // Config parsed successfully, now validate deeply
                self.validate_config_internal(&config, path.parent(), &content, file_path)
                    .await
            }
            Err(e) => {
                // Parse error - extract line number if possible
                let (line_number, snippet) =
                    Self::extract_parse_error_info(&e.to_string(), &content);

                ValidationResult {
                    file_path,
                    errors: vec![ValidationError {
                        mock_id: None,
                        error_type: ErrorType::ParseError,
                        message: e.to_string(),
                        snippet,
                        suggestion: Some("Check syntax according to the file format".to_string()),
                        line_number,
                    }],
                    warnings: vec![],
                    handler_count: None,
                }
            }
        }
    }

    /// Validate a script mock file (`.js`/`.mjs`/`.ts`/`.mts`) by
    /// bundling and evaluating it on the QuickJS engine — the same path
    /// `mock serve` uses to load it. Errors carry source locations
    /// remapped through the bundle's sourcemap.
    #[cfg(feature = "scripting")]
    async fn validate_script_file(&self, path: &Path) -> ValidationResult {
        let host = crate::scripting::ScriptHost::new();
        match host.load_file(path, None).await {
            Ok(defs) => {
                let warnings = if defs.is_empty() {
                    vec![ValidationWarning {
                        mock_id: None,
                        message: "Script evaluated but registered no handlers".to_string(),
                        warning_type: WarningType::EmptyScript,
                        line_number: None,
                        snippet: None,
                        suggestion: Some(
                            "Register handlers with http.*/graphql.*/ws.*/sse calls".to_string(),
                        ),
                    }]
                } else {
                    vec![]
                };
                ValidationResult {
                    file_path: Some(path.to_path_buf()),
                    errors: vec![],
                    warnings,
                    handler_count: Some(defs.len()),
                }
            }
            Err(e) => ValidationResult {
                file_path: Some(path.to_path_buf()),
                errors: vec![ValidationError {
                    mock_id: None,
                    error_type: ErrorType::ScriptError,
                    message: e.to_string(),
                    snippet: None,
                    suggestion: None,
                    line_number: None,
                }],
                warnings: vec![],
                handler_count: None,
            },
        }
    }

    #[cfg(not(feature = "scripting"))]
    async fn validate_script_file(&self, path: &Path) -> ValidationResult {
        ValidationResult {
            file_path: Some(path.to_path_buf()),
            errors: vec![ValidationError {
                mock_id: None,
                error_type: ErrorType::ScriptError,
                message: "Script mock validation requires the `scripting` feature".to_string(),
                snippet: None,
                suggestion: Some(
                    "Build ferrimock with the `scripting` feature to validate script mocks"
                        .to_string(),
                ),
                line_number: None,
            }],
            warnings: vec![],
            handler_count: None,
        }
    }

    /// Validate a configuration with optional config directory
    pub async fn validate_config(
        &self,
        config: &MockCollectionConfig,
        config_dir: Option<&Path>,
    ) -> ValidationResult {
        self.validate_config_internal(config, config_dir, "", None)
            .await
    }

    /// Validate content from a string (for stdin mode).
    ///
    /// `extension` should be "json", "yaml", or "yml".
    pub async fn validate_content(&self, content: &str, extension: &str) -> ValidationResult {
        let config_result = match extension {
            "json" => serde_json::from_str::<MockCollectionConfig>(content)
                .map_err(|e| crate::mp_err!("JSON parse error: {e}")),
            "yaml" | "yml" => serde_yaml_ng::from_str::<MockCollectionConfig>(content)
                .map_err(|e| crate::mp_err!("YAML parse error: {e}")),
            _ => {
                return ValidationResult {
                    file_path: None,
                    errors: vec![ValidationError {
                        mock_id: None,
                        error_type: ErrorType::UnsupportedFormat,
                        message: format!("Unsupported format: {extension}"),
                        snippet: None,
                        suggestion: Some("Use json, yaml, or yml".to_string()),
                        line_number: None,
                    }],
                    warnings: vec![],
                    handler_count: None,
                };
            }
        };

        match config_result {
            Ok(config) => {
                self.validate_config_internal(&config, None, content, None)
                    .await
            }
            Err(e) => {
                let (line_number, snippet) =
                    Self::extract_parse_error_info(&e.to_string(), content);
                ValidationResult {
                    file_path: None,
                    errors: vec![ValidationError {
                        mock_id: None,
                        error_type: ErrorType::ParseError,
                        message: e.to_string(),
                        snippet,
                        suggestion: Some("Check syntax according to the file format".to_string()),
                        line_number,
                    }],
                    warnings: vec![],
                    handler_count: None,
                }
            }
        }
    }

    /// Validate all files in a directory
    pub async fn validate_directory(&self, dir: &Path) -> Vec<ValidationResult> {
        let mut results = Vec::new();

        // Read directory
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                results.push(ValidationResult {
                    file_path: Some(dir.to_path_buf()),
                    errors: vec![ValidationError {
                        mock_id: None,
                        error_type: ErrorType::FileReadError,
                        message: format!("Failed to read directory: {e}"),
                        snippet: None,
                        suggestion: None,
                        line_number: None,
                    }],
                    warnings: vec![],
                    handler_count: None,
                });
                return results;
            }
        };

        // Collect all valid config files
        for entry in read_dir.filter_map(std::result::Result::ok) {
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let extension = path.extension().and_then(|e| e.to_str());
            if matches!(
                extension,
                Some("json" | "yaml" | "yml" | "js" | "mjs" | "ts" | "mts")
            ) {
                results.push(self.validate_file(&path).await);
            }
        }

        results
    }

    /// Internal validation with full context
    #[allow(clippy::match_same_arms, clippy::indexing_slicing)] // Keeping explicit arms for clarity in validation logic; loop indices are bounds-safe
    async fn validate_config_internal(
        &self,
        config: &MockCollectionConfig,
        config_dir: Option<&Path>,
        file_content: &str,
        file_path: Option<PathBuf>,
    ) -> ValidationResult {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Track seen mock IDs for duplicate detection
        let mut seen_ids = FxHashSet::default();

        // Validate each mock
        for mock in &config.mocks {
            let mock_id = Some(mock.id.clone());

            // Check for duplicate IDs
            if !seen_ids.insert(&mock.id) {
                let line_number = Self::find_line_number(file_content, &mock.id);
                warnings.push(ValidationWarning {
                    mock_id: mock_id.clone(),
                    message: format!("Duplicate mock ID: '{}'", mock.id),
                    warning_type: WarningType::DuplicateId,
                    line_number,
                    snippet: line_number
                        .and_then(|line| Self::extract_snippet(file_content, line, &mock.id)),
                    suggestion: Some("Each mock should have a unique ID".to_string()),
                });
            }

            // Check if disabled
            if !mock.enabled {
                let line_number = Self::find_line_number(file_content, &mock.id);
                warnings.push(ValidationWarning {
                    mock_id: mock_id.clone(),
                    message: "Mock is disabled".to_string(),
                    warning_type: WarningType::DisabledMock,
                    line_number,
                    snippet: None,
                    suggestion: Some("Set enabled = true or remove the mock".to_string()),
                });
            }

            // Validate streaming (ws/sse) configuration
            if mock.sse.is_some() || mock.ws.is_some() {
                let mut conflict = |message: &str| {
                    let line_number = Self::find_line_number(file_content, &mock.id);
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::ConflictingModes,
                        message: message.to_string(),
                        snippet: None,
                        suggestion: Some(
                            "A ws/sse mock may only combine `match`, `response.headers`, and its own block"
                                .to_string(),
                        ),
                        line_number,
                    });
                };
                if mock.sse.is_some() && mock.ws.is_some() {
                    conflict("Cannot combine `sse` and `ws` in one mock");
                }
                if mock.patch.is_some() {
                    conflict("Cannot combine `sse`/`ws` with `patch`");
                }
                if mock.delay.is_some() {
                    conflict("Cannot combine `sse`/`ws` with a top-level `delay`");
                }
                if mock.request.as_ref().is_some_and(|r| !r.is_empty()) {
                    conflict("Cannot combine `sse`/`ws` with request transforms");
                }
                if mock
                    .response_config
                    .as_ref()
                    .is_some_and(crate::config::response::ResponseConfig::is_full_mock)
                {
                    conflict("Cannot combine `sse`/`ws` with a full mock response body");
                }

                let script_error = match (&mock.sse, &mock.ws) {
                    (Some(sse), _) => sse.clone().into_script().err(),
                    (None, Some(ws)) => ws.clone().into_script().err(),
                    (None, None) => None,
                };
                if let Some(e) = script_error {
                    let line_number = Self::find_line_number(file_content, &mock.id);
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::InvalidStreamingConfig,
                        message: e.to_string(),
                        snippet: None,
                        suggestion: None,
                        line_number,
                    });
                }
            }

            // Validate match configuration
            if let Some(ref match_config) = mock.match_config {
                // Validate HTTP methods
                let mut all_methods = match_config.methods.clone();
                if let Some(ref method) = match_config.method {
                    all_methods.push(method.clone());
                }

                for method_str in &all_methods {
                    if let Err(e) = Method::from_str(method_str) {
                        let line_number = Self::find_line_number(file_content, method_str);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidMethod,
              message: format!("Invalid HTTP method '{method_str}': {e}"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, method_str)),
              suggestion: Some(
                "Valid methods are: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS, TRACE, CONNECT".to_string(),
              ),
              line_number,
            });
                    }
                }

                // Validate URL patterns (check regex patterns)
                let mut all_urls = match_config.urls.clone();
                if let Some(ref url) = match_config.url {
                    all_urls.push(url.clone());
                }

                for url_pattern in &all_urls {
                    // Ask the parser the loader will use, rather than guessing
                    // again here: an `exact:`-prefixed URL is a literal however
                    // many brackets it holds, and a query string carrying PHP
                    // array params (`?ids[]=1`) is not a character class.
                    if let Err(e) = crate::config::parse_url_pattern(url_pattern) {
                        let line_number = Self::find_line_number(file_content, url_pattern);
                        errors.push(ValidationError {
                mock_id: mock_id.clone(),
                error_type: ErrorType::InvalidRegex,
                message: format!("Invalid URL pattern: {e}"),
                snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, url_pattern)),
                suggestion: Some(
                  "Check pattern syntax: brackets, parentheses, and special characters must be balanced".to_string(),
                ),
                line_number,
              });
                    }
                }

                // Validate header names
                for header_name in match_config.headers.keys() {
                    if HeaderName::from_str(header_name).is_err() {
                        let line_number = Self::find_line_number(file_content, header_name);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidHeaderName,
              message: format!("Invalid header name: '{header_name}'"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, header_name)),
              suggestion: Some("Header names must be valid HTTP header names (alphanumeric and hyphens)".to_string()),
              line_number,
            });
                    }
                }
            } else {
                errors.push(ValidationError {
                    mock_id: mock_id.clone(),
                    error_type: ErrorType::MissingField,
                    message: "Missing 'match' configuration".to_string(),
                    snippet: None,
                    suggestion: Some(
                        "Add match configuration: match.url or match.methods".to_string(),
                    ),
                    line_number: None,
                });
            }

            // Validate return configuration
            if let Some(ref response_config) = mock.response_config {
                // Check status code if present
                if let Some(status) = response_config.status()
                    && !(100..=599).contains(&status)
                {
                    let status_str = status.to_string();
                    let line_number = Self::find_line_number(file_content, &status_str);
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::InvalidStatusCode,
                        message: format!("Invalid HTTP status code: {status}"),
                        snippet: line_number.and_then(|line| {
                            Self::extract_snippet(file_content, line, &status_str)
                        }),
                        suggestion: Some("Status code must be between 100 and 599".to_string()),
                        line_number,
                    });
                }

                // Validate mutual exclusivity of body type fields
                {
                    let mut body_field_count = 0;
                    let mut body_field_names = Vec::new();
                    if response_config.body().is_some() {
                        body_field_count += 1;
                        body_field_names.push("body");
                    }
                    if response_config.template().is_some() {
                        body_field_count += 1;
                        body_field_names.push("template");
                    }
                    if response_config.file_ref().is_some() {
                        body_field_count += 1;
                        body_field_names.push("file");
                    }
                    if response_config.template_file_ref().is_some() {
                        body_field_count += 1;
                        body_field_names.push("template_file");
                    }
                    if response_config.json().is_some_and(|j| !j.is_null()) {
                        body_field_count += 1;
                        body_field_names.push("json");
                    }
                    if body_field_count > 1 {
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::MutuallyExclusiveFields,
              message: format!(
                "Multiple response body fields set: {}. Only one of body, template, file, template_file, json may be used.",
                body_field_names.join(", ")
              ),
              snippet: None,
              suggestion: Some("Remove all but one response body field.".to_string()),
              line_number: None,
            });
                    }
                }

                // Validate template (inline Tera template - always validated)
                if let Some(tmpl) = response_config.template()
                    && let Err(e) = template::validate_template(tmpl)
                {
                    let line_number = Self::find_line_number(file_content, tmpl);
                    errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::TemplateError,
              message: format!("Template validation failed: {e}"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, "{{")),
              suggestion: Some(
                "Check template syntax: variables use {{ var }}, control flow uses {% if %}".to_string(),
              ),
              line_number,
            });
                }

                // Validate template_file (load file, check existence, validate as template)
                if let Some(tf_ref) = response_config.template_file_ref()
                    && let Some(dir) = config_dir
                {
                    let full_path = dir.join(tf_ref);
                    if full_path.exists() {
                        // Validate template file content
                        if let Ok(template_content) = std::fs::read_to_string(&full_path)
                            && let Err(e) = template::validate_template(&template_content)
                        {
                            errors.push(ValidationError {
                                mock_id: mock_id.clone(),
                                error_type: ErrorType::TemplateError,
                                message: format!("Template file '{tf_ref}' validation failed: {e}"),
                                snippet: None,
                                suggestion: Some("Check template file syntax".to_string()),
                                line_number: None,
                            });
                        }
                    } else {
                        let line_number = Self::find_line_number(file_content, tf_ref);
                        errors.push(ValidationError {
                            mock_id: mock_id.clone(),
                            error_type: ErrorType::FileNotFound,
                            message: format!("Referenced template file does not exist: {tf_ref}"),
                            snippet: line_number
                                .and_then(|line| Self::extract_snippet(file_content, line, tf_ref)),
                            suggestion: Some(format!("Expected file at: {}", full_path.display())),
                            line_number,
                        });
                    }
                }

                // Validate file (check existence only, no template processing)
                if let Some(file_ref) = response_config.file_ref()
                    && let Some(dir) = config_dir
                {
                    let full_path = dir.join(file_ref);
                    if !full_path.exists() {
                        let line_number = Self::find_line_number(file_content, file_ref);
                        errors.push(ValidationError {
                            mock_id: mock_id.clone(),
                            error_type: ErrorType::FileNotFound,
                            message: format!("Referenced file does not exist: {file_ref}"),
                            snippet: line_number.and_then(|line| {
                                Self::extract_snippet(file_content, line, file_ref)
                            }),
                            suggestion: Some(format!("Expected file at: {}", full_path.display())),
                            line_number,
                        });
                    }
                }
            } else {
                let has_request_transforms = mock.request.as_ref().is_some_and(|r| !r.is_empty());
                let has_patch = mock.patch.is_some();
                let has_delay = mock.delay.is_some();
                let has_streaming = mock.sse.is_some() || mock.ws.is_some();
                let has_network_error = mock.network_error == Some(true);
                if !has_request_transforms
                    && !has_patch
                    && !has_delay
                    && !has_streaming
                    && !has_network_error
                {
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::MissingField,
                        message: "Missing 'response' configuration".to_string(),
                        snippet: None,
                        suggestion: Some(
                            "Add response, sse, ws, patch, delay, network_error, or \
                             request transform configuration"
                                .to_string(),
                        ),
                        line_number: None,
                    });
                }
            }

            // Validate top-level delay
            if let Some(ref delay) = mock.delay
                && let Err(_e) = crate::config::response::parse_duration(delay)
            {
                let line_number = Self::find_line_number(file_content, delay);
                errors.push(ValidationError {
                    mock_id: mock_id.clone(),
                    error_type: ErrorType::InvalidDuration,
                    message: format!("Invalid top-level delay duration: '{delay}'"),
                    snippet: line_number
                        .and_then(|line| Self::extract_snippet(file_content, line, delay)),
                    suggestion: Some(
                        "Duration must have a unit suffix: '100ms', '2s', '500us'".to_string(),
                    ),
                    line_number,
                });
            }

            // Validate top-level patch configuration
            if let Some(ref patches_config) = mock.patch {
                // Validate jsonpath values that contain template expressions
                for (path, value) in &patches_config.jsonpath {
                    if let Some(s) = value.as_str()
                        && (s.contains("{{") || s.contains("{%"))
                        && let Err(e) = template::validate_template(s)
                    {
                        let line_number = Self::find_line_number(file_content, s);
                        errors.push(ValidationError {
                  mock_id: mock_id.clone(),
                  error_type: ErrorType::TemplateError,
                  message: format!("Invalid template in patch jsonpath '{path}': {e}"),
                  snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, "{{")),
                  suggestion: Some(
                    "Check template syntax: variables use {{ var }}, functions use {{ func() }}".to_string(),
                  ),
                  line_number,
                });
                    }
                }

                // Validate regex patterns and replacement values
                for regex_config in &patches_config.regex {
                    // Validate regex pattern itself
                    if let Err(e) = Regex::new(&regex_config.pattern) {
                        let line_number =
                            Self::find_line_number(file_content, &regex_config.pattern);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidPatchRegex,
              message: format!("Invalid regex pattern in patch: {e}"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, &regex_config.pattern)),
              suggestion: Some(
                "Check regex syntax: brackets, parentheses, and special characters must be balanced".to_string(),
              ),
              line_number,
            });
                    }

                    // Validate template expressions in replacement
                    if (regex_config.replacement.contains("{{")
                        || regex_config.replacement.contains("{%"))
                        && let Err(e) = template::validate_template(&regex_config.replacement)
                    {
                        let line_number =
                            Self::find_line_number(file_content, &regex_config.replacement);
                        errors.push(ValidationError {
                mock_id: mock_id.clone(),
                error_type: ErrorType::TemplateError,
                message: format!("Invalid template in patch regex replacement: {e}"),
                snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, "{{")),
                suggestion: Some(
                  "Check template syntax: variables use {{ var }}, functions use {{ func() }}".to_string(),
                ),
                line_number,
              });
                    }
                }

                // Validate header add names and values
                for (name, value) in &patches_config.headers.add {
                    // Validate header name
                    if HeaderName::from_str(name).is_err() {
                        let line_number = Self::find_line_number(file_content, name);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidPatchHeaderName,
              message: format!("Invalid header name in patch: '{name}'"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, name)),
              suggestion: Some("Header names must be valid HTTP header names (alphanumeric and hyphens)".to_string()),
              line_number,
            });
                    }

                    // Validate template expressions in value
                    if (value.contains("{{") || value.contains("{%"))
                        && let Err(e) = template::validate_template(value)
                    {
                        let line_number = Self::find_line_number(file_content, value);
                        errors.push(ValidationError {
                mock_id: mock_id.clone(),
                error_type: ErrorType::TemplateError,
                message: format!("Invalid template in patch header '{name}': {e}"),
                snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, "{{")),
                suggestion: Some(
                  "Check template syntax: variables use {{ var }}, functions use {{ func() }}".to_string(),
                ),
                line_number,
              });
                    }
                }

                // Validate header remove names
                for name in &patches_config.headers.remove {
                    if HeaderName::from_str(name).is_err() {
                        let line_number = Self::find_line_number(file_content, name);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidPatchHeaderName,
              message: format!("Invalid header name in patch remove: '{name}'"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, name)),
              suggestion: Some("Header names must be valid HTTP header names (alphanumeric and hyphens)".to_string()),
              line_number,
            });
                    }
                }
            }

            // Validate request transform configuration
            if let Some(ref request_config) = mock.request {
                if request_config.is_empty() {
                    let line_number = Self::find_line_number(file_content, "[request");
                    warnings.push(ValidationWarning {
                        mock_id: mock_id.clone(),
                        message: "Empty request transform section (no fields set)".to_string(),
                        warning_type: WarningType::EmptyRequestTransform,
                        line_number,
                        snippet: None,
                        suggestion: Some(
                            "Add request transform fields or remove the [request] section"
                                .to_string(),
                        ),
                    });
                }

                // Validate delay duration
                if let Some(ref delay) = request_config.delay
                    && let Err(_e) = crate::config::response::parse_duration(delay)
                {
                    let line_number = Self::find_line_number(file_content, delay);
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::InvalidDuration,
                        message: format!("Invalid request delay duration: '{delay}'"),
                        snippet: line_number
                            .and_then(|line| Self::extract_snippet(file_content, line, delay)),
                        suggestion: Some(
                            "Duration must have a unit suffix: '100ms', '2s', '500us'".to_string(),
                        ),
                        line_number,
                    });
                }

                // Validate timeout duration
                if let Some(ref timeout) = request_config.timeout
                    && let Err(_e) = crate::config::response::parse_duration(timeout)
                {
                    let line_number = Self::find_line_number(file_content, timeout);
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::InvalidDuration,
                        message: format!("Invalid request timeout duration: '{timeout}'"),
                        snippet: line_number
                            .and_then(|line| Self::extract_snippet(file_content, line, timeout)),
                        suggestion: Some(
                            "Duration must have a unit suffix: '100ms', '2s', '500us'".to_string(),
                        ),
                        line_number,
                    });
                }

                // Validate forward_to URL
                if let Some(ref forward_to) = request_config.forward_to
                    && !forward_to.starts_with("http://")
                    && !forward_to.starts_with("https://")
                {
                    let line_number = Self::find_line_number(file_content, forward_to);
                    errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidUrl,
              message: format!(
                "Invalid forward_to URL: '{forward_to}' (must start with http:// or https://)"
              ),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, forward_to)),
              suggestion: Some("forward_to must be a full URL, e.g., 'https://staging.example.com'".to_string()),
              line_number,
            });
                }

                // Validate rewrite_path template syntax
                if let Some(ref rewrite_path) = request_config.rewrite_path
                    && (rewrite_path.contains("{{") || rewrite_path.contains("{%"))
                    && let Err(e) = template::validate_template(rewrite_path)
                {
                    let line_number = Self::find_line_number(file_content, rewrite_path);
                    errors.push(ValidationError {
                        mock_id: mock_id.clone(),
                        error_type: ErrorType::InvalidRewritePathTemplate,
                        message: format!("Invalid Tera template in rewrite_path: {e}"),
                        snippet: line_number.and_then(|line| {
                            Self::extract_snippet(file_content, line, rewrite_path)
                        }),
                        suggestion: Some(
                            "Check template syntax: use {{ captures.param }} for URL captures"
                                .to_string(),
                        ),
                        line_number,
                    });
                }

                // Validate request header names in headers.add
                for header_name in request_config.headers.add.keys() {
                    if HeaderName::from_str(header_name).is_err() {
                        let line_number = Self::find_line_number(file_content, header_name);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidRequestHeaderName,
              message: format!("Invalid request header name: '{header_name}'"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, header_name)),
              suggestion: Some("Header names must be valid HTTP header names (alphanumeric and hyphens)".to_string()),
              line_number,
            });
                    }
                }

                // Validate request body regex patterns
                for regex_config in &request_config.body.regex {
                    if let Err(e) = Regex::new(&regex_config.pattern) {
                        let line_number =
                            Self::find_line_number(file_content, &regex_config.pattern);
                        errors.push(ValidationError {
              mock_id: mock_id.clone(),
              error_type: ErrorType::InvalidRequestBodyRegex,
              message: format!("Invalid regex pattern in request body patch: {e}"),
              snippet: line_number.and_then(|line| Self::extract_snippet(file_content, line, &regex_config.pattern)),
              suggestion: Some(
                "Check regex syntax: brackets, parentheses, and special characters must be balanced".to_string(),
              ),
              line_number,
            });
                    }
                }

                // Check for conflicting modes: full mock body + request transforms
                if let Some(ref response_config) = mock.response_config
                    && response_config.is_full_mock()
                    && !request_config.is_empty()
                {
                    let line_number = Self::find_line_number(file_content, "forward_to")
                        .or_else(|| Self::find_line_number(file_content, "rewrite_path"))
                        .or_else(|| Self::find_line_number(file_content, "[request"));
                    errors.push(ValidationError {
              mock_id,
              error_type: ErrorType::ConflictingModes,
              message: "Cannot combine request transforms with full mock response (response.body/response.json)".to_string(),
              snippet: None,
              suggestion: Some("Request transforms require passthrough mode. Remove response.body/response.json, or use top-level `patch` instead.".to_string()),
              line_number,
            });
                }
            }
        }

        // Try to convert to mock definitions to catch any remaining errors
        match config
            .clone()
            .into_mock_definitions_with_dir(config_dir, None)
            .await
        {
            Ok(mock_defs) => {
                // Check for overlapping patterns (warnings only)
                for i in 0..mock_defs.len() {
                    for j in (i + 1)..mock_defs.len() {
                        if Self::mocks_may_overlap(&mock_defs[i], &mock_defs[j]) {
                            warnings.push(ValidationWarning {
                mock_id: Some(format!("{} and {}", mock_defs[i].id, mock_defs[j].id).into()),
                message: format!(
                  "Mocks '{}' and '{}' may have overlapping patterns (priority: {} vs {})",
                  mock_defs[i].id, mock_defs[j].id, mock_defs[i].priority, mock_defs[j].priority
                ),
                warning_type: WarningType::OverlappingPatterns,
                line_number: Self::find_line_number(file_content, &mock_defs[i].id),
                snippet: None,
                suggestion: Some("Assign different priorities to control which mock matches first".to_string()),
              });
                        }
                    }
                }
            }
            Err(e) => {
                // Conversion error - this catches issues not caught by individual field validation
                errors.push(ValidationError {
                    mock_id: None,
                    error_type: ErrorType::ConversionError,
                    message: format!("Failed to convert config to mock definitions: {e}"),
                    snippet: None,
                    suggestion: None,
                    line_number: None,
                });
            }
        }

        ValidationResult {
            file_path,
            errors,
            warnings,
            handler_count: None,
        }
    }

    /// Extract parse error information from error message
    fn extract_parse_error_info(
        error_msg: &str,
        content: &str,
    ) -> (Option<usize>, Option<CodeSnippet>) {
        // Try to extract line number from common error formats
        // JSON: "... at line 10 column 5"
        // YAML: "... at line 3"

        let line_number = if let Some(captures) = Regex::new(r"line (\d+)")
            .ok()
            .and_then(|re| re.captures(error_msg))
        {
            captures
                .get(1)
                .and_then(|m| m.as_str().parse::<usize>().ok())
        } else {
            None
        };

        let snippet = line_number.and_then(|line| Self::extract_snippet_by_line(content, line));

        (line_number, snippet)
    }

    /// Find the line number where a string appears in the content
    fn find_line_number(content: &str, search: &str) -> Option<usize> {
        content
            .lines()
            .enumerate()
            .find(|(_, line)| line.contains(search))
            .map(|(idx, _)| idx + 1)
    }

    /// Extract a code snippet around a line
    #[allow(clippy::indexing_slicing)] // Line number bounds checked: `line_number > lines.len()` guard above
    fn extract_snippet(content: &str, line_number: usize, highlight: &str) -> Option<CodeSnippet> {
        let lines: Vec<&str> = content.lines().collect();
        if line_number == 0 || line_number > lines.len() {
            return None;
        }

        let line = lines[line_number - 1];
        let highlight_start = line.find(highlight)?;
        let highlight_end = highlight_start + highlight.len();

        Some(CodeSnippet {
            line_number,
            code: line.to_string(),
            highlight_start,
            highlight_end,
        })
    }

    /// Extract a code snippet by line number only
    #[allow(clippy::indexing_slicing)] // Line number bounds checked: `line_number > lines.len()` guard above
    fn extract_snippet_by_line(content: &str, line_number: usize) -> Option<CodeSnippet> {
        let lines: Vec<&str> = content.lines().collect();
        if line_number == 0 || line_number > lines.len() {
            return None;
        }

        let line = lines[line_number - 1];

        Some(CodeSnippet {
            line_number,
            code: line.to_string(),
            highlight_start: 0,
            highlight_end: line.len(),
        })
    }

    /// Check if two mocks may have overlapping patterns
    fn mocks_may_overlap(mock1: &MockDefinition, mock2: &MockDefinition) -> bool {
        // If they have different scopes, they can't overlap
        if mock1.scope != mock2.scope {
            return false;
        }

        // A `once` mock is written to be shadowed: it answers, retires, and
        // hands the request to whatever stands behind it. Reporting that as an
        // accidental overlap would flag every sequence — including the ones
        // `mock convert` builds from a recording — as something to go and fix.
        if mock1.once || mock2.once {
            return false;
        }

        // If methods don't overlap, they can't conflict
        // Note: empty methods means "match all methods"
        if !mock1.request.methods.is_empty() && !mock2.request.methods.is_empty() {
            let methods_overlap = mock1
                .request
                .methods
                .iter()
                .any(|m| mock2.request.methods.contains(m));
            if !methods_overlap {
                return false;
            }
        }

        // Check if URL patterns might overlap
        let urls_overlap = if !mock1.request.url_patterns.is_empty()
            && !mock2.request.url_patterns.is_empty()
        {
            Self::url_patterns_may_overlap(&mock1.request.url_patterns, &mock2.request.url_patterns)
        } else {
            // If either has no URL patterns, assume possible overlap
            true
        };

        if !urls_overlap {
            return false;
        }

        // Even if URLs overlap, GraphQL mocks with different operation matchers don't conflict
        if let (Some(gql1), Some(gql2)) = (
            &mock1.request.graphql_matcher,
            &mock2.request.graphql_matcher,
        ) && Self::graphql_matchers_discriminate(gql1, gql2)
        {
            return false;
        }

        // Mocks with different body matchers further discriminate
        if let (Some(body1), Some(body2)) =
            (&mock1.request.body_matcher, &mock2.request.body_matcher)
            && Self::body_matchers_discriminate(body1, body2)
        {
            return false;
        }

        // Mocks with different required header matchers discriminate
        if Self::header_matchers_discriminate(
            &mock1.request.header_matchers,
            &mock2.request.header_matchers,
        ) {
            return false;
        }

        // Mocks with different required query param matchers discriminate
        if Self::query_matchers_discriminate(
            &mock1.request.query_matchers,
            &mock2.request.query_matchers,
        ) {
            return false;
        }

        true
    }

    /// Check if two sets of URL patterns may overlap
    /// This is a conservative heuristic - when in doubt, we assume overlap
    fn url_patterns_may_overlap(
        patterns1: &[crate::types::UrlPattern],
        patterns2: &[crate::types::UrlPattern],
    ) -> bool {
        use crate::engine::types::UrlPattern;

        // Check each pair of patterns
        for pattern1 in patterns1 {
            for pattern2 in patterns2 {
                // Try to determine if these two patterns could match the same URL
                match (pattern1, pattern2) {
                    // Exact vs Exact: only overlap if identical
                    (UrlPattern::Exact(s1), UrlPattern::Exact(s2)) => {
                        if s1 == s2 {
                            return true;
                        }
                    }

                    // Exact vs Prefix: overlap if exact starts with prefix
                    (UrlPattern::Exact(s), UrlPattern::Prefix(prefix))
                    | (UrlPattern::Prefix(prefix), UrlPattern::Exact(s)) => {
                        if s.starts_with(prefix) {
                            return true;
                        }
                    }

                    // Exact vs Suffix: overlap if exact ends with suffix
                    (UrlPattern::Exact(s), UrlPattern::Suffix(suffix))
                    | (UrlPattern::Suffix(suffix), UrlPattern::Exact(s)) => {
                        if s.ends_with(suffix) {
                            return true;
                        }
                    }

                    // Exact vs Regex: test if regex matches the exact string
                    (UrlPattern::Exact(s), UrlPattern::Regex(re))
                    | (UrlPattern::Regex(re), UrlPattern::Exact(s)) => {
                        if re.is_match(s) {
                            return true;
                        }
                    }

                    // Exact vs Glob: test if glob matches the exact string
                    (UrlPattern::Exact(s), UrlPattern::Glob(g))
                    | (UrlPattern::Glob(g), UrlPattern::Exact(s)) => {
                        if g.is_match(s) {
                            return true;
                        }
                    }

                    // Prefix vs Prefix: overlap if one is a prefix of the other
                    (UrlPattern::Prefix(p1), UrlPattern::Prefix(p2)) => {
                        if p1.starts_with(p2) || p2.starts_with(p1) {
                            return true;
                        }
                    }

                    // Suffix vs Suffix: overlap if one is a suffix of the other
                    (UrlPattern::Suffix(s1), UrlPattern::Suffix(s2)) => {
                        if s1.ends_with(s2) || s2.ends_with(s1) {
                            return true;
                        }
                    }

                    // Regex vs Regex, Glob vs Glob, href regexes, and other
                    // complex combinations: conservative approach - assume
                    // they might overlap (proper static analysis is too
                    // complex for regex/glob patterns)
                    (
                        UrlPattern::Regex(_)
                        | UrlPattern::HrefRegex(_)
                        | UrlPattern::Prefix(_)
                        | UrlPattern::Suffix(_)
                        | UrlPattern::Glob(_),
                        UrlPattern::Regex(_) | UrlPattern::HrefRegex(_) | UrlPattern::Glob(_),
                    )
                    | (
                        UrlPattern::Regex(_)
                        | UrlPattern::HrefRegex(_)
                        | UrlPattern::Glob(_)
                        | UrlPattern::Suffix(_),
                        UrlPattern::Prefix(_),
                    )
                    | (
                        UrlPattern::Regex(_)
                        | UrlPattern::HrefRegex(_)
                        | UrlPattern::Glob(_)
                        | UrlPattern::Prefix(_),
                        UrlPattern::Suffix(_),
                    )
                    | (UrlPattern::Exact(_), UrlPattern::HrefRegex(_))
                    | (UrlPattern::HrefRegex(_), UrlPattern::Exact(_)) => {
                        // Conservative: assume overlap for complex pattern combinations
                        return true;
                    }
                }
            }
        }

        // No overlaps detected
        false
    }

    /// Check if two GraphQL matchers are guaranteed to match different requests
    fn graphql_matchers_discriminate(
        gql1: &crate::types::GraphQLMatcher,
        gql2: &crate::types::GraphQLMatcher,
    ) -> bool {
        // If either matches any operation (graphql = "*"), they could overlap with anything
        if gql1.match_any || gql2.match_any {
            return false;
        }

        // Different operation types (query vs mutation vs subscription) never overlap
        if let (Some(type1), Some(type2)) = (&gql1.operation_type, &gql2.operation_type)
            && type1 != type2
        {
            return true;
        }

        // Introspection queries are always queries (never mutations/subscriptions).
        // If one is an introspection matcher and the other targets mutations or subscriptions,
        // they cannot overlap.
        if gql1.introspection_matcher.is_some()
            && let Some(t) = &gql2.operation_type
            && matches!(
                t,
                crate::types::GraphQLOperationType::Mutation
                    | crate::types::GraphQLOperationType::Subscription
            )
        {
            return true;
        }
        if gql2.introspection_matcher.is_some()
            && let Some(t) = &gql1.operation_type
            && matches!(
                t,
                crate::types::GraphQLOperationType::Mutation
                    | crate::types::GraphQLOperationType::Subscription
            )
        {
            return true;
        }

        // Different operation names never overlap
        if let (Some(name1), Some(name2)) = (&gql1.operation_name, &gql2.operation_name)
            && name1 != name2
        {
            return true;
        }

        // One is introspection-only and the other targets a named operation -- distinct
        match (&gql1.introspection_matcher, &gql2.introspection_matcher) {
            (Some(_), None) if gql2.operation_name.is_some() => return true,
            (None, Some(_)) if gql1.operation_name.is_some() => return true,
            _ => {}
        }

        false
    }

    /// Check if two body matchers are guaranteed to match different requests
    fn body_matchers_discriminate(
        body1: &crate::types::BodyMatcher,
        body2: &crate::types::BodyMatcher,
    ) -> bool {
        use crate::engine::types::BodyMatcher;
        // JSONPath matchers on different paths discriminate
        if let (BodyMatcher::JsonPath { path: p1, .. }, BodyMatcher::JsonPath { path: p2, .. }) =
            (body1, body2)
            && p1 != p2
        {
            return true;
        }
        // Same path but different expected values
        // We can't easily compare values here without runtime context,
        // but different paths are a clear discriminator
        // Different matcher types (contains vs jsonpath vs regex) are different enough
        // to likely not overlap, but we can't guarantee it statically
        false
    }

    /// Check if header matchers guarantee different requests
    #[allow(clippy::match_same_arms)] // Keeping explicit arms for pattern matching clarity
    fn header_matchers_discriminate(
        headers1: &[crate::types::HeaderMatcher],
        headers2: &[crate::types::HeaderMatcher],
    ) -> bool {
        // If one mock requires a header that the other doesn't mention, they could still both match
        // the same request. But if they require the SAME header with DIFFERENT exact values, they
        // discriminate.
        for h1 in headers1 {
            for h2 in headers2 {
                if h1.name == h2.name {
                    match (&h1.pattern, &h2.pattern) {
                        (
                            crate::types::HeaderMatchPattern::Exact(v1),
                            crate::types::HeaderMatchPattern::Exact(v2),
                        ) if v1 != v2 => {
                            return true;
                        }
                        (
                            crate::types::HeaderMatchPattern::Exact(_),
                            crate::types::HeaderMatchPattern::Exact(_),
                        ) => {}
                        (
                            crate::types::HeaderMatchPattern::Present,
                            crate::types::HeaderMatchPattern::Absent,
                        )
                        | (
                            crate::types::HeaderMatchPattern::Absent,
                            crate::types::HeaderMatchPattern::Present,
                        ) => {
                            return true;
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }

    /// Check if query parameter matchers guarantee different requests
    #[allow(clippy::match_same_arms)] // Keeping explicit arms for pattern matching clarity
    fn query_matchers_discriminate(
        queries1: &[crate::types::QueryMatcher],
        queries2: &[crate::types::QueryMatcher],
    ) -> bool {
        for q1 in queries1 {
            for q2 in queries2 {
                if q1.name == q2.name {
                    match (&q1.pattern, &q2.pattern) {
                        (
                            crate::types::QueryMatchPattern::Exact(v1),
                            crate::types::QueryMatchPattern::Exact(v2),
                        ) if v1 != v2 => {
                            return true;
                        }
                        (
                            crate::types::QueryMatchPattern::Exact(_),
                            crate::types::QueryMatchPattern::Exact(_),
                        ) => {}
                        (
                            crate::types::QueryMatchPattern::Present,
                            crate::types::QueryMatchPattern::Absent,
                        )
                        | (
                            crate::types::QueryMatchPattern::Absent,
                            crate::types::QueryMatchPattern::Present,
                        ) => {
                            return true;
                        }
                        _ => {}
                    }
                }
            }
        }
        false
    }
}

impl Default for MockValidator {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of validation
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationResult {
    /// Path to the file that was validated
    pub file_path: Option<PathBuf>,
    /// List of errors found
    pub errors: Vec<ValidationError>,
    /// List of warnings found
    pub warnings: Vec<ValidationWarning>,
    /// Handlers registered by a script mock file (`.js`/`.ts`), which is
    /// validated by evaluating it; declarative files report None.
    pub handler_count: Option<usize>,
}

impl ValidationResult {
    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Check if there are any warnings
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Get the number of errors
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    /// Get the number of warnings
    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// Format errors in Rust compiler style
    pub fn format_errors(&self) -> String {
        let mut output = String::new();

        for error in &self.errors {
            output.push_str(&error.format(self.file_path.as_ref()));
            output.push('\n');
        }

        output
    }

    /// Format warnings in Rust compiler style
    pub fn format_warnings(&self) -> String {
        let mut output = String::new();

        for warning in &self.warnings {
            output.push_str(&warning.format(self.file_path.as_ref()));
            output.push('\n');
        }

        output
    }

    /// Format everything (errors and warnings) together
    pub fn format_all(&self) -> String {
        let mut output = String::new();

        if !self.errors.is_empty() {
            output.push_str(&self.format_errors());
        }

        if !self.warnings.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&self.format_warnings());
        }

        output
    }
}

/// A validation error with context
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationError {
    /// ID of the mock that has the error (if applicable)
    pub mock_id: Option<LeanString>,
    /// Type of error
    pub error_type: ErrorType,
    /// Error message
    pub message: String,
    /// Code snippet showing the error
    pub snippet: Option<CodeSnippet>,
    /// Suggestion for fixing the error
    pub suggestion: Option<String>,
    /// Line number where the error occurred
    pub line_number: Option<usize>,
}

impl ValidationError {
    /// Format the error in Rust compiler style
    #[allow(clippy::format_push_string)] // Error formatting intentionally uses format! for readability
    pub fn format(&self, file_path: Option<&PathBuf>) -> String {
        let mut output = String::new();

        // Error header: error[E001]: message
        let error_code = self.error_type.code();
        output.push_str(&format!("error[{}]: {}\n", error_code, self.message));

        // File location: --> path/to/file.yaml:line:col
        if let Some(path) = file_path {
            let location = if let Some(line) = self.line_number {
                format!("{}:{}", path.display(), line)
            } else {
                format!("{}", path.display())
            };
            output.push_str(&format!("  --> {location}\n"));
        }

        // Code snippet with highlighting
        if let Some(snippet) = &self.snippet {
            output.push_str(&snippet.format());
        }

        // Mock ID context
        if let Some(ref id) = self.mock_id {
            output.push_str(&format!("   = note: in mock '{id}'\n"));
        }

        // Suggestion
        if let Some(ref suggestion) = self.suggestion {
            output.push_str(&format!("   = help: {suggestion}\n"));
        }

        output
    }
}

/// A validation warning
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationWarning {
    /// ID of the mock that has the warning (if applicable)
    pub mock_id: Option<LeanString>,
    /// Warning message
    pub message: String,
    /// Type of warning
    pub warning_type: WarningType,
    /// Line number where the warning occurred
    pub line_number: Option<usize>,
    /// Code snippet showing the warning
    pub snippet: Option<CodeSnippet>,
    /// Suggestion for fixing the warning
    pub suggestion: Option<String>,
}

impl ValidationWarning {
    /// Format the warning in Rust compiler style
    #[allow(clippy::format_push_string)] // Warning formatting intentionally uses format! for readability
    pub fn format(&self, file_path: Option<&PathBuf>) -> String {
        let mut output = String::new();

        // Warning header: warning[W001]: message
        let warning_code = self.warning_type.code();
        output.push_str(&format!("warning[{}]: {}\n", warning_code, self.message));

        // File location
        if let Some(path) = file_path {
            let location = if let Some(line) = self.line_number {
                format!("{}:{}", path.display(), line)
            } else {
                format!("{}", path.display())
            };
            output.push_str(&format!("  --> {location}\n"));
        }

        // Code snippet with highlighting
        if let Some(snippet) = &self.snippet {
            output.push_str(&snippet.format());
        }

        // Mock ID context
        if let Some(ref id) = self.mock_id {
            output.push_str(&format!("   = note: {id}\n"));
        }

        // Suggestion
        if let Some(ref suggestion) = self.suggestion {
            output.push_str(&format!("   = help: {suggestion}\n"));
        }

        output
    }
}

/// Code snippet with highlighting
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodeSnippet {
    /// Line number (1-indexed)
    pub line_number: usize,
    /// The code line
    pub code: String,
    /// Start position of highlight (0-indexed)
    pub highlight_start: usize,
    /// End position of highlight (0-indexed)
    pub highlight_end: usize,
}

impl CodeSnippet {
    /// Format the snippet with line numbers and highlighting
    #[allow(clippy::format_push_string, clippy::indexing_slicing)] // Diagnostic formatting code
    pub fn format(&self) -> String {
        let mut output = String::new();

        // Line number and separator
        output.push_str("   |\n");
        output.push_str(&format!("{:3} | {}\n", self.line_number, self.code));

        // Highlight indicator (^^^)
        let padding = " ".repeat(self.highlight_start);
        let highlight_len = (self.highlight_end - self.highlight_start).max(1);
        let highlight = "^".repeat(highlight_len);
        output.push_str(&format!("   | {padding}{highlight}\n"));

        output
    }
}

/// Type of validation error
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ErrorType {
    /// Parse error (JSON/YAML syntax)
    ParseError,
    /// Invalid HTTP method
    InvalidMethod,
    /// Invalid regex pattern
    InvalidRegex,
    /// Invalid HTTP status code
    InvalidStatusCode,
    /// Invalid header name
    InvalidHeaderName,
    /// Template syntax error
    TemplateError,
    /// Referenced file not found
    FileNotFound,
    /// Missing required field
    MissingField,
    /// Error during conversion to MockDefinition
    ConversionError,
    /// File read error
    FileReadError,
    /// Unsupported file format
    UnsupportedFormat,
    /// Invalid duration format in request transforms
    InvalidDuration,
    /// Invalid URL in request forward_to
    InvalidUrl,
    /// Invalid Tera template in rewrite_path
    InvalidRewritePathTemplate,
    /// Invalid HTTP header name in request headers.add
    InvalidRequestHeaderName,
    /// Invalid regex in request body patches
    InvalidRequestBodyRegex,
    /// Conflicting full mock body with request transforms
    ConflictingModes,
    /// Multiple mutually exclusive response body fields set
    MutuallyExclusiveFields,
    /// Invalid regex pattern in response patch
    InvalidPatchRegex,
    /// Invalid HTTP header name in response patch
    InvalidPatchHeaderName,
    /// Invalid ws/sse streaming configuration
    InvalidStreamingConfig,
    /// Script mock file failed to bundle or evaluate
    ScriptError,
}

impl ErrorType {
    /// Get the error code (e.g., "E001")
    pub fn code(&self) -> String {
        match self {
            ErrorType::ParseError => "E001".to_string(),
            ErrorType::InvalidMethod => "E002".to_string(),
            ErrorType::InvalidRegex => "E003".to_string(),
            ErrorType::InvalidStatusCode => "E004".to_string(),
            ErrorType::InvalidHeaderName => "E005".to_string(),
            ErrorType::TemplateError => "E006".to_string(),
            ErrorType::FileNotFound => "E007".to_string(),
            ErrorType::MissingField => "E008".to_string(),
            ErrorType::ConversionError => "E009".to_string(),
            ErrorType::FileReadError => "E010".to_string(),
            ErrorType::UnsupportedFormat => "E011".to_string(),
            ErrorType::InvalidDuration => "E012".to_string(),
            ErrorType::InvalidUrl => "E013".to_string(),
            ErrorType::InvalidRewritePathTemplate => "E014".to_string(),
            ErrorType::InvalidRequestHeaderName => "E015".to_string(),
            ErrorType::InvalidRequestBodyRegex => "E016".to_string(),
            ErrorType::ConflictingModes => "E017".to_string(),
            ErrorType::MutuallyExclusiveFields => "E018".to_string(),
            ErrorType::InvalidPatchRegex => "E019".to_string(),
            ErrorType::InvalidPatchHeaderName => "E020".to_string(),
            ErrorType::InvalidStreamingConfig => "E021".to_string(),
            ErrorType::ScriptError => "E022".to_string(),
        }
    }
}

/// Type of validation warning
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum WarningType {
    /// Duplicate mock ID
    DuplicateId,
    /// Mock is disabled
    DisabledMock,
    /// Overlapping URL patterns
    OverlappingPatterns,
    /// Empty request transform section
    EmptyRequestTransform,
    /// Script file registered no handlers
    EmptyScript,
}

impl WarningType {
    /// Get the warning code (e.g., "W001")
    pub fn code(&self) -> String {
        match self {
            WarningType::DuplicateId => "W001".to_string(),
            WarningType::DisabledMock => "W002".to_string(),
            WarningType::OverlappingPatterns => "W003".to_string(),
            WarningType::EmptyRequestTransform => "W004".to_string(),
            WarningType::EmptyScript => "W005".to_string(),
        }
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
    use smallvec::smallvec;

    #[tokio::test]
    async fn test_validator_creation() {
        let validator = MockValidator::new();
        assert!(validator.config.check_files);
    }

    #[tokio::test]
    async fn test_validate_missing_file() {
        let validator = MockValidator::new();
        let result = validator
            .validate_file(Path::new("/nonexistent.yaml"))
            .await;

        assert!(result.has_errors());
        assert_eq!(result.error_count(), 1);
        assert!(matches!(
            result.errors[0].error_type,
            ErrorType::FileReadError
        ));
    }

    #[test]
    fn test_error_type_codes() {
        assert_eq!(ErrorType::ParseError.code(), "E001");
        assert_eq!(ErrorType::InvalidMethod.code(), "E002");
        assert_eq!(ErrorType::InvalidRegex.code(), "E003");
        assert_eq!(ErrorType::TemplateError.code(), "E006");
    }

    #[test]
    fn test_warning_type_codes() {
        assert_eq!(WarningType::DuplicateId.code(), "W001");
        assert_eq!(WarningType::DisabledMock.code(), "W002");
        assert_eq!(WarningType::OverlappingPatterns.code(), "W003");
    }

    #[test]
    fn test_code_snippet_format() {
        let snippet = CodeSnippet {
            line_number: 10,
            code: r#"method = "INVALID""#.to_string(),
            highlight_start: 10,
            highlight_end: 19,
        };

        let formatted = snippet.format();
        assert!(formatted.contains("10 |"));
        assert!(formatted.contains("INVALID"));
        assert!(formatted.contains("^^^^^^^^^"));
    }

    #[test]
    fn test_validation_result_helpers() {
        let result = ValidationResult {
            file_path: None,
            errors: vec![ValidationError {
                mock_id: None,
                error_type: ErrorType::ParseError,
                message: "test error".to_string(),
                snippet: None,
                suggestion: None,
                line_number: None,
            }],
            warnings: vec![ValidationWarning {
                mock_id: None,
                message: "test warning".to_string(),
                warning_type: WarningType::DuplicateId,
                line_number: None,
                snippet: None,
                suggestion: None,
            }],
            handler_count: None,
        };

        assert!(result.has_errors());
        assert!(result.has_warnings());
        assert_eq!(result.error_count(), 1);
        assert_eq!(result.warning_count(), 1);
    }

    #[test]
    fn test_url_patterns_different_methods_and_paths() {
        use crate::engine::types::{
            BodySource, RequestMatcher, ResponseGenerator, ResponseMode, UrlPattern,
        };
        use http::{Method, StatusCode};
        use rustc_hash::FxHashMap;
        use std::sync::Arc;

        // POST /api/users (exact match)
        let mock1 = MockDefinition {
            id: "create-user".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
            request: RequestMatcher {
                methods: smallvec![Method::POST],
                url_patterns: smallvec![UrlPattern::exact("/api/users")],
                header_matchers: smallvec![],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator {
                status: StatusCode::OK,
                headers: FxHashMap::default(),
                body: BodySource::Inline(Arc::new(bytes::Bytes::from(""))),
                delay: None,
                mode: ResponseMode::Static,
                structured_response: false,
                context_uses_headers: true,
                context_uses_body: true,
            },
        };

        // GET /api/users/:id (regex with parameter)
        let mock2 = MockDefinition {
            id: "get-user".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![
                    UrlPattern::regex("^/api/users/(?P<id>[^/]+)$").expect("valid regex")
                ],
                header_matchers: smallvec![],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator {
                status: StatusCode::OK,
                headers: FxHashMap::default(),
                body: BodySource::Inline(Arc::new(bytes::Bytes::from(""))),
                delay: None,
                mode: ResponseMode::Static,
                structured_response: false,
                context_uses_headers: true,
                context_uses_body: true,
            },
        };

        // These should NOT overlap - different methods and different URL patterns
        assert!(
            !MockValidator::mocks_may_overlap(&mock1, &mock2),
            "POST /api/users and GET /api/users/:id should NOT overlap (different methods)"
        );
    }

    #[test]
    fn test_url_patterns_exact_vs_regex_no_match() {
        use crate::engine::types::UrlPattern;

        // Test exact pattern vs regex that doesn't match
        let patterns1 = vec![UrlPattern::exact("/api/users")];
        let patterns2 = vec![UrlPattern::regex("^/api/users/(?P<id>[^/]+)$").expect("valid regex")];

        // /api/users doesn't match the regex ^/api/users/(?P<id>[^/]+)$
        // because the regex requires an additional path segment
        assert!(
            !MockValidator::url_patterns_may_overlap(&patterns1, &patterns2),
            "/api/users should not overlap with /api/users/:id pattern"
        );
    }

    #[test]
    fn test_url_patterns_exact_vs_regex_match() {
        use crate::engine::types::UrlPattern;

        // Test exact pattern vs regex that DOES match
        let patterns1 = vec![UrlPattern::exact("/api/users/123")];
        let patterns2 = vec![UrlPattern::regex("^/api/users/(?P<id>[^/]+)$").expect("valid regex")];

        // /api/users/123 DOES match the regex ^/api/users/(?P<id>[^/]+)$
        assert!(
            MockValidator::url_patterns_may_overlap(&patterns1, &patterns2),
            "/api/users/123 should overlap with /api/users/:id pattern"
        );
    }

    #[test]
    fn test_url_patterns_exact_match() {
        use crate::engine::types::UrlPattern;

        // Same exact patterns should overlap
        let patterns1 = vec![UrlPattern::exact("/api/users")];
        let patterns2 = vec![UrlPattern::exact("/api/users")];

        assert!(
            MockValidator::url_patterns_may_overlap(&patterns1, &patterns2),
            "Identical exact patterns should overlap"
        );
    }

    #[test]
    fn test_url_patterns_different_exact() {
        use crate::engine::types::UrlPattern;

        // Different exact patterns should NOT overlap
        let patterns1 = vec![UrlPattern::exact("/api/users")];
        let patterns2 = vec![UrlPattern::exact("/api/posts")];

        assert!(
            !MockValidator::url_patterns_may_overlap(&patterns1, &patterns2),
            "Different exact patterns should not overlap"
        );
    }

    #[test]
    fn test_mocks_with_different_query_params() {
        use crate::engine::types::{
            BodySource, QueryMatcher, RequestMatcher, ResponseGenerator, ResponseMode, UrlPattern,
        };
        use http::{Method, StatusCode};
        use rustc_hash::FxHashMap;
        use std::sync::Arc;

        // Mock 1: GET /api/users?role=admin
        let mock1 = MockDefinition {
            id: "admin-users".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact("/api/users")],
                header_matchers: smallvec![],
                query_matchers: smallvec![QueryMatcher::exact("role", "admin")],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator {
                status: StatusCode::OK,
                headers: FxHashMap::default(),
                body: BodySource::Inline(Arc::new(bytes::Bytes::from(""))),
                delay: None,
                mode: ResponseMode::Static,
                structured_response: false,
                context_uses_headers: true,
                context_uses_body: true,
            },
        };

        // Mock 2: GET /api/users?role=user
        let mock2 = MockDefinition {
            id: "regular-users".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact("/api/users")],
                header_matchers: smallvec![],
                query_matchers: smallvec![QueryMatcher::exact("role", "user")],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator {
                status: StatusCode::OK,
                headers: FxHashMap::default(),
                body: BodySource::Inline(Arc::new(bytes::Bytes::from(""))),
                delay: None,
                mode: ResponseMode::Static,
                structured_response: false,
                context_uses_headers: true,
                context_uses_body: true,
            },
        };

        // These should NOT overlap - same URL and method, but different query params discriminate
        assert!(
            !MockValidator::mocks_may_overlap(&mock1, &mock2),
            "Mocks with different query parameter values should NOT overlap"
        );
    }

    #[test]
    fn test_mocks_with_different_headers() {
        use crate::engine::types::{
            BodySource, HeaderMatcher, RequestMatcher, ResponseGenerator, ResponseMode, UrlPattern,
        };
        use http::header::HeaderName;
        use http::{Method, StatusCode};
        use rustc_hash::FxHashMap;
        use std::sync::Arc;

        // Mock 1: GET /api/data with X-API-Version: 1
        let mock1 = MockDefinition {
            id: "api-v1".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact("/api/data")],
                header_matchers: smallvec![HeaderMatcher::exact(
                    HeaderName::from_static("x-api-version"),
                    "1".to_string(),
                )],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator {
                status: StatusCode::OK,
                headers: FxHashMap::default(),
                body: BodySource::Inline(Arc::new(bytes::Bytes::from(""))),
                delay: None,
                mode: ResponseMode::Static,
                structured_response: false,
                context_uses_headers: true,
                context_uses_body: true,
            },
        };

        // Mock 2: GET /api/data with X-API-Version: 2
        let mock2 = MockDefinition {
            id: "api-v2".into(),
            priority: 100,
            enabled: true,
            once: false,
            scope: None,
            source_file: None,
            request_transforms: None,
            vars: None,
            streaming: None,
            request: RequestMatcher {
                methods: smallvec![Method::GET],
                url_patterns: smallvec![UrlPattern::exact("/api/data")],
                header_matchers: smallvec![HeaderMatcher::exact(
                    HeaderName::from_static("x-api-version"),
                    "2".to_string(),
                )],
                query_matchers: smallvec![],
                body_matcher: None,
                graphql_matcher: None,
            },
            response: ResponseGenerator {
                status: StatusCode::OK,
                headers: FxHashMap::default(),
                body: BodySource::Inline(Arc::new(bytes::Bytes::from(""))),
                delay: None,
                mode: ResponseMode::Static,
                structured_response: false,
                context_uses_headers: true,
                context_uses_body: true,
            },
        };

        // These should NOT overlap - same URL and method, but different headers discriminate
        assert!(
            !MockValidator::mocks_may_overlap(&mock1, &mock2),
            "Mocks with different header values should NOT overlap"
        );
    }
}
