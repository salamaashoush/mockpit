//! Replay-based fidelity verification for consolidation.
//!
//! Consolidation is lossy compression of a recording, and its only self-check
//! used to be "does the generated template parse". A reduction ratio on its own
//! is unfalsifiable -- collapsing every mock into one scores 99%.
//!
//! This verifies consolidation the way a codec is verified: decode the
//! compressed artifact and diff it against the original. Every recorded request
//! is replayed against the consolidated collection and the answer compared to
//! what was actually recorded, at levels that fail independently:
//!
//! | Level | Question |
//! | --- | --- |
//! | matched | did anything answer at all? |
//! | no cross-talk | did the *right lineage* answer, or a stranger? |
//! | status exact | same status code? |
//! | shape equal | same JSON key set and value kinds, recursively? |
//! | constants held | did object fields the recording never varied stay put? |
//! | value equal | byte-identical (only expected where a group was duplicates) |
//!
//! Cross-talk is the level nothing else catches. An over-broad `{id}` pattern
//! still "matches", still returns 200, still has the right shape -- and answers
//! with a different resource's body. Lineage is what exposes it, which is why
//! [`Provenance`] is threaded through consolidation.
//!
//! The same checks run against the *unconsolidated* collection as a baseline, so
//! a failure can be attributed: a recording the recorder itself cannot replay is
//! not the consolidator's fault.

use crate::Result;
use crate::config::MockCollectionConfig;
use crate::consolidator::provenance::Provenance;
use crate::engine::{MockAction, MockMatcher, MockRegistry};
use crate::recorder::{RecordedInteraction, RecordedRequest};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use lean_string::LeanString;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

/// Header the matcher stamps on every generated response, carrying the id of
/// the mock that answered.
const MOCK_ID_HEADER: &str = "x-mock-id";
/// Header the matcher stamps when a mock's template failed to render.
const MOCK_ERROR_HEADER: &str = "x-mock-error";

/// Knobs for how strictly a replay must reproduce its recording.
#[derive(Debug, Clone)]
pub struct FidelityOptions {
    /// Directory that relative `file:` response bodies resolve against.
    pub base_dir: Option<PathBuf>,
    /// Treat a JSON integer and a JSON float as different kinds.
    pub strict_numbers: bool,
    /// Require a replayed array to have the same length as the recording.
    /// Off by default: a template that generates a plausible number of items is
    /// doing its job, and array length is rarely load-bearing.
    pub strict_array_len: bool,
    /// Treat `null` against a non-null value as a kind divergence.
    pub strict_null: bool,
    /// How many leading array elements to compare. Arrays in real recordings run
    /// to thousands of entries and their shape is homogeneous; probing the head
    /// finds kind divergences without walking the tail.
    pub array_probe: usize,
    /// Cap on leaf values collected per response body. Counts in
    /// [`FidelityScore`] stay exact; only the walk is bounded.
    pub leaf_budget: usize,
    /// Cap on stored examples per divergence kind. Counts stay exact.
    pub max_examples: usize,
    /// Clear the process-global template persistence store before verifying.
    ///
    /// Pagination templates written by the consolidator call `store_get_or_set`,
    /// so replays leave counters behind that a later run would read back. The
    /// store is process-global and shared with any live mock server in the same
    /// process, so this defaults to off; a standalone verification run (the CLI)
    /// should turn it on.
    pub reset_persistence: bool,
}

impl Default for FidelityOptions {
    fn default() -> Self {
        Self {
            base_dir: None,
            strict_numbers: true,
            strict_array_len: false,
            strict_null: true,
            array_probe: 8,
            leaf_budget: 4096,
            max_examples: 25,
            reset_persistence: false,
        }
    }
}

/// Per-level tallies over the whole recording.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FidelityScore {
    pub total: usize,
    pub matched: usize,
    pub no_cross_talk: usize,
    pub status_exact: usize,
    pub shape_equal: usize,
    pub constants_held: usize,
    pub value_equal: usize,
    /// Scalar leaves the recordings carried, across every interaction.
    ///
    /// `value_equal` is all-or-nothing per interaction: a response whose id now
    /// answers correctly still scores nothing while any other field is
    /// generated. Counting leaves says how much of each answer is right, which
    /// is the only way a change to one field shows up at all.
    pub leaves: usize,
    /// Leaves the replay answered with the recorded value.
    pub leaves_equal: usize,
    /// Interactions clearing every level except `value_equal` -- the headline
    /// number, since templating deliberately varies values.
    pub behavioral: usize,
}

