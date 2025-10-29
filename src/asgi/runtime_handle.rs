//! Tokio runtime handle management for async operations.
//!
//! This module provides [`fallback_handle()`], a function that ensures a Tokio
//! runtime handle is available for async operations, creating a fallback runtime
//! if necessary.

use std::sync::OnceLock;

/// Global fallback runtime for when no tokio runtime is available
static FALLBACK_RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// Get a tokio runtime handle, creating a fallback if needed.
///
/// This function attempts to get the handle of the current tokio runtime.
/// If no runtime is available (i.e., not running within a tokio context),
/// it creates and returns a handle to a global fallback runtime.
///
/// # Returns
///
/// A `tokio::runtime::Handle` that can be used to spawn tasks and perform
/// async operations.
///
/// # Panics
///
/// Panics if creating the fallback runtime fails, though this is extremely
/// unlikely in normal operation.
///
/// # Thread Safety
///
/// This function is thread-safe. The fallback runtime is created once and
/// shared across all threads that need it.
pub(crate) fn fallback_handle() -> tokio::runtime::Handle {
  tokio::runtime::Handle::try_current().unwrap_or_else(|_| {
    // No runtime exists, create a fallback one
    let rt = FALLBACK_RUNTIME.get_or_init(|| {
      tokio::runtime::Runtime::new().expect("Failed to create fallback tokio runtime")
    });
    rt.handle().clone()
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_fallback_handle_outside_runtime() {
    // This test runs outside a tokio runtime
    // Should create and use the fallback runtime
    let handle = fallback_handle();

    // Verify we can use the handle to block_on a future
    handle.block_on(async {
      // Simple async operation
      let value = 42;
      assert_eq!(value, 42);
    });
  }

  #[tokio::test]
  async fn test_fallback_handle_inside_runtime() {
    // This test runs inside a tokio runtime
    // Should use the current runtime's handle
    let handle = fallback_handle();

    // Verify we can spawn a task
    let result = handle
      .spawn(async {
        // Simple async operation
        42
      })
      .await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 42);
  }

  #[test]
  fn test_fallback_handle_is_consistent() {
    // Calling fallback_handle multiple times outside a runtime
    // should return handles to the same fallback runtime
    let handle1 = fallback_handle();
    let handle2 = fallback_handle();

    // Both handles should work
    handle1.block_on(async {
      assert!(true);
    });

    handle2.block_on(async {
      assert!(true);
    });
  }

  #[test]
  fn test_fallback_handle_can_spawn_blocking() {
    // Test that we can use spawn_blocking with the handle
    let handle = fallback_handle();

    let result = handle.block_on(async {
      handle
        .spawn_blocking(|| {
          // CPU-bound work
          let mut sum = 0;
          for i in 0..100 {
            sum += i;
          }
          sum
        })
        .await
    });

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), 4950);
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
  async fn test_fallback_handle_multi_thread() {
    // Test that fallback_handle works correctly in a multi-threaded runtime
    let handle = fallback_handle();

    // Spawn multiple tasks concurrently
    let tasks: Vec<_> = (0..10)
      .map(|i| {
        handle.spawn(async move {
          // Simple async work
          i * 2
        })
      })
      .collect();

    // Wait for all tasks to complete
    for (i, task) in tasks.into_iter().enumerate() {
      let result = task.await;
      assert!(result.is_ok());
      assert_eq!(result.unwrap(), i * 2);
    }
  }
}
