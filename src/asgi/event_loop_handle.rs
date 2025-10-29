//! Python event loop handle management for Rust async integration.
//!
//! This module provides [`EventLoopHandle`], a type that wraps a Python asyncio
//! event loop and ensures proper cleanup when dropped, along with factory functions
//! for creating and managing the shared event loop.

use std::sync::{Arc, Mutex, OnceLock, Weak};

use pyo3::prelude::*;

use crate::HandlerError;

/// Handle to a shared Python event loop.
///
/// This handle manages a Python asyncio event loop that runs in a background thread.
/// When the last handle is dropped, the event loop is stopped.
///
/// # Thread Safety
///
/// This type implements `Send` and `Sync` to allow sharing across threads, though
/// the actual Python event loop runs in its own dedicated thread.
pub struct EventLoopHandle {
  event_loop: Py<PyAny>,
}

impl EventLoopHandle {
  /// Create a new EventLoopHandle with a Python event loop.
  ///
  /// This constructor:
  /// 1. Ensures Python is initialized with proper symbol visibility
  /// 2. Creates a new Python asyncio event loop
  /// 3. Starts a background thread to run the event loop
  /// 4. Returns a handle to the event loop
  ///
  /// # Errors
  ///
  /// Returns `HandlerError` if:
  /// - Python initialization fails
  /// - Creating the event loop fails
  /// - Starting the event loop thread fails
  pub fn new() -> Result<Self, HandlerError> {
    // Ensure Python is initialized with proper symbol visibility
    crate::asgi::ensure_python_initialized();

    // Create event loop
    let event_loop = Python::attach(|py| -> Result<Py<PyAny>, HandlerError> {
      let asyncio = py.import("asyncio")?;
      let event_loop = asyncio.call_method0("new_event_loop")?;
      let event_loop_py = event_loop.unbind();

      // Start Python thread that just runs the event loop
      let loop_ = event_loop_py.clone_ref(py);

      // Spawn a dedicated std::thread for the Python event loop
      // We cannot use tokio's spawn_blocking because:
      // 1. It would attach to the current runtime (e.g., test runtime)
      // 2. When that runtime drops, it waits for blocking tasks to complete
      // 3. But the Python event loop runs forever, causing a deadlock
      std::thread::Builder::new()
        .name("python-event-loop".to_string())
        .spawn(move || {
          Self::loop_thread(loop_);
        })
        .expect("Failed to spawn Python event loop thread");

      Ok(event_loop_py)
    })?;

    Ok(Self::with_loop(event_loop))
  }

  /// Create an EventLoopHandle from an existing Python event loop object.
  ///
  /// # Arguments
  ///
  /// * `event_loop` - A Python `asyncio.AbstractEventLoop` object
  ///
  /// # Note
  ///
  /// The event loop should already be running in a background thread before
  /// creating this handle. This handle only manages the lifecycle, it doesn't
  /// start the event loop.
  pub fn with_loop(event_loop: Py<PyAny>) -> Self {
    Self { event_loop }
  }

  /// Get or create a shared Python event loop.
  ///
  /// This method maintains a weak reference to the shared event loop. If the event loop
  /// is still alive, it returns a strong reference to it. Otherwise, it creates a new
  /// event loop.
  ///
  /// # Thread Safety
  ///
  /// This method is thread-safe and uses a mutex to protect concurrent access to
  /// the weak reference.
  ///
  /// # Errors
  ///
  /// Returns `HandlerError` if:
  /// - The mutex is poisoned
  /// - Creating a new event loop fails
  pub fn get_or_create() -> Result<Arc<Self>, HandlerError> {
    let mut guard = PYTHON_EVENT_LOOP
      .get_or_init(|| Mutex::new(Weak::new()))
      .lock()?;

    // Try to upgrade the weak reference
    if let Some(handle) = guard.upgrade() {
      return Ok(handle);
    }

    // Create new handle
    let new_handle = Arc::new(Self::new()?);
    *guard = Arc::downgrade(&new_handle);

    Ok(new_handle)
  }

