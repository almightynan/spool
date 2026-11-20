// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! IPP status codes (RFC 8011 Appendix B) and operation ids (RFC 8011 section 5.4.15,
//! CUPS IPP extensions).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatusCode(pub u16);

impl StatusCode {
    pub const OK: Self = Self(0x0000);
    pub const OK_IGNORED_OR_SUBSTITUTED: Self = Self(0x0001);
    pub const OK_CONFLICTING: Self = Self(0x0002);
    pub const BAD_REQUEST: Self = Self(0x0400);
    pub const FORBIDDEN: Self = Self(0x0401);
    pub const NOT_AUTHENTICATED: Self = Self(0x0402);
    pub const NOT_AUTHORIZED: Self = Self(0x0403);
    pub const NOT_POSSIBLE: Self = Self(0x0404);
    pub const TIMEOUT: Self = Self(0x0405);
    pub const NOT_FOUND: Self = Self(0x0406);
    pub const GONE: Self = Self(0x0407);
    pub const DOCUMENT_FORMAT_NOT_SUPPORTED: Self = Self(0x040A);
    pub const ATTRIBUTES_OR_VALUES_NOT_SUPPORTED: Self = Self(0x040B);
    pub const INTERNAL_ERROR: Self = Self(0x0500);
    pub const OPERATION_NOT_SUPPORTED: Self = Self(0x0501);
    pub const SERVICE_UNAVAILABLE: Self = Self(0x0502);
    pub const VERSION_NOT_SUPPORTED: Self = Self(0x0503);
    pub const DEVICE_ERROR: Self = Self(0x0504);
    pub const TEMPORARY_ERROR: Self = Self(0x0505);
    pub const NOT_ACCEPTING_JOBS: Self = Self(0x0506);
    pub const BUSY: Self = Self(0x0507);
    pub const JOB_CANCELED: Self = Self(0x0508);

    /// RFC 8011 Appendix B: "successful" is 0x0000 to 0x00FF.
    pub fn is_success(self) -> bool {
        self.0 <= 0x00FF
    }

    /// Keyword name from RFC 8011 Appendix B, if this crate knows it.
    pub fn keyword(self) -> Option<&'static str> {
        Some(match self.0 {
            0x0000 => "successful-ok",
            0x0001 => "successful-ok-ignored-or-substituted-attributes",
            0x0002 => "successful-ok-conflicting-attributes",
            0x0400 => "client-error-bad-request",
            0x0401 => "client-error-forbidden",
            0x0402 => "client-error-not-authenticated",
            0x0403 => "client-error-not-authorized",
            0x0404 => "client-error-not-possible",
            0x0405 => "client-error-timeout",
            0x0406 => "client-error-not-found",
            0x0407 => "client-error-gone",
            0x0408 => "client-error-request-entity-too-large",
            0x0409 => "client-error-request-value-too-long",
            0x040A => "client-error-document-format-not-supported",
            0x040B => "client-error-attributes-or-values-not-supported",
            0x040C => "client-error-uri-scheme-not-supported",
            0x040D => "client-error-charset-not-supported",
            0x040E => "client-error-conflicting-attributes",
            0x040F => "client-error-compression-not-supported",
            0x0410 => "client-error-compression-error",
            0x0411 => "client-error-document-format-error",
            0x0412 => "client-error-document-access-error",
            0x0500 => "server-error-internal-error",
            0x0501 => "server-error-operation-not-supported",
            0x0502 => "server-error-service-unavailable",
            0x0503 => "server-error-version-not-supported",
            0x0504 => "server-error-device-error",
            0x0505 => "server-error-temporary-error",
            0x0506 => "server-error-not-accepting-jobs",
            0x0507 => "server-error-busy",
            0x0508 => "server-error-job-canceled",
            0x0509 => "server-error-multiple-document-jobs-not-supported",
            _ => return None,
        })
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.keyword() {
            Some(k) => write!(f, "{k} (0x{:04x})", self.0),
            None => write!(f, "status 0x{:04x}", self.0),
        }
    }
}

/// Operation ids used by this app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Operation(pub u16);

impl Operation {
    // RFC 8011 section 5.4.15
    pub const PRINT_JOB: Self = Self(0x0002);
    pub const CANCEL_JOB: Self = Self(0x0008);
    pub const GET_JOB_ATTRIBUTES: Self = Self(0x0009);
    pub const GET_JOBS: Self = Self(0x000A);
    pub const GET_PRINTER_ATTRIBUTES: Self = Self(0x000B);
    pub const HOLD_JOB: Self = Self(0x000C);
    pub const RELEASE_JOB: Self = Self(0x000D);
    pub const RESTART_JOB: Self = Self(0x000E);
    // CUPS IPP extensions (cups.org/doc/spec-ipp.html)
    pub const CUPS_GET_DEFAULT: Self = Self(0x4001);
    pub const CUPS_GET_PRINTERS: Self = Self(0x4002);
    pub const CUPS_ADD_MODIFY_PRINTER: Self = Self(0x4003);
    pub const CUPS_DELETE_PRINTER: Self = Self(0x4004);
    pub const CUPS_SET_DEFAULT: Self = Self(0x400A);
}