#[allow(clippy::cast_precision_loss)] // Interaction counts are far below f64's exact range
impl FidelityScore {
    fn ratio(part: usize, total: usize) -> f64 {
        if total == 0 {
            1.0
        } else {
            part as f64 / total as f64
        }
    }

    pub fn matched_ratio(&self) -> f64 {
        Self::ratio(self.matched, self.total)
    }

    pub fn no_cross_talk_ratio(&self) -> f64 {
        Self::ratio(self.no_cross_talk, self.total)
    }

    pub fn status_exact_ratio(&self) -> f64 {
        Self::ratio(self.status_exact, self.total)
    }

    pub fn shape_equal_ratio(&self) -> f64 {
        Self::ratio(self.shape_equal, self.total)
    }

    pub fn constants_held_ratio(&self) -> f64 {
        Self::ratio(self.constants_held, self.total)
    }

    pub fn value_equal_ratio(&self) -> f64 {
        Self::ratio(self.value_equal, self.total)
    }

    /// Fraction of recorded leaves the replay answered with the recorded value.
    pub fn leaves_equal_ratio(&self) -> f64 {
        Self::ratio(self.leaves_equal, self.leaves)
    }

    /// Fraction of interactions that survived consolidation behaviourally.
    pub fn behavioral_ratio(&self) -> f64 {
        Self::ratio(self.behavioral, self.total)
    }
}

/// A recorded interaction, identified enough to find again.
#[derive(Debug, Clone)]
pub struct InteractionRef {
    pub interaction_id: String,
    pub method: String,
    pub target: String,
}

/// A request answered by a mock that does not descend from its own recording.
#[derive(Debug, Clone)]
pub struct CrossTalk {
    pub interaction: InteractionRef,
    pub matched_mock: String,
    pub expected_origin: String,
}

/// A replay that answered from the right lineage but got something wrong.
#[derive(Debug, Clone)]
pub struct Divergence {
    pub interaction: InteractionRef,
    pub mock_id: String,
    pub detail: String,
}

/// What replaying a whole recording revealed.
#[derive(Debug, Clone)]
pub struct FidelityReport {
    /// Scores for the consolidated collection.
    pub score: FidelityScore,
    /// The same scores for the collection *before* consolidation. Anything
    /// already failing here is a recorder or matcher problem, not a
    /// consolidation problem, and the delta is what consolidation actually cost.
    pub baseline: FidelityScore,
    /// Requests nothing in the consolidated collection answered.
    pub unmatched: Vec<InteractionRef>,
    /// Requests the *original* collection could not answer either.
    pub baseline_unmatched: Vec<InteractionRef>,
    pub cross_talk: Vec<CrossTalk>,
    pub status_mismatch: Vec<Divergence>,
    pub shape_mismatch: Vec<Divergence>,
    pub constant_drift: Vec<Divergence>,
    pub render_errors: Vec<Divergence>,
    /// True when at least one example list hit `max_examples`. The counts in
    /// [`Self::score`] are unaffected.
    pub examples_capped: bool,
}

impl FidelityReport {
    /// Whether consolidation preserved behaviour at or above `threshold`.
    pub fn passes(&self, threshold: f64) -> bool {
        self.score.behavioral_ratio() >= threshold
    }

    /// How much behavioural fidelity consolidation cost relative to the
    /// unconsolidated recording. Negative means consolidation lost ground.
    pub fn behavioral_delta(&self) -> f64 {
        self.score.behavioral_ratio() - self.baseline.behavioral_ratio()
    }
}

