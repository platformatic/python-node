use std::{
  env::{current_dir, var},
  ffi::CString,
  fs::{read_dir, read_to_string},
  future::Future,
  path::{Path, PathBuf},
  pin::Pin,
  sync::{Arc, Mutex, OnceLock, Weak},
  task::{Context, Poll},
};

#[cfg(target_os = "linux")]
use std::{ffi::CStr, mem};

use bytes::Bytes;
use http_handler::{
  Handler, Request, RequestBody, RequestExt, Response, StreamChunk, WebSocketMode, extensions::DocumentRoot
};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use tokio::sync::oneshot;

use crate::{HandlerError, PythonHandlerTarget};

/// HTTP response tuple: (status_code, headers)
type HttpResponse = (u16, Vec<(String, String)>);
/// Result type for HTTP response operations
type HttpResponseResult = Result<HttpResponse, HandlerError>;

/// Global runtime for when no tokio runtime is available
static FALLBACK_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub(crate) fn fallback_handle() -> tokio::runtime::Handle {
  tokio::runtime::Handle::try_current().unwrap_or_else(|_| {
    // No runtime exists, create a fallback one
    let rt = FALLBACK_RUNTIME.get_or_init(|| {
      tokio::runtime::Runtime::new().expect("Failed to create fallback tokio runtime")
    });
    rt.handle().clone()
  })
}

/// Future that polls a Python concurrent.futures.Future for completion
///
/// This future polls a Python concurrent.futures.Future and returns a Result
/// containing either the success value or the exception when the future completes.
struct PythonFuturePoller(Py<PyAny>);

impl PythonFuturePoller {
  fn new(future: Py<PyAny>) -> Self {
    Self(future)
  }
}

impl Future for PythonFuturePoller {
  type Output = Result<Py<PyAny>, PyErr>;

  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    Python::attach(|py| {
      let future_bound = self.0.bind(py);

      // First check if future is done
      let is_done: bool = future_bound
        .call_method0("done")
        .ok()
        .and_then(|result| result.extract().ok())
        .unwrap_or(false);

      if is_done {
        // Future is done - get the result (Ok for success, Err for exception)
        // Use 0.0 timeout since we know it's already done
        Poll::Ready(match future_bound.call_method1("result", (0.0,)) {
          Ok(value) => Ok(value.unbind()),
          Err(err) => Err(err),
        })
      } else {
        // Not done yet, wake the task to poll again
        cx.waker().wake_by_ref();
        Poll::Pending
      }
    })
  }
}

/// Global Python event loop handle storage
static PYTHON_EVENT_LOOP: OnceLock<Mutex<Weak<EventLoopHandle>>> = OnceLock::new();

mod http;
mod http_method;
mod http_version;
mod info;
mod lifespan;
mod receiver;
mod sender;
mod websocket;

pub use http::{HttpConnectionScope, HttpReceiveMessage, HttpSendMessage};
pub use http_method::HttpMethod;
pub use http_version::HttpVersion;
pub use info::AsgiInfo;
#[allow(unused_imports)]
pub use lifespan::{LifespanReceiveMessage, LifespanScope, LifespanSendMessage};
pub use receiver::Receiver;
pub use sender::{AcknowledgedMessage, Sender};
#[allow(unused_imports)]
pub use websocket::{
  WebSocketConnectionScope, WebSocketReceiveMessage, WebSocketSendException, WebSocketSendMessage,
};

/// Handle to a shared Python event loop
pub struct EventLoopHandle {
  event_loop: Py<PyAny>,
}

impl EventLoopHandle {
  /// Get the Python event loop object
  pub fn event_loop(&self) -> &Py<PyAny> {
    &self.event_loop
  }
}

impl Drop for EventLoopHandle {
  fn drop(&mut self) {
    // Stop the Python event loop when the last handle is dropped
    Python::attach(|py| {
      if let Err(e) = self.event_loop.bind(py).call_method0("stop") {
        eprintln!("Failed to stop Python event loop: {e}");
      }
    });
  }
}

