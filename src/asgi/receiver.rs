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
          Python::with_gil(|py| Ok(msg.into_pyobject(py)?.unbind()))
        } else {
          Err(PyValueError::new_err("No message received"))
        }
      }
      ReceiverType::WebSocket(rx) => {
        let message = rx.lock().await.recv().await;
        if let Some(msg) = message {
          Python::with_gil(|py| Ok(msg.into_pyobject(py)?.unbind()))
        } else {
          Err(PyValueError::new_err("No message received"))
        }
      }
      ReceiverType::Lifespan(rx) => {
        let message = rx.lock().await.recv().await;
        if let Some(msg) = message {
          Python::with_gil(|py| Ok(msg.into_pyobject(py)?.unbind()))
        } else {
          Err(PyValueError::new_err("No message received"))
        }
      }
    }
  }
}