/// Replay `interactions` against both collections and report what changed.
///
/// `provenance` must describe `consolidated`; pass the map that
/// [`crate::consolidator::MockConsolidator`] produced for it. An empty
/// provenance makes every lineage unprovable and every match reads as
/// cross-talk, which is the honest answer for a collection nobody can vouch for.
pub async fn verify(
    interactions: &[RecordedInteraction],
    original: &MockCollectionConfig,
    consolidated: &MockCollectionConfig,
    provenance: &Provenance,
    options: &FidelityOptions,
) -> Result<FidelityReport> {
    if options.reset_persistence {
        crate::template::get_global_persistence_store().clear();
    }

    let base_dir = options.base_dir.as_deref();
    let original_matcher = build_matcher(original, base_dir).await?;
    let origins = resolve_origins(&original_matcher, interactions);

    let identity = identity_provenance(original);
    let baseline = evaluate(
        &original_matcher,
        &identity,
        interactions,
        &origins,
        options,
    )
    .await;

    if options.reset_persistence {
        crate::template::get_global_persistence_store().clear();
    }

    let consolidated_matcher = build_matcher(consolidated, base_dir).await?;
    let mut main = evaluate(
        &consolidated_matcher,
        provenance,
        interactions,
        &origins,
        options,
    )
    .await;

    main.baseline_unmatched = interactions
        .iter()
        .zip(origins.iter())
        .filter(|(_, origin)| origin.is_none())
        .map(|(interaction, _)| interaction_ref(interaction))
        .take(options.max_examples)
        .collect();

    Ok(FidelityReport {
        score: main.score,
        baseline: baseline.score,
        unmatched: main.unmatched,
        baseline_unmatched: main.baseline_unmatched,
        cross_talk: main.cross_talk,
        status_mismatch: main.status_mismatch,
        shape_mismatch: main.shape_mismatch,
        constant_drift: main.constant_drift,
        render_errors: main.render_errors,
        examples_capped: main.examples_capped,
    })
}

/// Accumulator for one pass over the recording.
#[derive(Debug, Default)]
struct Evaluation {
    score: FidelityScore,
    unmatched: Vec<InteractionRef>,
    baseline_unmatched: Vec<InteractionRef>,
    cross_talk: Vec<CrossTalk>,
    status_mismatch: Vec<Divergence>,
    shape_mismatch: Vec<Divergence>,
    constant_drift: Vec<Divergence>,
    render_errors: Vec<Divergence>,
    examples_capped: bool,
}

/// Keep at most `cap` examples, flagging the report once any list overflows.
/// Counts in [`FidelityScore`] are tallied separately and stay exact.
fn push_capped<T>(sink: &mut Vec<T>, capped: &mut bool, item: T, cap: usize) {
    if sink.len() < cap {
        sink.push(item);
    } else {
        *capped = true;
    }
}

