use crate::{host::HostControl, native::NativeManager, network::NetworkManager};
use helix_privd::{GameKind, GamePortPolicySpec};
use serde_json::{Value, json};
use std::{
    fs,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn inventory(
    host: &HostControl,
    network: &NetworkManager,
    native: Option<&NativeManager>,
) -> Result<Value, String> {
    let start_on_boot = host
        .status()
        .ok()
        .and_then(|status| status.get("start_on_boot").cloned())
        .unwrap_or(Value::Null);
    let firewall = network
        .inventory(&[])
        .ok()
        .and_then(|inventory| inventory.get("firewall").cloned())
        .unwrap_or(Value::Null);
    let minecraft_forward = native.and_then(|manager| {
        manager
            .game_port_policy(GameKind::Minecraft)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/policy/auto_forward_on_create")
                    .and_then(Value::as_bool)
            })
    });
    let unattended = unattended_upgrades_state(host);
    let docker_live_restore = docker_live_restore(host);
    let fail2ban = unit_flag(host, "fail2ban.service");
    let timesync = unit_flag(host, "systemd-timesyncd.service");
    let ssh_password = ssh_directive("PasswordAuthentication");
    let ptrace = sysctl_flag(
        "/proc/sys/kernel/yama/ptrace_scope",
        &["1", "2", "3"],
        "ptrace_scope",
    );
    let kptr = sysctl_flag(
        "/proc/sys/kernel/kptr_restrict",
        &["1", "2"],
        "kptr_restrict",
    );
    let dmesg = sysctl_flag("/proc/sys/kernel/dmesg_restrict", &["1"], "dmesg_restrict");
    let rp_filter = sysctl_flag(
        "/proc/sys/net/ipv4/conf/all/rp_filter",
        &["1", "2"],
        "rp_filter",
    );
    let userns = sysctl_flag(
        "/proc/sys/kernel/unprivileged_userns_clone",
        &["1"],
        "unprivileged_userns_clone",
    );
    Ok(json!({
        "schema_version": 1,
        "tips": [
            {
                "id": "ssh_keys",
                "title": "Prefer SSH keys over passwords",
                "body": "Password SSH is a brute-force target. Key-only login, a non-default port only if you already understand the trade, and Fail2ban if SSH is reachable from more than your LAN."
            },
            {
                "id": "keep_ufw",
                "title": "Keep a host firewall on",
                "body": "UFW is the Ubuntu default. Enable it from Network with the SSH-safety flow. Helix will not turn it off from this page. Game ports should be explicit allow rules, not a disabled firewall."
            },
            {
                "id": "ntp",
                "title": "Keep the clock honest",
                "body": "TLS, logs, and SteamCMD all assume a sane clock. systemd-timesyncd or chrony should stay enabled on a game host."
            },
            {
                "id": "updates",
                "title": "Apply security updates",
                "body": "unattended-upgrades covers distro security patches without a full Host Updates session. It does not reboot for you. Kernel updates still need a planned restart."
            },
            {
                "id": "docker_isolation",
                "title": "Leave game servers in containers",
                "body": "Do not install Wine, SteamCMD, Java, or dedicated servers on the host OS. Helix already isolates those runtimes. Host packages and game processes sharing one user is a wider blast radius."
            },
            {
                "id": "lan_only",
                "title": "Keep the dashboard on the LAN",
                "body": "Helix is a private console. Publishing it, putting a generic reverse proxy in front, or trusting X-Forwarded-For is a different product. Tailscale is the supported remote path when you need one."
            },
            {
                "id": "backups",
                "title": "Helix is not the only copy",
                "body": "Worlds, AMP instances, and Plex libraries need copies Helix does not own. Recoverable trash is not an off-host backup."
            },
            {
                "id": "sysctl_persist",
                "title": "Live sysctl is not persistence",
                "body": "Helix reports kernel knobs from /proc. Changing them yourself only survives reboot if you also write sysctl.d. Do not disable unprivileged user namespaces to “harden Docker”; that breaks rootless and some Engine setups."
            }
        ],
        "controls": [
            control(
                "csrf_and_sessions",
                "Helix session and CSRF proofs",
                "Every signed-in request needs the session cookie plus a matching CSRF token. That pairing is compiled in; it is not a dashboard switch.",
                "Turn this off only if you want a weaker login surface. Helix does not offer that.",
                "always_on",
                true,
                false,
                true,
                "Session cookies are HttpOnly, SameSite=Strict, and bound to the current login. CSRF tokens rotate and are compared as exact proofs. Disabling that would let a browser on the LAN replay or forge dashboard actions.",
                None,
            ),
            control(
                "lan_bind",
                "Private LAN bind",
                "The dashboard listens on the configured private address, not on the public internet.",
                "A public bind would expose owner login to the wider network. Helix keeps that out of scope.",
                "always_on",
                true,
                false,
                true,
                "Helix is a private-alpha LAN console. Publishing it, putting it behind a generic reverse proxy, or trusting forwarded headers is a different product with a different review.",
                None,
            ),
            control(
                "helix_start_on_boot",
                "Start Helix after host boot",
                "Sets the Docker restart policy on the exact dashboard and gateway containers so Helix comes back after a reboot.",
                "Turn this off if you want the host to boot without bringing the dashboard back until you start those containers yourself.",
                if start_on_boot.get("enabled").and_then(Value::as_bool) == Some(true) { "enabled" } else if start_on_boot.get("enabled").and_then(Value::as_bool) == Some(false) { "disabled" } else { "mixed" },
                start_on_boot.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                true,
                true,
                "This does not start or stop the running dashboard now. Recreating those containers from Compose can reapply the compose restart policy, so Helix re-reads both policies whenever status is requested.",
                Some("start helix after boot"),
            ),
            control(
                "minecraft_auto_forward",
                "Forward Minecraft ports when creating a server",
                "When on, a newly created Minecraft server also requests the exact UPnP mapping and matching UFW rule for its game port, if the router and firewall already allow that flow.",
                "Keep this off for LAN-only servers. Turn it on only if you intentionally want new Minecraft servers reachable from outside.",
                if minecraft_forward == Some(true) { "enabled" } else if minecraft_forward == Some(false) { "disabled" } else { "unavailable" },
                minecraft_forward.unwrap_or(false),
                native.is_some(),
                false,
                "This never enables UFW by itself and never claims the port is reachable from the public internet. V Rising stays private in this Helix release. Existing servers keep the exposure they already have.",
                Some("forward minecraft ports on create"),
            ),
            control(
                "unattended_upgrades",
                "Automatic security updates",
                "Uses the distro unattended-upgrades service when it is already installed. Helix only enables or disables that exact unit.",
                "Turn it off before a maintenance window if an automatic package change would surprise you. Leave it on if you want security updates applied without opening Host updates every time.",
                unattended.state,
                unattended.enabled,
                unattended.available,
                true,
                "Helix does not install unattended-upgrades, change its config, or reboot after updates. If the unit is missing, install it from your distribution and return here.",
                Some("enable unattended-upgrades"),
            ),
            control(
                "fail2ban",
                "Fail2ban",
                "Bans repeat offenders on SSH and other jails when the fail2ban unit is already installed.",
                "Turn it off only while debugging a lockout. Leave it on if SSH or game panels are reachable beyond a single trusted machine.",
                fail2ban.state,
                fail2ban.enabled,
                fail2ban.available,
                true,
                "Helix only enables or disables fail2ban.service. It does not install the package, rewrite jail.local, or open extra ports.",
                Some("enable fail2ban"),
            ),
            control(
                "time_sync",
                "systemd-timesyncd",
                "Keeps this host’s clock aligned through the distro NTP client when that unit exists.",
                "Turn it off only if another NTP client such as chrony already owns time on this host.",
                timesync.state,
                timesync.enabled,
                timesync.available,
                true,
                "Helix does not install chrony or change NTP pools. A drifting clock breaks TLS, SteamCMD, and log correlation.",
                Some("enable systemd-timesyncd"),
            ),
            control(
                "ufw_active",
                "Host firewall (UFW)",
                "Reports whether Ubuntu’s Uncomplicated Firewall is installed and active.",
                "Leave UFW inactive only on a host that already has another honest firewall story. Enabling it is a confirmed Network flow that first preserves SSH.",
                if firewall.get("active").and_then(Value::as_bool) == Some(true) { "enabled" } else if firewall.get("installed").and_then(Value::as_bool) == Some(true) { "disabled" } else { "unavailable" },
                firewall.get("active").and_then(Value::as_bool).unwrap_or(false),
                false,
                true,
                "Helix will not disable UFW from this page. Enable it from Network with the SSH-safety confirmation. Helix-owned game rules are UUID-commented allow rules, not a general firewall editor.",
                None,
            ),
            control(
                "docker_live_restore",
                "Docker live restore",
                "When live-restore is on, running containers keep running across a Docker daemon restart.",
                "Turning this off means a Docker restart takes game servers and Helix containers down with the daemon.",
                docker_live_restore.state,
                docker_live_restore.enabled,
                false,
                true,
                "Changing live-restore requires Docker daemon configuration Helix does not rewrite. Edit daemon.json on the host if you need this, then restart Docker in a planned window.",
                None,
            ),
            control(
                "apparmor",
                "AppArmor",
                "Linux mandatory access control for confined services, when the kernel module is loaded.",
                "Disable AppArmor only if you are debugging a confinement issue and understand the wider host policy. Helix does not change LSM state.",
                apparmor_state().state,
                apparmor_state().enabled,
                false,
                true,
                "This is a kernel/LSM fact, not a Helix switch. A host without AppArmor is still usable; it just has one less confinement layer.",
                None,
            ),
            control(
                "ssh_root_login",
                "SSH root login",
                "Reads PermitRootLogin from the primary sshd_config. Included drop-in files are not parsed.",
                "Allowing password root login makes a stolen password a host-takeover. Key-only or prohibit-password is the usual hardening.",
                ssh_permit_root().state,
                ssh_permit_root().permissive,
                false,
                false,
                "Helix does not rewrite sshd_config. Change SSH in the file or with the tools you already use, then reload sshd yourself.",
                None,
            ),
            control(
                "kernel_aslr",
                "Kernel address space layout randomization",
                "Reads /proc/sys/kernel/randomize_va_space. 2 is the usual full randomization.",
                "Lowering this is a debugging choice. Leave it at 2 on a machine that runs game servers and a dashboard.",
                kernel_aslr().state,
                kernel_aslr().recommended,
                false,
                true,
                "Helix does not write sysctl. A live change would not survive reboot unless you also persist it in sysctl.d.",
                None,
            ),
            control(
                "kernel_ptrace",
                "Yama ptrace scope",
                "Reads /proc/sys/kernel/yama/ptrace_scope. 1 or higher stops casual process tracing.",
                "Lowering this is a debugging choice. Leave it at 1+ on a machine that holds owner sessions and game data.",
                ptrace.state,
                ptrace.recommended,
                false,
                ptrace.recommended,
                "Helix does not write this sysctl. 0 lets any process in the same user ptrace another, which is convenient for gdb and worse for a shared host.",
                None,
            ),
            control(
                "kernel_kptr",
                "Kernel pointer restriction",
                "Reads /proc/sys/kernel/kptr_restrict. 1 or 2 hides kernel addresses from unprivileged readers.",
                "Set this lower only when a diagnostic tool needs raw kallsyms and you accept the information leak.",
                kptr.state,
                kptr.recommended,
                false,
                kptr.recommended,
                "This is an observed kernel fact. Persist a change in sysctl.d if you change it on the host.",
                None,
            ),
            control(
                "kernel_dmesg",
                "dmesg restriction",
                "Reads /proc/sys/kernel/dmesg_restrict. 1 keeps kernel logs away from unprivileged users.",
                "Opening dmesg to every login is a debugging shortcut, not a game-host default.",
                dmesg.state,
                dmesg.recommended,
                false,
                dmesg.recommended,
                "Kernel ring buffer often includes hardware and network details. Helix does not rewrite this knob.",
                None,
            ),
            control(
                "net_rp_filter",
                "Reverse-path filtering",
                "Reads /proc/sys/net/ipv4/conf/all/rp_filter. 1 or 2 drops spoofed source addresses on this host.",
                "Disable this only if you already understand asymmetric routing on this box. It is not a LAN game-server default to leave off.",
                rp_filter.state,
                rp_filter.recommended,
                false,
                rp_filter.recommended,
                "This is one anti-spoofing layer. It does not replace UFW or router filters.",
                None,
            ),
            control(
                "unprivileged_userns",
                "Unprivileged user namespaces",
                "Reads unprivileged_userns_clone when the kernel exposes it. Docker and some sandboxes need this on.",
                "Do not flip this off to “harden Docker.” That breaks Engine and rootless setups Helix relies on.",
                userns.state,
                userns.enabled,
                false,
                false,
                "This is observed only. Helix will not disable user namespaces from the dashboard.",
                None,
            ),
            control(
                "ssh_password_auth",
                "SSH password authentication",
                "Reads PasswordAuthentication from the primary sshd_config. Drop-in files are not parsed.",
                "Password SSH is a brute-force surface. Prefer keys. Helix does not rewrite sshd.",
                ssh_password.state,
                !ssh_password.permissive,
                false,
                true,
                "A yes here means the daemon still accepts passwords if the rest of sshd agrees. Change the file and reload sshd yourself.",
                None,
            ),
            control(
                "typed_broker",
                "Typed privileged broker",
                "Host changes go through helix-privd as exact operations. There is no general root shell API.",
                "A general shell endpoint would let the dashboard run arbitrary root commands. Helix refuses that on purpose.",
                "always_on",
                true,
                false,
                true,
                "Hooks, Docker actions, package updates, and firewall edits are allowlisted and verified. If a browser asks for a command string, the broker rejects it.",
                None,
            ),
        ],
        "facts": {
            "kernel": kernel_release(),
            "apparmor": apparmor_state().detail,
            "ssh_permit_root": ssh_permit_root().detail,
            "aslr": kernel_aslr().detail,
            "docker_live_restore": docker_live_restore.detail,
            "unattended_upgrades": unattended.detail,
            "fail2ban": fail2ban.detail,
            "timesyncd": timesync.detail,
            "ssh_password_auth": ssh_password.detail,
            "ptrace_scope": ptrace.detail,
            "kptr_restrict": kptr.detail,
            "dmesg_restrict": dmesg.detail,
            "rp_filter": rp_filter.detail,
            "ufw": firewall.get("status").cloned().unwrap_or(Value::Null)
        },
        "collected_at_unix_ms": now_unix_ms()
    }))
}

