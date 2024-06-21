use std::{
    collections::HashSet,
    fs::File,
    io::{ErrorKind, Write as _},
    path::{Path, PathBuf}, sync::Arc, future::IntoFuture,
};

use futures::{Future, future::BoxFuture};
use itertools::Itertools;
use parse_wiki_text_2::Configuration as MediawikiConfig;

use super::{
    mediawiki::{self, WIKI_CONFIGURATION},
    options::TextOptions,
};
use super::{
    options::GeneratorOptions,
    processing::{MapXMLEntities, ProcessingPass as _},
};
use crate::dump_data::{DocumentContext, WikiPage};

fn sanitize_escapes(text: impl AsRef<str>, checked: char) -> String {
    let mut result = String::with_capacity(text.as_ref().len() + 16);

    let mut escaped = false;
    for c in text.as_ref().chars() {
        match c {
            it if !escaped && it == checked => {
                result.push('\\');
                result.push(it);
            }
            '\\' => {
                escaped = !escaped;
            }
            other => {
                result.push(other);
                escaped = false;
            }
        }
    }

    result
}

pub struct Dictionary {
    file: PathBuf,
    words: HashSet<String>,
}

impl Dictionary {
    pub fn new(target: impl AsRef<Path>) -> Self {
        let file = target.as_ref().to_path_buf();
        let words = if let Ok(base) = std::fs::read_to_string(&file) {
            HashSet::from_iter(base.split('\n').map(str::to_string))
        } else {
            HashSet::with_capacity(1024)
        };

        Dictionary { file, words }
    }

    /// Push text into dictionary.
    ///
    /// This method is a bit faulty because it can only rely on common grammar
    /// rules to separate words out of the text.
    ///
    /// Examples of input that will be handled incorrectly:
    /// - `I was there with Dr. Abigail to see the show.` is treated as two
    ///   sentences and `Dr.` will be stripped of punctuation.
    pub async fn push(&mut self, text: impl AsRef<str>) {
        // iterate over words with forward context
        let words = text
            .as_ref()
            .split(' ')
            .map(|word| {
                let is_uppercase = word
                    .chars()
                    .next()
                    .map(|it| it.is_uppercase())
                    .unwrap_or_default();
                (Some(word.trim()), is_uppercase)
            })
            .chain(std::iter::once((None, true)));
        for ((word, _is_uppercase), (next_word, is_next_uppercase)) in words.tuple_windows() {
            let mut word = unsafe {
                // SAFETY: None is inserted only as next_word of last window.
                word.unwrap_unchecked()
            };
            if word.ends_with('.') {
                if word.len() == 2 {
                    // name abbr.
                    continue;
                }
                if let Some(next_word) = next_word {
                    if next_word.starts_with('\n') || is_next_uppercase {
                        // end of sentence
                        word = word.strip_suffix('.').unwrap();
                    } // else abbr.
                } else {
                    word = word.strip_suffix('.').unwrap();
                }
            }
            self.words.insert(word.to_string());
        }
    }

    async fn push_arc(&mut self, text: Arc<String>) {
        self.push(text.as_str()).await;
    }

    pub fn write(self) -> std::io::Result<()> {
        let mut dictionary_file = File::create(self.file)?;
        for item in self.words {
            dictionary_file.write_all(item.as_bytes())?;
            dictionary_file.write_all(b"\n")?;
        }

        Ok(())
    }
}

pub struct DataGenerator {
    metadata: Option<File>,
    text_dump: Option<File>,
    redirects: Option<File>,
    dictionary: Option<Dictionary>,
    mediawiki_parser: MediawikiConfig,
    text_options: TextOptions,
    first_write: bool,
    closed: bool,
}

impl DataGenerator {
    pub fn new(
        output_path: impl AsRef<Path>,
        generator_options: GeneratorOptions,
        text_options: TextOptions,
    ) -> std::io::Result<Self> {
        let output_path = output_path.as_ref();
        if output_path.is_file() {
            log::error!("output path points to a file and not a directory");
        }
        if !output_path.exists() {
            std::fs::create_dir_all(output_path)?;
        }

        // TODO: Allow disabling generation of individual files
        let metadata = if generator_options.metadata {
            let metadata = output_path.join("wiki_page_info.json");
            let mut metadata = File::create(metadata)?;
            metadata.write_all(b"[\n")?;
            Some(metadata)
        } else {
            None
        };

        let text_dump = if generator_options.text {
            let text_dump = output_path.join("wiki_sentences.txt");
            let text_dump = File::create(text_dump)?;
            Some(text_dump)
        } else {
            None
        };

        let redirects = if generator_options.redirects {
            let redirects = output_path.join("redirects.json");
            let mut redirects = File::create(redirects)?;
            redirects.write_all(b"{\n")?;
            Some(redirects)
        } else {
            None
        };

        let dictionary = if generator_options.dictionary {
            let dictionary = output_path.join("dictionary.txt");
            Some(Dictionary::new(dictionary))
        } else {
            None
        };

        Ok(DataGenerator {
            metadata,
            text_dump,
            redirects,
            dictionary,
            mediawiki_parser: MediawikiConfig::new(&WIKI_CONFIGURATION),
            text_options,
            first_write: true,
            closed: false,
        })
    }

