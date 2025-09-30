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
    Python::attach(|py| {
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
    Python::attach(|py| {
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
    Python::attach(|py| {
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

  #[test]
  fn test_lifespan_send_message_from_pyobject_error_cases() {
    Python::attach(|py| {
      // Test missing 'type' key
      let dict = PyDict::new(py);
      let result: Result<LifespanSendMessage, _> = dict.extract();
      assert!(result.is_err());
      assert!(
        result
          .unwrap_err()
          .to_string()
          .contains("Missing 'type' key")
      );

      // Test unknown message type
      let dict = PyDict::new(py);
      dict.set_item("type", "unknown.message.type").unwrap();
      let result: Result<LifespanSendMessage, _> = dict.extract();
      assert!(result.is_err());
      assert!(
        result
          .unwrap_err()
          .to_string()
          .contains("Unknown Lifespan send message type")
      );

      // Test non-dict object
      let list = py.eval(c"[]", None, None).unwrap();
      let result: Result<LifespanSendMessage, _> = list.extract();
      assert!(result.is_err());

      // Test invalid type value (not string)
      let dict = PyDict::new(py);
      dict.set_item("type", 123).unwrap();
      let result: Result<LifespanSendMessage, _> = dict.extract();
      assert!(result.is_err());
    });
  }

  #[test]
  fn test_lifespan_send_message_traits() {
    // Test Debug trait
    let msg1 = LifespanSendMessage::LifespanStartupComplete;
    let msg2 = LifespanSendMessage::LifespanShutdownComplete;

    let debug1 = format!("{:?}", msg1);
    let debug2 = format!("{:?}", msg2);
    assert!(debug1.contains("LifespanStartupComplete"));
    assert!(debug2.contains("LifespanShutdownComplete"));

    // Test Clone
    let cloned1 = msg1.clone();
    let cloned2 = msg2.clone();

    // Test PartialEq and Eq
    assert_eq!(msg1, cloned1);
    assert_eq!(msg2, cloned2);
    assert_ne!(msg1, msg2);

    // Test Hash
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(msg1);
    set.insert(cloned1); // Should not increase size due to equality
    set.insert(msg2);
    assert_eq!(set.len(), 2); // Only unique messages
  }

  #[test]
  fn test_lifespan_receive_message_traits() {
    // Test all the derive traits for LifespanReceiveMessage
    let msg1 = LifespanReceiveMessage::LifespanStartup;
    let msg2 = LifespanReceiveMessage::LifespanShutdown;

    // Test Debug
    let debug1 = format!("{:?}", msg1);
    let debug2 = format!("{:?}", msg2);
    assert!(debug1.contains("LifespanStartup"));
    assert!(debug2.contains("LifespanShutdown"));

    // Test Clone and Copy
    let cloned1 = msg1.clone();
    let copied1 = msg1;

    // Test PartialEq and Eq
    assert_eq!(msg1, cloned1);
    assert_eq!(msg1, copied1);
    assert_ne!(msg1, msg2);

    // Test Hash
    use std::collections::HashSet;
    let mut set = HashSet::new();
    set.insert(msg1);
    set.insert(copied1); // Should not increase size due to equality
    set.insert(msg2);
    assert_eq!(set.len(), 2); // Only unique messages
  }

  #[test]
  fn test_lifespan_scope_with_populated_state() {
    Python::attach(|py| {
      // Create a state dictionary with some data
      let state_dict = PyDict::new(py);
      state_dict.set_item("initialized", true).unwrap();
      state_dict.set_item("counter", 42).unwrap();

      let lifespan_scope = LifespanScope {
        state: Some(state_dict.unbind()),
      };

      let py_obj = lifespan_scope.into_pyobject(py).unwrap();

      // Verify the scope structure
      assert_eq!(
        dict_extract!(py_obj, "type", String),
        "lifespan".to_string()
      );

      // Verify ASGI info is present
      let asgi_info = dict_get!(py_obj, "asgi");
      let asgi_dict = asgi_info.downcast::<PyDict>().unwrap();
      assert_eq!(
        asgi_dict
          .get_item("version")
          .unwrap()
          .unwrap()
          .extract::<String>()
          .unwrap(),
        "3.0"
      );
      assert_eq!(
        asgi_dict
          .get_item("spec_version")
          .unwrap()
          .unwrap()
          .extract::<String>()
          .unwrap(),
        "2.0"
      );

      // Verify state is preserved
      let state_obj = dict_get!(py_obj, "state");
      let state_dict = state_obj.downcast::<PyDict>().unwrap();
      assert_eq!(
        state_dict
          .get_item("initialized")
          .unwrap()
          .unwrap()
          .extract::<bool>()
          .unwrap(),
        true
      );
      assert_eq!(
        state_dict
          .get_item("counter")
          .unwrap()
          .unwrap()
          .extract::<i32>()
          .unwrap(),
        42
      );
    });
  }
}
