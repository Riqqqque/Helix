//! Rack-machine hub: hub SSH identity, probes, Wake-on-LAN, and power control.
//!
//! privd owns the hub keypair on disk but every SSH invocation (probes and the
//! terminal bridge alike) runs as the unprivileged terminal account, so remote
//! commands never execute with broker credentials.

use crate::bounded_command::run_bounded_command;
use helix_privd::{
    HubIdentityInfo, MachineAuthKind, MachinePowerAction, MachineProbeResult, MachineTarget,
    MachineTerminalSpecResult,
};
use serde::Deserialize;
use std::{
    fs,
    io::ErrorKind,
    net::{TcpStream, ToSocketAddrs, UdpSocket},
    os::unix::fs::{MetadataExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SSH_PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_PROBE_OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HubConfig {
    #[serde(default = "default_key_root")]
    pub(crate) key_root: PathBuf,
    #[serde(default = "default_terminal_socket")]
    pub(crate) terminal_socket: PathBuf,
    #[serde(default = "default_ssh_binary")]
    pub(crate) ssh_binary: PathBuf,
    #[serde(default = "default_ssh_keygen_binary")]
    pub(crate) ssh_keygen_binary: PathBuf,
    #[serde(default = "default_timeout_binary")]
    pub(crate) timeout_binary: PathBuf,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            key_root: default_key_root(),
            terminal_socket: default_terminal_socket(),
            ssh_binary: default_ssh_binary(),
            ssh_keygen_binary: default_ssh_keygen_binary(),
            timeout_binary: default_timeout_binary(),
        }
    }
}

fn default_key_root() -> PathBuf {
    PathBuf::from("/var/lib/helix-hub")
}

fn default_terminal_socket() -> PathBuf {
    PathBuf::from("/run/helix/terminal/terminal.sock")
}

fn default_ssh_binary() -> PathBuf {
    PathBuf::from("/usr/bin/ssh")
}

fn default_ssh_keygen_binary() -> PathBuf {
    PathBuf::from("/usr/bin/ssh-keygen")
}

fn default_timeout_binary() -> PathBuf {
    PathBuf::from("/usr/bin/timeout")
}

/// The terminal daemon's uid/gid, discovered from the socket it owns. Key
/// files are chowned to this account so the PTY bridge can read them while
/// they stay unreadable to everything else.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalAccount {
    uid: u32,
    gid: u32,
    name: Option<String>,
    home: Option<String>,
}

pub(crate) struct Hub {
    config: HubConfig,
}

impl Hub {
    pub(crate) fn new(config: HubConfig) -> Result<Self, String> {
        if !config.key_root.is_absolute() {
            return Err("hub key_root must be an absolute path".to_owned());
        }
        if !config.terminal_socket.is_absolute() {
            return Err("hub terminal_socket must be an absolute path".to_owned());
        }
        for (name, binary) in [
            ("ssh", &config.ssh_binary),
            ("ssh-keygen", &config.ssh_keygen_binary),
            ("timeout", &config.timeout_binary),
        ] {
            if !binary.is_absolute() || !binary.is_file() {
                return Err(format!(
                    "hub {name} binary is missing at {}",
                    binary.display()
                ));
            }
        }
        Ok(Self { config })
    }

    fn terminal_account(&self) -> Result<TerminalAccount, String> {
        let metadata = fs::metadata(&self.config.terminal_socket).map_err(|error| {
            if error.kind() == ErrorKind::NotFound {
                "the terminal service is not running, so the hub cannot tell which account to use"
                    .to_owned()
            } else {
                format!("could not inspect the terminal socket: {error}")
            }
        })?;
        let uid = metadata.uid();
        let gid = metadata.gid();
        if uid == 0 {
            return Err(
                "the terminal socket is owned by root; refusing to run SSH as root".to_owned(),
            );
        }
        let (name, home) = passwd_entry_for_uid(uid);
        Ok(TerminalAccount {
            uid,
            gid,
            name,
            home,
        })
    }

    fn key_path(&self) -> PathBuf {
        self.config.key_root.join("id_ed25519")
    }

