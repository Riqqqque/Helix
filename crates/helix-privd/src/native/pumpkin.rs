use super::*;

const RELEASES: &str = "https://api.github.com/repos/Pumpkin-MC/Pumpkin/releases?per_page=100";

pub(super) fn bedrock_ready(port: u16) -> bool {
    let timeout = Duration::from_millis(600);
    if TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), timeout).is_err() {
        return false;
    }
    let Ok(socket) = UdpSocket::bind("127.0.0.1:0") else {
        return false;
    };
    if socket.set_read_timeout(Some(timeout)).is_err()
        || socket.connect(("127.0.0.1", port)).is_err()
    {
        return false;
    }
    let mut ping = vec![1];
    ping.extend_from_slice(&now_unix_ms().to_be_bytes());
    ping.extend_from_slice(&[
        0, 255, 255, 0, 254, 254, 254, 254, 253, 253, 253, 253, 18, 52, 86, 120,
    ]);
    ping.extend_from_slice(&1_u64.to_be_bytes());
    let mut pong = [0_u8; 2048];
    socket.send(&ping).is_ok()
        && socket
            .recv(&mut pong)
            .is_ok_and(|size| size >= 35 && pong[0] == 0x1c)
}

pub(super) fn release_versions(tag: &str) -> Result<(&str, &str), String> {
    if tag.len() > 100
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        return Err("Invalid Pumpkin release tag".to_owned());
    }
    let (_, versions) = tag
        .split_once('+')
        .ok_or("Pumpkin release has no declared client versions")?;
    let (java, bedrock) = versions
        .split_once('-')
        .ok_or("Pumpkin release has no declared Bedrock version")?;
    validate_version(java)?;
    validate_version(bedrock)?;
    Ok((java, bedrock))
}

fn asset_name() -> Result<&'static str, String> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("pumpkin-X64-Linux-musl"),
        "aarch64" => Ok("pumpkin-ARM64-Linux-musl"),
        _ => Err("Pumpkin downloads support Linux x86-64 and ARM64 hosts".to_owned()),
    }
}

fn release_artifact(release: &Value, asset_name: &str) -> Result<Artifact, String> {
    if release["draft"] != false || release["prerelease"] != false {
        return Err(
            "Choose a published, versioned Pumpkin release, not a moving nightly".to_owned(),
        );
    }
    let tag = release["tag_name"]
        .as_str()
        .ok_or("Pumpkin release is missing its tag")?;
    let (java, _) = release_versions(tag)?;
    let asset = release["assets"]
        .as_array()
        .and_then(|assets| assets.iter().find(|a| a["name"] == asset_name))
        .ok_or("This Pumpkin release has no binary for the host architecture")?;
    let digest = asset["digest"]
        .as_str()
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| valid_hex(s, 64))
        .ok_or("Pumpkin has not published a SHA-256 digest for this binary")?;
    let size = asset["size"]
        .as_u64()
        .ok_or("Pumpkin binary size is missing")?;
    if !(1024 * 1024..=MAX_SERVER_JAR_BYTES).contains(&size) {
        return Err("Pumpkin binary size is outside the download limit".to_owned());
    }
    let url = asset["browser_download_url"]
        .as_str()
        .ok_or("Pumpkin binary URL is missing")?;
    let expected = format!(
        "https://github.com/Pumpkin-MC/Pumpkin/releases/download/{}/{}",
        tag.replace('+', "%2B"),
        asset_name
    );
    if url != expected && url != expected.replace("%2B", "+") {
        return Err("Pumpkin binary URL does not match its release and architecture".to_owned());
    }
    Ok(Artifact {
        software: MinecraftSoftware::Pumpkin,
        version: java.to_owned(),
        build: tag.to_owned(),
        java_version: 0,
        url: url.to_owned(),
        local_source: None,
        expected_hash: Some(ExpectedHash {
            algorithm: HashAlgorithm::Sha256,
            value: digest.to_owned(),
        }),
        install_server: false,
    })
}

