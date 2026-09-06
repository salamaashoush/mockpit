# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Ferrimock is a high-performance HTTP mocking engine for Node.js, powered by Rust via NAPI. It provides an MSW-compatible API that is 1.1-1.7x faster than MSW on the interception path (see Benchmarking), plus declarative YAML/JSON mocks with Tera template rendering and 115+ fake data generators.

## Workspace Structure

Monorepo with Cargo workspace (3 Rust crates) + bun workspaces (3 JS packages).

### Rust Crates

**ferrimock** (library) -- Core mock engine:
- `types` - Core types: RequestContext, URL patterns, matchers, body sources, HandlerFn
- `config` - Mock configuration parsing (YAML/JSON), HAR file loading.
  `network_error: true` desugars to the marker-header template the server and the
  interceptor already honour — faults add no runtime path
- `engine` - MockRegistry, MockMatcher, validation, scopes, call tracking
- `engine::registry` match counting - every match increments a monotonic per-mock
  counter (always on, one relaxed add). `match_count`/`verify` are the assertion
  surface; `get_call_count` is the *retained* call-tracking buffer and plateaus at
  its window, so never assert on it
- `engine::diagnostics` - `MockMatcher::explain()`: per-criterion match reports and
  ranked near misses. Evaluates through the matcher's own predicates — never
  reimplement matching logic in a renderer (the CLI used to, and drifted)
- `handler` - MSW-style handler builder API (http::get, graphql::query, etc.)
- `template` - Tera template rendering with 115+ fake data functions
- `fake_data` - Fake data generators: names, emails, UUIDs, images, PDFs
- `fake_data::rng` - Seedable random source behind every generator, template
  function and filter. Unseeded it delegates to `rand::rng()`; seeded, draws come
  from a thread-scoped stream (installed per mock id by `template::renderer`) or
  a process-wide stream. Generators must never call `rand::rng()` directly —
  that bypasses `--seed`.
- `core::world` - The entity world: the merged `EntityGraph` (types, keys,
  relations) and the seeded `EntityStore` over it. Owned by `MockRegistry`
  beside `PersistenceStore` — one is untyped scratch state, the other is the
  world a mocked API pretends to have. A spec *populates* it; nothing owns it,
  which is what lets a template, a script and a schema-derived route read and
  write the same entities. Rule of thumb: if it has a type and a key in the API
  you are mocking, it is the world; if it is a counter or a flag for your test,
  it is the store.
- `core::world::store` - Three layers, one mutable: *census* (how many instances
  and their keys, eager and tiny), *base* (field values derived from seed +
  entity + ordinal + field path — pure, never stored), *delta* (creations,
  patches, tombstones). So the world is deterministic given the seed, and the
  state is deterministic given the seed plus the sequence of writes. Two passes
  run over a record after its fields are drawn, and neither makes a field depend
  on another record: lifecycle timestamps are dealt back out in the order their
  names say they happened, and `*_count` fields are answered from the relation
  they name.
- `consolidator` - Smart mock consolidation with pattern detection
- `type_detector` - What a field holds. One `FieldType` vocabulary and one
  name-matching layer (`matches_field_name` and friends, which normalise
  `createdAt`/`created_at`/`CREATED-AT`), shared by every lane including
  `ferrimock-ml`. The *rules* are not shared, and deliberately:
  `detect_from_semantic_context` confirms a guess about a name against the
  values a recording actually carried, while `spec::infer::semantics` has no
  values and instead has a declared type and a `format`. Neither is a subset of
  the other, and where both answer they must agree — pinned by a test in
  `spec::infer::semantics`. The one declared exception is a bare `name`: the
  spec lane knows the owning entity, so `Folder.name` is a folder's name there
  and a person's in the recording lane. A third, name-only rule set used to sit
  beside these for the old GraphQL mock generator; both are gone.
- `graphql` - Reading a GraphQL schema: introspection over the wire, the
  response parsed into a `ParsedSchema`, and SDL written back out. Only reading —
  what a schema *means* is `spec::infer::graphql` and what it *serves* is
  `spec::bind::graphql`.
- `server` - HTTP server utilities: hot reload, graceful shutdown
- `api` - Mock management HTTP API (axum router)
- `proxy` - Reverse proxy: mocks first, upstream for everything else (feature
  `proxy`). An axum router on an axum server. See "The proxy" below.
- `recorder` - HTTP request/response recording
- `scripting` - JS-scripted mock handlers on embedded QuickJS (feature `scripting`)
- `spec` - Reading a schema into the world and binding it to a protocol
  (feature `spec`). `infer` (SDL -> entity graph), `bind` (graph + store ->
  executable GraphQL schema), `emit` (backend -> ordinary `MockDefinition`).
  Deliberately holds no state: the world lives in `core`.

