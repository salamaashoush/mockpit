//! Ferrimock CLI commands: mock management and fake data generation.

pub(crate) mod consolidate;
pub(crate) mod convert;
pub(crate) mod create;
mod dispatch;
pub(crate) mod export;
pub mod fake;
pub(crate) mod format;
pub(crate) mod list;
pub mod proxy;
pub(crate) mod recordings;
pub(crate) mod reload;
pub(crate) mod serve;
pub(crate) mod show;
pub(crate) mod test;
pub mod ui;
pub(crate) mod validate;
pub(crate) mod wizard;
pub mod world;

// Re-export the mock command entry point
pub use dispatch::execute;
// Re-export the fake command types and entry point
pub use fake::{FakeAction, FakeCommand};

// ---------------------------------------------------------------------------
// CLI argument types
// ---------------------------------------------------------------------------

use clap::{Args, Subcommand};

/// Mock management subcommand
#[derive(Args, Debug, Clone)]
pub struct MockCommand {
    #[command(subcommand)]
    pub action: MockAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum MockAction {
    /// Write a new mock file, from flags or through a wizard
    ///
    /// With a URL, the flags describe the mock and the file is written
    /// straight away. Without one, or with --interactive, a wizard asks for
    /// the URL pattern (Express, regex, or glob, detected from what you
    /// type), the methods, any header, query, or body matchers, and the
    /// response, and shows the result before saving it.
    ///
    /// Examples:
    ///   mock create "/api/users/:id" -m GET -s 200 --template
    ///   mock create "/api/orders" -m POST -s 201 -b @order.json
    ///   mock create
    #[command(visible_alias = "new", verbatim_doc_comment)]
    Create {
        /// URL pattern to match (omit to start interactive wizard)
        #[arg(value_name = "URL")]
        url: Option<String>,

        /// Output file path (defaults to `mocks/collections/MOCK_ID.yaml`)
        #[arg(short = 'o', long, value_name = "FILE")]
        output: Option<String>,

        /// HTTP method (GET, POST, etc.)
        #[arg(short = 'm', long, value_name = "METHOD", default_value = "GET")]
        method: String,

        /// Response status code
        #[arg(short = 's', long, value_name = "CODE", default_value = "200")]
        status: u16,

        /// Response body (JSON string or @file.json)
        #[arg(short = 'b', long, value_name = "BODY")]
        body: Option<String>,

        /// Generate a body template with fake data instead of a fixed body
        #[arg(short = 't', long)]
        template: bool,

        /// Mock ID (auto-generated if not provided)
        #[arg(short = 'i', long, value_name = "ID")]
        id: Option<String>,

        /// Mock priority (higher = matched first)
        #[arg(short = 'p', long, value_name = "PRIORITY", default_value = "100")]
        priority: u32,

        /// Collection name/scope
        #[arg(short = 'c', long, value_name = "NAME")]
        collection: Option<String>,

        /// Mock kind: http (default), ws (WebSocket), or sse (Server-Sent Events)
        #[arg(short = 'k', long, value_name = "KIND", default_value = "http")]
        kind: String,

        /// Launch interactive wizard for step-by-step mock creation
        #[arg(short = 'I', long)]
        interactive: bool,
    },

    /// List the mocks in the collections directory
    #[command(visible_alias = "ls")]
    List {
        /// Filter by collection name
        #[arg(short = 'c', long, value_name = "NAME")]
        collection: Option<String>,

        /// Show detailed information
        #[arg(short = 'v', long)]
        verbose: bool,
    },

    /// Print one mock's definition
    #[command(visible_alias = "s")]
    Show {
        /// Mock ID
        #[arg(value_name = "MOCK_ID")]
        mock_id: String,
    },

    /// Show which mock a request would hit, and why the others would not
    ///
    /// Builds the request from the path, method, headers, and body you give
    /// and runs it through the matcher without a server. --render also
    /// shows the response the mock would send, with its template filled
    /// in; --debug lists every mock with the criterion it failed on.
    ///
    /// Examples:
    ///   mock test -m GET /api/users/123 --render
    ///   mock test -m POST /api/users -H "Content-Type: application/json" --body '{"name": "Ann"}'
    ///   mock test -m GET /api/users/123 --debug
    #[command(visible_alias = "t", verbatim_doc_comment)]
    Test {
        /// HTTP method
        #[arg(short = 'm', long, value_name = "METHOD", default_value = "GET")]
        method: String,

        /// Request path
        #[arg(value_name = "PATH")]
        path: String,

        /// Query string (optional)
        #[arg(short = 'q', long, value_name = "QUERY")]
        query: Option<String>,

        /// Request headers (can be used multiple times, format: "Name: Value")
        #[arg(short = 'H', long = "header", value_name = "HEADER", action = clap::ArgAction::Append)]
        headers: Vec<String>,

        /// Request body (JSON string or @file.json)
        #[arg(short = 'b', long, value_name = "BODY")]
        body: Option<String>,

        /// Render the response with fake data (show actual mock output)
        #[arg(short = 'r', long)]
        render: bool,

        /// Debug mode - show verbose matching information for all mocks
        #[arg(short = 'd', long)]
        debug: bool,

        /// Load mocks from a specific file instead of the collections directory
        #[arg(short = 'f', long = "mock-file", value_name = "FILE")]
        mock_file: Option<String>,

        /// Output in JSON format for programmatic use
        #[arg(short = 'j', long)]
        json: bool,
    },

    /// Load the collections directory and report how many mocks it holds
    #[command(visible_alias = "r")]
    Reload {
        /// Mock collections directory
        #[arg(short = 'd', long, value_name = "DIR")]
        dir: Option<String>,
    },

    /// List the recorded sessions in the recordings directory
    #[command(visible_alias = "rec")]
    Recordings {
        /// Recordings directory
        #[arg(short = 'd', long, value_name = "DIR")]
        dir: Option<String>,
    },

    /// Check that mock files parse, load, and reference only things that exist
    #[command(visible_alias = "v")]
    Validate {
        /// Mock collections directory or specific file path (defaults to mocks/collections)
        #[arg(value_name = "PATH")]
        path: Option<String>,

        /// Output format (text for human-readable, json for machine-readable)
        #[arg(short = 'f', long, value_parser = ["text", "json"], default_value = "text")]
        format: String,

        /// Read from stdin instead of a file (requires --file-format)
        #[arg(long)]
        stdin: bool,

        /// File format for stdin input (json, yaml, yml)
        #[arg(long, value_name = "FORMAT", value_parser = ["json", "yaml", "yml"], requires = "stdin")]
        file_format: Option<String>,
    },

    /// Rewrite mock files in the canonical layout
    ///
    /// Fields come out in a fixed order (id, priority, enabled, scope, match,
    /// response, request) with sorted keys, so two people editing the same
    /// file produce the same diff. --check reports instead of writing and
    /// exits 1 when a file would change.
    ///
    /// Examples:
    ///   mock format mocks/collections/
    ///   mock format --check mocks/
    ///   cat mock.yaml | mock format --stdin --file-format yaml
    #[command(visible_alias = "fmt", verbatim_doc_comment)]
    Format {
        /// Mock collections directory or specific file path (defaults to mocks/collections)
        #[arg(value_name = "PATH")]
        path: Option<String>,

        /// Check formatting without modifying files (exit 1 if any file would change)
        #[arg(long)]
        check: bool,

        /// Read from stdin and write formatted output to stdout (requires --file-format)
        #[arg(long)]
        stdin: bool,

        /// File format for stdin input (json, yaml, yml)
        #[arg(long, value_name = "FORMAT", value_parser = ["json", "yaml", "yml"], requires = "stdin")]
        file_format: Option<String>,
    },

    /// Turn a HAR file into a mock collection
    ///
    /// The result replays cleanly by default: absolute URLs become relative
    /// paths, static assets and other domains are left out, sensitive and
    /// infrastructure headers are stripped, and access_token query
    /// parameters are removed. Each flag below keeps one of those.
    #[command(visible_alias = "conv")]
    Convert {
        /// Input HAR file
        #[arg(value_name = "INPUT")]
        input: String,

        /// Output mock collection file
        #[arg(value_name = "OUTPUT")]
        output: String,

        /// Output format: json, yaml. Defaults to the output file's extension,
        /// then to yaml.
        #[arg(short = 'f', long, value_name = "FORMAT", value_parser = ["json", "yaml"])]
        format: Option<String>,

        // Accepted so existing scripts keep working; conversion has always
        // matched recorded URLs exactly and never read this.
        #[arg(short = 'm', long, value_name = "STRATEGY", value_parser = ["exact", "pattern"], default_value = "pattern", hide = true)]
        matching: String,

        /// Interactive pattern editing
        #[arg(short = 'I', long)]
        interactive: bool,

        /// Include OPTIONS preflight
        #[arg(long)]
        preflight: bool,

        /// Include redirect responses (3xx)
        #[arg(long)]
        redirects: bool,

        /// Keep browser headers
        #[arg(long)]
        browser_headers: bool,

        /// Keep absolute URLs (don't normalize to relative paths)
        #[arg(long)]
        absolute_urls: bool,

        /// Only include entries from these domains (comma-separated, e.g. "api.example.com,cdn.example.com").
        /// Subdomains are included automatically. When not set, all domains are included.
        #[arg(long, value_name = "DOMAINS", value_delimiter = ',')]
        domains: Vec<String>,

        /// Include static assets (.js, .css, .png, etc.)
        #[arg(long)]
        static_assets: bool,

        /// Keep sensitive headers (Authorization, Cookie, Set-Cookie)
        #[arg(long)]
        keep_sensitive_headers: bool,

        /// Keep infrastructure headers (date, server, x-envoy-*, alt-svc)
        #[arg(long)]
        keep_infra_headers: bool,

        /// Extract large/binary response bodies to separate files
        #[arg(long)]
        extract_bodies: bool,

        /// Body size threshold in KB for extraction (default: 100)
        #[arg(long, value_name = "KB", default_value = "100")]
        body_threshold_kb: usize,

        /// Answer every repeat of a request with the first recording of it,
        /// instead of replaying the recorded answers in order
        #[arg(long)]
        flatten_repeats: bool,

        /// Drop each entry's recorded latency instead of keeping it as a delay.
        ///
        /// A recording made against a real service carries its real latency, so
        /// mocks converted from one wait exactly as long as the service did.
        /// That is what you want for a demonstration and never what you want in
        /// a test suite.
        #[arg(long)]
        no_delays: bool,
    },

    /// Write a mock collection out as a HAR file
    #[command(visible_alias = "exp")]
    Export {
        /// Mock collections directory
        #[arg(short = 'd', long, value_name = "DIR")]
        dir: Option<String>,

        /// Output HAR file path
        #[arg(short = 'o', long, value_name = "FILE")]
        output: String,

        /// Filter by collection name
        #[arg(short = 'c', long, value_name = "NAME")]
        collection: Option<String>,
    },

    /// Merge a recording's repeated requests into patterns
    ///
    /// Requests that differ only in an id, a page number, or a value the
    /// type detector can place become one mock with a path pattern and a
    /// template, so a large recording shrinks to a small collection that
    /// still answers every request it recorded. --verify replays the
    /// recording through the result and reports what changed.
    ///
    /// Examples:
    ///   mock consolidate recordings/session.json mocks/api.yaml
    ///   mock consolidate recordings/session.json mocks/api.yaml --verify
    ///   mock consolidate recordings/session.json mocks/api.yaml --no-templates
    #[command(visible_alias = "opt", verbatim_doc_comment)]
    Consolidate {
        /// Input mock collection
        #[arg(value_name = "INPUT")]
        input: String,

        /// Output consolidated mocks
        #[arg(value_name = "OUTPUT")]
        output: String,

        /// Output format: json, yaml
        #[arg(short = 'f', long, value_name = "FORMAT", value_parser = ["json", "yaml"], default_value = "json")]
        format: String,

        /// Min similar requests to form pattern
        #[arg(long, value_name = "N", default_value = "3")]
        min_pattern: usize,

        /// Skip template extraction
        #[arg(long)]
        no_templates: bool,

        /// Template endpoints recorded only once, reading each value for what it
        /// is rather than as a fixed answer
        ///
        /// A lone recording is otherwise reproduced verbatim, so its mock
        /// answers that one request and nothing else. This widens paths whose
        /// segments read as identifiers and generates the fields whose values
        /// the detector can place, at the cost of no longer replaying the
        /// recording exactly.
        #[arg(long)]
        generalize: bool,

        /// Replay a recording through the consolidated mocks and report what changed
        ///
        /// Takes a recording session (JSON/YAML) or a HAR file -- the formats
        /// that keep requests alongside responses. A consolidated mock
        /// collection cannot be used here: it no longer records what was asked.
        #[arg(long, value_name = "RECORDING")]
        verify: Option<String>,

        /// Fail when behavioural fidelity falls below this ratio (implies --verify)
        #[arg(long, value_name = "RATIO", requires = "verify")]
        fail_under: Option<f64>,

        /// Show detailed stats
        #[arg(short = 'v', long)]
        verbose: bool,
    },

    /// Serve the mocks over HTTP, with nothing upstream
    ///
    /// A request that matches a mock gets its response; one that matches
    /// none gets a 404 whose body names the closest mock and the criterion
    /// it failed on. --watch reloads the files when they change, --cors
    /// lets a page on another origin call it, and --log-matches prints
    /// which mock answered each request.
    ///
    /// Examples:
    ///   mock serve --port 3006 --watch
    ///   mock serve --mocks ./mocks/api/ --cors --log-matches
    ///   mock serve -f mocks/api-users.yaml
    #[command(visible_alias = "sv", verbatim_doc_comment)]
    Serve {
        /// Mock collections directory (same as --mocks)
        #[arg(value_name = "DIR")]
        dir: Option<String>,

        /// Port to listen on
        #[arg(short = 'p', long, default_value = "3006")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Mock collections directory
        #[arg(short = 'm', long, value_name = "DIR")]
        mocks: Option<String>,

        /// Load a specific mock file (can be combined with --mocks)
        #[arg(short = 'f', long = "mock-file", value_name = "FILE")]
        mock_file: Option<String>,

        /// Watch mock files and hot-reload on change
        #[arg(short = 'w', long)]
        watch: bool,

        /// Enable CORS headers for browser access
        #[arg(long)]
        cors: bool,

        /// Enable template rendering endpoint (POST /__mock/render)
        #[arg(long)]
        enable_render_endpoint: bool,

        /// Log mock match details for every request (mock ID, captures, elapsed time)
        #[arg(long)]
        log_matches: bool,

        /// Enable verbose request logging
        #[arg(short = 'v', long)]
        verbose: bool,

        /// Open browser to server URL
        #[arg(short = 'o', long)]
        open: bool,

        /// Omit the near-miss explanation from unmatched (404) responses
        #[arg(long)]
        no_explain: bool,
    },
}
