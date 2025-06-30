//! A Rust library for handling HTTP requests using a Python backend.
//!
//! This library provides a way to handle HTTP requests in Rust by delegating
//! the handling to a Python backend. It allows you to define a Python
//! handler that can process requests and return responses.

// TODO: Gate napi things behind napi-support feature so this can also be a plain Rust library.
//       Without the feature gate this will fail to run cargo test due to missing napi dependencies.

// #![deny(clippy::all)]
#![warn(clippy::dbg_macro, clippy::print_stdout)]
#![warn(missing_docs)]

use std::ops::Deref;
use std::path::PathBuf;

use http_handler::{Handler, Request, Response, response::Builder};
use http_handler::napi::{Request as NapiRequest, Response as NapiResponse};
#[allow(unused_imports)]
use http_rewriter::napi::Rewriter;
#[macro_use]
extern crate napi_derive;
use napi::bindgen_prelude::*;
use pyo3::exceptions::PyRuntimeError;
use pyo3::{prelude::*, BoundObject};
use pyo3::types::IntoPyDict;
use pyo3::ffi::c_str;
use tokio::runtime::Runtime;

/// The Python module and function for handling requests.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PythonHandlerTarget {
  /// The name of the Python file (without the .py extension).
  pub file: String,
  /// The name of the function within the Python file that will handle requests.
  pub function: String,
}

impl Default for PythonHandlerTarget {
  fn default() -> Self {
    PythonHandlerTarget {
      file: "main".to_string(),
      function: "app".to_string(),
    }
  }
}

impl TryFrom<&str> for PythonHandlerTarget {
  type Error = Error;

  fn try_from(value: &str) -> Result<Self> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 2 {
      return Err(Error::from_reason("Invalid format, expected \"file:function\"".to_string()));
    }
    Ok(PythonHandlerTarget {
      file: parts[0].to_string(),
      function: parts[1].to_string(),
    })
  }
}

impl From<PythonHandlerTarget> for String {
  fn from(target: PythonHandlerTarget) -> Self {
    format!("{}:{}", target.file, target.function)
  }
}

impl FromNapiValue for PythonHandlerTarget {
  unsafe fn from_napi_value(env: sys::napi_env, napi_val: sys::napi_value) -> Result<Self> {
    let mut result = PythonHandlerTarget {
        file: String::new(),
        function: String::new(),
    };

    let mut ty = 0;
    unsafe { check_status!(sys::napi_typeof(env, napi_val, &mut ty)) }?;
    if ty == sys::ValueType::napi_string {
        let mut length: usize = 0;
        unsafe { check_status!(sys::napi_get_value_string_utf8(env, napi_val, std::ptr::null_mut(), 0, &mut length)) }?;
        let mut buffer = vec![0u8; length + 1];
        unsafe { check_status!(sys::napi_get_value_string_utf8(env, napi_val, buffer.as_mut_ptr() as *mut i8, length + 1, &mut length)) }?;
        let full_str = std::str::from_utf8(&buffer[..length])
          .map_err(|_| Error::from_reason("Invalid UTF-8 string".to_string()))?;
        result = full_str.try_into()?;
    } else if ty == sys::ValueType::napi_object {
        let mut file_val: sys::napi_value = std::ptr::null_mut();
        let mut func_val: sys::napi_value = std::ptr::null_mut();
        unsafe { check_status!(sys::napi_get_named_property(env, napi_val, c_str!("file").as_ptr(), &mut file_val)) }?;
        unsafe { check_status!(sys::napi_get_named_property(env, napi_val, c_str!("function").as_ptr(), &mut func_val)) }?;
        result.file = unsafe { String::from_napi_value(env, file_val) }?;
        result.function = unsafe { String::from_napi_value(env, func_val) }?;
    } else {
        return Err(Error::from_reason("Expected string or object input".to_string()));
    }

    Ok(result)
  }
}

impl ToNapiValue for PythonHandlerTarget {
  unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
    let mut result: sys::napi_value = std::ptr::null_mut();
    let full_str = format!("{}:{}", val.file, val.function);
    unsafe { check_status!(sys::napi_create_string_utf8(env, full_str.as_ptr() as *const i8, full_str.len() as isize, &mut result)) }?;
    Ok(result)
  }
}

/// Options for configuring the Python handler.
#[napi(object)]
#[derive(Clone, Debug, Default)]
pub struct PythonOptions {
  /// The document root for the PHP instance.
  pub docroot: Option<String>,
  /// The name of the Python module and function which will handle requests.
  /// Formatted as "module:function".
  pub app_target: Option<PythonHandlerTarget>,
  // /// Request rewriter
  // pub rewriter: Option<Rewriter>,
}