pub fn set_control(
    host: &HostControl,
    native: Option<&NativeManager>,
    id: &str,
    enabled: bool,
    confirmation: &str,
) -> Result<Value, String> {
    if !valid_control_id(id) {
        return Err("the security control is invalid".to_owned());
    }
    match id {
        "helix_start_on_boot" => {
            require_confirmation(
                confirmation,
                if enabled {
                    "start helix after boot"
                } else {
                    "do not start helix after boot"
                },
            )?;
            host.set_start_on_boot(enabled)
        }
        "minecraft_auto_forward" => {
            require_confirmation(
                confirmation,
                if enabled {
                    "forward minecraft ports on create"
                } else {
                    "keep minecraft private on create"
                },
            )?;
            let native =
                native.ok_or_else(|| "the Helix server manager is not configured".to_owned())?;
            let current = native.game_port_policy(GameKind::Minecraft)?;
            let mut spec: GamePortPolicySpec = serde_json::from_value(
                current
                    .get("policy")
                    .cloned()
                    .ok_or_else(|| "Minecraft port policy was missing".to_owned())?,
            )
            .map_err(|_| "Minecraft port policy was invalid".to_owned())?;
            spec.auto_forward_on_create = enabled;
            native.set_game_port_policy(spec)
        }
        "unattended_upgrades" => {
            require_confirmation(
                confirmation,
                if enabled {
                    "enable unattended-upgrades"
                } else {
                    "disable unattended-upgrades"
                },
            )?;
            set_unattended_upgrades(host, enabled)
        }
        "fail2ban" => {
            require_confirmation(
                confirmation,
                if enabled {
                    "enable fail2ban"
                } else {
                    "disable fail2ban"
                },
            )?;
            set_named_unit(host, "fail2ban", "fail2ban.service", enabled)
        }
        "time_sync" => {
            require_confirmation(
                confirmation,
                if enabled {
                    "enable systemd-timesyncd"
                } else {
                    "disable systemd-timesyncd"
                },
            )?;
            set_named_unit(host, "time_sync", "systemd-timesyncd.service", enabled)
        }
        _ => Err("that security control cannot be changed from Helix".to_owned()),
    }
}

