//! Typed Office-to-PDF RPCs for the Desktop preview surface.

use crate::{transport::HttpTransport, workspace_files::WorkspaceFileScope, Error};
use serde::Deserialize;
use std::sync::atomic::AtomicBool;

/// A converted Office document returned by the Host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfficePreview {
    /// Complete PDF bytes.
    pub pdf: Vec<u8>,
    /// OOXML font families unavailable to this conversion.
    pub missing_fonts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderedDocument {
    #[serde(default)]
    data: Vec<u8>,
    #[serde(default)]
    missing_fonts: Vec<String>,
}

/// Native client for Host Office-to-PDF conversion.
pub struct OfficeToPdfClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> OfficeToPdfClient<'a> {
    /// Creates a client over an authenticated transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Converts one authorized Office file to PDF.
    pub fn render(
        &self,
        scope: WorkspaceFileScope,
        path: impl Into<String>,
        priority: &str,
    ) -> Result<OfficePreview, Error> {
        let value = self.transport.request(
            "officeToPdf/render",
            serde_json::json!({
                "workspaceFileScopeId": scope.session_id,
                "path": path.into(),
                "priority": priority,
            }),
            &AtomicBool::new(false),
        )?;
        let rendered: RenderedDocument = serde_json::from_value(value)?;
        Ok(OfficePreview {
            pdf: rendered.data,
            missing_fonts: rendered.missing_fonts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rendered_pdf_bytes() {
        let rendered: RenderedDocument = serde_json::from_value(serde_json::json!({
            "absolutePath": "/x/report.docx",
            "version": "v",
            "bytes": 4,
            "offset": 0,
            "data": [37, 80, 68, 70],
            "eof": true,
            "missingFonts": ["Calibri"],
            "generation": 1
        }))
        .unwrap();
        assert_eq!(rendered.data, [37, 80, 68, 70]);
        assert_eq!(rendered.missing_fonts, ["Calibri"]);
    }
}
