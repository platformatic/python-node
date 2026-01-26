# python-node Source Code Architecture

## Overview

This directory contains the Rust source code for `@platformatic/python-node`, a high-performance Node.js native addon that enables running ASGI 3.0-compatible Python applications directly within Node.js processes. The implementation bridges three runtime environments: Node.js (JavaScript), Rust (native code), and Python (ASGI applications), providing near-zero overhead communication between them.

### What This Library Does

The library acts as an **ASGI server embedded in Node.js**. It:
- Translates Node.js HTTP requests into Python ASGI protocol messages
- Manages Python asyncio event loops from Rust's tokio runtime
- Handles bidirectional streaming of request/response data
- Supports HTTP, WebSocket, and Lifespan protocols per ASGI 3.0 specification
- Provides both synchronous and asynchronous APIs for Node.js

### Key Architecture Principles

1. **Three-Runtime Bridge**: Coordinates execution across JavaScript (Node.js), Rust (native code), and Python (asyncio)
2. **Zero-Copy Streaming**: Uses duplex streams for efficient data transfer without unnecessary buffering
3. **Shared Event Loop**: Maintains a single Python asyncio event loop shared across all requests
4. **Async-First Design**: Built on tokio for Rust async and pyo3-async-runtimes for Python async integration
5. **ASGI 3.0 Compliance**: Full implementation of ASGI HTTP, WebSocket, and Lifespan protocols

---

## Directory Structure

```
src/
├── lib.rs              # Main entry point: NAPI bindings and public API
└── asgi/               # ASGI protocol implementation
    ├── mod.rs          # Core ASGI handler, Python integration, request routing
    ├── http.rs         # HTTP connection scope and message types
    ├── websocket.rs    # WebSocket connection scope and message types
    ├── lifespan.rs     # Lifespan protocol (startup/shutdown events)
    ├── receiver.rs     # Python→Rust message receiver (callable from Python)
    ├── sender.rs       # Rust→Python message sender with acknowledgments
    ├── event_loop_handle.rs    # Python asyncio event loop management
    ├── runtime_handle.rs       # Tokio runtime fallback management
    ├── python_future_poller.rs # Python future polling in Rust
    ├── http_method.rs  # HTTP method enum with Python conversions
    ├── http_version.rs # HTTP version enum with Python conversions
    └── info.rs         # ASGI version info structure
```

---

## Core Components

### 1. Main Entry Point: `lib.rs`

**Responsibilities:**
- Exports the `PythonHandler` class to Node.js via NAPI-RS
- Defines `PythonHandlerTarget` for specifying Python module:function
- Implements three request handling modes:
  - `handle_request()`: Async with buffered body (backward compatible)
  - `handle_stream()`: Async with streaming body (efficient)
  - `handle_request_sync()`: Synchronous blocking call
- Defines the `HandlerError` enum for error propagation

**Key Types:**

```rust
pub struct PythonHandlerTarget {
    pub file: String,     // Python module name (without .py)
    pub function: String, // Function name in module
}

pub struct PythonHandler {
    asgi: Arc<Asgi>,  // Shared ASGI handler
}
```

**Request Flow:**

1. JavaScript calls `python.handleRequest(request)`
2. NAPI converts JavaScript request to Rust `Request` type
3. `PythonRequestTask` or `PythonStreamTask` spawns on tokio threadpool
4. Task calls `asgi.handle(request)` which invokes Python
5. Response headers return immediately, body streams in background
6. NAPI converts Rust `Response` back to JavaScript

**Important Implementation Details:**

- **Body Writing Strategy**: Request body is written to a duplex stream in a separate task to prevent deadlocks when body size exceeds buffer capacity
- **Fallback Runtime**: Uses `asgi::fallback_handle()` to ensure a tokio runtime is always available, even in synchronous contexts
- **Extension Passing**: Request metadata (body buffers, WebSocket mode, socket info) is passed via HTTP extensions

---

### 2. ASGI Core Handler: `asgi/mod.rs`

