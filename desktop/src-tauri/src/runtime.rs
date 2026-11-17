//! The one place that decides which async runtime blocking work runs on. The desktop app uses Tauri's
//! runtime (the one its commands already run on); the headless build has no Tauri, so blocking work goes
//! to the surrounding Tokio runtime (the web server's, or the stdio MCP server's).

use std::fmt;

/// A blocking task that panicked or was cancelled.
#[derive(Debug)]
pub(crate) struct JoinFailure(String);

impl fmt::Display for JoinFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Run `work` on a thread meant for blocking, without stalling the async executor.
pub(crate) async fn spawn_blocking<F, R>(work: F) -> Result<R, JoinFailure>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    #[cfg(feature = "desktop")]
    let joined = tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| JoinFailure(error.to_string()));
    #[cfg(not(feature = "desktop"))]
    let joined = tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| JoinFailure(error.to_string()));
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn blocking_work_runs_off_the_calling_task_and_returns_its_value() {
        let caller = std::thread::current().id();
        let (worker, value) = spawn_blocking(move || (std::thread::current().id(), 6 * 7))
            .await
            .expect("the task runs");
        assert_eq!(value, 42);
        assert_ne!(
            worker, caller,
            "it must not run on the async executor's thread"
        );
    }

    #[tokio::test]
    async fn a_panicking_task_is_an_error_not_a_crash() {
        let error = spawn_blocking(|| -> u8 { panic!("boom") })
            .await
            .expect_err("a panic is reported");
        assert!(!error.to_string().is_empty());
    }
}