fn control(
    id: &str,
    title: &str,
    summary: &str,
    off_reason: &str,
    state: &str,
    enabled: bool,
    writable: bool,
    recommended: bool,
    implications: &str,
    confirmation_enable: Option<&str>,
) -> Value {
    json!({
        "id": id,
        "title": title,
        "summary": summary,
        "off_reason": off_reason,
        "state": state,
        "enabled": enabled,
        "writable": writable,
        "recommended": recommended,
        "implications": implications,
        "confirmation_enable": confirmation_enable,
        "confirmation_disable": match id {
            "helix_start_on_boot" => Value::String("do not start helix after boot".to_owned()),
            "minecraft_auto_forward" => Value::String("keep minecraft private on create".to_owned()),
            "unattended_upgrades" => Value::String("disable unattended-upgrades".to_owned()),
            "fail2ban" => Value::String("disable fail2ban".to_owned()),
            "time_sync" => Value::String("disable systemd-timesyncd".to_owned()),
            _ => Value::Null
        }
    })
}

struct FlagState {
    state: &'static str,
    enabled: bool,
    available: bool,
    recommended: bool,
    permissive: bool,
    detail: String,
}

fn unattended_upgrades_state(host: &HostControl) -> FlagState {
    match host.systemctl_state("is-enabled", "unattended-upgrades.service") {
        Ok(state) if state == "not-found" => FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: "unattended-upgrades.service is not installed".to_owned(),
        },
        Ok(state) => {
            let enabled = matches!(
                state.as_str(),
                "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
            );
            FlagState {
                state: if enabled { "enabled" } else { "disabled" },
                enabled,
                available: true,
                recommended: true,
                permissive: false,
                detail: format!("systemd is-enabled={state}"),
            }
        }
        Err(error) => FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: error,
        },
    }
}

