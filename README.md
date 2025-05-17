[![wakatime](https://wakatime.com/badge/github/angeloanan/vod-squirrel.svg)](https://wakatime.com/badge/github/angeloanan/vod-squirrel)
# VOD Squirrel

Archive your favorite Twitch.TV streams to YouTube!

See the [motivation of the project](#motivation) below.

## Usage

### Prerequisites

You will need to get a Google OAuth token with `https://www.googleapis.com/auth/youtube.upload` scope.

The easiest way of doing it is to utilize the [Google OAuth 2.0 Playground](https://developers.google.com/oauthplayground)

<details>
<summary>Click to see detailed instructions</summary>

1. Visit the [Google OAuth 2.0 Playground](https://developers.google.com/oauthplayground)
2. On the left navbar, select the `YouTube Data API v3` -> `https://www.googleapis.com/auth/youtube.upload` scope
3. Click on **Authorize APIs**
4. Login / select your Google account and allow the app to access your YouTube account
5. Once redirected back to the playground, click on **"Exchange authorization code for tokens"** button
6. Copy the `Access token` string. (You might need renavigate to the `Step 2` tab to see the access token)

</details>

Once you have an **access token**, set it as the environment variable `OAUTH_TOKEN`.

### Running the App

Download the latest release from the [releases page](https://github.com/angeloanan/vod-squirrel/releases) or development build via [commit workflows](https://github.com/angeloanan/vod-squirrel/actions/workflows/dev.yml).

If you are on Mac / Linux, allow the app to be run by `chmod +x vod-squirrel`.

Run the app with the VOD ID / URL you want to archive as an argument

```sh
$ ./vod-squirrel https://twitch.tv/videos/123456789
```

> [!IMPORTANT]
> Archiving a long video might fail due to error `Too many files open`. You can fix this by increasing your system's `ulimit` for the maximum number of open files (`ulimit -n 10240`).
> 
> You might want to check the OS' global maximum number of open files before setting the `ulimit` value above (`cat /proc/sys/fs/file-max`).

You can use the `--help` flag to get a list of all available options:

```sh
$ ./vod-squirrel --help
Downloads a Twitch.tv Video (VOD) and uploads it to YouTube for archival purposes

Usage: vod-squirrel [OPTIONS] <VOD>

Arguments:
  <VOD>  Twitch video ID / URL to process

Options:
  -c, --cleanup                    Cleanups the remnant of the clips afterward [default: true]
  -p, --parallelism <PARALLELISM>  The amount of parallel downloads [default: 20]
      --temp-dir <TEMP_DIR>        Directory where videos are processed (defaults to system's temporary directory)
  -h, --help                       Print help
  -V, --version                    Print version
```

### Monitor Mode

Work in progress.

## Building

This project uses [Rust](https://www.rust-lang.org/) and [Cargo](https://doc.rust-lang.org/cargo/).

You do not need to have OpenSSL installed to build the project as the project uses the [rustls](https://github.com/rustls/rustls) crate to provide TLS support.

To build the project, clone the repository and run `cargo build --release`.

```bash
git clone https://github.com/angeloanan/vod-squirrel.git
cd vod-squirrel
cargo build --release
```

## Motivation

* Twitch VOD expires after 60 days (or lesser for non-partners!)
* [Twitch is implementing a 100 hours storage limit on highlights & upload](https://gamerant.com/twitch-100-hour-storage-limit-highlights-uploads-video-on-demand-change/)
* Twitch has slow CDN from where I live
* Manually downloading & uploading VODs is annoying
* VOD channels usually take some time before they upload a new VOD