  /// Run a Python event loop forever in the current thread.
  ///
  /// This function sets the given event loop as the current event loop for the thread
  /// and runs it forever. It's intended to be called in a blocking context (e.g., from
  /// `tokio::task::spawn_blocking`).
  ///
  /// # Arguments
  ///
  /// * `event_loop` - A Python asyncio event loop object to run
  ///
  /// # Panics
  ///
  /// If the Python event loop encounters a fatal error, the error is printed to stderr
  /// but the function does not panic.
  pub fn loop_thread(event_loop: Py<PyAny>) {
    Python::attach(|py| {
      // Set the event loop for this thread and run it
      let asyncio = py.import("asyncio")?;
      asyncio.call_method1("set_event_loop", (event_loop.bind(py),))?;

      // Get the current event loop and run it forever
      asyncio
        .call_method0("get_event_loop")?
        .call_method0("run_forever")?;

      Ok::<(), PyErr>(())
    })
    .unwrap_or_else(|e| {
      eprintln!("Python event loop thread error: {e}");
    });
  }

  /// Get a reference to the Python event loop object.
  ///
  /// Returns a reference to the underlying `Py<PyAny>` that represents
  /// the Python asyncio event loop.
  pub fn event_loop(&self) -> &Py<PyAny> {
    &self.event_loop
  }
}

impl Drop for EventLoopHandle {
  fn drop(&mut self) {
    // Stop the Python event loop when the last handle is dropped
    Python::attach(|py| {
      if let Err(e) = self.event_loop.bind(py).call_method0("stop") {
        eprintln!("Failed to stop Python event loop: {e}");
      }
    });
  }
}

unsafe impl Send for EventLoopHandle {}
unsafe impl Sync for EventLoopHandle {}

/// Global Python event loop handle storage
static PYTHON_EVENT_LOOP: OnceLock<Mutex<Weak<EventLoopHandle>>> = OnceLock::new();

#[cfg(test)]
mod tests {
  use super::*;
  use crate::asgi::ensure_python_initialized;

  fn ensure_test_python() {
    ensure_python_initialized();
  }

  #[test]
  fn test_event_loop_handle_creation() {
    ensure_test_python();

    Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let event_loop = asyncio.call_method0("new_event_loop").unwrap();
      let event_loop_py = event_loop.unbind();

      let _handle = EventLoopHandle::with_loop(event_loop_py);
      // Just verify we can create it
    });
  }

  #[test]
  fn test_event_loop_handle_getter() {
    ensure_test_python();

    Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let event_loop = asyncio.call_method0("new_event_loop").unwrap();
      let event_loop_py = event_loop.unbind();
      let event_loop_clone = event_loop_py.clone_ref(py);

      let handle = EventLoopHandle::with_loop(event_loop_py);

      // Verify event_loop() returns the same event loop
      assert!(handle.event_loop().is(&event_loop_clone));
    });
  }

  #[test]
  fn test_event_loop_handle_is_running() {
    ensure_test_python();

    Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let event_loop = asyncio.call_method0("new_event_loop").unwrap();
      let event_loop_py = event_loop.unbind();

      let handle = EventLoopHandle::with_loop(event_loop_py);

      // Verify we can check if the loop is running
      let is_running: bool = handle
        .event_loop()
        .bind(py)
        .call_method0("is_running")
        .unwrap()
        .extract()
        .unwrap();

      // Should not be running since we never started it
      assert!(!is_running);
    });
  }

  #[test]
  fn test_event_loop_handle_drop_stops_loop() {
    ensure_test_python();

    let event_loop_py = Python::attach(|py| {
      let asyncio = py.import("asyncio").unwrap();
      let event_loop = asyncio.call_method0("new_event_loop").unwrap();
      event_loop.unbind()
    });

    let event_loop_clone = Python::attach(|py| event_loop_py.clone_ref(py));

    {
      let handle = EventLoopHandle::with_loop(event_loop_py);
      drop(handle); // Explicitly drop to trigger stop()
    }

    // After drop, calling stop again should be idempotent (or at least not crash)
    Python::attach(|py| {
      // The loop should still be accessible but calling stop again is fine
      let result = event_loop_clone.bind(py).call_method0("stop");
      // Either it succeeds (idempotent) or it was already stopped
      // We just verify it doesn't panic
      let _ = result;
    });
  }

  #[test]
  fn test_event_loop_handle_send_sync() {
    ensure_test_python();

    // This test verifies that EventLoopHandle implements Send and Sync
    fn is_send<T: Send>() {}
    fn is_sync<T: Sync>() {}

    is_send::<EventLoopHandle>();
    is_sync::<EventLoopHandle>();
  }
}
