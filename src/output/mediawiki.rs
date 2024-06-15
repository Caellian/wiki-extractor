use std::fmt::Write as _;

use parse_wiki_text_2::*;

use super::options::TextOptions;

pub const WIKI_CONFIGURATION: ConfigurationSource = ConfigurationSource {
    category_namespaces: &["category"],
    extension_tags: &[
        "categorytree",
        "ce",
        "charinsert",
        "chem",
        "gallery",
        "graph",
        "hiero",
        "imagemap",
        "indicator",
        "inputbox",
        "mapframe",
        "maplink",
        "math",
        "nowiki",
        "poem",
        "pre",
        "ref",
        "references",
        "score",
        "section",
        "source",
        "syntaxhighlight",
        "templatedata",
        "timeline",
    ],
    file_namespaces: &["file", "image"],
    link_trail: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz",
    magic_words: &[
        "DISAMBIG",
        "FORCETOC",
        "HIDDENCAT",
        "INDEX",
        "NEWSECTIONLINK",
        "NOCC",
        "NOCOLLABORATIONHUBTOC",
        "NOCONTENTCONVERT",
        "NOEDITSECTION",
        "NOGALLERY",
        "NOGLOBAL",
        "NOINDEX",
        "NONEWSECTIONLINK",
        "NOTC",
        "NOTITLECONVERT",
        "NOTOC",
        "STATICREDIRECT",
        "TOC",
    ],
    protocols: &[
        "//",
        "bitcoin:",
        "ftp://",
        "ftps://",
        "geo:",
        "git://",
        "gopher://",
        "http://",
        "https://",
        "irc://",
        "ircs://",
        "magnet:",
        "mailto:",
        "mms://",
        "news:",
        "nntp://",
        "redis://",
        "sftp://",
        "sip:",
        "sips:",
        "sms:",
        "ssh://",
        "svn://",
        "tel:",
        "telnet://",
        "urn:",
        "worldwind://",
        "xmpp:",
    ],
    redirect_magic_words: &["REDIRECT"],
};

pub fn nodes_to_string(raw: &str, nodes: &Vec<Node<'_>>, options: &TextOptions) -> String {
    let mut buffer = String::with_capacity(128);
    for inner in nodes {
        buffer.push_str(&node_to_string(raw, inner, options));
    }
    buffer
}

pub fn node_to_string(raw: &str, node: &Node<'_>, options: &TextOptions) -> String {
    let mut buffer = String::with_capacity(128);

    match node {
        Node::Text { value, .. } => buffer.push_str(value),
        Node::CharacterEntity { character, .. } => buffer.push(*character),
        Node::ParagraphBreak { .. } => buffer.push('\n'),
        Node::ExternalLink { nodes, .. } => {
            buffer.push_str(&nodes_to_string(raw, nodes, options));
        }
        Node::Heading { nodes, level, .. } => {
            if options.include_formatting {
                buffer.push_str(&"#".repeat(*level as usize));
                buffer.push(' ');
            }
            for inner in nodes {
                buffer.push_str(&node_to_string(raw, inner, options));
            }
            buffer.push('\n');
        }
        Node::Link { text, .. } => {
            for inner in text {
                buffer.push_str(&node_to_string(raw, inner, options));
            }
        }
        Node::Preformatted { nodes, .. } if options.include_preformatted => {
            buffer.push('\n');
            if options.include_formatting {
                buffer.push_str("```\n");
                buffer.push_str(&nodes_to_string(raw, nodes, options));
                buffer.push_str("```\n");
            } else {
                buffer.push_str(&nodes_to_string(raw, nodes, options));
            }
            buffer.push('\n');
        }
        Node::Table { rows, .. } if options.include_tables && options.include_formatting => {
            // not the prettiest formatting, but valid markdown
            buffer.push('\n');
            let mut is_first_row = true;
            for TableRow { cells, .. } in rows {
                buffer.push('|');
                for TableCell { content, .. } in cells {
                    buffer.push(' ');
                    buffer.push_str(&nodes_to_string(raw, content, options));
                    buffer.push_str(" |");
                }
                buffer.push('\n');
                if is_first_row {
                    buffer.push('|');
                    for _ in 0..cells.len() {
                        buffer.push_str("-|");
                    }
                    buffer.push('\n');
                    is_first_row = false;
                }
            }
        }
        Node::Table { rows, .. } if options.include_tables => {
            buffer.push('\n');
            for TableRow { cells, .. } in rows {
                for TableCell { content, type_, .. } in cells {
                    if *type_ == TableCellType::Ordinary {
                        let cell_text = nodes_to_string(raw, content, options);
                        if options.only_sentences && !cell_text.contains('.') {
                            continue;
                        }
                        buffer.push_str(&cell_text);
                        buffer.push('\n');
                    }
                }
            }
        }
        Node::OrderedList { items, .. } => {
            buffer.push('\n');
            for (i, ListItem { nodes, .. }) in items.iter().enumerate() {
                if options.include_formatting {
                    let _ = buffer.write_fmt(format_args!("{}. ", i));
                }
                let content = nodes_to_string(raw, nodes, options);
                if options.only_sentences && !content.ends_with('.') {
                    continue;
                }
                buffer.push_str(&content);
                buffer.push('\n');
            }
        }
        Node::UnorderedList { items, .. } => {
            buffer.push('\n');
            for ListItem { nodes, .. } in items {
                if options.include_formatting {
                    buffer.push_str("- ");
                }
                let content = nodes_to_string(raw, nodes, options);
                if options.only_sentences && !content.ends_with('.') {
                    continue;
                }
                buffer.push_str(&content);
                buffer.push('\n');
            }
        }
        Node::DefinitionList { items, .. } if options.include_formatting => {
            buffer.push('\n');
            let last = DefinitionListItemType::Details;
            for DefinitionListItem {
                type_: ty, nodes, ..
            } in items
            {
                if *ty == last && *ty != DefinitionListItemType::Details {
                    // definition list with consecutive term types; return nothing.
                    // does this make sense?
                    // multiple details for alternate definitions is ok.
                    return String::new();
                }
                match ty {
                    DefinitionListItemType::Term => {
                        buffer.push_str(&nodes_to_string(raw, nodes, options));
                        buffer.push('\n');
                    }
                    DefinitionListItemType::Details => {
                        buffer.push_str(": ");
                        buffer.push_str(&nodes_to_string(raw, nodes, options));
                        buffer.push('\n');
                    }
                }
            }
        }
        Node::DefinitionList { items, .. } => {
            buffer.push('\n');
            for DefinitionListItem {
                type_: ty, nodes, ..
            } in items
            {
                if *ty == DefinitionListItemType::Details {
                    buffer.push_str(&nodes_to_string(raw, nodes, options));
                    buffer.push('\n');
                }
            }
        }
        Node::Bold { .. } if options.include_formatting => {
            buffer.push_str("**");
        }
        Node::Italic { .. } if options.include_formatting => {
            buffer.push('_');
        }
        Node::BoldItalic { .. } if options.include_formatting => {
            buffer.push_str("***");
        }
        Node::Template {
            name, parameters, ..
        } => {
            buffer.push_str(&resolve_template(name, parameters));
        }
        _ => {}
    }

    buffer
}

fn resolve_template(_name: &[Node<'_>], _parameters: &[Parameter<'_>]) -> String {
    // TODO: {{lang-fr|anarchiste}}
    // Unicode CLDR has mapping from country codes to short names
    String::new()
}
