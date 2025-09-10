use std::{
  env::{current_dir, var},
  ffi::CString,
  fs::{read_dir, read_to_string},
  path::{Path, PathBuf},
  sync::{Arc, OnceLock, RwLock, Weak},
};

#[cfg(target_os = "linux")]
use std::{ffi::CStr, mem};

use bytes::BytesMut;
use http_handler::{Handler, Request, RequestExt, Response, extensions::DocumentRoot};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use tokio::sync::oneshot;

use crate::{HandlerError, PythonHandlerTarget};

/// HTTP response tuple: (status_code, headers, body)
type HttpResponse = (u16, Vec<(String, String)>, Vec<u8>);
/// Result type for HTTP response operations
type HttpResponseResult = Result<HttpResponse, HandlerError>;

/// Global runtime for when no tokio runtime is available
static FALLBACK_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

fn fallback_handle() -> tokio::runtime::Handle {
  if let Ok(handle) = tokio::runtime::Handle::try_current() {
    handle
  } else {
    // No runtime exists, create a fallback one
    let rt = FALLBACK_RUNTIME.get_or_init(|| {
      tokio::runtime::Runtime::new().expect("Failed to create fallback tokio runtime")
    });
    rt.handle().clone()
  }
}

/// Global Python event loop handle storage
static PYTHON_EVENT_LOOP: OnceLock<RwLock<Weak<EventLoopHandle>>> = OnceLock::new();

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
  event_loop: PyObject,
}

impl EventLoopHandle {
  /// Get the Python event loop object
  pub fn event_loop(&self) -> &PyObject {
    &self.event_loop
  }
}

impl Drop for EventLoopHandle {
  fn drop(&mut self) {
    // Stop the Python event loop when the last handle is dropped
    Python::with_gil(|py| {
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
  let weak_handle = PYTHON_EVENT_LOOP.get_or_init(|| RwLock::new(Weak::new()));

  // Try to upgrade the weak reference
  if let Some(handle) = weak_handle.read().unwrap().upgrade() {
    return Ok(handle);
  }

  // Need write lock to create new handle
  let mut guard = weak_handle.write().unwrap();

  // Double-check in case another thread created it
  if let Some(handle) = guard.upgrade() {
    return Ok(handle);
  }

  // Create new event loop handle
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
  pyo3::prepare_freethreaded_python();

  // Create event loop
  let event_loop = Python::with_gil(|py| -> Result<PyObject, HandlerError> {
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
  app_function: PyObject,
}

unsafe impl Send for Asgi {}
unsafe impl Sync for Asgi {}

impl Asgi {
  /// Create a new Asgi instance, loading the Python app and using shared event loop
  pub fn new(
    docroot: Option<String>,
    app_target: Option<PythonHandlerTarget>,
  ) -> Result<Self, HandlerError> {
    // Determine document root
    let docroot = PathBuf::from(if let Some(docroot) = docroot {
      docroot
    } else {
      current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(HandlerError::CurrentDirectoryError)?
    });

    let target = app_target.unwrap_or_default();

    // Get or create shared Python event loop
    let event_loop_handle = ensure_python_event_loop()?;

    // Load Python app
    let app_function = Python::with_gil(|py| -> Result<PyObject, HandlerError> {
      // Load and compile Python module
      let entrypoint = docroot
        .join(format!("{}.py", target.file))
        .canonicalize()
        .map_err(HandlerError::EntrypointNotFoundError)?;

      let code = read_to_string(entrypoint).map_err(HandlerError::EntrypointNotFoundError)?;
      let code = CString::new(code).map_err(HandlerError::StringCovertError)?;
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

  /// Handle a request synchronously
  pub fn handle_sync(&self, request: Request) -> Result<Response, HandlerError> {
    fallback_handle().block_on(self.handle(request))
  }
}

#[async_trait::async_trait]
impl Handler for Asgi {
  type Error = HandlerError;

  async fn handle(&self, request: Request) -> Result<Response, Self::Error> {
    // Set document root extension
    let mut request = request;
    request.set_document_root(DocumentRoot {
      path: self.docroot.clone(),
    });

    // Create ASGI scope
    let scope: HttpConnectionScope = (&request).try_into()?;

    // Create channels for ASGI communication
    let (rx_receiver, rx) = Receiver::http();
    let (tx_sender, tx_receiver) = Sender::http();

    // Send request body
    let request_message = HttpReceiveMessage::Request {
      body: request.body().to_vec(),
      more_body: false,
    };
    rx.send(request_message).map_err(|_| {
      HandlerError::PythonError(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
        "Failed to send request",
      ))
    })?;

    // Create response channel
    let (response_tx, response_rx) = oneshot::channel();

    // Spawn task to collect response
    tokio::spawn(collect_response_messages(tx_receiver, response_tx));

    // Submit the ASGI app call to Python event loop
    Python::with_gil(|py| {
      let scope_py = scope.into_pyobject(py)?;
      let coro = self
        .app_function
        .call1(py, (scope_py, rx_receiver, tx_sender))?;

      let asyncio = py.import("asyncio")?;
      asyncio.call_method1(
        "run_coroutine_threadsafe",
        (coro, self.event_loop_handle.event_loop()),
      )?;

      Ok::<(), HandlerError>(())
    })?;

    // Wait for response
    let (status, headers, body) = response_rx.await??;

    // Build response
    let mut builder = http_handler::response::Builder::new().status(status);
    for (name, value) in headers {
      builder = builder.header(name.as_bytes(), value.as_bytes());
    }

    builder
      .body(BytesMut::from(&body[..]))
      .map_err(HandlerError::HttpHandlerError)
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
        if entry_path.is_dir() {
          if let Some(dir_name) = entry_path.file_name().and_then(|n| n.to_str()) {
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
fn start_python_event_loop_thread(event_loop: PyObject) {
  // Initialize Python for this thread
  pyo3::prepare_freethreaded_python();

  Python::with_gil(|py| {
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

/// Collect ASGI response messages
async fn collect_response_messages(
  mut tx_receiver: tokio::sync::mpsc::UnboundedReceiver<AcknowledgedMessage<HttpSendMessage>>,
  response_tx: oneshot::Sender<HttpResponseResult>,
) {
  let mut status = 500u16;
  let mut headers = Vec::new();
  let mut body = Vec::new();
  let mut response_started = false;

  while let Some(ack_msg) = tx_receiver.recv().await {
    let AcknowledgedMessage { message, ack } = ack_msg;

    match message {
      HttpSendMessage::HttpResponseStart {
        status: s,
        headers: h,
        ..
      } => {
        status = s;
        headers = h;
        response_started = true;
      }
      HttpSendMessage::HttpResponseBody { body: b, more_body } => {
        if response_started {
          body.extend_from_slice(&b);
          if !more_body {
            let _ = ack.send(());
            let _ = response_tx.send(Ok((status, headers, body)));
            return;
          }
        }
      }
    }

    let _ = ack.send(());
  }

  // If we got here, the channel closed without a complete response
  let _ = response_tx.send(Err(if response_started {
    HandlerError::ResponseInterrupted
  } else {
    HandlerError::NoResponse
  }));
}
