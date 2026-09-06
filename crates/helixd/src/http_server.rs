//! Bounded HTTP/1 serving, overload signaling, and graceful connection drain.

use axum::{Extension, Router, body::Body, extract::ConnectInfo, http::Request, serve::Listener};
use hyper::{body::Incoming, server::conn::http1};
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    task::{JoinError, JoinSet},
    time::timeout,
};
use tower::ServiceExt as _;
use tracing::{trace, warn};

pub(crate) const MAX_CONCURRENT_CONNECTIONS: usize = 128;
pub(crate) const REQUEST_HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HTTP1_BUFFER_BYTES: usize = 32 * 1024;
const MAX_CONCURRENT_OVERLOAD_RESPONSES: usize = 32;
const OVERLOAD_IO_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_OVERLOAD_HEADER_BYTES: usize = 8 * 1024;

const OVERLOAD_RESPONSE: &[u8] = b"HTTP/1.1 503 Service Unavailable\r\n\
Connection: close\r\n\
Content-Length: 0\r\n\
Cache-Control: no-store\r\n\
Retry-After: 1\r\n\r\n";

pub(crate) async fn serve<F>(listener: TcpListener, app: Router, shutdown: F) -> io::Result<()>
where
    F: Future<Output = ()> + Send,
{
    serve_with_options(
        listener,
        app,
        MAX_CONCURRENT_CONNECTIONS,
        REQUEST_HEADER_TIMEOUT,
        shutdown,
    )
    .await
}

async fn serve_with_options<F>(
    mut listener: TcpListener,
    app: Router,
    maximum_connections: usize,
    request_header_timeout: Duration,
    shutdown: F,
) -> io::Result<()>
where
    F: Future<Output = ()> + Send,
{
    assert!(maximum_connections > 0, "connection limit must be positive");
    assert!(
        !request_header_timeout.is_zero(),
        "request header timeout must be positive"
    );

    let connection_slots = Arc::new(Semaphore::new(maximum_connections));
    let overload_slots = Arc::new(Semaphore::new(
        maximum_connections.min(MAX_CONCURRENT_OVERLOAD_RESPONSES),
    ));
    let (shutdown_tx, _) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            completed = connections.join_next(), if !connections.is_empty() => {
                if let Some(result) = completed {
                    report_connection_task(result);
                }
            }
            accepted = Listener::accept(&mut listener) => {
                let (stream, peer) = accepted;
                if let Err(error) = stream.set_nodelay(true) {
                    warn!(%error, %peer, "could not enable TCP_NODELAY on an accepted connection");
                }
                let Ok(connection_slot) = Arc::clone(&connection_slots).try_acquire_owned() else {
                    if let Ok(overload_slot) = Arc::clone(&overload_slots).try_acquire_owned() {
                        connections.spawn(reject_overloaded(stream, overload_slot));
                    }
                    continue;
                };
                connections.spawn(serve_connection(
                    stream,
                    peer,
                    app.clone(),
                    request_header_timeout,
                    connection_slot,
                    shutdown_tx.subscribe(),
                ));
            }
        }
    }

    let _ = shutdown_tx.send(true);
    while let Some(result) = connections.join_next().await {
        report_connection_task(result);
    }
    Ok(())
}

async fn reject_overloaded(mut stream: TcpStream, _overload_slot: OwnedSemaphorePermit) {
    let received_headers = timeout(OVERLOAD_IO_TIMEOUT, async {
        let mut received = Vec::with_capacity(1024);
        let mut buffer = [0_u8; 1024];
        loop {
            let count = stream.read(&mut buffer).await?;
            if count == 0 {
                return Ok::<bool, io::Error>(false);
            }
            for byte in &buffer[..count] {
                if received.len() == MAX_OVERLOAD_HEADER_BYTES {
                    return Ok::<bool, io::Error>(false);
                }
                received.push(*byte);
                if received.ends_with(b"\r\n\r\n") {
                    return Ok::<bool, io::Error>(true);
                }
            }
        }
    })
    .await;
    if !matches!(received_headers, Ok(Ok(true))) {
        return;
    }

    match timeout(OVERLOAD_IO_TIMEOUT, stream.write_all(OVERLOAD_RESPONSE)).await {
        Ok(Ok(())) => {
            let _ = stream.shutdown().await;
        }
        Ok(Err(error)) => {
            trace!(%error, "an overloaded connection closed before its response")
        }
        Err(_) => trace!("an overloaded connection timed out before its response"),
    }
}