**Responsibilities:**
- Manages the Python ASGI application lifecycle
- Loads Python modules and functions
- Coordinates between tokio (Rust async) and asyncio (Python async)
- Implements HTTP and WebSocket request handling
- Manages virtual environment discovery

**Key Types:**

```rust
pub struct Asgi {
    docroot: PathBuf,                    // Python module search path
    event_loop_handle: Arc<EventLoopHandle>,  // Shared Python event loop
    app_function: Py<PyAny>,             // Python ASGI callable
}
```

**ASGI Request/Response Flow:**

```
Node.js Request
    ↓
[lib.rs] Convert to Rust Request
    ↓
[mod.rs] Create ASGI scope from request
    ↓
[mod.rs] Create Receiver/Sender channels
    ↓
[mod.rs] Submit to Python event loop: app(scope, receive, send)
    ↓
[spawn_http_forwarding_task] Spawn background tasks:
    ├── Request body → Python (via receive channel)
    └── Python → Response body (via send channel)
    ↓
[mod.rs] Return Response with headers immediately
    ↓
[lib.rs] Stream body chunks to JavaScript
    ↓
Node.js Response
```

**Critical Async Coordination:**

The handler uses `tokio::spawn` to create a forwarding task that bridges Rust and Python asyncio:

```rust
tokio::spawn(async move {
    loop {
        tokio::select! {
            // Forward response messages from Python
            response_msg = tx_receiver.recv() => { ... }

            // Monitor Python future for exceptions
            result = future_poller => { ... }

            // Timeout if no response.start received
            _ = timeout => { ... }
        }
    }
});
```

This design allows:
- Headers to return immediately (low latency)
- Body to stream progressively (memory efficient)
- Concurrent request processing (high throughput)
- Graceful error handling (resilient)

**Python Environment Setup:**

The `setup_python_paths()` function:
1. Adds `docroot` to `sys.path`
2. Detects `VIRTUAL_ENV` environment variable
3. Dynamically discovers `lib/python3.*/site-packages` directories
4. Inserts all paths into Python's module search path

**Platform-Specific Initialization:**

On Linux, `ensure_python_initialized()` uses `dlopen()` with `RTLD_GLOBAL` to ensure Python symbols are globally visible, which is required for C extension modules to work correctly.

---

### 3. ASGI Protocol Messages: `asgi/http.rs`, `asgi/websocket.rs`, `asgi/lifespan.rs`

**Responsibilities:**
- Define ASGI message types as Rust enums
- Implement conversions between Rust and Python types
- Create ASGI connection scopes from HTTP requests

**HTTP Message Flow:**

```
Receive (Rust → Python):
- HttpReceiveMessage::Request { body, more_body }
- HttpReceiveMessage::Disconnect

Send (Python → Rust):
- HttpSendMessage::HttpResponseStart { status, headers, trailers }
- HttpSendMessage::HttpResponseBody { body, more_body }
```

**Scope Creation:**

The `HttpConnectionScope` extracts from a Rust `Request`:
- HTTP version, method, scheme, path, query string
- Headers (lowercased per ASGI spec)
- Client/server socket addresses
- Document root (from extension)
- Raw path bytes (for percent-encoded data)

**PyO3 Integration:**

All ASGI types implement:
- `IntoPyObject<'py>`: Convert Rust → Python dictionaries
- `FromPyObject<'a, 'py>`: Extract Python dictionaries → Rust

Example:
```rust
impl<'py> IntoPyObject<'py> for HttpConnectionScope {
    fn into_pyobject(self, py: Python<'py>) -> PyResult<PyDict> {
        let dict = PyDict::new(py);
        dict.set_item("type", "http")?;
        dict.set_item("method", self.method.into_pyobject(py)?)?;
        // ... more fields
        Ok(dict)
    }
}
```

**WebSocket Differences:**

- Scope type is `"websocket"` instead of `"http"`
- Includes `subprotocols` field from `Sec-WebSocket-Protocol` header
- Uses `ws`/`wss` schemes instead of `http`/`https`
- Requires explicit `Accept` message before data flow

---

