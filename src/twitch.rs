use std::sync::LazyLock;

use anyhow::{Context, Ok, Result, bail};
use chrono::Utc;
use m3u8_rs::{MasterPlaylist, MediaPlaylist};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::instrument;

pub static TWITCH_VIDEO_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https:?//(?:www\.)twitch.tv/videos/(\d+)").unwrap());

// https://github.com/SuperSonicHub1/twitch-graphql-api#getting-your-client-id
pub const TWITCH_PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub title: String,
    #[serde(rename = "thumbnailURLs")]
    pub thumbnail_url: Vec<String>,
    pub created_at: chrono::DateTime<Utc>,
    pub length_seconds: u64,
    pub owner: Channel,
    pub view_count: u64,
    pub game: Game,
    pub description: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub id: String,
    pub display_name: String,
    #[serde(rename = "boxArtURL")]
    pub box_art_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub display_name: String,
    pub login: String,
}

#[instrument(skip(client))]
pub async fn get_vod_info(client: reqwest::Client, video_id: u64) -> Result<Option<VideoInfo>> {
    let req = client
        .post("https://gql.twitch.tv/gql")
        .json(&json!({
            "query": format!("{{
                video(id: \"{video_id}\") {{
                    title
                    thumbnailURLs(height: 1080, width: 1920)
                    createdAt
                    lengthSeconds
                    owner {{
                        id
                        displayName
                        login
                    }}
                    viewCount
                    game {{
                        id
                        displayName
                        boxArtURL
                    }}
                    description
                    status
                }}
            }}"),
            "variables": {}
        }))
        .send()
        .await
        .context("Fetching VOD info")
        .unwrap();

    let mut json = req
        .json::<Value>()
        .await
        .context("Parsing VOD info request")
        .unwrap();

    Ok(
        serde_json::from_value::<Option<VideoInfo>>(json["data"]["video"].take())
            .context("Parsing VOD info data")?,
    )
}

/// Fetches the access tokens used to access a VOD's m3u8 master playlist file
///
/// Returns: `(token_value, token_signature)`
#[instrument(skip(client, oauth_token))]
pub async fn get_vod_tokens(
    client: reqwest::Client,
    oauth_token: Option<&str>,
    video_id: u64,
) -> Result<(String, String)> {
    let mut req = client.post("https://gql.twitch.tv/gql");

    if let Some(token) = oauth_token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let req = req.json(&json!({
            "query": "query PlaybackAccessToken_Template($login: String!, $isLive: Boolean!, $vodID: ID!, $isVod: Boolean!, $playerType: String!) {
                streamPlaybackAccessToken(
                    channelName: $login
                    params: {platform: \"web\", playerBackend: \"mediaplayer\", playerType: $playerType}
                ) @include(if: $isLive) {
                    value
                    signature
                    __typename
                }
                videoPlaybackAccessToken(
                    id: $vodID
                    params: {platform: \"web\", playerBackend: \"mediaplayer\", playerType: $playerType}
                ) @include(if: $isVod) {
                    value
                    signature
                    __typename
                }
            }",
            "variables": {
                "isLive": false,
                "login": "",
                "isVod": true,
                "vodID": video_id.to_string(),
                "playerType": "embed"
            }
        }))
        .send()
        .await
        .unwrap();

    let res = req.json::<Value>().await.unwrap();
    let Some(token) = res["data"]["videoPlaybackAccessToken"].as_object() else {
        bail!("`videoPlaybackAccessToken` does not exist. The VOD might be private!")
    };

    Ok((
        token["value"].as_str().unwrap().to_string(),
        token["signature"].as_str().unwrap().to_string(),
    ))
}

/// Fetches the stream's master .m3u8 file
///
/// You should call `get_twitch_playback_token` to get the signature values
#[instrument(skip(client, token_value, token_signature))]
pub async fn get_vod_playlist_file(
    client: reqwest::Client,
    video_id: u64,
    token_value: &str,
    token_signature: &str,
) -> Result<MasterPlaylist> {
    let req = client
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
    println!("{body}");

    let (_, playlist) = m3u8_rs::parse_master_playlist(body.as_bytes()).unwrap();

    Ok(playlist)
}

/// Fetches a stream variant media
#[instrument(skip(client, uri))]
pub async fn get_vod_media(
    client: reqwest::Client,
    uri: impl reqwest::IntoUrl,
) -> Result<MediaPlaylist> {
    let req = client
        .get(uri)
        .send()
        .await
        .context("Fetching VOD media file")?;
    let body = req.text().await.context("Decoding VOD media file")?;

    let (_, playlist) = m3u8_rs::parse_media_playlist(body.as_bytes()).unwrap();

    Ok(playlist)
}