fn set_unattended_upgrades(host: &HostControl, enabled: bool) -> Result<Value, String> {
    set_named_unit(
        host,
        "unattended_upgrades",
        "unattended-upgrades.service",
        enabled,
    )
}

fn set_named_unit(
    host: &HostControl,
    id: &str,
    unit: &str,
    enabled: bool,
) -> Result<Value, String> {
    let current = host.systemctl_state("is-enabled", unit)?;
    if current == "not-found" {
        return Err(format!("{unit} is not installed on this host"));
    }
    let verb = if enabled { "enable" } else { "disable" };
    let output = host.runner.run(
        &host.config.systemctl_binary,
        &[verb.to_owned(), unit.to_owned()],
        Duration::from_secs(20),
    )?;
    crate::host::require_success(output)?;
    let after = host.systemctl_state("is-enabled", unit)?;
    let now_enabled = matches!(
        after.as_str(),
        "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
    );
    if now_enabled != enabled {
        return Err(format!("systemd did not reach the requested {unit} state"));
    }
    Ok(json!({
        "id": id,
        "enabled": now_enabled,
        "enabled_state": after,
        "verified": true,
        "updated_at_unix_ms": now_unix_ms()
    }))
}

fn unit_flag(host: &HostControl, unit: &str) -> FlagState {
    match host.systemctl_state("is-enabled", unit) {
        Ok(state) if state == "not-found" => FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: format!("{unit} is not installed"),
        },
        Ok(state) => {
            let enabled = matches!(
                state.as_str(),
                "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
            );
            FlagState {
                state: if enabled { "enabled" } else { "disabled" },
                enabled,
                available: true,
                recommended: true,
                permissive: false,
                detail: format!("systemd is-enabled={state}"),
            }
        }
        Err(error) => FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: error,
        },
    }
}

