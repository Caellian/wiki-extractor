//! Utilities for handling partial/streaming XML data.

use std::{
    alloc::Layout,
    collections::HashMap,
    fmt::Display,
    ops::{Deref, DerefMut},
    str::FromStr,
};

use quick_xml::events::{
    attributes::{AttrError, Attribute, Attributes},
    BytesStart, Event as XMLEvent,
};

pub mod error {
    use std::fmt::Display;
    use std::{convert::Infallible, str::Utf8Error};

    use quick_xml::events::attributes::AttrError;
    use thiserror::Error;

    #[derive(Debug)]
    pub enum ValueErrorKind {
        NonUTF8,
        InvalidInt,
        InvalidFloat,
    }

    impl Display for ValueErrorKind {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(match self {
                ValueErrorKind::NonUTF8 => "not a UTF-8 value",
                ValueErrorKind::InvalidInt => "invalid integer value",
                ValueErrorKind::InvalidFloat => "invalid float value",
            })
        }
    }

    #[derive(Debug, Error)]
    #[error("invalid {field} value: {reason}")]
    pub struct ValueError {
        field: &'static str,
        reason: ValueErrorKind,
    }

    pub trait FieldResultMap<T, E: std::error::Error> {
        fn map_field_err(self, field: &'static str) -> Result<T, E>;
    }

    macro_rules! impl_err_mappings {
        [$($err: ty => $variant: ident),+ $(,)?] => {
           $(
                impl<T> FieldResultMap<T, ValueError> for Result<T, $err> {
                    fn map_field_err(self, field: &'static str) -> Result<T, ValueError> {
                        self.map_err(|_| ValueError {
                            field,
                            reason: ValueErrorKind::$variant,
                        })
                    }
                }
            )+
        };
    }

    impl_err_mappings![
        Utf8Error => NonUTF8,
        std::num::ParseIntError => InvalidInt,
        std::num::ParseFloatError => InvalidFloat,
    ];

    impl<T> FieldResultMap<T, ValueError> for Result<T, Infallible> {
        fn map_field_err(self, field: &'static str) -> Result<T, ValueError> {
            match self {
                Ok(it) => Ok(it),
                Err(_) => unreachable!(
                    "reached unreachable error while parsing '{}' value for {}",
                    field,
                    std::any::type_name::<T>()
                ),
            }
        }
    }

    #[derive(Debug, Error)]
    pub enum ParseError {
        #[error("invalid document format: {reason}")]
        InvalidFormat { reason: &'static str },
        #[error("invalid attribute format: {0}")]
        BadAttribute(
            #[from]
            #[source]
            AttrError,
        ),
        #[error("{parent} missing '{attribute}' attribute")]
        MissingAttribute {
            parent: &'static str,
            attribute: &'static str,
        },
        #[error(transparent)]
        ValueError(#[from] ValueError),
        #[error("tag is in '{0}' state")]
        BadCloseableState(super::CloseableState),
        #[error("can't handle event: {reason}")]
        UnhandledEvent { reason: &'static str },

        #[error("invalid stream character/encoding: {0}")]
        EncodingError(
            #[from]
            #[source]
            Utf8Error,
        ),

        #[error(transparent)]
        Other(#[from] Box<dyn std::error::Error>),
    }

    impl From<Infallible> for ParseError {
        fn from(_: Infallible) -> Self {
            unreachable!()
        }
    }
}

pub type ParseResult<T> = std::result::Result<T, ParseError>;
pub use error::{FieldResultMap, ParseError};

use self::error::ValueError;

#[derive(Clone, Debug)]
pub struct AttributeMap<'a>(Option<Attributes<'a>>);

impl<'a> AttributeMap<'a> {
    pub fn none() -> Self {
        AttributeMap(None)
    }

    pub fn of(tag: &'a BytesStart<'a>) -> Self {
        if tag.attributes().count() > 0 {
            AttributeMap(Some(tag.attributes()))
        } else {
            AttributeMap::none()
        }
    }

    pub fn into_hashmap(self) -> ParseResult<HashMap<String, String>> {
        let mut result = HashMap::new();
        for item in self {
            let item = item?;
            result.insert(
                std::str::from_utf8(item.key.0)?.to_string(),
                std::str::from_utf8(&item.value)?.to_string(),
            );
        }
        Ok(result)
    }

    pub fn get(&self, name: impl AsRef<str>) -> Option<ParseResult<&'a str>> {
        let name = name.as_ref();

        let attributes = match &self.0 {
            Some(it) => it.clone(),
            None => return None,
        };

        for attribute in attributes {
            let attribute = match attribute {
                Ok(it) => it,
                Err(it) => return Some(Err(it.into())),
            };

            let key = match std::str::from_utf8(attribute.key.0) {
                Ok(it) => it,
                Err(it) => return Some(Err(it.into())),
            };

            if key == name {
                let value = match std::str::from_utf8(attribute.value.as_ref()) {
                    Ok(it) => unsafe {
                        // SAFETY: Correcting `'a: 'local` lifetime of a utf8
                        // representation of bytes from the buffer, which should
                        // outlive Attributes<'a>, into `'a` because they're equal.
                        std::mem::transmute::<&str, &'a str>(it)
                    },
                    Err(it) => return Some(Err(it.into())),
                };

                return Some(Ok(value));
            }
        }

        None
    }
}

impl<'a> Iterator for AttributeMap<'a> {
    type Item = Result<Attribute<'a>, AttrError>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            Some(it) => it.next(),
            None => None,
        }
    }
}

