use std::{
    io::Read as _,
    path::Path,
    process::{Command, ExitStatus, Stdio},
    thread,
    time::Duration,
};

pub(crate) struct BoundedCommandOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

pub(crate) fn run_bounded_command(
    timeout_binary: &Path,
    program: &Path,
    args: &[String],
    timeout: Duration,
    environment: &[(&str, &str)],
    maximum_bytes: usize,
) -> Result<BoundedCommandOutput, String> {
    let mut command = Command::new(timeout_binary);
    command
        .arg("--signal=TERM")
        .arg("--kill-after=2s")
        .arg(format!("{}s", timeout.as_secs().max(1)))
        .arg(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in environment {
        command.env(name, value);
    }
    let mut child = command
        .spawn()
        .map_err(|_| format!("could not run {}", program.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("could not read {} output", program.display()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("could not read {} output", program.display()))?;
    let stream_limit = u64::try_from(maximum_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let streams = thread::scope(|scope| {
        let stderr_worker = scope.spawn(move || {
            let mut bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
            stderr
                .take(stream_limit)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let mut stdout_bytes = Vec::with_capacity(maximum_bytes.min(64 * 1024));
        let stdout_result = stdout
            .take(stream_limit)
            .read_to_end(&mut stdout_bytes)
            .map(|_| stdout_bytes);
        let stderr_result = stderr_worker
            .join()
            .map_err(|_| std::io::Error::other("command output reader failed"))?;
        Ok::<_, std::io::Error>((stdout_result?, stderr_result?))
    });
    let (stdout, stderr) = match streams {
        Ok(streams) => streams,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("could not read {} output", program.display()));
        }
    };
    let status = child
        .wait()
        .map_err(|_| format!("could not wait for {}", program.display()))?;
    if stdout.len() > maximum_bytes
        || stderr.len() > maximum_bytes
        || stdout.len().saturating_add(stderr.len()) > maximum_bytes
    {
        return Err(format!("{} returned too much output", program.display()));
    }
    Ok(BoundedCommandOutput {
        status,
        stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_output_is_rejected_at_the_streaming_boundary() {
        let args = vec!["0123456789".to_owned()];
        let error = run_bounded_command(
            Path::new("/usr/bin/timeout"),
            Path::new("/usr/bin/printf"),
            &args,
            Duration::from_secs(2),
            &[],
            8,
        )
        .err()
        .expect("oversized output must be rejected");
        assert!(error.contains("too much output"));
    }
}