unsafe impl Send for EventLoopHandle {}
unsafe impl Sync for EventLoopHandle {}

/// Ensure a Python event loop exists and return a handle to it
fn ensure_python_event_loop() -> Result<Arc<EventLoopHandle>, HandlerError> {
  let mut guard = PYTHON_EVENT_LOOP
    .get_or_init(|| Mutex::new(Weak::new()))
    .lock()?;

  // Try to upgrade the weak reference
  if let Some(handle) = guard.upgrade() {
    return Ok(handle);
  }

  // Create new handle
  let new_handle = Arc::new(create_event_loop_handle()?);
  *guard = Arc::downgrade(&new_handle);

  Ok(new_handle)
}

/// Create a new EventLoopHandle with a Python event loop
fn create_event_loop_handle() -> Result<EventLoopHandle, HandlerError> {
  // Ensure Python symbols are globally available before initializing
  #[cfg(target_os = "linux")]
  ensure_python_symbols_global();

  // Initialize Python if not already initialized
  Python::initialize();

  // Create event loop
  let event_loop = Python::attach(|py| -> Result<Py<PyAny>, HandlerError> {
    let asyncio = py.import("asyncio")?;
    let event_loop = asyncio.call_method0("new_event_loop")?;
    let event_loop_py = event_loop.unbind();

    // Start Python thread that just runs the event loop
    let loop_ = event_loop_py.clone_ref(py);

    // Try to use current runtime, fallback to creating a new one
    fallback_handle().spawn_blocking(move || {
      start_python_event_loop_thread(loop_);
    });

    Ok(event_loop_py)
  })?;

  Ok(EventLoopHandle { event_loop })
}

/// Core ASGI handler that loads and manages a Python ASGI application
pub struct Asgi {
  docroot: PathBuf,
  // Shared Python event loop handle
  event_loop_handle: Arc<EventLoopHandle>,
  // ASGI app function
  app_function: Py<PyAny>,
}

unsafe impl Send for Asgi {}
unsafe impl Sync for Asgi {}

impl Asgi {
  /// Create a new Asgi instance, loading the Python app and using shared event loop
  pub fn new(
    docroot: Option<String>,
    app_target: Option<PythonHandlerTarget>,
  ) -> Result<Self, HandlerError> {
    let target = app_target.unwrap_or_default();

    let docroot = docroot
      .map(|d| Ok(PathBuf::from(d)))
      .unwrap_or_else(|| current_dir().map_err(HandlerError::CurrentDirectoryError))?;

    // Get or create shared Python event loop
    let event_loop_handle = ensure_python_event_loop()?;

    // Load Python app
    let app_function = Python::attach(|py| -> Result<Py<PyAny>, HandlerError> {
      // Load and compile Python module
      let entrypoint = docroot
        .join(format!("{}.py", target.file))
        .canonicalize()
        .map_err(HandlerError::EntrypointNotFoundError)?;

      let code = read_to_string(entrypoint)
        .map_err(HandlerError::EntrypointNotFoundError)
        .and_then(|s| CString::new(s).map_err(HandlerError::StringCovertError))?;

      let file_name =
        CString::new(format!("{}.py", target.file)).map_err(HandlerError::StringCovertError)?;
      let module_name =
        CString::new(target.file.clone()).map_err(HandlerError::StringCovertError)?;

      // Set up sys.path
      setup_python_paths(py, &docroot)?;

      // Load the ASGI app
      let module = PyModule::from_code(py, &code, &file_name, &module_name)?;
      let app_function = module.getattr(&target.function)?.unbind();

      Ok(app_function)
    })?;

    // Create the Asgi instance
    Ok(Asgi {
      docroot,
      event_loop_handle,
      app_function,
    })
  }

