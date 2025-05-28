use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use futures_util::StreamExt as _;
use reqwest_websocket::{Message, RequestBuilderExt as _};
use serde_json::{
    Value::{self, String as JsonString},
    json,
};
use tokio::sync::{Notify, OnceCell, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, warn};

const TWITCH_EVENTSUB_ADD_URL: &str = "https://api.twitch.tv/helix/eventsub/subscriptions";
const TWITCH_EVENTSUB_WS_URL: &str = "wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=30";

// https://twitchtokengenerator.com
const TWITCH_OAUTH_CLIENT_ID: &str = "gp762nuuoqcoxypju8c569th9wz7q5";

/// Initiates a Twitch eventsub feed listening for channels going offline after stream
///
/// # Returns
/// Returns an [`mpsc::Receiver`] channel with channel ID as a message
/// Each message equates to a channel going offline
#[instrument(skip(ct, broadcaster_ids))]
pub async fn listen_for_offline(
    ct: CancellationToken,
    broadcaster_ids: Vec<u64>,
) -> Result<mpsc::Receiver<u64>> {
    let client = reqwest::Client::builder()
        .user_agent(format!(
            "{}/{} (+{})",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_REPOSITORY")
        ))
        .http1_only() // https://github.com/jgraef/reqwest-websocket/issues/2
        .build()
        .unwrap();

    let (tx, rx) = mpsc::channel::<u64>(1);

    let session_id = Arc::new(tokio::sync::OnceCell::new());
    let notify = Arc::new(tokio::sync::Notify::new());

    // Handle websocket connection
    {
        let session_id = session_id.clone();
        let notify = notify.clone();
        let ws_client = client.clone();

        tokio::spawn(async move {
            let mut connection_url: Option<String> = None;

            'all: loop {
                info!("(re-)Connecting to Twitch EventSub via WebSocket");
                let upgrade = ws_client
                    .clone()
                    .get(connection_url.unwrap_or(TWITCH_EVENTSUB_WS_URL.to_string()))
                    .upgrade()
                    .send()
                    .await
                    .context("Connecting to Twitch's EventSub Endpoint")
                    .unwrap();

                let ws = upgrade
                    .into_websocket()
                    .await
                    .context("Upgrading HTTP request into a WebSocket")
                    .unwrap();
                let (_sink, stream) = ws.split();

                tokio::pin!(stream);
                'session: loop {
                    let action = tokio::select! {
                        () = ct.cancelled() => break 'all,
                        () = tokio::time::sleep(Duration::from_secs(40)) => {
                            panic!("Didn't get any message for 40s. Connection is effectively poisoned!")
                            // TODO: Handle re-registration (resend EventSub subscriptions)
                        },
                        Some(Ok(message)) = stream.next() => handle_ws_message(message, &session_id, &notify, &tx).await
                    };

                    if let SocketAction::Reconnect(url) = action {
                        connection_url = Some(url);
                        break 'session;
                    }
                }
            }
        });
    }

    // Register streamer once session_id is initialized
    {
        let session_id = session_id.clone();
        let notify = notify.clone();
        tokio::spawn(async move {
            notify.notified().await;
            info!("EventSub Session ID: {}", session_id.get().unwrap());

            for id in broadcaster_ids {
                info!("Requesting Twitch to send `stream.offline` event for broadcaster ID {id}");
                let req = client
                    .post(TWITCH_EVENTSUB_ADD_URL)
                    .header("Client-Id", TWITCH_OAUTH_CLIENT_ID)
                    .bearer_auth(
                        std::env::var("TWITCH_OAUTH_ACCESS_TOKEN")
                        .expect("Env var TWITCH_OAUTH_ACCESS_TOKEN is missing; Generate one on https://twitchtokengenerator.com/quick/yRxQrfaVAK"),
                    )
                    .json(&json!({
                        "type": "stream.offline",
                        "version": "1",
                        "condition": { "broadcaster_user_id": id.to_string() },
                        "transport": { "method": "websocket", "session_id": session_id.get().unwrap() }
                    }))
                    .send()
                    .await
                    .context(format!("Sending `stream.offline` event request for userid {id}"))
                    .unwrap();

                if !req.status().is_success() {
                    error!("Request failed: {}", req.text().await.unwrap());
                }
            }

            info!("All broadcaster listened successfully!");
        });
    }

    Ok(rx)
}

enum SocketAction {
    None,
    Reconnect(String),
}
async fn handle_ws_message(
    message: reqwest_websocket::Message,
    session_id: &OnceCell<Value>,
    notify: &Arc<Notify>,
    tx: &mpsc::Sender<u64>,
) -> SocketAction {
    match message {
        Message::Text(m) => {
            let m: Value = serde_json::from_str(&m).unwrap();

            if !session_id.initialized() {
                session_id
                    .set(m["payload"]["session"]["id"].clone())
                    .unwrap();
                notify.notify_one();
                return SocketAction::None;
            }

            let JsonString(message_type) = &m["metadata"]["message_type"] else {
                error!("Twitch message is missing message_type\n{m}\nSkipping...");
                return SocketAction::None;
            };

            match message_type.as_str() {
                "session_keepalive" => {
                    // Noop
                    return SocketAction::None;
                }
                "notification" => {
                    info!("Received notification message!");
                    let channel_id = &m["payload"]["event"]["broadcaster_user_id"]
                        .as_str()
                        .context("Stream offline notification message does not contain broadcaster_user_id!")
                        .unwrap()
                        .parse::<u64>()
                        .unwrap();
                    tx.send(*channel_id)
                        .await
                        .expect("Unable to announce offline");

                    return SocketAction::None;
                }
                "session_reconnect" => {
                    info!("Need to reconnect!");
                    let reconnect_url = m["payload"]["session"]["reconnect_url"].as_str().unwrap();
                    return SocketAction::Reconnect(reconnect_url.to_string());
                }

                other => {
                    warn!("Unhandled message type: {other}");
                    return SocketAction::None;
                }
            }
        }

        Message::Close { code, reason } => {
            // TODO: Proper retry / error handling code
            error!("{:?} {:?}", code, reason);
            panic!("Twitch closed WS connection!");
        }

        _ => {
            return SocketAction::None;
        }
    }
}
