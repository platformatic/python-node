//! Python Future polling implementation for Rust async integration.
//!
//! This module provides [`PythonFuturePoller`], a type that implements the Rust
//! [`Future`] trait by polling a Python `concurrent.futures.Future` object.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use pyo3::prelude::*;

/// Future that polls a Python concurrent.futures.Future for completion.
///
/// This future polls a Python concurrent.futures.Future and returns a Result
/// containing either the success value or the exception when the future completes.
pub struct PythonFuturePoller(Py<PyAny>);

impl PythonFuturePoller {
  /// Create a new `PythonFuturePoller` from a Python future object.
  ///
  /// # Arguments
  ///
  /// * `future` - A Python `concurrent.futures.Future` or `asyncio.Future` object
  pub fn new(future: Py<PyAny>) -> Self {
    Self(future)
  }
}

impl Future for PythonFuturePoller {
  type Output = Result<Py<PyAny>, PyErr>;

  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    Python::attach(|py| {
      let future_bound = self.0.bind(py);

      // First check if future is done
      let is_done: bool = future_bound
        .call_method0("done")
        .ok()
        .and_then(|result| result.extract().ok())
        .unwrap_or(false);

      if is_done {
        // Future is done - get the result (Ok for success, Err for exception)
        Poll::Ready(match future_bound.call_method0("result") {
          Ok(value) => Ok(value.unbind()),
          Err(err) => Err(err),
        })
      } else {
        // Not done yet, wake the task to poll again
        cx.waker().wake_by_ref();
        Poll::Pending
      }
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::asgi::ensure_python_initialized;

  /// Ensure Python is initialized for tests (only once)
  fn ensure_test_python() {
    ensure_python_initialized();
  }

  #[test]
  fn test_python_future_poller_creation() {
    ensure_test_python();

    Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let loop_ = asyncio.call_method0("new_event_loop").unwrap();
      let future = loop_.call_method0("create_future").unwrap();

      let _poller = PythonFuturePoller::new(future.unbind());
      // Just verify we can create it
    });
  }

  #[tokio::test]
  async fn test_python_future_poller_with_completed_future() {
    ensure_test_python();

    let future = Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let loop_ = asyncio.call_method0("new_event_loop").unwrap();
      let future = loop_.call_method0("create_future").unwrap();

      // Immediately complete the future with a result
      future.call_method1("set_result", (42,)).unwrap();

      future.unbind()
    });

    let poller = PythonFuturePoller::new(future);
    let result = poller.await;

    assert!(result.is_ok());
    Python::attach(|py| {
      let value: i32 = result.unwrap().extract(py).unwrap();
      assert_eq!(value, 42);
    });
  }

  #[tokio::test]
  async fn test_python_future_poller_with_exception() {
    ensure_test_python();

    let future = Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let loop_ = asyncio.call_method0("new_event_loop").unwrap();
      let future = loop_.call_method0("create_future").unwrap();

      // Set an exception on the future
      let exception = py
        .import("builtins")
        .unwrap()
        .getattr("ValueError")
        .unwrap()
        .call1(("test error",))
        .unwrap();
      future.call_method1("set_exception", (exception,)).unwrap();

      future.unbind()
    });

    let poller = PythonFuturePoller::new(future);
    let result = poller.await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_str = format!("{:?}", err);
    assert!(
      err_str.contains("ValueError") || err_str.contains("test error"),
      "Expected ValueError with 'test error', got: {}",
      err_str
    );
  }

  #[tokio::test]
  async fn test_python_future_poller_with_string_result() {
    ensure_test_python();

    let future = Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let loop_ = asyncio.call_method0("new_event_loop").unwrap();
      let future = loop_.call_method0("create_future").unwrap();

      // Complete with a string
      future.call_method1("set_result", ("hello world",)).unwrap();

      future.unbind()
    });

    let poller = PythonFuturePoller::new(future);
    let result = poller.await;

    assert!(result.is_ok());
    Python::attach(|py| {
      let value: String = result.unwrap().extract(py).unwrap();
      assert_eq!(value, "hello world");
    });
  }

  #[tokio::test]
  async fn test_python_future_poller_with_none() {
    ensure_test_python();

    let future = Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let loop_ = asyncio.call_method0("new_event_loop").unwrap();
      let future = loop_.call_method0("create_future").unwrap();

      // Complete with None
      future.call_method1("set_result", (py.None(),)).unwrap();

      future.unbind()
    });

    let poller = PythonFuturePoller::new(future);
    let result = poller.await;

    assert!(result.is_ok());
    Python::attach(|py| {
      assert!(result.unwrap().is_none(py));
    });
  }
}
