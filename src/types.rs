use serde::{Serialize, Deserialize};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Version {
    #[serde(rename = "2.0")]
    V2,
    #[serde(untagged)]
    Other(String)
}

#[derive(Debug, PartialEq, Clone, Hash, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Null,
    Number(i64),
    String(String)
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Params {
    Array(Vec<serde_json::Value>),
    Object(serde_json::Map<String, serde_json::Value>)
}

impl From<Params> for serde_json::Value {
    fn from(value: Params) -> serde_json::Value {
        match value {
            Params::Array(array) => serde_json::Value::Array(array),
            Params::Object(object) => serde_json::Value::Object(object)
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: Option<Version>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Params>,
    pub id: Id
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: Option<Version>,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Params>,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Success(SuccessResponse),
    Error(ErrorResponse)
}

impl Response {
    pub fn id(&self) -> &Id {
        match self {
            Self::Success(success_response) => &success_response.id,
            Self::Error(error_response) => &error_response.id
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub jsonrpc: Option<Version>,
    pub result: serde_json::Value,
    pub id: Id,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub jsonrpc: Option<Version>,
    pub error: Error,
    pub id: Id,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl<E: std::error::Error> From<E> for Error {
    fn from(err: E) -> Self {
        Self { 
            code: ErrorCode::InternalError,
            message: err.to_string(), 
            data: None
        }
    }
}

#[derive(Debug, PartialEq, Clone, Hash, Eq, Serialize, Deserialize)]
#[serde(into = "i32", from = "i32")]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    ServerError(i32),
    Other(i32)
}

impl From<ErrorCode> for i32 {
    fn from(code: ErrorCode) -> i32 {
        match code {
            ErrorCode::ParseError => -32700,
            ErrorCode::InvalidRequest => -32600,
            ErrorCode::MethodNotFound => -32601,
            ErrorCode::InvalidParams => -32602,
            ErrorCode::InternalError => -32603,
            ErrorCode::ServerError(code) => code,
            ErrorCode::Other(code) => code,
        }
    }
}

impl From<i32> for ErrorCode {
    fn from(code: i32) -> ErrorCode {
        match code {
            -32700 => ErrorCode::ParseError,
            -32600 => ErrorCode::InvalidRequest,
            -32601 => ErrorCode::MethodNotFound,
            -32602 => ErrorCode::InvalidParams,
            -32603 => ErrorCode::InternalError,
            code if code >= -32099 && code <= -32000 => ErrorCode::ServerError(code),
            other => ErrorCode::Other(other),
        }
    }
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Notification(Notification),
    Response(Response),
}