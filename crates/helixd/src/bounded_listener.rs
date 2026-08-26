use axum::serve::Listener;
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{OwnedSemaphorePermit, Semaphore},
};

pub(crate) const MAX_CONCURRENT_CONNECTIONS: usize = 128;

pub(crate) struct BoundedTcpListener {
    inner: TcpListener,
    connection_slots: Arc<Semaphore>,
}

impl BoundedTcpListener {
    pub(crate) fn new(inner: TcpListener, maximum_connections: usize) -> Self {
        assert!(maximum_connections > 0, "connection limit must be positive");
        Self {
            inner,
            connection_slots: Arc::new(Semaphore::new(maximum_connections)),
        }
    }
}

pub(crate) struct BoundedTcpStream {
    inner: TcpStream,
    _connection_slot: OwnedSemaphorePermit,
}

impl BoundedTcpStream {
    pub(crate) fn set_nodelay(&self, no_delay: bool) -> io::Result<()> {
        self.inner.set_nodelay(no_delay)
    }
}

impl Listener for BoundedTcpListener {
    type Io = BoundedTcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        let connection_slot = Arc::clone(&self.connection_slots)
            .acquire_owned()
            .await
            .expect("the private connection semaphore is never closed");
        let (inner, address) = Listener::accept(&mut self.inner).await;
        (
            BoundedTcpStream {
                inner,
                _connection_slot: connection_slot,
            },
            address,
        )
    }

    fn local_addr(&self) -> io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

impl AsyncRead for BoundedTcpStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for BoundedTcpStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn accepted_connection_flood_is_capped_until_a_stream_closes() {
        const LIMIT: usize = 4;
        let tcp_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let address = tcp_listener.local_addr().expect("test listener address");
        let mut listener = BoundedTcpListener::new(tcp_listener, LIMIT);
        let mut clients = Vec::new();
        let mut accepted = Vec::new();

        for _ in 0..LIMIT {
            clients.push(TcpStream::connect(address).await.expect("connect client"));
            accepted.push(Listener::accept(&mut listener).await.0);
        }
        assert_eq!(listener.connection_slots.available_permits(), 0);

        let waiting_client = TcpStream::connect(address)
            .await
            .expect("connect waiting client");
        let mut waiting_accept = Box::pin(Listener::accept(&mut listener));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut waiting_accept)
                .await
                .is_err(),
            "the listener accepted more than its configured cap"
        );

        drop(accepted.pop());
        let (_stream, _peer) = tokio::time::timeout(Duration::from_secs(1), waiting_accept)
            .await
            .expect("accept resumes after a stream closes");
        drop(waiting_client);
        drop(clients);
    }
}
