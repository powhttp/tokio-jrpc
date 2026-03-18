use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio_jrpc::{App, ChannelError, ClientError, ProtocolError, RuntimeError, TransportError};
use serde::Deserialize;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream};

async fn read_json_line(reader: &mut BufReader<DuplexStream>) -> serde_json::Value {
    let mut line = String::new();
    let bytes_read = reader.read_line(&mut line).await.unwrap();
    assert!(bytes_read > 0);
    serde_json::from_str(&line).unwrap()
}

#[tokio::test]
async fn returns_parse_error_for_invalid_json() {
    let (mut input, app_reader) = tokio::io::duplex(1024);
    let (app_writer, output) = tokio::io::duplex(1024);
    let app = App::new(app_reader, app_writer, ());
    let run_handle = tokio::spawn(app.run());
    let mut output_reader = BufReader::new(output);

    input.write_all(b"{\n").await.unwrap();
    let response = read_json_line(&mut output_reader).await;

    assert_eq!(response["error"]["code"], -32700);
    assert_eq!(response["error"]["message"], "Parse error");
    assert_eq!(response["id"], serde_json::Value::Null);

    drop(input);
    let run_result = run_handle.await.unwrap();
    assert!(run_result.is_ok());
}

#[tokio::test]
async fn returns_invalid_request_for_empty_batch() {
    let (mut input, app_reader) = tokio::io::duplex(1024);
    let (app_writer, output) = tokio::io::duplex(1024);
    let app = App::new(app_reader, app_writer, ());
    let run_handle = tokio::spawn(app.run());
    let mut output_reader = BufReader::new(output);

    input.write_all(b"[]\n").await.unwrap();
    let response = read_json_line(&mut output_reader).await;

    assert_eq!(response["error"]["code"], -32600);
    assert_eq!(response["error"]["message"], "Invalid Request");
    assert_eq!(response["id"], serde_json::Value::Null);

    drop(input);
    let run_result = run_handle.await.unwrap();
    assert!(run_result.is_ok());
}

#[tokio::test]
async fn returns_method_not_found_with_message() {
    let (mut input, app_reader) = tokio::io::duplex(1024);
    let (app_writer, output) = tokio::io::duplex(1024);
    let app = App::new(app_reader, app_writer, ());
    let run_handle = tokio::spawn(app.run());
    let mut output_reader = BufReader::new(output);

    let request = json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "missing_method",
        "params": {}
    });

    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let response = read_json_line(&mut output_reader).await;

    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], "Method not found");
    assert_eq!(response["id"], 12);

    drop(input);
    let run_result = run_handle.await.unwrap();
    assert!(run_result.is_ok());
}

#[derive(Deserialize)]
struct NeedsParams {
    value: String,
}

#[tokio::test]
async fn returns_invalid_params_when_request_params_missing() {
    let (mut input, app_reader) = tokio::io::duplex(1024);
    let (app_writer, output) = tokio::io::duplex(1024);
    
    let app = App::new(app_reader, app_writer, ())
        .method("echo", |params: NeedsParams, _, _| async move {
            Ok::<_, io::Error>(params.value)
        });

    let run_handle = tokio::spawn(app.run());
    let mut output_reader = BufReader::new(output);

    let request = json!({
        "jsonrpc": "2.0",
        "id": "req-1",
        "method": "echo"
    });

    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let response = read_json_line(&mut output_reader).await;

    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(response["id"], "req-1");

    drop(input);
    let run_result = run_handle.await.unwrap();
    assert!(run_result.is_ok());
}

#[tokio::test]
async fn client_returns_typed_errors() {
    let (app_reader, _) = tokio::io::duplex(1024);
    let (app_writer, _) = tokio::io::duplex(1024);
    let app = App::new(app_reader, app_writer, ());
    let client = app.client_handle();

    let invalid_shape = client.request::<serde_json::Value>("method", 1).await.unwrap_err();
    assert!(matches!(
        invalid_shape,
        ClientError::Protocol(ProtocolError::InvalidParamsShape)
    ));

    drop(app);
    let send_failed = client.notify("method", json!({})).unwrap_err();
    assert!(matches!(
        send_failed,
        ClientError::Channel(ChannelError::SendFailed)
    ));
}

struct FailingWriter;

impl AsyncWrite for FailingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::Error::other("write failed")))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("flush failed")))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn run_returns_typed_write_error() {
    let (_input, app_reader) = tokio::io::duplex(1024);
    let app = App::new(app_reader, FailingWriter, ());
    let client = app.client_handle();
    let run_handle = tokio::spawn(app.run());

    client.notify("event", json!({})).unwrap();

    let run_result = run_handle.await.unwrap();
    assert!(matches!(
        run_result,
        Err(RuntimeError::Transport(TransportError::Write(_)))
    ));
}
