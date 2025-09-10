//! A Rust library for handling HTTP requests using a Python backend.
//!
//! This library provides a way to handle HTTP requests in Rust by delegating
//! the handling to a Python backend. It allows you to define a Python
//! handler that can process requests and return responses.

// #![deny(clippy::all)]
#![warn(clippy::dbg_macro, clippy::print_stdout)]
#![warn(missing_docs)]

use std::ffi::c_char;
#[cfg(feature = "napi-support")]
use std::sync::Arc;

#[cfg(feature = "napi-support")]
use http_handler::napi::{Request as NapiRequest, Response as NapiResponse};
#[cfg(feature = "napi-support")]
use http_handler::{Handler, Request, Response};
#[cfg(feature = "napi-support")]
#[allow(unused_imports)]
use http_rewriter::napi::Rewriter;
#[cfg(feature = "napi-support")]
#[macro_use]
extern crate napi_derive;
#[cfg(feature = "napi-support")]
use napi::bindgen_prelude::*;

mod asgi;
use crate::asgi::HttpReceiveMessage;
pub use asgi::Asgi;
use tokio::sync::{mpsc::error::SendError, oneshot::error::RecvError};

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
  type Error = String;

  fn try_from(value: &str) -> std::result::Result<Self, String> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 2 {
      return Err("Invalid format, expected \"file:function\"".to_string());
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

#[cfg(feature = "napi-support")]
impl FromNapiValue for PythonHandlerTarget {
  unsafe fn from_napi_value(env: sys::napi_env, napi_val: sys::napi_value) -> Result<Self> {
    use pyo3::ffi::c_str;
    let mut result = PythonHandlerTarget {
      file: String::new(),
      function: String::new(),
    };

    let mut ty = 0;
    unsafe { check_status!(sys::napi_typeof(env, napi_val, &mut ty)) }?;
    if ty == sys::ValueType::napi_string {
      let mut length: usize = 0;
      unsafe {
        check_status!(sys::napi_get_value_string_utf8(
          env,
          napi_val,
          std::ptr::null_mut(),
          0,
          &mut length
        ))
      }?;
      let mut buffer = vec![0u8; length + 1];
      unsafe {
        check_status!(sys::napi_get_value_string_utf8(
          env,
          napi_val,
          buffer.as_mut_ptr() as *mut c_char,
          length + 1,
          &mut length
        ))
      }?;
      let full_str = std::str::from_utf8(&buffer[..length])
        .map_err(|_| Error::from_reason("Invalid UTF-8 string".to_string()))?;
      result = full_str.try_into().map_err(Error::from_reason)?;
    } else if ty == sys::ValueType::napi_object {
      let mut file_val: sys::napi_value = std::ptr::null_mut();
      let mut func_val: sys::napi_value = std::ptr::null_mut();
      unsafe {
        check_status!(sys::napi_get_named_property(
          env,
          napi_val,
          c_str!("file").as_ptr(),
          &mut file_val
        ))
      }?;
      unsafe {
        check_status!(sys::napi_get_named_property(
          env,
          napi_val,
          c_str!("function").as_ptr(),
          &mut func_val
        ))
      }?;
      result.file = unsafe { String::from_napi_value(env, file_val) }?;
      result.function = unsafe { String::from_napi_value(env, func_val) }?;
    } else {
      return Err(Error::from_reason(
        "Expected string or object input".to_string(),
      ));
    }

    Ok(result)
  }
}

#[cfg(feature = "napi-support")]
impl ToNapiValue for PythonHandlerTarget {
  unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
    let mut result: sys::napi_value = std::ptr::null_mut();
    let full_str = format!("{}:{}", val.file, val.function);
    unsafe {
      check_status!(sys::napi_create_string_utf8(
        env,
        full_str.as_ptr() as *const c_char,
        full_str.len() as isize,
        &mut result
      ))
    }?;
    Ok(result)
  }
}