async fn evaluate(
    matcher: &MockMatcher,
    provenance: &Provenance,
    interactions: &[RecordedInteraction],
    origins: &[Option<LeanString>],
    options: &FidelityOptions,
) -> Evaluation {
    let mut eval = Evaluation {
        score: FidelityScore {
            total: interactions.len(),
            ..FidelityScore::default()
        },
        ..Evaluation::default()
    };

    let merged_into = merge_targets(provenance);
    let agreed = agreed_constants_by_group(interactions, origins, &merged_into, options);

    for (interaction, origin) in interactions.iter().zip(origins.iter()) {
        let reference = interaction_ref(interaction);
        let Some(replay) = replay_one(matcher, &interaction.request).await else {
            push_capped(
                &mut eval.unmatched,
                &mut eval.examples_capped,
                reference,
                options.max_examples,
            );
            continue;
        };
        eval.score.matched += 1;

        // Lineage. An interaction the original collection could not answer has
        // no origin to compare against, so it cannot prove cross-talk either way
        // and is credited -- the miss is already reported as baseline_unmatched.
        let lineage_ok = match origin {
            Some(origin) => provenance.descends_from(&replay.mock_id, origin.as_str()),
            None => true,
        };
        if lineage_ok {
            eval.score.no_cross_talk += 1;
        } else {
            let expected_origin = origin
                .as_ref()
                .map_or_else(String::new, ToString::to_string);
            push_capped(
                &mut eval.cross_talk,
                &mut eval.examples_capped,
                CrossTalk {
                    interaction: reference.clone(),
                    matched_mock: replay.mock_id.clone(),
                    expected_origin,
                },
                options.max_examples,
            );
        }

        if replay.render_failed {
            push_capped(
                &mut eval.render_errors,
                &mut eval.examples_capped,
                Divergence {
                    interaction: reference.clone(),
                    mock_id: replay.mock_id.clone(),
                    detail: replay.body,
                },
                options.max_examples,
            );
            continue;
        }

        let status_ok = replay.status == interaction.response.status;
        if status_ok {
            eval.score.status_exact += 1;
        } else {
            push_capped(
                &mut eval.status_mismatch,
                &mut eval.examples_capped,
                Divergence {
                    interaction: reference.clone(),
                    mock_id: replay.mock_id.clone(),
                    detail: format!(
                        "recorded {} but replayed {}",
                        interaction.response.status, replay.status
                    ),
                },
                options.max_examples,
            );
        }

        let recorded_json = serde_json::from_str::<JsonValue>(&interaction.response.body).ok();
        let replayed_json = serde_json::from_str::<JsonValue>(&replay.body).ok();

        let shape_divergences = match (&recorded_json, &replayed_json) {
            (Some(recorded), Some(replayed)) => compare_shape(recorded, replayed, options),
            (None, None) => Vec::new(),
            (Some(_), None) => vec!["recorded JSON but replayed body is not JSON".to_string()],
            (None, Some(_)) => vec!["recorded body is not JSON but replayed JSON".to_string()],
        };
        let shape_ok = shape_divergences.is_empty();
        if shape_ok {
            eval.score.shape_equal += 1;
        } else {
            push_capped(
                &mut eval.shape_mismatch,
                &mut eval.examples_capped,
                Divergence {
                    interaction: reference.clone(),
                    mock_id: replay.mock_id.clone(),
                    detail: shape_divergences.join("; "),
                },
                options.max_examples,
            );
        }

        // Constants the recording never varied within this lineage must survive.
        // Missing keys are already reported as shape divergences, so only keys
        // present in both are compared -- one defect, one report.
        let lineage_constants = origin
            .as_ref()
            .and_then(|o| merged_into.get(o))
            .and_then(|group| agreed.get(group));
        let constant_divergences: Vec<String> = match (lineage_constants, &replayed_json) {
            (Some(constants), Some(replayed)) => {
                let leaves = flatten_leaves(replayed, options.leaf_budget);
                constants
                    .leaves
                    .iter()
                    .filter_map(|(pointer, expected)| {
                        let actual = leaves.get(pointer)?;
                        (actual != expected).then(|| {
                            format!("{pointer}: recorded {expected} but replayed {actual}")
                        })
                    })
                    // A value every element of a list agreed on is as much a
                    // constant as a top-level field, and templating the list is
                    // no licence to invent one.
                    .chain(element_constant_drift(
                        replayed,
                        &constants.elements.constants(),
                    ))
                    .collect()
            }
            _ => Vec::new(),
        };
        let constants_ok = constant_divergences.is_empty();
        if constants_ok {
            eval.score.constants_held += 1;
        } else {
            push_capped(
                &mut eval.constant_drift,
                &mut eval.examples_capped,
                Divergence {
                    interaction: reference.clone(),
                    mock_id: replay.mock_id.clone(),
                    detail: constant_divergences.join("; "),
                },
                options.max_examples,
            );
        }

        let value_equal = match (&recorded_json, &replayed_json) {
            (Some(recorded), Some(replayed)) => recorded == replayed,
            _ => interaction.response.body == replay.body,
        };
        if value_equal {
            eval.score.value_equal += 1;
        }

        if let (Some(recorded), Some(replayed)) = (&recorded_json, &replayed_json) {
            let recorded_leaves = flatten_leaves(recorded, options.leaf_budget);
            let replayed_leaves = flatten_leaves(replayed, options.leaf_budget);
            eval.score.leaves += recorded_leaves.len();
            eval.score.leaves_equal += recorded_leaves
                .iter()
                .filter(|(pointer, expected)| replayed_leaves.get(*pointer) == Some(*expected))
                .count();
        }

        if lineage_ok && status_ok && shape_ok && constants_ok {
            eval.score.behavioral += 1;
        }
    }

    eval
}

/// What a single replay produced.
struct Replay {
    mock_id: String,
    status: u16,
    body: String,
    render_failed: bool,
}

