use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use rlimit::Resource;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Truncates a string to a maximum length, adding `...` to the end if it was truncated.
///
/// This function will continuously try to reduce length if string is being
/// truncated in the middle of a UTF codepoint
///
/// # Arguments
/// * `string` - The string to truncate
/// * `max_length` - The maximum length of the string
///
/// # Panics
/// Should never panic, unwrap is safe.
pub fn truncate_string(string: &impl ToString, max_length: usize) -> String {
    let string = string.to_string();
    if string.len() <= max_length {
        return string;
    }

    let mut attempted_len = max_length;
    let mut truncated = string.get(..attempted_len - 3);
    while truncated.is_none() {
        attempted_len -= 1;
        truncated = string.get(..attempted_len - 3);
    }

    // SAFETY: Should never panic due to the above
    format!("{}...", truncated.unwrap())
}

/// # Panics
pub fn warn_ulimit() {
    let (limit, _) = rlimit::getrlimit(Resource::NOFILE).unwrap();
    if limit <= 2048 {
        warn!(
            "Your file limit is very low which may introduce an error while processing long videos. Consider raising your file limit via `ulimit -n 10240`"
        );
    }
}

#[must_use]
pub fn init_http_client() -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        "User-Agent",
        HeaderValue::from_str(&format!(
            "{}/{} (+{})",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_REPOSITORY")
        ))
        .unwrap(),
    );

    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("Unable to build HTTP client")
}

/// Spawn a task that watches for CTRL + C signal and cancels a [`CancellationToken`] when caught
pub fn spawn_ct_watcher(ct: CancellationToken) {
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Caught CTRL+C signal!");
        ct.cancel();
    });
}