/// Options for configuring the Python handler.
#[cfg_attr(feature = "napi-support", napi(object))]
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
#[cfg(feature = "napi-support")]
#[napi(js_name = "Python")]
pub struct PythonHandler {
  asgi: Arc<Asgi>,
}

#[cfg(feature = "napi-support")]
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
  pub fn new(options: Option<PythonOptions>) -> Result<Self> {
    let options = options.unwrap_or_default();
    let asgi = Asgi::new(options.docroot, options.app_target)
      .map_err(|e| Error::from_reason(e.to_string()))?;

    Ok(PythonHandler {
      asgi: Arc::new(asgi),
    })
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
  pub fn docroot(&self) -> String {
    // We need to access the docroot from the Asgi struct
    // Since Asgi has a PathBuf docroot field, we convert it to String
    self.asgi.docroot().display().to_string()
  }

  /// Handle a Python request.
  ///
  /// # Examples
  ///
  /// ```js
  /// const python = new Python({
  ///   docroot: process.cwd(),
  ///   argv: process.argv
  /// });
  ///
  /// const response = await python.handleRequest(new Request({
  ///   method: 'GET',
  ///   url: 'http://example.com'
  /// }));
  ///
  /// console.log(response.status);
  /// console.log(response.body);
  /// ```
  #[napi]
  pub async fn handle_request(&self, request: &NapiRequest) -> Result<NapiResponse> {
    use std::ops::Deref;
    let response = self
      .asgi
      .handle(request.deref().clone())
      .await
      .map_err(|e| Error::from_reason(e.to_string()))?;
    Ok(response.into())
  }
  // pub fn handle_request(
  //   &self,
  //   request: &NapiRequest,
  //   signal: Option<AbortSignal>,
  // ) -> AsyncTask<PythonRequestTask> {
  //   use std::ops::Deref;
  //   AsyncTask::with_optional_signal(
  //     PythonRequestTask {
  //       asgi: self.asgi.clone(),
  //       request: request.deref().clone(),
  //     },
  //     signal,
  //   )
  // }

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
    use std::ops::Deref;
    let mut task = PythonRequestTask {
      asgi: self.asgi.clone(),
      request: request.deref().clone(),
    };

    task.compute().map(Into::<NapiResponse>::into)
  }
}

/// Task container to run a Python request in a worker thread.
#[cfg(feature = "napi-support")]
pub struct PythonRequestTask {
  asgi: Arc<Asgi>,
  request: Request,
}