async fn replay_one(matcher: &MockMatcher, request: &RecordedRequest) -> Option<Replay> {
    let method = Method::from_bytes(request.method.as_bytes()).ok()?;
    let (path, query) = split_target(request);
    let headers = to_header_map(&request.headers);
    let body = request.body.as_ref().map(String::as_bytes);

    let action = matcher
        .try_match_parts(&method, &path, query.as_deref(), &headers, body)
        .await?;

    match action {
        MockAction::FullMock(response) => {
            let status = response.status().as_u16();
            let mock_id = response
                .headers()
                .get(MOCK_ID_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let render_failed = response.headers().contains_key(MOCK_ERROR_HEADER);
            let body = String::from_utf8_lossy(response.body()).into_owned();
            Some(Replay {
                mock_id,
                status,
                body,
                render_failed,
            })
        }
        // A recorded collection never produces passthrough mocks; if one shows
        // up the replay cannot be compared to a recording, so report it as a
        // miss rather than inventing a verdict.
        MockAction::PatchUpstream { .. } => None,
    }
}

/// Which mock in the unconsolidated collection each interaction belongs to.
///
/// Uses [`MockMatcher::find_match`] rather than a full replay: lineage only
/// needs the id, and skipping rendering keeps the pass free of template side
/// effects.
fn resolve_origins(
    matcher: &MockMatcher,
    interactions: &[RecordedInteraction],
) -> Vec<Option<LeanString>> {
    interactions
        .iter()
        .map(|interaction| {
            let request = &interaction.request;
            let method = Method::from_bytes(request.method.as_bytes()).ok()?;
            let (path, query) = split_target(request);
            let headers = to_header_map(&request.headers);
            let body = request.body.as_ref().map(String::as_bytes);
            matcher
                .find_match(&method, &path, query.as_deref(), &headers, body)
                .map(|m| m.mock.id.clone())
        })
        .collect()
}

/// Which consolidated mock each original mock was merged into.
fn merge_targets(provenance: &Provenance) -> FxHashMap<LeanString, LeanString> {
    let mut targets = FxHashMap::default();
    for (consolidated, origins) in provenance.entries() {
        for origin in origins {
            targets
                .entry(origin.clone())
                .or_insert(consolidated.clone());
        }
    }
    targets
}

/// Leaf values every interaction merged into the same mock agreed on.
///
/// Computed from the recording itself, never from the analyzer's own
/// `constant_fields` -- the analyzer must not grade its own homework. Grouping
/// by the *merge target* rather than by origin is what makes this a statement
/// about consolidation: for a collection that was never consolidated every
/// group is a single recording, so agreement degenerates to "reproduce the body
/// exactly", which is precisely the right bar for an unconsolidated mock.
fn agreed_constants_by_group(
    interactions: &[RecordedInteraction],
    origins: &[Option<LeanString>],
    merged_into: &FxHashMap<LeanString, LeanString>,
    options: &FidelityOptions,
) -> FxHashMap<LeanString, GroupConstants> {
    let mut by_group: FxHashMap<LeanString, GroupConstants> = FxHashMap::default();
    // A group with any non-JSON member has no comparable leaves at all, and must
    // stay empty however many JSON members follow it.
    let mut poisoned: FxHashSet<LeanString> = FxHashSet::default();

    for (interaction, origin) in interactions.iter().zip(origins.iter()) {
        let Some(group) = origin.as_ref().and_then(|o| merged_into.get(o)) else {
            continue;
        };
        if poisoned.contains(group) {
            continue;
        }

        let Ok(body) = serde_json::from_str::<JsonValue>(&interaction.response.body) else {
            poisoned.insert(group.clone());
            by_group.insert(group.clone(), GroupConstants::default());
            continue;
        };

        let leaves = flatten_leaves(&body, options.leaf_budget);
        let elements = collect_element_fields(&body);
        match by_group.get_mut(group) {
            None => {
                by_group.insert(group.clone(), GroupConstants { leaves, elements });
            }
            Some(agreed) => {
                agreed
                    .leaves
                    .retain(|pointer, value| leaves.get(pointer) == Some(&*value));
                agreed.elements.absorb(&elements);
            }
        }
    }

    by_group
}

/// What a group of recordings agreed on, and therefore what consolidating them
/// is not allowed to change.
#[derive(Debug, Default)]
struct GroupConstants {
    /// Object fields, and whole arrays, that never varied.
    leaves: FxHashMap<String, JsonValue>,
    /// What the group's array elements said about themselves.
    elements: ElementFields,
}

fn identity_provenance(collection: &MockCollectionConfig) -> Provenance {
    let mut provenance = Provenance::new();
    for mock in &collection.mocks {
        provenance.record_identity(mock.id.clone());
    }
    provenance
}

async fn build_matcher(
    collection: &MockCollectionConfig,
    base_dir: Option<&Path>,
) -> Result<MockMatcher> {
    let mut collection = collection.clone();
    // Verification asks what a mock answers, not when. A recording made against
    // a real service carries its latency, and the HAR converter keeps it -- so
    // replaying honestly means sleeping through every recorded second, and a
    // few hundred interactions take an afternoon. Fidelity is not a timing
    // measurement, and nothing here reads the clock.
    strip_delays(&mut collection);
    strip_behaviour(&mut collection);

    let definitions = collection
        .into_mock_definitions_with_dir(base_dir, None)
        .await?;
    Ok(MockMatcher::new(MockRegistry::with_mocks(definitions)))
}

/// Drop every behaviour a mount asked for beyond answering.
///
/// Not a default — a constraint. This scores status, shape and value equality
/// against what was recorded, and a 304, a 412 or a record a lagging replica
/// is holding back fails all three. It fails the *unconsolidated* baseline
/// identically, so the attribution logic could not tell a consolidator bug
/// from a mock behaving exactly as its mount asked it to.
fn strip_behaviour(collection: &mut MockCollectionConfig) {
    use crate::config::ServeConfig;

    for mock in &mut collection.mocks {
        if let Some(ServeConfig::Explicit {
            protocol, schema, ..
        }) = &mock.serve
        {
            mock.serve = Some(ServeConfig::Explicit {
                protocol: protocol.clone(),
                schema: schema.clone(),
                behaviour: crate::config::serve::Behaviour::none(),
            });
        }
    }
}

/// Drop every configured delay from a collection about to be replayed.
fn strip_delays(collection: &mut MockCollectionConfig) {
    for mock in &mut collection.mocks {
        mock.delay = None;
    }
}

fn interaction_ref(interaction: &RecordedInteraction) -> InteractionRef {
    let (path, query) = split_target(&interaction.request);
    let target = query.map_or_else(|| path.clone(), |q| format!("{path}?{q}"));
    InteractionRef {
        interaction_id: interaction.id.clone(),
        method: interaction.request.method.clone(),
        target,
    }
}

/// Split a recorded request into the path and query the matcher expects.
///
/// `uri` is whatever the recording caller passed: a bare path, a path with a
/// query already attached, or an absolute URL. An explicit `query` wins when the
/// caller supplied one.
fn split_target(request: &RecordedRequest) -> (String, Option<String>) {
    let (path, embedded_query) = if let Ok(url) = url::Url::parse(&request.uri)
        && url.has_host()
    {
        (
            url.path().to_string(),
            url.query().map(std::string::ToString::to_string),
        )
    } else if let Some(split) = request.uri.split_once('?') {
        (split.0.to_string(), Some(split.1.to_string()))
    } else {
        (request.uri.clone(), None)
    };

    let query = request
        .query
        .as_ref()
        .filter(|q| !q.is_empty())
        .cloned()
        .or(embedded_query);

    (path, query)
}

fn to_header_map(headers: &[(String, String)]) -> HeaderMap {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            map.append(name, value);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// JSON shape comparison
// ---------------------------------------------------------------------------

/// The kind of a JSON value, at the granularity shape comparison cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Null,
    Bool,
    Integer,
    Float,
    String,
    Array,
    Object,
}

impl Kind {
    fn of(value: &JsonValue, strict_numbers: bool) -> Self {
        match value {
            JsonValue::Null => Self::Null,
            JsonValue::Bool(_) => Self::Bool,
            JsonValue::Number(n) => {
                if !strict_numbers || n.is_i64() || n.is_u64() {
                    Self::Integer
                } else {
                    Self::Float
                }
            }
            JsonValue::String(_) => Self::String,
            JsonValue::Array(_) => Self::Array,
            JsonValue::Object(_) => Self::Object,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool => "boolean",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
        }
    }
}

fn compare_shape(
    recorded: &JsonValue,
    replayed: &JsonValue,
    options: &FidelityOptions,
) -> Vec<String> {
    let mut divergences = Vec::new();
    walk_shape(recorded, replayed, "", options, &mut divergences);
    divergences
}

fn walk_shape(
    recorded: &JsonValue,
    replayed: &JsonValue,
    pointer: &str,
    options: &FidelityOptions,
    out: &mut Vec<String>,
) {
    let here = if pointer.is_empty() { "/" } else { pointer };

    let recorded_kind = Kind::of(recorded, options.strict_numbers);
    let replayed_kind = Kind::of(replayed, options.strict_numbers);

    if recorded_kind != replayed_kind {
        let nullable =
            !options.strict_null && (recorded_kind == Kind::Null || replayed_kind == Kind::Null);
        if !nullable {
            out.push(format!(
                "{here}: recorded {} but replayed {}",
                recorded_kind.name(),
                replayed_kind.name()
            ));
        }
        return;
    }

    match (recorded, replayed) {
        (JsonValue::Object(recorded_map), JsonValue::Object(replayed_map)) => {
            let missing: Vec<&str> = recorded_map
                .keys()
                .filter(|k| !replayed_map.contains_key(*k))
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                out.push(format!("{here}: replay dropped {}", missing.join(", ")));
            }

            let extra: Vec<&str> = replayed_map
                .keys()
                .filter(|k| !recorded_map.contains_key(*k))
                .map(String::as_str)
                .collect();
            if !extra.is_empty() {
                out.push(format!("{here}: replay invented {}", extra.join(", ")));
            }

            for (key, recorded_value) in recorded_map {
                if let Some(replayed_value) = replayed_map.get(key) {
                    walk_shape(
                        recorded_value,
                        replayed_value,
                        &format!("{pointer}/{key}"),
                        options,
                        out,
                    );
                }
            }
        }
        (JsonValue::Array(recorded_items), JsonValue::Array(replayed_items)) => {
            if options.strict_array_len && recorded_items.len() != replayed_items.len() {
                out.push(format!(
                    "{here}: recorded {} items but replayed {}",
                    recorded_items.len(),
                    replayed_items.len()
                ));
            } else if !recorded_items.is_empty() && replayed_items.is_empty() {
                out.push(format!(
                    "{here}: recorded {} items but replayed none",
                    recorded_items.len()
                ));
            }

            let probe = options
                .array_probe
                .min(recorded_items.len())
                .min(replayed_items.len());
            for index in 0..probe {
                let (Some(recorded_item), Some(replayed_item)) =
                    (recorded_items.get(index), replayed_items.get(index))
                else {
                    continue;
                };
                walk_shape(
                    recorded_item,
                    replayed_item,
                    &format!("{pointer}/{index}"),
                    options,
                    out,
                );
            }
        }
        _ => {}
    }
}

