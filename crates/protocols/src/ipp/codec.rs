// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Binary encoding of IPP messages (RFC 8010 section 3).

use super::value::{tag, Attribute, Group, GroupTag, OutOfBand, StringKind, Value};
use std::fmt;

/// A raw IPP message: request or response, before interpretation.
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub version: (u8, u8),
    /// operation-id for requests, status-code for responses.
    pub code: u16,
    pub request_id: u32,
    pub groups: Vec<Group>,
    /// Data after the end-of-attributes tag (a document in requests).
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The input ended before the structure was complete.
    Truncated { offset: usize },
    /// An attribute value appeared before any group delimiter.
    ValueOutsideGroup { offset: usize },
    /// An additional value (empty name) had no attribute before it.
    OrphanValue { offset: usize },
    /// A fixed-size value had the wrong length.
    BadLength { offset: usize, tag: u8, len: usize },
    /// Collection structure did not follow RFC 8010 section 3.1.6.
    BadCollection { offset: usize },
    /// Nesting deeper than [`MAX_COLLECTION_DEPTH`].
    TooDeep,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { offset } => write!(f, "IPP message ends early at byte {offset}"),
            Self::ValueOutsideGroup { offset } => {
                write!(f, "IPP attribute outside any group at byte {offset}")
            }
            Self::OrphanValue { offset } => {
                write!(f, "IPP additional value without attribute at byte {offset}")
            }
            Self::BadLength { offset, tag, len } => write!(
                f,
                "IPP value with tag 0x{tag:02x} has invalid length {len} at byte {offset}"
            ),
            Self::BadCollection { offset } => {
                write!(f, "IPP collection is malformed at byte {offset}")
            }
            Self::TooDeep => write!(f, "IPP collections are nested too deeply"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Printers do not nest collections deeply; this bounds recursion on hostile input.
pub const MAX_COLLECTION_DEPTH: usize = 16;

// ---------- encoding ----------

impl Message {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256 + self.data.len());
        out.push(self.version.0);
        out.push(self.version.1);
        out.extend_from_slice(&self.code.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());
        for g in &self.groups {
            out.push(g.tag.to_byte());
            for a in &g.attributes {
                encode_attribute(&mut out, &a.name, &a.values);
            }
        }
        out.push(GroupTag::END_OF_ATTRIBUTES);
        out.extend_from_slice(&self.data);
        out
    }
}

fn put_len_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    // Lengths are 16 bit (RFC 8010 section 3.1.4). Longer values are truncated
    // rather than producing a corrupt length field.
    let len = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(&bytes[..len]);
}

fn encode_attribute(out: &mut Vec<u8>, name: &str, values: &[Value]) {
    for (i, v) in values.iter().enumerate() {
        let n = if i == 0 { name } else { "" };
        encode_value(out, n, v);
    }
}

fn encode_value(out: &mut Vec<u8>, name: &str, v: &Value) {
    let (t, body): (u8, Vec<u8>) = match v {
        Value::Integer(i) => (tag::INTEGER, i.to_be_bytes().to_vec()),
        Value::Enum(i) => (tag::ENUM, i.to_be_bytes().to_vec()),
        Value::Boolean(b) => (tag::BOOLEAN, vec![u8::from(*b)]),
        Value::OctetString(b) => (tag::OCTET_STRING, b.clone()),
        Value::DateTime(d) => (tag::DATE_TIME, d.to_vec()),
        Value::Resolution {
            cross_feed,
            feed,
            units,
        } => {
            let mut b = Vec::with_capacity(9);
            b.extend_from_slice(&cross_feed.to_be_bytes());
            b.extend_from_slice(&feed.to_be_bytes());
            b.push(*units as u8);
            (tag::RESOLUTION, b)
        }
        Value::Range { lower, upper } => {
            let mut b = Vec::with_capacity(8);
            b.extend_from_slice(&lower.to_be_bytes());
            b.extend_from_slice(&upper.to_be_bytes());
            (tag::RANGE_OF_INTEGER, b)
        }
        Value::WithLanguage {
            is_name,
            language,
            text,
        } => {
            let mut b = Vec::new();
            put_len_bytes(&mut b, language.as_bytes());
            put_len_bytes(&mut b, text.as_bytes());
            let t = if *is_name {
                tag::NAME_WITH_LANGUAGE
            } else {
                tag::TEXT_WITH_LANGUAGE
            };
            (t, b)
        }
        Value::String(kind, s) => (kind.tag(), s.as_bytes().to_vec()),
        Value::OutOfBand(o) => (o.tag(), Vec::new()),
        Value::Unknown { tag: t, data } => (*t, data.clone()),
        Value::Collection(members) => {
            out.push(tag::BEG_COLLECTION);
            put_len_bytes(out, name.as_bytes());
            put_len_bytes(out, &[]);
            for m in members {
                out.push(tag::MEMBER_ATTR_NAME);
                put_len_bytes(out, &[]);
                put_len_bytes(out, m.name.as_bytes());
                for mv in &m.values {
                    encode_value(out, "", mv);
                }
            }
            out.push(tag::END_COLLECTION);
            put_len_bytes(out, &[]);
            put_len_bytes(out, &[]);
            return;
        }
    };
    out.push(t);
    put_len_bytes(out, name.as_bytes());
    put_len_bytes(out, &body);
}

