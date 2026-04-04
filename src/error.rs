//! Error types and HTTP error decoding for Zenodo responses.
//!
//! [`ZenodoError`] intentionally covers both structured Zenodo API failures and
//! lower-level transport or local I/O problems so callers can decide whether an
//! operation should be retried, surfaced to users, or treated as a workflow
//! invariant violation.

use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Field-specific validation error returned by Zenodo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    /// The field name, when Zenodo reports one.
    #[serde(default)]
    pub field: Option<String>,
    /// Human-readable error message for the field.
    pub message: String,
}

/// Errors produced by the Zenodo client.
#[derive(Debug, Error)]
pub enum ZenodoError {
    /// Zenodo returned a non-success HTTP status.
    #[error("Zenodo returned HTTP {status}: {message:?}")]
    Http {
        /// HTTP status returned by Zenodo.
        status: StatusCode,
        /// Summary message extracted from the response body, when available.
        message: Option<String>,
        /// Field-level validation errors extracted from the response body.
        field_errors: Vec<FieldError>,
        /// Trimmed raw response body for diagnostics.
        raw_body: Option<String>,
    },
    /// A transport error occurred while sending or receiving a request.
    #[error(transparent)]
    Transport(
        /// Underlying transport error.
        #[from]
        reqwest::Error,
    ),
    /// JSON serialization or deserialization failed.
    #[error(transparent)]
    Json(
        /// Underlying JSON error.
        #[from]
        serde_json::Error,
    ),
    /// A local I/O operation failed.
    #[error(transparent)]
    Io(
        /// Underlying I/O error.
        #[from]
        std::io::Error,
    ),
    /// A URL could not be parsed or joined.
    #[error(transparent)]
    Url(
        /// Underlying URL parse error.
        #[from]
        url::ParseError,
    ),
    /// A required environment variable could not be read.
    #[error("failed to read environment variable {name}: {source}")]
    EnvVar {
        /// Environment variable name.
        name: String,
        /// Underlying environment lookup error.
        #[source]
        source: std::env::VarError,
    },
    /// Zenodo returned data that violates a workflow invariant.
    #[error("invalid Zenodo state: {0}")]
    InvalidState(
        /// Description of the invalid state.
        String,
    ),
    /// A required link relation was missing from a Zenodo payload.
    #[error("missing Zenodo link: {0}")]
    MissingLink(
        /// Missing link relation name.
        &'static str,
    ),
    /// A requested file key was not present on a record.
    #[error("missing record file: {key}")]
    MissingFile {
        /// Missing record file key.
        key: String,
    },
    /// Multiple uploads targeted the same final filename.
    #[error("duplicate upload filename: {filename}")]
    DuplicateUploadFilename {
        /// Duplicate filename seen in the upload set.
        filename: String,
    },
    /// A keep-existing upload would overwrite an existing draft file.
    #[error("draft already contains file and replacement policy forbids overwrite: {filename}")]
    ConflictingDraftFile {
        /// Conflicting filename already present on the draft.
        filename: String,
    },
    /// A selector could not be resolved to a record or artifact.
    #[error("unsupported selector: {0}")]
    UnsupportedSelector(
        /// Description of the unsupported selector.
        String,
    ),
    /// A checksum validation step failed.
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Expected checksum value.
        expected: String,
        /// Actual checksum value.
        actual: String,
    },
    /// Polling timed out before Zenodo reached the requested state.
    #[error("timed out waiting for Zenodo {0}")]
    Timeout(
        /// Label for the operation that timed out.
        &'static str,
    ),
}

impl ZenodoError {
    pub(crate) async fn from_response(response: Response) -> Self {
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        let body = match response.bytes().await {
            Ok(body) => body,
            Err(error) => return Self::Transport(error),
        };

        decode_http_error(status, content_type.as_deref(), &body)
    }
}

