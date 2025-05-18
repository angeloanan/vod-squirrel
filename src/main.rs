#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::cargo)]
#![warn(clippy::perf)]
#![warn(clippy::complexity)]
#![warn(clippy::style)]
#![allow(clippy::multiple_crate_versions)]

use std::{io::ErrorKind, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{Context, Result, bail};
use clap::Parser;
use ffmpeg::concat_video;
use reqwest::{
    Url,
    header::{HeaderMap, HeaderValue},
};
use tokio::{fs::File, io::AsyncWriteExt, select, sync::Semaphore};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use twitch::{
    TWITCH_PUBLIC_CLIENT_ID, TWITCH_VIDEO_ID_REGEX, get_vod_info, get_vod_media,
    get_vod_playlist_file, get_vod_tokens,
};
use util::{truncate_string, warn_ulimit};
use youtube::{VideoDetail, upload_video};

pub mod ffmpeg;
pub mod twitch;
pub mod util;
pub mod youtube;

/// Archives a Twitch.tv Video (VOD) by uploading it to YouTube
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Twitch video ID / URL to process
    vod: String,

    /// Cleanups the remnant of the clips afterward [default: true]
    #[arg(short, long, default_value_t = true)]
    cleanup: bool,

    /// The amount of parallel downloads
    #[arg(short, long, default_value_t = 20)]
    parallelism: usize,


    /// Directory where videos are processed (defaults to system's temporary directory)
    #[arg(long)]
    temp_dir: Option<PathBuf>,
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    assert!((ffmpeg::is_installed().await), "ffmpeg is not installed!");
    warn_ulimit();

    let args = Args::parse();
    let oauth_token = std::env::var("OAUTH_TOKEN").expect("env OAUTH_TOKEN not provided");

    let client = init_http_client();
    let ct = CancellationToken::new();

    spawn_ct_watcher(ct.clone());

    let vod_id = if let Ok(i) = args.vod.parse::<u64>() {
        i
    } else if let Some(i) = TWITCH_VIDEO_ID_REGEX.captures(&args.vod) {
        let id = i.get(1).unwrap();
        id.as_str().parse::<u64>().unwrap()
    } else {
        panic!("Unable to parse VOD ID");
    };

    info!("Archiving VOD ID: {vod_id}");
    let vod_info = get_vod_info(client.clone(), vod_id).await?;
    let Some(vod_info) = vod_info else {
        panic!("VOD is innacessible!");
    };
    info!("VOD title: {}", vod_info.title);
    info!(
        "VOD author: {} ({})",
        vod_info.owner.display_name, vod_info.owner.login
    );
    info!("VOD date: {}", vod_info.created_at);

    // Get CDN access tokens
    let (token_value, token_signature) =
        get_vod_tokens(client.clone(), None, vod_id).await.unwrap();

    // Get VOD HLS master playlist file
    let vod_playlist =
        get_vod_playlist_file(client.clone(), vod_id, &token_value, &token_signature).await?;

    // TODO: Ensure that this is actually the highest quality variant of VOD
    let highest_quality = vod_playlist.variants.first().unwrap();
    info!("Highest quality media uri: {}", highest_quality.uri);
    let media = get_vod_media(client.clone(), &highest_quality.uri)
        .await
        .context("Getting VOD media")?;
    info!("Found {} segments to download!", media.segments.len());

    let temp_download_dir = args
        .temp_dir
        .map_or_else(std::env::temp_dir, |p| {
            if p.is_dir() {
                return p;
            }
            panic!("Provided temporary directory is not a valid directory!");
        })
        .join(format!("vod-squirrel-{vod_id}/"));
    info!(
        "Downloading on {temp_download_dir:?} with {} parallellism",
        args.parallelism
    );
    match tokio::fs::create_dir(&temp_download_dir).await {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            warn!("Folder already exists. Was there an uncompleted download?");
        }
        Err(e) => bail!(e),
    }

    let download_parallelism = Arc::new(Semaphore::new(args.parallelism));
    let mut download_tasks = tokio::task::JoinSet::new();
    let mut segment_file_names = Vec::new();

    // Start queueing for downloads
    for segment in media.segments {
        // Pre-add to segment_file_names
        // This is not a simple 1..length as stream name might contain `-muted` for silenced chunks
        segment_file_names.push(segment.uri.clone());

        let ct = ct.clone();
        let permit = download_parallelism.clone();
        let client = client.clone();
        let media_url = Url::from_str(&highest_quality.uri)?.join(&segment.uri)?;
        let temp_file_path = temp_download_dir.clone().join(&segment.uri);

        download_tasks.spawn(async move {
            let _permit = select! {
                () = ct.cancelled() => return,
                p = permit.acquire() => p.unwrap()
            };

            let req = client.get(media_url).send().await.unwrap();
            let mut res_stream = req.bytes_stream();

            let mut file = File::create(temp_file_path).await.unwrap();

            while let Some(data) = res_stream.next().await {
                let data = data
                    .context(format!("Downloading video stream {}", segment.uri))
                    .unwrap();
                file.write_all(&data).await.unwrap();
            }

            debug!("Done downloading {}!", segment.uri);
        });
    }

    download_tasks.join_all().await;
    info!("Done downloading all chunks!");

    info!("Concatenating video chunks now");
    let out_file_path = temp_download_dir.join("out.mp4");
    concat_video(&temp_download_dir, segment_file_names, &out_file_path).await?;
    info!("Successfully concatenated video!");

    info!("Final file path: {out_file_path:?}");
    let final_file = File::open(out_file_path).await.unwrap();

    info!("Uploading video to Youtube...");
    upload_video(
        client,
        &oauth_token,
        VideoDetail {
            title: &format!(
                "[{}] {}",
                vod_info.created_at.date_naive(),
                truncate_string(&vod_info.title, 85) // Twitch stream title could be up to 140 chars
            ),
            description: &indoc::formatdoc!(
                "Original stream title: {}
                Streamed {} @ https://twitch.tv/{}
                Game: {}",
                vod_info.title,
                vod_info.created_at,
                vod_info.owner.login,
                vod_info.game.display_name
            ),
        },
        final_file,
    )
    .await
    .unwrap();

    info!("Video successfully uploaded");

    if args.cleanup {
        info!("Cleaning up processing remnants");
        tokio::fs::remove_dir_all(temp_download_dir).await.unwrap();
    }

    info!("All done successfully!");

    Ok(())
}

fn init_http_client() -> reqwest::Client {
    let mut headers = HeaderMap::new();
    headers.insert(
        "Client-ID",
        HeaderValue::from_static(TWITCH_PUBLIC_CLIENT_ID),
    );
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
        .build()
        .expect("Unable to build HTTP client")
}

fn spawn_ct_watcher(ct: CancellationToken) {
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Caught CTRL+C signal!");
        ct.cancel();
    });
}
