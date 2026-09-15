use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl From<u32> for RequestId {
    fn from(value: u32) -> Self {
        Self::Number(i64::from(value))
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => value.fmt(formatter),
            Self::String(value) => value.fmt(formatter),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum IncomingMessage {
    Response(ServerResponse),
    Notification(ServerNotification),
    Request(ServerRequest),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerResponse {
    pub id: RequestId,
    pub outcome: Result<Value, RemoteError>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RemoteError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerRequest {
    pub id: RequestId,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Codex sent invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Codex sent a message that is not an object")]
    ExpectedObject,
    #[error("Codex message is missing `{0}`")]
    MissingField(&'static str),
    #[error("Codex message has an invalid `{0}`")]
    InvalidField(&'static str),
    #[error("Codex response must contain exactly one of `result` or `error`")]
    InvalidResponse,
}

pub fn encode_request<T: Serialize>(
    id: RequestId,
    method: &str,
    params: &T,
) -> Result<String, ProtocolError> {
    let mut object = Map::new();
    object.insert("id".to_owned(), serde_json::to_value(id)?);
    object.insert("method".to_owned(), Value::String(method.to_owned()));
    object.insert("params".to_owned(), serde_json::to_value(params)?);
    encode_object(object)
}

pub fn encode_notification<T: Serialize>(
    method: &str,
    params: &T,
) -> Result<String, ProtocolError> {
    let mut object = Map::new();
    object.insert("method".to_owned(), Value::String(method.to_owned()));
    object.insert("params".to_owned(), serde_json::to_value(params)?);
    encode_object(object)
}

pub fn encode_response<T: Serialize>(id: RequestId, result: &T) -> Result<String, ProtocolError> {
    let mut object = Map::new();
    object.insert("id".to_owned(), serde_json::to_value(id)?);
    object.insert("result".to_owned(), serde_json::to_value(result)?);
    encode_object(object)
}

pub fn encode_error_response(id: RequestId, error: &RemoteError) -> Result<String, ProtocolError> {
    let mut object = Map::new();
    object.insert("id".to_owned(), serde_json::to_value(id)?);
    object.insert("error".to_owned(), serde_json::to_value(error)?);
    encode_object(object)
}

pub fn decode_line(line: &str) -> Result<IncomingMessage, ProtocolError> {
    let value: Value = serde_json::from_str(line)?;
    let mut object = value
        .as_object()
        .cloned()
        .ok_or(ProtocolError::ExpectedObject)?;
    let method = object.remove("method");
    let id = object.remove("id");

    match (method, id) {
        (Some(method), Some(id)) => decode_request(method, id, object),
        (Some(method), None) => decode_notification(method, object),
        (None, Some(id)) => decode_response(id, object),
        (None, None) => Err(ProtocolError::MissingField("method or id")),
    }
}

fn encode_object(object: Map<String, Value>) -> Result<String, ProtocolError> {
    let mut line = serde_json::to_string(&Value::Object(object))?;
    line.push('\n');
    Ok(line)
}

fn decode_request(
    method: Value,
    id: Value,
    mut object: Map<String, Value>,
) -> Result<IncomingMessage, ProtocolError> {
    Ok(IncomingMessage::Request(ServerRequest {
        id: parse_id(id)?,
        method: parse_method(method)?,
        params: object.remove("params").unwrap_or(Value::Null),
    }))
}

fn decode_notification(
    method: Value,
    mut object: Map<String, Value>,
) -> Result<IncomingMessage, ProtocolError> {
    Ok(IncomingMessage::Notification(ServerNotification {
        method: parse_method(method)?,
        params: object.remove("params").unwrap_or(Value::Null),
    }))
}

fn decode_response(
    id: Value,
    mut object: Map<String, Value>,
) -> Result<IncomingMessage, ProtocolError> {
    let result = object.remove("result");
    let error = object.remove("error");
    let outcome = match (result, error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(serde_json::from_value(error)?),
        _ => return Err(ProtocolError::InvalidResponse),
    };
    Ok(IncomingMessage::Response(ServerResponse {
        id: parse_id(id)?,
        outcome,
    }))
}

fn parse_method(value: Value) -> Result<String, ProtocolError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or(ProtocolError::InvalidField("method"))
}

fn parse_id(value: Value) -> Result<RequestId, ProtocolError> {
    serde_json::from_value(value).map_err(|_| ProtocolError::InvalidField("id"))
}