### 4. Channel Communication: `asgi/receiver.rs`, `asgi/sender.rs`

**Responsibilities:**
- Bridge Rust tokio channels with Python async callables
- Provide ASGI-compatible `receive()` and `send()` functions to Python
- Handle backpressure via acknowledgment channels

**Receiver Pattern:**

```rust
#[pyclass]
pub struct Receiver(ReceiverType);

enum ReceiverType {
    Http(Arc<Mutex<mpsc::UnboundedReceiver<HttpReceiveMessage>>>),
    WebSocket(Arc<Mutex<mpsc::UnboundedReceiver<WebSocketReceiveMessage>>>),
    Lifespan(Arc<Mutex<mpsc::UnboundedReceiver<LifespanReceiveMessage>>>),
}

#[pymethods]
impl Receiver {
    async fn __call__(&mut self) -> PyResult<Py<PyDict>> {
        // Python calls: message = await receive()
        // This awaits on the Rust channel and converts to Python dict
    }
}
```

**Sender Pattern with Acknowledgments:**

```rust
pub struct AcknowledgedMessage<T> {
    pub message: T,
    pub ack: oneshot::Sender<()>,  // Acknowledge receipt
}

#[pymethods]
impl Sender {
    async fn __call__(&mut self, args: Py<PyDict>) -> PyResult<Py<PyAny>> {
        // 1. Send message to Rust
        // 2. Wait for acknowledgment (backpressure)
        // 3. Return to Python
    }
}
```

**Why Acknowledgments?**

This implements **async backpressure**: Python's `await send(message)` doesn't complete until Rust has processed the message. This prevents Python from overwhelming the Rust side with data faster than it can be written to the network.

---

### 5. Event Loop Management: `asgi/event_loop_handle.rs`

**Responsibilities:**
- Create and manage a shared Python asyncio event loop
- Run the event loop in a dedicated thread
- Cleanup on drop (stop event loop)
- Prevent multiple event loops per process

**Design Pattern:**

```rust
pub struct EventLoopHandle {
    event_loop: Py<PyAny>,  // Python asyncio.AbstractEventLoop
}

static PYTHON_EVENT_LOOP: OnceLock<Mutex<Weak<EventLoopHandle>>> = OnceLock::new();

impl EventLoopHandle {
    pub fn get_or_create() -> Result<Arc<Self>, HandlerError> {
        // 1. Try to upgrade existing weak reference
        // 2. If none exists, create new event loop
        // 3. Spawn dedicated thread running loop.run_forever()
        // 4. Return Arc to handle
    }
}
```

**Thread Model:**

- Python event loop runs in a dedicated `std::thread` (not tokio thread)
- This prevents runtime shutdown issues (tokio waits for blocking tasks)
- Multiple `Asgi` instances share the same event loop
- Event loop stops when last `EventLoopHandle` is dropped

**Submitting Work:**

```rust
Python::attach(|py| {
    let coro = app_function.call1(py, (scope, receive, send))?;
    let asyncio = py.import("asyncio")?;
    let future = asyncio.call_method1(
        "run_coroutine_threadsafe",
        (coro, event_loop_handle.event_loop())
    )?;
    Ok(future.unbind())
})
```

This uses `asyncio.run_coroutine_threadsafe()` to submit coroutines from Rust threads to the Python event loop thread.

---

### 6. Runtime Coordination: `asgi/runtime_handle.rs`

**Responsibilities:**
- Provide tokio runtime handle for async operations
- Create fallback runtime if needed (for sync API calls)
- Ensure async operations can run in any context

**Implementation:**

```rust
static FALLBACK_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub(crate) fn fallback_handle() -> tokio::runtime::Handle {
    tokio::runtime::Handle::try_current()
        .unwrap_or_else(|_| {
            let rt = FALLBACK_RUNTIME.get_or_init(|| {
                tokio::runtime::Runtime::new()
                    .expect("Failed to create fallback tokio runtime")
            });
            rt.handle().clone()
        })
}
```

**Use Cases:**

