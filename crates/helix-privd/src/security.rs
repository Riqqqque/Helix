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
    Ok(json!({
        "schema_version": 1,
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
    let current = host.systemctl_state("is-enabled", "unattended-upgrades.service")?;
    if current == "not-found" {
        return Err("unattended-upgrades is not installed on this host".to_owned());
    }
    let verb = if enabled { "enable" } else { "disable" };
    let output = host.runner.run(
        &host.config.systemctl_binary,
        &[verb.to_owned(), "unattended-upgrades.service".to_owned()],
        Duration::from_secs(20),
    )?;
    crate::host::require_success(output)?;
    let after = host.systemctl_state("is-enabled", "unattended-upgrades.service")?;
    let now_enabled = matches!(
        after.as_str(),
        "enabled" | "enabled-runtime" | "linked" | "linked-runtime" | "alias"
    );
    if now_enabled != enabled {
        return Err("systemd did not reach the requested unattended-upgrades state".to_owned());
    }
    Ok(json!({
        "id": "unattended_upgrades",
        "enabled": now_enabled,
        "enabled_state": after,
        "verified": true,
        "updated_at_unix_ms": now_unix_ms()
    }))
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
