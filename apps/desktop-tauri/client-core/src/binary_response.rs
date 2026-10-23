//! Connection RPC multipart responses, with byte attachments restored before typed decoding.

use futures::{executor::block_on, stream};
use serde_json::Value;
use std::{collections::HashMap, convert::Infallible};

/// Invalid multipart framing, metadata, correlation, or attachment references.
#[derive(Debug, thiserror::Error)]
pub enum BinaryResponseError {
    #[error("invalid multipart response: {0}")]
    Multipart(#[from] multer::Error),
    #[error("invalid binary response metadata: {0}")]
    Metadata(#[from] serde_json::Error),
    #[error("invalid binary response: {0}")]
    Invalid(&'static str),
    #[error("response correlation mismatch: expected {expected}, got {actual}")]
    CorrelationMismatch { expected: String, actual: String },
}

struct Part {
    is_file: bool,
    bytes: Vec<u8>,
}

/// Parses raw multipart/form-data and returns the validated success `result.value`.
///
/// `content_type` is the complete HTTP header, including its boundary parameter;
/// `expected_rpc_id` is the ID minted for the request. Every named field must be
/// unique, metadata must be text, and every file part must be referenced exactly
/// once by a bytes attachment targeting an existing JSON-null placeholder.
///
/// Bytes become JSON arrays of integers in 0..=255, not base64 or objects with
/// numeric keys. This is the representation consumed by `WorkspaceFileBytes`'s
/// `Vec<u8>` deserializer. Multipart parsing operates only on the supplied body
/// and does not perform I/O or require a Tokio runtime.
pub fn parse_binary_response(
    content_type: &str,
    body: &[u8],
    expected_rpc_id: &str,
) -> Result<Value, BinaryResponseError> {
    let boundary = multer::parse_boundary(content_type)?;
    let mut fields = block_on(async {
        let input = stream::iter([Ok::<_, Infallible>(body.to_vec())]);
        let mut multipart = multer::Multipart::new(input, boundary);
        let mut fields = HashMap::new();
        while let Some(field) = multipart.next_field().await? {
            let name = field
                .name()
                .ok_or(BinaryResponseError::Invalid("unnamed field"))?
                .to_owned();
            if fields.contains_key(&name) {
                return Err(BinaryResponseError::Invalid("duplicate field"));
            }
            let is_file = field.file_name().is_some();
            let bytes = field.bytes().await?.to_vec();
            fields.insert(name, Part { is_file, bytes });
        }
        Ok::<_, BinaryResponseError>(fields)
    })?;

    let metadata = fields
        .remove("metadata")
        .filter(|part| !part.is_file)
        .ok_or(BinaryResponseError::Invalid("missing text metadata"))?;
    let envelope: Value = serde_json::from_slice(&metadata.bytes)?;
    if !envelope.is_object()
        || envelope.get("type").and_then(Value::as_str) != Some("server-response")
    {
        return Err(BinaryResponseError::Invalid("server-response type"));
    }
    let rpc_id = envelope
        .get("rpcId")
        .and_then(Value::as_str)
        .ok_or(BinaryResponseError::Invalid("rpcId"))?;
    if rpc_id != expected_rpc_id {
        return Err(BinaryResponseError::CorrelationMismatch {
            expected: expected_rpc_id.to_owned(),
            actual: rpc_id.to_owned(),
        });
    }
    let result = envelope
        .get("result")
        .filter(|result| result.is_object() && result.get("ok") == Some(&Value::Bool(true)))
        .ok_or(BinaryResponseError::Invalid("expected success result"))?;
    let attachments = envelope
        .get("attachments")
        .and_then(Value::as_array)
        .filter(|attachments| !attachments.is_empty())
        .ok_or(BinaryResponseError::Invalid("missing attachments"))?;
    let mut value = result
        .get("value")
        .cloned()
        .ok_or(BinaryResponseError::Invalid("missing result value"))?;
    for attachment in attachments {
        if !attachment.is_object()
            || attachment.get("codec").and_then(Value::as_str) != Some("bytes")
        {
            return Err(BinaryResponseError::Invalid("attachment codec"));
        }
        let name = attachment
            .get("part")
            .and_then(Value::as_str)
            .ok_or(BinaryResponseError::Invalid("attachment part"))?;
        let path = attachment
            .get("path")
            .and_then(Value::as_array)
            .ok_or(BinaryResponseError::Invalid("attachment path"))?;
        let part =
            fields
                .remove(name)
                .filter(|part| part.is_file)
                .ok_or(BinaryResponseError::Invalid(
                    "missing or non-file attachment part",
                ))?;
        let placeholder = placeholder_at(&mut value, path)?;
        if !placeholder.is_null() {
            return Err(BinaryResponseError::Invalid(
                "attachment placeholder is not null",
            ));
        }
        *placeholder = Value::Array(part.bytes.into_iter().map(Value::from).collect());
    }
    if !fields.is_empty() {
        return Err(BinaryResponseError::Invalid("unreferenced parts"));
    }
    Ok(value)
}

fn placeholder_at<'a>(
    mut value: &'a mut Value,
    path: &[Value],
) -> Result<&'a mut Value, BinaryResponseError> {
    for segment in path {
        value = match value {
            Value::Object(object) => segment.as_str().and_then(|key| object.get_mut(key)),
            Value::Array(array) => segment
                .as_f64()
                .filter(|index| {
                    index.fract() == 0.0 && *index >= 0.0 && *index <= 9_007_199_254_740_991.0
                })
                .and_then(|index| usize::try_from(index as u64).ok())
                .and_then(|index| array.get_mut(index)),
            _ => None,
        }
        .ok_or(BinaryResponseError::Invalid("attachment path"))?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CONTENT_TYPE: &str = "multipart/form-data; boundary=\"fixture-boundary\"";
    const OCTETS: &[u8] = b"\0\x01\x7f\x80\xff\r\n--fixture-boundarX\r\n";

    fn metadata() -> Value {
        json!({
            "type": "server-response", "rpcId": "r1",
            "result": {"ok": true, "value": {"data": null, "text": "retained"}},
            "attachments": [{"part": "bytes-0", "codec": "bytes", "path": ["data"]}]
        })
    }

    // File parts match the Host's FormData Blob serialization, including filename="blob".
    fn fixture(parts: &[(&str, bool, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, file, data) in parts {
            body.extend_from_slice(
                format!("--fixture-boundary\r\nContent-Disposition: form-data; name=\"{name}\"")
                    .as_bytes(),
            );
            if *file {
                body.extend_from_slice(
                    b"; filename=\"blob\"\r\nContent-Type: application/octet-stream",
                );
            }
            body.extend_from_slice(b"\r\n\r\n");
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(b"--fixture-boundary--\r\n");
        body
    }

    fn parse(metadata: &Value, bytes: &[u8]) -> Result<Value, BinaryResponseError> {
        parse_binary_response(
            CONTENT_TYPE,
            &fixture(&[
                ("metadata", false, &metadata.to_string().into_bytes()),
                ("bytes-0", true, bytes),
            ]),
            "r1",
        )
    }

    #[test]
    fn restores_binary_octets_in_either_part_order() {
        let meta = metadata().to_string();
        for parts in [
            vec![
                ("metadata", false, meta.as_bytes()),
                ("bytes-0", true, OCTETS),
            ],
            vec![
                ("bytes-0", true, OCTETS),
                ("metadata", false, meta.as_bytes()),
            ],
        ] {
            let value = parse_binary_response(CONTENT_TYPE, &fixture(&parts), "r1").unwrap();
            assert_eq!(value["text"], "retained");
            assert_eq!(
                serde_json::from_value::<Vec<u8>>(value["data"].clone()).unwrap(),
                OCTETS
            );
        }
    }

    #[test]
    fn restores_root_empty_bytes_nested_arrays_and_literal_object_keys() {
        for (value, path, expected) in [
            (Value::Null, json!([]), json!([])),
            (
                json!({"items": [null]}),
                json!(["items", 0]),
                json!({"items": [[]]}),
            ),
            (
                json!({"__proto__": null}),
                json!(["__proto__"]),
                json!({"__proto__": []}),
            ),
            (json!([null]), json!([0.0]), json!([[]])),
        ] {
            let mut meta = metadata();
            meta["result"]["value"] = value;
            meta["attachments"][0]["path"] = path;
            assert_eq!(parse(&meta, &[]).unwrap(), expected);
        }
    }

    #[test]
    fn restores_multiple_attachments_independently_of_part_order() {
        let mut meta = metadata();
        meta["result"]["value"] = json!({"data": null, "nested": [null]});
        meta["attachments"]
            .as_array_mut()
            .unwrap()
            .push(json!({"part": "bytes-1", "codec": "bytes", "path": ["nested", 0]}));
        let meta = meta.to_string();
        let body = fixture(&[
            ("bytes-1", true, b"\xff\0"),
            ("metadata", false, meta.as_bytes()),
            ("bytes-0", true, OCTETS),
        ]);
        let restored = parse_binary_response(CONTENT_TYPE, &body, "r1").unwrap();
        assert_eq!(restored["nested"][0], json!([255, 0]));
        assert_eq!(
            serde_json::from_value::<Vec<u8>>(restored["data"].clone()).unwrap(),
            OCTETS
        );
    }

    #[test]
    fn rejects_invalid_envelopes_and_attachment_metadata() {
        for (pointer, invalid) in [
            ("/type", json!("client-request")),
            ("/rpcId", json!(12)),
            ("/result", json!([])),
            ("/result/ok", json!(false)),
            ("/result/ok", json!("true")),
            ("/attachments", json!([])),
            ("/attachments", Value::Null),
            ("/attachments/0", json!([])),
            ("/attachments/0/codec", json!("base64")),
            ("/attachments/0/part", json!(1)),
            ("/attachments/0/part", json!("absent")),
            ("/attachments/0/path", Value::Null),
            ("/attachments/0/path", json!(["absent"])),
            ("/attachments/0/path", json!([0])),
            ("/attachments/0/path", json!(["text", "child"])),
            ("/result/value/data", json!(false)),
        ] {
            let mut meta = metadata();
            *meta.pointer_mut(pointer).unwrap() = invalid;
            assert!(parse(&meta, OCTETS).is_err(), "accepted {meta}");
        }
        let mut meta = metadata();
        meta["rpcId"] = json!("other");
        assert!(matches!(
            parse(&meta, OCTETS),
            Err(BinaryResponseError::CorrelationMismatch { .. })
        ));
        meta = metadata();
        meta["result"].as_object_mut().unwrap().remove("value");
        assert!(parse(&meta, OCTETS).is_err());
    }

    #[test]
    fn rejects_invalid_array_paths_and_nonexistent_placeholders() {
        for path in [
            json!([-1]),
            json!([0.5]),
            json!([1]),
            json!(["0"]),
            json!([9007199254740992_u64]),
            json!([null]),
        ] {
            let mut meta = metadata();
            meta["result"]["value"] = json!([null]);
            meta["attachments"][0]["path"] = path;
            assert!(parse(&meta, OCTETS).is_err());
        }
    }

    #[test]
    fn rejects_duplicate_missing_text_and_extra_fields() {
        let meta = metadata().to_string();
        let text = ("metadata", false, meta.as_bytes());
        let file = ("bytes-0", true, OCTETS);
        for parts in [
            vec![text, text, file],
            vec![text, file, file],
            vec![file],
            vec![("metadata", true, meta.as_bytes()), file],
            vec![text, ("bytes-0", false, OCTETS)],
            vec![text],
            vec![text, file, ("extra", false, b"extra")],
            vec![text, file, ("extra", true, b"extra")],
            vec![("metadata", false, b"not json"), file],
        ] {
            assert!(parse_binary_response(CONTENT_TYPE, &fixture(&parts), "r1").is_err());
        }
    }

    #[test]
    fn rejects_reused_parts_and_placeholders() {
        let mut meta = metadata();
        let duplicate = meta["attachments"][0].clone();
        meta["attachments"].as_array_mut().unwrap().push(duplicate);
        assert!(parse(&meta, OCTETS).is_err());
        meta["attachments"][1]["part"] = json!("bytes-1");
        let body = fixture(&[
            ("metadata", false, meta.to_string().as_bytes()),
            ("bytes-0", true, OCTETS),
            ("bytes-1", true, OCTETS),
        ]);
        assert!(parse_binary_response(CONTENT_TYPE, &body, "r1").is_err());
    }

    #[test]
    fn rejects_invalid_content_type_and_truncated_multipart() {
        let body = fixture(&[
            ("metadata", false, metadata().to_string().as_bytes()),
            ("bytes-0", true, OCTETS),
        ]);
        for content_type in [
            "application/json",
            "multipart/form-data",
            "multipart/mixed; boundary=fixture-boundary",
            "multipart/form-data; boundary=other",
        ] {
            assert!(parse_binary_response(content_type, &body, "r1").is_err());
        }
        assert!(parse_binary_response(CONTENT_TYPE, &body[..body.len() - 12], "r1").is_err());
    }
}
