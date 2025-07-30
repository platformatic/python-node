use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use tokio::sync::{mpsc, oneshot};

use crate::asgi::{
  http::HttpSendMessage, lifespan::LifespanSendMessage, websocket::WebSocketSendMessage,
};

/// Message wrapper that includes acknowledgment channel
pub struct AcknowledgedMessage<T> {
  pub message: T,
  pub ack: oneshot::Sender<()>,
}

enum SenderType {
  Http(mpsc::UnboundedSender<AcknowledgedMessage<HttpSendMessage>>),
  WebSocket(mpsc::UnboundedSender<AcknowledgedMessage<WebSocketSendMessage>>),
  Lifespan(mpsc::UnboundedSender<AcknowledgedMessage<LifespanSendMessage>>),
}

/// Allows Python to send messages to Rust.
#[pyclass]
pub struct Sender(SenderType);

impl Sender {
  pub fn http() -> (
    Sender,
    mpsc::UnboundedReceiver<AcknowledgedMessage<HttpSendMessage>>,
  ) {
    let (tx, rx) = mpsc::unbounded_channel::<AcknowledgedMessage<HttpSendMessage>>();
    (Sender(SenderType::Http(tx)), rx)
  }

  pub fn websocket() -> (
    Sender,
    mpsc::UnboundedReceiver<AcknowledgedMessage<WebSocketSendMessage>>,
  ) {
    let (tx, rx) = mpsc::unbounded_channel::<AcknowledgedMessage<WebSocketSendMessage>>();
    (Sender(SenderType::WebSocket(tx)), rx)
  }

  pub fn lifespan() -> (
    Sender,
    mpsc::UnboundedReceiver<AcknowledgedMessage<LifespanSendMessage>>,
  ) {
    let (tx, rx) = mpsc::unbounded_channel::<AcknowledgedMessage<LifespanSendMessage>>();
    (Sender(SenderType::Lifespan(tx)), rx)
  }
}

#[pymethods]
impl Sender {
  async fn __call__(&mut self, args: Py<PyDict>) -> PyResult<PyObject> {
    // Create acknowledgment channel
    let (ack_tx, ack_rx) = oneshot::channel::<()>();

    // Send message with acknowledgment channel
    let send_result: PyResult<()> = Python::with_gil(|py| {
      let args_dict = args.bind(py);
      match &self.0 {
        SenderType::Http(tx) => {
          let msg = HttpSendMessage::extract_bound(args_dict)?;
          tx.send(AcknowledgedMessage {
            message: msg,
            ack: ack_tx,
          })
          .map_err(|_| PyValueError::new_err("connection closed"))?;
          Ok(())
        }
        SenderType::WebSocket(tx) => {
          let msg = WebSocketSendMessage::extract_bound(args_dict)?;
          tx.send(AcknowledgedMessage {
            message: msg,
            ack: ack_tx,
          })
          .map_err(|_| PyValueError::new_err("connection closed"))?;
          Ok(())
        }
        SenderType::Lifespan(tx) => {
          let msg = LifespanSendMessage::extract_bound(args_dict)?;
          tx.send(AcknowledgedMessage {
            message: msg,
            ack: ack_tx,
          })
          .map_err(|_| PyValueError::new_err("connection closed"))?;
          Ok(())
        }
      }
    });

    // Return error if send failed
    send_result?;

    // Wait for acknowledgment
    match ack_rx.await {
      Ok(()) => Python::with_gil(|py| Ok(py.None())),
      Err(_) => Err(PyValueError::new_err("message not acknowledged")),
    }
  }
}
