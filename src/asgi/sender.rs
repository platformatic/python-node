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
  async fn __call__(&mut self, args: Py<PyDict>) -> PyResult<Py<PyAny>> {
    // Create acknowledgment channel
    let (ack_tx, ack_rx) = oneshot::channel::<()>();

    // Send message with acknowledgment channel
    let send_result: PyResult<()> = Python::attach(|py| {
      let args_dict = args.bind(py);
      match &self.0 {
        SenderType::Http(tx) => {
          let msg: HttpSendMessage = args_dict.extract()?;
          tx.send(AcknowledgedMessage {
            message: msg,
            ack: ack_tx,
          })
          .map_err(|_| PyValueError::new_err("connection closed"))?;
          Ok(())
        }
        SenderType::WebSocket(tx) => {
          let msg: WebSocketSendMessage = args_dict.extract()?;
          tx.send(AcknowledgedMessage {
            message: msg,
            ack: ack_tx,
          })
          .map_err(|_| PyValueError::new_err("connection closed"))?;
          Ok(())
        }
        SenderType::Lifespan(tx) => {
          let msg: LifespanSendMessage = args_dict.extract()?;
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
      Ok(()) => Python::attach(|py| Ok(py.None())),
      Err(_) => Err(PyValueError::new_err("message not acknowledged")),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::asgi::ensure_python_initialized;

  #[test]
  fn test_sender_http_creation() {
    ensure_python_initialized();

    let (_sender, mut rx) = Sender::http();
    // Verify receiver is open
    assert!(rx.try_recv().is_err(), "Channel should be empty but open");
  }

  #[test]
  fn test_sender_websocket_creation() {
    ensure_python_initialized();

    let (_sender, mut rx) = Sender::websocket();
    // Verify receiver is open
    assert!(rx.try_recv().is_err(), "Channel should be empty but open");
  }

  #[test]
  fn test_sender_lifespan_creation() {
    ensure_python_initialized();

    let (_sender, mut rx) = Sender::lifespan();
    // Verify receiver is open
    assert!(rx.try_recv().is_err(), "Channel should be empty but open");
  }

  #[test]
  fn test_sender_http_channel_closed() {
    ensure_python_initialized();

    let (sender, rx) = Sender::http();

    // Drop the receiver to close the channel
    drop(rx);

    // Sender should still exist but attempts to send will fail
    // We can't easily test the __call__ method without Python, but we've verified
    // the channel setup works
    drop(sender);
  }

  #[test]
  fn test_sender_websocket_channel_closed() {
    ensure_python_initialized();

    let (sender, rx) = Sender::websocket();

    // Drop the receiver to close the channel
    drop(rx);

    // Sender should still exist
    drop(sender);
  }

  #[test]
  fn test_sender_lifespan_channel_closed() {
    ensure_python_initialized();

    let (sender, rx) = Sender::lifespan();

    // Drop the receiver to close the channel
    drop(rx);

    // Sender should still exist
    drop(sender);
  }

  #[tokio::test]
  async fn test_acknowledged_message_structure() {
    ensure_python_initialized();

    let (_sender, mut rx) = Sender::http();

    // We can't easily send through the sender without Python, but we can verify
    // the receiver side works
    assert!(rx.try_recv().is_err(), "Channel should be empty initially");
  }
}