pub(crate) fn decode_http_error(
    status: StatusCode,
    content_type: Option<&str>,
    body: &[u8],
) -> ZenodoError {
    let raw_body = trimmed_body(body);
    let parsed = if looks_like_json(content_type, body) {
        parse_json_error(body)
    } else {
        None
    };

    let (message, field_errors) = match parsed {
        Some((message, field_errors)) => (message, field_errors),
        None => (raw_body.clone(), Vec::new()),
    };

    ZenodoError::Http {
        status,
        message,
        field_errors,
        raw_body,
    }
}

fn looks_like_json(content_type: Option<&str>, body: &[u8]) -> bool {
    if content_type
        .is_some_and(|value| value.starts_with("application/json") || value.ends_with("+json"))
    {
        return true;
    }

    body.iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| matches!(byte, b'{' | b'['))
}

fn parse_json_error(body: &[u8]) -> Option<(Option<String>, Vec<FieldError>)> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let message = if let Some(message) = value.get("message").and_then(Value::as_str) {
        Some(message.to_owned())
    } else {
        value
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };

    let field_errors = if let Some(errors) = value.get("errors") {
        parse_field_errors(errors).unwrap_or_default()
    } else {
        Vec::new()
    };

    Some((message, field_errors))
}

fn parse_field_errors(value: &Value) -> Option<Vec<FieldError>> {
    match value {
        Value::Array(items) => {
            let mut errors = Vec::new();
            for item in items {
                match item {
                    Value::Object(map) => {
                        let message =
                            if let Some(message) = map.get("message").and_then(Value::as_str) {
                                message.to_owned()
                            } else {
                                "unknown error".to_owned()
                            };
                        errors.push(FieldError {
                            field: map.get("field").and_then(Value::as_str).map(str::to_owned),
                            message,
                        });
                    }
                    Value::String(message) => errors.push(FieldError {
                        field: None,
                        message: message.clone(),
                    }),
                    _ => {}
                }
            }
            Some(errors)
        }
        Value::Object(map) => {
            let mut errors = Vec::new();
            for (field, message) in map {
                let message = if let Some(message) = message.as_str() {
                    message.to_owned()
                } else {
                    message.to_string()
                };
                errors.push(FieldError {
                    field: Some(field.clone()),
                    message,
                });
            }
            Some(errors)
        }
        _ => None,
    }
}

fn trimmed_body(body: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(body);
    for line in text.lines().map(str::trim) {
        if !line.is_empty() {
            return Some(line.chars().take(512).collect());
        }
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.chars().take(512).collect())
}

#[cfg(test)]
mod tests {
    use super::{decode_http_error, parse_field_errors, parse_json_error, trimmed_body};
    use reqwest::StatusCode;
    use serde_json::json;

