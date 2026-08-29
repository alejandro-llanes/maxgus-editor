//! JSON-RPC 2.0 messages and the `Content-Length` framing LSP uses.

use crate::{LspError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A request identifier. The specification allows numbers or strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl From<i64> for RequestId {
    fn from(n: i64) -> RequestId {
        RequestId::Number(n)
    }
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{n}"),
            RequestId::String(s) => write!(f, "{s}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: RequestId,
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub method: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// `null` for a response to a request that could not be parsed.
    pub id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
}

/// Any JSON-RPC message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl Message {
    /// The wire form, without framing.
    pub fn to_json(&self) -> Value {
        let mut value = match self {
            Message::Request(r) => {
                let mut v = serde_json::json!({ "id": r.id, "method": r.method });
                // A null `params` is omitted rather than sent as null.
                if !r.params.is_null() {
                    v["params"] = r.params.clone();
                }
                v
            }
            Message::Notification(n) => {
                let mut v = serde_json::json!({ "method": n.method });
                if !n.params.is_null() {
                    v["params"] = n.params.clone();
                }
                v
            }
            Message::Response(r) => {
                let mut v = serde_json::json!({ "id": r.id });
                if let Some(result) = &r.result {
                    v["result"] = result.clone();
                } else if r.error.is_none() {
                    // A successful response must carry `result`, even if null.
                    v["result"] = Value::Null;
                }
                if let Some(error) = &r.error {
                    v["error"] = serde_json::to_value(error).expect("error is serialisable");
                }
                v
            }
        };
        value["jsonrpc"] = Value::String("2.0".into());
        value
    }

    /// Classifies a JSON object as one of the three message kinds.
    ///
    /// The distinguishing rule is the one from the specification: a `method`
    /// with an `id` is a request, `method` alone is a notification, and an
    /// `id` with `result` or `error` is a response.
    pub fn from_json(value: Value) -> Result<Message> {
        let object =
            value.as_object().ok_or_else(|| LspError::Protocol("not a JSON object".into()))?;
        let id = object.get("id").filter(|v| !v.is_null()).cloned();
        let method = object.get("method").and_then(Value::as_str).map(str::to_string);
        let params = object.get("params").cloned().unwrap_or(Value::Null);

        match (method, id) {
            (Some(method), Some(id)) => {
                let id = serde_json::from_value(id)?;
                Ok(Message::Request(Request { id, method, params }))
            }
            (Some(method), None) => Ok(Message::Notification(Notification { method, params })),
            (None, id) => {
                let id = id.map(serde_json::from_value).transpose()?;
                let error = object
                    .get("error")
                    .filter(|v| !v.is_null())
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()?;
                Ok(Message::Response(Response {
                    id,
                    result: object.get("result").cloned(),
                    error,
                }))
            }
        }
    }

    /// The message with `Content-Length` framing, ready to write.
    pub fn encode(&self) -> Vec<u8> {
        let body = serde_json::to_vec(&self.to_json()).expect("messages are serialisable");
        let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(&body);
        out
    }
}

/// The result of trying to decode one message from a buffer.
#[derive(Debug, PartialEq)]
pub enum Decoded {
    /// A message, and how many bytes it consumed.
    Message(Box<Message>, usize),
    /// More bytes are needed.
    Incomplete,
}

/// Decodes the first framed message in `buffer`.
///
/// Headers are parsed case-insensitively and any header other than
/// `Content-Length` is ignored, which is what the specification requires of a
/// client that does not negotiate `Content-Type`.
pub fn decode(buffer: &[u8]) -> Result<Decoded> {
    const SEPARATOR: &[u8] = b"\r\n\r\n";
    let Some(header_end) = find(buffer, SEPARATOR) else {
        // Guard against a peer that never sends a separator.
        if buffer.len() > 64 * 1024 {
            return Err(LspError::Protocol("header block is implausibly long".into()));
        }
        return Ok(Decoded::Incomplete);
    };
    let headers = std::str::from_utf8(&buffer[..header_end])
        .map_err(|_| LspError::Protocol("headers are not valid UTF-8".into()))?;

    let mut content_length = None;
    for line in headers.split("\r\n").filter(|l| !l.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return Err(LspError::Protocol(format!("header line without a colon: `{line}`")));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| LspError::Protocol(format!("bad Content-Length: `{value}`")))?,
            );
        }
    }
    let length =
        content_length.ok_or_else(|| LspError::Protocol("no Content-Length header".into()))?;

    let body_start = header_end + SEPARATOR.len();
    let body_end = body_start + length;
    if buffer.len() < body_end {
        return Ok(Decoded::Incomplete);
    }
    let value: Value = serde_json::from_slice(&buffer[body_start..body_end])?;
    Ok(Decoded::Message(Box::new(Message::from_json(value)?), body_end))
}