/// A Python handler that can handle HTTP requests.
// TODO: Make this actually handle Python requests
#[napi(js_name = "Python")]
pub struct PythonHandler(PythonOptions);

#[napi]
impl PythonHandler {
  /// Create a new Python handler with the given options.
  ///
  /// # Examples
  ///
  /// ```js
  /// const python = new Python({
  ///   argv: process.argv,
  ///   docroot: process.cwd(),
  /// });
  /// ```
  #[napi(constructor)]
  pub fn new(options: Option<PythonOptions>) -> Self {
    PythonHandler(options.unwrap_or_default())
  }

  /// Get the document root for this Python handler.
  ///
  /// # Examples
  ///
  /// ```js
  /// const python = new Python({
  ///   docroot: process.cwd(),
  /// });
  ///
  /// console.log(python.docroot);
  /// ```
  #[napi(getter)]
  pub fn docroot(&self) -> Option<String> {
    self.0.docroot.clone()
  }

  /// Handle a PHP request.
  ///
  /// # Examples
  ///
  /// ```js
  /// const php = new Php({
  ///   docroot: process.cwd(),
  ///   argv: process.argv
  /// });
  ///
  /// const response = php.handleRequest(new Request({
  ///   method: 'GET',
  ///   url: 'http://example.com'
  /// }));
  ///
  /// console.log(response.status);
  /// console.log(response.body);
  /// ```
  #[napi]
  pub fn handle_request(
    &self,
    request: &NapiRequest,
    signal: Option<AbortSignal>,
  ) -> AsyncTask<PythonRequestTask> {
    AsyncTask::with_optional_signal(
      PythonRequestTask {
        options: self.0.clone(),
        request: request.deref().clone(),
      },
      signal,
    )
  }

  /// Handle a PHP request synchronously.
  ///
  /// # Examples
  ///
  /// ```js
  /// const php = new Php({
  ///   docroot: process.cwd(),
  ///   argv: process.argv
  /// });
  ///
  /// const response = php.handleRequestSync(new Request({
  ///   method: 'GET',
  ///   url: 'http://example.com'
  /// }));
  ///
  /// console.log(response.status);
  /// console.log(response.body);
  /// ```
  #[napi]
  pub fn handle_request_sync(&self, request: &NapiRequest) -> Result<NapiResponse> {
    let mut task = PythonRequestTask {
      options: self.0.clone(),
      request: request.deref().clone(),
    };

    task.compute().map(Into::<NapiResponse>::into)
  }
}

/// Task container to run a Python request in a worker thread.
pub struct PythonRequestTask {
  options: PythonOptions,
  request: Request,
}

use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use pyo3::types::{PyDict, PyModule};

/// Allows Python to receive messages from Rust.
#[pyclass]
struct Receiver {
  rx: Arc<Mutex<mpsc::UnboundedReceiver<Py<PyDict>>>>
}

impl Receiver {
  pub fn new() -> (Receiver, mpsc::UnboundedSender<Py<PyDict>>) {
    let (tx, rx) = mpsc::unbounded_channel::<Py<PyDict>>();
    (Receiver { rx: Arc::new(Mutex::new(rx)) }, tx)
  }
}

#[pymethods]
impl Receiver {
  async fn __call__(&mut self) -> PyResult<Py<PyDict>> {
    let rx = self.rx.clone();

    // TODO: Don't block the current thread. This is just a test implementation.
    let next = tokio::runtime::Handle::current().block_on(async {
      println!("it got here...");
      // Wait for a message from the Rust sender
      rx.lock().await.recv().await.ok_or_else(|| PyErr::new::<PyRuntimeError, _>("No message received"))
    })?;

    Ok(next)
  }
}

/// Allows Python to send messages to Rust.
#[pyclass]
pub struct Sender {
  tx: mpsc::UnboundedSender<Py<PyDict>>
}

impl Sender {
  pub fn new() -> (Sender, mpsc::UnboundedReceiver<Py<PyDict>>) {
    let (tx, rx) = mpsc::unbounded_channel::<Py<PyDict>>();
    (Sender { tx }, rx)
  }
}

#[pymethods]
impl Sender {
  fn __call__<'a>(&'a mut self, py: Python<'a>, args: Py<PyDict>) -> PyResult<PyObject> {
    match self.tx.send(args) {
      Ok(_) => Ok(py.None()),
      Err(_) => Err(PyErr::new::<PyRuntimeError, _>("connection closed")),
    }
  }
}

#[async_trait::async_trait]
impl Handler for PythonRequestTask {
  type Error = Error;

