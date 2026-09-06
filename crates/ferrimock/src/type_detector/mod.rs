//! Sophisticated type detection system for mock consolidation
//!
//! This module implements a multi-layered type detection algorithm based on research from:
//! - BigQuery Schema Auto-Detection
//! - Sherlock (MIT) - Deep Learning for Semantic Type Detection
//! - Sato (VLDB 2020) - Contextual Semantic Type Detection
//!
//! ## Detection Layers
//!
//! 1. **Semantic Context Analysis** - Field names provide strong hints
//! 2. **Statistical Feature Extraction** - Character distributions, entropy, length statistics
//! 3. **Priority-Ordered Pattern Matching** - Specific to general type detection
//! 4. **Multi-Sample Validation** - Confidence scoring across multiple values
//!
//! ## Usage
//!
//! ```rust
//! use ferrimock::type_detector::TypeDetector;
//! use serde_json::json;
//!
//! let detector = TypeDetector::new();
//! let values = vec![json!("test@example.com"), json!("user@domain.org")];
//! let value_refs: Vec<&serde_json::Value> = values.iter().collect();
//! let (field_type, confidence) = detector.detect_type("email", &value_refs);
//! ```

pub mod analyzers;
pub mod checkers;
#[allow(clippy::expect_used)]
// Static regex initialization - panics are appropriate for invalid compile-time patterns
pub mod constants;
pub mod features;
pub mod semantic;
pub mod types;

// Re-export public types
pub use features::TypeFeatures;
pub use semantic::detect_from_semantic_context;
pub use types::{
    ArrayPattern, BooleanSpelling, DateFormat, FieldType, ObjectAnalysis, PaginationDirection,
    PaginationScheme, PaginationUrlPattern, TimestampFormat,
};

use serde_json::Value as JsonValue;
use std::sync::Arc;

use analyzers::{analyze_array_pattern, analyze_numbers, analyze_object_pattern};

use checkers::get_checkers;
use constants::DATA_URI_REGEX;
use features::{check_categorical, extract_features};
use semantic::calculate_semantic_boost;

/// What the detector may consult beyond the values in front of it.
#[derive(Clone, Copy)]
pub struct DetectionContext<'a> {
    /// Domain knowledge the built-in heuristics do not have.
    pub profile: &'a dyn crate::profile::ConsolidationProfile,
}

/// Borrowable stand-in for callers with no profile to offer.
static BUILTIN_PROFILE: crate::profile::DefaultProfile = crate::profile::DefaultProfile;

impl DetectionContext<'static> {
    /// A context with no domain knowledge, for callers that have none to pass.
    pub fn builtin() -> Self {
        Self {
            profile: &BUILTIN_PROFILE,
        }
    }
}

impl Default for DetectionContext<'static> {
    fn default() -> Self {
        Self::builtin()
    }
}

/// Main type detection engine
pub struct TypeDetector {
    profile: Arc<dyn crate::profile::ConsolidationProfile>,
    generalize: bool,
}

impl TypeDetector {
    /// Create a detector with only the built-in heuristics.
    pub fn new() -> Self {
        Self {
            profile: crate::profile::default_profile(),
            generalize: false,
        }
    }

    /// Create a detector that consults `profile` before falling back to the
    /// built-in heuristics.
    pub fn with_profile(profile: Arc<dyn crate::profile::ConsolidationProfile>) -> Self {
        Self {
            profile,
            generalize: false,
        }
    }

    /// Read a field seen once as evidence of what it is, rather than as a
    /// constant.
    ///
    /// With several recordings, a field that never changed is a constant, and
    /// that is a fact about the endpoint. With one recording there is no such
    /// fact, and the default reads the value as fixed -- which is why a single
    /// recording produces a copy of itself rather than a template. Turning this
    /// on asks the value what it is instead.
    #[must_use]
    pub fn generalizing(mut self, generalize: bool) -> Self {
        self.generalize = generalize;
        self
    }

    /// Whether a field seen once is read as evidence rather than as a constant.
    pub fn generalizes(&self) -> bool {
        self.generalize
    }

