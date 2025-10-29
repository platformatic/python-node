use std::{
  env::{current_dir, var},
  ffi::CString,
  fs::{read_dir, read_to_string},
  path::{Path, PathBuf},
  sync::Arc,
};

#[cfg(target_os = "linux")]
use std::{ffi::CStr, mem};

use bytes::BytesMut;
use http_handler::{
  Handler, Request, RequestBody, RequestExt, Response, ResponseException, WebSocketMode,
  extensions::DocumentRoot, websocket::WebSocketEncoder,
};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, oneshot};

use crate::{HandlerError, PythonHandlerTarget};

/// HTTP response tuple: (status_code, headers)
type HttpResponse = (u16, Vec<(String, String)>);
/// Result type for HTTP response operations
type HttpResponseResult = Result<HttpResponse, HandlerError>;

mod event_loop_handle;
mod http;
mod http_method;
mod http_version;
mod info;
mod lifespan;
mod python_future_poller;
mod receiver;
mod runtime_handle;
mod sender;
mod websocket;

use event_loop_handle::EventLoopHandle;
use python_future_poller::PythonFuturePoller;

pub(crate) use runtime_handle::fallback_handle;

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
    let event_loop_handle = EventLoopHandle::get_or_create()?;

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

// Helper function: Forward HTTP request data from DuplexStream to Python
// Returns when stream ends or error occurs
async fn forward_http_request<R>(
  mut request_stream: R,
  rx: tokio::sync::mpsc::UnboundedSender<HttpReceiveMessage>,
) where
  R: tokio::io::AsyncRead + Unpin,
{
  const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer
  let mut buffer = BytesMut::with_capacity(BUFFER_SIZE);
  loop {
    let n = match request_stream.read_buf(&mut buffer).await {
      Ok(n) => n,
      Err(_) => break,
    };

    if n == 0 {
      // EOF - send final message
      let _ = rx.send(HttpReceiveMessage::Request {
        body: vec![],
        more_body: false,
      });
      break;
    }

    // Send the data we read
    let data = buffer.split_to(n).to_vec();
    if rx
      .send(HttpReceiveMessage::Request {
        body: data,
        more_body: true,
      })
      .is_err()
    {
      // Python dropped receiver
      break;
    }
  }
}

// Helper function: Forward WebSocket request data from DuplexStream to Python
// Returns when stream ends or error occurs
async fn forward_websocket_request<R>(
  request_stream: R,
  rx: tokio::sync::mpsc::UnboundedSender<WebSocketReceiveMessage>,
) where
  R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
  // JavaScript now sends WebSocket frames (auto-encoded by Request::write())
  // Use WebSocketDecoder to decode them
  let mut decoder = http_handler::websocket::WebSocketDecoder::new(request_stream);

  loop {
    match decoder.read_message().await {
      Ok(Some(frame)) => {
        // Got a WebSocket frame - forward to Python based on type
        if frame.is_text() {
          if let Some(text) = frame.payload_as_text()
            && rx
              .send(WebSocketReceiveMessage::Receive {
                text: Some(text),
                bytes: None,
              })
              .is_err()
          {
            // Python receiver dropped
            break;
          }
        } else if frame.is_binary() {
          if rx
            .send(WebSocketReceiveMessage::Receive {
              text: None,
              bytes: Some(frame.payload),
            })
            .is_err()
          {
            // Python receiver dropped
            break;
          }
        } else if frame.is_close() {
          // Got close frame - send disconnect to Python
          let (code, reason) = frame.parse_close_payload().unzip();
          let _ = rx.send(WebSocketReceiveMessage::Disconnect { code, reason });
          break;
        }
        // Ignore ping/pong frames (handled automatically)
      }
      Ok(None) => {
        // Stream ended - send disconnect
        let _ = rx.send(WebSocketReceiveMessage::Disconnect {
          code: Some(1000),
          reason: None,
        });
        break;
      }
      Err(_) => {
        // Error reading - send disconnect
        let _ = rx.send(WebSocketReceiveMessage::Disconnect {
          code: Some(1006), // Abnormal closure
          reason: None,
        });
        break;
      }
    }
  }
}