  /// Get the document root for this ASGI handler.
  pub fn docroot(&self) -> &Path {
    &self.docroot
  }
}

// Helper function: Forward HTTP request chunk
// Returns true if the loop should break
async fn forward_http_request_chunk(
  chunk: Option<StreamChunk<Bytes>>,
  rx: &tokio::sync::mpsc::UnboundedSender<HttpReceiveMessage>,
  request_done: &mut bool,
) -> bool {
  match chunk {
    Some(StreamChunk::Data(bytes)) => {
      if rx.send(HttpReceiveMessage::Request {
        body: bytes.to_vec(),
        more_body: true,
      }).is_err() {
        // Python dropped receiver - stop forwarding
        return true;
      }
    }
    Some(StreamChunk::End) => {
      // Send final request chunk
      if rx.send(HttpReceiveMessage::Request {
        body: vec![],
        more_body: false,
      }).is_err() {
        // Python dropped receiver - stop forwarding
        return true;
      }
      *request_done = true;
    }
    None => {
      // Request channel closed without End marker - send final message
      if rx.send(HttpReceiveMessage::Request {
        body: vec![],
        more_body: false,
      }).is_err() {
        // Python dropped receiver - stop forwarding
        return true;
      }
      *request_done = true;
    }
  }
  false
}

// Helper function: Forward WebSocket request chunk
// Returns true if the loop should break
async fn forward_websocket_request_chunk(
  chunk: Option<StreamChunk<Bytes>>,
  rx: &tokio::sync::mpsc::UnboundedSender<WebSocketReceiveMessage>,
) -> bool {
  match chunk {
    Some(StreamChunk::Data(bytes)) => {
      // Try to decode as text, fallback to binary
      let send_result = if let Ok(text) = String::from_utf8(bytes.to_vec()) {
        rx.send(WebSocketReceiveMessage::Receive {
          text: Some(text),
          bytes: None,
        })
      } else {
        rx.send(WebSocketReceiveMessage::Receive {
          text: None,
          bytes: Some(bytes.to_vec()),
        })
      };

      if send_result.is_err() {
        // Python receiver dropped - stop forwarding
        return true;
      }
    }
    Some(StreamChunk::End) => {
      // Send disconnect message
      if rx.send(WebSocketReceiveMessage::Disconnect {
        code: Some(1000),
        reason: None,
      }).is_err() {
        // Python receiver dropped - already disconnected
      }
      return true;
    }
    None => {
      // Client disconnected
      if rx.send(WebSocketReceiveMessage::Disconnect {
        code: Some(1000),
        reason: None,
      }).is_err() {
        // Python receiver dropped - already disconnected
      }
      return true;
    }
  }
  false
}

// Helper function: Handle HTTP response message
// Returns true if the loop should break
async fn handle_http_response_message(
  msg: Option<AcknowledgedMessage<HttpSendMessage>>,
  response_tx: &mut Option<oneshot::Sender<HttpResponseResult>>,
  response_body_tx: &tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>,
  response_body_tx_keepalive: &Arc<Mutex<Option<tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>>>>,
) -> bool {
  match msg {
    Some(AcknowledgedMessage {
      message: HttpSendMessage::HttpResponseStart { status, headers, .. },
      ack,
    }) if response_tx.is_some() => {
      // Send response.start back to main task (oneshot - only once)
      if let Some(tx) = response_tx.take() {
        if tx.send(Ok((status, headers))).is_err() {
          // Main task dropped receiver - stop forwarding
          return true;
        }
      }
      if ack.send(()).is_err() {
        // Python dropped receiver - stop forwarding
        return true;
      }
    }
    Some(AcknowledgedMessage {
      message: HttpSendMessage::HttpResponseBody { body, more_body },
      ack,
    }) => {
      // Acknowledge receipt
      if ack.send(()).is_err() {
        // Python dropped receiver - stop forwarding
        return true;
      }

      // Send body data if not empty
      if !body.is_empty()
        && response_body_tx.send(Ok(StreamChunk::Data(Bytes::from(body)))).await.is_err() {
        // Client dropped receiver - stop forwarding
        return true;
      }

      // Check if this was the final chunk
      if !more_body {
        // Send End marker and close channel
        if response_body_tx.send(Ok(StreamChunk::End)).await.is_err() {
          // Client dropped receiver - already disconnected, exit cleanly
        }
        drop(response_body_tx_keepalive.lock().unwrap().take());
        return true;
      }
    }
    None => {
      // Python sender closed
      return true;
    }
    _ => {
      // Ignore other message types (e.g., duplicate response.start)
    }
  }
  false
}