    pub async fn process_document(
        &mut self,
        document: &mut DocumentContext,
    ) -> std::io::Result<()> {
        if self.closed {
            panic!("called process document with closed DataGenerator");
        }

        let has_pages =
            |doc: &DocumentContext| doc.pages.first().map(|it| it.closed).unwrap_or_default();

        while has_pages(document) {
            let page = document.pages.remove(0);
            match self.process_page(page).await {
                Ok(jobs) => {
                    futures::future::join_all(jobs).await;
                }
                Err(err) => {
                    if err.kind() == ErrorKind::Unsupported {
                        continue;
                    } else {
                        return Err(err);
                    }
                }
            }
            self.first_write = false;
        }

        Ok(())
    }

    async fn process_page(&mut self, mut page: WikiPage) -> std::io::Result<Vec<BoxFuture<'_, ()>>> {
        if let Some(redirect) = &page.redirect {
            if let Some(redirect_file) = &mut self.redirects {
                if let Some(title) = page.title.value() {
                    if !self.first_write {
                        let _ = redirect_file.write_all(b",\n");
                    }
                    let _ = redirect_file.write_all(b"  \"");
                    let escaped = sanitize_escapes(title, '\"');
                    let _ = redirect_file.write_all(escaped.as_bytes());
                    let _ = redirect_file.write_all(b"\": \"");
                    let escaped = sanitize_escapes(redirect, '\"');
                    let _ = redirect_file.write_all(escaped.as_bytes());
                    let _ = redirect_file.write_all(b"\"");
                }
            }
            return Ok(vec![]);
        }

        let mut revisions = std::mem::take(&mut page.revisions);
        let rev = match revisions.last_mut() {
            Some(it) => it,
            None => return Ok(vec![]),
        };

        if rev.model.value().map(|it| it.as_str()) != Some("wikitext")
            && rev.format.value().map(|it| it.as_str()) != Some("text/x-wiki")
        {
            // program is outdated/broken
            let message = format!(
                "Unhandled page ({}: {}) model/format: {{ model: \"{}\"; format: \"{}\" }}\n{:#?}",
                page.id.value().map(usize::to_string).unwrap_or_default(),
                page.title.value().map(String::as_str).unwrap_or(""),
                rev.model.value().map(String::as_str).unwrap_or_default(),
                rev.format.value().map(String::as_str).unwrap_or_default(),
                page
            );
            return Err(std::io::Error::new(ErrorKind::Unsupported, message));
        }

        // Cleanup XML encoding of nested XML content
        let raw_text = match rev.text.take_value() {
            Some(it) => MapXMLEntities::process(it),
            None => return Ok(vec![]),
        };

        let nodes = match self.mediawiki_parser.parse(&raw_text) {
            Ok(it) => {
                if !it.warnings.is_empty() {
                    let warnings = "- ".to_string()
                        + it.warnings
                            .into_iter()
                            .map(|it| it.message.to_string())
                            .unique()
                            .join("\n- ")
                            .as_ref();
                    log::warn!(
                        "Well-formedness issues on ({}: {}):\n{}",
                        page.id.value().map(usize::to_string).unwrap_or_default(),
                        page.title.value().map(String::as_str).unwrap_or(""),
                        warnings
                    )
                }
                it.nodes
            }
            Err(err) => {
                let message = format!(
                    "can't parse page: ({}: {}): {:?}",
                    page.id.value().map(usize::to_string).unwrap_or_default(),
                    page.title.value().map(String::as_str).unwrap_or(""),
                    err
                );
                return Err(std::io::Error::new(ErrorKind::Unsupported, message));
            }
        };

        let mut jobs: Vec<BoxFuture<'_, ()>> = Vec::with_capacity(2);

        let text = Arc::new(mediawiki::nodes_to_text(&nodes, &self.text_options));
        if let Some(dictionary) = &mut self.dictionary {
            jobs.push(Box::pin(dictionary.push_arc(text.clone())));
        }

        if let Some(text_dump) = &mut self.text_dump {
            text_dump.write_all(text.as_bytes())?;
        }

        Ok(jobs)
    }

    pub fn finalize(mut self) -> std::io::Result<()> {
        if self.closed {
            panic!("called finalize on DataGenerator twice");
        }

        if let Some(mut redirects) = self.redirects {
            redirects.write_all(b"}\n")?;
            redirects.flush()?;
        }

        if let Some(mut metadata) = self.metadata {
            metadata.write_all(b"]\n")?;
            metadata.flush()?;
        }

        if let Some(dictionary) = self.dictionary {
            dictionary.write()?;
        }

        self.closed = true;

        Ok(())
    }
}