impl<'a> Deref for AttributeMap<'a> {
    type Target = Option<Attributes<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> DerefMut for AttributeMap<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub trait HandleEvent {
    fn handle_event(&mut self, event: XMLEvent<'_>) -> ParseResult<()>;
}

pub trait FromAttributes: Sized {
    fn from_attributes(attributes: AttributeMap<'_>) -> ParseResult<Self>;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseableState {
    #[default]
    Unopened,
    Open,
    Closed,
}

impl Display for CloseableState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CloseableState::Unopened => "unopened",
            CloseableState::Open => "open",
            CloseableState::Closed => "closed",
        })
    }
}

pub trait Closeable: Sized + HandleEvent {
    const KEY: &'static str;

    #[inline(always)]
    fn get_tag_key(&self) -> &'static str {
        Self::KEY
    }

    fn close_state(&self) -> CloseableState;

    #[inline(always)]
    fn is_open(&self) -> bool {
        self.close_state() == CloseableState::Open
    }

    fn close(&mut self) -> ParseResult<()>;
}

pub trait CloseableConstructor: Closeable + FromAttributes {
    fn new_open(attributes: AttributeMap<'_>) -> ParseResult<Self>;

    fn new_closed(attributes: AttributeMap<'_>) -> ParseResult<Self>;
}

impl<C: Closeable + FromAttributes> CloseableConstructor for C {
    #[inline(always)]
    fn new_open(attributes: AttributeMap<'_>) -> ParseResult<Self> {
        log::trace!("constructing open {}", std::any::type_name::<Self>());
        Self::from_attributes(attributes)
    }

    #[inline(always)]
    fn new_closed(attributes: AttributeMap<'_>) -> ParseResult<Self> {
        log::trace!("constructing closed {}", std::any::type_name::<Self>());
        let mut value = Self::new_open(attributes)?;
        value.close()?;
        Ok(value)
    }
}

/// Struct version of [`Closeable`].
#[derive(Debug, Default, Clone)]
pub enum Handle<D: HandleEvent + FromAttributes, const KEY: &'static str> {
    #[default]
    Unopened,
    Open(*mut D),
    Closed(D),
}

impl<D: HandleEvent + FromAttributes, const KEY: &'static str> Drop for Handle<D, KEY> {
    fn drop(&mut self) {
        if let Handle::Open(value) = self {
            if !value.is_null() {
                unsafe {
                    // SAFETY: value is uniquely owned by NestedHandle.
                    let mut ptr = std::ptr::null_mut();
                    std::mem::swap(value, &mut ptr);
                    std::alloc::dealloc(ptr as *mut u8, Layout::new::<D>());
                }
            }
        }
    }
}

impl<D: HandleEvent + FromAttributes, const KEY: &'static str> Handle<D, KEY> {
    pub fn new_open(mut data: D) -> Self {
        Handle::Open({
            let addr = std::ptr::addr_of_mut!(data);
            // Not a leak because we'll destroy pointee in Drop or make it
            // owned with NestedHandle::finish.
            std::mem::forget(data);
            addr
        })
    }

    pub fn value(&self) -> Option<&D> {
        match self {
            Handle::Closed(value) => Some(value),
            _ => None,
        }
    }

    pub fn partial_value(&self) -> Option<&D> {
        match self {
            Handle::Open(value) => unsafe {
                // SAFETY: HandleOpen always returns an initialized type
                Some(value.as_ref().expect("invalid Closeable state"))
            },
            Handle::Closed(value) => Some(value),
            _ => None,
        }
    }
}

