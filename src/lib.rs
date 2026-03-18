use std::collections::HashMap;
use std::future::Future;
use std::fmt;
use std::io;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::select;
use tokio::sync::{mpsc, oneshot};

use crate::types::{ErrorResponse, Id, Message, Notification, Params, Request, Response, SuccessResponse, Version};
pub use crate::types::{Error, ErrorCode};

mod types;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    SendFailed,
    ResponseChannelClosed,
    MessageChannelClosed,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SendFailed => write!(f, "message channel send failed"),
            Self::ResponseChannelClosed => write!(f, "response channel closed"),
            Self::MessageChannelClosed => write!(f, "message channel closed"),
        }
    }
}

impl std::error::Error for ChannelError {}

#[derive(Debug)]
pub enum SerializationError {
    SerializeParams(serde_json::Error),
    DeserializeResult(serde_json::Error),
    SerializeMessage(serde_json::Error),
}

impl fmt::Display for SerializationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SerializeParams(_) => write!(f, "failed to serialize params"),
            Self::DeserializeResult(_) => write!(f, "failed to deserialize response result"),
            Self::SerializeMessage(_) => write!(f, "failed to serialize message"),
        }
    }
}

impl std::error::Error for SerializationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SerializeParams(err) => Some(err),
            Self::DeserializeResult(err) => Some(err),
            Self::SerializeMessage(err) => Some(err),
        }
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    InvalidParamsShape,
    Remote(Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParamsShape => {
                write!(f, "invalid params shape, expected object, array, or null")
            }
            Self::Remote(err) => write!(f, "remote error: {}", err.message),
        }
    }
}

impl std::error::Error for ProtocolError {}

#[derive(Debug)]
pub enum ClientError {
    Serialization(SerializationError),
    Protocol(ProtocolError),
    Channel(ChannelError),
}

impl From<SerializationError> for ClientError {
    fn from(value: SerializationError) -> Self {
        Self::Serialization(value)
    }
}

impl From<ProtocolError> for ClientError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}

impl From<ChannelError> for ClientError {
    fn from(value: ChannelError) -> Self {
        Self::Channel(value)
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialization(err) => err.fmt(f),
            Self::Protocol(err) => err.fmt(f),
            Self::Channel(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialization(err) => Some(err),
            Self::Protocol(err) => Some(err),
            Self::Channel(err) => Some(err),
        }
    }
}

#[derive(Debug)]
pub enum TransportError {
    Read(io::Error),
    Write(io::Error),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(_) => write!(f, "failed to read from transport"),
            Self::Write(_) => write!(f, "failed to write to transport"),
        }
    }
}

impl std::error::Error for TransportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(err) => Some(err),
            Self::Write(err) => Some(err),
        }
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    Transport(TransportError),
    Serialization(SerializationError),
    Channel(ChannelError),
}

impl From<SerializationError> for RuntimeError {
    fn from(value: SerializationError) -> Self {
        Self::Serialization(value)
    }
}

impl From<ChannelError> for RuntimeError {
    fn from(value: ChannelError) -> Self {
        Self::Channel(value)
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(err) => err.fmt(f),
            Self::Serialization(err) => err.fmt(f),
            Self::Channel(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(err) => Some(err),
            Self::Serialization(err) => Some(err),
            Self::Channel(err) => Some(err),
        }
    }
}

#[derive(Debug)]
enum ChannelMessage {
    Request {
        method: String,
        params: Option<Params>,
        response_tx: oneshot::Sender<Response>,
    },
    Response(Response),
    Notification(Notification),
}

#[derive(Clone)]
pub struct ClientHandle {
    tx: mpsc::UnboundedSender<ChannelMessage>,
}

fn parse_error_response() -> Response {
    Response::Error(ErrorResponse {
        jsonrpc: Some(Version::V2),
        error: Error {
            code: ErrorCode::ParseError,
            message: "Parse error".to_string(),
            data: None,
        },
        id: Id::Null,
    })
}

fn invalid_request_response() -> Response {
    Response::Error(ErrorResponse {
        jsonrpc: Some(Version::V2),
        error: Error {
            code: ErrorCode::InvalidRequest,
            message: "Invalid Request".to_string(),
            data: None,
        },
        id: Id::Null,
    })
}

fn method_not_found_response(id: Id) -> Response {
    Response::Error(ErrorResponse {
        jsonrpc: Some(Version::V2),
        error: Error {
            code: ErrorCode::MethodNotFound,
            message: "Method not found".to_string(),
            data: None,
        },
        id,
    })
}

fn invalid_params_response(id: Id, message: String) -> Response {
    Response::Error(ErrorResponse {
        jsonrpc: Some(Version::V2),
        error: Error {
            code: ErrorCode::InvalidParams,
            message,
            data: None,
        },
        id,
    })
}