**ferrimock-napi** (cdylib) -- Node.js NAPI bindings:
- `http_ns.rs` - `http.get/post/put/delete/patch/head/options/all` with RegExp, absolute URLs, `{ once }`
- `graphql_ns.rs` - `graphql.query/mutation/operation` (string or RegExp names, endpoint scoping)
- `response_ns.rs` - `HttpResponse.json/text/html/xml/arrayBuffer/redirect/error` builders
- `handler_bridge.rs` - HandlerFn (TSFN for server) + FunctionRef (direct call for interceptor)
- `request_context.rs` - RequestInfo / GraphQLRequestInfo resolver info (MSW shapes; `request` is a real Fetch Request)
- `server.rs` - FerrimockServer with FunctionRef-optimized matchRequest (fall-through/exclude support), use/resetHandlers/resetRuntimeHandlers/listHandlers
- `fake_ns.rs` - 115+ fake data generators exposed to JS
- `world_ns.rs` - `world.types/count/get/list/related/create/update/replace/delete/reset/pendingWrites`
  over the engine's entity world, mirroring the QuickJS `world.*` surface so a
  handler behaves the same on either runtime. Synchronous (a DashMap read behind
  an Arc). The addon enables the `spec` feature; without it the loader would
  ignore every `.graphql` and the world would always be empty under Node.

**ferrimock-cli** (binary and library) -- CLI for mock management and fake data
generation. `ferrimock_cli::ops` is the embedding surface: every `mock` and
`fake` operation as a function over a plain option struct (`CreateMock`,
`TestMockParams`, `ops::fake::Image`, and so on), with no clap in sight. A
host that ships these commands under its own name defines its own clap types,
flags, and help, and converts into those structs; the clap types in `commands`
are ferrimock's own binary and one caller among others.
Changing a flag there does not reach embedders; changing a field in `ops` does,
so a field added to an `ops` struct is a minor-version change and a removed one
is major.

### JavaScript Packages

**ferrimock** -- Main user-facing package:
- `node.ts` - setupServer (the MSW drop-in entry point, exported as `ferrimock/node`)
- `interceptor.ts` - FerrimockInterceptor (patches fetch/XHR/ClientRequest), fall-through loop, lifecycle events, boundary, onUnhandledRequest
- `http-response.ts` - HttpResponse class extending the native Response
- `registration.ts` - http/graphql factories (Response normalization, generators, graphql.link, collection window)
- `msw-compat.ts` - delay(), passthrough(), bypass() utilities
- `events.ts` - LifecycleEvents emitter (request:start/match/unhandled/end, response:mocked/bypass, unhandledException)
- `config.ts` / `loader.ts` - Config loading

**ferrimock** (npm) -- bare-specifier alias re-exporting ferrimock, so mock files
`import { http } from 'ferrimock'` in both Node and the embedded QuickJS runtime.
`world` is exported here too; note `crates/ferrimock-napi/index.mjs` is a
hand-maintained ESM shim listing each named export, while `index.js` and
`index.d.ts` are generated by the napi CLI and must never be hand-edited.
The only CLI is the Rust binary (ferrimock-cli).

**@ferrimock/playwright** -- Playwright fixture adapter.

## Essential Commands

```bash
# Rust
cargo check --workspace                          # Fast compile check
cargo test -p ferrimock --features proxy --test proxy_tests  # Proxy end-to-end
cargo test -p ferrimock --lib                       # Run Rust unit tests
cargo test --workspace --all-features               # Everything
cargo check -p ferrimock-napi                       # Check NAPI bindings

# Build native module
cd crates/ferrimock-napi && bunx @napi-rs/cli build --platform --release

# JavaScript tests
bun test ./packages/core/test/                    # All JS tests
bun test ./crates/ferrimock-napi/test/world.test.ts  # Entity world from Node
bun test ./packages/core/test/msw-compat.test.ts  # MSW compatibility tests
bun test ./packages/core/test/interceptor.test.ts # Interceptor + benchmarks
bun test ./crates/ferrimock-napi/test/              # NAPI binding tests
```

## Architecture

### NAPI FunctionRef Optimization

The key performance optimization: `matchRequest()` uses `FunctionRef` to call JS handlers directly from the deferred resolver callback (~1us) instead of ThreadsafeFunction (~22us UV loop wakeup).

Flow:
1. `matchRequest()` called from JS
2. `spawn_future_with_callback` runs Rust matching on tokio
3. Deferred resolver runs on JS thread:
   - Declarative mock: response already built in Rust
   - Handler mock: `FunctionRef::borrow_back()` + `Function::call()` (~1us direct napi_call_function)
   - Async handlers: detected via `napi_is_promise`, chained with `PromiseRaw::then()`
4. Result: JS handler calls are 1.1-1.7x faster than MSW, depending on runtime and scenario

Key files:
- `handler_bridge.rs` - TSFN (server mode) + FunctionRef (interceptor mode)
- `server.rs` - `match_request` with `MaybePromise` return type for sync/async handler support

### Mock Request Flow

1. Request arrives -> `MockMatcher::find_match()`
2. URL pattern matching (Express `:id`, Glob, Regex, Exact) by priority
3. Header/query/body/GraphQL matching evaluation
4. Once handlers auto-disable after first match
5. Response generation: inline, template (Tera), file, or handler (JS function)

### QuickJS Scripting (feature `scripting`)

`.js`/`.mjs`/`.ts`/`.mts` mock files run on embedded QuickJS (rquickjs 0.12,
`parallel` feature) — no Node needed. Architecture:

- rolldown bundler front-end (`scripting/bundle.rs`): TS transpile, node_modules +
  relative import resolution, tree-shaking, single ESM output; only the `ferrimock`
  specifier stays external (re-links against the runtime ModuleDef). Source maps
  remap error positions back to original files (`remap_error`).