impl<D: HandleEvent + FromAttributes, const KEY: &'static str> Closeable for Handle<D, KEY> {
    const KEY: &'static str = KEY;

    #[inline(always)]
    fn close_state(&self) -> CloseableState {
        match self {
            Handle::Unopened => CloseableState::Unopened,
            Handle::Open(_) => CloseableState::Open,
            Handle::Closed(_) => CloseableState::Closed,
        }
    }

    fn close(&mut self) -> ParseResult<()> {
        match self {
            Handle::Open(data) => {
                *self = Handle::Closed(unsafe {
                    // SAFETY: This function just changes the enum variant.
                    // Basically we're doing a swap of MaybeUninit value,
                    // only without Copy bound.
                    let mut value = std::ptr::null_mut();
                    std::mem::swap(data, &mut value);
                    value.read()
                })
            }
            _ => return Err(ParseError::BadCloseableState(self.close_state())),
        }
        Ok(())
    }
}

impl<D: HandleEvent + FromAttributes, const KEY: &'static str> FromAttributes for Handle<D, KEY> {
    fn from_attributes(attr: AttributeMap<'_>) -> ParseResult<Self> {
        D::from_attributes(attr).map(Handle::new_open)
    }
}

impl<D: HandleEvent + FromAttributes, const KEY: &'static str> HandleEvent for Handle<D, KEY> {
    fn handle_event(&mut self, event: XMLEvent<'_>) -> ParseResult<()> {
        match (self, event) {
            (handle, XMLEvent::End(end)) if handle.is_open() && end.name().0 == KEY.as_bytes() => {
                handle.close()
            }
            (Handle::Open(data), other) => unsafe {
                data.as_mut()
                    .expect("invalid Closeable variant value")
                    .handle_event(other)
            },
            (other, _) => Err(ParseError::BadCloseableState(other.close_state())),
        }
    }
}

fn is_formatting(tag: &XMLEvent<'_>) -> bool {
    const IGNORED: &[u8] = b"\x0A\x20";
    if let XMLEvent::Text(content) = tag {
        return content.iter().all(|it| IGNORED.contains(it));
    }
    false
}

#[derive(Debug, Default, Clone)]
pub enum XMLList<D: Closeable, const KEY: &'static str> {
    #[default]
    Unopened,
    Open(Vec<D>),
    Closed(Vec<D>),
}

impl<D: Closeable, const KEY: &'static str> XMLList<D, KEY> {
    pub fn new_open() -> Self {
        XMLList::Open(Vec::new())
    }

    pub fn value(&self) -> Option<&[D]> {
        match self {
            XMLList::Closed(value) => Some(value),
            _ => None,
        }
    }

    pub fn partial_value(&self) -> Option<&[D]> {
        match self {
            XMLList::Open(value) => Some(value),
            XMLList::Closed(value) => Some(value),
            _ => None,
        }
    }
}

impl<D: CloseableConstructor, const KEY: &'static str> Closeable for XMLList<D, KEY> {
    const KEY: &'static str = KEY;

    #[inline(always)]
    fn close_state(&self) -> CloseableState {
        match self {
            XMLList::Unopened => CloseableState::Unopened,
            XMLList::Open(_) => CloseableState::Open,
            XMLList::Closed(_) => CloseableState::Closed,
        }
    }

    fn close(&mut self) -> ParseResult<()> {
        match self {
            XMLList::Open(data) => {
                *self = XMLList::Closed({
                    let mut value = Vec::new();
                    std::mem::swap(data, &mut value);
                    value
                })
            }
            _ => return Err(ParseError::BadCloseableState(self.close_state())),
        }
        Ok(())
    }
}

impl<D: CloseableConstructor, const KEY: &'static str> FromAttributes for XMLList<D, KEY> {
    fn from_attributes(_: AttributeMap<'_>) -> ParseResult<Self> {
        ParseResult::Ok(XMLList::new_open())
    }
}

