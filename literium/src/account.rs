//! `lit account` — identity against the litorium platform.
//!
//! Passwordless, CLI-first: the email-link device authorization
//! grant (authentes `DEVICE-AUTH.md`; the RFC 8628 shape with an
//! emailed one-time link as the approval channel). Creating an
//! account and signing in are the same act — the first verified
//! sign-in provisions the account — so there is no separate
//! `create` verb.
//!
//! The stored credential is the **rotating refresh token**
//! (litorium `SYNC.md` §4): access tokens live minutes and are
//! minted on demand by [`access_token`], which MUST persist the
//! newly returned refresh token immediately — authentes revokes
//! the whole family if a rotated-out token is ever reused.
//!
//! Credentials live at `$XDG_CONFIG_HOME/lit/credentials`
//! (default `~/.config/lit/credentials`), mode 0600, one
//! `key = "value"` per line. The identity host defaults to
//! litorium's issuer; `LIT_ID_URL` overrides it (local
//! development against a wrangler dev authentes).

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::error::Error;

pub const DEFAULT_ISSUER: &str = "https://id.litorium.org";
const CLIENT_ID: &str = "lit";
const SCOPE: &str = "pyloros";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
const USER_AGENT: &str = concat!("lit/", env!("CARGO_PKG_VERSION"));

pub fn issuer() -> String {
    std::env::var("LIT_ID_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ISSUER.to_string())
        .trim_end_matches('/')
        .to_string()
}

// ── The credentials file ────────────────────────────────────────

pub struct Credentials {
    pub issuer: String,
    pub email: String,
    pub refresh_token: String,
    /// Display cache; the authoritative copy lives with the
    /// account (`GET /account`).
    pub handle: Option<String>,
}

pub fn credentials_path() -> Result<PathBuf, Error> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| Error("cannot locate a config directory (no HOME)".into()))?;
    Ok(base.join("lit").join("credentials"))
}

pub fn load() -> Result<Option<Credentials>, Error> {
    let path = credentials_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error(format!("read {}: {e}", path.display()))),
    };
    let mut fields = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(
                key.trim().to_string(),
                value.trim().trim_matches('"').to_string(),
            );
        }
    }
    let take = |k: &str| -> Result<String, Error> {
        fields
            .get(k)
            .cloned()
            .ok_or_else(|| Error(format!("credentials file lacks {k}: {}", path.display())))
    };
    Ok(Some(Credentials {
        issuer: take("issuer")?,
        email: take("email")?,
        refresh_token: take("refresh_token")?,
        handle: fields.get("handle").cloned(),
    }))
}

pub fn save(credentials: &Credentials) -> Result<(), Error> {
    let path = credentials_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error(format!("create {}: {e}", dir.display())))?;
    }
    let mut body = format!(
        "issuer = \"{}\"\nemail = \"{}\"\nrefresh_token = \"{}\"\n",
        credentials.issuer, credentials.email, credentials.refresh_token
    );
    if let Some(handle) = &credentials.handle {
        body.push_str(&format!("handle = \"{handle}\"\n"));
    }
    write_private(&path, &body)
}

#[cfg(unix)]
fn write_private(path: &PathBuf, body: &str) -> Result<(), Error> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| Error(format!("write {}: {e}", path.display())))?;
    file.write_all(body.as_bytes())
        .map_err(|e| Error(format!("write {}: {e}", path.display())))
}

#[cfg(not(unix))]
fn write_private(path: &PathBuf, body: &str) -> Result<(), Error> {
    std::fs::write(path, body).map_err(|e| Error(format!("write {}: {e}", path.display())))
}

pub fn erase() -> Result<bool, Error> {
    let path = credentials_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error(format!("remove {}: {e}", path.display()))),
    }
}

// ── HTTP plumbing ───────────────────────────────────────────────

/// POST a form; return (status, body). Non-2xx is data here, not
/// failure — the device poll reads its state from 400 bodies.
fn post_form(url: &str, form: &[(&str, &str)]) -> Result<(u16, String), Error> {
    let request = ureq::post(url).set("User-Agent", USER_AGENT);
    match request.send_form(form) {
        Ok(response) => {
            let status = response.status();
            let body = response
                .into_string()
                .map_err(|e| Error(format!("read response from {url}: {e}")))?;
            Ok((status, body))
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Ok((status, body))
        }
        Err(e) => Err(Error(format!("cannot reach {url}: {e}"))),
    }
}