- Bytecode disk cache (`scripting/bytecode_cache.rs`): `Module::write` output cached
  under an ABI-tagged dir (QuickJS version, crate version, arch, endianness, pointer
  width), validated by content hashes of every transitive input from the source map.
  `FERRIMOCK_CACHE_DIR` overrides location; `FERRIMOCK_NO_BYTECODE_CACHE` disables.
- GOTCHA: rolldown_common force-enables `serde_json/arbitrary_precision`
  workspace-wide, which breaks serde untagged-enum buffering on floats. HAR parsing
  goes through `config::parse_har` (AP-safe); never `serde_json::from_str::<Har>`.

- One `ScriptEngine` per script file (`scripting/host.rs`). Hot reload / poison
  recovery = drop the file's engine, re-evaluate on a fresh one. Module-scope state
  resets on reload.
- Single-owner VM event loop (`scripting/vm.rs`): exactly one never-completing tokio
  task polls the runtime scheduler; everything else submits jobs via `VmHandle`.
  Never use transient `async_with!` against the runtime — rquickjs's scheduler has a
  single waker slot and a short-lived poller kills it.
- `http.get(path, fn)` at evaluation time persists the handler into VM-side slots
  (`scripting/slots.rs`) and the loader builds normal `MockDefinition`s with
  `BodySource::Handler` — matching never crosses into JS.
- Two-layer timeout (`scripting/bridge.rs`): QuickJS interrupt handler kills runaway
  bytecode at `handler_timeout` (poisons the engine); a tokio backstop (+1s grace)
  frees requests parked on host awaits.
- `fake.*` dispatches through the same Tera function registry templates use
  (`scripting/bindings/fake.rs`) — one source of truth, embedder plugin functions
  (`register_template_function`) work from JS automatically.
- Tests: `tests/scripting_tests.rs`. Bench: `benches/script_performance.rs`
  (~10us per scripted handler call).

### The proxy (feature `proxy`)

A reverse proxy in front of a dev server or a backend: a request that matches a
mock is answered locally, everything else is forwarded. One origin covers both,
which is what removes the CORS configuration and the application change.

**Nothing on the forwarding path is collected.** That is the whole design, and
every part of the module exists to keep it true:

- `MockRegistry::needs_request_body()` decides whether a request body is read at
  all. It is a registry-wide fact, so a setup with no body-matching mock never
  touches a body and a 2GB upload costs one frame. A body that turns out to be
  over the cap becomes `PendingBody::Chained`: frames already read cannot be
  pushed back, so they are re-emitted ahead of the rest of the stream rather
  than dropped.
- The request body stays `axum::body::Body` end to end. Forwarding an upstream
  `Incoming` through `Body::new` is a move, not a copy, so the proxy never
  needs a body type of its own.
- Recording tees rather than collects (`proxy::tee::TeeBody`, the one body
  combinator axum does not provide). Collecting first would mean a recorded
  event stream never delivers its first event. **It commits from `PinnedDrop`
  as well as from the terminal `poll_frame`**: a known-length body
  is finished once that many bytes are written and hyper stops polling there, so
  the `None` that would have committed never arrives. `is_end_stream()` is what
  separates that from a client that disconnected half way, whose partial capture
  must not be recorded as a complete response.

**Only two things force a collect**, and both are opt-in: a `patch:` mock (a
JSONPath cannot be applied to a stream) and recording. Both also force
`Accept-Encoding: identity` upstream, because a patch operates on JSON and the
recorder stores text. Everything else forwards the browser's own
`Accept-Encoding` and never looks at the body, so compressed bytes pass through
uncosted.

**WebSockets connect upstream before the client gets its 101.** The subprotocol
is chosen by the upstream and has to be echoed in the response; answering first
and connecting after would mean guessing, and a client handed a subprotocol
nobody selected closes immediately. axum writes the 101 and owns the client
half; `to_axum`/`to_tungstenite` map the two frame enums, and the ping and pong
payloads have to survive because several clients echo them back and check. This is the path a dev server's HMR channel
takes, so breaking it looks like "my edits stopped showing up" rather than like
a proxy bug.

Route resolution is a linear scan over a `compile()`-sorted table, which beats
any map at the handful of routes a dev setup has. A prefix matches on **whole
path segments**: without that, `/apifoo` goes to the `/api` route.

Upstream tuning is in `proxy::UpstreamConfig`, measured rather than guessed:
`benches/proxy_overhead.rs` runs every arm against the same upstream through
the same client, so the direct baseline is right beside the proxy number.
Quoting a proxy figure without it is quoting loopback TCP. `pool_timer` is not
a tuning knob but a correctness fix: without it `pool_idle_timeout` never
fires, and the pool grows to its cap per upstream and stays there for the life
of the process.

Tests are `tests/proxy_tests.rs`, driving a real client over a real socket to a
real upstream (`tests/proxy/upstream.rs`). The properties that matter are not
observable any other way: the SSE test asserts the first event arrives inside
200ms against an upstream that spaces three events 120ms apart, which a proxy
that collected the body could not do. That test is what caught the missing
`PinnedDrop` commit.

### MSW API Compatibility

