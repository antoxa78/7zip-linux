//! Process helpers for running 7z / tar.

use tokio::io::AsyncReadExt;
use tokio::task::JoinHandle;

/// Takes the child's stderr and drains it on a background task.
///
/// When stdout is read in a loop while stderr is piped but never read, a child
/// that writes more than one pipe buffer (~64 KiB) of warnings blocks forever and
/// the UI waits forever with it. Draining stderr concurrently avoids that.
pub fn drain_stderr(child: &mut tokio::process::Child) -> JoinHandle<Vec<u8>> {
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    })
}

/// Waits for the stderr drain task and returns its text.
pub async fn collect_stderr(handle: JoinHandle<Vec<u8>>) -> String {
    match handle.await {
        Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
        Err(_) => String::new(),
    }
}
