#[cfg(target_os = "linux")]
mod linux_daemon {
    use clap::Parser;
    use helix_terminal::{
        ExitResponse, Frame, OpenRequest, PROTOCOL_VERSION, ReadyResponse, TerminalDimensions,
        decode_frame_length, decode_json, decode_resize, encode_frame, encode_json, kind,
    };
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::{
        error::Error,
        fs,
        io::{self, Read as _, Write as _},
        os::unix::{
            fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _},
            net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        thread,
    };
    use tracing::{info, warn};
    use tracing_subscriber::EnvFilter;

    type DynError = Box<dyn Error + Send + Sync>;
    const MAX_CONCURRENT_TERMINALS: usize = 2;
    const OUTPUT_CHUNK_BYTES: usize = 16 * 1024;
    const PATH_VALUE: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

    #[derive(Debug, Parser)]
    #[command(
        name = "helix-terminald",
        version,
        about = "Helix unprivileged PTY bridge"
    )]
    struct Args {
        #[arg(long, value_name = "PATH")]
        socket: PathBuf,
        #[arg(long, value_name = "PATH")]
        shell: PathBuf,
        /// Start shells here. Defaults to the service process working directory.
        #[arg(long, value_name = "PATH")]
        working_directory: Option<PathBuf>,
        #[arg(long, value_name = "NAME")]
        user: String,
        #[arg(long, value_name = "UID")]
        allowed_peer_uid: u32,
    }

    #[derive(Clone)]
    struct SessionConfig {
        shell: PathBuf,
        working_directory: PathBuf,
        user: String,
    }

    struct SessionSlot<'a>(&'a AtomicUsize);

    impl Drop for SessionSlot<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub fn run() -> Result<(), DynError> {
        init_tracing()?;
        let args = Args::parse();
        let working_directory = args
            .working_directory
            .clone()
            .map_or_else(std::env::current_dir, Ok)?;
        validate_args(&args, &working_directory)?;
        prepare_socket_path(&args.socket)?;
        let listener = UnixListener::bind(&args.socket)?;
        fs::set_permissions(&args.socket, fs::Permissions::from_mode(0o660))?;
        let config = Arc::new(SessionConfig {
            shell: args.shell,
            working_directory,
            user: args.user,
        });
        let active = Arc::new(AtomicUsize::new(0));
        info!(
            socket = %args.socket.display(),
            allowed_peer_uid = args.allowed_peer_uid,
            "terminal bridge is ready"
        );

        for connection in listener.incoming() {
            let stream = match connection {
                Ok(stream) => stream,
                Err(error) => {
                    warn!(%error, "terminal socket accept failed");
                    continue;
                }
            };
            if let Err(error) = validate_peer(&stream, args.allowed_peer_uid) {
                warn!(%error, "rejected terminal socket peer");
                continue;
            }
            let reserved = active
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                    (current < MAX_CONCURRENT_TERMINALS).then_some(current + 1)
                })
                .is_ok();
            if !reserved {
                let _ = send_error(
                    &stream,
                    "Helix already has the maximum number of terminal sessions open.",
                );
                continue;
            }
            let config = Arc::clone(&config);
            let active = Arc::clone(&active);
            thread::spawn(move || {
                let _slot = SessionSlot(active.as_ref());
                if let Err(error) = handle_connection(stream, &config) {
                    warn!(%error, "terminal session ended with a transport error");
                }
            });
        }
        Ok(())
    }

    fn validate_args(args: &Args, working_directory: &Path) -> Result<(), io::Error> {
        if rustix::process::geteuid().is_root() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "helix-terminald refuses to run as root",
            ));
        }
        for (name, path) in [
            ("socket", args.socket.as_path()),
            ("shell", args.shell.as_path()),
            ("working directory", working_directory),
        ] {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{name} must be absolute"),
                ));
            }
        }
        let shell = fs::metadata(&args.shell)?;
        if !shell.is_file() || shell.permissions().mode() & 0o111 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the configured shell is not executable",
            ));
        }
        if !fs::metadata(working_directory)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the working directory is not a directory",
            ));
        }
        if args.user.is_empty()
            || args.user.len() > 64
            || !args
                .user
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the terminal user name is invalid",
            ));
        }
        if args.allowed_peer_uid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the allowed terminal peer must not be root",
            ));
        }
        Ok(())
    }

    fn validate_peer(stream: &UnixStream, allowed_uid: u32) -> Result<(), io::Error> {
        let credentials = rustix::net::sockopt::socket_peercred(stream)?;
        if !peer_uid_allowed(credentials.uid.as_raw(), allowed_uid) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "terminal socket peer does not match the configured dashboard user",
            ));
        }
        Ok(())
    }

    fn peer_uid_allowed(actual_uid: u32, allowed_uid: u32) -> bool {
        actual_uid == allowed_uid && allowed_uid != 0
    }

    fn prepare_socket_path(path: &Path) -> Result<(), io::Error> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket has no parent"))?;
        if !fs::metadata(parent)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket parent is not a directory",
            ));
        }
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if metadata.file_type().is_socket()
                    && metadata.uid() == rustix::process::geteuid().as_raw() =>
            {
                fs::remove_file(path)
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "refusing to replace a non-socket or foreign socket path",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn handle_connection(
        mut read_stream: UnixStream,
        config: &SessionConfig,
    ) -> Result<(), io::Error> {
        let first = read_frame(&mut read_stream)?;
        if first.kind != kind::CLIENT_OPEN {
            send_error(
                &read_stream,
                "The terminal client did not begin with a valid open request.",
            )?;
            return Ok(());
        }
        let open: OpenRequest = decode_json(&first.payload).map_err(protocol_io_error)?;
        if open.protocol_version != PROTOCOL_VERSION {
            send_error(
                &read_stream,
                "The terminal client and host service use different protocol versions.",
            )?;
            return Ok(());
        }
        let dimensions = open.dimensions.validate().map_err(protocol_io_error)?;
        run_pty(read_stream, config, dimensions)
    }

    fn run_pty(
        mut read_stream: UnixStream,
        config: &SessionConfig,
        dimensions: TerminalDimensions,
    ) -> Result<(), io::Error> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: dimensions.rows,
                cols: dimensions.columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(other_io_error)?;
        let mut command = CommandBuilder::new(&config.shell);
        command.arg("-l");
        command.cwd(&config.working_directory);
        command.env_clear();
        command.env("HOME", &config.working_directory);
        command.env("USER", &config.user);
        command.env("LOGNAME", &config.user);
        command.env("SHELL", &config.shell);
        command.env("PATH", PATH_VALUE);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("LANG", "C.UTF-8");
        let mut reader = pair.master.try_clone_reader().map_err(other_io_error)?;
        let mut terminal_input = pair.master.take_writer().map_err(other_io_error)?;
        let mut child = pair.slave.spawn_command(command).map_err(other_io_error)?;
        drop(pair.slave);

        let output_stream = Arc::new(Mutex::new(read_stream.try_clone()?));
        send_json(
            output_stream.as_ref(),
            kind::SERVER_READY,
            &ReadyResponse {
                protocol_version: PROTOCOL_VERSION,
                user: config.user.clone(),
                shell: config.shell.to_string_lossy().into_owned(),
            },
        )?;
        let exited = Arc::new(AtomicBool::new(false));
        let output_writer = Arc::clone(&output_stream);
        let output_thread = thread::spawn(move || {
            let mut buffer = [0_u8; OUTPUT_CHUNK_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if send_frame_locked(
                            output_writer.as_ref(),
                            kind::SERVER_OUTPUT,
                            &buffer[..count],
                        )
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });

        let mut killer = child.clone_killer();
        let monitor_writer = Arc::clone(&output_stream);
        let monitor_exited = Arc::clone(&exited);
        let monitor_stream = read_stream.try_clone()?;
        let monitor_thread = thread::spawn(move || {
            let status = child.wait();
            monitor_exited.store(true, Ordering::Release);
            if let Ok(status) = status {
                let _ = send_json(
                    monitor_writer.as_ref(),
                    kind::SERVER_EXIT,
                    &ExitResponse {
                        exit_code: status.exit_code(),
                        signal: status.signal().map(str::to_owned),
                    },
                );
            } else {
                let _ = send_error_locked(
                    monitor_writer.as_ref(),
                    "The terminal shell ended unexpectedly.",
                );
            }
            let _ = monitor_stream.shutdown(std::net::Shutdown::Both);
        });

        loop {
            let frame = match read_frame(&mut read_stream) {
                Ok(frame) => frame,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::BrokenPipe
                    ) =>
                {
                    break;
                }
                Err(error) => return Err(error),
            };
            match frame.kind {
                kind::CLIENT_INPUT
                    if frame.payload.len() <= helix_terminal::MAX_TERMINAL_INPUT_BYTES =>
                {
                    terminal_input.write_all(&frame.payload)?;
                    terminal_input.flush()?;
                }
                kind::CLIENT_RESIZE => {
                    let next = decode_resize(&frame.payload).map_err(protocol_io_error)?;
                    pair.master
                        .resize(PtySize {
                            rows: next.rows,
                            cols: next.columns,
                            pixel_width: 0,
                            pixel_height: 0,
                        })
                        .map_err(other_io_error)?;
                }
                kind::CLIENT_CLOSE => break,
                _ => {
                    send_error_locked(
                        output_stream.as_ref(),
                        "The terminal client sent an invalid frame.",
                    )?;
                    break;
                }
            }
        }
        drop(terminal_input);
        if !exited.load(Ordering::Acquire) {
            let _ = killer.kill();
        }
        let _ = monitor_thread.join();
        let _ = read_stream.shutdown(std::net::Shutdown::Both);
        let _ = output_thread.join();
        Ok(())
    }

    fn read_frame(stream: &mut UnixStream) -> Result<Frame, io::Error> {
        let mut header = [0_u8; 4];
        stream.read_exact(&mut header)?;
        let length = decode_frame_length(header).map_err(protocol_io_error)?;
        let mut body = vec![0_u8; length];
        stream.read_exact(&mut body)?;
        let kind = body[0];
        Ok(Frame {
            kind,
            payload: body.split_off(1),
        })
    }

    fn send_json<T: serde::Serialize>(
        stream: &Mutex<UnixStream>,
        frame_kind: u8,
        value: &T,
    ) -> Result<(), io::Error> {
        let payload = encode_json(value).map_err(protocol_io_error)?;
        send_frame_locked(stream, frame_kind, &payload)
    }

    fn send_error(stream: &UnixStream, message: &str) -> Result<(), io::Error> {
        let cloned = stream.try_clone()?;
        send_error_locked(&Mutex::new(cloned), message)
    }

    fn send_error_locked(stream: &Mutex<UnixStream>, message: &str) -> Result<(), io::Error> {
        send_frame_locked(stream, kind::SERVER_ERROR, message.as_bytes())
    }

    fn send_frame_locked(
        stream: &Mutex<UnixStream>,
        frame_kind: u8,
        payload: &[u8],
    ) -> Result<(), io::Error> {
        let bytes = encode_frame(frame_kind, payload).map_err(protocol_io_error)?;
        let mut writer = stream
            .lock()
            .map_err(|_| io::Error::other("terminal transport lock failed"))?;
        writer.write_all(&bytes)
    }

    fn protocol_io_error(error: impl std::fmt::Display) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, error.to_string())
    }

    fn other_io_error(error: impl std::fmt::Display) -> io::Error {
        io::Error::other(error.to_string())
    }

    fn init_tracing() -> Result<(), DynError> {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("helix_terminal=info"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .compact()
            .try_init()?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::peer_uid_allowed;

        #[test]
        fn peer_uid_requires_an_exact_non_root_match() {
            assert!(peer_uid_allowed(10_001, 10_001));
            assert!(!peer_uid_allowed(1_000, 10_001));
            assert!(!peer_uid_allowed(0, 0));
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    linux_daemon::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("helix-terminald is available only on Linux hosts");
    std::process::exit(1);
}