/// Flatten a JSON value to `pointer -> value` pairs, bounded by `budget`.
///
/// Objects are descended into; an array is taken whole, as a single leaf at its
/// own pointer. Both halves of that matter.
///
/// Descending into an array would address its contents by position, and position
/// is not identity: a list of one and a list of three can agree on element 0 by
/// coincidence, and a template asked to reproduce that coincidence could only do
/// it by emitting a fixed list.
///
/// Taking the array whole says something stronger and true. If every recording
/// in a group returned the *same* list, that list is as constant as any scalar,
/// and a template that randomises it has changed what the endpoint says.
fn flatten_leaves(value: &JsonValue, budget: usize) -> FxHashMap<String, JsonValue> {
    let mut out = FxHashMap::default();
    let mut remaining = budget;
    collect_leaves(value, &mut String::new(), &mut out, &mut remaining);
    out
}

/// How often each value appeared in a given array-element field.
///
/// Keyed by `"/entries[]/type"`: the array's pointer, a `[]` marking the step
/// through it, then the element field. Positions never appear, so nothing here
/// depends on element order or on two recordings having lists of equal length.
#[derive(Debug, Default)]
struct ElementFields {
    /// How many elements each array contributed.
    elements: FxHashMap<String, usize>,
    /// Distinct values seen per element field, and how many elements carried it.
    values: FxHashMap<String, (FxHashSet<String>, usize)>,
}

