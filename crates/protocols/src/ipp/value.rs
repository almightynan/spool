// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! IPP attribute values and tags (RFC 8010 section 3.5).

/// Delimiter tags that start an attribute group (RFC 8010 section 3.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupTag {
    Operation,
    Job,
    Printer,
    Unsupported,
    Subscription,
    EventNotification,
    Resource,
    Document,
    System,
    /// A delimiter value this crate does not name, kept so decoding never fails on it.
    Other(u8),
}

impl GroupTag {
    pub const END_OF_ATTRIBUTES: u8 = 0x03;

    pub fn from_byte(b: u8) -> Self {
        match b {
            0x01 => Self::Operation,
            0x02 => Self::Job,
            0x04 => Self::Printer,
            0x05 => Self::Unsupported,
            0x06 => Self::Subscription,
            0x07 => Self::EventNotification,
            0x08 => Self::Resource,
            0x09 => Self::Document,
            0x0A => Self::System,
            other => Self::Other(other),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            Self::Operation => 0x01,
            Self::Job => 0x02,
            Self::Printer => 0x04,
            Self::Unsupported => 0x05,
            Self::Subscription => 0x06,
            Self::EventNotification => 0x07,
            Self::Resource => 0x08,
            Self::Document => 0x09,
            Self::System => 0x0A,
            Self::Other(b) => b,
        }
    }
}

/// Value tags (RFC 8010 section 3.5.2).
pub mod tag {
    pub const UNSUPPORTED: u8 = 0x10;
    pub const UNKNOWN: u8 = 0x12;
    pub const NO_VALUE: u8 = 0x13;
    pub const NOT_SETTABLE: u8 = 0x15;
    pub const DELETE_ATTRIBUTE: u8 = 0x16;
    pub const ADMIN_DEFINE: u8 = 0x17;
    pub const INTEGER: u8 = 0x21;
    pub const BOOLEAN: u8 = 0x22;
    pub const ENUM: u8 = 0x23;
    pub const OCTET_STRING: u8 = 0x30;
    pub const DATE_TIME: u8 = 0x31;
    pub const RESOLUTION: u8 = 0x32;
    pub const RANGE_OF_INTEGER: u8 = 0x33;
    pub const BEG_COLLECTION: u8 = 0x34;
    pub const TEXT_WITH_LANGUAGE: u8 = 0x35;
    pub const NAME_WITH_LANGUAGE: u8 = 0x36;
    pub const END_COLLECTION: u8 = 0x37;
    pub const TEXT: u8 = 0x41;
    pub const NAME: u8 = 0x42;
    pub const KEYWORD: u8 = 0x44;
    pub const URI: u8 = 0x45;
    pub const URI_SCHEME: u8 = 0x46;
    pub const CHARSET: u8 = 0x47;
    pub const NATURAL_LANGUAGE: u8 = 0x48;
    pub const MIME_MEDIA_TYPE: u8 = 0x49;
    pub const MEMBER_ATTR_NAME: u8 = 0x4A;
    pub const EXTENSION: u8 = 0x7F;
}

/// Out-of-band values (RFC 8010 section 3.5.2, 0x10 to 0x1F).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutOfBand {
    Unsupported,
    Unknown,
    NoValue,
    NotSettable,
    DeleteAttribute,
    AdminDefine,
    Other(u8),
}

impl OutOfBand {
    pub fn from_tag(t: u8) -> Self {
        match t {
            tag::UNSUPPORTED => Self::Unsupported,
            tag::UNKNOWN => Self::Unknown,
            tag::NO_VALUE => Self::NoValue,
            tag::NOT_SETTABLE => Self::NotSettable,
            tag::DELETE_ATTRIBUTE => Self::DeleteAttribute,
            tag::ADMIN_DEFINE => Self::AdminDefine,
            other => Self::Other(other),
        }
    }

    pub fn tag(self) -> u8 {
        match self {
            Self::Unsupported => tag::UNSUPPORTED,
            Self::Unknown => tag::UNKNOWN,
            Self::NoValue => tag::NO_VALUE,
            Self::NotSettable => tag::NOT_SETTABLE,
            Self::DeleteAttribute => tag::DELETE_ATTRIBUTE,
            Self::AdminDefine => tag::ADMIN_DEFINE,
            Self::Other(t) => t,
        }
    }
}

/// String-valued tags share one representation; the tag says which syntax it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    Text,
    Name,
    Keyword,
    Uri,
    UriScheme,
    Charset,
    NaturalLanguage,
    MimeMediaType,
}

impl StringKind {
    pub fn from_tag(t: u8) -> Option<Self> {
        Some(match t {
            tag::TEXT => Self::Text,
            tag::NAME => Self::Name,
            tag::KEYWORD => Self::Keyword,
            tag::URI => Self::Uri,
            tag::URI_SCHEME => Self::UriScheme,
            tag::CHARSET => Self::Charset,
            tag::NATURAL_LANGUAGE => Self::NaturalLanguage,
            tag::MIME_MEDIA_TYPE => Self::MimeMediaType,
            _ => return None,
        })
    }

