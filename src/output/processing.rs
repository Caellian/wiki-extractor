//! Contains text processing logic.

use std::sync::LazyLock;

use aho_corasick::AhoCorasick;

pub trait ProcessingPass {
    fn process(chunk: impl AsRef<str>) -> String;
}

pub struct MapXMLEntities;
impl ProcessingPass for MapXMLEntities {
    fn process(chunk: impl AsRef<str>) -> String {
        static PATTERNS: LazyLock<AhoCorasick> = LazyLock::new(|| {
            AhoCorasick::new(["&lt;", "&gt;", "&amp;", "&apos;", "&quot;"]).unwrap()
        });
        static REPLACEMENTS: &[&str] = &["<", ">", "&", "'", "\""];
        PATTERNS.replace_all(chunk.as_ref(), REPLACEMENTS)
    }
}

pub struct CollapseWhitespace;
impl ProcessingPass for CollapseWhitespace {
    fn process(chunk: impl AsRef<str>) -> String {
        let mut result = String::with_capacity(chunk.as_ref().len());

        let mut newline_count = 0;
        let mut space_count = 0;
        for c in chunk.as_ref().chars() {
            match c {
                '\n' => {
                    newline_count += 1;
                    space_count = 0;
                    if newline_count > 2 {
                        continue;
                    }
                }
                ' ' | '\u{00A0}' => {
                    if newline_count > 0 {
                        // if on new line pretend we've seen space before so
                        // starting spaces don't get printed.
                        space_count += 1;
                    }
                    newline_count = 0;
                    space_count += 1;
                    if space_count == 1 {
                        result.push(' ');
                    }
                    continue;
                }
                _ => {
                    newline_count = 0;
                    space_count = 0;
                }
            }
            result.push(c);
        }

        result
    }
}

pub struct StripWords;
impl ProcessingPass for StripWords {
    fn process(chunk: impl AsRef<str>) -> String {
        let mut result = String::with_capacity(chunk.as_ref().len());

        let mut delimited = true;
        for c in chunk.as_ref().chars() {
            match c {
                c if c.is_alphabetic() => {
                    result.push(c);
                    delimited = false;
                }
                c if c.is_whitespace() => {
                    if delimited {
                        continue;
                    }
                    result.push(' ');
                    delimited = true;
                }
                '\u{002D}'
                | '\u{058A}'
                | '\u{1806}'
                | '\u{2010}'..='\u{2015}'
                | '\u{FE58}'
                | '\u{FE63}'
                | '\u{FF0D}' => result.push('-'),
                _ => {}
            }
        }

        result
    }
}
