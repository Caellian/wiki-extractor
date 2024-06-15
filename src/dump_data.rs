#![allow(dead_code)]

use quick_xml::events::Event as XMLEvent;
use serde::{Deserialize, Serialize};

use crate::{close_all_nested, forward_closeable, impl_forwarding_closeable_handler};
use crate::{input::data::DumpLocation, xml_util::*};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Namespace {
    key: isize,
    name: String,
    #[serde(skip)]
    state: CloseableState,
}

impl FromAttributes for Namespace {
    fn from_attributes(attr: AttributeMap<'_>) -> ParseResult<Self> {
        let mut key = None;

        for it in attr {
            let attribute = it?;

            if attribute.key.0 == b"key" {
                let key_str = std::str::from_utf8(&attribute.value).map_field_err("key")?;
                key = Some(key_str.parse::<isize>().map_field_err("key")?);
            }
        }

        match key {
            Some(key) => ParseResult::Ok(Namespace {
                key,
                name: String::with_capacity(8),
                state: CloseableState::Open,
            }),
            _ => Err(ParseError::MissingAttribute {
                parent: "namespace",
                attribute: "key",
            }),
        }
    }
}

impl HandleEvent for Namespace {
    fn handle_event(&mut self, event: XMLEvent<'_>) -> ParseResult<()> {
        match event {
            XMLEvent::End(tag) if tag.name().0 == b"namespace" => {
                return self.close();
            }
            XMLEvent::Text(text) => {
                self.name += std::str::from_utf8(&text).unwrap();
            }
            XMLEvent::CData(chars) => {
                self.name += std::str::from_utf8(&chars).unwrap();
            }
            XMLEvent::PI(_) | XMLEvent::DocType(_) | XMLEvent::Comment(_) => {}
            XMLEvent::Eof => panic!("unclosed Namespace"),
            _ => unimplemented!("unhandled Namespace child"),
        }
        Ok(())
    }
}

impl Closeable for Namespace {
    const KEY: &'static str = "namespace";

    fn close_state(&self) -> crate::xml_util::CloseableState {
        self.state
    }

