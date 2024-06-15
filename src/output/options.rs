use clap::Parser;

#[derive(Debug, Parser)]
pub struct TextOptions {
    /// Include headings in dump output.
    #[arg(short = 'H', long = "include-headings", default_value_t = false)]
    pub include_headings: bool,
    /// Include preformatted text in dump output.
    #[arg(short = 'P', long = "include-preformatted", default_value_t = false)]
    pub include_preformatted: bool,
    /// Include tables/table content in dump output.
    #[arg(short = 'T', long = "include-tables", default_value_t = true)]
    pub include_tables: bool,
    /// Produces Markdown instead of raw text dump.
    #[arg(short = 'M', long = "markdown", default_value_t = false)]
    pub include_formatting: bool,
    /// Makes produced output contain only sentences where possible.
    ///
    /// Not all edge cases are handled, but it will, for instance, exclude table
    /// cells and list items with text that doesn't end in punctuation.
    #[arg(short = 'S', long = "only-sentences", default_value_t = true)]
    pub only_sentences: bool,
}
