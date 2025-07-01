use std::{
  env::{current_dir, var},
  ffi::CString,
  fs::{read_dir, read_to_string},
  path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use std::os::raw::c_void;

#[cfg(target_os = "linux")]
unsafe extern "C" {
  fn dlopen(filename: *const i8, flag: i32) -> *mut c_void;
}

use bytes::BytesMut;
use http_handler::{Handler, Request, RequestExt, Response, extensions::DocumentRoot};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::PyModule;

use crate::{HandlerError, PythonHandlerTarget};

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

/// Load Python library with RTLD_GLOBAL on Linux to make symbols available
#[cfg(target_os = "linux")]
fn ensure_python_symbols_global() {
  unsafe {
    // Try to find the system Python library dynamically
    use std::process::Command;

    // First try to find the Python library using find command
    if let Ok(output) = Command::new("find")
      .args(&[
        "/usr/lib",
        "/usr/lib64",
        "/usr/local/lib",
        "-name",
        "libpython3*.so.*",
        "-type",
        "f",
      ])
      .output()
    {
      let output_str = String::from_utf8_lossy(&output.stdout);
      for lib_path in output_str.lines() {
        if let Ok(lib_cstring) = CString::new(lib_path) {
          let handle = dlopen(lib_cstring.as_ptr(), RTLD_NOW | RTLD_GLOBAL);
          if !handle.is_null() {
            // Successfully loaded Python library with RTLD_GLOBAL
            return;
          }
        }
      }
    }

    const RTLD_GLOBAL: i32 = 0x100;
    const RTLD_NOW: i32 = 0x2;

    // Fallback to trying common library names if find command fails
    // Try a range of Python versions (3.9 to 3.100 should cover future versions)
    for minor in 9..=100 {
      let lib_name = format!("libpython3.{}.so.1.0\0", minor);
      let handle = dlopen(lib_name.as_ptr() as *const i8, RTLD_NOW | RTLD_GLOBAL);
      if !handle.is_null() {
        // Successfully loaded Python library with RTLD_GLOBAL
        return;
      }
    }

    eprintln!("Failed to locate system Python library");
  }
}

/// Core ASGI handler that loads and manages a Python ASGI application
pub struct Asgi {
  app_function: PyObject,
  docroot: PathBuf,
}

impl Asgi {
  /// Create a new Asgi instance, loading the Python app immediately
  pub fn new(
    docroot: Option<String>,
    app_target: Option<PythonHandlerTarget>,
  ) -> Result<Self, HandlerError> {
    pyo3::prepare_freethreaded_python();

    // Ensure Python symbols are globally available before initializing
    #[cfg(target_os = "linux")]
    ensure_python_symbols_global();

    // Determine document root
    let docroot = PathBuf::from(if let Some(docroot) = docroot {
      docroot
    } else {
      current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(HandlerError::CurrentDirectoryError)?
    });

    // Load Python app immediately
    let target = app_target.unwrap_or_default();
    let app_function = Self::load_python_app(&docroot, &target)?;

    Ok(Asgi {
      app_function,
      docroot,
    })
  }

  fn load_python_app(
    docroot: &Path,
    target: &PythonHandlerTarget,
  ) -> Result<PyObject, HandlerError> {
    // Load and compile Python module
    let entrypoint = docroot
      .join(format!("{}.py", target.file))
      .canonicalize()
      .map_err(HandlerError::EntrypointNotFoundError)?;

    let code = read_to_string(entrypoint).map_err(HandlerError::EntrypointNotFoundError)?;
    let code = CString::new(code).map_err(HandlerError::StringCovertError)?;
    let file_name =
      CString::new(format!("{}.py", target.file)).map_err(HandlerError::StringCovertError)?;
    let module_name = CString::new(target.file.clone()).map_err(HandlerError::StringCovertError)?;

    Python::with_gil(|py| -> PyResult<PyObject> {
      // Set up sys.path with docroot and virtual environment paths
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

      let module = PyModule::from_code(py, &code, &file_name, &module_name)?;
      Ok(module.getattr(&target.function)?.unbind())
    })
    .map_err(HandlerError::PythonError)
  }

  /// Get the document root for this ASGI handler.
  pub fn docroot(&self) -> &Path {
    &self.docroot
  }

  /// Handle a request synchronously using the pyo3_async_runtimes managed runtime
  pub fn handle_sync(&self, request: Request) -> Result<Response, HandlerError> {
    pyo3_async_runtimes::tokio::get_runtime().block_on(self.handle(request))
  }

  /// Install an event loop for this thread, using uvloop if available
  pub fn install_loop(&self) -> Result<(), HandlerError> {
    Python::with_gil(|py| -> PyResult<()> {
      let asyncio = py.import("asyncio")?;

      // Check if there's already an event loop on this thread
      let needs_new_loop = match asyncio.call_method0("get_event_loop") {
        Ok(existing_loop) => {
          // Check if the existing loop is closed
          existing_loop.call_method0("is_closed")?.extract::<bool>()?
        }
        Err(_) => true, // No event loop exists
      };

      if needs_new_loop {
        // Set up event loop for this thread, using uvloop if available
        let loop_ = if let Ok(uvloop) = py.import("uvloop") {
          // Install uvloop policy if not already installed
          let _ = uvloop.call_method0("install");
          uvloop.call_method0("new_event_loop")?
        } else {
          asyncio.call_method0("new_event_loop")?
        };
        asyncio.call_method1("set_event_loop", (&loop_,))?;
      }

      Ok(())
    })
    .map_err(HandlerError::PythonError)
  }
}

#[async_trait::async_trait]
impl Handler for Asgi {
  type Error = HandlerError;

  async fn handle(&self, request: Request) -> Result<Response, Self::Error> {
    // Ensure the event loop is installed
    self.install_loop()?;

    // Set document root extension
    let mut request = request;
    request.set_document_root(DocumentRoot {
      path: self.docroot.clone(),
    });

    // Create the ASGI scope from the HTTP request
    let scope: HttpConnectionScope = (&request).try_into().map_err(HandlerError::PythonError)?;

    // Create channels for ASGI communication
    let (rx_receiver, rx) = Receiver::http();
    let (tx_sender, mut tx_receiver) = Sender::http();

    // Send request body to Python app
    let request_message = HttpReceiveMessage::Request {
      body: request.body().to_vec(),
      more_body: false,
    };
    if rx.send(request_message).is_err() {
      return Err(HandlerError::PythonError(PyRuntimeError::new_err(
        "Failed to send request to Python app",
      )));
    }

    // Process messages in a separate task
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
      let mut status = 500u16;
      let mut headers = Vec::new();
      let mut body = Vec::new();
      let mut response_started = false;

      while let Some(ack_msg) = tx_receiver.recv().await {
        let AcknowledgedMessage { message, ack } = ack_msg;

        // Process the message
        match message {
          HttpSendMessage::HttpResponseStart {
            status: s,
            headers: h,
            trailers: _,
          } => {
            status = s;
            headers = h;
            response_started = true;
          }
          HttpSendMessage::HttpResponseBody { body: b, more_body } => {
            if response_started {
              body.extend_from_slice(&b);
              if !more_body {
                // Response is complete - send acknowledgment before returning
                let _ = ack.send(());
                let _ = response_tx.send(Ok((status, headers, body)));
                return;
              }
            }
          }
        }

        // Send acknowledgment that message was processed
        let _ = ack.send(());
      }

      // Channel closed without complete response
      if response_started {
        let _ = response_tx.send(Err(HandlerError::ResponseInterrupted));
      } else {
        let _ = response_tx.send(Err(HandlerError::NoResponse));
      }
    });

    // Execute Python
    let py_func = Python::with_gil(|py| self.app_function.clone_ref(py));

    // Now create the coroutine and convert it to a future
    let coroutine = Python::with_gil(|py| {
      let scope_py = scope.into_pyobject(py)?;
      py_func.call1(py, (scope_py, rx_receiver, tx_sender))
    })?;

    // TODO: This will block the current thread until the coroutine completes.
    // We should see if there's a way to execute coroutines concurrently.
    // Blocking in an async function is not great as tokio will assume the
    // function should yield control when it's not busy, so we're wasting a
    // thread here. Likely we should implement `Stream` around a coroutine
    // wrapper to poll it instead. The `run` is internally running the
    // `run_until_complete` method, which blocks the current thread until
    // the coroutine completes.
    Python::with_gil(|py| {
      pyo3_async_runtimes::tokio::run(py, async move {
        Python::with_gil(|py| pyo3_async_runtimes::tokio::into_future(coroutine.into_bound(py)))?
          .await
      })
    })?;

    // If an error was sent through the channel, return it
    let maybe_response = response_rx.await?;
    let (status, headers, body) = maybe_response?;

    // If we reach here, we have a valid response
    let mut builder = http_handler::response::Builder::new().status(status);

    for (name, value) in headers {
      builder = builder.header(&name, &value);
    }

    builder
      .body(BytesMut::from(&body[..]))
      .map_err(HandlerError::HttpHandlerError)
  }
}
