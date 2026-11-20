// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Typed requests and responses on top of the raw codec.

use super::codec::{DecodeError, Message};
use super::status::{Operation, StatusCode};
use super::value::{Attribute, AttributeSet, Group, GroupTag, StringKind, Value};

/// IPP version sent by default (PWG 5100.12, IPP/2.0).
pub const DEFAULT_VERSION: (u8, u8) = (2, 0);
/// Fallback after server-error-version-not-supported (RFC 8011 section 4.1.8).
pub const FALLBACK_VERSION: (u8, u8) = (1, 1);

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub operation: Operation,
    pub version: (u8, u8),
    pub request_id: u32,
    operation_attrs: Vec<Attribute>,
    extra_groups: Vec<Group>,
    document: Vec<u8>,
}

impl Request {
    /// Starts a request with attributes-charset and attributes-natural-language
    /// first, as RFC 8011 section 4.1.4 requires.
    pub fn new(operation: Operation) -> Self {
        Self {
            operation,
            version: DEFAULT_VERSION,
            request_id: 1,
            operation_attrs: vec![
                Attribute::new(
                    "attributes-charset",
                    Value::String(StringKind::Charset, "utf-8".into()),
                ),
                Attribute::new(
                    "attributes-natural-language",
                    Value::String(StringKind::NaturalLanguage, "en".into()),
                ),
            ],
            extra_groups: Vec::new(),
            document: Vec::new(),
        }
    }

    /// Target printer-uri (RFC 8011 section 4.1.5).
    pub fn printer_uri(self, uri: &str) -> Self {
        self.op_attr(Attribute::new("printer-uri", Value::uri(uri)))
    }

    pub fn user(self, name: &str) -> Self {
        self.op_attr(Attribute::new("requesting-user-name", Value::name(name)))
    }

    pub fn requested_attributes(self, names: &[&str]) -> Self {
        let values = names.iter().map(|n| Value::keyword(*n)).collect();
        self.op_attr(Attribute::with_values("requested-attributes", values))
    }

    pub fn op_attr(mut self, a: Attribute) -> Self {
        self.operation_attrs.push(a);
        self
    }

    pub fn group(mut self, tag: GroupTag, attributes: Vec<Attribute>) -> Self {
        self.extra_groups.push(Group { tag, attributes });
        self
    }

    pub fn document(mut self, data: Vec<u8>) -> Self {
        self.document = data;
        self
    }

    pub fn with_version(mut self, version: (u8, u8)) -> Self {
        self.version = version;
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut groups = Vec::with_capacity(1 + self.extra_groups.len());
        groups.push(Group {
            tag: GroupTag::Operation,
            attributes: self.operation_attrs.clone(),
        });
        groups.extend(self.extra_groups.iter().cloned());
        Message {
            version: self.version,
            code: self.operation.0,
            request_id: self.request_id,
            groups,
            data: self.document.clone(),
        }
        .encode()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub version: (u8, u8),
    pub status: StatusCode,
    pub request_id: u32,
    pub groups: Vec<Group>,
}

impl Response {
    pub fn decode(buf: &[u8]) -> Result<Self, DecodeError> {
        let m = Message::decode(buf)?;
        Ok(Self {
            version: m.version,
            status: StatusCode(m.code),
            request_id: m.request_id,
            groups: m.groups,
        })
    }

    pub fn groups_of(&self, tag: GroupTag) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(move |g| g.tag == tag)
    }

    pub fn first_group(&self, tag: GroupTag) -> Option<&Group> {
        self.groups_of(tag).next()
    }

    /// "status-message" operation attribute (RFC 8011 section 4.1.6.2).
    pub fn status_message(&self) -> Option<&str> {
        self.first_group(GroupTag::Operation)
            .and_then(|g| g.str_of("status-message"))
    }
}