1. **Sync API**: `handle_request_sync()` needs a runtime to execute async code
2. **Tests**: Unit tests may not have a tokio runtime context
3. **NAPI Worker Threads**: NAPI task execution may occur outside main runtime

---

### 7. Python Future Polling: `asgi/python_future_poller.rs`

**Responsibilities:**
- Poll Python `concurrent.futures.Future` objects from Rust
- Detect when Python coroutines complete or raise exceptions
- Integrate with tokio's async ecosystem

**Why This Is Needed:**

When we submit a coroutine to Python via `run_coroutine_threadsafe()`, it returns a `concurrent.futures.Future`. We need to monitor this future to:
- Detect Python exceptions during request processing
- Cleanup resources when Python task completes
- Propagate errors back to Node.js

**Implementation Strategy:**

```rust
pub struct PythonFuturePoller {
    future: Py<PyAny>,  // concurrent.futures.Future
}

impl Future for PythonFuturePoller {
    type Output = Result<Py<PyAny>, PyErr>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Python::attach(|py| {
            if future.call_method0(py, "done")?.extract::<bool>()? {
                // Future completed - get result or exception
                Poll::Ready(future.call_method0(py, "result"))
            } else {
                // Not done yet - wake later
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
    }
}
```

This allows Rust's `tokio::select!` to monitor Python exceptions alongside other async events.

---

## Advanced Implementation Details

### Streaming Architecture

The library implements true bidirectional streaming using duplex channels:

```
┌─────────────┐                    ┌──────────────┐                    ┌─────────────┐
│  Node.js    │                    │     Rust     │                    │   Python    │
│  Request    │ ─── write() ────> │ DuplexStream │ ─── receive() ──> │    ASGI     │
│             │                    │   (64KB buf) │                    │     App     │
│  Response   │ <── read() ─────── │              │ <─── send() ────── │             │
└─────────────┘                    └──────────────┘                    └─────────────┘
     ↑                                                                         │
     └───────────── Headers available immediately ─────────────────────────────┘
     └───────────── Body streams chunk-by-chunk ────────────────────────────────
```

**Key Design Decisions:**

1. **Separate Request/Response Tasks**: Body reading and writing happen in parallel tasks to prevent deadlocks
2. **Early Header Return**: `handle()` returns as soon as Python sends `http.response.start`
3. **Chunked Streaming**: Body data flows in 64KB chunks to balance memory and throughput
4. **Backpressure**: Acknowledgments prevent overwhelming network buffers

### WebSocket Implementation

WebSocket support required additional complexity:

1. **Frame Encoding/Decoding**: Uses `WebSocketEncoder` and `WebSocketDecoder` from `http-handler`
2. **Connection Lifecycle**:
   - Client sends upgrade request
   - Rust sends `websocket.connect` to Python
   - Python must send `websocket.accept` before data flows
   - Data flows as `websocket.send` / `websocket.receive`
   - Either side can send `websocket.close`
3. **Stream Management**: WebSocket streams stay open until explicit close (unlike HTTP)

### Error Handling Strategy

The library has comprehensive error handling across three boundaries:

**Rust Errors (`HandlerError`):**
- I/O errors (file not found, network failures)
- Python errors (exceptions during execution)
- Channel errors (connection closed unexpectedly)
- Timeout errors (no response within deadline)

**Python Exceptions:**
- Caught via PyO3's `PyErr` type
- Converted to `HandlerError::PythonError`
- Propagated to Node.js as exceptions or response metadata

**Node.js Errors:**
- NAPI converts `HandlerError` to JavaScript `Error` objects
- Response object includes `exception` field for post-response errors
- Streaming errors stored in `ResponseException` extension

**Edge Cases:**

1. **Exception After Headers Sent**: Stored in `Arc<Mutex<Option<ResponseException>>>` for retrieval after stream completes
2. **Client Disconnect**: Sends `http.disconnect` or `websocket.disconnect` to Python
3. **Python Deadlock**: 30-second timeout on `http.response.start`

### Memory Management

