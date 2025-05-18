use std::{io::ErrorKind, path::Path, process::Stdio, str::FromStr};

use anyhow::{Context, Result, bail};
use tracing::{debug, error};

/// Checks if ffmpeg is installed / available in PATH
///
/// # Panics
/// Will panic if the child process cannot be spawned or if there is an error while awaiting its status.\
/// See `tokio::process::Command.status()`
pub async fn is_installed() -> bool {
    debug!("Checking for ffmpeg installation");
    tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .await
        .context("Checking if FFMPEG is installed / available in PATH")
        .unwrap()
        .success()
}

/// Concatenate videos using concat protocol\
/// See: <https://trac.ffmpeg.org/wiki/Concatenate#protocol>
///
/// # Errors
///
/// # Panics
/// Will panic if FFMPEG returned an error (e.g. not installed, encoding errors), process cannot be spawned or if there is an error while awaiting its status.\
/// See `tokio::process::Command.status()`
pub async fn concat_video(
    video_directory: &Path,
    file_names: Vec<String>,
    out_file: &Path,
) -> Result<()> {
    // TODO: Compare / consider using concatenate demuxer
    // Might fix ulimit issues as it might force ffmpeg to handle open files
    let mut input_string = String::from_str("concat:").unwrap();
    input_string.push_str(
        &file_names
            .iter()
            .map(|f| video_directory.join(f).to_str().unwrap().to_string())
            .collect::<Vec<String>>()
            .join("|"),
    );

    // TODO: Use FFMPEG's actual API for efficiency
    let child = match tokio::process::Command::new("ffmpeg")
        .args([
            "-stats",
            "-y",
            "-loglevel",
            "error",
            "-avoid_negative_ts",
            "make_zero",
            "-i",
            &input_string,
            "-c",
            "copy",
            out_file.to_str().unwrap(),
        ])
        .spawn()
    {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            bail!("`ffmpeg` is not installed or available in PATH!")
        }
        Err(e) => bail!("Unknown error: {e}"),
    };

    let out = child.wait_with_output().await.unwrap();
    if !out.status.success() {
        error!("Video concatenation is unsuccessful");
        error!("stdout: {}", String::from_utf8_lossy(&out.stdout));
        error!("stderr: {}", String::from_utf8_lossy(&out.stderr));
        bail!("FFMPEG exit code not success")
    }

    Ok(())
}