/// Error types for the Python request handler.
#[allow(clippy::large_enum_variant)]
#[derive(thiserror::Error, Debug)]
pub enum HandlerError {
  /// IO errors that may occur during file operations.
  #[error("IO Error: {0}")]
  IoError(#[from] std::io::Error),

  /// Error when the current directory cannot be determined.
  #[error("Failed to get current directory: {0}")]
  CurrentDirectoryError(std::io::Error),

  /// Error when the entry point for the Python application is not found.
  #[error("Entry point not found: {0}")]
  EntrypointNotFoundError(std::io::Error),

  /// Error when converting a string to a C-compatible string.
  #[error("Failed to convert string: {0}")]
  StringCovertError(#[from] std::ffi::NulError),

  /// Error when a Python operation fails.
  #[error("Python error: {0}")]
  PythonError(#[from] pyo3::prelude::PyErr),

  /// Error when response channel is closed before sending a response.
  #[error("No response sent")]
  NoResponse,

  /// Error when response is interrupted.
  #[error("Response interrupted")]
  ResponseInterrupted,

  /// Error when response channel is closed.
  #[error("Response channel closed: {0}")]
  ResponseChannelClosed(#[from] RecvError),

  /// Error when unable to send message to Python.
  #[error("Unable to send message to Python: {0}")]
  UnableToSendMessageToPython(#[from] SendError<HttpReceiveMessage>),

  /// Error when creating an HTTP response fails.
  #[error("Failed to create response: {0}")]
  HttpHandlerError(#[from] http_handler::Error),

  /// Error when event loop is closed.
  #[error("Event loop closed")]
  EventLoopClosed,

  /// Error when PYTHON_NODE_WORKERS is invalid
  #[error("Invalid PYTHON_NODE_WORKERS count: {0}")]
  InvalidWorkerCount(#[from] std::num::ParseIntError),
}

#[cfg(feature = "napi-support")]
#[cfg_attr(feature = "napi-support", napi)]
impl Task for PythonRequestTask {
  type Output = Response;
  type JsValue = NapiResponse;

  // Handle the Python request in the worker thread.
  fn compute(&mut self) -> Result<Self::Output> {
    self
      .asgi
      .handle_sync(self.request.clone())
      .map_err(|err| Error::from_reason(err.to_string()))
  }

  // Handle converting the PHP response to a JavaScript response in the main thread.
  fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
    Ok(Into::<NapiResponse>::into(output))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_python_handler_target_try_from_str_valid() {
    let target = PythonHandlerTarget::try_from("main:app").unwrap();
    assert_eq!(target.file, "main");
    assert_eq!(target.function, "app");

    let target = PythonHandlerTarget::try_from("my_module:my_function").unwrap();
    assert_eq!(target.file, "my_module");
    assert_eq!(target.function, "my_function");
  }

  #[test]
  fn test_python_handler_target_try_from_str_invalid() {
    // No colon
    let result = PythonHandlerTarget::try_from("invalid");
    assert!(result.is_err());
    assert_eq!(
      result.unwrap_err(),
      "Invalid format, expected \"file:function\""
    );

    // Multiple colons
    let result = PythonHandlerTarget::try_from("too:many:colons");
    assert!(result.is_err());
    assert_eq!(
      result.unwrap_err(),
      "Invalid format, expected \"file:function\""
    );

    // Empty string
    let result = PythonHandlerTarget::try_from("");
    assert!(result.is_err());
    assert_eq!(
      result.unwrap_err(),
      "Invalid format, expected \"file:function\""
    );

    // Only colon - this actually succeeds with empty file and function
    // The current implementation allows this: ":" -> file="", function=""
    let result = PythonHandlerTarget::try_from(":");
    assert!(result.is_ok());
    let target = result.unwrap();
    assert_eq!(target.file, "");
    assert_eq!(target.function, "");

    // Test with empty parts in different ways
    let result = PythonHandlerTarget::try_from(":function");
    assert!(result.is_ok());
    let target = result.unwrap();
    assert_eq!(target.file, "");
    assert_eq!(target.function, "function");

    let result = PythonHandlerTarget::try_from("file:");
    assert!(result.is_ok());
    let target = result.unwrap();
    assert_eq!(target.file, "file");
    assert_eq!(target.function, "");
  }

  #[test]
  fn test_python_handler_target_from_string_conversion() {
    let target = PythonHandlerTarget {
      file: "test_module".to_string(),
      function: "test_function".to_string(),
    };
    let result: String = target.into();
    assert_eq!(result, "test_module:test_function");
  }

  #[test]
  fn test_python_handler_target_default() {
    let target = PythonHandlerTarget::default();
    assert_eq!(target.file, "main");
    assert_eq!(target.function, "app");
  }

  #[test]
  fn test_python_handler_target_debug_clone_eq_hash() {
    let target1 = PythonHandlerTarget {
      file: "test".to_string(),
      function: "app".to_string(),
    };
    let target2 = target1.clone();

    // Test Debug
    let debug_str = format!("{:?}", target1);
    assert!(debug_str.contains("test"));
    assert!(debug_str.contains("app"));

    // Test Clone and PartialEq
    assert_eq!(target1, target2);

    // Test inequality
    let target3 = PythonHandlerTarget {
      file: "different".to_string(),
      function: "app".to_string(),
    };
    assert_ne!(target1, target3);

    // Test Hash by putting in a HashSet
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(target1);
    set.insert(target2); // Should not increase size due to equality
    assert_eq!(set.len(), 1);
  }
}
