// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! IPP/1.1 and IPP/2.0 message encoding (RFC 8010) and request building (RFC 8011).

pub mod codec;
pub mod message;
pub mod status;
pub mod value;

pub use codec::{DecodeError, Message};
pub use message::{Request, Response, DEFAULT_VERSION, FALLBACK_VERSION};
pub use status::{Operation, StatusCode};
pub use value::{Attribute, AttributeSet, Group, GroupTag, OutOfBand, StringKind, Value};