/// Bearer-authenticated request; same (status, body) contract
/// as [`post_form`].
fn with_bearer(
    method: &str,
    url: &str,
    token: &str,
    json: Option<&serde_json::Value>,
) -> Result<(u16, String), Error> {
    let request = ureq::request(method, url)
        .set("User-Agent", USER_AGENT)
        .set("Authorization", &format!("Bearer {token}"));
    let outcome = match json {
        Some(value) => request.send_json(value.clone()),
        None => request.call(),
    };
    match outcome {
        Ok(response) => {
            let status = response.status();
            let body = response
                .into_string()
                .map_err(|e| Error(format!("read response from {url}: {e}")))?;
            Ok((status, body))
        }
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Ok((status, body))
        }
        Err(e) => Err(Error(format!("cannot reach {url}: {e}"))),
    }
}

fn json_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(String::from)
}

fn oauth_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            json_field(&v, "error_description").or_else(|| json_field(&v, "error"))
        })
        .unwrap_or_else(|| "unexpected response".to_string())
}

// ── The device grant ────────────────────────────────────────────

pub struct DeviceGrant {
    pub user_code: String,
    device_code: String,
    interval: Duration,
    expires_in: Duration,
    issuer: String,
    email: String,
}

/// Step 1: initiate. authentes emails the one-time link and we
/// get the polling credential plus the user code the reader must
/// compare against the mail (RFC 8628 §5.4).
pub fn begin_login(email: &str) -> Result<DeviceGrant, Error> {
    let issuer = issuer();
    let (status, body) = post_form(
        &format!("{issuer}/device/authorize"),
        &[("client_id", CLIENT_ID), ("scope", SCOPE), ("email", email)],
    )?;
    if status == 501 {
        return Err(Error(
            "this identity host has no email sender configured".into(),
        ));
    }
    if status != 200 {
        return Err(Error(format!("sign-in refused: {}", oauth_error(&body))));
    }
    let value: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| Error(format!("bad initiation response: {e}")))?;
    let device_code = json_field(&value, "device_code")
        .ok_or_else(|| Error("initiation response lacks device_code".into()))?;
    let user_code = json_field(&value, "user_code")
        .ok_or_else(|| Error("initiation response lacks user_code".into()))?;
    let interval = value.get("interval").and_then(|v| v.as_u64()).unwrap_or(5);
    let expires_in = value
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(600);
    Ok(DeviceGrant {
        user_code,
        device_code,
        interval: Duration::from_secs(interval),
        expires_in: Duration::from_secs(expires_in),
        issuer,
        email: email.to_string(),
    })
}

/// Step 2: poll `/token` until the emailed link is redeemed.
/// Blocks (sleeping the advertised interval; `slow_down` backs
/// off per RFC 8628 §3.5) and returns the stored credentials.
pub fn wait_for_approval(grant: &DeviceGrant) -> Result<Credentials, Error> {
    let deadline = Instant::now() + grant.expires_in;
    let mut interval = grant.interval;
    loop {
        std::thread::sleep(interval);
        if Instant::now() > deadline {
            return Err(Error("the sign-in link expired — try again".into()));
        }
        let (status, body) = post_form(
            &format!("{}/token", grant.issuer),
            &[
                ("grant_type", DEVICE_GRANT),
                ("device_code", &grant.device_code),
                ("client_id", CLIENT_ID),
            ],
        )?;
        if status == 200 {
            let value: serde_json::Value = serde_json::from_str(&body)
                .map_err(|e| Error(format!("bad token response: {e}")))?;
            let refresh_token = json_field(&value, "refresh_token")
                .ok_or_else(|| Error("token response lacks refresh_token".into()))?;
            let credentials = Credentials {
                issuer: grant.issuer.clone(),
                email: grant.email.clone(),
                refresh_token,
                handle: None,
            };
            save(&credentials)?;
            return Ok(credentials);
        }
        let code = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| json_field(&v, "error"))
            .unwrap_or_default();
        match code.as_str() {
            "authorization_pending" => {}
            // RFC 8628 §3.5: back the interval off by 5 s.
            "slow_down" => interval += Duration::from_secs(5),
            "expired_token" => {
                return Err(Error("the sign-in link expired — try again".into()))
            }
            _ => return Err(Error(format!("sign-in refused: {}", oauth_error(&body)))),
        }
    }
}

/// Mint a fresh access token from the stored refresh token,
/// persisting the rotated refresh token before returning — the
/// non-negotiable discipline of SYNC.md §4.
pub fn access_token(credentials: &mut Credentials) -> Result<String, Error> {
    let (status, body) = post_form(
        &format!("{}/token", credentials.issuer),
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &credentials.refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )?;
    if status != 200 {
        return Err(Error(format!(
            "session expired or revoked ({}) — run `lit account login`",
            oauth_error(&body)
        )));
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| Error(format!("bad token response: {e}")))?;
    if let Some(rotated) = json_field(&value, "refresh_token") {
        credentials.refresh_token = rotated;
        save(credentials)?;
    }
    json_field(&value, "access_token")
        .ok_or_else(|| Error("token response lacks access_token".into()))
}