impl NativeManager {
    pub(super) fn resolve_pumpkin_bedrock_port(
        &self,
        java: u16,
        requested: Option<u16>,
        manifests: &[InstanceManifest],
    ) -> Result<u16, String> {
        let mut used = self.used_game_ports(manifests);
        used.insert(java);
        let candidates = if let Some(port) = requested {
            vec![port]
        } else {
            policy_candidates(&self.read_game_port_policy(GameKind::Minecraft)?)?
        };
        candidates.into_iter().find(|port| *port >= 1024 && !used.contains(port) && ensure_port_available(*port, true).is_ok())
            .ok_or_else(|| "Pumpkin needs a second free TCP/UDP port for Bedrock NetherNet. Choose another Bedrock port or expand the Minecraft port pool.".to_owned())
    }
    fn pumpkin_catalog(&self) -> Result<Vec<Artifact>, String> {
        let name = asset_name()?;
        let response = self.fetch_json(RELEASES, &["api.github.com"])?;
        let releases = response
            .as_array()
            .ok_or("Pumpkin release catalog is invalid")?;
        let artifacts: Vec<_> = releases
            .iter()
            .filter_map(|release| release_artifact(release, name).ok())
            .collect();
        if artifacts.is_empty() {
            return Err(
                "No verified Pumpkin release is available for this host; try again later"
                    .to_owned(),
            );
        }
        Ok(artifacts)
    }

    pub(super) fn pumpkin_versions(&self) -> Result<Vec<String>, String> {
        Ok(self
            .pumpkin_catalog()?
            .into_iter()
            .map(|a| a.build)
            .collect())
    }

    pub(super) fn resolve_pumpkin(&self, requested: &str) -> Result<Artifact, String> {
        self.pumpkin_catalog()?
            .into_iter()
            .find(|a| requested.eq_ignore_ascii_case("latest") || a.build == requested)
            .ok_or_else(|| {
                "That Pumpkin release is unavailable; refresh the release list".to_owned()
            })
    }

    pub(super) fn download_pumpkin(
        &self,
        artifact: &Artifact,
        destination: &Path,
    ) -> Result<String, String> {
        // GitHub release downloads redirect to a signed asset URL. Never follow arbitrary redirects.
        require_https_host(&artifact.url, &["github.com"])?;
        if !artifact
            .url
            .starts_with("https://github.com/Pumpkin-MC/Pumpkin/releases/download/")
        {
            return Err("Unexpected Pumpkin download source".to_owned());
        }
        let headers = run_program(
            Path::new("/usr/bin/curl"),
            &[
                "--silent".to_owned(),
                "--show-error".to_owned(),
                "--fail".to_owned(),
                "--head".to_owned(),
                "--proto".to_owned(),
                "=https".to_owned(),
                "--max-time".to_owned(),
                "30".to_owned(),
                artifact.url.clone(),
            ],
            35,
        )?;
        let redirect = headers
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(key, _)| key.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value.trim())
            })
            .ok_or("GitHub did not provide a Pumpkin release download location")?;
        require_https_host(redirect, &["release-assets.githubusercontent.com"])?;
        let partial = destination.with_extension("pumpkin.partial");
        let result = (|| {
            self.curl_no_redirect(redirect, &partial, MAX_SERVER_JAR_BYTES, 600)?;
            let digest = file_sha256(&partial)?;
            if !artifact
                .expected_hash
                .as_ref()
                .is_some_and(|hash| digest.eq_ignore_ascii_case(&hash.value))
            {
                return Err("Pumpkin binary failed its publisher SHA-256 check".to_owned());
            }
            let mut header = [0u8; 20];
            File::open(&partial)
                .and_then(|mut f| f.read_exact(&mut header))
                .map_err(|_| "Pumpkin binary is incomplete")?;
            let machine = u16::from_le_bytes([header[18], header[19]]);
            let expected_machine = if std::env::consts::ARCH == "aarch64" {
                183
            } else {
                62
            };
            if &header[..4] != b"\x7fELF"
                || header[4] != 2
                || header[5] != 1
                || machine != expected_machine
            {
                return Err("Pumpkin binary is not a compatible 64-bit Linux executable".to_owned());
            }
            fs::set_permissions(&partial, fs::Permissions::from_mode(0o550))
                .map_err(|_| "Could not protect Pumpkin executable")?;
            fs::rename(&partial, destination)
                .map_err(|_| "Could not activate the verified Pumpkin binary")?;
            Ok(digest)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&partial);
        }
        result
    }

    pub(super) fn pumpkin_runtime(&self) -> Result<String, String> {
        let tag = "alpine:3.24";
        self.docker(["pull", tag], 600)?;
        let digest = self.docker(
            [
                "image",
                "inspect",
                "--format",
                "{{index .RepoDigests 0}}",
                tag,
            ],
            30,
        )?;
        let digest = digest.trim();
        if !digest.starts_with("alpine@sha256:")
            || !valid_hex(digest.trim_start_matches("alpine@sha256:"), 64)
        {
            return Err("Docker did not return a pinned Alpine runtime".to_owned());
        }
        Ok(digest.to_owned())
    }
}

