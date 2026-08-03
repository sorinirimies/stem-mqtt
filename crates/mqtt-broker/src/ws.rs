//! MQTT-over-WebSocket transport (MQTT-5.0 §6, MQTT-3.1.1 Appendix B).
//!
//! Bridges a [`WebSocketStream`] to [`tokio::io::AsyncRead`]/[`AsyncWrite`]
//! so the exact same [`crate::connection::handle_connection`] used for raw
//! TCP connections also drives WebSocket ones — the packet codec and all
//! session/broker logic stay transport-agnostic.
//!
//! Framing (per spec): each WebSocket message contains a whole number of
//! complete MQTT Control Packets. On the way in we don't rely on message
//! boundaries at all — bytes from successive Binary messages are simply
//! concatenated into the same buffer `crate::connection`'s packet decoder
//! already reads from, so multiple packets per message, or fragmentation
//! across TCP reads, both just work. On the way out, every internal
//! `write_all(&bytes)` call in this crate always passes exactly one fully
//! encoded packet at a time, so [`WsByteStream::poll_write`] sending each
//! write as one Binary message satisfies the "whole number of packets per
//! message" requirement trivially.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::{Sink, Stream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

pub(crate) struct WsByteStream<S> {
    inner: WebSocketStream<S>,
    /// Bytes from a Binary message that didn't fit in the caller's buffer
    /// on a previous `poll_read`, held for the next call.
    read_leftover: Vec<u8>,
}

impl<S> WsByteStream<S> {
    pub(crate) fn new(inner: WebSocketStream<S>) -> Self {
        WsByteStream {
            inner,
            read_leftover: Vec::new(),
        }
    }
}

impl<S> AsyncRead for WsByteStream<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.read_leftover.is_empty() {
            let n = self.read_leftover.len().min(buf.remaining());
            buf.put_slice(&self.read_leftover[..n]);
            self.read_leftover.drain(..n);
            return Poll::Ready(Ok(()));
        }

        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(message))) => match message {
                    Message::Binary(data) => {
                        if data.is_empty() {
                            continue;
                        }
                        let n = data.len().min(buf.remaining());
                        buf.put_slice(&data[..n]);
                        if n < data.len() {
                            self.read_leftover.extend_from_slice(&data[n..]);
                        }
                        return Poll::Ready(Ok(()));
                    }
                    // MQTT Control Packets are binary; ping/pong/close are
                    // handled transparently by tungstenite already. A stray
                    // Text frame is not valid MQTT-over-WS — ignore it
                    // rather than tearing down the connection over it.
                    Message::Text(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                        continue
                    }
                    Message::Close(_) => return Poll::Ready(Ok(())), // EOF
                },
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Err(std::io::Error::other(e))),
                Poll::Ready(None) => return Poll::Ready(Ok(())), // EOF
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl<S> AsyncWrite for WsByteStream<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_ready(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(to_io_err(e))),
            Poll::Pending => return Poll::Pending,
        }
        Pin::new(&mut self.inner)
            .start_send(Message::Binary(buf.to_vec()))
            .map_err(to_io_err)?;
        // Flush immediately: our callers never call `flush()` themselves
        // (see module docs), so a write that isn't force-flushed here
        // would sit in tungstenite's internal buffer forever.
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(buf.len())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(to_io_err(e))),
            Poll::Pending => Poll::Ready(Ok(buf.len())), // already queued; flush completes async
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx).map_err(to_io_err)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_close(cx).map_err(to_io_err)
    }
}

fn to_io_err(e: tokio_tungstenite::tungstenite::Error) -> std::io::Error {
    std::io::Error::other(e)
}
