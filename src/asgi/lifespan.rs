use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::asgi::AsgiInfo;

/// The lifespan scope exists for the duration of the event loop.
#[derive(Debug)]
pub struct LifespanScope {
  /// An empty namespace where the application can persist state to be used
  /// when handling subsequent requests. Optional; if missing the server
  /// does not support this feature.
  state: Option<Py<PyDict>>,
}

impl<'py> IntoPyObject<'py> for LifespanScope {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
    let dict = PyDict::new(py);
    dict.set_item("type", "lifespan")?;
    dict.set_item("asgi", AsgiInfo::new("3.0", "2.0").into_pyobject(py)?)?;
    dict.set_item("state", self.state)?;
    Ok(dict)
  }
}

//
// Lifespan Scope
//

/// Lifespan Scope messages given to `receive()` function of an ASGI application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifespanReceiveMessage {
  LifespanStartup,
  LifespanShutdown,
}

// Only ever converted from Rust to Python.
impl<'py> IntoPyObject<'py> for LifespanReceiveMessage {
  type Target = PyDict;
  type Output = Bound<'py, Self::Target>;
  type Error = PyErr;

  fn into_pyobject(self, py: Python<'py>) -> PyResult<Self::Output> {
    let dict = PyDict::new(py);
    match self {
      LifespanReceiveMessage::LifespanStartup => {
        dict.set_item("type", "lifespan.startup")?;
      }
      LifespanReceiveMessage::LifespanShutdown => {
        dict.set_item("type", "lifespan.shutdown")?;
      }
    }
    Ok(dict)
  }
}

/// Lifespan Scope messages given to the `send()` function by an ASGI application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LifespanSendMessage {
  LifespanStartupComplete,
  LifespanShutdownComplete,
}

// Only ever converted from Python to Rust.
impl<'py> FromPyObject<'py> for LifespanSendMessage {
  fn extract_bound(ob: &Bound<'py, PyAny>) -> PyResult<Self> {
    let dict = ob.downcast::<PyDict>()?;
    let message_type = dict.get_item("type")?.ok_or_else(|| {
      PyValueError::new_err("Missing 'type' key in Lifespan send message dictionary")
    })?;

    let message_type: String = message_type.extract()?;
    match message_type.as_str() {
      "lifespan.startup.complete" => Ok(LifespanSendMessage::LifespanStartupComplete),
      "lifespan.shutdown.complete" => Ok(LifespanSendMessage::LifespanShutdownComplete),
      _ => Err(PyValueError::new_err(format!(
        "Unknown Lifespan send message type: {message_type}"
      ))),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  macro_rules! dict_get {
    ($dict:expr, $key:expr) => {
      $dict
        .get_item($key)
        .expect(&("Failed to get ".to_owned() + stringify!($key)))
        .expect(&("Item \"".to_owned() + stringify!($key) + "\" not found"))
    };
  }

  macro_rules! dict_extract {
    ($dict:expr, $key:expr, $type:ty) => {
      dict_get!($dict, $key)
        .extract::<$type>()
        .expect(&("Unable to convert to ".to_owned() + stringify!($type)))
    };
  }

  #[test]
  fn test_lifespan_scope_into_pyobject() {
    Python::with_gil(|py| {
      let lifespan_scope = LifespanScope { state: None };
      let py_obj = lifespan_scope.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(py_obj, "type", String),
        "lifespan".to_string()
      );
      assert!(dict_get!(py_obj, "state").is_none());

      let state = Some(PyDict::new(py).unbind());
      let lifespan_scope = LifespanScope { state };
      let py_obj = lifespan_scope.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(py_obj, "type", String),
        "lifespan".to_string()
      );
      assert!(!dict_get!(py_obj, "state").is_none());
    });
  }

  #[test]
  fn test_lifespan_receive_message_into_pyobject() {
    Python::with_gil(|py| {
      let message = LifespanReceiveMessage::LifespanStartup;
      let py_obj = message.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(py_obj, "type", String),
        "lifespan.startup".to_string()
      );

      let message = LifespanReceiveMessage::LifespanShutdown;
      let py_obj = message.into_pyobject(py).unwrap();
      assert_eq!(
        dict_extract!(py_obj, "type", String),
        "lifespan.shutdown".to_string()
      );
    });
  }

  #[test]
  fn test_lifespan_send_message_from_pyobject() {
    Python::with_gil(|py| {
      let dict = PyDict::new(py);
      dict.set_item("type", "lifespan.shutdown.complete").unwrap();
      let message: LifespanSendMessage = dict.extract().unwrap();
      assert_eq!(message, LifespanSendMessage::LifespanShutdownComplete);

      let dict = PyDict::new(py);
      dict.set_item("type", "lifespan.startup.complete").unwrap();
      let message: LifespanSendMessage = dict.extract().unwrap();
      assert_eq!(message, LifespanSendMessage::LifespanStartupComplete);
    });
  }
}
