use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use tracing::instrument;

use crate::twitch::{AUTHENTICATED_PUBLIC_HTTP_CLIENT, structs::VideoInfo};

/// Returns channel's past broadcast
///
/// Returns `None` if channel is not found
///
/// # Errors
/// Errors when there's a network error or when JSON response is invalid
#[instrument]
pub async fn list_channel_videos(channel_id: u64) -> Result<Option<Vec<VideoInfo>>> {
    let req = AUTHENTICATED_PUBLIC_HTTP_CLIENT
        .post("https://gql.twitch.tv/gql")
        .json(&json!({
            "query": "query LatestChannelVideo($id: ID, $type: BroadcastType = ARCHIVE, $limit: Int = 10) {
                user(id: $id) {
                    videos(first: $limit, type: $type) {
                        edges {
                            node {
                                id
                                title
                                description
                                createdAt
                                lengthSeconds
                                viewCount
                                status
                                game { displayName }
                                owner { login, displayName }
                            }
                        }
                    }
                }
            }",
            "variables": {
                "id": channel_id.to_string()
            }
        }))
        .send()
        .await
        .context("Fetching channel videos")?;

    ensure!(req.status().is_success(), "Failed to get channel videos");

    let mut json = req
        .json::<Value>()
        .await
        .context("Parsing VOD info request")?;

    let mut user = json["data"]["user"].take();
    if user.is_null() {
        return Ok(None);
    }

    let mut edges = user["videos"]["edges"].take();
    let videos = edges
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .map(|e| serde_json::from_value(e["node"].take()).unwrap())
        .collect::<Vec<VideoInfo>>();

    Ok(Some(videos))
}

#[instrument]
pub async fn get_video_info(video_id: u64) -> Result<Option<VideoInfo>> {
    let req = AUTHENTICATED_PUBLIC_HTTP_CLIENT
        .post("https://gql.twitch.tv/gql")
        .json(&json!({
            "query": "query VideoInfo($id: ID) {
                video(id: $id) {
                    id
                    title
                    description
                    createdAt
                    lengthSeconds
                    viewCount
                    status
                    game { displayName }
                    owner { login, displayName }
                }
            }",
            "variables": {
                "id": video_id.to_string()
            }
        }))
        .send()
        .await
        .context("Fetching VOD info")?;

    let mut json = req
        .json::<Value>()
        .await
        .context("Parsing VOD info request")?;

    serde_json::from_value::<Option<VideoInfo>>(json["data"]["video"].take())
        .context("Parsing VOD info data")
}

//

/// Fetches the access tokens used to access a VOD's m3u8 master playlist file
///
/// Returns: `(token_value, token_signature)`
#[instrument(skip(oauth_token))]
pub async fn get_video_cdn_tokens(
    video_id: u64,

    // Optional OAuth token for private videos
    oauth_token: Option<&str>,
) -> Result<(String, String)> {
    let mut req = AUTHENTICATED_PUBLIC_HTTP_CLIENT.post("https://gql.twitch.tv/gql");

    if let Some(token) = oauth_token {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let req = req
        .json(&json!({
            "query": "query GetPlaybackAccessToken($id: ID!) {
                videoPlaybackAccessToken(
                    id: $id
                    params: {platform: \"web\", playerBackend: \"mediaplayer\", playerType: \"embed\"}
                ) {
                    value
                    signature
                    __typename
                }
            }",
            "variables": {
                "id": video_id.to_string(),
            }
        }))
        .send()
        .await.context("Fetching VOD tokens")?;

    let res = req.json::<Value>().await.context("Parsing VOD tokens")?;
    let Some(token) = res["data"]["videoPlaybackAccessToken"].as_object() else {
        bail!("`videoPlaybackAccessToken` does not exist. The VOD might be private!")
    };

    Ok((
        token["value"].as_str().unwrap().to_string(),
        token["signature"].as_str().unwrap().to_string(),
    ))
}