impl<D: CloseableConstructor, const KEY: &'static str> HandleEvent for XMLList<D, KEY> {
    fn handle_event(&mut self, event: XMLEvent<'_>) -> ParseResult<()> {
        match (self, event) {
            (list, event) if matches!(list, XMLList::Open(_)) => {
                let data = match list {
                    XMLList::Open(data) => data,
                    _ => unreachable!("invalid variant"), // checked by outer match
                };

                match (data.last_mut(), event) {
                    (Some(child), event) if child.is_open() => {
                        return child.handle_event(event);
                    }
                    (_, XMLEvent::End(end)) if end.name().0 == KEY.as_bytes() => {
                        return list.close()
                    }
                    (_, XMLEvent::Start(start)) if start.name().0 == D::KEY.as_bytes() => {
                        log::trace!(
                            "parsing non-empty XMLList child {}",
                            String::from_utf8_lossy(start.name().0)
                        );
                        match D::new_open(AttributeMap::of(&start)) {
                            Ok(value) => {
                                data.push(value);
                            }
                            Err(err) => return ParseResult::Err(err),
                        }
                    }
                    (_, XMLEvent::Empty(empty)) if empty.name().0 == D::KEY.as_bytes() => {
                        log::trace!(
                            "parsing empty XMLList child {}",
                            String::from_utf8_lossy(empty.name().0)
                        );
                        match D::new_closed(AttributeMap::of(&empty)) {
                            Ok(value) => {
                                data.push(value);
                            }
                            Err(err) => return ParseResult::Err(err),
                        }
                    }
                    (_, ev) if is_formatting(&ev) => {}
                    _ => {
                        return Err(ParseError::UnhandledEvent {
                            reason: "can't forward event to any XMLList children",
                        })
                    }
                }
            }
            (XMLList::Open(_), _) => unreachable!("reached already checked XMLList::Open variant"),
            (other, _) => return Err(ParseError::BadCloseableState(other.close_state())),
        }
        Ok(())
    }
}

pub trait ParseValue: Sized {
    fn parse(
        field: &'static str,
        attributes: &HashMap<String, String>,
        raw: &str,
    ) -> Result<Self, ValueError>;
}

impl<T: FromStr> ParseValue for T
where
    T::Err: std::error::Error,
    Result<T, T::Err>: FieldResultMap<T, ValueError>,
{
    fn parse(
        field: &'static str,
        _: &HashMap<String, String>,
        raw: &str,
    ) -> Result<Self, ValueError> {
        T::from_str(raw).map_field_err(field)
    }
}

#[derive(Debug, Default, Clone)]
pub enum ValueTag<D: ParseValue, const KEY: &'static str> {
    #[default]
    Unopened,
    Open {
        attributes: HashMap<String, String>,
        buffer: String,
    },
    Closed {
        attributes: HashMap<String, String>,
        value: D,
    },
}

impl<D: ParseValue, const KEY: &'static str> ValueTag<D, KEY> {
    pub fn value(&self) -> Option<&D> {
        match self {
            ValueTag::Closed { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn take_value(&mut self) -> Option<D> {
        match std::mem::take(self) {
            ValueTag::Closed { value, .. } => Some(value),
            _ => None,
        }
    }

    pub fn buffer(&self) -> Option<&str> {
        match self {
            ValueTag::Open { buffer, .. } => Some(buffer.as_str()),
            _ => None,
        }
    }

    pub fn buffer_mut(&mut self) -> Option<&mut str> {
        match self {
            ValueTag::Open { buffer, .. } => Some(buffer.as_mut_str()),
            _ => None,
        }
    }

    pub fn attributes(&self) -> Option<&HashMap<String, String>> {
        Some(match self {
            ValueTag::Open { attributes, .. } | ValueTag::Closed { attributes, .. } => attributes,
            ValueTag::Unopened => return None,
        })
    }
}

impl<D: ParseValue, const KEY: &'static str> FromAttributes for ValueTag<D, KEY> {
    fn from_attributes(attributes: AttributeMap<'_>) -> ParseResult<Self> {
        Ok(ValueTag::Open {
            attributes: attributes.into_hashmap()?,
            buffer: String::with_capacity(4),
        })
    }
}

impl<D: ParseValue, const KEY: &'static str> HandleEvent for ValueTag<D, KEY> {
    fn handle_event(&mut self, event: XMLEvent<'_>) -> ParseResult<()> {
        match event {
            XMLEvent::End(end) if end.name().0 == KEY.as_bytes() => {
                return self.close();
            }
            XMLEvent::Text(text) => match self {
                ValueTag::Open { buffer, .. } => {
                    buffer.push_str(std::str::from_utf8(&text)?);
                }
                other => return Err(ParseError::BadCloseableState(other.close_state())),
            },
            XMLEvent::CData(cdata) => match self {
                ValueTag::Open { buffer, .. } => {
                    buffer.push_str(std::str::from_utf8(&cdata)?);
                }
                other => return Err(ParseError::BadCloseableState(other.close_state())),
            },
            XMLEvent::Comment(_) => {}
            XMLEvent::Eof => return Err(ParseError::BadCloseableState(self.close_state())),
            other => panic!(
                "value tag '{}' doesn't support nested tag: {:?}",
                KEY, other
            ),
        }
        Ok(())
    }
}