    fn close(&mut self) -> ParseResult<()> {
        self.name.shrink_to_fit();
        self.state = CloseableState::Closed;
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct SiteInfo {
    site_name: ValueTag<String, "sitename">,
    db_name: ValueTag<String, "dbname">,
    base: ValueTag<String, "base">,
    generator: ValueTag<String, "generator">,
    ns: XMLList<Namespace, "namespaces">,
    state: CloseableState,
}

impl FromAttributes for SiteInfo {
    fn from_attributes(_: AttributeMap<'_>) -> ParseResult<Self> {
        Ok(SiteInfo::default())
    }
}

impl_forwarding_closeable_handler! { SiteInfo as info => [
    info.site_name,
    info.db_name,
    info.base,
    info.generator,
    info.ns,
] or {
    match event {
        XMLEvent::End(tag) => {
            if tag.name().0 == b"siteinfo" {
                return info.close();
            }
        }
        _ => {}
    }
}}

impl Closeable for SiteInfo {
    const KEY: &'static str = "siteinfo";

    fn close_state(&self) -> CloseableState {
        self.state
    }

    fn close(&mut self) -> ParseResult<()> {
        close_all_nested![
            self.site_name,
            self.db_name,
            self.base,
            self.generator,
            self.ns,
        ];
        self.state = CloseableState::Closed;
        Ok(())
    }
}

// TODO: Use DateTime<Utc> for timestamp & proper sha1 type
#[derive(Debug, Default)]
pub struct Revision {
    pub id: ValueTag<usize, "id">,
    pub parent_id: ValueTag<usize, "parentid">,
    pub timestamp: ValueTag<String, "timestamp">,
    // contributor { username: str, id: usize }
    // minor
    pub comment: ValueTag<String, "comment">,
    pub model: ValueTag<String, "model">,
    pub format: ValueTag<String, "format">,
    pub text: ValueTag<String, "text">,
    pub sha1: ValueTag<String, "sha1">,
    pub state: CloseableState,
}

impl_forwarding_closeable_handler! {Revision as rev => [
    rev.id,
    rev.parent_id,
    rev.timestamp,
    rev.comment,
    rev.model,
    rev.format,
    rev.text,
    rev.sha1,
] or {match event {
    XMLEvent::End(tag) => {
        if tag.name().0 == b"revision" {
            return rev.close();
        }
    }
    _ => {}
}}}

impl Closeable for Revision {
    const KEY: &'static str = "revision";

    fn close_state(&self) -> CloseableState {
        self.state
    }

    fn close(&mut self) -> ParseResult<()> {
        close_all_nested![
            self.id,
            self.parent_id,
            self.timestamp,
            self.comment,
            self.model,
            self.format,
            self.text,
            self.sha1,
        ];
        self.state = CloseableState::Closed;
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct WikiPage {
    pub title: ValueTag<String, "title">,
    pub ns: ValueTag<isize, "ns">,
    pub id: ValueTag<usize, "id">,
    pub redirect: Option<String>,
    pub revisions: Vec<Revision>,
    pub closed: bool,
}

fn redirect_target(tag: AttributeMap<'_>) -> String {
    if let Some(Ok(value)) = tag.get("title") {
        value.to_string()
    } else {
        "unknown".to_string()
    }
}

impl_forwarding_closeable_handler! {WikiPage as page => [
    page.title,
    page.ns,
    page.id,
] or {
    match event {
        XMLEvent::Start(tag) => {
            let last_rev = page.revisions.last_mut();
            if let Some(last_rev) = last_rev {
                return last_rev.handle_event(XMLEvent::Start(tag));
            }

            if tag.name().0 == b"revision" {
                page.revisions.push(Revision {
                    state: CloseableState::Open,
                    ..Default::default()
                });
                return Ok(());
            }
        },
        XMLEvent::Empty(tag) => {
            let last_rev = page.revisions.last_mut();
            if let Some(last_rev) = last_rev {
                return last_rev.handle_event(XMLEvent::Empty(tag));
            }

            if tag.name().0 == b"redirect" {
                page.redirect = Some(redirect_target(AttributeMap::of(&tag)));
                return Ok(());
            }
        },
        XMLEvent::End(tag) => {
            let last_rev = page.revisions.last_mut();
            if let Some(last_rev) = last_rev {
                return last_rev.handle_event(XMLEvent::End(tag));
            }
            if tag.name().0 == b"page" {
                page.closed = true;
                return Ok(());
            }
        }
        other => {
            let last_rev = page.revisions.last_mut();
            if let Some(last_rev) = last_rev {
                return last_rev.handle_event(other);
            }
        }
    }
}}

impl Closeable for WikiPage {
    const KEY: &'static str = "page";

    fn close_state(&self) -> CloseableState {
        if self.closed {
            CloseableState::Closed
        } else {
            CloseableState::Open
        }
    }

    fn close(&mut self) -> ParseResult<()> {
        self.closed = true;
        Ok(())
    }
}

#[derive(Debug)]
pub struct DocumentContext {
    pub file_name: String,
    pub namespace: Option<String>,
    pub site_info: SiteInfo,
    pub pages: Vec<WikiPage>,
}

impl DocumentContext {
    pub fn new(dump_file: &DumpLocation) -> Self {
        DocumentContext {
            file_name: dump_file.name().to_string(),
            namespace: None,
            site_info: SiteInfo::default(),
            pages: Vec::with_capacity(1),
        }
    }
}

const VALIDATE_NAMESPACE: bool = true;

impl HandleEvent for DocumentContext {
    fn handle_event(&mut self, event: XMLEvent<'_>) -> ParseResult<()> {
        match event {
            XMLEvent::Start(tag) if VALIDATE_NAMESPACE && self.namespace.is_none() => {
                // this match case only handles document validation
                if tag.name().0 != b"mediawiki" {
                    return Err(ParseError::InvalidFormat {
                        reason: "not a mediawiki XML document",
                    });
                }
                let xmlns = tag
                    .attributes()
                    .filter_map(|it| it.ok().take_if(|it| it.key.0 == b"xmlns"))
                    .next()
                    .ok_or_else(|| ParseError::InvalidFormat {
                        reason: "missing XML namespace attribute",
                    })?;
                if !xmlns
                    .value
                    .starts_with(b"http://www.mediawiki.org/xml/export")
                {
                    return Err(ParseError::InvalidFormat {
                        reason: "not an mediawiki XML export",
                    });
                }
                self.namespace = Some(
                    std::str::from_utf8(&xmlns.value)
                        .map_err(|_| ParseError::InvalidFormat {
                            reason: "not a UTF-8 namespace",
                        })?
                        .to_string(),
                );
                return Ok(());
            }
            XMLEvent::Start(tag) => {
                forward_closeable!(XMLEvent::Start(tag) => [
                    self.site_info
                ]);
                let last_page = self.pages.last_mut();
                if let Some(last_page) = last_page {
                    if !last_page.closed {
                        return last_page.handle_event(XMLEvent::Start(tag));
                    }
                }

                if tag.name().0 == b"siteinfo" {
                    self.site_info.state = CloseableState::Open;
                    return Ok(());
                } else if tag.name().0 == b"page" {
                    self.pages.push(WikiPage::default());
                    return Ok(());
                }
            }
            XMLEvent::End(tag) => {
                forward_closeable!(XMLEvent::End(tag) => [
                    self.site_info
                ]);
                let last_page = self.pages.last_mut();
                if let Some(last_page) = last_page {
                    if !last_page.closed {
                        return last_page.handle_event(XMLEvent::End(tag));
                    }
                }
            }
            other => {
                forward_closeable!(other => [
                    self.site_info
                ]);
                let last_page = self.pages.last_mut();
                if let Some(last_page) = last_page {
                    if !last_page.closed {
                        return last_page.handle_event(other);
                    }
                }
            }
        }
        Ok(())
    }
}