fn set(config: &mut toml::Value, path: &[&str], value: toml::Value) -> Result<(), String> {
    let (key, rest) = path.split_first().ok_or("Empty Pumpkin setting path")?;
    let table = config
        .as_table_mut()
        .ok_or("Pumpkin setting section must be a TOML table")?;
    if rest.is_empty() {
        table.insert((*key).to_owned(), value);
    } else {
        set(
            table
                .entry((*key).to_owned())
                .or_insert_with(|| toml::Value::Table(Default::default())),
            rest,
            value,
        )?;
    }
    Ok(())
}

pub(super) fn initial_config(
    spec: &MinecraftCreateSpec,
    port: u16,
    bedrock_port: u16,
    rcon_port: u16,
    password: &str,
) -> Result<String, String> {
    let mut config = toml::Value::Table(Default::default());
    for edition in ["java", "bedrock"] {
        set(
            &mut config,
            &["networking", edition, "enabled"],
            true.into(),
        )?;
        set(
            &mut config,
            &["networking", edition, "online_mode"],
            true.into(),
        )?;
        set(
            &mut config,
            &["networking", edition, "motd"],
            spec.name.clone().into(),
        )?;
        set(
            &mut config,
            &["networking", edition, "max_players"],
            i64::from(spec.max_players).into(),
        )?;
        set(
            &mut config,
            &["networking", edition, "view_distance"],
            8.into(),
        )?;
        set(
            &mut config,
            &["networking", edition, "simulation_distance"],
            6.into(),
        )?;
    }
    set(
        &mut config,
        &["networking", "java", "address"],
        format!("0.0.0.0:{port}").into(),
    )?;
    set(
        &mut config,
        &["networking", "bedrock", "nethernet", "address"],
        format!("0.0.0.0:{bedrock_port}").into(),
    )?;
    set(
        &mut config,
        &["networking", "bedrock", "username_prefix"],
        ".".into(),
    )?;
    set(
        &mut config,
        &["networking", "query", "enabled"],
        false.into(),
    )?;
    set(&mut config, &["networking", "rcon", "enabled"], true.into())?;
    set(
        &mut config,
        &["networking", "rcon", "address"],
        format!("0.0.0.0:{rcon_port}").into(),
    )?;
    set(
        &mut config,
        &["networking", "rcon", "password"],
        password.into(),
    )?;
    set(&mut config, &["commands", "use_tty"], false.into())?;
    set(&mut config, &["commands", "use_console"], false.into())?;
    set(&mut config, &["logging", "color"], false.into())?;
    set(&mut config, &["world", "autosave_ticks"], 6000.into())?;
    toml::to_string_pretty(&config).map_err(|_| "Could not serialize Pumpkin settings".to_owned())
}

const FIELDS: &[(&str, &[&str])] = &[
    ("motd", &["networking", "java", "motd"]),
    ("max-players", &["networking", "java", "max_players"]),
    ("gamemode", &["default_gamemode"]),
    ("difficulty", &["default_difficulty"]),
    ("view-distance", &["networking", "java", "view_distance"]),
    (
        "simulation-distance",
        &["networking", "java", "simulation_distance"],
    ),
    ("online-mode", &["networking", "java", "online_mode"]),
    ("pvp", &["pvp", "enabled"]),
    ("white-list", &["white_list"]),
    ("enforce-whitelist", &["enforce_whitelist"]),
];

pub(super) fn properties(content: &str) -> Result<HashMap<String, String>, String> {
    let config: toml::Value = toml::from_str(content)
        .map_err(|_| "pumpkin.toml is invalid; correct it in Files before saving settings")?;
    let mut output = HashMap::new();
    for (name, path) in FIELDS {
        let value = path.iter().try_fold(&config, |node, key| node.get(key));
        if let Some(value) = value {
            let text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            output.insert(
                (*name).to_owned(),
                if ["gamemode", "difficulty"].contains(name) {
                    text.to_lowercase()
                } else {
                    text
                },
            );
        }
    }
    if let Some(port) = config
        .get("networking")
        .and_then(|n| n.get("java"))
        .and_then(|n| n.get("address"))
        .and_then(toml::Value::as_str)
        .and_then(|s| s.parse::<SocketAddr>().ok())
    {
        output.insert("server-port".to_owned(), port.port().to_string());
    }
    output.insert("spawn-protection".to_owned(), "0".to_owned());
    output.insert("player-idle-timeout".to_owned(), "0".to_owned());
    output.insert("allow-flight".to_owned(), "false".to_owned());
    Ok(output)
}

