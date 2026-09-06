//! Fake data CLI commands: generation, images, PDFs, templates, and HTTP server.

pub(crate) mod data;
mod generators;
pub(crate) mod image;
pub(crate) mod pdf;
pub(crate) mod preview;
pub(crate) mod server;

use clap::{Args, Subcommand};

/// Generate fake data, images, and PDFs, and preview templates
#[derive(Args, Debug, Clone)]
pub struct FakeCommand {
    #[command(subcommand)]
    pub action: FakeAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum FakeAction {
    /// Print fake values of one type: names, emails, UUIDs, and a hundred more
    #[command(visible_alias = "d")]
    Data {
        /// Type of fake data to generate
        #[arg(value_name = "TYPE")]
        generator: String,
        /// Number of values to generate
        #[arg(short = 'n', long, default_value = "1")]
        count: usize,
        /// Minimum value (for numeric generators like price, number)
        #[arg(long)]
        min: Option<f64>,
        /// Maximum value (for numeric generators like price, number)
        #[arg(long)]
        max: Option<f64>,
        /// Word count (for sentence, paragraph generators)
        #[arg(short = 'w', long)]
        words: Option<usize>,
        /// Length (for alphanumeric, token generators)
        #[arg(short = 'l', long)]
        length: Option<usize>,
        /// Output format: text, json, csv
        #[arg(short = 'f', long, default_value = "text")]
        format: String,
        /// Copy result to clipboard
        #[arg(short = 'c', long)]
        copy: bool,
        /// List available generators in a category
        #[arg(long)]
        #[allow(clippy::option_option)]
        list: Option<Option<String>>,
    },

    /// Write a placeholder, avatar, gradient, or noise image
    #[command(visible_alias = "img")]
    Image {
        /// Type of image: placeholder, avatar, gradient, checkerboard, noise, stripes
        #[arg(value_name = "TYPE", default_value = "placeholder")]
        image_type: String,
        /// Image width in pixels
        #[arg(short = 'W', long, default_value = "200")]
        width: u32,
        /// Image height in pixels
        #[arg(short = 'H', long, default_value = "200")]
        height: u32,
        /// Background color (hex, e.g., "#FF0000")
        #[arg(short = 'b', long)]
        bg_color: Option<String>,
        /// Text color (hex, for placeholder/avatar)
        #[arg(short = 't', long)]
        text_color: Option<String>,
        /// Text to display on image
        #[arg(long)]
        text: Option<String>,
        /// Initials for avatar (e.g., "JS")
        #[arg(short = 'i', long)]
        initials: Option<String>,
        /// Avatar/placeholder size (shorthand for equal width/height)
        #[arg(short = 's', long)]
        size: Option<u32>,
        /// Start color for gradient
        #[arg(long)]
        start: Option<String>,
        /// End color for gradient
        #[arg(long)]
        end: Option<String>,
        /// Direction: horizontal, vertical, diagonal
        #[arg(short = 'd', long, default_value = "horizontal")]
        direction: String,
        /// Image format: png, jpeg
        #[arg(short = 'F', long, default_value = "png")]
        image_format: String,
        /// JPEG quality (1-100)
        #[arg(short = 'q', long, default_value = "85")]
        quality: u8,
        /// Output file path
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// Output as base64 string
        #[arg(long)]
        base64: bool,
        /// Output as data URI
        #[arg(long)]
        data_uri: bool,
        /// Generate colored noise (vs grayscale)
        #[arg(long)]
        colored: bool,
        /// Open generated image in default viewer
        #[arg(long)]
        open: bool,
    },

    /// Write a PDF with generated text
    #[command(visible_alias = "doc")]
    Pdf {
        /// Number of pages
        #[arg(short = 'p', long, default_value = "1")]
        pages: u32,
        /// Custom text content
        #[arg(short = 't', long)]
        text: Option<String>,
        /// Output file path
        #[arg(short = 'o', long)]
        output: Option<String>,
        /// Output as base64 string
        #[arg(long)]
        base64: bool,
        /// Output as data URI
        #[arg(long)]
        data_uri: bool,
        /// Open generated PDF in default viewer
        #[arg(long)]
        open: bool,
    },

    /// List the generators, by category
    #[command(visible_alias = "ls")]
    List {
        /// Filter by category
        #[arg(short = 'c', long)]
        category: Option<String>,
        /// Search for generators by name
        #[arg(short = 's', long)]
        search: Option<String>,
        /// Show detailed descriptions and examples
        #[arg(short = 'v', long)]
        verbose: bool,
        /// Output format: text, json
        #[arg(short = 'f', long, default_value = "text")]
        format: String,
    },

    /// Render a template the way a mock response would
    #[command(visible_alias = "tpl")]
    Preview {
        /// Template string to render
        #[arg(value_name = "TEMPLATE")]
        template: Option<String>,
        /// Template file to render
        #[arg(short = 'f', long)]
        file: Option<String>,
        /// Context data as JSON
        #[arg(short = 'c', long)]
        context: Option<String>,
        /// Number of times to render
        #[arg(short = 'n', long, default_value = "1")]
        count: usize,
        /// Output format: text, json
        #[arg(short = 'F', long, default_value = "text")]
        format: String,
    },

    /// Serve the generators and template rendering over HTTP
    #[command(visible_alias = "s")]
    Serve {
        /// Port to listen on
        #[arg(short = 'p', long, default_value = "3005")]
        port: u16,
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Enable CORS headers
        #[arg(long)]
        cors: bool,
        /// Open browser
        #[arg(short = 'o', long)]
        open: bool,
        /// Enable verbose request logging
        #[arg(short = 'v', long)]
        verbose: bool,
    },
}

/// Execute fake command
pub async fn execute(cmd: FakeCommand) -> anyhow::Result<()> {
    use crate::ops::fake as ops;
    match cmd.action {
        FakeAction::Data {
            generator,
            count,
            min,
            max,
            words,
            length,
            format,
            copy,
            list,
        } => {
            if let Some(category) = list {
                ops::list_category(category.as_deref(), &format)
            } else {
                ops::data(ops::Data {
                    generator,
                    count,
                    min,
                    max,
                    words,
                    length,
                    format,
                    copy,
                })
            }
        }
        FakeAction::Image {
            image_type,
            width,
            height,
            bg_color,
            text_color,
            text,
            initials,
            size,
            start,
            end,
            direction,
            image_format,
            quality,
            output,
            base64,
            data_uri,
            colored,
            open,
        } => {
            let (width, height) = size.map_or((width, height), |s| (s, s));
            ops::image(ops::Image {
                image_type,
                width,
                height,
                bg_color,
                text_color,
                text,
                initials,
                start,
                end,
                direction,
                format: image_format,
                quality,
                output,
                base64,
                data_uri,
                colored,
                open,
            })
        }
        FakeAction::Pdf {
            pages,
            text,
            output,
            base64,
            data_uri,
            open,
        } => ops::pdf(ops::Pdf {
            pages,
            text,
            output,
            base64,
            data_uri,
            open,
        }),
        FakeAction::List {
            category,
            search,
            verbose,
            format,
        } => ops::list(ops::ListGenerators {
            category,
            search,
            verbose,
            format,
        }),
        FakeAction::Preview {
            template,
            file,
            context,
            count,
            format,
        } => {
            ops::preview(ops::Preview {
                template,
                file,
                context,
                count,
                format,
            })
            .await
        }
        FakeAction::Serve {
            port,
            host,
            cors,
            open,
            verbose,
        } => {
            ops::serve(ops::Server {
                port,
                host,
                cors,
                open,
                verbose,
            })
            .await
        }
    }
}