Implemented (MSW and web-standard naming only; no aliases):
- `setupServer(...handlers)` from `ferrimock/node`: listen/close/use/resetHandlers(...next)/restoreHandlers/listHandlers/boundary/events
- `http.get/post/put/delete/patch/head/options/all` with string, RegExp, and absolute-URL predicates; `{ once: true }`
- `graphql.query/mutation/operation` (string or RegExp operation names) + `graphql.link(url)`
- `HttpResponse` (extends Response in Node; native class in QuickJS): json/text/html/xml/arrayBuffer/formData/redirect/error + constructor
- Resolver info: `{ request, params, cookies, requestId }`; GraphQL: `{ query, variables, operationName, cookies, request, requestId }`
- `undefined` return = fall-through to the next handler; generator resolvers
- `delay()`, `passthrough()`, `bypass()`
- Lifecycle events incl. `unhandledException`; `onUnhandledRequest` strategies
- `ReadableStream` response bodies: the interceptor delivers the handler's
  original Response (live stream, zero copies) via the stream stash; the
  standalone TCP server and the QuickJS lane deliver drained (buffered) bodies
- `request.formData()` + `HttpResponse.formData()`: native on Node (real
  Request/Response); native `FormData`/`File` classes + multipart/urlencoded
  codecs on the QuickJS lane

Not covered (by design): `setupWorker` (browser service worker; the engine is a
native addon).

### Spec-derived backends and the entity world (feature `spec`)

A schema is a type system, not a list of observations, so it does not compile to
independent mocks. It compiles into the engine's **world** — entity types, keys,
relations — which a seeded store answers queries against and which protocol
bindings serve.

Two front ends compile into one world: `spec::infer::graphql` reads SDL and
`spec::infer::openapi` reads an OpenAPI 3.0/3.1 document. Both produce an
`EntityGraph`, so a `User` declared by a schema and a `User` described by a
document are one `User` with one set of instances. The module split is the same
on both sides: *read a spec* (`infer`), *bind it to a protocol* (`bind`), *mount
it as ordinary mocks* (`emit`). `spec::bind::plan::RootPlan` is shared — the six
rungs (get/list/create/update/delete/unclassified) are what both classifiers
land on, and what coverage counts.

The split that keeps this one system rather than two:

- **A schema declares entities, never routes.** It has nowhere to write down
  that it is served at `https://api.example.com/graphql` rather than on localhost,
  and guessing is how a proxy answers on the wrong host. Loading a `.graphql`
  populates the world and registers *zero* mocks; the loader warns when a world
  has entities but nothing serves them. An OpenAPI document *does* carry paths,
  but still not a host — `servers:` is reported, never mounted from.
- **A route is a mock.** `serve:` is a mode alongside `response:`, `patch:`,
  `sse:` and `ws:` — not a response body, but a protocol behavior bound to a
  matched URL, exactly like `ws:`. `match` says where the API answers, `serve`
  says which schema answers there.
- **The world is not the spec's.** It lives in `core`, is owned by
  `MockRegistry`, and is reachable from templates (`entity_*`), scripts
  (`world.*` on both the QuickJS and Node lanes) and HTTP. A JS handler that
  creates a user is answering the same question the schema's `users` query
  answers.

```yaml
world:
  schemas:
    - schemas/filestore.graphql
    - schemas/filestore-content.openapi.yaml   # merges into the SAME entity graph
  seed: 42
  counts: { User: 25 }

mocks:
  - id: filestore-graphql
    match:
      POST: https://api.example.com/graphql
    serve: graphql

  - id: filestore-rest
    match:
      url: https://api.example.com/2.0       # base; operations supply path + method
    serve: rest

  # An override is an ordinary mock winning on ordinary priority.
  - id: quota-exceeded
    match:
      POST: https://api.example.com/graphql
      graphql: { mutation: CreateFolder }
    response:
      json: { errors: [{ message: Storage quota exceeded }] }
```

Which file is read as what is decided by extension, never by contents — a file
that has to be opened before anyone can say what it is fails differently every
time its contents change. A bare `.yaml`/`.json` is a mock collection; an
OpenAPI document auto-loaded from a mocks directory is named `*.openapi.yaml`
(or `.yml`/`.json`); anything named under `world.schemas` loads whatever it is
called, as GraphQL for `.graphql`/`.gql` and as OpenAPI otherwise.

Schema-derived routes sit at `config::serve::SERVED_PRIORITY` (50), below the
default 100, so a hand-written mock outranks the backend without anyone doing
arithmetic. A `serve:` mock whose config spells out the default priority is read
as not having chosen.

GraphQL mounts as **one** mock matching any operation. Matching by operation
name would be finer grained, but the name is chosen by the client, not the
schema — a schema-derived backend cannot know it in advance, and pretending
otherwise would leave real requests unmatched. Consequence: `verify()` on a
GraphQL mount asserts the endpoint's total, not per-operation; assert on the
override mock for that.

OpenAPI is the opposite and expands to **one mock per operation**, id
`{mount-id}#{operationId}` (falling back to `{method}-{path}` when the document
names none). The mount supplies the base path and Host; the document supplies
method and path, which go through the ordinary `config::parse_url_pattern`, so
`{param}` becomes a named capture like any hand-written route. A `match.method`
on a `serve: rest` mock is a validation error — operations carry their own.
Accepted cost: a 500-operation document becomes 500 `MockDefinition`s. What it
buys is what a single glob mock cannot give — coverage that names the endpoints,
`verify("filestore-rest#getFolder", Exactly(1))`, and an override that is an ordinary
higher-priority mock at that path rather than a special case.