pub(super) fn update_config(
    content: &str,
    patch: &MinecraftSettingsPatch,
) -> Result<String, String> {
    if patch.player_idle_timeout != 0 || patch.allow_flight || patch.spawn_protection != 0 {
        return Err("Pumpkin does not expose idle kick, allow-flight, or spawn protection through this settings API; use compatible Pumpkin plugins".to_owned());
    }
    let mut config: toml::Value =
        toml::from_str(content).map_err(|_| "pumpkin.toml is invalid; nothing was changed")?;
    let values = parse_properties(&update_properties("", patch));
    for (name, path) in FIELDS {
        let value = values.get(*name).ok_or("Missing Pumpkin setting")?;
        let typed = if ["gamemode", "difficulty"].contains(name) {
            let mut title = value.clone();
            title[..1].make_ascii_uppercase();
            toml::Value::String(title)
        } else if *name == "motd" {
            toml::Value::String(patch.motd.clone())
        } else if let Ok(boolean) = value.parse::<bool>() {
            boolean.into()
        } else {
            value
                .parse::<i64>()
                .map_err(|_| "Invalid Pumpkin numeric setting")?
                .into()
        };
        set(&mut config, path, typed.clone())?;
        if path.len() == 3 && path[1] == "java" {
            set(&mut config, &["networking", "bedrock", path[2]], typed)?;
        }
    }
    set(
        &mut config,
        &["networking", "java", "address"],
        format!("0.0.0.0:{}", patch.game_port).into(),
    )?;
    toml::to_string_pretty(&config).map_err(|_| "Could not serialize Pumpkin settings".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_settings_preserve_custom_config_and_sync_java_bedrock() {
        let spec: MinecraftCreateSpec = serde_json::from_value(json!({
            "name":"Native test", "software":"pumpkin", "version":"latest", "memory_mb":1024,
            "max_players":4,"start_on_boot":false,"eula_accepted":true
        }))
        .unwrap();
        let original = initial_config(&spec, 25640, 25642, 25575, "test-only").unwrap();
        let original = format!("custom_value = 42\n{original}");
        let mut patch = MinecraftSettingsPatch {
            expected_revision: "a".repeat(64),
            motd: "Quoted \"MOTD\"".to_owned(),
            game_mode: MinecraftGameMode::Creative,
            difficulty: MinecraftDifficulty::Hard,
            max_players: 6,
            view_distance: 6,
            simulation_distance: 4,
            player_idle_timeout: 0,
            online_mode: true,
            pvp: false,
            allow_flight: false,
            white_list: true,
            enforce_white_list: true,
            spawn_protection: 0,
            game_port: 25641,
            memory_mb: 1024,
        };
        let changed = update_config(&original, &patch).unwrap();
        let config: toml::Value = toml::from_str(&changed).unwrap();
        assert_eq!(config["custom_value"].as_integer(), Some(42));
        assert_eq!(
            config["networking"]["rcon"]["password"].as_str(),
            Some("test-only")
        );
        assert_eq!(
            config["networking"]["java"]["address"].as_str(),
            Some("0.0.0.0:25641")
        );
        assert_eq!(
            config["networking"]["bedrock"]["nethernet"]["address"].as_str(),
            Some("0.0.0.0:25642")
        );
        assert_eq!(
            config["networking"]["query"]["enabled"].as_bool(),
            Some(false)
        );
        assert_eq!(properties(&changed).unwrap()["motd"], patch.motd);
        assert_eq!(properties(&changed).unwrap()["gamemode"], "creative");
        patch.allow_flight = true;
        assert!(update_config(&original, &patch).is_err());
        assert!(properties("[broken").is_err());
    }
    #[test]
    fn release_tags_require_both_client_versions_and_no_path_traversal() {
        assert_eq!(
            release_versions("0.1.0-dev+26.2-26.45").unwrap(),
            ("26.2", "26.45")
        );
        for tag in ["nightly", "../26.2", "0.1+26.2", "0.1+26.2-26.45?x"] {
            assert!(release_versions(tag).is_err());
        }
    }
    #[test]
    fn release_assets_require_exact_source_architecture_and_digest() {
        let mut release = json!({"draft":false,"prerelease":false,"tag_name":"0.1.0-dev+26.2-26.45","assets":[{
            "name":"pumpkin-X64-Linux-musl","size":127785312,"digest":format!("sha256:{}", "a".repeat(64)),
            "browser_download_url":"https://github.com/Pumpkin-MC/Pumpkin/releases/download/0.1.0-dev%2B26.2-26.45/pumpkin-X64-Linux-musl"}]});
        assert_eq!(
            release_artifact(&release, "pumpkin-X64-Linux-musl")
                .unwrap()
                .java_version,
            0
        );
        assert!(release_artifact(&release, "pumpkin-ARM64-Linux-musl").is_err());
        release["assets"][0]["digest"] = Value::Null;
        assert!(release_artifact(&release, "pumpkin-X64-Linux-musl").is_err());
    }
}