fn sysctl_flag(path: &str, recommended_values: &[&str], label: &str) -> FlagState {
    let text = fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().chars().take(16).collect::<String>())
        .unwrap_or_default();
    let recommended = recommended_values.iter().any(|value| *value == text);
    FlagState {
        state: if text.is_empty() {
            "unavailable"
        } else if recommended {
            "hardened"
        } else {
            "relaxed"
        },
        enabled: recommended,
        available: false,
        recommended,
        permissive: !text.is_empty() && !recommended,
        detail: if text.is_empty() {
            format!("{label} was not readable")
        } else {
            text
        },
    }
}

fn ssh_directive(key_name: &str) -> FlagState {
    let text = fs::read_to_string("/etc/ssh/sshd_config").unwrap_or_default();
    if text.len() > 256 * 1024 {
        return FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: false,
            permissive: false,
            detail: "sshd_config is too large to parse".to_owned(),
        };
    }
    let mut found = String::from("default");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if key.eq_ignore_ascii_case(key_name) {
            found = rest.trim().chars().take(32).collect();
        }
    }
    let lower = found.to_ascii_lowercase();
    let permissive = matches!(lower.as_str(), "yes" | "default");
    let state = match lower.as_str() {
        "yes" => "yes",
        "no" => "no",
        "default" => "default",
        _ => "other",
    };
    FlagState {
        state,
        enabled: permissive,
        available: false,
        recommended: !permissive,
        permissive,
        detail: found,
    }
}

