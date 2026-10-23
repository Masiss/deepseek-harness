//! Session file upload over the authenticated raw-byte route.

use crate::{transport::HttpTransport, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use url::form_urlencoded;

/// Receipt and durable reference returned by a completed upload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUploadValue {
    /// Opaque receipt referenced by a prompt's file part.
    pub receipt_id: String,
    /// Host-owned durable attachment reference.
    pub file: Value,
}

/// Uploads one file for a Session.
pub struct FileUploadClient<'a> {
    transport: &'a HttpTransport,
}

impl<'a> FileUploadClient<'a> {
    /// Creates an upload client over an HTTP transport.
    pub fn new(transport: &'a HttpTransport) -> Self {
        Self { transport }
    }

    /// Stores exact bytes as a staged receipt for the Session.
    pub fn upload(
        &self,
        session_id: &str,
        name: Option<&str>,
        bytes: Vec<u8>,
    ) -> Result<FileUploadValue, Error> {
        let query = form_urlencoded::Serializer::new(String::new())
            .append_pair("sessionId", session_id)
            .append_pair("name", name.unwrap_or_default())
            .finish();
        let path = format!("session/uploadFileBinary?{query}");
        let body = self.transport.post_json(
            &path,
            "application/octet-stream",
            bytes,
            &AtomicBool::new(false),
        )?;
        let record = body
            .as_object()
            .ok_or(Error::InvalidEnvelope("file upload result"))?;
        match record.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(serde_json::from_value(
                record.get("value").cloned().unwrap_or(Value::Null),
            )?),
            Some(false) => {
                let error = record
                    .get("error")
                    .and_then(Value::as_object)
                    .ok_or(Error::InvalidEnvelope("file upload failure"))?;
                let field = |name: &str| {
                    error
                        .get(name)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                let details = error
                    .get("details")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                Err(Error::Remote {
                    code: field("code"),
                    message: field("message"),
                    details,
                })
            }
            None => Err(Error::InvalidEnvelope("file upload result")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httptest::{matchers::*, responders::status_code, Expectation, Server};

    #[test]
    fn uploads_exact_bytes_with_session_and_name_query() {
        let server = Server::run();
        server.expect(
            Expectation::matching(all_of![
                request::method_path(
                    "POST",
                    "/api/session/uploadFileBinary",
                ),
                request::query("sessionId=s1&name=notes+%26+refs.pdf"),
                request::headers(contains(("content-type", "application/octet-stream"))),
                request::body("bytes"),
            ])
            .respond_with(status_code(200).body(
                r#"{"ok":true,"value":{"receiptId":"r1","file":{"attachmentId":"sha256:x"}}}"#,
            )),
        );
        let transport = HttpTransport::new(&server.url_str("/api/")).unwrap();
        let value = FileUploadClient::new(&transport)
            .upload("s1", Some("notes & refs.pdf"), b"bytes".to_vec())
            .unwrap();
        assert_eq!(value.receipt_id, "r1");
        assert_eq!(value.file["attachmentId"], "sha256:x");
    }

    #[test]
    fn upload_rejection_preserves_the_remote_failure() {
        let server = Server::run();
        server.expect(Expectation::matching(request::method_path(
            "POST",
            "/api/session/uploadFileBinary",
        ))
        .respond_with(status_code(200).body(
            r#"{"ok":false,"error":{"code":"attachment/too-large","message":"Too large","details":{"bytes":9}}}"#,
        )));
        let transport = HttpTransport::new(&server.url_str("/api/")).unwrap();
        let error = FileUploadClient::new(&transport)
            .upload("s1", None, vec![0])
            .unwrap_err();
        let Error::Remote {
            code,
            message,
            details,
        } = error
        else {
            panic!("expected remote failure")
        };
        assert_eq!(code, "attachment/too-large");
        assert_eq!(message, "Too large");
        assert_eq!(details["bytes"], 9);
    }
}
