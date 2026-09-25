use std::{
    ffi::OsStr,
    fs::File,
    io::{Read as _, Write as _},
    os::fd::OwnedFd,
    path::Path,
};

use serde_json::{Value, json};

use super::game_def::{GameDef, PortSlot};
use crate::{GameCreateSpec, GameKind};

const DOCKERFILE: &str = include_str!("../../hytale/Dockerfile");
const ENTRYPOINT: &str = include_str!("../../hytale/entrypoint.sh");

/// Hytale speaks QUIC, so the single game port is UDP only.
const SLOTS: &[PortSlot] = &[PortSlot::game(0, false, true)];

const STATE_DIR: &str = ".helix";
const AUTH_FILE: &str = "hytale-auth.json";
const CONSOLE_FIFO: &str = "console.fifo";
const MAX_AUTH_FILE_BYTES: u64 = 4 * 1024;
pub(crate) const MAX_CONSOLE_COMMAND_BYTES: usize = 512;

pub(crate) const DEF: GameDef = GameDef {
    kind: GameKind::Hytale,
    slug: "hytale",
    display: "Hytale",
    runtime_image: "helix-hytale-runtime:1",
    dockerfile: DOCKERFILE,
    entrypoint: ENTRYPOINT,
    artifact: "hytale://server",
    memory: (4_096, 32_768),
    players: (1, 100),
    defaults: (6_144, 16),
    slots: SLOTS,
    caves_slot: None,
    settings_file: "hytale.json",
    install_markers: &["Server/HytaleServer.jar", "Assets.zip"],
    data_dirs: &["mods", "universe", "logs"],
    pool: (5_520, 5_570),
    create_settings,
};

fn create_settings(spec: &GameCreateSpec, _generated: &str) -> Value {
    let mut settings = json!({
        "auto_update": true,
        "patchline": "release",
    });
    if let Some(password) = spec
        .server_password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings["server_password"] = Value::from(password);
    }
    settings
}

/// The sign-in prompt the runtime last saw, validated so a compromised server
/// cannot make Helix show an arbitrary link: only `https://*.hytale.com` URLs
/// and short alphanumeric codes are returned.
pub(crate) fn read_auth_prompt(data_path: &Path) -> Value {
    let Ok(body) = read_state_file(data_path, AUTH_FILE, MAX_AUTH_FILE_BYTES) else {
        return json!({ "state": "unknown" });
    };
    let Ok(parsed) = serde_json::from_slice::<Value>(&body) else {
        return json!({ "state": "unknown" });
    };
    sanitize_auth_prompt(&parsed)
}

fn sanitize_auth_prompt(parsed: &Value) -> Value {
    let state = match parsed.get("state").and_then(Value::as_str) {
        Some("needs_sign_in") => "needs_sign_in",
        Some("signed_in") => "signed_in",
        _ => return json!({ "state": "unknown" }),
    };
    let stage = match parsed.get("stage").and_then(Value::as_str) {
        Some("download") => "download",
        _ => "server",
    };
    let url = parsed
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| is_hytale_https_url(url));
    let code = parsed.get("code").and_then(Value::as_str).filter(|code| {
        (4..=32).contains(&code.len())
            && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    if state == "needs_sign_in" && url.is_none() {
        return json!({ "state": "unknown" });
    }
    json!({
        "state": state,
        "stage": stage,
        "url": url,
        "code": code,
        "updated_at_unix": parsed.get("updated_at").and_then(Value::as_u64),
    })
}

fn is_hytale_https_url(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("https://") else {
        return false;
    };
    if value.len() > 512
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./?=&_%:~+-".contains(&b))
    {
        return false;
    }
    let host = rest.split(['/', '?']).next().unwrap_or_default();
    if host.contains([':', '@']) {
        return false;
    }
    host == "hytale.com" || host.ends_with(".hytale.com")
}

/// Send one console line to the running server through its stdin pipe.
pub(crate) fn send_console_command(data_path: &Path, command: &str) -> Result<(), String> {
    let command = command.trim();
    if command.is_empty()
        || command.len() > MAX_CONSOLE_COMMAND_BYTES
        || command.chars().any(char::is_control)
    {
        return Err("console command must be one non-empty line under 512 bytes".to_owned());
    }
    let command = if command.starts_with('/') {
        command.to_owned()
    } else {
        format!("/{command}")
    };
    let state = open_state_dir(data_path)?;
    // The runtime owns this folder, so refuse links and anything but a FIFO.
    // O_NONBLOCK makes the open fail instead of hanging when nothing reads it.
    let fifo = rustix::fs::openat(
        &state,
        CONSOLE_FIFO,
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| "the Hytale console is not running yet; start the server first".to_owned())?;
    let stat =
        rustix::fs::fstat(&fifo).map_err(|_| "could not inspect the Hytale console".to_owned())?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Fifo {
        return Err("the Hytale console pipe was replaced; restart the server".to_owned());
    }
    let mut line = command.into_bytes();
    line.push(b'\n');
    // One write of at most PIPE_BUF bytes is atomic, so commands never interleave.
    let mut pipe = File::from(fifo);
    pipe.write_all(&line)
        .map_err(|_| "the Hytale console did not accept the command".to_owned())
}

fn open_state_dir(data_path: &Path) -> Result<OwnedFd, String> {
    let root = rustix::fs::open(
        data_path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| "the server folder is unavailable".to_owned())?;
    rustix::fs::openat(
        &root,
        STATE_DIR,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| "the Hytale runtime has not started yet".to_owned())
}

fn read_state_file(data_path: &Path, name: &str, maximum: u64) -> Result<Vec<u8>, String> {
    let state = open_state_dir(data_path)?;
    let fd = rustix::fs::openat(
        &state,
        OsStr::new(name),
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| "no Hytale sign-in state yet".to_owned())?;
    let file = File::from(fd);
    let metadata = file
        .metadata()
        .map_err(|_| "could not inspect the Hytale sign-in state".to_owned())?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err("the Hytale sign-in state is invalid".to_owned());
    }
    let mut body = Vec::new();
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|_| "could not read the Hytale sign-in state".to_owned())?;
    if u64::try_from(body.len()).unwrap_or(u64::MAX) > maximum {
        return Err("the Hytale sign-in state is invalid".to_owned());
    }
    Ok(body)
}

