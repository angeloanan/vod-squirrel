// TODO: Swap this to use thiserror, this technically is a library
use anyhow::{Result, bail};
use regex::Regex;
use std::sync::LazyLock;

// https://github.com/SuperSonicHub1/twitch-graphql-api#getting-your-client-id
pub const AUTHENTICATED_PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
pub const AUTHENTICATED_PUBLIC_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Client-ID",
        reqwest::header::HeaderValue::from_static(AUTHENTICATED_PUBLIC_CLIENT_ID),
    );
    headers.insert(
        "User-Agent",
        reqwest::header::HeaderValue::from_str(&format!(
            "{}/{} (+{})",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_REPOSITORY")
        ))
        .unwrap(),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
});

pub const VIDEO_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https:?//(?:www\.)twitch.tv/videos/(\d+)").unwrap());

/// Extracts a video ID out from a user-inputted URL string
/// # Errors
/// Error when unable to parse twitch video ID
pub fn extract_video_id(input: &str) -> Result<u64> {
    if let Ok(i) = input.parse::<u64>() {
        return Ok(i);
    }

    if let Some(i) = VIDEO_ID_REGEX.captures(input) {
        let id = i.get(1).unwrap();
        return Ok(id.as_str().parse::<u64>()?);
    }

    bail!("Unable to parse Twitch Video URL / ID");
}