fn docker_live_restore(host: &HostControl) -> FlagState {
    match host.runner.run(
        &host.config.docker_binary,
        &[
            "info".to_owned(),
            "--format".to_owned(),
            "{{.LiveRestoreEnabled}}".to_owned(),
        ],
        Duration::from_secs(15),
    ) {
        Ok(output) if output.success => {
            let text = crate::host::first_line(&output.stdout)
                .unwrap_or("")
                .to_ascii_lowercase();
            let enabled = text == "true";
            FlagState {
                state: if enabled { "enabled" } else { "disabled" },
                enabled,
                available: false,
                recommended: true,
                permissive: false,
                detail: text,
            }
        }
        Ok(_) | Err(_) => FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: "Docker did not report live-restore".to_owned(),
        },
    }
}

fn apparmor_state() -> FlagState {
    let text = fs::read_to_string("/sys/module/apparmor/parameters/enabled")
        .ok()
        .map(|value| value.trim().to_owned());
    match text.as_deref() {
        Some("Y") => FlagState {
            state: "enabled",
            enabled: true,
            available: false,
            recommended: true,
            permissive: false,
            detail: "Y".to_owned(),
        },
        Some("N") => FlagState {
            state: "disabled",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: "N".to_owned(),
        },
        _ => FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: true,
            permissive: false,
            detail: "AppArmor module state was not readable".to_owned(),
        },
    }
}

fn ssh_permit_root() -> FlagState {
    let text = fs::read_to_string("/etc/ssh/sshd_config").unwrap_or_default();
    if text.len() > 256 * 1024 {
        return FlagState {
            state: "unavailable",
            enabled: false,
            available: false,
            recommended: false,
            permissive: false,
            detail: "sshd_config is too large to parse".to_owned(),
        };
    }
    let mut found = String::from("default");
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if key.eq_ignore_ascii_case("PermitRootLogin") {
            found = rest.trim().chars().take(32).collect();
        }
    }
    let (state, permissive, enabled) = match found.to_ascii_lowercase().as_str() {
        "yes" => ("yes", true, true),
        "without-password" => ("without-password", true, true),
        "prohibit-password" => ("prohibit-password", false, true),
        "forced-commands-only" => ("forced-commands-only", true, true),
        "no" => ("no", false, false),
        "default" => ("default", false, true),
        _ => ("other", true, true),
    };
    FlagState {
        state,
        enabled,
        available: false,
        recommended: false,
        permissive,
        detail: found,
    }
}

fn kernel_aslr() -> FlagState {
    let text = fs::read_to_string("/proc/sys/kernel/randomize_va_space")
        .ok()
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    FlagState {
        state: if text.is_empty() {
            "unavailable"
        } else {
            "observed"
        },
        enabled: text == "2",
        available: false,
        recommended: text == "2",
        permissive: text != "2",
        detail: if text.is_empty() {
            "randomize_va_space was not readable".to_owned()
        } else {
            text
        },
    }
}

fn kernel_release() -> Value {
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|value| value.trim().chars().take(64).collect::<String>())
        .filter(|value| !value.is_empty())
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn require_confirmation(actual: &str, expected: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("type `{expected}` exactly to confirm this change"))
    }
}

fn valid_control_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (2..=64).contains(&id.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn now_unix_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_ids_are_bounded_snake_case() {
        assert!(valid_control_id("helix_start_on_boot"));
        assert!(!valid_control_id("HELIX"));
        assert!(!valid_control_id("rm -rf /"));
        assert!(!valid_control_id(""));
    }
}
