// SPDX-FileCopyrightText: 2026 almightynan <https://github.com/almightynan>
// SPDX-License-Identifier: GPL-3.0-or-later
//! Writing scanned pages to disk (on the worker runtime).

use super::pdf::{write_pdf, PdfError, PdfPage};
use super::session::ScannedPage;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveFormat {
    Pdf,
    Jpeg,
}

impl SaveFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Jpeg => "jpg",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    Pdf(PdfError),
    Io(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pdf(e) => e.fmt(f),
            Self::Io(e) => write!(f, "could not write the file: {e}"),
        }
    }
}

impl std::error::Error for SaveError {}

/// Saves pages to `path`. PDF puts all pages in one file. JPEG writes one file
/// for a single page, or `name-1.jpg`, `name-2.jpg` for several.
/// Returns the files written.
pub async fn save_scan(
    pages: &[ScannedPage],
    format: SaveFormat,
    path: &Path,
) -> Result<Vec<PathBuf>, SaveError> {
    let io = |e: std::io::Error| SaveError::Io(e.to_string());
    // Scanners without JPEG output send PDF pages; those are written as they
    // came, one file per page when there are several.
    let native_pdf = !pages.is_empty() && pages.iter().all(|p| p.data.starts_with(b"%PDF"));
    match format {
        SaveFormat::Pdf if native_pdf => write_numbered(pages, path, "pdf").await,
        SaveFormat::Pdf => {
            let refs: Vec<PdfPage<'_>> = pages
                .iter()
                .map(|p| PdfPage {
                    jpeg: &p.data,
                    dpi: p.dpi,
                })
                .collect();
            let bytes = write_pdf(&refs).map_err(SaveError::Pdf)?;
            tokio::fs::write(path, bytes).await.map_err(io)?;
            Ok(vec![path.to_path_buf()])
        }
        SaveFormat::Jpeg => write_numbered(pages, path, "jpg").await,
    }
}

/// One file for one page, else `name-1.ext`, `name-2.ext` next to `path`.
async fn write_numbered(
    pages: &[ScannedPage],
    path: &Path,
    ext: &str,
) -> Result<Vec<PathBuf>, SaveError> {
    let io = |e: std::io::Error| SaveError::Io(e.to_string());
    match pages {
        [] => Err(SaveError::Pdf(PdfError::NoPages)),
        [one] => {
            tokio::fs::write(path, &one.data).await.map_err(io)?;
            Ok(vec![path.to_path_buf()])
        }
        many => {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "scan".into());
            let dir = path.parent().unwrap_or(Path::new("."));
            let mut out = Vec::new();
            for (i, p) in many.iter().enumerate() {
                let file = dir.join(format!("{stem}-{}.{ext}", i + 1));
                tokio::fs::write(&file, &p.data).await.map_err(io)?;
                out.push(file);
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::jpeg::test_jpeg;

    #[tokio::test]
    async fn pdf_and_numbered_jpegs() {
        let dir = std::env::temp_dir().join(format!("spool-save-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let page = ScannedPage {
            data: test_jpeg(100, 100, 3),
            content_type: Some("image/jpeg".into()),
            dpi: 100,
        };
        let pages = vec![page.clone(), page];
        let pdf = save_scan(&pages, SaveFormat::Pdf, &dir.join("a.pdf"))
            .await
            .unwrap();
        assert_eq!(pdf.len(), 1);
        assert!(std::fs::read(&pdf[0]).unwrap().starts_with(b"%PDF"));
        let jpgs = save_scan(&pages, SaveFormat::Jpeg, &dir.join("b.jpg"))
            .await
            .unwrap();
        assert_eq!(jpgs, vec![dir.join("b-1.jpg"), dir.join("b-2.jpg")]);
        let native = vec![ScannedPage {
            data: b"%PDF-1.7 native".to_vec(),
            content_type: Some("application/pdf".into()),
            dpi: 300,
        }];
        let out = save_scan(&native, SaveFormat::Pdf, &dir.join("c.pdf"))
            .await
            .unwrap();
        assert_eq!(std::fs::read(&out[0]).unwrap(), b"%PDF-1.7 native");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