### Inferring an entity graph from an OpenAPI document

A GraphQL schema states which types have identity; a document does not, so
identity is read off the shape of the surface. Every fact carries the `Rule` that
produced it and a `Confidence`, and `ferrimock world explain` prints both —
inference that cannot explain itself is not usable on a real document.

`CollectionItemPair` (a collection path beside an item path) decides which
schemas are entities and which path parameter addresses one; `SchemaRef`,
`PathNesting`, `SpecLink`, `ForeignKeyName` and `VendorExtension` decide the
relations between them. The `Carrier` says how a link rides on the wire, and the
choice is load-bearing: a `ForeignKey(field)` relation *is* the scalar field, not
a sibling of it, because the store already writes a to-one link's value as the
target's key — so `folder.user_id` holds a key that resolves rather than a
plausible-looking UUID that does not.

The carrier may name a *different* field than the one holding the link, which is
what a document declaring both `user_id` and a `$ref`'d `customer` compiles to:
one link, written twice. Left as two relations they derive independently and the
key names a different user than the object beside it. `Carrier::key_field` is
the one place that answers "which field holds this link's key".

A path addressing one instance by several parameters that each name a field of
the schema — `/repos/{owner}/{repo}` — produces a `CompositeKey` over all of
them. A key of one part keeps the derivation it has always had; the parts of a
composite key are each derived from their own field, or every part of the key
reads as the same value.

Name matching matches names, never meanings: `owner_id` finds an entity called
`Owner`, and nothing teaches the engine that an owner is a `User`. Domain
knowledge belongs in a `ConsolidationProfile` (`spec_relation` for `x-`
extensions, `pagination_dialect` for what this API calls a limit), never in the
engine.

The document is read off a `serde_json::Value` rather than deserialized into
typed structs. Typed OpenAPI crates model 3.0 and 3.1 as separate type systems
and shape their `Either` fields as untagged enums — and untagged buffering under
`arbitrary_precision` turns a number into a private one-key map (see below).
Walking a `Value` meets neither problem, and the 3.0/3.1 divergences (`nullable`,
`exclusiveMinimum`, `type: [x, "null"]`) are few enough to name in one reader.

Resolution is explicit: `serve: graphql` binds the single GraphQL schema in the
world, and refuses with both paths named when there is more than one. Say which
with `serve: { protocol: graphql, schema: <path> }`. Two mounts of the same
schema serve identical data — that is what sharing the world means, and
"same schema, two independent datasets" is deliberately not offered.