/// Index of the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Message {
        Message::Request(Request {
            id: RequestId::Number(1),
            method: "textDocument/hover".into(),
            params: json!({"line": 3}),
        })
    }

    #[test]
    fn a_request_encodes_with_content_length_framing() {
        let bytes = request().encode();
        let text = String::from_utf8(bytes).unwrap();
        let (headers, body) = text.split_once("\r\n\r\n").unwrap();
        assert_eq!(headers, format!("Content-Length: {}", body.len()));
        assert!(body.contains(r#""jsonrpc":"2.0""#));
        assert!(body.contains(r#""method":"textDocument/hover""#));
    }

    #[test]
    fn messages_round_trip_through_encode_and_decode() {
        for message in [
            request(),
            Message::Notification(Notification {
                method: "initialized".into(),
                params: json!({}),
            }),
            Message::Response(Response {
                id: Some(RequestId::Number(7)),
                result: Some(json!({"contents": "docs"})),
                error: None,
            }),
        ] {
            let bytes = message.encode();
            let Decoded::Message(decoded, used) = decode(&bytes).unwrap() else {
                panic!("expected a complete message");
            };
            assert_eq!(*decoded, message);
            assert_eq!(used, bytes.len());
        }
    }

    #[test]
    fn a_partial_message_reports_incomplete() {
        let bytes = request().encode();
        assert_eq!(decode(&bytes[..10]).unwrap(), Decoded::Incomplete, "headers cut short");
        assert_eq!(
            decode(&bytes[..bytes.len() - 5]).unwrap(),
            Decoded::Incomplete,
            "body cut short"
        );
        assert_eq!(decode(b"").unwrap(), Decoded::Incomplete);
    }

    #[test]
    fn decoding_reports_how_much_it_consumed_so_streams_can_continue() {
        let mut stream = request().encode();
        let second = Message::Notification(Notification {
            method: "exit".into(),
            params: Value::Null,
        });
        stream.extend_from_slice(&second.encode());

        let Decoded::Message(first, used) = decode(&stream).unwrap() else { panic!() };
        assert_eq!(*first, request());
        let Decoded::Message(rest, _) = decode(&stream[used..]).unwrap() else { panic!() };
        assert_eq!(*rest, second);
    }

    #[test]
    fn extra_headers_are_ignored_and_names_are_case_insensitive() {
        let body = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        let framed = format!(
            "content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}",
            body.len()
        );
        let Decoded::Message(m, _) = decode(framed.as_bytes()).unwrap() else { panic!() };
        assert!(matches!(*m, Message::Notification(_)));
    }

    #[test]
    fn a_missing_content_length_is_an_error() {
        let err = decode(b"Content-Type: x\r\n\r\n{}").unwrap_err();
        assert!(err.to_string().contains("no Content-Length"));
    }

    #[test]
    fn a_malformed_content_length_is_an_error() {
        let err = decode(b"Content-Length: abc\r\n\r\n{}").unwrap_err();
        assert!(err.to_string().contains("bad Content-Length"));
    }

    #[test]
    fn a_header_line_without_a_colon_is_an_error() {
        let err = decode(b"garbage\r\n\r\n{}").unwrap_err();
        assert!(err.to_string().contains("without a colon"));
    }

    #[test]
    fn an_endless_header_block_is_rejected_rather_than_buffered_forever() {
        let flood = vec![b'x'; 70 * 1024];
        assert!(decode(&flood).is_err());
    }

    #[test]
    fn a_body_that_is_not_json_is_an_error() {
        let framed = b"Content-Length: 3\r\n\r\nnot";
        assert!(matches!(decode(framed), Err(LspError::Json(_))));
    }

    #[test]
    fn message_kinds_are_told_apart_by_method_and_id() {
        let r = Message::from_json(json!({"jsonrpc":"2.0","id":1,"method":"m"})).unwrap();
        assert!(matches!(r, Message::Request(_)));

        let n = Message::from_json(json!({"jsonrpc":"2.0","method":"m"})).unwrap();
        assert!(matches!(n, Message::Notification(_)));

        let ok = Message::from_json(json!({"jsonrpc":"2.0","id":1,"result":null})).unwrap();
        assert!(matches!(ok, Message::Response(_)));
    }

    #[test]
    fn a_null_id_is_treated_as_absent() {
        // Servers send `"id": null` on a parse error they cannot attribute.
        let m = Message::from_json(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse"}}))
            .unwrap();
        let Message::Response(r) = m else { panic!("expected a response") };
        assert!(r.id.is_none());
        assert_eq!(r.error.unwrap().code, -32700);
    }

    #[test]
    fn string_request_ids_are_supported() {
        let m = Message::from_json(json!({"jsonrpc":"2.0","id":"abc","method":"m"})).unwrap();
        let Message::Request(r) = m else { panic!() };
        assert_eq!(r.id, RequestId::String("abc".into()));
        assert_eq!(r.id.to_string(), "abc");
    }

    #[test]
    fn an_error_response_carries_its_code_and_message() {
        let m = Message::from_json(json!({
            "jsonrpc":"2.0","id":4,
            "error":{"code":-32601,"message":"method not found"}
        }))
        .unwrap();
        let Message::Response(r) = m else { panic!() };
        let e = r.error.unwrap();
        assert_eq!(e.code, -32601);
        assert_eq!(e.message, "method not found");
        assert!(r.result.is_none());
    }

    #[test]
    fn a_non_object_payload_is_rejected() {
        assert!(Message::from_json(json!([1, 2, 3])).is_err());
        assert!(Message::from_json(json!("string")).is_err());
    }

    #[test]
    fn a_successful_response_always_carries_a_result_field() {
        let encoded = Message::Response(Response { id: Some(1.into()), result: None, error: None })
            .to_json();
        assert!(encoded.get("result").is_some());
    }

    #[test]
    fn params_are_omitted_when_null() {
        let encoded = Message::Notification(Notification {
            method: "exit".into(),
            params: Value::Null,
        })
        .to_json();
        assert!(encoded.get("params").is_none());
    }

    #[test]
    fn framing_counts_bytes_not_characters() {
        let message = Message::Notification(Notification {
            method: "m".into(),
            params: json!({"text": "héllo wörld ünïcode"}),
        });
        let bytes = message.encode();
        let Decoded::Message(decoded, used) = decode(&bytes).unwrap() else { panic!() };
        assert_eq!(*decoded, message);
        assert_eq!(used, bytes.len());
    }
}