    /// What one recording of a field says it is, when there is nothing to
    /// compare it against.
    ///
    /// `None` means the value stays as it was recorded. That is the honest
    /// answer for anything the detector cannot place: ten random characters in
    /// place of a value the API actually returned is further from the truth
    /// than the value itself, not closer to it.
    pub fn classify_single(&self, field_name: &str, value: &JsonValue) -> Option<FieldType> {
        // A structure is never a literal. Its own fields get this same question,
        // one level down.
        if value.is_object() || value.is_array() {
            return Some(self.detect_type(field_name, &[value]).0);
        }

        // Nothing to say about an absent value, and a flag the client branches
        // on is not something to invent: a `hasNextPage` answered at random
        // sends the caller looking for a page that was never recorded.
        if is_absent(value) || value.is_boolean() {
            return None;
        }
        if value.as_str().is_some_and(str::is_empty) {
            return None;
        }

        let (field_type, _) = self.detect_type(field_name, &[value]);
        match field_type {
            // `RandomString` is the detector's way of saying it does not know.
            // An enum of one sample is the same answer worn differently:
            // choosing at random between a single value is the literal with
            // extra steps, and it claims a set nothing established.
            FieldType::RandomString | FieldType::Categorical { .. } | FieldType::Constant(_) => {
                None
            }
            other => Some(other),
        }
    }

    fn context(&self) -> DetectionContext<'_> {
        DetectionContext {
            profile: self.profile.as_ref(),
        }
    }

    /// Detect type with confidence score using field name context
    ///
    /// # Arguments
    /// * `field_name` - The name of the field (provides semantic context)
    /// * `values` - Sample values from the field
    ///
    /// # Returns
    /// Tuple of (detected type, confidence score 0.0-1.0)
    pub fn detect_type(&self, field_name: &str, values: &[&JsonValue]) -> (FieldType, f64) {
        if values.is_empty() {
            return (FieldType::RandomString, 0.5);
        }

        // Samples that stand in for a value rather than being one are dropped
        // before anything looks at them. A recording is full of nulls, `N/A`s and
        // values a proxy blanked, and every check below asks what share of the
        // samples match -- so one `N/A` in three drags the share to 0.67 and a
        // field of perfectly good locale codes is answered `RandomString`.
        //
        // The field is not less of a locale field because one response had
        // nothing to put in it.
        let present: Vec<&JsonValue> = values
            .iter()
            .copied()
            .filter(|value| !is_absent(value))
            .collect();
        // Unless there is nothing else: a field that only ever held `N/A` is
        // described by that, and pretending it is empty says less.
        let values = if present.is_empty() { values } else { &present };

        // Layer 0: the profile knows this API and the heuristics do not.
        if let Some((field_type, confidence)) = self.profile.classify_field(field_name, values) {
            return (keep_kind(field_type, values), confidence);
        }

        // Layer 1: Semantic context from field name (strong hints return immediately)
        if let Some((field_type, confidence)) =
            detect_from_semantic_context(field_name, values, &self.context())
        {
            let strings: Vec<&str> = values.iter().filter_map(|value| value.as_str()).collect();
            return (keep_kind(refine(field_type, &strings), values), confidence);
        }

        // Layer 2-4: Pattern-based detection with weighted scoring
        let (field_type, base_confidence) = self.detect_type_from_values(values);

        // Apply semantic boost based on field name
        let boost = calculate_semantic_boost(field_name, &field_type);
        let boosted_confidence = (base_confidence * (1.0 + boost)).clamp(0.0, 1.0);

        (keep_kind(field_type, values), boosted_confidence)
    }

    /// Detect type without field name context (pattern-based only)
    pub fn detect_type_from_values(&self, values: &[&JsonValue]) -> (FieldType, f64) {
        if values.is_empty() {
            return (FieldType::RandomString, 0.5);
        }

        // Check for JSON primitive types first
        if values.iter().all(|v| v.is_number()) {
            return analyze_numbers(values);
        }

        if values.iter().all(|v| v.is_boolean()) {
            return (
                FieldType::Boolean {
                    spelling: BooleanSpelling::default(),
                },
                1.0,
            );
        }

        if values.iter().all(|v| v.is_array()) {
            return analyze_array_pattern(values, |vals| self.detect_type_from_values(vals));
        }

        if values.iter().all(|v| v.is_object()) {
            return analyze_object_pattern(values, self);
        }

        // String type detection - extract string values
        let strings: Option<Vec<&str>> = values.iter().map(|v| v.as_str()).collect();

        if let Some(strs) = strings {
            // Check for categorical/enum before feature extraction (low cardinality)
            if let Some(categorical) = check_categorical(&strs) {
                return categorical;
            }

            // Extract statistical features
            let features = extract_features(&strs);

            // Run priority-ordered pattern detection
            let (field_type, confidence) = detect_from_patterns(values, &features, &self.context());
            (keep_kind(refine(field_type, &strs), values), confidence)
        } else {
            (FieldType::RandomString, 0.5)
        }
    }
}