async fn serve_connection(
    stream: TcpStream,
    peer: SocketAddr,
    app: Router,
    request_header_timeout: Duration,
    _connection_slot: OwnedSemaphorePermit,
    mut shutdown: watch::Receiver<bool>,
) {
    let io = TokioIo::new(stream);
    let service = app
        .layer(Extension(ConnectInfo(peer)))
        .map_request(|request: Request<Incoming>| request.map(Body::new));
    let service = TowerToHyperService::new(service);
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(request_header_timeout)
        .max_buf_size(MAX_HTTP1_BUFFER_BYTES);
    let connection = builder.serve_connection(io, service).with_upgrades();
    tokio::pin!(connection);

    tokio::select! {
        result = &mut connection => {
            if let Err(error) = result {
                trace!(%error, %peer, "HTTP connection closed with an error");
            }
        }
        changed = shutdown.changed() => {
            if changed.is_ok() && *shutdown.borrow() {
                connection.as_mut().graceful_shutdown();
                if let Err(error) = connection.await {
                    trace!(%error, %peer, "HTTP connection closed during graceful shutdown");
                }
            }
        }
    }
}

fn report_connection_task(result: Result<(), JoinError>) {
    if let Err(error) = result {
        warn!(%error, "an HTTP connection task failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, routing::get};
    use tokio::sync::oneshot;

    async fn raw_get(address: SocketAddr) -> String {
        raw_request(
            address,
            format!("GET /healthz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"),
        )
        .await
    }

    async fn raw_request(address: SocketAddr, request: String) -> String {
        let mut stream = TcpStream::connect(address).await.expect("connect client");
        stream
            .write_all(request.as_bytes())
            .await
            .expect("write request");
        let mut response = String::new();
        timeout(Duration::from_secs(1), stream.read_to_string(&mut response))
            .await
            .expect("response deadline")
            .expect("read response");
        response
    }

    #[tokio::test]
    async fn silent_connections_get_fail_fast_overload_then_expire() {
        const LIMIT: usize = 2;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("test listener address");
        let app = Router::new().route("/healthz", get(|| async { StatusCode::NO_CONTENT }));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(serve_with_options(
            listener,
            app,
            LIMIT,
            Duration::from_millis(100),
            async move {
                let _ = shutdown_rx.await;
            },
        ));

        let first = TcpStream::connect(address)
            .await
            .expect("first silent client");
        let second = TcpStream::connect(address)
            .await
            .expect("second silent client");
        tokio::time::sleep(Duration::from_millis(20)).await;

        let overloaded = raw_get(address).await;
        assert!(
            overloaded.starts_with("HTTP/1.1 503 Service Unavailable\r\n"),
            "unexpected overload response: {overloaded:?}"
        );

        tokio::time::sleep(Duration::from_millis(150)).await;
        let recovered = raw_get(address).await;
        assert!(
            recovered.starts_with("HTTP/1.1 204 No Content\r\n"),
            "unexpected recovery response: {recovered:?}"
        );

        drop(first);
        drop(second);
        let _ = shutdown_tx.send(());
        timeout(Duration::from_secs(1), server)
            .await
            .expect("server shutdown deadline")
            .expect("server task")
            .expect("server result");
    }

    #[tokio::test]
    async fn oversized_request_headers_never_reach_the_application() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = listener.local_addr().expect("test listener address");
        let app = Router::new().route("/healthz", get(|| async { StatusCode::NO_CONTENT }));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(serve_with_options(
            listener,
            app,
            2,
            Duration::from_secs(1),
            async move {
                let _ = shutdown_rx.await;
            },
        ));

        let request = format!(
            "GET /healthz HTTP/1.1\r\nHost: {address}\r\nX-Fill: {}\r\nConnection: close\r\n\r\n",
            "x".repeat(MAX_HTTP1_BUFFER_BYTES * 4)
        );
        let response = raw_request(address, request).await;
        assert!(
            !response.starts_with("HTTP/1.1 204 No Content\r\n"),
            "oversized headers reached the application"
        );

        let _ = shutdown_tx.send(());
        timeout(Duration::from_secs(1), server)
            .await
            .expect("server shutdown deadline")
            .expect("server task")
            .expect("server result");
    }
}