impl ElementFields {
    /// Element fields that took exactly one value and appeared in every element.
    ///
    /// A field missing from some elements is optional, not constant, and a field
    /// with two values is a discriminator the template is free to vary.
    fn constants(&self) -> FxHashMap<String, String> {
        self.values
            .iter()
            .filter_map(|(key, (seen, occurrences))| {
                let array = key.split("[]/").next()?;
                let elements = self.elements.get(array)?;
                let only =
                    (seen.len() == 1 && occurrences == elements).then(|| seen.iter().next())??;
                Some((key.clone(), only.clone()))
            })
            .collect()
    }

    /// Merge another recording's observations in.
    fn absorb(&mut self, other: &Self) {
        for (array, count) in &other.elements {
            *self.elements.entry(array.clone()).or_insert(0) += count;
        }
        for (key, (seen, occurrences)) in &other.values {
            let entry = self
                .values
                .entry(key.clone())
                .or_insert_with(|| (FxHashSet::default(), 0));
            entry.0.extend(seen.iter().cloned());
            entry.1 += occurrences;
        }
    }
}

/// Record what every array element in `value` said about itself.
fn collect_element_fields(value: &JsonValue) -> ElementFields {
    let mut out = ElementFields::default();
    walk_elements(value, &mut String::new(), &mut out);
    out
}

