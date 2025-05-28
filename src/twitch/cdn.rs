use anyhow::{Context, Result};
use m3u8_rs::{MasterPlaylist, MediaPlaylist};
use tracing::{info, instrument};

use crate::twitch::AUTHENTICATED_PUBLIC_HTTP_CLIENT;

/// Fetches the stream's master .m3u8 file
///
/// You should call `twitch::api::get_video_cdn_tokens` to get the signature values
#[instrument(skip(token_value, token_signature))]
pub async fn get_video_playlist_file(
    video_id: u64,
    token_value: &str,
    token_signature: &str,
) -> Result<MasterPlaylist> {
    let req = AUTHENTICATED_PUBLIC_HTTP_CLIENT
        .get(format!("https://usher.ttvnw.net/vod/{video_id}.m3u8"))
        .query(&[
            ("sig", token_signature),
            ("token", token_value),
            ("allow_source", "true"),
            ("allow_audio_only", "true"),
            ("platform", "web"),
            ("player_backend", "mediaplayer"),
            ("playlist_include_framerate", "true"),
            ("supported_codecs", "av1,h265,h264"),
        ])
        .send()
        .await
        .context("Fetching VOD playlist file")?;

    let body = req.text().await.context("Decoding VOD playlist file")?;

    let (_, playlist) = m3u8_rs::parse_master_playlist(body.as_bytes()).unwrap();
    info!(
        "Available VOD quality: {}",
        playlist
            .variants
            .iter()
            .map(|v| v
                .resolution
                .map_or("Unknown resolution".to_string(), |v| v.to_string()))
            .collect::<Vec<String>>()
            .join(", ")
    );

    Ok(playlist)
}

/// Fetches a stream variant media
#[instrument(skip(uri))]
pub async fn get_video_media(uri: impl reqwest::IntoUrl) -> Result<MediaPlaylist> {
    let req = AUTHENTICATED_PUBLIC_HTTP_CLIENT
        .get(uri)
        .send()
        .await
        .context("Fetching VOD media file")?;
    let body = req.text().await.context("Decoding VOD media file")?;

    let (_, playlist) = m3u8_rs::parse_media_playlist(body.as_bytes()).unwrap();

    Ok(playlist)
}