    #[test]
    fn parses_json_error_bodies() {
        let error = decode_http_error(
            StatusCode::BAD_REQUEST,
            Some("application/json"),
            br#"{"message":"bad metadata","errors":[{"field":"metadata.title","message":"required"}]}"#,
        );

        match error {
            super::ZenodoError::Http {
                message,
                field_errors,
                ..
            } => {
                assert_eq!(message.as_deref(), Some("bad metadata"));
                assert_eq!(field_errors.len(), 1);
                assert_eq!(field_errors[0].field.as_deref(), Some("metadata.title"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parses_plaintext_error_bodies() {
        let error = decode_http_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some("text/plain"),
            b"upstream exploded\nstack trace omitted",
        );

        match error {
            super::ZenodoError::Http { message, .. } => {
                assert_eq!(message.as_deref(), Some("upstream exploded"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parses_object_shaped_field_errors_and_json_without_content_type() {
        let error = decode_http_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            None,
            br#"{"title":"validation failed","errors":{"metadata.creators":"required"}}"#,
        );

        match error {
            super::ZenodoError::Http {
                message,
                field_errors,
                ..
            } => {
                assert_eq!(message.as_deref(), Some("validation failed"));
                assert_eq!(field_errors[0].field.as_deref(), Some("metadata.creators"));
                assert_eq!(field_errors[0].message, "required");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn preserves_string_array_errors_and_empty_bodies() {
        let error = decode_http_error(
            StatusCode::BAD_REQUEST,
            Some("application/problem+json"),
            br#"{"errors":["first","second"]}"#,
        );
        match error {
            super::ZenodoError::Http { field_errors, .. } => {
                assert_eq!(field_errors.len(), 2);
                assert_eq!(field_errors[0].message, "first");
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let empty = decode_http_error(StatusCode::BAD_GATEWAY, Some("text/plain"), b"   ");
        match empty {
            super::ZenodoError::Http {
                message, raw_body, ..
            } => {
                assert_eq!(message, None);
                assert_eq!(raw_body, None);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn covers_title_only_invalid_json_and_mixed_error_shapes() {
        let title_only = decode_http_error(
            StatusCode::BAD_REQUEST,
            Some("application/json"),
            br#"{"title":"just title"}"#,
        );
        match title_only {
            super::ZenodoError::Http { message, .. } => {
                assert_eq!(message.as_deref(), Some("just title"));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let malformed = decode_http_error(
            StatusCode::BAD_REQUEST,
            Some("application/json"),
            br#"{"broken":"json""#,
        );
        match malformed {
            super::ZenodoError::Http {
                message, raw_body, ..
            } => {
                assert_eq!(message.as_deref(), Some("{\"broken\":\"json\""));
                assert_eq!(raw_body.as_deref(), Some("{\"broken\":\"json\""));
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let mixed = decode_http_error(
            StatusCode::BAD_REQUEST,
            Some("application/json"),
            br#"{"errors":[{"field":"a"},42],"title":"mix"}"#,
        );
        match mixed {
            super::ZenodoError::Http { field_errors, .. } => {
                assert_eq!(field_errors.len(), 1);
                assert_eq!(field_errors[0].message, "unknown error");
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let object_non_string = decode_http_error(
            StatusCode::BAD_REQUEST,
            Some("application/json"),
            br#"{"errors":{"field":{"nested":true}}}"#,
        );
        match object_non_string {
            super::ZenodoError::Http { field_errors, .. } => {
                assert_eq!(field_errors[0].message, "{\"nested\":true}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn direct_error_helpers_cover_remaining_shapes() {
        let parsed = parse_json_error(br#"{"title":"title only"}"#).unwrap();
        assert_eq!(parsed.0.as_deref(), Some("title only"));
        assert!(parsed.1.is_empty());

        let object_errors = parse_field_errors(&json!({
            "metadata.title": { "detail": "required" }
        }))
        .unwrap();
        assert_eq!(object_errors[0].field.as_deref(), Some("metadata.title"));
        assert_eq!(object_errors[0].message, r#"{"detail":"required"}"#);

        let array_errors = parse_field_errors(&json!([
            { "field": "metadata.title" }
        ]))
        .unwrap();
        assert_eq!(array_errors[0].message, "unknown error");

        assert_eq!(parse_field_errors(&json!(true)), None);
        assert_eq!(
            trimmed_body(b"   single line without newline   "),
            Some("single line without newline".into())
        );
    }

    #[tokio::test]
    async fn from_response_decodes_reqwest_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 418 I'm a teapot\r\ncontent-type: text/plain\r\ncontent-length: 13\r\n\r\nbrew failed\r\n",
                )
                .await;
            let _ = stream.shutdown().await;
        });

        let response = reqwest::get(format!("http://{address}/")).await.unwrap();
        let error = super::ZenodoError::from_response(response).await;

        match error {
            super::ZenodoError::Http {
                status,
                message,
                raw_body,
                ..
            } => {
                assert_eq!(status, StatusCode::IM_A_TEAPOT);
                assert_eq!(message.as_deref(), Some("brew failed"));
                assert_eq!(raw_body.as_deref(), Some("brew failed"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
