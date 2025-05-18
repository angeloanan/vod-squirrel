use std::{str::FromStr, time::Duration};

use anyhow::{Context, Result, ensure};
use pkce::{code_challenge, code_verifier};
use reqwest::{Client, Url};
use serde_json::Value;
use tokio::{select, sync::watch};
use tokio_util::sync::CancellationToken;
use tracing::{info, instrument};

const OAUTH_AUTH_URI: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const OAUTH_TOKEN_URI: &str = "https://oauth2.googleapis.com/token";
const OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/youtube.upload";

const GOOGLE_CLIENT_ID: &str =
    "915486646698-1etc3ipkfvd77phikc9qrghnr1c1cam8.apps.googleusercontent.com";
/// oOoOoh a public secret. Scary~~
/// - <https://developers.google.com/identity/protocols/oauth2#installed> - [...] a client secret, which you embed in the
///   source code of your application. (In this context, the client secret is obviously not treated as a secret.)
/// - <https://stackoverflow.com/questions/72171227/what-does-the-google-client-secret-in-an-oauth2-application-give-access-to>
const GOOGLE_CLIENT_SECRET: &str = "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";
const GOOGLE_REDIRECT_URL: &str = "http://localhost:32547";

#[must_use = "Why create this then"]
pub fn generate_login_url() -> (Vec<u8>, Url) {
    let verifier = code_verifier(64);

    let mut url = Url::from_str(OAUTH_AUTH_URI).unwrap();
    url.query_pairs_mut()
        .append_pair("client_id", GOOGLE_CLIENT_ID)
        .append_pair("redirect_uri", GOOGLE_REDIRECT_URL)
        .append_pair("response_type", "code")
        .append_pair("scope", OAUTH_SCOPE)
        .append_pair("code_challenge", &code_challenge(&verifier))
        .append_pair("code_challenge_method", "S256")
        .finish();

    (verifier, url)
}

/// Exchanges OAuth authorization code into (refresh token, access token) pair
/// # Errors
/// Errors on network error / invalid auth code / malformed body
pub async fn exchange_auth_code(
    client: reqwest::Client,
    verifier: &[u8],
    code: &str,
) -> Result<(String, String)> {
    let req = client
        .post(OAUTH_TOKEN_URI)
        .query(&[
            ("client_id", GOOGLE_CLIENT_ID),
            ("client_secret", GOOGLE_CLIENT_SECRET),
            ("code", code),
            ("code_verifier", std::str::from_utf8(verifier).unwrap()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", GOOGLE_REDIRECT_URL),
        ])
        .send()
        .await
        .context("Sending authentication code exchange request")?;

    ensure!(
        req.status().is_success(),
        "Failed to exchange auth code to tokens"
    );
    let json = req.json::<Value>().await.unwrap();
    Ok((
        json["refresh_token"].as_str().unwrap().to_string(),
        json["access_token"].as_str().unwrap().to_string(),
    ))
}

/// Returned access token will always be active for 3600 seconds
/// # Errors
/// Errors on network error / JSON parsing error / invalid refresh token
pub async fn generate_access_token(
    client: reqwest::Client,
    refresh_token: &str,
) -> Result<Box<str>> {
    let req = client
        .post(OAUTH_TOKEN_URI)
        .query(&[
            ("client_id", GOOGLE_CLIENT_ID),
            ("client_secret", GOOGLE_CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .context("Sending refresh access token request")?;
    //

    ensure!(
        req.status().is_success(),
        "Unable to generate access token!"
    );

    let json = req
        .json::<Value>()
        .await
        .context("Parsing access token request")?;

    Ok(json["access_token"]
        .as_str()
        .unwrap()
        .to_string()
        .into_boxed_str())
}

/// # Panics
#[must_use = "You might use an expired access token"]
#[instrument(skip(ct))]
pub fn watch_access_token(ct: CancellationToken) -> watch::Receiver<Option<Box<str>>> {
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let refresh_token =
        std::env::var("REFRESH_TOKEN").expect("env var REFRESH_TOKEN not provided!");

    // Looping per 59 min in bg task
    // if refresh token is expired, bail / panic, go panic
    let (set_access_token, access_token) = watch::channel::<Option<Box<str>>>(None);
    tokio::spawn(async move {
        loop {
            let Ok(access_token) = generate_access_token(client.clone(), &refresh_token).await
            else {
                ct.cancel();
                panic!("Unable to renew access token!");
            };
            set_access_token.send_replace(Some(access_token));

            info!("Refreshed access token. Sleeping for 3600 sec");
            select! {
                () = tokio::time::sleep(Duration::from_secs(3540)) => {
                    info!("Refreshing access token");
                }
                () = ct.cancelled() => {
                    info!("Caught cancellation token!");
                    break
                }
            }
        }
    });

    access_token
}
