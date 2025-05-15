use anyhow::{Result, ensure};
use reqwest::{Body, Client, header::AUTHORIZATION};
use serde_json::json;
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use tracing::instrument;

#[derive(Debug, Default, Clone)]
pub struct VideoDetail<'a> {
    pub title: &'a str,
    pub description: &'a str,
}

#[instrument(skip(client, oauth_token, file, video_detail))]
pub async fn upload_video<'a>(
    client: Client,
    oauth_token: &str,
    video_detail: VideoDetail<'a>,
    file: File,
) -> Result<()> {
    file.sync_all().await.unwrap();
    let file_size = file.metadata().await.unwrap().len();

    let init_upload_req = client
        .post("https://www.googleapis.com/upload/youtube/v3/videos")
        .header(AUTHORIZATION, format!("Bearer {oauth_token}"))
        .header("X-Upload-Content-Length", file_size)
        .header("X-Upload-Content-Type", "video/*")
        .query(&[("uploadType", "resumable"), ("part", "snippet,status")])
        .json(&json!({
            "snippet": {
                "title": video_detail.title,
                "description": format!(
                    "{}\n\nAutomatically archived using VOD Squirrel {}: https://github.com/angeloanan/vod-squirrel",
                    video_detail.description,
                    env!("CARGO_PKG_VERSION")
                )
            },
            "status": {
                "privacyStatus": "unlisted",
                "selfDeclaredMadeForKids": false
            }
        }))
        .send()
        .await
        .unwrap();

    ensure!(
        init_upload_req.status() == 200,
        "Unable to initialize video upload"
    );
    let upload_url = init_upload_req
        .headers()
        .get("Location")
        .unwrap()
        .to_str()
        .unwrap();

    let req = client
        .put(upload_url)
        .header(AUTHORIZATION, format!("Bearer {oauth_token}"))
        .body(Body::wrap_stream(ReaderStream::new(file)))
        .send()
        .await
        .unwrap();
    println!("{}, {}", req.status(), req.text().await.unwrap());

    Ok(())
}
