//! The operations behind the `mock` and `fake` subcommands, for a host that
//! defines its own command line.
//!
//! ferrimock's own binary and any tool that embeds it both call these. The
//! types here are plain data with public fields, so a host builds them from
//! whatever argument parser it uses and keeps its own flag names, help text,
//! and aliases. Nothing in this module depends on clap, and the clap types in
//! [`crate::commands`] are one caller among others.

pub use crate::commands::consolidate::{ConsolidateArgs, consolidate_mocks};
pub use crate::commands::convert::{ConvertHarOptions, convert_har, format_for};
pub use crate::commands::export::export_to_har;
pub use crate::commands::format::format_mocks;
pub use crate::commands::list::list_mocks;
pub use crate::commands::recordings::list_recordings;
pub use crate::commands::reload::reload_mocks;
pub use crate::commands::serve::{MockServerConfig, serve_mock_server};
pub use crate::commands::show::show_mock;
pub use crate::commands::test::{TestMockParams, test_mock_match};
pub use crate::commands::validate::validate_mocks;

/// What `mock create` needs to write one mock.
///
/// Without a `url`, or with `interactive`, the wizard runs and the other
/// fields become its starting answers.
#[derive(Debug, Clone)]
pub struct CreateMock {
    pub url: Option<String>,
    /// Defaults to `mocks/collections/<id>.yaml`.
    pub output: Option<String>,
    pub method: String,
    pub status: u16,
    /// A JSON string, or `@path` to read one from a file.
    pub body: Option<String>,
    /// Generate a body template with fake data instead of a fixed body.
    pub template: bool,
    pub id: Option<String>,
    pub priority: u32,
    pub collection: Option<String>,
    /// `http`, `ws`, or `sse`.
    pub kind: String,
    pub interactive: bool,
}

impl Default for CreateMock {
    fn default() -> Self {
        Self {
            url: None,
            output: None,
            method: "GET".to_string(),
            status: 200,
            body: None,
            template: false,
            id: None,
            priority: 100,
            collection: None,
            kind: "http".to_string(),
            interactive: false,
        }
    }
}

/// Write a mock file from `opts`, through the wizard when it asks for one.
pub fn create_mock(opts: CreateMock) -> anyhow::Result<()> {
    let CreateMock {
        url,
        output,
        method,
        status,
        body,
        template,
        id,
        priority,
        collection,
        kind,
        interactive,
    } = opts;
    match (interactive, url) {
        (true, url) | (false, url @ None) => crate::commands::wizard::run_wizard(
            url, output, &method, status, body, template, id, priority, collection, &kind,
        ),
        (false, Some(url)) => crate::commands::create::create_mock(
            output,
            &method,
            &url,
            status,
            body,
            template,
            id,
            priority,
            collection.as_deref(),
            &kind,
            false,
        ),
    }
}

/// The operations behind the `fake` subcommand.
pub mod fake {
    use crate::commands::fake::{data, image, pdf, preview, server};

    /// What `fake data` needs to print values of one generator.
    #[derive(Debug, Clone)]
    pub struct Data {
        pub generator: String,
        pub count: usize,
        pub min: Option<f64>,
        pub max: Option<f64>,
        /// Word count, for the sentence and paragraph generators.
        pub words: Option<usize>,
        /// Length, for the alphanumeric and token generators.
        pub length: Option<usize>,
        /// `text`, `json`, or `csv`.
        pub format: String,
        /// Also put the result on the clipboard.
        pub copy: bool,
    }

    impl Default for Data {
        fn default() -> Self {
            Self {
                generator: String::new(),
                count: 1,
                min: None,
                max: None,
                words: None,
                length: None,
                format: "text".to_string(),
                copy: false,
            }
        }
    }

    /// Print `count` values of `generator` in `format`.
    pub fn data(opts: &Data) -> anyhow::Result<()> {
        data::generate_fake_data(
            &opts.generator,
            opts.count,
            opts.min,
            opts.max,
            opts.words,
            opts.length,
            &opts.format,
            opts.copy,
        )
    }

    /// Print the generators in `category` (all of them when `None`) in `format`.
    pub fn list_category(category: Option<&str>, format: &str) -> anyhow::Result<()> {
        data::list_generators_for_category(category, format)
    }

    /// What `fake list` needs.
    #[derive(Debug, Clone, Default)]
    pub struct ListGenerators {
        pub category: Option<String>,
        /// Substring of a generator's name or description.
        pub search: Option<String>,
        /// Include descriptions and examples.
        pub verbose: bool,
        /// `text` or `json`; empty means text.
        pub format: String,
    }

    /// Print the generators, filtered by `opts`.
    pub fn list(opts: &ListGenerators) -> anyhow::Result<()> {
        let format = if opts.format.is_empty() {
            "text"
        } else {
            opts.format.as_str()
        };
        data::list_generators(
            opts.category.as_deref(),
            opts.search.as_deref(),
            opts.verbose,
            format,
        )
    }

