#![allow(dead_code)]
use std::sync::mpsc::Receiver;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::config::{
    Account, ACCOUNT_TYPE_ELY_OAUTH, ACCOUNT_TYPE_ELY_PASSWORD, ACCOUNT_TYPE_OFFLINE,
};
use crate::util::{
    DEVICE_GRANT_TYPE, ELY_CLIENT_ID, OAUTH_DEVICE_CODE_URL, OAUTH_INFO_URL, OAUTH_SCOPES,
    OAUTH_TOKEN_URL, YGGDRASIL_AUTH_URL, YGGDRASIL_REFRESH_URL,
};
use crate::worker::{LaunchDecision, TaskCtx, TaskEvent};

/// Marker string used by workers to signal the user cancelled the task.
pub const CANCELLED_MARKER: &str = "__CANCELLED__";

/// Build an offline-mode account from a player name (classic offline UUID).
pub fn offline_account(username: &str) -> Account {
    let uuid = Uuid::new_v3(&Uuid::nil(), username.as_bytes()).to_string();
    Account {
        uuid,
        username: username.to_string(),
        access_token: Uuid::new_v4().to_string(),
        refresh_token: None,
        client_token: None,
        account_type: ACCOUNT_TYPE_OFFLINE.to_string(),
        expires_at: None,
    }
}

#[derive(Debug, Clone)]
struct OAuthTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    id: i64,
    uuid: String,
    username: String,
}

#[derive(Debug, Deserialize)]
struct YggdrasilProfile {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YggdrasilAuthResponse {
    access_token: String,
    client_token: String,
    selected_profile: Option<YggdrasilProfile>,
}

/// Response of the OAuth2 Device Authorization Grant `devicecode` request.
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    interval: u64,
    expires_in: u64,
}

/// Poll the token endpoint until the user approves the device code or an
/// error / timeout occurs.
fn poll_device_tokens(
    client: &Client,
    decisions: &Receiver<LaunchDecision>,
    device_code: &str,
    interval: u64,
    expires_in: u64,
) -> Result<OAuthTokens> {
    let deadline = SystemTime::now() + Duration::from_secs(expires_in.max(60));
    let mut interval = interval.max(1);
    loop {
        if matches!(decisions.try_recv(), Ok(LaunchDecision::Cancel)) {
            bail!(CANCELLED_MARKER);
        }
        if SystemTime::now() > deadline {
            bail!(crate::lang::current().code_expired);
        }
        let resp = client
            .post(OAUTH_TOKEN_URL)
            .form(&[
                ("grant_type", DEVICE_GRANT_TYPE),
                ("client_id", ELY_CLIENT_ID),
                ("device_code", device_code),
            ])
            .send()
            .context(crate::lang::current().failed_connect_ely)?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().context("Respons token bukan JSON")?;
        if status.is_success() {
            return Ok(OAuthTokens {
                access_token: body
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("Respons token tidak berisi access_token"))?
                    .to_string(),
                refresh_token: body
                    .get("refresh_token")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                expires_in: body.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600),
            });
        }
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        match err {
            "authorization_pending" => {
                std::thread::sleep(Duration::from_secs(interval));
            }
            "slow_down" => {
                interval += 5;
                std::thread::sleep(Duration::from_secs(interval));
            }
            "access_denied" => bail!(crate::lang::current().access_denied),
            "expired_token" => bail!(crate::lang::current().code_expired),
            other => bail!(crate::lang::current().login_denied.replace("{}", other)),
        }
    }
}

fn fetch_user_info(client: &Client, access_token: &str) -> Result<UserInfo> {
    let resp = client
        .get(OAUTH_INFO_URL)
        .bearer_auth(access_token)
        .send()
        .context(crate::lang::current().failed_fetch_account)?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("Respons info akun bukan JSON")?;
    if !status.is_success() {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        bail!(
            crate::lang::current()
                .failed_fetch_account_status
                .replace("{}", &status.to_string())
                .replacen("{}", msg, 1)
        );
    }
    Ok(serde_json::from_value(body).context("Format info akun tidak dikenal")?)
}

/// Run the OAuth2 Device Authorization Grant flow. Desktop applications on
/// Ely.by have no redirect_uri, so the user authorizes by opening the
/// `verification_uri` page and entering the `user_code`.
pub fn login_oauth_device(ctx: &TaskCtx) -> Result<Account> {
    let resp = ctx
        .client
        .post(OAUTH_DEVICE_CODE_URL)
        .form(&[("client_id", ELY_CLIENT_ID), ("scope", OAUTH_SCOPES)])
        .send()
        .context(crate::lang::current().failed_start_login)?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("Respons device code bukan JSON")?;
    if !status.is_success() {
        let err = body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let desc = body
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        bail!(
            crate::lang::current()
                .login_request_denied
                .replace("{}", err)
                .replacen("{}", desc, 1)
        );
    }
    let dev: DeviceCodeResponse =
        serde_json::from_value(body).context("Format respons device code tidak dikenal")?;

    ctx.reporter.log(
        crate::lang::current()
            .open_page_code
            .replace("{}", &dev.verification_uri)
            .replacen("{}", &dev.user_code, 1),
    );
    let _ = ctx.tx.send(TaskEvent::DeviceCodeRequired {
        code: dev.user_code.clone(),
        verification_uri: dev.verification_uri.clone(),
    });

    let tokens = poll_device_tokens(&ctx.client, &ctx.decisions, &dev.device_code, dev.interval, dev.expires_in)?;
    ctx.reporter.log(crate::lang::current().token_received_fetching_profile);

    let info = fetch_user_info(&ctx.client, &tokens.access_token)?;

    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + tokens.expires_in as i64;

    Ok(Account {
        uuid: normalize_uuid(&info.uuid),
        username: info.username,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        client_token: None,
        account_type: ACCOUNT_TYPE_ELY_OAUTH.to_string(),
        expires_at: Some(expires_at),
    })
}

