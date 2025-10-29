use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use tokio::sync::{Mutex, mpsc};

use crate::asgi::{
  http::HttpReceiveMessage, lifespan::LifespanReceiveMessage, websocket::WebSocketReceiveMessage,
};

enum ReceiverType {
  Http(Arc<Mutex<mpsc::UnboundedReceiver<HttpReceiveMessage>>>),
  WebSocket(Arc<Mutex<mpsc::UnboundedReceiver<WebSocketReceiveMessage>>>),
  Lifespan(Arc<Mutex<mpsc::UnboundedReceiver<LifespanReceiveMessage>>>),
}

/// Allows Python to receive messages from Rust.
#[pyclass]
pub struct Receiver(ReceiverType);

impl Receiver {
  /// Create a new Receiver instance for http ASGI message types.
  pub fn http() -> (Receiver, mpsc::UnboundedSender<HttpReceiveMessage>) {
    let (tx, rx) = mpsc::unbounded_channel::<HttpReceiveMessage>();
    let rx = Arc::new(Mutex::new(rx));
    (Receiver(ReceiverType::Http(rx)), tx)
  }

  /// Create a new Receiver instance for websocket ASGI message types.
  pub fn websocket() -> (Receiver, mpsc::UnboundedSender<WebSocketReceiveMessage>) {
    let (tx, rx) = mpsc::unbounded_channel::<WebSocketReceiveMessage>();
    let rx = Arc::new(Mutex::new(rx));
    (Receiver(ReceiverType::WebSocket(rx)), tx)
  }

  /// Create a new Receiver instance for lifespan ASGI message types.
  pub fn lifespan() -> (Receiver, mpsc::UnboundedSender<LifespanReceiveMessage>) {
    let (tx, rx) = mpsc::unbounded_channel::<LifespanReceiveMessage>();
    let rx = Arc::new(Mutex::new(rx));
    (Receiver(ReceiverType::Lifespan(rx)), tx)
  }
}

#[pymethods]
impl Receiver {
  async fn __call__(&mut self) -> PyResult<Py<PyDict>> {
    match &self.0 {
      ReceiverType::Http(rx) => {
        let message = rx.lock().await.recv().await;
        if let Some(msg) = message {
          Python::attach(|py| Ok(msg.into_pyobject(py)?.unbind()))
        } else {
          Err(PyValueError::new_err("No message received"))
        }
      }
      ReceiverType::WebSocket(rx) => {
        let message = rx.lock().await.recv().await;
        if let Some(msg) = message {
          Python::attach(|py| Ok(msg.into_pyobject(py)?.unbind()))
        } else {
          Err(PyValueError::new_err("No message received"))
        }
      }
      ReceiverType::Lifespan(rx) => {
        let message = rx.lock().await.recv().await;
        if let Some(msg) = message {
          Python::attach(|py| Ok(msg.into_pyobject(py)?.unbind()))
        } else {
          Err(PyValueError::new_err("No message received"))
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::asgi::ensure_python_initialized;
  use crate::asgi::http::HttpReceiveMessage;
  use crate::asgi::lifespan::LifespanReceiveMessage;
  use crate::asgi::websocket::WebSocketReceiveMessage;

  #[test]
  fn test_receiver_http_creation() {
    ensure_python_initialized();

    let (_receiver, tx) = Receiver::http();
    // Verify we can send a message
    let result = tx.send(HttpReceiveMessage::Request {
      body: vec![],
      more_body: false,
    });
    assert!(result.is_ok(), "Should be able to send message");
  }

  #[test]
  fn test_receiver_websocket_creation() {
    ensure_python_initialized();

    let (_receiver, tx) = Receiver::websocket();
    // Verify we can send a message
    let result = tx.send(WebSocketReceiveMessage::Connect);
    assert!(result.is_ok(), "Should be able to send message");
  }

  #[test]
  fn test_receiver_lifespan_creation() {
    ensure_python_initialized();

    let (_receiver, tx) = Receiver::lifespan();
    // Verify we can send a message
    let result = tx.send(LifespanReceiveMessage::LifespanStartup);
    assert!(result.is_ok(), "Should be able to send message");
  }

  #[tokio::test]
  async fn test_receiver_http_message_flow() {
    ensure_python_initialized();

    let (_receiver, tx) = Receiver::http();

    // Send a message and verify it succeeds
    let message = HttpReceiveMessage::Request {
      body: b"test body".to_vec(),
      more_body: false,
    };
    let result = tx.send(message);
    assert!(
      result.is_ok(),
      "Should be able to send message through channel"
    );
  }

  #[tokio::test]
  async fn test_receiver_websocket_message_flow() {
    ensure_python_initialized();

    let (_receiver, tx) = Receiver::websocket();

    // Send a message and verify it succeeds
    let message = WebSocketReceiveMessage::Receive {
      bytes: None,
      text: Some("test message".to_string()),
    };
    let result = tx.send(message);
    assert!(
      result.is_ok(),
      "Should be able to send message through channel"
    );
  }

  #[tokio::test]
  async fn test_receiver_lifespan_message_flow() {
    ensure_python_initialized();

    let (_receiver, tx) = Receiver::lifespan();

    // Send multiple messages and verify they succeed
    let result1 = tx.send(LifespanReceiveMessage::LifespanStartup);
    assert!(result1.is_ok(), "Should be able to send startup message");

    let result2 = tx.send(LifespanReceiveMessage::LifespanShutdown);
    assert!(result2.is_ok(), "Should be able to send shutdown message");
  }
}