/// Keep the JSON kind the recording used.
///
/// A class says what a value *is*; it does not say whether the API wrote it as
/// a number or as text. `"sequence_id": "0"` is a sequence and a string at once,
/// and answering it with a bare `0` changes the type every client parses --
/// which the shape check reports as a divergence even though the class was
/// right.
fn keep_kind(field_type: FieldType, values: &[&JsonValue]) -> FieldType {
    if !field_type.writes_bare() {
        return field_type;
    }
    // Only when the recording never wrote this field as anything but text. A
    // field holding both `0` and `"0"` is already inconsistent, and the bare
    // form is the one that reads back as a number.
    if values.iter().all(|value| value.is_string()) {
        FieldType::Stringified(Box::new(field_type))
    } else {
        field_type
    }
}

/// Record the shape the values were actually written in.
///
/// A checker answers with the *class* of a field; which spelling of that class
/// the recording used is read back off the samples here. Both halves matter: a
/// field of `17/03/2024`s answered with `2024-03-17` is the right class and the
/// wrong value, and a client parsing it breaks either way.
fn refine(field_type: FieldType, values: &[&str]) -> FieldType {
    /// The shape the majority of the samples share, if they agree at all.
    fn agreed<T: Copy + PartialEq>(values: &[&str], read: impl Fn(&str) -> Option<T>) -> Option<T> {
        let seen: Vec<T> = values.iter().filter_map(|value| read(value)).collect();
        let first = *seen.first()?;
        seen.iter().all(|shape| *shape == first).then_some(first)
    }

    match field_type {
        // A flag keeps the spelling it was written in, for the same reason a
        // date keeps its format.
        FieldType::Boolean { .. } => FieldType::Boolean {
            spelling: agreed(values, BooleanSpelling::of).unwrap_or_default(),
        },
        FieldType::IsoDate { .. } => FieldType::IsoDate {
            format: agreed(values, DateFormat::of).unwrap_or_default(),
        },
        FieldType::Timestamp { .. } => FieldType::Timestamp {
            format: agreed(values, TimestampFormat::of).unwrap_or_default(),
        },
        FieldType::HexString { .. } => {
            let stripped = |value: &str| value.trim_start_matches('#').to_string();
            let length = agreed(values, |value| Some(stripped(value).len()));
            // Upper case only when every sample that has a letter uses it --
            // hex digits are half digits, and a sample of `00112233` says
            // nothing about case either way.
            let cased: Vec<bool> = values
                .iter()
                .filter(|value| value.chars().any(char::is_alphabetic))
                .map(|value| {
                    value
                        .chars()
                        .filter(|c| c.is_alphabetic())
                        .all(char::is_uppercase)
                })
                .collect();
            FieldType::HexString {
                length,
                upper: !cased.is_empty() && cased.iter().all(|upper| *upper),
            }
        }
        other => other,
    }
}