// ---------- decoding ----------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(DecodeError::Truncated { offset: self.pos })?;
        let s = self
            .buf
            .get(self.pos..end)
            .ok_or(DecodeError::Truncated { offset: self.pos })?;
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    fn len_bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.u16()? as usize;
        self.take(n)
    }
}

/// One "attribute-with-one-value" or "additional-value" record (RFC 8010 section 3.1.3).
struct RawValue<'a> {
    offset: usize,
    tag: u8,
    name: &'a [u8],
    data: &'a [u8],
}

fn read_raw<'a>(r: &mut Reader<'a>) -> Result<RawValue<'a>, DecodeError> {
    let offset = r.pos;
    let tag = r.u8()?;
    let name = r.len_bytes()?;
    let data = r.len_bytes()?;
    Ok(RawValue {
        offset,
        tag,
        name,
        data,
    })
}

fn i32_at(d: &[u8], at: usize) -> i32 {
    i32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn need_len(raw: &RawValue<'_>, len: usize) -> Result<(), DecodeError> {
    if raw.data.len() == len {
        Ok(())
    } else {
        Err(DecodeError::BadLength {
            offset: raw.offset,
            tag: raw.tag,
            len: raw.data.len(),
        })
    }
}

/// Decodes a simple (non-collection) value.
fn simple_value(raw: &RawValue<'_>) -> Result<Value, DecodeError> {
    let d = raw.data;
    Ok(match raw.tag {
        tag::INTEGER => {
            need_len(raw, 4)?;
            Value::Integer(i32_at(d, 0))
        }
        tag::ENUM => {
            need_len(raw, 4)?;
            Value::Enum(i32_at(d, 0))
        }
        tag::BOOLEAN => {
            need_len(raw, 1)?;
            Value::Boolean(d[0] != 0)
        }
        tag::OCTET_STRING => Value::OctetString(d.to_vec()),
        tag::DATE_TIME => {
            need_len(raw, 11)?;
            let mut a = [0u8; 11];
            a.copy_from_slice(d);
            Value::DateTime(a)
        }
        tag::RESOLUTION => {
            need_len(raw, 9)?;
            Value::Resolution {
                cross_feed: i32_at(d, 0),
                feed: i32_at(d, 4),
                units: d[8] as i8,
            }
        }
        tag::RANGE_OF_INTEGER => {
            need_len(raw, 8)?;
            Value::Range {
                lower: i32_at(d, 0),
                upper: i32_at(d, 4),
            }
        }
        tag::TEXT_WITH_LANGUAGE | tag::NAME_WITH_LANGUAGE => {
            let mut r = Reader { buf: d, pos: 0 };
            let bad = DecodeError::BadLength {
                offset: raw.offset,
                tag: raw.tag,
                len: d.len(),
            };
            let language = r.len_bytes().map_err(|_| bad.clone())?;
            let text = r.len_bytes().map_err(|_| bad)?;
            Value::WithLanguage {
                is_name: raw.tag == tag::NAME_WITH_LANGUAGE,
                language: lossy(language),
                text: lossy(text),
            }
        }
        t if (0x10..=0x1F).contains(&t) => Value::OutOfBand(OutOfBand::from_tag(t)),
        t => match StringKind::from_tag(t) {
            Some(kind) => Value::String(kind, lossy(d)),
            None => Value::Unknown {
                tag: t,
                data: d.to_vec(),
            },
        },
    })
}

/// Reads collection members after a begCollection value (RFC 8010 section 3.1.6).
fn read_collection(r: &mut Reader<'_>, depth: usize) -> Result<Vec<Attribute>, DecodeError> {
    if depth > MAX_COLLECTION_DEPTH {
        return Err(DecodeError::TooDeep);
    }
    let mut members: Vec<Attribute> = Vec::new();
    loop {
        let raw = read_raw(r)?;
        match raw.tag {
            tag::END_COLLECTION => return Ok(members),
            tag::MEMBER_ATTR_NAME => {
                members.push(Attribute {
                    name: lossy(raw.data),
                    values: Vec::new(),
                });
            }
            tag::BEG_COLLECTION => {
                let inner = read_collection(r, depth + 1)?;
                let m = members
                    .last_mut()
                    .ok_or(DecodeError::BadCollection { offset: raw.offset })?;
                m.values.push(Value::Collection(inner));
            }
            _ => {
                let v = simple_value(&raw)?;
                let m = members
                    .last_mut()
                    .ok_or(DecodeError::BadCollection { offset: raw.offset })?;
                m.values.push(v);
            }
        }
    }
}

impl Message {
    pub fn decode(buf: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader { buf, pos: 0 };
        let major = r.u8()?;
        let minor = r.u8()?;
        let code = r.u16()?;
        let request_id = r.u32()?;
        let mut groups: Vec<Group> = Vec::new();
        loop {
            let t = r.peek().ok_or(DecodeError::Truncated { offset: r.pos })?;
            if t == GroupTag::END_OF_ATTRIBUTES {
                r.pos += 1;
                break;
            }
            if t < 0x10 {
                r.pos += 1;
                groups.push(Group::new(GroupTag::from_byte(t)));
                continue;
            }
            let raw = read_raw(&mut r)?;
            let group = groups
                .last_mut()
                .ok_or(DecodeError::ValueOutsideGroup { offset: raw.offset })?;
            let value = if raw.tag == tag::BEG_COLLECTION {
                Value::Collection(read_collection(&mut r, 1)?)
            } else {
                simple_value(&raw)?
            };
            if raw.name.is_empty() {
                let last = group
                    .attributes
                    .last_mut()
                    .ok_or(DecodeError::OrphanValue { offset: raw.offset })?;
                last.values.push(value);
            } else {
                group.attributes.push(Attribute {
                    name: lossy(raw.name),
                    values: vec![value],
                });
            }
        }
        Ok(Self {
            version: (major, minor),
            code,
            request_id,
            groups,
            data: buf[r.pos..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Message {
        let mut op = Group::new(GroupTag::Operation);
        op.push(Attribute::new(
            "attributes-charset",
            Value::String(StringKind::Charset, "utf-8".into()),
        ));
        op.push(Attribute::with_values(
            "requested-attributes",
            vec![
                Value::keyword("printer-state"),
                Value::keyword("marker-levels"),
            ],
        ));
        let mut job = Group::new(GroupTag::Job);
        job.push(Attribute::new(
            "media-col",
            Value::Collection(vec![
                Attribute::new(
                    "media-size",
                    Value::Collection(vec![
                        Attribute::new("x-dimension", Value::Integer(21000)),
                        Attribute::new("y-dimension", Value::Integer(29700)),
                    ]),
                ),
                Attribute::new("media-source", Value::keyword("main")),
            ]),
        ));
        job.push(Attribute::new(
            "printer-resolution",
            Value::Resolution {
                cross_feed: 600,
                feed: 600,
                units: 3,
            },
        ));
        job.push(Attribute::new(
            "job-name",
            Value::WithLanguage {
                is_name: true,
                language: "en".into(),
                text: "Report".into(),
            },
        ));
        job.push(Attribute::new(
            "copies",
            Value::Range {
                lower: 1,
                upper: 99,
            },
        ));
        job.push(Attribute::new("x", Value::OutOfBand(OutOfBand::NoValue)));
        Message {
            version: (2, 0),
            code: 0x000B,
            request_id: 7,
            groups: vec![op, job],
            data: b"doc".to_vec(),
        }
    }

    #[test]
    fn round_trip() {
        let m = sample();
        let bytes = m.encode();
        let back = Message::decode(&bytes).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn truncated_input_is_an_error_at_every_cut() {
        let bytes = sample().encode();
        // Every prefix shorter than the end-of-attributes tag must fail cleanly.
        let end = bytes.len() - 3;
        for cut in 0..end {
            assert!(Message::decode(&bytes[..cut]).is_err(), "cut {cut}");
        }
    }

    #[test]
    fn value_before_group_is_rejected() {
        let bytes = [
            2, 0, 0, 0, 0, 0, 0, 1, 0x21, 0, 1, b'a', 0, 4, 0, 0, 0, 1, 3,
        ];
        assert_eq!(
            Message::decode(&bytes),
            Err(DecodeError::ValueOutsideGroup { offset: 8 })
        );
    }

    #[test]
    fn bad_integer_length_is_rejected() {
        let bytes = [2, 0, 0, 0, 0, 0, 0, 1, 1, 0x21, 0, 1, b'a', 0, 2, 0, 0, 3];
        assert!(matches!(
            Message::decode(&bytes),
            Err(DecodeError::BadLength { len: 2, .. })
        ));
    }

    #[test]
    fn deep_collections_are_bounded() {
        let mut bytes = vec![2, 0, 0, 0, 0, 0, 0, 1, 1];
        bytes.extend_from_slice(&[0x34, 0, 1, b'c', 0, 0]);
        for _ in 0..40 {
            bytes.extend_from_slice(&[0x4A, 0, 0, 0, 1, b'm']);
            bytes.extend_from_slice(&[0x34, 0, 0, 0, 0]);
        }
        assert_eq!(Message::decode(&bytes), Err(DecodeError::TooDeep));
    }
}