// Helper function: Handle HTTP response message from Python and write to DuplexStream
// Returns true if the loop should break
async fn handle_http_response_message<W>(
  msg: Option<AcknowledgedMessage<HttpSendMessage>>,
  response_tx: &mut Option<oneshot::Sender<HttpResponseResult>>,
  response_stream: &mut W,
) -> bool
where
  W: tokio::io::AsyncWrite + Unpin,
{
  match msg {
    Some(AcknowledgedMessage {
      message: HttpSendMessage::HttpResponseStart {
        status, headers, ..
      },
      ack,
    }) if response_tx.is_some() => {
      // Send response.start back to main task (oneshot - only once)
      if let Some(tx) = response_tx.take()
        && tx.send(Ok((status, headers))).is_err()
      {
        // Main task dropped receiver - stop forwarding
        return true;
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

      // Write body data if not empty
      if !body.is_empty() {
        if response_stream.write_all(&body).await.is_err() {
          // Client disconnected
          return true;
        }
      }

      // Check if this was the final chunk
      if !more_body {
        // Close the write side
        let _ = response_stream.shutdown().await;
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

// Helper function: Handle WebSocket response message from Python and write to DuplexStream
// Returns true if the loop should break
async fn handle_websocket_response_message<W>(
  msg: Option<AcknowledgedMessage<WebSocketSendMessage>>,
  encoder: &http_handler::websocket::WebSocketEncoder<W>,
) -> bool
where
  W: tokio::io::AsyncWrite + Unpin + Send,
{
  match msg {
    Some(ack_msg) => {
      match ack_msg.message {
        WebSocketSendMessage::Send { text, bytes } => {
          // Send WebSocket frames to JavaScript (will be auto-decoded by Response::next())
          let result = if let Some(text) = text {
            encoder.write_text(&text, false).await
          } else if let Some(bytes) = bytes {
            encoder.write_binary(&bytes, false).await
          } else {
            Ok(())
          };

          if result.is_err() {
            // Client disconnected or write error
            return true;
          }
        }
        WebSocketSendMessage::Close { code, reason } => {
          // Send close frame
          let reason_str = reason.as_deref();
          let _ = encoder.write_close(code, reason_str).await;
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
async fn handle_python_exception<W>(
  result: Result<Py<PyAny>, PyErr>,
  response_tx: Option<oneshot::Sender<HttpResponseResult>>,
  response_stream: Option<&mut W>,
  response_exception: Option<&Arc<Mutex<Option<ResponseException>>>>,
) -> bool
where
  W: tokio::io::AsyncWrite + Unpin,
{
  if let Err(py_err) = result {
    let error_msg = py_err.to_string();

    // Python exception - send error via oneshot if response not yet started
    if let Some(tx) = response_tx {
      let _ = tx.send(Err(HandlerError::PythonError(py_err)));
    } else {
      // Response already started - store error for later retrieval and close stream
      if let Some(exception_holder) = response_exception {
        let mut exc = exception_holder.lock().await;
        *exc = Some(ResponseException::new(error_msg));
      }

      if let Some(stream) = response_stream {
        use tokio::io::AsyncWriteExt;
        let _ = stream.shutdown().await;
      }
    }
  }
  // Always return true when Python future completes (success or error)
  // to exit the forwarding loop
  true
}

// Helper function: Handle response timeout
// Returns true if the loop should break
fn handle_response_timeout(response_tx: Option<oneshot::Sender<HttpResponseResult>>) -> bool {
  // Send timeout error via oneshot
  if let Some(tx) = response_tx {
    let _ = tx.send(Err(HandlerError::NoResponse));
  }
  true
}

// Spawn HTTP forwarding task
fn spawn_http_forwarding_task<R, W>(
  request_stream: R,
  mut tx_receiver: tokio::sync::mpsc::UnboundedReceiver<AcknowledgedMessage<HttpSendMessage>>,
  rx: tokio::sync::mpsc::UnboundedSender<HttpReceiveMessage>,
  response_stream: W,
  response_tx: oneshot::Sender<HttpResponseResult>,
  future: Py<PyAny>,
  response_exception: Arc<Mutex<Option<ResponseException>>>,
) where
  R: tokio::io::AsyncRead + Unpin + Send + 'static,
  W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
  tokio::spawn(async move {
    let mut response_tx = Some(response_tx);
    let mut future_poller = PythonFuturePoller::new(future);
    let timeout = tokio::time::sleep(tokio::time::Duration::from_secs(30));
    tokio::pin!(timeout);

    // Spawn request forwarding as separate task
    let mut request_done = Some(tokio::spawn(forward_http_request(request_stream, rx)));
    let mut response_stream = response_stream;

    loop {
      tokio::select! {
        // Forward response messages from Python to Node.js
        response_msg = tx_receiver.recv() => {
          if handle_http_response_message(
            response_msg,
            &mut response_tx,
            &mut response_stream,
          ).await {
            break;
          }
        }

        // Monitor Python future for exceptions
        result = Pin::new(&mut future_poller) => {
          if handle_python_exception(result, response_tx.take(), Some(&mut response_stream), Some(&response_exception)).await {
            break;
          }
        }

        // Timeout after 30 seconds without response.start
        _ = &mut timeout, if response_tx.is_some() => {
          if handle_response_timeout(response_tx.take()) {
            break;
          }
        }

        // Wait for request forwarding to complete (only poll once)
        _ = async { request_done.as_mut().unwrap().await }, if request_done.is_some() => {
          request_done = None;
        }

        // Exit loop if all branches are done
        else => break,
      }
    }
  });
}

// Spawn WebSocket forwarding task
fn spawn_websocket_forwarding_task<R, W>(
  request_stream: R,
  mut tx_receiver: tokio::sync::mpsc::UnboundedReceiver<AcknowledgedMessage<WebSocketSendMessage>>,
  rx: tokio::sync::mpsc::UnboundedSender<WebSocketReceiveMessage>,
  response_stream: W,
  future: Py<PyAny>,
) where
  R: tokio::io::AsyncRead + Unpin + Send + 'static,
  W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
  tokio::spawn(async move {
    let mut future_poller = PythonFuturePoller::new(future);

    // Create WebSocket encoder for sending frames to client
    let encoder = WebSocketEncoder::new(response_stream);

    // Spawn request forwarding as separate task
    let mut request_done = Some(tokio::spawn(forward_websocket_request(request_stream, rx)));

    // Track if close frame was sent (write_close also closes the stream)
    let mut close_sent = false;

    loop {
      tokio::select! {
        // Forward WebSocket messages from Python to client
        response_msg = tx_receiver.recv() => {
          if handle_websocket_response_message(response_msg, &encoder).await {
            close_sent = true;
            break;
          }
        }

        // Monitor Python future for exceptions
        result = Pin::new(&mut future_poller) => {
          if handle_python_exception::<W>(result, None, None, None).await {
            break;
          }
        }

        // Wait for request forwarding to complete (only poll once)
        _ = async { request_done.as_mut().unwrap().await }, if request_done.is_some() => {
          request_done = None;
        }

        // Exit loop if all branches complete
        else => break,
      }
    }

    // Close the response stream only if close frame wasn't sent
    // (write_close already closes the stream)
    if !close_sent {
      let _ = encoder.end().await;
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
    let (parts, body) = request.into_parts();

    // Create response body
    let response_body = body.create_response();

    // Clone bodies for bidirectional forwarding
    // RequestBody implements AsyncRead (reads from read_side)
    // ResponseBody implements AsyncWrite (writes to write_side)
    let request_reader = body.clone();
    let response_writer = response_body.clone();

    if is_websocket {
      // WebSocket mode
      // Create WebSocket scope from parts by temporarily reconstructing a request
      let temp_request = Request::from_parts(parts.clone(), RequestBody::new());
      let scope: WebSocketConnectionScope = (&temp_request).try_into()?;

      // Create channels for ASGI communication
      let (rx_receiver, rx) = Receiver::websocket();
      let (tx_sender, mut tx_receiver) = Sender::websocket();

      // Send connect
      rx.send(WebSocketReceiveMessage::Connect)
        .map_err(|_| HandlerError::NoResponse)?;

      // Submit ASGI app to Python
      let future = Python::attach(|py| {
        let scope_py = scope.into_pyobject(py)?;
        let coro = self
          .app_function
          .call1(py, (scope_py, rx_receiver, tx_sender))?;

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
      spawn_websocket_forwarding_task(request_reader, tx_receiver, rx, response_writer, future);

      // Return 101 Switching Protocols response with WebSocket body
      http_handler::response::Builder::new()
        .status(101)
        .extension(WebSocketMode) // Mark response as WebSocket for auto-decoding
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

      // Create oneshot channel for sending response.start (or error) back to main task
      let (response_tx, response_rx) = oneshot::channel::<HttpResponseResult>();

      // Submit ASGI app to Python to get the future
      let future = Python::attach(|py| {
        let scope_py = scope.into_pyobject(py)?;
        let coro = self
          .app_function
          .call1(py, (scope_py, rx_receiver, tx_sender))?;

        let asyncio = py.import("asyncio")?;
        let future = asyncio.call_method1(
          "run_coroutine_threadsafe",
          (coro, self.event_loop_handle.event_loop()),
        )?;

        Ok::<Py<PyAny>, HandlerError>(future.unbind())
      })?;

      // Create exception holder to capture errors that occur after response.start
      let response_exception = Arc::new(Mutex::new(None));
      let response_exception_clone = Arc::clone(&response_exception);

      // Spawn HTTP forwarding task
      spawn_http_forwarding_task(
        request_reader,
        tx_receiver,
        rx,
        response_writer,
        response_tx,
        future,
        response_exception_clone,
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

      let mut response = builder
        .body(response_body)
        .map_err(HandlerError::HttpHandlerError)?;

      // Insert response exception extension so NAPI layer can check for errors after stream ends
      // The exception will be set if a Python error occurs during streaming
      response.extensions_mut().insert(response_exception);

      Ok(response)
    }
  }
}

impl Asgi {
  /// Handle a request synchronously (continued for compatibility)
  pub fn handle_sync(&self, request: Request) -> Result<Response, HandlerError> {
    fallback_handle().block_on(self.handle(request))
  }
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
          && let Some(dir_name) = entry_path.file_name().and_then(|n| n.to_str())
        {
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

/// Ensure Python is initialized exactly once with proper symbol visibility.
///
/// This function uses a OnceLock to ensure Python initialization happens only once
/// per process, even when called from multiple threads or tests.
pub(crate) fn ensure_python_initialized() {
  use std::sync::OnceLock;
  static INIT: OnceLock<()> = OnceLock::new();
  INIT.get_or_init(|| {
    // On Linux, load Python library with RTLD_GLOBAL to expose interpreter symbols
    #[cfg(target_os = "linux")]
    unsafe {
      let mut info: libc::Dl_info = mem::zeroed();
      if libc::dladdr(pyo3::ffi::Py_Initialize as *const _, &mut info) == 0
        || info.dli_fname.is_null()
      {
        eprintln!("unable to locate libpython for RTLD_GLOBAL promotion");
      } else {
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
      }
    }

    Python::initialize();
  });
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::env;
  use std::fs;
  use std::sync::Arc;
  use tokio::io::DuplexStream;

  /// Helper to create a test duplex stream pair
  fn create_test_streams() -> (DuplexStream, DuplexStream) {
    tokio::io::duplex(1024)
  }

  #[test]
  fn test_find_python_site_packages_empty() {
    // Test with a non-existent directory
    let temp_dir = std::env::temp_dir().join("nonexistent_venv");
    let result = find_python_site_packages(&temp_dir);
    assert_eq!(result.len(), 0);
  }

  #[test]
  fn test_find_python_site_packages_with_structure() {
    // Create a temporary directory structure that mimics a virtual environment
    let temp_dir = std::env::temp_dir().join("test_venv_structure");
    let lib_dir = temp_dir.join("lib");
    let python_dir = lib_dir.join("python3.12");
    let site_packages = python_dir.join("site-packages");

    // Create the directory structure
    fs::create_dir_all(&site_packages).ok();

    let result = find_python_site_packages(&temp_dir);

    // Verify we found at least one site-packages directory
    assert!(!result.is_empty());
    assert!(result.iter().any(|p| p.ends_with("site-packages")));

    // Cleanup
    fs::remove_dir_all(&temp_dir).ok();
  }

  #[test]
  fn test_find_python_site_packages_multiple_versions() {
    // Create a temporary directory with multiple Python versions
    let temp_dir = std::env::temp_dir().join("test_venv_multi");

    for version in &["python3.11", "python3.12"] {
      let lib_dir = temp_dir.join("lib");
      let python_dir = lib_dir.join(version);
      let site_packages = python_dir.join("site-packages");
      fs::create_dir_all(&site_packages).ok();
    }

    let result = find_python_site_packages(&temp_dir);

    // Should find both site-packages directories
    assert!(result.len() >= 1);

    // Cleanup
    fs::remove_dir_all(&temp_dir).ok();
  }

  #[tokio::test]
  async fn test_setup_python_paths() {
    ensure_python_initialized();

    Python::attach(|py| {
      let docroot = PathBuf::from("/test/docroot");
      let result = setup_python_paths(py, &docroot);

      // Should succeed
      assert!(result.is_ok());

      // Verify sys.path was modified
      let sys = py.import("sys").unwrap();
      let path = sys.getattr("path").unwrap();
      let path_list: Vec<String> = path.extract().unwrap();

      // The docroot should be in the path
      assert!(path_list.iter().any(|p| p.contains("test/docroot")));
    });
  }

  #[tokio::test]
  async fn test_setup_python_paths_with_venv() {
    ensure_python_initialized();

    // Create a test virtual environment structure
    let temp_venv = std::env::temp_dir().join("test_venv_for_paths");
    let lib_dir = temp_venv.join("lib");
    let python_dir = lib_dir.join("python3.12");
    let site_packages = python_dir.join("site-packages");
    fs::create_dir_all(&site_packages).ok();

    // Set VIRTUAL_ENV
    unsafe {
      env::set_var("VIRTUAL_ENV", temp_venv.to_string_lossy().to_string());
    }

    Python::attach(|py| {
      let docroot = PathBuf::from("/test/docroot");
      let result = setup_python_paths(py, &docroot);
      assert!(result.is_ok());
    });

    // Cleanup
    unsafe {
      env::remove_var("VIRTUAL_ENV");
    }
    fs::remove_dir_all(&temp_venv).ok();
  }

  #[tokio::test]
  async fn test_handle_python_exception_with_error() {
    ensure_python_initialized();

    let (tx, mut rx) = oneshot::channel::<HttpResponseResult>();

    let py_err = Python::attach(|_py| {
      // Create a Python exception
      pyo3::exceptions::PyValueError::new_err("Test error")
    });

    let should_break =
      handle_python_exception::<tokio::io::DuplexStream>(Err(py_err), Some(tx), None, None).await;

    // Should return true to break the loop
    assert!(should_break);

    // Should have sent an error
    let result = rx.try_recv();
    assert!(result.is_ok());
    let result = result.unwrap();
    assert!(result.is_err());
  }

  #[tokio::test]
  async fn test_handle_python_exception_without_tx() {
    ensure_python_initialized();

    let py_err = Python::attach(|_py| pyo3::exceptions::PyValueError::new_err("Test error"));

    // Should handle gracefully when no sender is provided
    let should_break =
      handle_python_exception::<tokio::io::DuplexStream>(Err(py_err), None, None, None).await;
    assert!(should_break);
  }

  #[tokio::test]
  async fn test_handle_python_exception_with_success() {
    ensure_python_initialized();

    let (tx, _rx) = oneshot::channel::<HttpResponseResult>();

    let py_obj = Python::attach(|py| py.None());

    let should_break =
      handle_python_exception::<tokio::io::DuplexStream>(Ok(py_obj), Some(tx), None, None).await;

    // Should still return true (Python future completed)
    assert!(should_break);
  }

  #[test]
  fn test_handle_response_timeout() {
    let (tx, mut rx) = oneshot::channel::<HttpResponseResult>();

    let should_break = handle_response_timeout(Some(tx));

    // Should return true to break the loop
    assert!(should_break);

    // Should have sent a timeout error
    let result = rx.try_recv();
    assert!(result.is_ok());
    let result = result.unwrap();
    assert!(result.is_err());

    match result {
      Err(HandlerError::NoResponse) => (),
      _ => panic!("Expected NoResponse error"),
    }
  }

  #[test]
  fn test_handle_response_timeout_no_tx() {
    // Should handle gracefully when no sender is provided
    let should_break = handle_response_timeout(None);
    assert!(should_break);
  }

  #[tokio::test]
  async fn test_handle_http_response_message_start() {
    let (_request_stream, mut response_stream) = create_test_streams();

    let (tx, mut rx) = oneshot::channel::<HttpResponseResult>();
    let mut response_tx = Some(tx);

    let (ack_tx, mut ack_rx) = oneshot::channel::<()>();

    let msg = Some(AcknowledgedMessage {
      message: HttpSendMessage::HttpResponseStart {
        status: 200,
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        trailers: false,
      },
      ack: ack_tx,
    });

    let should_break =
      handle_http_response_message(msg, &mut response_tx, &mut response_stream).await;

    // Should not break yet
    assert!(!should_break);

    // Response should have been sent
    assert!(response_tx.is_none());

    let result = rx.try_recv();
    assert!(result.is_ok());
    let (status, headers) = result.unwrap().unwrap();
    assert_eq!(status, 200);
    assert_eq!(headers.len(), 1);

    // Ack should have been sent
    assert!(ack_rx.try_recv().is_ok());
  }

  #[tokio::test]
  async fn test_handle_http_response_message_body() {
    let (_request_stream, mut response_stream) = create_test_streams();

    let mut response_tx = None; // Already sent

    let (ack_tx, mut ack_rx) = oneshot::channel::<()>();

    let msg = Some(AcknowledgedMessage {
      message: HttpSendMessage::HttpResponseBody {
        body: b"Hello, World!".to_vec(),
        more_body: true,
      },
      ack: ack_tx,
    });

    let should_break =
      handle_http_response_message(msg, &mut response_tx, &mut response_stream).await;

    // Should not break (more_body is true)
    assert!(!should_break);

    // Ack should have been sent
    assert!(ack_rx.try_recv().is_ok());
  }

  #[tokio::test]
  async fn test_handle_http_response_message_body_final() {
    let (_request_stream, mut response_stream) = create_test_streams();

    let mut response_tx = None;

    let (ack_tx, mut ack_rx) = oneshot::channel::<()>();

    let msg = Some(AcknowledgedMessage {
      message: HttpSendMessage::HttpResponseBody {
        body: b"Final chunk".to_vec(),
        more_body: false, // Final chunk
      },
      ack: ack_tx,
    });

    let should_break =
      handle_http_response_message(msg, &mut response_tx, &mut response_stream).await;

    // Should break (final chunk)
    assert!(should_break);

    // Ack should have been sent
    assert!(ack_rx.try_recv().is_ok());
  }

  #[tokio::test]
  async fn test_handle_http_response_message_none() {
    let (_request_stream, mut response_stream) = create_test_streams();

    let mut response_tx = None;

    let should_break =
      handle_http_response_message(None, &mut response_tx, &mut response_stream).await;

    // Should break (channel closed)
    assert!(should_break);
  }

  #[tokio::test]
  async fn test_handle_websocket_response_message_send_text() {
    let (_request_stream, response_stream) = create_test_streams();
    let encoder = WebSocketEncoder::new(response_stream);

    let (ack_tx, mut ack_rx) = oneshot::channel::<()>();

    let msg = Some(AcknowledgedMessage {
      message: WebSocketSendMessage::Send {
        text: Some("Hello, WebSocket!".to_string()),
        bytes: None,
      },
      ack: ack_tx,
    });

    let should_break = handle_websocket_response_message(msg, &encoder).await;

    // Should not break
    assert!(!should_break);

    // Ack should have been sent
    assert!(ack_rx.try_recv().is_ok());
  }

  #[tokio::test]
  async fn test_handle_websocket_response_message_send_bytes() {
    let (_request_stream, response_stream) = create_test_streams();
    let encoder = WebSocketEncoder::new(response_stream);

    let (ack_tx, mut ack_rx) = oneshot::channel::<()>();

    let msg = Some(AcknowledgedMessage {
      message: WebSocketSendMessage::Send {
        text: None,
        bytes: Some(vec![1, 2, 3, 4]),
      },
      ack: ack_tx,
    });

    let should_break = handle_websocket_response_message(msg, &encoder).await;

    // Should not break
    assert!(!should_break);

    // Ack should have been sent
    assert!(ack_rx.try_recv().is_ok());
  }

  #[tokio::test]
  async fn test_handle_websocket_response_message_close() {
    let (_request_stream, response_stream) = create_test_streams();
    let encoder = WebSocketEncoder::new(response_stream);

    let (ack_tx, _ack_rx) = oneshot::channel::<()>();

    let msg = Some(AcknowledgedMessage {
      message: WebSocketSendMessage::Close {
        code: Some(1000),
        reason: Some("Normal closure".to_string()),
      },
      ack: ack_tx,
    });

    let should_break = handle_websocket_response_message(msg, &encoder).await;

    // Should break (close message)
    assert!(should_break);
  }

  #[tokio::test]
  async fn test_handle_websocket_response_message_none() {
    let (_request_stream, response_stream) = create_test_streams();
    let encoder = WebSocketEncoder::new(response_stream);

    let should_break = handle_websocket_response_message(None, &encoder).await;

    // Should break (channel closed)
    assert!(should_break);
  }

  #[tokio::test]
  async fn test_forward_http_request_with_data() {
    let (request_stream, write_stream) = create_test_streams();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<HttpReceiveMessage>();

    // Write some data to the stream
    tokio::spawn(async move {
      let mut stream = write_stream;
      stream.write_all(b"Test data").await.unwrap();
      stream.shutdown().await.unwrap();
    });

    // Start forwarding
    tokio::spawn(forward_http_request(request_stream, tx));

    // Should receive the data
    let msg = rx.recv().await;
    assert!(msg.is_some());

    match msg.unwrap() {
      HttpReceiveMessage::Request { body, more_body } => {
        assert_eq!(body, b"Test data");
        assert!(more_body);
      }
      HttpReceiveMessage::Disconnect => panic!("Expected Request message"),
    }

    // Should receive EOF
    let msg = rx.recv().await;
    assert!(msg.is_some());

    match msg.unwrap() {
      HttpReceiveMessage::Request { body, more_body } => {
        assert!(body.is_empty());
        assert!(!more_body);
      }
      HttpReceiveMessage::Disconnect => panic!("Expected Request message"),
    }
  }

  #[tokio::test]
  async fn test_forward_http_request_empty_stream() {
    let (request_stream, write_stream) = create_test_streams();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<HttpReceiveMessage>();

    // Close the stream immediately
    drop(write_stream);

    // Start forwarding
    tokio::spawn(forward_http_request(request_stream, tx));

    // Should receive EOF immediately
    let msg = rx.recv().await;
    assert!(msg.is_some());

    match msg.unwrap() {
      HttpReceiveMessage::Request { body, more_body } => {
        assert!(body.is_empty());
        assert!(!more_body);
      }
      HttpReceiveMessage::Disconnect => panic!("Expected Request message"),
    }
  }

  #[tokio::test]
  async fn test_forward_websocket_request_text_frame() {
    use http_handler::websocket::WebSocketEncoder;

    let (request_stream, write_stream) = create_test_streams();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WebSocketReceiveMessage>();

    // Write a text frame
    tokio::spawn(async move {
      let encoder = WebSocketEncoder::new(write_stream);
      encoder
        .write_text("Hello, WebSocket!", false)
        .await
        .unwrap();
      encoder.end().await.unwrap();
    });

    // Start forwarding
    tokio::spawn(forward_websocket_request(request_stream, tx));

    // Should receive the text message
    let msg = rx.recv().await;
    assert!(msg.is_some());

    match msg.unwrap() {
      WebSocketReceiveMessage::Receive { text, bytes } => {
        assert_eq!(text, Some("Hello, WebSocket!".to_string()));
        assert!(bytes.is_none());
      }
      _ => panic!("Expected Receive message"),
    }
  }

  #[tokio::test]
  async fn test_forward_websocket_request_binary_frame() {
    use http_handler::websocket::WebSocketEncoder;

    let (request_stream, write_stream) = create_test_streams();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WebSocketReceiveMessage>();

    // Write a binary frame
    tokio::spawn(async move {
      let encoder = WebSocketEncoder::new(write_stream);
      encoder.write_binary(&[1, 2, 3, 4], false).await.unwrap();
      encoder.end().await.unwrap();
    });

    // Start forwarding
    tokio::spawn(forward_websocket_request(request_stream, tx));

    // Should receive the binary message
    let msg = rx.recv().await;
    assert!(msg.is_some());

    match msg.unwrap() {
      WebSocketReceiveMessage::Receive { text, bytes } => {
        assert!(text.is_none());
        assert_eq!(bytes, Some(vec![1, 2, 3, 4]));
      }
      _ => panic!("Expected Receive message"),
    }
  }

  #[tokio::test]
  async fn test_forward_websocket_request_close_frame() {
    use http_handler::websocket::WebSocketEncoder;

    let (request_stream, write_stream) = create_test_streams();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WebSocketReceiveMessage>();

    // Write a close frame
    tokio::spawn(async move {
      let encoder = WebSocketEncoder::new(write_stream);
      encoder
        .write_close(Some(1000), Some("Normal closure"))
        .await
        .unwrap();
    });

    // Start forwarding
    tokio::spawn(forward_websocket_request(request_stream, tx));

    // Should receive the disconnect message
    let msg = rx.recv().await;
    assert!(msg.is_some());

    match msg.unwrap() {
      WebSocketReceiveMessage::Disconnect { code, reason } => {
        assert_eq!(code, Some(1000));
        assert!(reason.is_some());
      }
      _ => panic!("Expected Disconnect message"),
    }
  }

  #[test]
  fn test_ensure_python_initialized_idempotent() {
    // Should be safe to call multiple times
    ensure_python_initialized();
    ensure_python_initialized();
    ensure_python_initialized();

    // Python should be initialized
    Python::attach(|py| {
      // Should be able to use Python
      let sys = py.import("sys").unwrap();
      assert!(sys.hasattr("version").unwrap());
    });
  }

  // ===== Integration Tests =====
  // These tests verify the full Asgi handler flow end-to-end

  /// Helper to create a test request and spawn body writing task
  /// Returns the request immediately while body writing happens concurrently
  fn create_test_request(method: &str, path: &str, body: Vec<u8>) -> Request {
    use http_handler::{Method, Uri, Version};
    use tokio::io::AsyncWriteExt;

    let method_enum = match method {
      "GET" => Method::GET,
      "POST" => Method::POST,
      "PUT" => Method::PUT,
      "DELETE" => Method::DELETE,
      _ => Method::GET,
    };

    let uri: Uri = path.parse().unwrap();

    // Build a basic HTTP request using ::http::Request builder
    let http_request = ::http::Request::builder()
      .method(method_enum)
      .uri(uri)
      .version(Version::HTTP_11)
      .body(())
      .unwrap();

    // Split into parts and body
    let (parts, _) = http_request.into_parts();

    // Create request from parts with proper body
    let request = Request::from_parts(parts, RequestBody::new());

    // Spawn a task to write body data to the request stream
    // This allows the request to be returned immediately while body writing happens concurrently
    let mut body_writer = request.body().clone();
    tokio::spawn(async move {
      if !body.is_empty() {
        body_writer.write_all(&body).await.unwrap();
      }

      // Always close the stream when done
      body_writer.shutdown().await.unwrap();
    });

    request
  }

  /// Helper to read full response body
  async fn read_response_body(response: Response) -> (u16, Vec<(String, String)>, Vec<u8>) {
    use http_body_util::BodyExt;

    let (parts, mut body) = response.into_parts();
    let status = parts.status.as_u16();

    let headers: Vec<(String, String)> = parts
      .headers
      .iter()
      .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
      .collect();

    let mut body_bytes = Vec::new();
    while let Some(result) = body.frame().await {
      if let Ok(frame) = result {
        if let Ok(data) = frame.into_data() {
          body_bytes.extend_from_slice(&data);
        }
      }
    }

    (status, headers, body_bytes)
  }

  #[tokio::test]
  async fn test_asgi_integration_basic_request() {
    ensure_python_initialized();

    // Create Asgi handler with echo_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "echo_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create a simple POST request with body
    let request = create_test_request("POST", "/test/path", b"Hello, World!".to_vec());

    // Handle the request
    let response = asgi
      .handle(request)
      .await
      .expect("Failed to handle request");

    // Read the full response
    let (status, headers, body) = read_response_body(response).await;

    // Verify response
    assert_eq!(status, 200);
    assert!(
      headers
        .iter()
        .any(|(k, v)| k == "content-type" && v.contains("application/json"))
    );
    assert!(
      headers
        .iter()
        .any(|(k, v)| k == "x-echo-method" && v == "POST")
    );
    assert!(
      headers
        .iter()
        .any(|(k, v)| k == "x-echo-path" && v == "/test/path")
    );

    let body_str = String::from_utf8(body).unwrap();
    assert!(body_str.contains("Hello, World!"));
    assert!(body_str.contains("POST"));
    assert!(body_str.contains("/test/path"));
  }

  #[tokio::test]
  async fn test_asgi_integration_streaming_response() {
    ensure_python_initialized();

    // Create Asgi handler with stream_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "stream_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create a GET request to streaming endpoint
    let request = create_test_request("GET", "/stream?count=3", vec![]);

    // Handle the request - this should return IMMEDIATELY after headers are ready
    let response = asgi
      .handle(request)
      .await
      .expect("Failed to handle request");

    // Verify we got headers back (status code available)
    let status = response.status().as_u16();
    assert_eq!(status, 200, "Should get 200 status immediately");

    // Now read the streaming body
    let (_, headers, body) = read_response_body(response).await;

    // Verify response headers
    assert!(
      headers
        .iter()
        .any(|(k, v)| k == "content-type" && v.contains("text/plain"))
    );

    // Verify we got all chunks
    let body_str = String::from_utf8(body).unwrap();
    assert!(body_str.contains("Chunk 1"));
    assert!(body_str.contains("Chunk 2"));
    assert!(body_str.contains("Chunk 3"));
  }

  #[tokio::test]
  async fn test_asgi_integration_early_header_return() {
    ensure_python_initialized();

    // Create Asgi handler with stream_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "stream_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create request
    let request = create_test_request("GET", "/stream?count=5", vec![]);

    // Track timing - handle() should return quickly with just headers
    let start = std::time::Instant::now();
    let response = asgi
      .handle(request)
      .await
      .expect("Failed to handle request");
    let header_time = start.elapsed();

    // Headers should be available almost immediately (well before all chunks)
    // The stream_app has 0.01s delay per chunk, so 5 chunks = ~50ms
    // We should get headers back in much less time
    assert!(
      header_time.as_millis() < 30,
      "Headers should return quickly, took {}ms",
      header_time.as_millis()
    );

    // Verify we have a valid response with headers
    assert_eq!(response.status().as_u16(), 200);
    assert!(response.headers().get("content-type").is_some());

    // The body stream should still be available for reading
    let (_, _, body) = read_response_body(response).await;
    let body_str = String::from_utf8(body).unwrap();

    // Verify all chunks arrived
    for i in 1..=5 {
      assert!(body_str.contains(&format!("Chunk {}", i)));
    }
  }

  #[tokio::test]
  async fn test_asgi_integration_streaming_request_body() {
    ensure_python_initialized();

    // Create Asgi handler with echo_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "echo_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create request with body
    let large_body = "x".repeat(10000); // 10KB body
    let request = create_test_request("POST", "/test", large_body.as_bytes().to_vec());

    // Handle request
    let response = asgi
      .handle(request)
      .await
      .expect("Failed to handle request");

    // Verify response
    let (status, _, body) = read_response_body(response).await;
    assert_eq!(status, 200);

    let body_str = String::from_utf8(body).unwrap();
    // The echo app should echo back our large body
    assert!(
      body_str.contains(&large_body[..100]),
      "Should contain start of body"
    );
  }

  #[tokio::test]
  async fn test_asgi_integration_websocket_connection() {
    ensure_python_initialized();

    // Create Asgi handler with websocket_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "websocket_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create WebSocket upgrade request
    let mut request = create_test_request("GET", "/echo", vec![]);

    // Add WebSocket mode extension
    request.extensions_mut().insert(http_handler::WebSocketMode);

    // Handle request
    let response = asgi
      .handle(request)
      .await
      .expect("Failed to handle request");

    // Verify we got 101 Switching Protocols
    assert_eq!(
      response.status().as_u16(),
      101,
      "Should get 101 for WebSocket upgrade"
    );

    // Response body should be the WebSocket stream
    // We can't easily test the full WebSocket flow here without more infrastructure,
    // but we've verified the upgrade works
  }

  #[tokio::test]
  async fn test_asgi_integration_error_handling() {
    ensure_python_initialized();

    // Create Asgi handler with error_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "error_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create request that should trigger error
    let request = create_test_request("GET", "/error", vec![]);

    // Handle request - should return error
    let result = asgi.handle(request).await;

    // Should get an error
    assert!(result.is_err(), "Error path should return an error");

    match result {
      Err(HandlerError::PythonError(_)) => {
        // Expected - Python raised an exception
      }
      Err(e) => panic!("Expected PythonError, got: {:?}", e),
      Ok(_) => panic!("Expected error but got success"),
    }
  }

  #[tokio::test]
  async fn test_asgi_integration_status_codes() {
    ensure_python_initialized();

    // Create Asgi handler with status_app
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "status_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Test different status codes
    for status_code in &[200, 201, 404, 500] {
      let request = create_test_request("GET", &format!("/status/{}", status_code), vec![]);
      let response = asgi
        .handle(request)
        .await
        .expect("Failed to handle request");

      assert_eq!(
        response.status().as_u16(),
        *status_code,
        "Should return status code {}",
        status_code
      );
    }
  }

  #[tokio::test]
  async fn test_asgi_integration_concurrent_requests() {
    ensure_python_initialized();

    // Create Asgi handler
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Arc::new(
      Asgi::new(
        Some(test_fixtures.to_string_lossy().to_string()),
        Some(PythonHandlerTarget {
          file: "echo_app".to_string(),
          function: "app".to_string(),
        }),
      )
      .expect("Failed to create Asgi handler"),
    );

    // Launch multiple concurrent requests
    let mut handles = vec![];

    for i in 0..10 {
      let asgi = Arc::clone(&asgi);
      let handle = tokio::spawn(async move {
        let body = format!("Request {}", i);
        let request = create_test_request("POST", "/test", body.as_bytes().to_vec());

        let response = asgi
          .handle(request)
          .await
          .expect("Failed to handle request");
        let (status, _, response_body) = read_response_body(response).await;

        assert_eq!(status, 200);
        let response_str = String::from_utf8(response_body).unwrap();
        assert!(response_str.contains(&format!("Request {}", i)));
      });
      handles.push(handle);
    }

    // Wait for all requests to complete
    for handle in handles {
      handle.await.expect("Task failed");
    }
  }

  #[tokio::test]
  async fn test_asgi_docroot() {
    ensure_python_initialized();

    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "echo_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Verify docroot is set correctly
    assert_eq!(asgi.docroot(), test_fixtures.as_path());
  }

  /// Test that replicates the exact NAPI layer pattern to see if we can reproduce the hang
  #[test]
  fn test_asgi_integration_napi_pattern() {
    use tokio::io::AsyncWriteExt;

    ensure_python_initialized();

    // Create Asgi handler
    let test_fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test/fixtures");
    let asgi = Asgi::new(
      Some(test_fixtures.to_string_lossy().to_string()),
      Some(PythonHandlerTarget {
        file: "echo_app".to_string(),
        function: "app".to_string(),
      }),
    )
    .expect("Failed to create Asgi handler");

    // Create request without spawning concurrent body writer
    let method_enum = http_handler::Method::POST;
    let uri: http_handler::Uri = "/test/path".parse().unwrap();
    let http_request = ::http::Request::builder()
      .method(method_enum)
      .uri(uri)
      .version(http_handler::Version::HTTP_11)
      .body(())
      .unwrap();
    let (parts, _) = http_request.into_parts();
    let mut request = Request::from_parts(parts, RequestBody::new());

    // Replicate the NAPI pattern: use fallback_handle().block_on()
    let response = super::fallback_handle().block_on(async {
      let body_data = b"Hello, World!";

      // Write body data synchronously (like NAPI layer does)
      {
        let body = request.body_mut();
        body
          .write_all(body_data)
          .await
          .expect("Failed to write body");
      }

      // Shutdown stream (like NAPI layer does for non-WebSocket)
      {
        let body = request.body_mut();
        body.shutdown().await.expect("Failed to shutdown stream");
      }

      // Now call handle
      asgi
        .handle(request)
        .await
        .expect("Failed to handle request")
    });

    // Read and verify response
    let (status, headers, body) = fallback_handle().block_on(read_response_body(response));
    assert_eq!(status, 200);
    assert!(
      headers
        .iter()
        .any(|(k, v)| k == "content-type" && v.contains("application/json"))
    );

    let body_str = String::from_utf8(body).expect("Invalid UTF-8 in response body");
    assert!(
      body_str.contains("Hello, World!"),
      "Response should echo the request body"
    );
  }
}
