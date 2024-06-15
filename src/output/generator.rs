use std::{
    collections::HashSet,
    fs::File,
    io::Write as _,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use itertools::Itertools;
use parse_wiki_text_2::{Configuration as MediawikiConfig, *};

use super::processing::{CollapseWhitespace, MapXMLEntities, ProcessingPass as _};
use super::{
    mediawiki::{self, WIKI_CONFIGURATION},
    options::TextOptions,
};
use crate::dump_data::DocumentContext;

/// List of lowercase Wikipedia section titles to skip.
const SKIP_SECTIONS: &[&str] = &[
    "see also",        // contains mostly links and no sentences
    "references",      // not sentences
    "further reading", // not sentences
    "external links",  // not sentences
];

static MAX_SKIP_LEN: LazyLock<usize> = LazyLock::new(|| {
    SKIP_SECTIONS
        .iter()
        .map(|it| it.len())
        .max()
        .unwrap_or_default()
});

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

pub struct DataGenerator {
    metadata: File,
    text_dump: File,
    redirects: File,
    dictionary_target: PathBuf,
    dictionary: HashSet<String>,
    mediawiki_parser: MediawikiConfig,
    text_options: TextOptions,
    first_write: bool,
    closed: bool,
}

impl DataGenerator {
    pub fn new(output_path: impl AsRef<Path>, text_options: TextOptions) -> std::io::Result<Self> {
        let output_path = output_path.as_ref();
        if output_path.is_file() {
            log::error!("output path points to a file and not a directory");
        }
        if !output_path.exists() {
            std::fs::create_dir_all(output_path)?;
        }

        // TODO: Allow disabling generation of individual files
        let metadata = output_path.join("wiki_page_info.json");
        let mut metadata = File::create(metadata)?;
        metadata.write_all(b"[\n")?;

        let redirects = output_path.join("redirects.json");
        let mut redirects = File::create(redirects)?;
        redirects.write_all(b"{\n")?;

        let text_dump = output_path.join("wiki_sentences.txt");
        let text_dump = File::create(text_dump)?;

        Ok(DataGenerator {
            metadata,
            text_dump,
            redirects,
            dictionary_target: output_path.join("dictionary.txt"),
            dictionary: HashSet::with_capacity(1024),
            mediawiki_parser: MediawikiConfig::new(&WIKI_CONFIGURATION),
            text_options,
            first_write: true,
            closed: false,
        })
    }

    /// Push an article into dictionary.
    /// 
    /// This method is a bit faulty because it can only rely on common grammar
    /// rules to separate words out of the text.
    /// 
    /// Examples of input that will be handled incorrectly:
    /// - `I was there with Dr. Abigail to see the show.` is treated as two
    ///   sentences and `Dr.` will be stripped of punctuation.
    fn push_dictionary(&mut self, text: impl AsRef<str>) {
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
            self.dictionary.insert(word.to_string());
        }
    }

    pub fn process_document(&mut self, document: &mut DocumentContext) -> std::io::Result<()> {
        if self.closed {
            panic!("called process document with closed DataGenerator");
        }

        let has_pages =
            |doc: &DocumentContext| doc.pages.first().map(|it| it.closed).unwrap_or_default();

        while has_pages(document) {
            let mut page = document.pages.remove(0);

            if let Some(redirect) = &page.redirect {
                if let Some(title) = page.title.value() {
                    if !self.first_write {
                        let _ = self.redirects.write_all(b",\n");
                    }
                    let _ = self.redirects.write_all(b"  \"");
                    let escaped = sanitize_escapes(title, '\"');
                    let _ = self.redirects.write_all(escaped.as_bytes());
                    let _ = self.redirects.write_all(b"\": \"");
                    let escaped = sanitize_escapes(redirect, '\"');
                    let _ = self.redirects.write_all(escaped.as_bytes());
                    let _ = self.redirects.write_all(b"\"");
                    continue;
                }
            }

            let mut revisions = std::mem::take(&mut page.revisions);
            let rev = match revisions.last_mut() {
                Some(it) => it,
                None => break,
            };

            if rev.model.value().expect("revision missing model info") != "wikitext"
                && rev.format.value().expect("revision missing format info") != "text/x-wiki"
            {
                // program is outdated/broken
                log::error!("Unhandled page ({}: {}) model/format: {{ model: \"{}\"; format: \"{}\" }}\n{:#?}",
                    page.id.value().map(usize::to_string).unwrap_or_default(),
                    page.title.value().map(String::as_str).unwrap_or(""),
                    rev.model.value().map(String::as_str).unwrap_or_default(),
                    rev.format.value().map(String::as_str).unwrap_or_default(),
                    page
                );
                continue;
            }

            let text = match rev.text.take_value() {
                Some(it) => MapXMLEntities::process(it),
                None => continue,
            };

            let nodes = match self.mediawiki_parser.parse(&text) {
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
                    log::error!(
                        "can't parse page: ({}: {}): {:?}",
                        page.id.value().map(usize::to_string).unwrap_or_default(),
                        page.title.value().map(String::as_str).unwrap_or(""),
                        err
                    );
                    continue;
                }
            };

            let mut text = String::with_capacity(2048);
            let mut skip_section = None;
            for node in nodes {
                if let Some(req_level) = skip_section {
                    if let Node::Heading { level, .. } = node {
                        if level <= req_level {
                            skip_section = None;
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                let content = mediawiki::node_to_string(&text, &node, &self.text_options);
                let trimmed = content.trim();
                if let Node::Heading { level, .. } = node {
                    let trimmed = if self.text_options.include_formatting {
                        unsafe {
                            // SAFETY: '#' char takes up a single byte and
                            // formatting adds level '#'s, followed by a space
                            std::str::from_utf8_unchecked(
                                trimmed.as_bytes().split_at(level as usize + 1).1,
                            )
                        }
                    } else {
                        trimmed
                    };
                    // avoid O(3n) lowercase check with O(1) len check
                    if trimmed.len() <= *MAX_SKIP_LEN {
                        let lower = trimmed.to_ascii_lowercase();
                        if SKIP_SECTIONS.contains(&lower.as_str()) {
                            skip_section = Some(level);
                            continue;
                        }
                    }
                    if !self.text_options.include_headings {
                        self.push_dictionary(trimmed);
                        continue;
                    }
                }
                if trimmed.is_empty() {
                    continue;
                }
                if text.as_bytes().last() == Some(&b'.') {
                    text.push(' ');
                }
                text.push_str(&content);
            }
            text = CollapseWhitespace::process(text);
            self.push_dictionary(&text);

            let _ = self.text_dump.write_all(text.as_bytes());
            self.first_write = false;
        }

        Ok(())
    }

    pub fn finalize(mut self) -> std::io::Result<()> {
        if self.closed {
            panic!("called finalize on DataGenerator twice");
        }

        self.redirects.write_all(b"}\n")?;
        self.redirects.flush()?;

        self.metadata.write_all(b"]\n")?;
        self.metadata.flush()?;

        let mut dictionary_file = File::create(self.dictionary_target)?;
        for item in self.dictionary {
            dictionary_file.write_all(item.as_bytes())?;
            dictionary_file.write_all(b"\n")?;
        }
        drop(dictionary_file);

        self.closed = true;

        Ok(())
    }
}
