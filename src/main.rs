#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![warn(clippy::cargo)]
#![warn(clippy::perf)]
#![warn(clippy::complexity)]
#![warn(clippy::style)]
#![forbid(unsafe_code)]
#![allow(clippy::multiple_crate_versions, clippy::missing_panics_doc)]

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use ffmpeg::concat_video;
use reqwest::Url;
use tokio::{fs::File, io::AsyncWriteExt, select, sync::Semaphore};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use twitch::{
    VideoInfo, extract_video_id, get_vod_info, get_vod_media, get_vod_playlist_file,
    get_vod_tokens, list_channel_videos,
};
use util::{truncate_string, warn_ulimit};
use youtube::{VideoDetail, upload_video};

pub mod eventsub;
pub mod ffmpeg;
pub mod google;
pub mod oauth_server;
pub mod twitch;
pub mod util;
pub mod youtube;

/// Archives a Twitch.tv Video (VOD) by uploading it to YouTube
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// The amount of parallel downloads to be done at once
    #[arg(short, long, default_value_t = 20, value_name = "COUNT")]
    parallelism: usize,

    /// Cleanup the unprocessed video chunks afterward [default: true]
    #[arg(short, long, default_value_t = true)]
    cleanup: bool,

    /// Directory where videos are processed (defaults to system's temporary directory)
    #[arg(long, value_name = "DIR")]
    temp_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Login and authorize a channel to archive to
    Login,

    /// Download and save a Twitch video
    Download {
        /// Twitch video URL / ID to Download
        #[arg(value_name = "VOD_URL")]
        vod: String,

        /// The file name / path of the video output
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },

    /// Download and archive a Twitch video to YouTube
    Archive {
        /// Twitch video URL / ID to Download
        #[arg(value_name = "VOD_URL")]
        vod: String,
    },

    /// Automatically monitors a channel for VODs and archives new ones
    Monitor {
        /// Twitch Channel(s) ID to monitor
        channel_id: Vec<u64>,
    },
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    dotenvy::dotenv().ok();

    warn_ulimit();

    let args = Args::parse();

    let client = util::init_http_client();
    let ct = CancellationToken::new();
    util::spawn_ct_watcher(ct.clone());

    let temp_download_dir = args.temp_dir.map_or_else(std::env::temp_dir, |p| {
        if p.is_dir() {
            return p;
        }
        panic!("Provided temporary directory is not a valid directory!");
    });

    match args.command {
        Commands::Login => {
            let (verifier, url) = google::generate_login_url();
            info!("Please login and authorize a channel to upload to: {url}");

            info!("Waiting for redirect...");
            let code = oauth_server::wait_google_redirect().await.unwrap();
            info!("Redirect caught! Generating tokens...");
            let (refresh_token, _access_token) =
                google::exchange_auth_code(client, &verifier, &code)
                    .await
                    .unwrap();

            info!(
                "Authorization complete! Please put the following string as REFRESH_TOKEN: {refresh_token}"
            );
            if !ffmpeg::is_installed().await {
                warn!(
                    "`ffmpeg` is not installed - Please install it before downloading / archiving / monitoring using this tool!"
                );
            }
        }

        Commands::Download { vod, path } => {
            assert!(
                (ffmpeg::is_installed().await),
                "Unable to continue because `ffmpeg` is not installed!"
            );

            // Try creating a file on the path
            // Wont prevent TOCTOU but it'll ease the checking of valid filename
            if path.exists() {
                panic!(
                    "File with the same name / path exists! Please delete or rename said file before continuing"
                )
            }
            tokio::fs::File::create_new(path.clone())
                .await
                .expect("Unable to create file on the given path!");
            tokio::fs::remove_file(path.clone()).await.unwrap();

            let vod_id = extract_video_id(&vod)
                .expect("Unable to extract for video ID. Did you paste in the correct URL / ID?");

            let _video_info = get_and_print_video_info(client.clone(), vod_id)
                .await
                .expect("Unable to fetch for Twitch Video. Is the video public?");

            let file = download(
                ct.clone(),
                client,
                &temp_download_dir,
                args.parallelism,
                vod_id,
            )
            .await
            .unwrap();

            if ct.is_cancelled() {
                info!("CTRL + C caught! Quitting early...");
                return Ok(());
            }

            tokio::fs::rename(&file, path)
                .await
                .expect("Unable to move file!");

            if args.cleanup {
                info!("Cleaning up processing remnants");
                tokio::fs::remove_dir_all(file.parent().unwrap())
                    .await
                    .unwrap();
            }
        }

        Commands::Archive { vod } => {
            assert!(
                (ffmpeg::is_installed().await),
                "Unable to continue because `ffmpeg` is not installed!"
            );
            let access_token = google::watch_access_token(ct.clone());
            let vod_id = extract_video_id(&vod)
                .expect("Unable to extract for video ID. Did you paste in the correct URL / ID?");

            let video_info = get_and_print_video_info(client.clone(), vod_id)
                .await
                .expect("Unable to fetch for Twitch Video. Is the video public?");

            let file_path = download(
                ct.clone(),
                client.clone(),
                &temp_download_dir,
                args.parallelism,
                vod_id,
            )
            .await
            .unwrap();

            if ct.is_cancelled() {
                info!("CTRL + C caught! Quitting early...");
                return Ok(());
            }

            info!("Final file path: {file_path:?}");

            let latest_access_token = &access_token.borrow().clone().unwrap();
            archive(
                ct.clone(),
                client.clone(),
                &file_path,
                video_info,
                latest_access_token,
            )
            .await
            .unwrap();

            if args.cleanup {
                info!("Cleaning up processing remnants");
                tokio::fs::remove_dir_all(file_path.parent().unwrap())
                    .await
                    .unwrap();
            }
        }

        Commands::Monitor { channel_id } => {
            assert!(
                (ffmpeg::is_installed().await),
                "Unable to continue because `ffmpeg` is not installed!"
            );
            let access_token = google::watch_access_token(ct.clone());

            let mut receiver = eventsub::listen_for_offline(ct.clone(), channel_id)
                .await
                .context("Initiating EventSub connection")
                .unwrap();

            while let Some(uid) = receiver.recv().await {
                info!("User {uid} finished streaming!");

                let temp_download_dir = temp_download_dir.clone();

                let vids = list_channel_videos(client.clone(), uid)
                    .await
                    .unwrap()
                    .unwrap();
                let latest_vod = vids.first().cloned().unwrap();
                let vod_id = latest_vod.id.parse::<u64>().unwrap();

                let file_path = download(
                    ct.clone(),
                    client.clone(),
                    &temp_download_dir,
                    args.parallelism,
                    vod_id,
                )
                .await
                .unwrap();

                info!("Final file path: {file_path:?}");

                if ct.is_cancelled() {
                    info!("CTRL + C caught! Quitting early...");
                    return Ok(());
                }

                let latest_access_token = &access_token.borrow().clone().unwrap();
                archive(
                    ct.clone(),
                    client.clone(),
                    &file_path,
                    latest_vod,
                    latest_access_token,
                )
                .await
                .unwrap();

                if args.cleanup {
                    info!("Cleaning up processing remnants");
                    tokio::fs::remove_dir_all(file_path.parent().unwrap())
                        .await
                        .unwrap();
                }
            }
        }
    }

    info!("All done!");

    Ok(())
}