impl<D: ParseValue, const KEY: &'static str> Closeable for ValueTag<D, KEY> {
    const KEY: &'static str = KEY;

    #[inline(always)]
    fn close_state(&self) -> CloseableState {
        match self {
            ValueTag::Unopened => CloseableState::Unopened,
            ValueTag::Open { .. } => CloseableState::Open,
            ValueTag::Closed { .. } => CloseableState::Closed,
        }
    }

    fn close(&mut self) -> ParseResult<()> {
        let (value, attributes) = match self {
            ValueTag::Open { buffer, attributes } => {
                (D::parse(KEY, attributes, buffer)?, attributes)
            }
            other => return Err(ParseError::BadCloseableState(other.close_state())),
        };
        *self = ValueTag::Closed {
            attributes: std::mem::take(attributes),
            value,
        };
        Ok(())
    }
}

#[macro_export]
macro_rules! forward_closeable {
    ($tag_value: expr => [$($entry: expr),+ $(,)?]) => {
        $(
            if $entry.is_open() {
                log::trace!(concat![stringify!($entry), " closeable is open; forwarding event"]);
                return $entry.handle_event($tag_value);
            }
        )+
    };
}
#[macro_export]
macro_rules! start_closeable {
    ($tag_value: expr => [$($entry: expr),+ $(,)?]) => {
        $(
            if $tag_value.name().0 == $entry.get_tag_key().as_bytes() {
                $entry = CloseableConstructor::new_open(AttributeMap::of(&$tag_value))?;
                return Ok(());
            }
        )+
    };
}
#[macro_export]
macro_rules! empty_closeable {
    ($tag_value: expr => [$($entry: expr),+ $(,)?]) => {
        $(
            if $tag_value.name().0 == $entry.get_tag_key().as_bytes() {
                $entry = CloseableConstructor::new_closed(AttributeMap::of(&$tag_value))?;
                return Ok(());
            }
        )+
    };
}
#[macro_export]
macro_rules! close_all_nested {
    ($($entry: expr),+ $(,)?) => {
        $(
            if $entry.is_open() {
                $entry.close()?;
            }
        )+
    };
}
#[macro_export]
macro_rules! impl_forwarding_closeable_handler {
    {$target: ty as $alias: ident => [$($entry: expr),+ $(,)?] or { match $ev: tt $fallthrough: tt }} => {
        impl HandleEvent for $target {
            fn handle_event(&mut self, $ev: XMLEvent<'_>) -> ParseResult<()> {
                log::trace!("{} handling event: {:?}", std::any::type_name::<$target>(), $ev);
                let $alias = self;
                match $ev {
                    XMLEvent::Start(tag) => {
                        $crate::forward_closeable!(XMLEvent::Start(tag) => [
                            $($entry),+
                        ]);
                        $crate::start_closeable!(tag => [
                            $($entry),+
                        ]);
                        match XMLEvent::Start(tag) $fallthrough
                    }
                    XMLEvent::Empty(tag) => {
                        forward_closeable!(XMLEvent::Empty(tag) => [
                            $($entry),+
                        ]);
                        $crate::empty_closeable!(tag => [
                            $($entry),+
                        ]);
                        match XMLEvent::Empty(tag) $fallthrough
                    }
                    XMLEvent::End(tag) => {
                        $crate::forward_closeable!(XMLEvent::End(tag) => [
                            $($entry),+
                        ]);
                        if tag.name().0 == $alias.get_tag_key().as_bytes() {
                            return $alias.close();
                        }
                        match XMLEvent::End(tag) $fallthrough
                    }
                    other_tag_value_ => {
                        $crate::forward_closeable!(other_tag_value_ => [
                            $($entry),+
                        ]);
                        match other_tag_value_ $fallthrough
                    }
                }
                Ok(())
            }
        }
    };
    {$target: ty as $alias: ident => [$($entry: expr),+ $(,)?]} => {
        impl_forwarding_closeable_handler!($target as $alias => [$($entry),+] or {
            match event {
                _ => {}
            }
        });
    };
}
