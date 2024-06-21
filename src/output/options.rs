use clap::Parser;

#[derive(Debug, Parser)]
pub struct GeneratorOptions {
    /// Collect redirection articles in a file.
    #[arg(short = 'R', long = "collect-redirects", default_value_t = false)]
    pub redirects: bool,
    /// Collect article metadata.
    #[arg(short = 'M', long = "collect-metadata", default_value_t = false)]
    pub metadata: bool,
    /// Collect all words into a dictionary.
    #[arg(short = 'D', long = "build-dictionary", default_value_t = false)]
    pub dictionary: bool,
    /// Collect text content into a dump file.
    #[arg(short = 'T', long = "collect-text", default_value_t = false)]
    pub text: bool,
}

impl GeneratorOptions {
    pub fn any(&self) -> bool {
        [self.redirects, self.metadata, self.dictionary, self.text]
            .into_iter()
            .any(|it| it)
    }
}

#[derive(Debug, Parser)]
pub struct TextOptions {
    /// Include headings in dump output.
    #[arg(short = 'H', long = "include-headings", default_value_t = false)]
    pub include_headings: bool,
    /// Include preformatted text in dump output.
    #[arg(short = 'P', long = "include-preformatted", default_value_t = false)]
    pub include_preformatted: bool,
    /// Exclude table content in dump output.
    #[arg(long = "no-tables", default_value_t = true)]
    pub include_tables: bool,
    /// Produce Markdown instead of raw text dump.
    #[arg(long = "markdown", default_value_t = false)]
    pub include_formatting: bool,
    /// Make produced output contain only sentences when possible
    ///
    /// Not all edge cases are handled, but it will (for instance) exclude table
    /// cells and list items with text that doesn't end in punctuation.
    #[arg(short = 'S', long = "only-sentences", default_value_t = true)]
    pub only_sentences: bool,
}
