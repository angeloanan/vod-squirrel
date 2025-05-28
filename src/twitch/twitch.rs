use std::sync::LazyLock;

use anyhow::{Context, Result, bail, ensure};
use chrono::Utc;
use m3u8_rs::{MasterPlaylist, MediaPlaylist};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::instrument;