// Helper function: Handle WebSocket response message
// Returns true if the loop should break
async fn handle_websocket_response_message(
  msg: Option<AcknowledgedMessage<WebSocketSendMessage>>,
  response_body_tx: &tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>,
) -> bool {
  match msg {
    Some(ack_msg) => {
      match ack_msg.message {
        WebSocketSendMessage::Send { text, bytes } => {
          if let Some(text) = text {
            if response_body_tx.send(Ok(StreamChunk::Data(Bytes::from(text)))).await.is_err() {
              // Client disconnected - stop forwarding
              return true;
            }
          } else if let Some(bytes) = bytes {
            if response_body_tx.send(Ok(StreamChunk::Data(Bytes::from(bytes)))).await.is_err() {
              // Client disconnected - stop forwarding
              return true;
            }
          }
        }
        WebSocketSendMessage::Close { .. } => {
          // Send End marker and close connection
          if response_body_tx.send(Ok(StreamChunk::End)).await.is_err() {
            // Client disconnected - already closed
          }
          return true;
        }
        _ => {}
      }
      // Acknowledge receipt
      if ack_msg.ack.send(()).is_err() {
        // Python dropped receiver - stop forwarding
        return true;
      }
    }
    None => {
      // Python sender closed
      return true;
    }
  }
  false
}

// Helper function: Handle Python exception
// Returns true if the loop should break
async fn handle_python_exception(
  result: Result<Py<PyAny>, PyErr>,
  response_tx: Option<oneshot::Sender<HttpResponseResult>>,
  response_body_tx: &tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>,
  response_body_tx_keepalive: Option<&Arc<Mutex<Option<tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>>>>>,
) -> bool {
  if let Err(py_err) = result {
    // Python exception - propagate error depending on state
    if let Some(tx) = response_tx {
      // Response.start not yet sent - send error via oneshot channel
      let _ = tx.send(Err(HandlerError::PythonError(py_err)));
    } else {
      // Response.start already sent - send error via response body stream
      let error_msg = Python::attach(|_py| py_err.to_string());
      let _ = response_body_tx.send(Err(error_msg)).await;
    }
    // Close response stream and exit
    if let Some(keepalive) = response_body_tx_keepalive {
      drop(keepalive.lock().unwrap().take());
    }
    return true;
  }
  false
}

// Helper function: Handle response timeout
// Returns true if the loop should break
fn handle_response_timeout(
  response_tx: Option<oneshot::Sender<HttpResponseResult>>,
  response_body_tx_keepalive: &Arc<Mutex<Option<tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>>>>,
) -> bool {
  // Send timeout error via oneshot
  if let Some(tx) = response_tx {
    let _ = tx.send(Err(HandlerError::NoResponse));
  }
  drop(response_body_tx_keepalive.lock().unwrap().take());
  true
}

