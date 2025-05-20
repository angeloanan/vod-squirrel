use anyhow::{Context, Result};
use reqwest::Url;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};

/// Spawns a very hacky HTTP server at `localhost:32547` and listens an OAuth redirect from Google
///
/// # Errors
/// * If port is taken
/// * If redirect is not Google's redirect
/// * If redirect does not contain a code
/// * If redirect contains malformed HTTP headers
pub async fn wait_google_redirect() -> Result<Box<str>> {
    let socket = TcpListener::bind("127.0.0.1:32547")
        .await
        .context("Launching HTTP server on localhost:32547")?;

    let (mut stream, _addr) = socket
        .accept()
        .await
        .context("Accepting new HTTP connection")?;

    let mut reader = BufReader::new(&mut stream);
    let mut req = String::new();
    reader
        .read_line(&mut req)
        .await
        .context("Reading connection data")
        .unwrap();

    let redirect_url = req
        .split_whitespace()
        .nth(1)
        .context("Get URL data")
        .unwrap();
    let url = Url::parse(&("http://localhost".to_string() + redirect_url)).unwrap();

    let code = url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, code)| code.into_owned().into_boxed_str())
        .unwrap();

    let response_str = "Authentication complete - Go back to VOD Squirrel!";
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{response_str}",
                response_str.len()
            )
            .as_bytes(),
        )
        .await
        .ok();

    Ok(code)
}