/// Whether a sample stands in for a value rather than being one.
///
/// Recordings carry these constantly: a field the upstream had nothing for, a
/// value a redacting proxy blanked, a placeholder somebody typed. None of them
/// says anything about what the field holds, and counting them as evidence
/// against every pattern is how a field gets no answer at all.
fn is_absent(value: &JsonValue) -> bool {
    let JsonValue::String(text) = value else {
        return value.is_null();
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    // Wholly masked: `***`, `••••`, `XXXXXXXX`.
    if trimmed.len() >= 3
        && (trimmed.chars().all(|c| c == '*')
            || trimmed.chars().all(|c| c == '\u{2022}')
            || trimmed.chars().all(|c| c == 'X'))
    {
        return true;
    }
    // Bracketed markers: `[REDACTED]`, `<hidden>`.
    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('<') && trimmed.ends_with('>'))
    {
        return true;
    }
    // Partly masked: `****ient`, `4111****1111`. A proxy that hides the middle
    // of a value leaves something that is no longer the value, and counting it
    // as one drags every match ratio down -- one masked sample in two is enough
    // to put a field of perfectly good URLs under any threshold.
    if masked_run(trimmed) {
        return true;
    }

    matches!(
        trimmed.to_lowercase().as_str(),
        "-" | "--"
            | "n/a"
            | "na"
            | "null"
            | "nil"
            | "none"
            | "unknown"
            | "undefined"
            | "tbd"
            | "not set"
            | "not available"
            | "no value"
    )
}

/// Whether text carries a run of masking characters long enough to be a
/// redaction rather than part of the value.
///
/// Three is the shortest run a masker uses and longer than any value spells by
/// accident. `X` is deliberately not a mask character here: it is a letter, and
/// `XXX` turns up inside real values.
fn masked_run(text: &str) -> bool {
    const SHORTEST_MASK: usize = 3;

    let mut run = 0;
    let mut previous = '\0';
    for character in text.chars() {
        if matches!(character, '*' | '\u{2022}') && (run == 0 || character == previous) {
            run += 1;
            if run >= SHORTEST_MASK {
                return true;
            }
        } else {
            run = 0;
        }
        previous = character;
    }
    false
}

/// Layer 3 & 4: Priority-ordered pattern matching with multi-sample validation
/// Now uses weighted scoring to collect all potential types
fn detect_from_patterns(
    values: &[&JsonValue],
    features: &features::TypeFeatures,
    ctx: &DetectionContext<'_>,
) -> (FieldType, f64) {
    let strings: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();

    if strings.is_empty() {
        return (FieldType::RandomString, 0.5);
    }

    // Collect all potential types with their scores
    let mut potential_types: Vec<(FieldType, f64)> = Vec::new();

    for checker in get_checkers() {
        if let Some(confidence) = (checker.checker_fn)(&strings, features, ctx)
            && confidence >= checker.threshold
        {
            potential_types.push((checker.field_type, confidence));
        }
    }

    // If no types passed threshold, return default
    if potential_types.is_empty() {
        return (FieldType::RandomString, 0.5);
    }

    // Return the type with highest confidence
    let (field_type, confidence) = potential_types
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or((FieldType::RandomString, 0.5));

    // For DownloadUrl, add sample URL for file type detection
    if matches!(field_type, FieldType::DownloadUrl { .. }) {
        let sample_url = strings
            .iter()
            .find(|s| s.len() > 100)
            .map(|s| (*s).to_string());
        return (FieldType::DownloadUrl { sample_url }, confidence);
    }

    // A URL whose path ends in an image extension is an image URL whether or not
    // the field name said so -- which is the only thing that can classify one
    // sitting under a name like `data` or `value`. Narrowing the winner here
    // rather than adding a competing checker keeps it independent of the order
    // checkers run in.
    if matches!(field_type, FieldType::Url) && strings.iter().all(|url| is_image_url(url)) {
        return (FieldType::ImageUrl, confidence);
    }

    // For DataUri, extract mime type for smart generation
    if matches!(field_type, FieldType::DataUri { .. }) {
        let mime_type = strings.iter().find_map(|s| {
            DATA_URI_REGEX
                .captures(s)
                .and_then(|caps| caps.get(1))
                .map(|m| m.as_str().to_string())
        });
        return (FieldType::DataUri { mime_type }, confidence);
    }

    (field_type, confidence)
}

/// Whether a URL's path ends in an image file extension.
fn is_image_url(url: &str) -> bool {
    const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];

    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

impl Default for TypeDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::uninlined_format_args,
    clippy::float_cmp,
    clippy::match_wildcard_for_single_variants,
    clippy::manual_string_new
)]
mod tests;