/// Refresh an OAuth-based account using its `refresh_token`.
pub fn refresh_oauth(client: &Client, account: &Account) -> Result<Account> {
    let refresh_token = account
        .refresh_token
        .as_deref()
        .ok_or_else(|| anyhow!(crate::lang::current().no_refresh_token))?;
    let params = vec![
        ("client_id", ELY_CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("scope", OAUTH_SCOPES),
        ("refresh_token", refresh_token),
    ];
    let resp = client
        .post(OAUTH_TOKEN_URL)
        .form(&params)
        .send()
        .context(crate::lang::current().failed_refresh_token)?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("Respons refresh bukan JSON")?;
    if !status.is_success() {
        let err = body.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        let desc = body
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        bail!("Refresh token ditolak ({err}): {desc}. Silakan login ulang.");
    }
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
        + body.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600) as i64;

    let mut updated = account.clone();
    updated.access_token = body
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Respons refresh tidak berisi access_token"))?
        .to_string();
    if let Some(rt) = body.get("refresh_token").and_then(|v| v.as_str()) {
        updated.refresh_token = Some(rt.to_string());
    }
    updated.expires_at = Some(expires_at);
    Ok(updated)
}

/// Log in using Ely.by username/e-mail + password (Yggdrasil compatible API).
/// Pass a TOTP code to re-run after receiving a 2FA challenge.
pub fn login_password(
    client: &Client,
    username: &str,
    password: &str,
    client_token: &str,
    twofa: Option<&str>,
) -> Result<Account> {
    let password = match twofa {
        Some(code) => format!("{password}:{code}"),
        None => password.to_string(),
    };
    let body = json!({
        "username": username,
        "password": password,
        "clientToken": client_token,
        "requestUser": true,
    });
    let resp = client
        .post(YGGDRASIL_AUTH_URL)
        .json(&body)
        .send()
        .context(crate::lang::current().failed_connect_ely)?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let msg = parsed
            .get("errorMessage")
            .and_then(|v| v.as_str())
            .unwrap_or(crate::lang::current().wrong_credentials);
        if msg.contains("two factor auth") || msg.contains("two-factor auth") {
            bail!("__NEED_2FA__");
        }
        bail!(crate::lang::current().login_failed.replace("{}", msg));
    }
    let parsed: YggdrasilAuthResponse = serde_json::from_str(&text)
        .context(crate::lang::current().unknown_auth_response)?;
    let profile = parsed
        .selected_profile
        .ok_or_else(|| anyhow!(crate::lang::current().no_minecraft_profile))?;
    Ok(Account {
        uuid: normalize_uuid(&profile.id),
        username: profile.name,
        access_token: parsed.access_token,
        refresh_token: None,
        client_token: Some(parsed.client_token),
        account_type: ACCOUNT_TYPE_ELY_PASSWORD.to_string(),
        expires_at: Some(now_unix() + 24 * 3600),
    })
}

/// Refresh a password-based account using the Yggdrasil refresh endpoint.
pub fn refresh_password(client: &Client, account: &Account) -> Result<Account> {
    let client_token = account
        .client_token
        .as_deref()
        .ok_or_else(|| anyhow!(crate::lang::current().no_client_token))?;
    let body = json!({
        "accessToken": account.access_token,
        "clientToken": client_token,
        "requestUser": true,
    });
    let resp = client
        .post(YGGDRASIL_REFRESH_URL)
        .json(&body)
        .send()
        .context(crate::lang::current().failed_refresh_token)?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let msg = parsed
            .get("errorMessage")
            .and_then(|v| v.as_str())
            .unwrap_or(crate::lang::current().invalid_token);
        bail!(
            crate::lang::current()
                .refresh_failed
                .replace("{}", msg)
        );
    }
    let parsed: YggdrasilAuthResponse = serde_json::from_str(&text)
        .context(crate::lang::current().unknown_refresh_response)?;
    let mut updated = account.clone();
    updated.access_token = parsed.access_token;
    updated.client_token = Some(parsed.client_token);
    updated.expires_at = Some(now_unix() + 24 * 3600);
    Ok(updated)
}

/// Download the Ely.by head skin for an account and return RGBA pixels.
pub fn fetch_avatar(ctx: &TaskCtx, uuid: &str) -> Result<Option<(u32, u32, Vec<u8>)>> {
    let url = format!("https://skinsystem.ely.by/head/{uuid}.png");
    let resp = ctx
        .client
        .get(&url)
        .timeout(Duration::from_secs(15))
        .send()
        .context(crate::lang::current().failed_fetch_avatar)?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let bytes = resp
        .bytes()
        .context(crate::lang::current().failed_read_avatar)?;
    let img = image::load_from_memory(&bytes).context(crate::lang::current().invalid_avatar_image)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(Some((w, h, rgba.into_raw())))
}

pub fn normalize_uuid(id: &str) -> String {
    let trimmed = id.trim().replace('-', "");
    match Uuid::parse_str(&trimmed) {
        Ok(u) => u.to_string(),
        Err(_) => id.to_string(),
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
