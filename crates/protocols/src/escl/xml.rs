// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! A small element tree for eSCL documents.
//!
//! eSCL documents are a few kilobytes, so a tree is simpler and safer than
//! streaming. Elements are matched by local name: devices use different
//! prefixes for the same namespaces, and eSCL element names do not collide
//! across the scan and pwg namespaces.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Element {
    pub name: String,
    pub text: String,
    pub children: Vec<Element>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlError {
    Syntax(String),
    NoRoot,
    UnexpectedRoot {
        expected: &'static str,
        found: String,
    },
    TooDeep,
    Missing(&'static str),
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(e) => write!(f, "invalid XML: {e}"),
            Self::NoRoot => write!(f, "XML document has no root element"),
            Self::UnexpectedRoot { expected, found } => {
                write!(f, "expected <{expected}> but found <{found}>")
            }
            Self::TooDeep => write!(f, "XML nesting is too deep"),
            Self::Missing(n) => write!(f, "required element <{n}> is missing"),
        }
    }
}

impl std::error::Error for XmlError {}

const MAX_DEPTH: usize = 32;

impl Element {
    pub fn parse(xml: &str) -> Result<Self, XmlError> {
        // No trim_text: trimming per event would drop spaces around entity
        // references. Readers trim the joined text instead.
        let mut reader = Reader::from_str(xml);
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;
        loop {
            let ev = reader
                .read_event()
                .map_err(|e| XmlError::Syntax(e.to_string()))?;
            match ev {
                Event::Start(s) => {
                    if stack.len() >= MAX_DEPTH {
                        return Err(XmlError::TooDeep);
                    }
                    stack.push(Element {
                        name: s.local_name().as_ref().to_string(),
                        ..Default::default()
                    });
                }
                Event::Empty(s) => {
                    let e = Element {
                        name: s.local_name().as_ref().to_string(),
                        ..Default::default()
                    };
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(e),
                        None => root = root.or(Some(e)),
                    }
                }
                Event::Text(t) => {
                    if let Some(top) = stack.last_mut() {
                        top.text.push_str(&t.xml10_content());
                    }
                }
                Event::CData(t) => {
                    if let Some(top) = stack.last_mut() {
                        top.text.push_str(&t.xml10_content());
                    }
                }
                Event::GeneralRef(r) => {
                    if let Some(top) = stack.last_mut() {
                        if r.is_char_ref() {
                            if let Ok(Some(c)) = r.resolve_char_ref() {
                                top.text.push(c);
                            }
                        } else if let Some(s) = quick_xml::escape::resolve_predefined_entity(&r) {
                            top.text.push_str(s);
                        }
                    }
                }
                Event::End(_) => {
                    let Some(done) = stack.pop() else {
                        return Err(XmlError::Syntax("unbalanced end tag".into()));
                    };
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(done),
                        None => root = root.or(Some(done)),
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        if !stack.is_empty() {
            return Err(XmlError::Syntax("document ends inside an element".into()));
        }
        root.ok_or(XmlError::NoRoot)
    }

    pub fn expect_root(self, name: &'static str) -> Result<Self, XmlError> {
        if self.name == name {
            Ok(self)
        } else {
            Err(XmlError::UnexpectedRoot {
                expected: name,
                found: self.name,
            })
        }
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }

    pub fn child_u32(&self, name: &str) -> Option<u32> {
        self.child_text(name)?.parse().ok()
    }

    /// Follows a path of child names.
    pub fn path(&self, names: &[&str]) -> Option<&Element> {
        let mut cur = self;
        for n in names {
            cur = cur.child(n)?;
        }
        Some(cur)
    }
}

/// Escapes text for element content.
pub fn escape(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixes_entities_and_empty_elements() {
        let e = Element::parse(
            "<?xml version=\"1.0\"?><a:Root xmlns:a=\"x\"><a:B>1 &amp; 2&#33;</a:B><C/></a:Root>",
        )
        .unwrap();
        assert_eq!(e.name, "Root");
        assert_eq!(e.child_text("B"), Some("1 & 2!"));
        assert!(e.child("C").is_some());
    }

    #[test]
    fn rejects_broken_documents() {
        assert!(Element::parse("<a><b></a>").is_err());
        assert!(Element::parse("<a>").is_err());
        assert_eq!(Element::parse(""), Err(XmlError::NoRoot));
        let deep = "<a>".repeat(40) + &"</a>".repeat(40);
        assert_eq!(Element::parse(&deep), Err(XmlError::TooDeep));
    }
}