/// The account as the platform sees it (`GET /account`).
/// Refreshes the handle cache in the credentials file when it
/// drifted (e.g. set from another machine).
pub fn whoami(credentials: &mut Credentials) -> Result<Option<String>, Error> {
    let token = access_token(credentials)?;
    let (status, body) = with_bearer(
        "GET",
        &format!("{}/account", credentials.issuer),
        &token,
        None,
    )?;
    if status != 200 {
        return Err(Error(format!("account lookup: {}", oauth_error(&body))));
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| Error(format!("bad account response: {e}")))?;
    let handle = value
        .get("user")
        .and_then(|u| u.get("handle"))
        .and_then(|h| h.as_str())
        .map(String::from);
    if handle != credentials.handle {
        credentials.handle = handle.clone();
        save(credentials)?;
    }
    Ok(handle)
}

/// Claim the account's handle (`POST /account/handle`) — the
/// namespace future litorium surfaces hang the account's work
/// under. Set-once server-side; policy (grammar, minimum free
/// length, reserved names) is the deployment's.
pub fn set_handle(credentials: &mut Credentials, handle: &str) -> Result<(), Error> {
    let token = access_token(credentials)?;
    let (status, body) = with_bearer(
        "POST",
        &format!("{}/account/handle", credentials.issuer),
        &token,
        Some(&serde_json::json!({ "handle": handle })),
    )?;
    if status != 200 {
        return Err(Error(oauth_error(&body)));
    }
    credentials.handle = Some(handle.to_string());
    save(credentials)?;
    Ok(())
}

/// Best-effort server-side revocation (RFC 7009); the caller
/// erases the file regardless.
pub fn revoke(credentials: &Credentials) {
    let _ = post_form(
        &format!("{}/revoke", credentials.issuer),
        &[
            ("token", &credentials.refresh_token),
            ("client_id", CLIENT_ID),
        ],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_config_home<T>(f: impl FnOnce() -> T) -> T {
        let dir = std::env::temp_dir().join(format!("lit-account-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let out = f();
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn credentials_roundtrip_and_erase() {
        with_config_home(|| {
            assert!(load().unwrap().is_none());
            save(&Credentials {
                issuer: "https://id.example".into(),
                email: "w@example.com".into(),
                refresh_token: "abc.def".into(),
                handle: Some("a-test-handle".into()),
            })
            .unwrap();
            let back = load().unwrap().unwrap();
            assert_eq!(back.issuer, "https://id.example");
            assert_eq!(back.email, "w@example.com");
            assert_eq!(back.refresh_token, "abc.def");
            assert_eq!(back.handle.as_deref(), Some("a-test-handle"));

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(credentials_path().unwrap())
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600);
            }

            assert!(erase().unwrap());
            assert!(!erase().unwrap());
            assert!(load().unwrap().is_none());
        });
    }

    #[test]
    fn issuer_env_override_and_default() {
        std::env::remove_var("LIT_ID_URL");
        assert_eq!(issuer(), DEFAULT_ISSUER);
        std::env::set_var("LIT_ID_URL", "http://localhost:8787/");
        assert_eq!(issuer(), "http://localhost:8787");
        std::env::remove_var("LIT_ID_URL");
    }

    /// Live leg, run explicitly after a completed login
    /// (`cargo test -- --ignored live_refresh`), with
    /// XDG_CONFIG_HOME and LIT_ID_URL pointing at the harness:
    /// two consecutive refreshes prove the rotated token is
    /// persisted before use (SYNC.md §4's discipline).
    #[test]
    #[ignore]
    fn live_refresh_rotates_twice() {
        let mut credentials = load().unwrap().expect("login first");
        let first = credentials.refresh_token.clone();
        let access1 = access_token(&mut credentials).unwrap();
        assert_ne!(credentials.refresh_token, first, "rotation persisted");
        let access2 = access_token(&mut credentials).unwrap();
        assert!(!access1.is_empty() && !access2.is_empty());
        // Fresh load sees the newest rotation.
        let reloaded = load().unwrap().unwrap();
        assert_eq!(reloaded.refresh_token, credentials.refresh_token);
    }

    #[test]
    fn oauth_error_extraction() {
        assert_eq!(
            oauth_error(r#"{"error":"expired_token","error_description":"gone"}"#),
            "gone"
        );
        assert_eq!(oauth_error(r#"{"error":"slow_down"}"#), "slow_down");
        assert_eq!(oauth_error("not json"), "unexpected response");
    }
}