fn internal_error_response(id: Id, message: String) -> Response {
    Response::Error(ErrorResponse {
        jsonrpc: Some(Version::V2),
        error: Error {
            code: ErrorCode::InternalError,
            message,
            data: None,
        },
        id,
    })
}

fn serialize_params(params: impl Serialize) -> Result<Option<Params>, ClientError> {
    let value = serde_json::to_value(params)
        .map_err(|err| ClientError::Serialization(SerializationError::SerializeParams(err)))?;

    match value {
        serde_json::Value::Array(value) => Ok(Some(Params::Array(value))),
        serde_json::Value::Object(value) => Ok(Some(Params::Object(value))),
        serde_json::Value::Null => Ok(None),
        _ => Err(ClientError::Protocol(ProtocolError::InvalidParamsShape)),
    }
}

impl ClientHandle {
    fn new(tx: mpsc::UnboundedSender<ChannelMessage>) -> Self {
        Self { tx }
    }

    pub async fn request<R: DeserializeOwned>(
        &self,
        method: impl Into<String>,
        params: impl Serialize,
    ) -> Result<R, ClientError> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(ChannelMessage::Request {
                method: method.into(),
                params: serialize_params(params)?,
                response_tx,
            })
            .map_err(|_| ClientError::Channel(ChannelError::SendFailed))?;

        let response = response_rx
            .await
            .map_err(|_| ClientError::Channel(ChannelError::ResponseChannelClosed))?;

        match response {
            Response::Success(success_response) => {
                serde_json::from_value(success_response.result)
                    .map_err(|err| {
                        ClientError::Serialization(SerializationError::DeserializeResult(err))
                    })
            },
            Response::Error(error_response) => {
                Err(ClientError::Protocol(ProtocolError::Remote(error_response.error)))
            }
        }
    }

    pub fn notify(&self, method: impl Into<String>, params: impl Serialize) -> Result<(), ClientError> {
        let notification = Notification {
            jsonrpc: Some(Version::V2),
            method: method.into(),
            params: serialize_params(params)?,
        };

        self.tx
            .send(ChannelMessage::Notification(notification))
            .map_err(|_| ClientError::Channel(ChannelError::SendFailed))
    }
}

type MethodHandler<S> = Box<dyn Fn(Request, mpsc::UnboundedSender<ChannelMessage>, S) + Send>;

type NotificationHandler<S> =
    Box<dyn Fn(Notification, mpsc::UnboundedSender<ChannelMessage>, S) + Send>;