    pub fn tag(self) -> u8 {
        match self {
            Self::Text => tag::TEXT,
            Self::Name => tag::NAME,
            Self::Keyword => tag::KEYWORD,
            Self::Uri => tag::URI,
            Self::UriScheme => tag::URI_SCHEME,
            Self::Charset => tag::CHARSET,
            Self::NaturalLanguage => tag::NATURAL_LANGUAGE,
            Self::MimeMediaType => tag::MIME_MEDIA_TYPE,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Enum(i32),
    OctetString(Vec<u8>),
    /// RFC 2579 DateAndTime, 11 octets.
    DateTime([u8; 11]),
    Resolution {
        cross_feed: i32,
        feed: i32,
        units: i8,
    },
    Range {
        lower: i32,
        upper: i32,
    },
    /// textWithLanguage or nameWithLanguage.
    WithLanguage {
        is_name: bool,
        language: String,
        text: String,
    },
    String(StringKind, String),
    Collection(Vec<Attribute>),
    OutOfBand(OutOfBand),
    /// Any other tag, kept as raw bytes.
    Unknown {
        tag: u8,
        data: Vec<u8>,
    },
}

impl Value {
    pub fn keyword(s: impl Into<String>) -> Self {
        Self::String(StringKind::Keyword, s.into())
    }
    pub fn uri(s: impl Into<String>) -> Self {
        Self::String(StringKind::Uri, s.into())
    }
    pub fn name(s: impl Into<String>) -> Self {
        Self::String(StringKind::Name, s.into())
    }
    pub fn text(s: impl Into<String>) -> Self {
        Self::String(StringKind::Text, s.into())
    }
    pub fn mime(s: impl Into<String>) -> Self {
        Self::String(StringKind::MimeMediaType, s.into())
    }

    /// String content for any string-like value (including the WithLanguage forms
    /// and octetStrings that hold UTF-8, which PWG 5100.13 uses for printer-input-tray).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(_, s) => Some(s),
            Self::WithLanguage { text, .. } => Some(text),
            Self::OctetString(b) => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }

    /// Integer content for integer and enum values.
    pub fn as_int(&self) -> Option<i32> {
        match self {
            Self::Integer(i) | Self::Enum(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_collection(&self) -> Option<&[Attribute]> {
        match self {
            Self::Collection(c) => Some(c),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub values: Vec<Value>,
}

impl Attribute {
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            values: vec![value],
        }
    }

    pub fn with_values(name: impl Into<String>, values: Vec<Value>) -> Self {
        Self {
            name: name.into(),
            values,
        }
    }

    pub fn first(&self) -> Option<&Value> {
        self.values.first()
    }

    pub fn first_str(&self) -> Option<&str> {
        self.first().and_then(Value::as_str)
    }

    pub fn first_int(&self) -> Option<i32> {
        self.first().and_then(Value::as_int)
    }

    pub fn first_bool(&self) -> Option<bool> {
        self.first().and_then(Value::as_bool)
    }

    pub fn strs(&self) -> impl Iterator<Item = &str> {
        self.values.iter().filter_map(Value::as_str)
    }

    pub fn ints(&self) -> impl Iterator<Item = i32> + '_ {
        self.values.iter().filter_map(Value::as_int)
    }
}

/// Lookup helpers shared by groups and collections.
pub trait AttributeSet {
    fn attributes(&self) -> &[Attribute];

    fn get(&self, name: &str) -> Option<&Attribute> {
        self.attributes().iter().find(|a| a.name == name)
    }

    fn str_of(&self, name: &str) -> Option<&str> {
        self.get(name).and_then(Attribute::first_str)
    }

    fn int_of(&self, name: &str) -> Option<i32> {
        self.get(name).and_then(Attribute::first_int)
    }

    fn bool_of(&self, name: &str) -> Option<bool> {
        self.get(name).and_then(Attribute::first_bool)
    }

    fn strs_of(&self, name: &str) -> Vec<&str> {
        self.get(name)
            .map(|a| a.strs().collect())
            .unwrap_or_default()
    }

    fn ints_of(&self, name: &str) -> Vec<i32> {
        self.get(name)
            .map(|a| a.ints().collect())
            .unwrap_or_default()
    }

    fn collection_of(&self, name: &str) -> Option<&[Attribute]> {
        self.get(name)
            .and_then(Attribute::first)
            .and_then(Value::as_collection)
    }
}

impl AttributeSet for [Attribute] {
    fn attributes(&self) -> &[Attribute] {
        self
    }
}

impl AttributeSet for Vec<Attribute> {
    fn attributes(&self) -> &[Attribute] {
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub tag: GroupTag,
    pub attributes: Vec<Attribute>,
}

impl Group {
    pub fn new(tag: GroupTag) -> Self {
        Self {
            tag,
            attributes: Vec::new(),
        }
    }

    pub fn push(&mut self, attr: Attribute) {
        self.attributes.push(attr);
    }
}

impl AttributeSet for Group {
    fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }
}