  async fn handle(&self, request: http_handler::Request) -> std::result::Result<Response, Self::Error> {
    use std::{env::current_dir, fs::read_to_string};
    use std::ffi::CString;

    // TODO: Also do something with the rewriter, if it exists.
    let PythonOptions { docroot, app_target } = self.options.clone();

    let docroot = PathBuf::from(if let Some(docroot) = docroot { docroot.clone() } else {
      current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .map_err(|_| Error::from_reason("Failed to get current directory".to_string()))?
    });
    let target = app_target.unwrap_or_default();

    let entrypoint = docroot.join(target.file.clone() + ".py").canonicalize()
      .map_err(|_| Error::from_reason(format!("Python entrypoint not found: {}", target.file)))?;

    let code = read_to_string(entrypoint.clone())
      .map_err(|_| Error::from_reason(format!("Failed to read Python entrypoint: {}", entrypoint.display())))?;

    let code = CString::new(code)
      .map_err(|_| Error::from_reason("Failed to convert Python code to CString".to_string()))?;

    let file_name = CString::new(target.file.clone() + ".py")
      .map_err(|_| Error::from_reason("Failed to convert file name to CString".to_string()))?;

    let module_name = CString::new(target.file)
      .map_err(|_| Error::from_reason("Failed to convert module name to CString".to_string()))?;

    let app = Python::with_gil(|py| -> PyResult<PyObject> {
      Ok(PyModule::from_code(py, &code, &file_name, &module_name)?.into())
    })
      .map_err(|err| Error::from_reason(format!("Failed to load Python module: {err}")))?;

    Python::with_gil(|py| {
      let func = app.getattr(py, target.function)?;

      // For ASGI, we need to set up an ASGI application
      // This is a placeholder for actual ASGI handling logic
      let scope = [("type", "http"), ("method", "GET"), ("path", "/")].into_py_dict(py)?;
      let (receive, receiver_tx) = Receiver::new();
      let (send, mut sender_rx) = Sender::new();

      // TODO: Wire up receiver_tx and sender_rx to Request and Response.


      let lifespan_startup = Python::with_gil(|py| {
          let scope = PyDict::new(py);
          scope.set_item("type", "lifespan.startup")?;
          let scope: Py<PyDict> = scope.into();
          Ok::<Py<PyDict>, PyErr>(scope)
      })?;

      if receiver_tx.send(lifespan_startup).is_err() {
          return Err(PyErr::new::<PyRuntimeError, _>("Failed to send lifespan startup message"));
      }

      tokio::task::spawn(async move {
        while let Some(msg) = sender_rx.recv().await {
          println!("Received message from Python: {:?}", msg);
          receiver_tx.send(msg).unwrap_or_else(|_| {
            println!("Failed to send message to Rust receiver");
          });
        }
      });

      // Create a Python future for sending and receiving, which won't do anything for the mock
      let coroutine = func.call1(py, (scope, receive, send))?.into_bound(py);

      // TODO: pyo3_async_runtimes can only run Python coroutines when already
      // in closure context. So we need to use asyncio below.
      // let fut = pyo3_async_runtimes::tokio::into_future(coroutine)?;
      // pyo3_async_runtimes::tokio::run(py, fut)

      // TODO: Figure out how to execute Python coroutines as Rust futures so
      // we can rely on the Rust executor. For now, we can rely on asyncio, but
      // with the downside that it blocks the thread.
      let asyncio = py.import("asyncio")?;
      asyncio.call_method1("run", (coroutine,))?;

      Ok(())
    })
      .map_err(|err| Error::from_reason(format!("Python error: {err}")))?;

    // Ensure Python's stderr is flushed before proceeding.
    Python::with_gil(|py| py.run(c"import sys; sys.stdout.flush()", None, None))
      .map_err(|err| Error::from_reason(format!("Failed to initialize Python module: {err}")))?;

    Builder::new()
      .status(200)
      .header("Content-Type", "text/plain")
      .body(request.body().to_owned())
      .map_err(|err| Error::from_reason(err.to_string()))
  }
}

#[napi]
impl Task for PythonRequestTask {
  type Output = Response;
  type JsValue = NapiResponse;

  // Handle the PHP request in the worker thread.
  fn compute(&mut self) -> Result<Self::Output> {
    // Can't use Handle::current() as this thread won't have a runtime configured.
    // let runtime = tokio::runtime::Handle::current();
    let runtime = Runtime::new().map_err(|err| Error::from_reason(err.to_string()))?;

    runtime.block_on(async {
      let result = self.handle(self.request.clone()).await;
      result.map_err(|err| Error::from_reason(err.to_string()))
    })
  }

  // Handle converting the PHP response to a JavaScript response in the main thread.
  fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
    Ok(Into::<NapiResponse>::into(output))
  }
}