fn walk_elements(value: &JsonValue, pointer: &mut String, out: &mut ElementFields) {
    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(key);
                walk_elements(child, pointer, out);
                pointer.truncate(mark);
            }
        }
        JsonValue::Array(items) => {
            let array = pointer.clone();
            for item in items {
                let Some(fields) = item.as_object() else {
                    continue;
                };
                *out.elements.entry(array.clone()).or_insert(0) += 1;
                for (key, field) in fields {
                    // One step into the element: discriminators live at the top
                    // of an entry, and deeper paths reintroduce the ambiguity
                    // this whole scheme exists to avoid.
                    let entry = out
                        .values
                        .entry(format!("{array}[]/{key}"))
                        .or_insert_with(|| (FxHashSet::default(), 0));
                    entry.0.insert(field.to_string());
                    entry.1 += 1;
                }
            }
        }
        _ => {}
    }
}

/// Every element field in `value` that disagrees with an established constant.
fn element_constant_drift(value: &JsonValue, constants: &FxHashMap<String, String>) -> Vec<String> {
    let observed = collect_element_fields(value);
    let mut drift = Vec::new();

    for (key, expected) in constants {
        let Some((seen, occurrences)) = observed.values.get(key) else {
            continue;
        };
        let Some(elements) = key
            .split("[]/")
            .next()
            .and_then(|array| observed.elements.get(array))
        else {
            continue;
        };

        if occurrences != elements {
            drift.push(format!(
                "{key}: every recorded element carried this field, {} of {elements} replayed ones do not",
                elements - occurrences
            ));
        } else if seen.len() != 1 || seen.iter().next() != Some(expected) {
            let mut got: Vec<&str> = seen.iter().map(String::as_str).collect();
            got.sort_unstable();
            drift.push(format!(
                "{key}: recorded {expected} in every element but replayed {}",
                got.join(", ")
            ));
        }
    }

    drift
}

fn collect_leaves(
    value: &JsonValue,
    pointer: &mut String,
    out: &mut FxHashMap<String, JsonValue>,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }

    let mut leaf = |value: &JsonValue| {
        let key = if pointer.is_empty() { "/" } else { &*pointer };
        out.insert(key.to_string(), value.clone());
        *remaining -= 1;
    };

    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                let mark = pointer.len();
                pointer.push('/');
                pointer.push_str(key);
                collect_leaves(child, pointer, out, remaining);
                pointer.truncate(mark);
            }
        }
        // See the doc comment: whole, never by position.
        array @ JsonValue::Array(_) => leaf(array),
        scalar => leaf(scalar),
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]
mod tests;