async fn get_and_print_video_info(client: reqwest::Client, vod_id: u64) -> Result<VideoInfo> {
    info!("Downloading Twitch Video ID: {vod_id}");

    let vod_info = get_vod_info(client.clone(), vod_id).await?;
    let Some(vod_info) = vod_info else {
        panic!("VOD is innacessible!");
    };

    info!("Video title: {}", vod_info.title);
    info!(
        "Video author: {} ({})",
        vod_info.owner.display_name, vod_info.owner.login
    );
    info!("Video date: {}", vod_info.created_at);

    Ok(vod_info)
}

async fn download(
    ct: CancellationToken,
    client: reqwest::Client,
    temp_download_dir: &Path,
    parallelism: usize,
    vod_id: u64,
) -> Result<PathBuf> {
    // Get CDN access tokens
    let (token_value, token_signature) =
        get_vod_tokens(client.clone(), None, vod_id).await.unwrap();

    // Get VOD HLS master playlist file
    let vod_playlist =
        get_vod_playlist_file(client.clone(), vod_id, &token_value, &token_signature).await?;

    // TODO: Ensure that this is actually the highest quality variant of VOD
    let highest_quality = vod_playlist.variants.first().unwrap();
    info!("Highest quality media uri: {}", highest_quality.uri);

    // Get VOD media playlist file
    let media = get_vod_media(client.clone(), &highest_quality.uri)
        .await
        .context("Getting VOD media")?;
    let segment_count = media.segments.len();
    info!("Found {segment_count} segments to download!");

    let temp_download_dir = temp_download_dir.join(format!("vod-squirrel-{vod_id}/"));
    match tokio::fs::create_dir(&temp_download_dir).await {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            warn!("Folder already exists. Was there an uncompleted download?");
        }
        Err(e) => bail!(e),
    }

    info!("Downloading on {temp_download_dir:?} with {parallelism} parallellism",);

    let download_parallelism = Arc::new(Semaphore::new(parallelism));
    let mut download_tasks = tokio::task::JoinSet::new();
    let mut segment_file_names = Vec::new();
    let pb = indicatif::ProgressBar::new(segment_count as u64);

    // Start queueing for downloads
    for segment in media.segments {
        // Pre-add to segment_file_names
        // This is not a simple 1..length as stream name might contain `-muted` for silenced chunks
        segment_file_names.push(segment.uri.clone());

        let ct = ct.clone();
        let pb = pb.clone();
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
                file.write_all(&data)
                    .await
                    .context("Writing video data to disk")
                    .unwrap();
            }

            debug!("Done downloading {}!", segment.uri);
            pb.inc(1);
        });
    }

    download_tasks.join_all().await;
    pb.finish_and_clear();
    info!("Done downloading all chunks!");

    info!("Concatenating video chunks now");
    let out_file_path = temp_download_dir.join("out.mp4");
    concat_video(&temp_download_dir, segment_file_names, &out_file_path).await?;
    info!("Successfully concatenated video!");

    Ok(out_file_path)
}

async fn archive(
    ct: CancellationToken,
    client: reqwest::Client,
    video_path: &Path,
    video_info: VideoInfo,
    access_token: &str,
) -> Result<()> {
    let final_file = File::open(video_path).await.unwrap();

    info!("Uploading video to Youtube...");
    upload_video(
        ct.clone(),
        client,
        access_token,
        VideoDetail {
            title: &format!(
                "[{}] {}",
                video_info.created_at.date_naive(),
                // Twitch stream title could be up to 140 chars
                // YouTube title could only go up to 100 chars
                truncate_string(&video_info.title, 85)
            ),
            description: &indoc::formatdoc!(
                "Original stream title: {}
                Streamed {} @ https://twitch.tv/{}
                Game: {}",
                video_info.title,
                video_info.created_at,
                video_info.owner.login,
                video_info.game.display_name
            ),
        },
        final_file,
    )
    .await
    .unwrap();
    info!("Video successfully uploaded");

    Ok(())
}