// Spawn HTTP forwarding task
fn spawn_http_forwarding_task(
  mut request_rx: tokio::sync::mpsc::Receiver<StreamChunk<Bytes>>,
  mut tx_receiver: tokio::sync::mpsc::UnboundedReceiver<AcknowledgedMessage<HttpSendMessage>>,
  rx: tokio::sync::mpsc::UnboundedSender<HttpReceiveMessage>,
  response_body_tx_clone: tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>,
  response_body_tx_keepalive_for_task: Arc<Mutex<Option<tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>>>>,
  response_tx: oneshot::Sender<HttpResponseResult>,
  future: Py<PyAny>,
) {
  tokio::spawn(async move {
    let mut request_done = false;
    let mut response_tx = Some(response_tx);
    let mut future_poller = PythonFuturePoller::new(future);
    let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(30));
    tokio::pin!(timeout);

    loop {
      tokio::select! {
        // Forward request body chunks from Node.js to Python
        request_chunk = request_rx.recv(), if !request_done => {
          if forward_http_request_chunk(request_chunk, &rx, &mut request_done).await {
            break;
          }
        }

        // Forward response messages from Python to Node.js
        response_msg = tx_receiver.recv() => {
          if handle_http_response_message(
            response_msg,
            &mut response_tx,
            &response_body_tx_clone,
            &response_body_tx_keepalive_for_task,
          ).await {
            break;
          }
        }

        // Monitor Python future for exceptions
        result = Pin::new(&mut future_poller) => {
          if handle_python_exception(
            result,
            response_tx.take(),
            &response_body_tx_clone,
            Some(&response_body_tx_keepalive_for_task),
          ).await {
            break;
          }
        }

        // Timeout after 30 seconds without response.start
        _ = &mut timeout, if response_tx.is_some() => {
          if handle_response_timeout(response_tx.take(), &response_body_tx_keepalive_for_task) {
            break;
          }
        }

        // Exit loop if both directions are done
        else => break,
      }
    }
  });
}

// Spawn WebSocket forwarding task
fn spawn_websocket_forwarding_task(
  mut request_rx: tokio::sync::mpsc::Receiver<StreamChunk<Bytes>>,
  mut tx_receiver: tokio::sync::mpsc::UnboundedReceiver<AcknowledgedMessage<WebSocketSendMessage>>,
  rx: tokio::sync::mpsc::UnboundedSender<WebSocketReceiveMessage>,
  response_body_tx_clone: tokio::sync::mpsc::Sender<Result<StreamChunk<Bytes>, String>>,
  future: Py<PyAny>,
) {
  tokio::spawn(async move {
    let mut future_poller = PythonFuturePoller::new(future);

    loop {
      tokio::select! {
        // Forward WebSocket messages from client to Python
        request_chunk = request_rx.recv() => {
          if forward_websocket_request_chunk(request_chunk, &rx).await {
            break;
          }
        }

        // Forward WebSocket messages from Python to client
        response_msg = tx_receiver.recv() => {
          if handle_websocket_response_message(response_msg, &response_body_tx_clone).await {
            break;
          }
        }

        // Monitor Python future for exceptions
        result = Pin::new(&mut future_poller) => {
          if handle_python_exception(result, None, &response_body_tx_clone, None).await {
            break;
          }
        }

        // Exit loop if all branches complete
        else => break,
      }
    }
  });
}

impl Handler for Asgi {
  type Error = HandlerError;