**Zero-Copy Paths:**
- Request body: Node.js Buffer → Rust slice → Python bytes (no copy)
- Response body: Python bytes → Rust slice → Node.js Buffer (no copy)
- Headers: Converted to owned strings (necessary for async lifetime)

**Lifecycle Management:**
- `Arc<Asgi>`: Shared across requests, cloned cheaply
- `Py<PyAny>`: PyO3 handles Python reference counting
- `Arc<EventLoopHandle>`: Weak references prevent event loop leaks
- Channels: Automatically cleaned up when sender/receiver dropped

### Threading Model

```
┌─────────────────────────────────────────────────────────────┐
│ Node.js Main Thread                                         │
│  ├─ NAPI calls PythonHandler                                │
│  └─ Returns Response object (headers available immediately) │
└─────────────────────────────────────────────────────────────┘
                    │
                    ├───── spawns ─────> ┌─────────────────────────────┐
                    │                     │ Tokio Worker Thread Pool    │
                    │                     │  ├─ PythonRequestTask       │
                    │                     │  ├─ Body writer task        │
                    │                     │  ├─ Forwarding task         │
                    │                     │  └─ Body reader task        │
                    │                     └─────────────────────────────┘
                    │
                    └───── submits ─────> ┌─────────────────────────────┐
                                          │ Python Event Loop Thread    │
                                          │  └─ app(scope, receive, send)│
                                          └─────────────────────────────┘
```

**Thread Safety:**
- `Asgi`: Implements `Send + Sync` via `unsafe impl`
- `EventLoopHandle`: Safe to share via `Arc`
- Python calls: Protected by GIL via `Python::attach()`
- Channels: Tokio's `mpsc` is multi-producer, single-consumer

---

## Common Patterns and Gotchas

### Pattern: Converting Between Rust and Python

```rust
// Rust → Python
impl<'py> IntoPyObject<'py> for MyType {
    type Target = PyDict;
    type Output = Bound<'py, Self::Target>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
        let dict = PyDict::new(py);
        dict.set_item("field", self.field)?;
        Ok(dict)
    }
}

// Python → Rust
impl<'a, 'py> FromPyObject<'a, 'py> for MyType {
    type Error = PyErr;

    fn extract(ob: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        let dict = ob.cast::<PyDict>()?;
        let field = dict.get_item("field")?.extract()?;
        Ok(MyType { field })
    }
}
```

### Pattern: Spawning Async Tasks with Cleanup

```rust
tokio::spawn(async move {
    // Use owned data (moved into closure)
    let result = do_async_work(owned_data).await;

    // Cleanup happens when task exits or panics
    drop(cleanup_handle);

    result
});
```

### Gotcha: Python GIL and Async

**Problem**: Python's Global Interpreter Lock (GIL) blocks all Python execution

**Solution**: Use `Python::attach()` for brief GIL acquisitions:
```rust
// BAD: Holds GIL during entire async operation
Python::attach(|py| {
    async_operation().await  // Deadlock!
})

// GOOD: Release GIL between operations
let py_data = Python::attach(|py| extract_data(py));
async_operation(py_data).await;
let result = Python::attach(|py| convert_result(py, data));
```

This is irrelevant as of Python v3.14+, but we can link with anything back to
v3.8 so we should avoid holding the GIL too long to not make older versions
very slow or even at risk of deadlock.

### Gotcha: Request Body Deadlock

**Problem**: Writing request body while waiting for response can deadlock if buffer is full

**Solution**: Spawn separate task for body writing:
```rust
let body_writer = tokio::spawn(async move {
    body_stream.write_all(&data).await?;
    body_stream.shutdown().await?;
});

// Handle response concurrently
let response = asgi.handle(request).await?;

// Wait for body writing to complete
body_writer.await??;
```

### Gotcha: Extension Lifetimes

**Problem**: Extensions are stored in `http::Extensions` which requires `'static`

**Solution**: Use `Arc` or `Box` for non-static data:
```rust
request.extensions_mut().insert(Arc::new(my_data));
// Later
let data = request.extensions().get::<Arc<MyData>>().cloned();
```

---

