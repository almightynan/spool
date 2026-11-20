// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Media size names. Keywords follow PWG 5101.1 self-describing names
//! ("class_name_WxHunit"); sizes in media-col are in 1/100 mm (PWG 5100.7).

/// Common sizes as (PWG name part, display name, width 1/100 mm, height 1/100 mm).
const KNOWN: &[(&str, &str, i32, i32)] = &[
    ("a3", "A3", 29700, 42000),
    ("a4", "A4", 21000, 29700),
    ("a5", "A5", 14800, 21000),
    ("a6", "A6", 10500, 14800),
    ("b5", "B5", 17600, 25000),
    ("letter", "Letter", 21590, 27940),
    ("legal", "Legal", 21590, 35560),
    ("executive", "Executive", 18415, 26670),
    ("index-4x6", "4 x 6 in", 10160, 15240),
    ("5x7", "5 x 7 in", 12700, 17780),
    ("index-5x8", "5 x 8 in", 12700, 20320),
    ("dl", "DL envelope", 11000, 22000),
    ("c5", "C5 envelope", 16200, 22900),
    ("number-10", "No. 10 envelope", 10477, 24130),
    ("hagaki", "Hagaki", 10000, 14800),
];

/// Display name for a media size given in 1/100 mm.
pub fn media_size_name(x: i32, y: i32) -> String {
    // 1 mm tolerance: printers round PWG sizes differently.
    for (_, name, w, h) in KNOWN {
        if (x - w).abs() <= 100 && (y - h).abs() <= 100 {
            return (*name).to_string();
        }
    }
    format!("{} x {} mm", (x + 50) / 100, (y + 50) / 100)
}

/// Display name for a PWG 5101.1 media keyword such as "iso_a4_210x297mm".
pub fn media_keyword_name(keyword: &str) -> String {
    let parts: Vec<&str> = keyword.splitn(3, '_').collect();
    if let [_, name, dims] = parts.as_slice() {
        if let Some((_, display, _, _)) = KNOWN.iter().find(|(k, ..)| k == name) {
            return (*display).to_string();
        }
        if let Some(d) = dims.strip_suffix("mm") {
            return format!("{} mm", d.replace('x', " x "));
        }
        if let Some(d) = dims.strip_suffix("in") {
            return format!("{} in", d.replace('x', " x "));
        }
    }
    keyword.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords() {
        assert_eq!(media_keyword_name("iso_a4_210x297mm"), "A4");
        assert_eq!(media_keyword_name("na_letter_8.5x11in"), "Letter");
        assert_eq!(media_keyword_name("na_index-4x6_4x6in"), "4 x 6 in");
        assert_eq!(media_keyword_name("om_custom_100x200mm"), "100 x 200 mm");
        assert_eq!(media_keyword_name("oddball"), "oddball");
    }

    #[test]
    fn sizes() {
        assert_eq!(media_size_name(21000, 29700), "A4");
        assert_eq!(media_size_name(21590, 27940), "Letter");
        assert_eq!(media_size_name(12000, 18000), "120 x 180 mm");
    }
}
