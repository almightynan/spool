// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! The CUPS test page, sent as a banner description that cupsd renders with
//! its own template (cups-filters bannertopdf, "default-testpage.pdf").
//! Content is identical to /usr/share/cups/data/testprint from cups-filters,
//! which is not visible inside the Flatpak sandbox.

pub const TEST_PAGE_FORMAT: &str = "application/vnd.cups-pdf-banner";

pub const TEST_PAGE: &str = "#PDF-BANNER\n\
Template default-testpage.pdf\n\
Show printer-name printer-info printer-location printer-make-and-model printer-driver-name printer-driver-version paper-size imageable-area job-id options time-at-creation time-at-processing\n\
\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_system_file_when_present() {
        if let Ok(sys) = std::fs::read_to_string("/usr/share/cups/data/testprint") {
            assert_eq!(sys, TEST_PAGE);
        }
    }
}