## Testing Strategy

The codebase includes comprehensive unit and integration tests:

**Unit Tests** (`#[test]`, `#[tokio::test]`):
- Type conversions (Rust ↔ Python)
- Channel communication
- Error handling
- Event loop management

**Integration Tests** (in `mod.rs`):
- Full HTTP request/response cycles
- Concurrent request processing
- Streaming request/response bodies
- WebSocket connections
- Error propagation
- Status code handling

**Test Fixtures** (`test/fixtures/*.py`):
- `main.py`: Basic "Hello World" ASGI app
- `echo_app.py`: Echo request data in response
- `stream_app.py`: Chunked streaming responses
- `error_app.py`: Exception handling tests
- `websocket_app.py`: WebSocket echo server
- `status_app.py`: Custom status codes

**Testing Best Practices:**
- Use `ensure_python_initialized()` before PyO3 operations
- Create test streams with `tokio::io::duplex()`
- Mock ASGI apps with minimal Python code
- Test both success and error paths
- Verify cleanup (no leaked channels, threads, etc.)

---

## Future Development Considerations

### Performance Optimization Opportunities

1. **Zero-Copy Body Transfer**: Investigate using Python buffer protocol for true zero-copy
2. **Header Caching**: Reuse header conversions for repeated header names
3. **Connection Pooling**: Reuse Python event loop for multiple `Asgi` instances (already done)
4. **Batch Message Processing**: Send multiple body chunks in one Python call

### Missing Features

1. **HTTP/2 Server Push**: ASGI spec supports it, but we're pure http1.1 currently
2. **Trailers**: HTTP trailers for streaming responses
3. **Lifespan Protocol**: Startup/shutdown events (implementation exists but not exposed)
4. **WebSocket Compression**: Per-message deflate extension

### API Improvements

1. **Streaming Request Body**: Currently buffered before sending to Python
2. **Progress Callbacks**: Notify JavaScript of upload/download progress
3. **Request Cancellation**: Abort in-flight Python requests
4. **Custom ASGI Scope Fields**: Allow extending scope with user data

### Platform Support

1. **Windows**: Currently supports macOS and Linux only
2. **PyPy**: Test compatibility with PyPy (should work via PyO3)
3. **WebAssembly**: Investigate WASI-based Python execution

---

## Debugging Tips

### Enable Rust Logging

```bash
RUST_LOG=python_node=debug npm test
```

### Inspect Python Exceptions

```rust
match result {
    Err(HandlerError::PythonError(py_err)) => {
        eprintln!("Python error: {}", py_err);
        eprintln!("Traceback:");
        Python::attach(|py| {
            py_err.print(py);
        });
    }
}
```

### Debug ASGI Messages

```python
async def app(scope, receive, send):
    print(f"Scope: {scope}")
    while True:
        message = await receive()
        print(f"Received: {message}")
        if message['type'] == 'http.disconnect':
            break
```

### Trace Channel Communication

```rust
let (tx, rx) = mpsc::unbounded_channel();
let tx = {
    let _tx = tx.clone();
    move |msg| {
        eprintln!("Sending: {:?}", msg);
        _tx.send(msg)
    }
};
```

---

## Summary

The python-node library is a sophisticated multi-runtime bridge that demonstrates advanced Rust systems programming concepts:

- **FFI Mastery**: Safe integration with Python C API via PyO3
- **Async Coordination**: Bridging tokio and asyncio runtimes
- **NAPI Expertise**: Native Node.js addon development
- **Protocol Implementation**: Full ASGI 3.0 specification
- **Production-Ready**: Comprehensive error handling and testing

The architecture prioritizes:
- **Performance**: Streaming, zero-copy where possible, concurrent execution
- **Safety**: Rust's type system prevents data races and memory errors
- **Ergonomics**: Simple JavaScript API hiding complex internals
- **Correctness**: Extensive testing and ASGI compliance

For developers working on this codebase, understanding the async coordination between three runtimes is crucial. Pay careful attention to GIL acquisition, channel lifetimes, and task spawning patterns.
