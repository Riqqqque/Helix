use std::{fs, path::Path, process::Command};

fn helixctl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_helixctl"))
}

fn output_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\r', "")
}

#[test]
fn scaffold_and_check_do_not_require_a_helix_installation() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let destination = temp.path().join("my status card");
    let missing_config = temp.path().join("missing-helix.toml");

    let created = helixctl()
        .env("HELIX_CONFIG", &missing_config)
        .args([
            "strand",
            "new",
            "status-card",
            "--name",
            "Status Card",
            "--publisher",
            "Example author",
            "--kind",
            "ui-only",
            "--output",
        ])
        .arg(&destination)
        .output()
        .expect("run Strand scaffold");
    assert!(
        created.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        output_text(&created.stdout),
        output_text(&created.stderr)
    );
    assert!(destination.join("strand.toml").is_file());
    assert!(output_text(&created.stdout).contains("cannot install or run Strands yet"));

    let checked = helixctl()
        .env("HELIX_CONFIG", missing_config)
        .args(["strand", "check"])
        .arg(&destination)
        .output()
        .expect("run Strand check");
    assert!(
        checked.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        output_text(&checked.stdout),
        output_text(&checked.stderr)
    );
    let stdout = output_text(&checked.stdout);
    assert!(stdout.contains("Strand manifest is valid"));
    assert!(stdout.contains("Capabilities: none (deny by default)"));
    assert!(stdout.contains("Execution status: not installable or runnable"));

    let rejected_options = helixctl()
        .arg("--data-dir")
        .arg(temp.path())
        .args(["strand", "check"])
        .arg(&destination)
        .output()
        .expect("reject irrelevant administrative options");
    assert!(!rejected_options.status.success());
    assert!(
        output_text(&rejected_options.stderr)
            .contains("Strand project commands do not use --config or --data-dir")
    );
}

#[test]
fn scaffold_never_overwrites_an_existing_destination() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let destination = temp.path().join("existing");
    fs::create_dir(&destination).expect("create existing destination");
    let marker = destination.join("keep.txt");
    fs::write(&marker, "keep me").expect("write marker");

    let output = new_strand_at(&destination);
    assert!(!output.status.success());
    assert!(output_text(&output.stderr).contains("never overwrites files"));
    assert_eq!(fs::read_to_string(marker).expect("read marker"), "keep me");
}

#[test]
fn malformed_manifest_fails_with_an_actionable_error() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let destination = temp.path().join("invalid");
    let created = new_strand_at(&destination);
    assert!(created.status.success());
    let manifest = destination.join("strand.toml");
    let mut text = fs::read_to_string(&manifest).expect("read manifest");
    text.push_str("\nroot_shell = true\n");
    fs::write(&manifest, text).expect("damage manifest");

    let output = helixctl()
        .args(["strands", "validate"])
        .arg(&destination)
        .output()
        .expect("check malformed Strand");
    assert!(!output.status.success());
    let stderr = output_text(&output.stderr);
    assert!(stderr.contains("could not parse Strand manifest"));
    assert!(stderr.contains("unknown field"));
}

fn new_strand_at(destination: &Path) -> std::process::Output {
    helixctl()
        .args(["strand", "new", "existing", "--output"])
        .arg(destination)
        .output()
        .expect("run Strand scaffold")
}