pub struct App<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, S> {
    reader: Lines<BufReader<R>>,
    writer: BufWriter<W>,
    method_handlers: HashMap<String, MethodHandler<S>>,
    notification_handlers: HashMap<String, NotificationHandler<S>>,
    response_receiver: HashMap<Id, oneshot::Sender<Response>>,
    message_tx: mpsc::UnboundedSender<ChannelMessage>,
    message_rx: mpsc::UnboundedReceiver<ChannelMessage>,
    next_request_id: i64,
    state: S,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin, S: Clone> App<R, W, S> {
    pub fn new(reader: R, writer: W, state: S) -> Self {
        let (message_tx, message_rx) = mpsc::unbounded_channel();

        Self {
            reader: BufReader::new(reader).lines(),
            writer: BufWriter::new(writer),
            method_handlers: HashMap::new(),
            notification_handlers: HashMap::new(),
            response_receiver: HashMap::new(),
            message_tx,
            message_rx,
            next_request_id: 1,
            state,
        }
    }

    pub fn method<P, T, E, Fut, H>(mut self, method: impl Into<String>, handler: H) -> Self
    where
        P: DeserializeOwned + Send + 'static,
        T: Serialize,
        E: Into<Error>,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        H: Fn(P, ClientHandle, S) -> Fut + Send + 'static,
    {
        let boxed_handler =
            move |request: Request, message_tx: mpsc::UnboundedSender<ChannelMessage>, state: S| {
                let Request { params, id, .. } = request;
                let params_value = params.map(Into::into).unwrap_or(serde_json::Value::Null);
                match serde_json::from_value::<P>(params_value) {
                    Ok(params) => {
                        let result_fut = handler(params, ClientHandle::new(message_tx.clone()), state);
                        tokio::spawn(async move {
                            let response = match result_fut.await {
                                Ok(value) => match serde_json::to_value(value) {
                                    Ok(result) => Response::Success(SuccessResponse {
                                        jsonrpc: Some(Version::V2),
                                        result,
                                        id,
                                    }),
                                    Err(err) => internal_error_response(id, err.to_string()),
                                },
                                Err(err) => Response::Error(ErrorResponse {
                                    jsonrpc: Some(Version::V2),
                                    error: err.into(),
                                    id,
                                }),
                            };
                            let _ = message_tx.send(ChannelMessage::Response(response));
                        });
                    }
                    Err(err) => {
                        let _ = message_tx.send(ChannelMessage::Response(invalid_params_response(
                            id,
                            err.to_string(),
                        )));
                    }
                }
            };
        self.method_handlers.insert(method.into(), Box::new(boxed_handler));
        self
    }

    pub fn notification<M, P, Fut, H>(mut self, method: M, handler: H) -> Self
    where
        M: Into<String>,
        P: DeserializeOwned + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
        H: Fn(P, ClientHandle, S) -> Fut + Send + 'static,
    {
        let boxed_handler = 
            move |notification: Notification, message_tx: mpsc::UnboundedSender<ChannelMessage>, state: S| {
                let Notification { params, .. } = notification;
                let params_value = params.map(Into::into).unwrap_or(serde_json::Value::Null);

                if let Ok(params) = serde_json::from_value::<P>(params_value) {
                    tokio::spawn(handler(params, ClientHandle::new(message_tx.clone()), state));
                }
            };
        self.notification_handlers.insert(method.into(), Box::new(boxed_handler));
        self
    }

    pub fn client_handle(&self) -> ClientHandle {
        ClientHandle::new(self.message_tx.clone())
    }

    fn enqueue_message(&self, message: ChannelMessage) -> Result<(), RuntimeError> {
        self.message_tx
            .send(message)
            .map_err(|_| RuntimeError::Channel(ChannelError::MessageChannelClosed))
    }

    fn handle_inbound_message(&mut self, message: Message) -> Result<(), RuntimeError> {
        match message {
            Message::Request(request) => {
                if let Some(handle_method) = self.method_handlers.get(&request.method) {
                    handle_method(request, self.message_tx.clone(), self.state.clone());
                } else {
                    self.enqueue_message(ChannelMessage::Response(method_not_found_response(request.id)))?;
                }
            }
            Message::Notification(notification) => {
                if let Some(handle_notification) = self.notification_handlers.get(&notification.method) {
                    handle_notification(notification, self.message_tx.clone(), self.state.clone());
                }
            }
            Message::Response(response) => {
                if let Some(response_tx) = self.response_receiver.remove(response.id()) {
                    let _ = response_tx.send(response);
                }
            }
        }

        Ok(())
    }

    fn process_single_value(&mut self, value: serde_json::Value) -> Result<(), RuntimeError> {
        match serde_json::from_value::<Message>(value) {
            Ok(message) => self.handle_inbound_message(message),
            Err(_) => self.enqueue_message(ChannelMessage::Response(invalid_request_response())),
        }
    }

    fn process_incoming_value(&mut self, value: serde_json::Value) -> Result<(), RuntimeError> {
        match value {
            serde_json::Value::Array(items) => {
                if items.is_empty() {
                    self.enqueue_message(ChannelMessage::Response(invalid_request_response()))?;
                    return Ok(());
                }

                for item in items {
                    self.process_single_value(item)?;
                }
            }
            value => self.process_single_value(value)?,
        }
        Ok(())
    }

    async fn write_message(&mut self, message: &Message) -> Result<(), RuntimeError> {
        let message_bytes = serde_json::to_vec(message)
            .map_err(|err| RuntimeError::Serialization(SerializationError::SerializeMessage(err)))?;

        self.writer
            .write_all(&message_bytes)
            .await
            .map_err(|err| RuntimeError::Transport(TransportError::Write(err)))?;
        self.writer
            .write_u8(b'\n')
            .await
            .map_err(|err| RuntimeError::Transport(TransportError::Write(err)))?;
        self.writer
            .flush()
            .await
            .map_err(|err| RuntimeError::Transport(TransportError::Write(err)))
    }

    pub async fn run(mut self) -> Result<(), RuntimeError> {
        loop {
            select! {
                line_res = self.reader.next_line() => {
                    match line_res {
                        Ok(Some(line)) => {
                            match serde_json::from_str::<serde_json::Value>(line.as_str()) {
                                Ok(value) => self.process_incoming_value(value)?,
                                Err(_) => self.enqueue_message(ChannelMessage::Response(parse_error_response()))?,
                            }
                        }
                        Ok(None) => return Ok(()),
                        Err(err) => return Err(RuntimeError::Transport(TransportError::Read(err))),
                    }
                }
                send_res = self.message_rx.recv() => {
                    match send_res {
                        Some(ChannelMessage::Request { method, params, response_tx }) => {
                            let id = Id::Number(self.next_request_id);
                            self.next_request_id += 1;
                            let request = Request {
                                jsonrpc: Some(Version::V2),
                                method,
                                params,
                                id: id.clone(),
                            };
                            self.write_message(&Message::Request(request)).await?;
                            self.response_receiver.insert(id, response_tx);
                        }
                        Some(ChannelMessage::Response(response)) => {
                            self.write_message(&Message::Response(response)).await?;
                        }
                        Some(ChannelMessage::Notification(notification)) => {
                            self.write_message(&Message::Notification(notification)).await?;
                        }
                        None => return Err(RuntimeError::Channel(ChannelError::MessageChannelClosed)),
                    }
                }
            }
        }
    }
}