  async fn handle(&self, request: Request) -> Result<Response, Self::Error> {
    // Set document root extension
    let mut request = request;
    request.set_document_root(DocumentRoot {
      path: self.docroot.clone(),
    });

    // Check if this is a WebSocket request
    let is_websocket = request.extensions().get::<WebSocketMode>().is_some();

    // Extract parts
    let (parts, mut body) = request.into_parts();

    // Create response body and sender
    let (response_body, response_body_tx) = body.create_response();

    // Take request receiver for bidirectional forwarding
    let request_rx = body
      .take_request_rx()
      .ok_or(HandlerError::StreamAlreadyConsumed)?;

    // Clone response_body_tx for spawned task
    let response_body_tx_clone = response_body_tx.clone();

    if is_websocket {
      // WebSocket mode
      // Create WebSocket scope from parts by temporarily reconstructing a request
      let temp_request = Request::from_parts(parts.clone(), RequestBody::new());
      let scope: WebSocketConnectionScope = (&temp_request).try_into()?;

      // Create channels for ASGI communication
      let (rx_receiver, rx) = Receiver::websocket();
      let (tx_sender, mut tx_receiver) = Sender::websocket();

      // Send connect
      rx
        .send(WebSocketReceiveMessage::Connect)
        .map_err(|_| HandlerError::NoResponse)?;

      // Submit ASGI app to Python
      let future = Python::attach(|py| {
        let scope_py = scope.into_pyobject(py)?;
        let coro = self.app_function.call1(py, (scope_py, rx_receiver, tx_sender))?;

        let asyncio = py.import("asyncio")?;
        let future = asyncio.call_method1(
          "run_coroutine_threadsafe",
          (coro, self.event_loop_handle.event_loop()),
        )?;

        Ok::<Py<PyAny>, HandlerError>(future.unbind())
      })?;

      // Wait for accept
      match tx_receiver.recv().await {
        Some(AcknowledgedMessage {
          message: WebSocketSendMessage::Accept { .. },
          ack,
        }) => {
          // Acknowledge receipt
          if ack.send(()).is_err() {
            // Python dropped receiver - cannot continue
            return Err(HandlerError::WebSocketNotAccepted);
          }
        }
        _ => return Err(HandlerError::WebSocketNotAccepted),
      }

      // Spawn WebSocket forwarding task
      spawn_websocket_forwarding_task(
        request_rx,
        tx_receiver,
        rx,
        response_body_tx_clone,
        future,
      );

      // Return 101 Switching Protocols response with WebSocket body
      http_handler::response::Builder::new()
        .status(101)
        .body(response_body)
        .map_err(HandlerError::HttpHandlerError)
    } else {
      // HTTP mode
      // Create HTTP scope from parts by temporarily reconstructing a request
      let temp_request = Request::from_parts(parts.clone(), RequestBody::new());
      let scope: HttpConnectionScope = (&temp_request).try_into()?;

      // Create ASGI channels
      let (rx_receiver, rx) = Receiver::http();
      let (tx_sender, tx_receiver) = Sender::http();

      // Keep channel alive for HTTP streaming
      let response_body_tx_keepalive = Arc::new(Mutex::new(Some(response_body_tx.clone())));
      let response_body_tx_keepalive_for_task = response_body_tx_keepalive.clone();

      // Create oneshot channel for sending response.start (or error) back to main task
      let (response_tx, response_rx) = oneshot::channel::<HttpResponseResult>();

      // Submit ASGI app to Python to get the future
      let future = Python::attach(|py| {
        let scope_py = scope.into_pyobject(py)?;
        let coro = self.app_function.call1(py, (scope_py, rx_receiver, tx_sender))?;

        let asyncio = py.import("asyncio")?;
        let future = asyncio.call_method1(
          "run_coroutine_threadsafe",
          (coro, self.event_loop_handle.event_loop()),
        )?;

        Ok::<Py<PyAny>, HandlerError>(future.unbind())
      })?;

      // Spawn HTTP forwarding task
      spawn_http_forwarding_task(
        request_rx,
        tx_receiver,
        rx,
        response_body_tx_clone,
        response_body_tx_keepalive_for_task,
        response_tx,
        future,
      );

      // Wait for response.start (errors are propagated from the forwarding task)
      let (status, headers) = response_rx
        .await
        .map_err(|_| HandlerError::NoResponse)? // Channel closed without sending
        ?; // Unwrap Result from the task

      // Build and return response with headers and streaming body
      let mut builder = http_handler::response::Builder::new().status(status);
      for (name, value) in headers {
        builder = builder.header(name.as_bytes(), value.as_bytes());
      }

      builder
        .body(response_body)
        .map_err(HandlerError::HttpHandlerError)
    }
  }
}

