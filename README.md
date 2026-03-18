# tokio-jrpc

A simple, typed async [JSON-RPC 2.0](https://www.jsonrpc.org/specification) client and server built on [Tokio](https://tokio.rs). Works over any `AsyncRead + AsyncWrite` transport using newline-delimited JSON.

## Usage

```rust
use serde::Deserialize;
use tokio::io;
use tokio_jrpc::{App, Error};

#[derive(Deserialize)]
struct AddParams {
    a: i64,
    b: i64
}

#[derive(Deserialize)]
struct LogParams {
    message: String
}

#[tokio::main]
async fn main() {
    let app = App::new(io::stdin(), io::stdout(), ())
        .method("add", async |params: AddParams, _client, _state| -> Result<i64, Error> {
            Ok(params.a + params.b)
        })
        .notification("log", async |params: LogParams, _client, _state| {
            println!("{}", params.message);
        });

    app.run().await.unwrap();
}
```


Handlers receive a `ClientHandle` that can send requests and notifications back to the remote peer, but `ClientHandle`s can also be obtained via `app.client_handle()` before running the app:

```rust
#[derive(Serialize)]
struct GetTodoParams {
    id: u64
}

#[derive(Deserialize)]
struct Todo {
    id: u64,
    name: String,
    is_done: bool
}

let client = app.client_handle();

tokio::spawn(async move {
    let todo: Todo = client.request("get_todo", GetTodoParams { id: 1 }).await.unwrap();
    // ...
    client.notify("some_event", ()).unwrap();
});

app.run().await.unwrap();
```


`App::new` takes a third argument for shared state, which is cloned and passed into every handler. This is useful for sharing things like database connections or configuration:

```rust
let shared = Arc::new(MyState { /* ... */ });

let app = App::new(reader, writer, shared)
    .method("get_status", async |_params: (), _client, state: Arc<MyState>| -> Result<Status, Error> {
        Ok(state.status())
    });
```

## License

MIT