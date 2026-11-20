// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads image size and component count from a JPEG's start-of-frame marker
//! (ITU-T T.81 section B.2.2), so pages can be placed in a PDF unchanged.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    /// 1 gray, 3 YCbCr/RGB, 4 CMYK.
    pub components: u8,
}

pub fn read_info(data: &[u8]) -> Option<JpegInfo> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        // Fill bytes and markers without a length.
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xD0..=0xD7) {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let seg = data.get(i + 4..i + 2 + len)?;
            if seg.len() < 6 {
                return None;
            }
            let height = u16::from_be_bytes([seg[1], seg[2]]) as u32;
            let width = u16::from_be_bytes([seg[3], seg[4]]) as u32;
            return Some(JpegInfo {
                width,
                height,
                components: seg[5],
            });
        }
        i += 2 + len;
    }
    None
}

/// A tiny valid JPEG header for tests: SOI, APP0, SOF0 with the given size.
#[cfg(test)]
pub fn test_jpeg(width: u16, height: u16, components: u8) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    v.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    v.extend_from_slice(&[0xFF, 0xC0, 0x00, 8 + 3 * components, 8]);
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(&width.to_be_bytes());
    v.push(components);
    for c in 0..components {
        v.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sof0() {
        let j = test_jpeg(2480, 3508, 3);
        assert_eq!(
            read_info(&j),
            Some(JpegInfo {
                width: 2480,
                height: 3508,
                components: 3
            })
        );
    }

    #[test]
    fn rejects_non_jpeg_and_truncation() {
        assert_eq!(read_info(b"%PDF-1.4"), None);
        let j = test_jpeg(10, 10, 1);
        for cut in 0..j.len() - 2 {
            let _ = read_info(&j[..cut]);
        }
        assert_eq!(read_info(&j[..12]), None);
    }
}