impl Asgi {
  /// Handle a request synchronously (continued for compatibility)
  pub fn handle_sync(&self, request: Request) -> Result<Response, HandlerError> {
    fallback_handle().block_on(self.handle(request))
  }
}

/// Load Python library with RTLD_GLOBAL on Linux to expose interpreter symbols
#[cfg(target_os = "linux")]
fn ensure_python_symbols_global() {
  // Only perform the promotion once per process
  static GLOBALIZE_ONCE: OnceLock<()> = OnceLock::new();

  GLOBALIZE_ONCE.get_or_init(|| unsafe {
    let mut info: libc::Dl_info = mem::zeroed();
    if libc::dladdr(pyo3::ffi::Py_Initialize as *const _, &mut info) == 0
      || info.dli_fname.is_null()
    {
      eprintln!("unable to locate libpython for RTLD_GLOBAL promotion");
      return;
    }

    let path_cstr = CStr::from_ptr(info.dli_fname);
    let path_str = path_cstr.to_string_lossy();

    // Clear any prior dlerror state before attempting to reopen
    libc::dlerror();

    let handle = libc::dlopen(info.dli_fname, libc::RTLD_NOW | libc::RTLD_GLOBAL);
    if handle.is_null() {
      let error = libc::dlerror();
      if !error.is_null() {
        let msg = CStr::from_ptr(error).to_string_lossy();
        eprintln!("dlopen({path_str}) failed with RTLD_GLOBAL: {msg}",);
      } else {
        eprintln!("dlopen({path_str}) returned null without dlerror",);
      }
    }
  });
}

/// Find all Python site-packages directories in a virtual environment
fn find_python_site_packages(venv_path: &Path) -> Vec<PathBuf> {
  let mut site_packages_paths = Vec::new();

  // Check both lib and lib64 directories
  for lib_dir in &["lib", "lib64"] {
    let lib_path = venv_path.join(lib_dir);
    if let Ok(entries) = read_dir(lib_path) {
      for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir()
          && let Some(dir_name) = entry_path.file_name().and_then(|n| n.to_str()) {
          // Look for directories matching python3.* pattern
          if dir_name.starts_with("python3.") {
            let site_packages = entry_path.join("site-packages");
            if site_packages.exists() {
              site_packages_paths.push(site_packages);
            }
          }
        }
      }
    }
  }

  site_packages_paths
}

/// Set up Python sys.path with docroot and virtual environment paths
fn setup_python_paths(py: Python, docroot: &Path) -> PyResult<()> {
  let sys = py.import("sys")?;
  let path = sys.getattr("path")?;

  // Add docroot to sys.path
  path.call_method1("insert", (0, docroot.to_string_lossy()))?;

  // Check for VIRTUAL_ENV and add virtual environment paths
  if let Ok(virtual_env) = var("VIRTUAL_ENV") {
    let venv_path = PathBuf::from(&virtual_env);

    // Dynamically find all Python site-packages directories
    let site_packages_paths = find_python_site_packages(&venv_path);

    // Add all found site-packages paths to sys.path
    for site_packages in &site_packages_paths {
      path.call_method1("insert", (0, site_packages.to_string_lossy()))?;
    }

    // Also add the virtual environment root
    path.call_method1("insert", (0, virtual_env))?;
  }

  Ok(())
}

/// Start a Python thread that runs the event loop forever
fn start_python_event_loop_thread(event_loop: Py<PyAny>) {
  Python::attach(|py| {
    // Set the event loop for this thread and run it
    let asyncio = py.import("asyncio")?;
    asyncio.call_method1("set_event_loop", (event_loop.bind(py),))?;

    // Get the current event loop and run it forever
    asyncio
      .call_method0("get_event_loop")?
      .call_method0("run_forever")?;

    Ok::<(), PyErr>(())
  })
  .unwrap_or_else(|e| {
    eprintln!("Python event loop thread error: {e}");
  });
}
