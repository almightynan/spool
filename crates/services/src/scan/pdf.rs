// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Writes JPEG pages into a PDF 1.4 file (ISO 32000-1): each page holds one
//! image XObject with /DCTDecode, so the scanner's JPEG data is stored as-is.

use super::jpeg::{read_info, JpegInfo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    NotJpeg { page: usize },
    UnsupportedComponents { page: usize, components: u8 },
    NoPages,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJpeg { page } => write!(f, "page {page} is not a JPEG image"),
            Self::UnsupportedComponents { page, components } => {
                write!(
                    f,
                    "page {page} has {components} color channels, which is not supported"
                )
            }
            Self::NoPages => write!(f, "there are no pages to save"),
        }
    }
}

impl std::error::Error for PdfError {}

pub struct PdfPage<'a> {
    pub jpeg: &'a [u8],
    /// Scan resolution, used for the physical page size.
    pub dpi: u32,
}

pub fn write_pdf(pages: &[PdfPage<'_>]) -> Result<Vec<u8>, PdfError> {
    if pages.is_empty() {
        return Err(PdfError::NoPages);
    }
    let infos: Vec<JpegInfo> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let info = read_info(p.jpeg).ok_or(PdfError::NotJpeg { page: i + 1 })?;
            match info.components {
                1 | 3 => Ok(info),
                c => Err(PdfError::UnsupportedComponents {
                    page: i + 1,
                    components: c,
                }),
            }
        })
        .collect::<Result<_, _>>()?;

    // Objects: 1 catalog, 2 pages, then per page: page, image, content.
    let n = pages.len();
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    obj(
        &mut out,
        &mut offsets,
        format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes(),
    );
    for (i, (p, info)) in pages.iter().zip(&infos).enumerate() {
        let page_id = 3 + i * 3;
        let dpi = p.dpi.max(1) as f64;
        let w = info.width as f64 * 72.0 / dpi;
        let h = info.height as f64 * 72.0 / dpi;
        obj(
            &mut out,
            &mut offsets,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] \
                 /Resources << /XObject << /Im0 {} 0 R >> >> /Contents {} 0 R >>",
                page_id + 1,
                page_id + 2
            )
            .as_bytes(),
        );
        let cs = if info.components == 1 {
            "DeviceGray"
        } else {
            "DeviceRGB"
        };
        let mut img = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{cs} \
             /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            info.width,
            info.height,
            p.jpeg.len()
        )
        .into_bytes();
        img.extend_from_slice(p.jpeg);
        img.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &img);
        let content = format!("q {w:.2} 0 0 {h:.2} 0 0 cm /Im0 Do Q");
        obj(
            &mut out,
            &mut offsets,
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            )
            .as_bytes(),
        );
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::jpeg::test_jpeg;

    #[test]
    fn xref_offsets_point_at_objects() {
        let a = test_jpeg(2480, 3508, 3);
        let b = test_jpeg(1240, 1754, 1);
        let pdf = write_pdf(&[
            PdfPage { jpeg: &a, dpi: 300 },
            PdfPage { jpeg: &b, dpi: 150 },
        ])
        .unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        // A4 at 300 dpi is 595.2 x 841.92 pt.
        assert!(text.contains("/MediaBox [0 0 595.20 841.92]"));
        assert!(text.contains("/ColorSpace /DeviceGray"));
        // Work on bytes: the binary comment in line 2 is not UTF-8, so offsets
        // into a lossy string would be shifted.
        let find = |hay: &[u8], needle: &[u8]| {
            hay.windows(needle.len())
                .rposition(|w| w == needle)
                .unwrap()
        };
        let sx = find(&pdf, b"startxref\n") + b"startxref\n".len();
        let end = sx + pdf[sx..].iter().position(|b| *b == b'\n').unwrap();
        let xref_at: usize = std::str::from_utf8(&pdf[sx..end]).unwrap().parse().unwrap();
        assert!(pdf[xref_at..].starts_with(b"xref"));
        let table = std::str::from_utf8(&pdf[xref_at..sx]).unwrap();
        for (i, line) in table.lines().skip(3).take(8).enumerate() {
            let off: usize = line[..10].parse().unwrap();
            assert!(
                pdf[off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()),
                "object {}",
                i + 1
            );
        }
    }

    #[test]
    fn errors() {
        assert_eq!(write_pdf(&[]), Err(PdfError::NoPages));
        assert_eq!(
            write_pdf(&[PdfPage {
                jpeg: b"nope",
                dpi: 300
            }]),
            Err(PdfError::NotJpeg { page: 1 })
        );
        let cmyk = test_jpeg(10, 10, 4);
        assert_eq!(
            write_pdf(&[PdfPage {
                jpeg: &cmyk,
                dpi: 300
            }]),
            Err(PdfError::UnsupportedComponents {
                page: 1,
                components: 4
            })
        );
    }
}

#[cfg(test)]
mod real_jpeg {
    use super::*;
    /// `SPOOL_TEST_JPEG=x.jpg SPOOL_TEST_PDF_OUT=y.pdf cargo test -p spool-services real_jpeg -- --ignored`
    #[test]
    #[ignore = "writes a file for checking with an external PDF reader"]
    fn write_real_jpeg() {
        let (Ok(i), Ok(o)) = (
            std::env::var("SPOOL_TEST_JPEG"),
            std::env::var("SPOOL_TEST_PDF_OUT"),
        ) else {
            return;
        };
        let j = std::fs::read(i).unwrap();
        std::fs::write(
            o,
            write_pdf(&[
                PdfPage { jpeg: &j, dpi: 150 },
                PdfPage { jpeg: &j, dpi: 300 },
            ])
            .unwrap(),
        )
        .unwrap();
    }
}