Adding a schema **rebuilds** the store and replays every write onto it, so
loading a second schema does not discard state a handler already wrote. Entity
names and ordinals that already existed keep their exact values, because the
base layer derives from the seed. A patch whose record no longer exists (an
entity's count shrank) is reported as a `DeltaConflict` rather than dropped.
A rebuilt census steps over any key a created record already owns: growing a
count would otherwise re-derive a key that is live, and serve two records under
it. Skipping decouples an instance's ordinal from its position, which is why
everything pairing two instances compares *keys* rather than ordinals.

Each source's declaration is kept apart and the merged graph is recomposed from
all of them, so a reload replaces that source's contribution — a field removed
from a schema is removed from the world — while two schemas describing one
entity union their fields rather than one silently replacing the other.
Rebuilds are serialized; a write landing between the snapshot and the swap is
lost, which is a startup and hot-reload window, not a request-path one.

Who owns whom is a `Partition`: each parent draws a weight, the child positions
are cut in proportion, and both directions read the same boundaries. Hashing each
child independently spread them evenly, which no real dataset is. Three things
fall out of the range being the answer — the distribution is lopsided, reading
one parent's children costs that parent's children rather than a filter over
every child, and a `*_count` field is arithmetic rather than a scan. Partitions
depend only on the seed and the two census sizes, all fixed for a store's life,
so they are built on first use and never invalidated.

An entity that owns *itself* cannot use that partition. Cutting a census
against itself has a fixed point for every seed and every count — the owning
map is monotone over a rising boundary vector, so `owner_of(i) - i` has to
cross zero — and a third of a twelve-record hierarchy came out as its own
parent. Self-relations are levelled instead: positions are cut into contiguous
levels, each level is partitioned across the one above it, and level zero has
nothing above it, which is where the world's roots come from. A parent is
always at a lower level than its child, so a cycle of any length is impossible
rather than merely unlikely, and the range property survives — one parent still
owns one contiguous run, so reading its children is a range read and counting
them is still arithmetic. A `parent` the spec marked non-nullable is therefore
unsatisfiable, and `world explain` says so.

Because a hierarchy has generations, a delete cascades to a *fixpoint* rather
than one level. Stopping at the first generation leaves everything below it
pointing at a tombstone, which is the dangling key this store exists to make
impossible.

Ownership is contiguous in *partition position*; where an instance sits in the
*census* is a seeded shuffle of that. Without the separation the partition was
visible in a single response: the number of runs of the parent key down an
unsorted page equalled the number of distinct parents on it, exactly, with no
variance at any size -- an identity rather than a statistic. Levels would have
made it louder still, since every root would have come first. The shuffle is a
Fisher-Yates table and its inverse, two `Vec<u32>` beside a census that already
holds a `Vec<EntityKey>` and a slot map, so it costs one array index per child
read. Nothing outside `Ownership` is handed a range: the two spaces meeting in
a caller is exactly the bug -- a `*_count` drifting from the list endpoint by
one per write -- that keeping them apart prevents.

`Slot` is why the partition works after a census had to step over a reserved
key: `ordinal` is what a record's values derive from, `index` is where it sits
among its siblings, and everything pairing two instances works in `index`.

A `serve:` mount can ask for behaviour a document does not describe --
`serve: { protocol: rest, behaviour: { conditional: true, soft_delete: true,
problem_json: true, replica_lag: 2, idempotency: true } }`. A representation
carries an `ETag`, `If-None-Match` answers 304 and `If-Match` answers 412
(checked *before* the write, since the tag names the version the client
believes it is changing); a removed record answers 410 rather than 404, because
404 says try a different key and 410 says stop asking; errors come back as
`application/problem+json`; an `Idempotency-Key` replays the answer it already
got; and a lagging replica holds a created record back from lists.

Two constraints. Replica lag is counted in **writes**, not seconds: wall-clock
lag would make `keys()`, `count()` and every page total functions of the clock,
so two identical requests would answer differently and a delta snapshot has
nowhere to keep a timer. And **replay forces all of it off** -- not a default, a
constraint. `consolidator::fidelity` scores status, shape and value equality
against what was recorded, and a 304, a 412 or a held-back record fails all
three against the unconsolidated baseline as well, so the attribution logic
could not tell a consolidator bug from a mock behaving exactly as its mount
asked.

A root field returning one instance with no way to say *which* -- `viewer`,
`me`, `currentUser`, `GET /me` -- is `RootPlan::Viewer`, not a `Get` that lost
its argument. Read as a `Get` it had an empty key, and an empty key resolved to
record zero: the same person for every caller, with or without a token, on the
one endpoint whose entire purpose is to differ per caller. `world.viewer` names
the entity a credential is an instance of, and which instance is *derived* from
the credential -- so one token is one person across restarts and two tokens are
two people, with no session table to keep. No credential is a 401 carrying
`WWW-Authenticate`, because a client library that retries on 401 reads the
scheme out of it. With nothing bound the endpoint is not answerable at all, so
it is counted as unclassified and answered from its declared shape rather than
answered wrongly. Only an operation that actually needs the credential declares
`ContextNeeds` for headers, so nothing else pays to marshal them.

A record is somewhere. Fields inside one were mutually independent, because
every value derived from `(seed, entity, ordinal, path)` and nothing else -- so
a user in Tokyo got a French name, a `+44` phone and an `America/Bogota`
timezone, none of them individually implausible and the combination impossible.
`fake_data::place` is the *discrete* confounder that fixes it: name, phone,
country, currency, timezone, locale and postal code all read from one place per
record, and conditional independence given the place is the real generative
structure rather than an approximation of one. A continuous shared factor is
deliberately not there -- it would trade "every correlation is zero" for "every
correlation is equal", which is rank-one with a flat residual spectrum and its
own signature, and only loadings fitted from a recording remove that.

The place travels *one hop* along the derived path: a folder's files are where
the folder is, and the folder's own place is its own draw rather than its
parent's. A chain would put every record in the world in one country, and
resolving a parent through the delta would reach `get`, which reaches
`base_fields`, which is where the place is read from. Stated consequence: a
client that retargets a relation gets a child whose placed fields still agree
with the parent the seed gave it.

A status field is a position in a lifecycle, not a categorical draw.
`world.states` declares one per `Entity.field`, and what it declares is an
*implication*: `shipped` means `shipped_at` holds a value and `delivered_at`
does not. No correlation reproduces that -- a latent gives a probability where
the schema needs a certainty -- so the fields a state names are cleared after
everything else has run. The order is the lifecycle, which is why it is written
as a sequence rather than a mapping: a YAML mapping does not promise to keep
the order it was written in, and the order is the whole content. Writes move
along it, so a delivered order cannot return to draft and attempting it is the
409 the real service would also answer.

`core::world::store::clock` is the world's history. A creation time is a
monotone function of the ordinal and of nothing else -- in particular not of
how many instances exist, because placing arrival *i* among *N* would make
every timestamp a function of the count, so bumping `world.counts` or mounting
a second schema would silently rewrite the creation time of every record that
already existed. Both ends of the window are anchored without consulting the
count: the first arrival at the start of the entity's own history, the rest
closing on the present, so the newest is recent whether the world holds twelve
records or six hundred. The stated consequence is that arrivals are heavily
recency-weighted, the way a service whose volume grew is, rather than spread
evenly. The seasonal warp -- two humps a working day, a lull at lunch, almost
nothing overnight or at a weekend -- is *monotone*, pushing an instant's
position within its week through a rising cumulative intensity without ever
moving the week: any reshuffle inside a day would break an id that carries a
creation time as soon as two arrivals fall closer together than a day.

Ordinal, key and age rise together, which is what lets an id agree with a
timestamp. An integer key still counts from one, so `GET /orders/1` resolves on
an integer-keyed document. An opaque one is a ULID rather than a v4 uuid: a
uuid is the only id family in use that carries neither a count nor a clock, so
sorting a collection by it put the collection in an order unrelated to anything
that happened. A `format: uuid` is the document's own answer and still wins.

`core::world::store::bus` settles the fields of a record that are *functions*
of other fields of it: `full_name` is `first_name` plus `last_name`, `email`
holds a slug of the name, `slug` is the title slugified, an avatar URL ends in
the record's own id. These are not correlations and no latent vector produces
them at any dimension -- one field simply is a function of another, and a
record where the two disagree is wrong rather than improbable. `order_lifecycle`
was already this shape and is the precedent. The bus runs after the store has
written the key and the links, because a link ending in an id has to end in the
one the record is actually filed under; it is re-runnable, and a field the
caller stated is left alone.

`core::world::store::distribution` is where a value's *shape* lives, separate
from what draws it. Every draw is a pure map over the bytes the field already
derived, so nothing about laziness, replay or determinism changes -- only what
comes out. The defaults are the point: uniform everywhere is the loudest
statistical signature an engine can have, and it is not one a client has to
work to see. A number nothing bounded is log-uniform over orders of magnitude
with a little mass on zero; a number with a *narrow* declared range stays
uniform, because a rating or a percentage is not Benford-ish and a log-normal
truncated below a decade is uniform again anyway. An enum is Zipf over a
permutation keyed on the field, which gives a skewed marginal without claiming
which member is modal -- declaration order does not say: lifecycle enums list
the terminal state last, machine-emitted schemas are often alphabetical, and
protobuf mandates `UNSPECIFIED` first. A boolean gets a chance drawn per field
and pushed away from the middle. A collection length is geometric rather than
always two. Which member is *actually* modal, and what the real rates are, is
something only a recording can say.

`required` and `nullable` are separate facts on a `FieldDef`, because a schema
gives two separate answers: `required` says the key is in the payload,
`nullable` says the value may be null. A GraphQL field that was selected is
always present and may be null; an OpenAPI property left out of `required` may
not be there at all, and answering it with `null` because it happened to be
optional violates the `type: string` that declared it. So an optional field
loses its key and a nullable one keeps it holding null -- each at a rate drawn
per field rather than per record, the way a real column is null a twentieth of
the time or half of it. A filter over an absent field matches nothing but `Ne`,
which is what a real API does and what a test asserting on it has to expect.

How many instances an entity gets is read off its place in the graph rather
than from one constant. The child end of a to-one link is more numerous than
the parent end -- a file store has more files than folders and more folders
than users -- so the count fans out with depth and stops at a cap, because the
census is eager and a five-deep document would otherwise ask for ten thousand
leaves. `world.count` still sets a flat default for everything, `world.counts`
still names one entity, and `world.scale` multiplies whatever the default
resolved to, which is how a mount asks for a bigger world without naming every
entity in it. Size is not a cosmetic setting: an entity smaller than one page
hands a client the whole population in a single request, and a five-member enum
needs about forty draws before anything can tell its distribution from uniform,
so a world too small to sample is a world whose statistics cannot be tested.

`ferrimock world fit recordings/*.har --dir mocks/ -o world.fit.yaml` measures a
recording and writes the world that would have produced it. Realism is agreement
with an empirical distribution, so the highest-fidelity world is one whose
parameters were measured rather than guessed: every default in the value layer
is a defensible prior and none of them knows what this API's `status` field
actually holds or how many folders a real account has. What comes out is an
ordinary overrides file -- reviewable, diffable, committable, applied through
the same `FieldRules` a hand-written one is, never a private path back into the
store. The lifecycle inference is the part worth knowing about: a `status` is
read as one when *other fields go empty conditional on it*, and the order comes
out of the same evidence, since a state that empties more of the record is
earlier in the life of one.

`ferrimock world doctor` lints the generated world for the things that give a
mock away, and it is the number any change to the world is judged against. It
runs with no corpus, because the case a mock exists for is the one where no
corpus of real responses exists; each check fails independently and reports the
measurement that failed it, so a change either moves a check or it does not. Two
outcomes are not a pass: a **defect** is a behaviour no real API has, and a
check the world is **too small to measure** is reported as its own outcome
rather than silently as a clean bill.

The checks that measure a *distribution* read a census of their own rather than
the one the mount serves — `EntityStore::resized`, sized to `SAMPLE_CAP` — since
nothing about a lint requires the served counts and forty records cannot tell a
boolean from a fair coin. A count someone stated by name is left where they put
it, and the checks that are about the world *as served* (its page size, its
partitions, its counts) keep reading the store they were handed.

Every sample floor is derived from the **flattest** thing the generator can
draw, not its typical one: `LEAST_SKEW` for an enum's ranking and
`FLATTEST_FLAG` for a boolean's chance. A floor set at the middle of a
generator's range is by construction too small for half of what it draws, and
reports the half it cannot see as flat. Failing to reject uniformity is never
evidence of it.

`World::reset()` drops every write and leaves exactly what the seed derives —
call it between tests, or state leaks from one into the next.
`World::pending_writes()` is how you see that it did.

`MockRegistry::with_world()` gives a registry its own world for isolation, which
integration tests need because the process-global one is shared. `entity_*`
template functions read the *global* world (Tera's function registry is
stateless, so there is nowhere to thread a handle through — the same constraint
`PersistenceStore` already lives with), so `with_world` publishes its world there
when nothing has claimed it yet. The first registry in a process therefore gets
templates that read exactly what its routes serve; a second cannot displace it,
and keeps its own world for matching while templates go on reading the first.

### JSON into a JS runtime

`serde_json/arbitrary_precision` is force-enabled workspace-wide by
`rolldown_common`. Under it `serde_json::Value::Number`'s `Serialize` emits a
private one-key map that only serde_json's own deserializer intercepts, so any
*other* serializer produces `{"$serde_json::private::Number": "3"}` where a
number was meant.

- **QuickJS**: never `rquickjs_serde::to_value` for a `serde_json::Value`. Use
  `scripting::bindings::convert::json_to_js`, which walks the value into native
  values — also faster, since it skips the serde data model entirely. It defines
  own properties via `JS_DefineProperty` rather than `Object::set`, so a
  `__proto__` key in an entity lands as a field instead of firing the prototype
  setter.
- **Reading JS into Rust** stays on `rquickjs_serde::from_value`: the token is
  only ever produced by `Serialize`, so the inbound direction never meets it.
- **NAPI** needs no workaround — `napi`'s `ToNapiValue for &Value` matches the
  enum directly and `Number` goes through `is_i64`/`as_i64`/`as_f64`.

### Absolute URLs in a match

A bare absolute `match.url` splits into a path pattern plus a `Host` matcher,
the same way `http.get("https://api.example.com/x")` does — a server sees `GET /x`
with `Host: api.example.com`, never the whole URL, so keeping it as one string would
never match anything behind a proxy. An `exact:`-prefixed URL is left whole:
that is what the HAR loader and the consolidator emit, and they mean the request
line verbatim.

## Benchmarking

`benches/world_performance.rs` (feature `spec`) covers the entity world: seeding,
`count`, `get`, paged and filtered lists, writes, and rebuilds, plus mounting and
answering an OpenAPI document (`rest/*`). It is what caught an unfiltered
`limit: 25` costing 21ms on a 10,000-instance entity by materialising everything
before paginating; such a page is now answered from the census, deriving only the
window. Filtered or sorted lists still scan, which is inherent.

That scan is on the REST request path, because a query parameter naming a field
becomes a predicate: `rest/answer/list_filtered_25` costs ~23ms on a
10,000-instance entity against ~1.5us for a lookup and ~730us for an unfiltered
page. Reads by key and unfiltered pages are flat in the world's size; a filtered
list is linear in it. Worth knowing before pointing a load test at one.

Never measure ferrimock and another interceptor in the same process. Whichever
loads second is penalised — MSW measures 28.9us alone and 232.5us when it follows
ferrimock, an 8x swing decided by ordering alone. Cross-library numbers come from
`benchmarks/fair.mjs`, which runs each library in its own process and alternates
which goes first. `packages/core/test/comparison.test.ts` measures ferrimock only,
for the same reason.

Warm both arms identically, and never quote a server-mode figure (real TCP)
against an interceptor's (in-process). The README's original "3-4x faster than
MSW" came from breaking both rules at once.

## Code Standards

- Idiomatic Rust with zero-cost abstractions
- `anyhow::Result` for application code
- `unsafe` denied in ferrimock-napi (except marked `#[allow(unsafe_code)]` for NAPI FFI)
- FxHashMap for performance-critical paths (not std HashMap)
- All new code must include tests
- Run `cargo test -p ferrimock --lib` and `bun test` before committing

## Consolidation

Consolidation compresses a recording into patterns and templates. It is lossy,
so a reduction ratio on its own says nothing -- collapsing every mock into one
would score 99%. Every change to the consolidator has to be judged against
replay fidelity, not size.

### Fidelity

`consolidator::fidelity` replays each recorded request through the consolidated
collection and diffs the answer against what was really recorded, at levels that
fail independently: matched, no cross-talk, status exact, shape equal, constants
held, value equal. It scores the *unconsolidated* collection the same way, so a
failure is attributable -- a recording the recorder cannot replay is not the
consolidator's fault, and the delta between the two is what consolidation cost.

```bash
ferrimock mock consolidate in.json out.json --verify traffic.har --fail-under 0.95
```

`--verify` takes a recording session or a HAR -- the formats that keep requests
alongside responses. A consolidated mock collection cannot be verified against
itself: it no longer records what was asked.

### Domain knowledge

The engine ships defensible defaults and no API-specific rules. Anything that
depends on knowing a particular API -- that `/v2/` is a version rather than an
id, that `continuation` is a cursor, which hosts serve file content -- goes in a
`profile::ConsolidationProfile` supplied by the embedder. Do not add such rules
to the engine; add the hook the profile needs.

### Tests

- `tests/consolidator_fidelity.rs` -- scenarios someone thought of, each
  asserting both the reduction and the fidelity it must not cost.
- `tests/consolidator_props.rs` -- proptest over a generated synthetic API.
  Invariants are behavioural: grouping and templating are the engine's business,
  answering every recorded request correctly is not.
- `fuzz/` -- cargo-fuzz targets for crash safety and the invariants that hold
  over arbitrary input. Needs nightly:

```bash
cargo +nightly install cargo-fuzz
scripts/fuzz.sh          # every target, 60s each
scripts/fuzz.sh 0 consolidate   # one target, until stopped
```
