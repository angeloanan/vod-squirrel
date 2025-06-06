use anyhow::{Context, Result, bail};
use reqwest::{Body, Client, header::AUTHORIZATION};
use serde_json::json;
use tokio::{fs::File, select};
use tokio_util::{io::ReaderStream, sync::CancellationToken};
use tracing::{error, instrument, warn};

#[derive(Debug, Default, Clone)]
pub struct VideoDetail<'a> {
    pub title: &'a str,
    pub description: &'a str,
}

#[instrument(skip(ct, client, oauth_token, file, video_detail))]
pub async fn upload_video<'a>(
    ct: CancellationToken,
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
        .context("Initializing upload")?;

    if !init_upload_req.status().is_success() {
        error!(
            "Unable to initialize YouTube upload. Status {}",
            init_upload_req.status()
        );
        let body = init_upload_req.text().await.unwrap();
        error!(body);
        bail!("Unable to initialize YouTube upload");
    }

    let upload_url = init_upload_req
        .headers()
        .get("Location")
        .unwrap()
        .to_str()
        .unwrap();

    let pb = indicatif::ProgressBar::new(file_size);
    pb.set_style(
        indicatif::ProgressStyle::with_template(
            "[{elapsed_precise}] [{wide_bar}] {bytes}/{total_bytes} ({decimal_bytes_per_sec} {eta} left)",
        )
        .unwrap(),
    );

    let file = pb.wrap_async_read(file);

    select! {
        () = ct.cancelled() => warn!("Cancellation token caught in the middle of a video upload! Video is not fully uploaded to YouTube!"),

        req = client.put(upload_url).header(AUTHORIZATION, format!("Bearer {oauth_token}")).body(Body::wrap_stream(ReaderStream::new(file))).send() => {
            pb.finish_and_clear();
            let req = req.unwrap();
            println!("{}, {}", req.status(), req.text().await.unwrap());
        }
    }
    return Ok(());
}