    fn public_key_path(&self) -> PathBuf {
        self.config.key_root.join("id_ed25519.pub")
    }

    fn known_hosts_path(&self) -> PathBuf {
        self.config.key_root.join("known_hosts")
    }

    /// Ensure the hub directory and keypair exist, owned by the terminal
    /// account. Returns the identity for display, or an unavailable status.
    fn ensure_identity(&self) -> Result<(TerminalAccount, String, String), String> {
        let account = self.terminal_account()?;
        let key_root = &self.config.key_root;
        if !key_root.is_dir() {
            fs::create_dir_all(key_root)
                .map_err(|_| format!("could not create {}", key_root.display()))?;
        }
        fs::set_permissions(key_root, fs::Permissions::from_mode(0o700))
            .map_err(|_| format!("could not secure {}", key_root.display()))?;
        run_program(
            Path::new("/usr/bin/chown"),
            &[
                format!("{}:{}", account.uid, account.gid),
                key_root.to_string_lossy().into_owned(),
            ],
            10,
        )?;
        let key_path = self.key_path();
        if !key_path.is_file() {
            let output = run_bounded_command(
                &self.config.timeout_binary,
                &self.config.ssh_keygen_binary,
                &[
                    "-t".to_owned(),
                    "ed25519".to_owned(),
                    "-N".to_owned(),
                    String::new(),
                    "-C".to_owned(),
                    "helix-hub".to_owned(),
                    "-f".to_owned(),
                    key_path.to_string_lossy().into_owned(),
                    "-q".to_owned(),
                ],
                Duration::from_secs(20),
                &[],
                MAX_PROBE_OUTPUT_BYTES,
            )?;
            if !output.status.success() {
                return Err(format!(
                    "ssh-keygen could not create the hub identity: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        for (path, mode) in [
            (key_path.clone(), 0o600),
            (self.public_key_path(), 0o644),
            (self.known_hosts_path(), 0o600),
        ] {
            if !path.exists() && path == self.known_hosts_path() {
                fs::write(&path, b"")
                    .map_err(|_| format!("could not create {}", path.display()))?;
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                .map_err(|_| format!("could not secure {}", path.display()))?;
            run_program(
                Path::new("/usr/bin/chown"),
                &[
                    format!("{}:{}", account.uid, account.gid),
                    path.to_string_lossy().into_owned(),
                ],
                10,
            )?;
        }
        let public_key = fs::read_to_string(self.public_key_path())
            .map_err(|_| "the hub public key could not be read".to_owned())?;
        let fingerprint = self.fingerprint()?;
        Ok((account, public_key.trim().to_owned(), fingerprint))
    }

    fn fingerprint(&self) -> Result<String, String> {
        let output = run_bounded_command(
            &self.config.timeout_binary,
            &self.config.ssh_keygen_binary,
            &[
                "-l".to_owned(),
                "-f".to_owned(),
                self.public_key_path().to_string_lossy().into_owned(),
            ],
            Duration::from_secs(10),
            &[],
            MAX_PROBE_OUTPUT_BYTES,
        )?;
        if !output.status.success() {
            return Err("could not fingerprint the hub identity".to_owned());
        }
        let text = String::from_utf8_lossy(&output.stdout);
        text.split_whitespace()
            .nth(1)
            .map(str::to_owned)
            .ok_or_else(|| "the hub fingerprint could not be parsed".to_owned())
    }

    pub(crate) fn identity(&self) -> Result<HubIdentityInfo, String> {
        match self.ensure_identity() {
            Ok((account, public_key, fingerprint)) => Ok(HubIdentityInfo {
                available: true,
                public_key: Some(public_key),
                fingerprint_sha256: Some(fingerprint),
                user: account.name,
                detail: "Hub identity ready. Authorize this key on each machine you want Helix to reach.".to_owned(),
            }),
            Err(detail) => Ok(HubIdentityInfo {
                available: false,
                public_key: None,
                fingerprint_sha256: None,
                user: None,
                detail,
            }),
        }
    }

    fn validate_target(machine: &MachineTarget) -> Result<(), String> {
        validate_host(&machine.host)?;
        validate_username(&machine.username)?;
        if machine.port == 0 {
            return Err("machine port must be between 1 and 65535".to_owned());
        }
        Ok(())
    }

    fn ssh_argv(&self, machine: &MachineTarget, batch: bool) -> Vec<String> {
        let mut argv = vec!["ssh".to_owned()];
        if !batch {
            argv.push("-tt".to_owned());
        }
        argv.extend([
            "-o".to_owned(),
            "ConnectTimeout=10".to_owned(),
            "-o".to_owned(),
            "ServerAliveInterval=15".to_owned(),
            "-o".to_owned(),
            "ServerAliveCountMax=3".to_owned(),
            "-o".to_owned(),
            "StrictHostKeyChecking=accept-new".to_owned(),
            "-o".to_owned(),
            format!("UserKnownHostsFile={}", self.known_hosts_path().display()),
        ]);
        if batch {
            argv.extend(["-o".to_owned(), "BatchMode=yes".to_owned()]);
        }
        match machine.auth_kind {
            MachineAuthKind::Key => {
                argv.extend([
                    "-i".to_owned(),
                    self.key_path().to_string_lossy().into_owned(),
                    "-o".to_owned(),
                    "IdentitiesOnly=yes".to_owned(),
                    "-o".to_owned(),
                    "PreferredAuthentications=publickey".to_owned(),
                ]);
            }
            MachineAuthKind::Password => {
                argv.extend([
                    "-o".to_owned(),
                    "PubkeyAuthentication=no".to_owned(),
                    "-o".to_owned(),
                    "PreferredAuthentications=password,keyboard-interactive".to_owned(),
                    "-o".to_owned(),
                    "NumberOfPasswordPrompts=2".to_owned(),
                ]);
            }
            MachineAuthKind::System => {}
        }
        argv.extend([
            "-p".to_owned(),
            machine.port.to_string(),
            format!("{}@{}", machine.username, machine.host),
        ]);
        argv
    }

    /// ssh argv for the terminal bridge. The daemon executes this inside the
    /// PTY as the terminal account, so the hub key must already be provisioned
    /// and readable by that account.
    pub(crate) fn terminal_spec(
        &self,
        machine: &MachineTarget,
    ) -> Result<MachineTerminalSpecResult, String> {
        Self::validate_target(machine)?;
        if machine.auth_kind == MachineAuthKind::Key {
            self.ensure_identity()?;
        }
        Ok(MachineTerminalSpecResult {
            argv: self.ssh_argv(machine, false),
            detail: format!("{}@{}", machine.username, machine.host),
        })
    }

    /// Ensure the hub known_hosts exists with the terminal account as owner.
    /// Broker-run ssh executes as root; if root created this file the
    /// terminal account could no longer append host keys during interactive
    /// sessions, which would break accept-new trust for them.
    fn ensure_known_hosts(&self, account: &TerminalAccount) -> Result<(), String> {
        let path = self.known_hosts_path();
        if let Some(parent) = path.parent() {
            if !parent.is_dir() {
                fs::create_dir_all(parent)
                    .map_err(|_| format!("could not create {}", parent.display()))?;
            }
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| format!("could not secure {}", parent.display()))?;
            run_program(
                Path::new("/usr/bin/chown"),
                &[
                    format!("{}:{}", account.uid, account.gid),
                    parent.to_string_lossy().into_owned(),
                ],
                10,
            )?;
        }
        if !path.exists() {
            fs::write(&path, b"").map_err(|_| format!("could not create {}", path.display()))?;
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|_| format!("could not secure {}", path.display()))?;
        run_program(
            Path::new("/usr/bin/chown"),
            &[
                format!("{}:{}", account.uid, account.gid),
                path.to_string_lossy().into_owned(),
            ],
            10,
        )?;
        Ok(())
    }

    /// Run ssh directly from the broker. The service's no-new-privileges
    /// sandbox forbids uid transitions, so probes and power actions execute
    /// as root with an isolated HOME: the hub key root for hub/password
    /// auth, or the terminal account's home for system auth so its
    /// ~/.ssh config and keys still resolve. Interactive terminal sessions
    /// are unaffected — terminald already runs those as the terminal
    /// account inside the PTY.
    fn run_ssh(
        &self,
        account: &TerminalAccount,
        machine: &MachineTarget,
        argv: &[String],
        remote: &str,
    ) -> Result<crate::bounded_command::BoundedCommandOutput, String> {
        self.ensure_known_hosts(account)?;
        let mut args: Vec<String> = argv.iter().skip(1).cloned().collect();
        args.push(remote.to_owned());
        let home = match machine.auth_kind {
            MachineAuthKind::System => account.home.clone().unwrap_or_else(|| "/".to_owned()),
            _ => self.config.key_root.to_string_lossy().into_owned(),
        };
        let environment: [(&str, &str); 2] = [
            ("HOME", home.as_str()),
            (
                "PATH",
                "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            ),
        ];
        run_bounded_command(
            &self.config.timeout_binary,
            &self.config.ssh_binary,
            &args,
            SSH_PROBE_TIMEOUT,
            &environment,
            MAX_PROBE_OUTPUT_BYTES,
        )
    }

    pub(crate) fn probe(&self, machine: &MachineTarget) -> Result<MachineProbeResult, String> {
        Self::validate_target(machine)?;
        let reachable = tcp_open(&machine.host, machine.port, TCP_CONNECT_TIMEOUT);
        if !reachable {
            return Ok(MachineProbeResult::unreachable(format!(
                "{}:{} did not answer a TCP connection",
                machine.host, machine.port
            )));
        }
        if machine.auth_kind == MachineAuthKind::Password {
            return Ok(MachineProbeResult {
                status: "reachable",
                detail: "Port is open; sign in once from the terminal to verify credentials."
                    .to_owned(),
                ..MachineProbeResult::unreachable("")
            });
        }
        if machine.auth_kind == MachineAuthKind::Key {
            self.ensure_identity()?;
        }
        let account = self.terminal_account()?;
        let argv = self.ssh_argv(machine, true);
        let started = Instant::now();
        let output = self.run_ssh(&account, machine, &argv, PROBE_SCRIPT)?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() {
            return Ok(parse_probe(&stdout, latency_ms));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let (status, detail) = classify_ssh_failure(&stderr);
        Ok(MachineProbeResult {
            status,
            detail: detail.unwrap_or_else(|| {
                format!("SSH to {} exited with {}", machine.host, output.status)
            }),
            latency_ms: Some(latency_ms),
            ..MachineProbeResult::unreachable("")
        })
    }

    pub(crate) fn wake(&self, mac: &str) -> Result<String, String> {
        let bytes = parse_mac(mac)?;
        let mut packet = [0_u8; 102];
        packet[..6].fill(0xFF);
        for index in 0..16 {
            packet[6 + index * 6..6 + index * 6 + 6].copy_from_slice(&bytes);
        }
        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|_| "could not open a UDP socket for Wake-on-LAN".to_owned())?;
        socket
            .set_broadcast(true)
            .map_err(|_| "could not enable UDP broadcast for Wake-on-LAN".to_owned())?;
        for port in [9_u16, 7_u16] {
            socket
                .send_to(&packet, format!("255.255.255.255:{port}"))
                .map_err(|_| "the Wake-on-LAN packet could not be sent".to_owned())?;
        }
        Ok(format!("Wake-on-LAN packet sent to {mac}"))
    }

    pub(crate) fn power(
        &self,
        machine: &MachineTarget,
        action: MachinePowerAction,
    ) -> Result<String, String> {
        Self::validate_target(machine)?;
        if machine.auth_kind == MachineAuthKind::Password {
            return Err(
                "power actions need the hub key or your own SSH keys; open a terminal for password machines"
                    .to_owned(),
            );
        }
        if machine.auth_kind == MachineAuthKind::Key {
            self.ensure_identity()?;
        }
        let account = self.terminal_account()?;
        let verb = match action {
            MachinePowerAction::Reboot => "reboot",
            MachinePowerAction::PowerOff => "poweroff",
        };
        let remote = format!("systemctl {verb} 2>&1 || sudo -n systemctl {verb} 2>&1");
        let argv = self.ssh_argv(machine, true);
        let output = self.run_ssh(&account, machine, &argv, &remote)?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() {
            return Ok(format!("{verb} was sent to {}", machine.label));
        }
        if stderr.contains("password is required") || stdout.contains("password is required") {
            return Err(format!(
                "{} needs passwordless sudo on {} for power control",
                machine.username, machine.label
            ));
        }
        let (_, detail) = classify_ssh_failure(&stderr);
        Err(detail.unwrap_or_else(|| {
            let remote_detail = stdout.trim();
            if remote_detail.is_empty() {
                format!("the power action on {} failed", machine.label)
            } else {
                format!(
                    "{} refused the power action: {remote_detail}",
                    machine.label
                )
            }
        }))
    }
}

const PROBE_SCRIPT: &str = concat!(
    "echo __HX__;",
    "echo H=$(hostname 2>/dev/null || uname -n);",
    "echo K=$(uname -r);",
    "echo N=$(nproc 2>/dev/null || echo 0);",
    "sed -n 's/^PRETTY_NAME=//p' /etc/os-release 2>/dev/null | tr -d '\"' | sed 's/^/O=/';",
    "awk '{print \"U=\"int($1)}' /proc/uptime 2>/dev/null;",
    "awk '{print \"L=\"$1\" \"$2\" \"$3}' /proc/loadavg 2>/dev/null;",
    "awk '/^MemTotal:/{print \"MT=\"$2*1024}/^MemAvailable:/{print \"MA=\"$2*1024}' /proc/meminfo 2>/dev/null;",
    "df -B1 --output=size,avail / 2>/dev/null | awk 'NR==2{print \"D=\"$1\" \"$2}';",
    "command -v docker >/dev/null 2>&1 && docker ps -q 2>/dev/null | wc -l | sed 's/^/C=/';",
    "command -v systemctl >/dev/null 2>&1 && systemctl --failed --no-legend --no-pager 2>/dev/null | wc -l | sed 's/^/F=/';",
    "true"
);

fn parse_probe(stdout: &str, latency_ms: u64) -> MachineProbeResult {
    let mut result = MachineProbeResult {
        status: "online",
        detail: "SSH probe succeeded".to_owned(),
        latency_ms: Some(latency_ms),
        ..MachineProbeResult::unreachable("")
    };
    let Some((_, body)) = stdout.split_once("__HX__") else {
        return MachineProbeResult {
            status: "error",
            detail: "the remote probe returned unrecognizable output".to_owned(),
            latency_ms: Some(latency_ms),
            ..MachineProbeResult::unreachable("")
        };
    };
    let mut load = [0_f64; 3];
    let mut have_load = false;
    let mut disk = [0_u64; 2];
    let mut have_disk = false;
    for line in body.lines().take(64) {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "H" => result.hostname = bounded_string(value, 128),
            "K" => result.kernel = bounded_string(value, 128),
            "O" => result.os = bounded_string(value, 128),
            "N" => result.cpu_count = value.parse::<u32>().ok().filter(|count| *count > 0),
            "U" => result.uptime_seconds = value.parse::<u64>().ok(),
            "MT" => result.mem_total_bytes = value.parse::<u64>().ok(),
            "MA" => result.mem_available_bytes = value.parse::<u64>().ok(),
            "C" => result.containers_running = value.parse::<u32>().ok(),
            "F" => result.failed_units = value.parse::<u32>().ok(),
            "L" => {
                let parts: Vec<f64> = value
                    .split_whitespace()
                    .filter_map(|part| part.parse::<f64>().ok())
                    .collect();
                if parts.len() >= 3 && parts.iter().all(|part| part.is_finite() && *part >= 0.0) {
                    load = [parts[0], parts[1], parts[2]];
                    have_load = true;
                }
            }
            "D" => {
                let parts: Vec<u64> = value
                    .split_whitespace()
                    .filter_map(|part| part.parse::<u64>().ok())
                    .collect();
                if parts.len() >= 2 {
                    disk = [parts[0], parts[1]];
                    have_disk = true;
                }
            }
            _ => {}
        }
    }
    if have_load {
        result.load = Some(load);
    }
    if have_disk {
        result.disk_total_bytes = Some(disk[0]);
        result.disk_available_bytes = Some(disk[1]);
    }
    result
}

fn bounded_string(value: &str, maximum: usize) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > maximum
        || value.chars().any(|character| character.is_control())
    {
        return None;
    }
    Some(value.to_owned())
}

fn classify_ssh_failure(stderr: &str) -> (&'static str, Option<String>) {
    let lower = stderr.to_lowercase();
    let detail = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && line.len() <= 240)
        .map(str::to_owned);
    if lower.contains("permission denied") || lower.contains("authentication failed") {
        (
            "auth_failed",
            Some("SSH authentication was rejected; check the authorized key".to_owned()),
        )
    } else if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
    {
        (
            "host_key",
            Some(
                "the remote host key changed or is untrusted; clear it from the hub known_hosts"
                    .to_owned(),
            ),
        )
    } else if lower.contains("connection timed out")
        || lower.contains("connection refused")
        || lower.contains("no route to host")
        || lower.contains("could not resolve")
        || lower.contains("name or service not known")
    {
        ("unreachable", detail)
    } else {
        ("error", detail)
    }
}

fn tcp_open(host: &str, port: u16, timeout: Duration) -> bool {
    let mut candidates = match (host, port).to_socket_addrs() {
        Ok(candidates) => candidates,
        Err(_) => return false,
    };
    candidates.any(|address| TcpStream::connect_timeout(&address, timeout).is_ok())
}

fn parse_mac(value: &str) -> Result<[u8; 6], String> {
    let compact: Vec<u8> = value
        .bytes()
        .filter(|byte| *byte != b':' && *byte != b'-')
        .collect();
    if compact.len() != 12 || !compact.iter().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Wake-on-LAN needs a MAC address like aa:bb:cc:dd:ee:ff".to_owned());
    }
    let text = String::from_utf8_lossy(&compact);
    let mut bytes = [0_u8; 6];
    for (index, pair) in text.as_bytes().chunks(2).enumerate() {
        let text = std::str::from_utf8(pair).map_err(|_| "invalid MAC address".to_owned())?;
        bytes[index] =
            u8::from_str_radix(text, 16).map_err(|_| "invalid MAC address".to_owned())?;
    }
    Ok(bytes)
}