    /// What `fake image` needs to write one image.
    #[derive(Debug, Clone)]
    pub struct Image {
        /// `placeholder`, `avatar`, `gradient`, `checkerboard`, `noise`, or `stripes`.
        pub image_type: String,
        pub width: u32,
        pub height: u32,
        /// Hex colour, such as `#FF0000`.
        pub bg_color: Option<String>,
        pub text_color: Option<String>,
        pub text: Option<String>,
        /// For avatars.
        pub initials: Option<String>,
        /// Gradient start colour.
        pub start: Option<String>,
        /// Gradient end colour.
        pub end: Option<String>,
        /// `horizontal`, `vertical`, or `diagonal`.
        pub direction: String,
        /// `png` or `jpeg`.
        pub format: String,
        /// JPEG quality, 1 to 100.
        pub quality: u8,
        pub output: Option<String>,
        pub base64: bool,
        pub data_uri: bool,
        /// Coloured rather than grey noise.
        pub colored: bool,
        /// Open the result in the default viewer.
        pub open: bool,
    }

    impl Default for Image {
        fn default() -> Self {
            Self {
                image_type: "placeholder".to_string(),
                width: 200,
                height: 200,
                bg_color: None,
                text_color: None,
                text: None,
                initials: None,
                start: None,
                end: None,
                direction: "horizontal".to_string(),
                format: "png".to_string(),
                quality: 85,
                output: None,
                base64: false,
                data_uri: false,
                colored: false,
                open: false,
            }
        }
    }

    /// Write, print, or open the image `opts` describes.
    pub fn image(opts: &Image) -> anyhow::Result<()> {
        image::generate_fake_image(
            &opts.image_type,
            opts.width,
            opts.height,
            opts.bg_color.as_deref(),
            opts.text_color.as_deref(),
            opts.text.as_deref(),
            opts.initials.as_deref(),
            opts.start.as_deref(),
            opts.end.as_deref(),
            &opts.direction,
            &opts.format,
            opts.quality,
            opts.output.as_deref(),
            opts.base64,
            opts.data_uri,
            opts.colored,
            opts.open,
        )
    }

    /// What `fake pdf` needs to write one document.
    #[derive(Debug, Clone)]
    pub struct Pdf {
        pub pages: u32,
        pub text: Option<String>,
        pub output: Option<String>,
        pub base64: bool,
        pub data_uri: bool,
        pub open: bool,
    }

    impl Default for Pdf {
        fn default() -> Self {
            Self {
                pages: 1,
                text: None,
                output: None,
                base64: false,
                data_uri: false,
                open: false,
            }
        }
    }

    /// Write, print, or open the PDF `opts` describes.
    pub fn pdf(opts: &Pdf) -> anyhow::Result<()> {
        pdf::generate_fake_pdf(
            opts.pages,
            opts.text.as_deref(),
            opts.output.as_deref(),
            opts.base64,
            opts.data_uri,
            opts.open,
        )
    }

    /// What `fake preview` needs to render a template.
    #[derive(Debug, Clone)]
    pub struct Preview {
        /// The template text; `file` is read when this is `None`.
        pub template: Option<String>,
        pub file: Option<String>,
        /// Request context as JSON: `captures`, `headers`, `query`, `body`.
        pub context: Option<String>,
        /// How many times to render it.
        pub count: usize,
        /// `text` or `json`.
        pub format: String,
    }

    impl Default for Preview {
        fn default() -> Self {
            Self {
                template: None,
                file: None,
                context: None,
                count: 1,
                format: "text".to_string(),
            }
        }
    }

    /// Render the template `opts` names, the way a mock response would.
    pub async fn preview(opts: Preview) -> anyhow::Result<()> {
        preview::preview_template(
            opts.template.as_deref(),
            opts.file.as_deref(),
            opts.context.as_deref(),
            opts.count,
            &opts.format,
        )
        .await
    }

    /// What `fake serve` needs to listen.
    #[derive(Debug, Clone)]
    pub struct Server {
        pub port: u16,
        pub host: String,
        pub cors: bool,
        pub open: bool,
        pub verbose: bool,
    }

    impl Default for Server {
        fn default() -> Self {
            Self {
                port: 3005,
                host: "127.0.0.1".to_string(),
                cors: false,
                open: false,
                verbose: false,
            }
        }
    }

    /// Serve the generators and template rendering over HTTP until stopped.
    pub async fn serve(opts: Server) -> anyhow::Result<()> {
        server::serve_fake_data(opts.port, &opts.host, opts.cors, opts.open, opts.verbose).await
    }
}

#[cfg(test)]
// A test that cannot set up is a failed test; expect is the shortest way to say so.
#[allow(clippy::expect_used)]
mod tests {
    use super::{CreateMock, create_mock};

    /// A host builds `CreateMock` from its own flags and gets a file the
    /// registry loads. `template: true` is the branch that once wrote an
    /// unregistered function into the body.
    #[tokio::test]
    async fn create_mock_with_template_writes_a_loadable_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let output = dir.path().join("users.yaml");

        create_mock(CreateMock {
            url: Some("/api/users/:id".to_string()),
            output: Some(output.to_string_lossy().into_owned()),
            template: true,
            id: Some("get-user".to_string()),
            ..CreateMock::default()
        })
        .expect("create_mock");

        let registry = ferrimock::engine::MockRegistry::new();
        let loaded = registry
            .load_from_directory(dir.path().to_string_lossy().as_ref())
            .await
            .expect("the written mock loads");
        assert_eq!(loaded, 1, "one mock loaded from {}", output.display());
    }
}