/// One-line progress text for the create job while Hytale waits for sign-in.
pub(crate) fn sign_in_progress(data_path: &Path) -> Option<String> {
    let prompt = read_auth_prompt(data_path);
    if prompt.get("state").and_then(Value::as_str) != Some("needs_sign_in") {
        return None;
    }
    let url = prompt.get("url").and_then(Value::as_str)?;
    let what = if prompt.get("stage").and_then(Value::as_str) == Some("download") {
        "to download the server files"
    } else {
        "so players can join"
    };
    Some(match prompt.get("code").and_then(Value::as_str) {
        Some(code) => format!("Sign in to Hytale {what}: open {url} and confirm code {code}"),
        None => format!("Sign in to Hytale {what}: open {url}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hytale_https_links_are_shown() {
        assert!(is_hytale_https_url(
            "https://oauth.accounts.hytale.com/oauth2/device/verify?user_code=ABCD-1234"
        ));
        assert!(is_hytale_https_url("https://hytale.com/device"));
        for bad in [
            "http://oauth.accounts.hytale.com/verify",
            "https://hytale.com.evil.example/verify",
            "https://evilhytale.com/verify",
            "https://user@oauth.accounts.hytale.com/verify",
            "https://oauth.accounts.hytale.com:8443/verify",
            "https://oauth.accounts.hytale.com/verify\"><script>",
            "javascript:alert(1)",
        ] {
            assert!(!is_hytale_https_url(bad), "{bad}");
        }
    }

    #[test]
    fn prompts_are_reduced_to_validated_fields() {
        let prompt = sanitize_auth_prompt(&json!({
            "stage": "download",
            "state": "needs_sign_in",
            "url": "https://oauth.accounts.hytale.com/oauth2/device/verify?user_code=WXYZ",
            "code": "WXYZ",
            "updated_at": 1,
            "extra": "ignored"
        }));
        assert_eq!(prompt["state"], "needs_sign_in");
        assert_eq!(prompt["stage"], "download");
        assert_eq!(prompt["code"], "WXYZ");
        assert!(prompt.get("extra").is_none());
        let forged = sanitize_auth_prompt(&json!({
            "state": "needs_sign_in",
            "url": "https://phish.example/hytale.com/verify",
            "code": "<b>"
        }));
        assert_eq!(forged["state"], "unknown");
        let signed = sanitize_auth_prompt(&json!({"state": "signed_in", "url": "", "code": ""}));
        assert_eq!(signed["state"], "signed_in");
        assert!(signed["code"].is_null());
    }

    #[test]
    fn console_writes_only_reach_a_real_fifo() {
        use std::os::unix::fs::OpenOptionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        assert!(send_console_command(dir.path(), "say hi").is_err());
        std::fs::create_dir(dir.path().join(STATE_DIR)).unwrap();
        let fifo = dir.path().join(STATE_DIR).join(CONSOLE_FIFO);
        std::fs::write(&fifo, b"").unwrap();
        assert!(send_console_command(dir.path(), "say hi").is_err());
        std::fs::remove_file(&fifo).unwrap();
        std::os::unix::fs::symlink("/etc/hostname", &fifo).unwrap();
        assert!(send_console_command(dir.path(), "say hi").is_err());
        std::fs::remove_file(&fifo).unwrap();
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::Mode::from_raw_mode(0o600),
        )
        .unwrap();
        // No reader yet: the non-blocking open fails instead of hanging.
        assert!(send_console_command(dir.path(), "say hi").is_err());
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(&fifo)
            .unwrap();
        send_console_command(dir.path(), "say hi").unwrap();
        let mut received = String::new();
        let mut reader = reader;
        reader.read_to_string(&mut received).ok();
        assert_eq!(received, "/say hi\n");
        assert!(send_console_command(dir.path(), "a\nb").is_err());
        assert!(send_console_command(dir.path(), &"x".repeat(600)).is_err());
    }
}