fn validate_host(host: &str) -> Result<(), String> {
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.is_ascii()
        && !host.starts_with('-')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
        && host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        });
    if valid {
        Ok(())
    } else {
        Err("machine host must be a DNS name or IP address".to_owned())
    }
}

fn validate_username(username: &str) -> Result<(), String> {
    let valid = !username.is_empty()
        && username.len() <= 64
        && username.is_ascii()
        && !username.starts_with('-')
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err("machine username must be a short login name".to_owned())
    }
}

/// Look up name + home for a uid from /etc/passwd so broker-run ssh can
/// resolve the right ~/.ssh for `system` auth.
fn passwd_entry_for_uid(uid: u32) -> (Option<String>, Option<String>) {
    let Ok(passwd) = fs::read_to_string("/etc/passwd") else {
        return (None, None);
    };
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() < 7 {
            continue;
        }
        if fields[2].parse::<u32>() == Ok(uid) {
            return (Some(fields[0].to_owned()), Some(fields[5].to_owned()));
        }
    }
    (None, None)
}

fn run_program(program: &Path, args: &[String], timeout_seconds: u64) -> Result<(), String> {
    let output = run_bounded_command(
        Path::new("/usr/bin/timeout"),
        program,
        args,
        Duration::from_secs(timeout_seconds),
        &[],
        MAX_PROBE_OUTPUT_BYTES,
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
