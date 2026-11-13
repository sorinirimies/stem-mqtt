//! TLS for the blocking client over [rustls](https://docs.rs/rustls) (feature `rustls`).
//!
//! For PCs, Linux gateways and tests. Firmware normally uses the TLS stack of its SDK instead
//! (see the README): implement [`Transport`](crate::Transport) for it.
//!
//! ```no_run
//! use mqtt_client_blocking::{Client, Options, tls};
//!
//! // Verify the broker against a private CA (PEM); `None` trusts the public web roots.
//! let ca = std::fs::read("ca.pem")?;
//! let config = tls::client_config(Some(&ca), None)?;
//! let stream = tls::connect("broker.local", 8883, config)?;
//! let client = Client::connect(stream, Options::new("gateway"))?;
//! # drop(client);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::Transport;

/// A client certificate chain and its private key, both PEM, for mutual TLS.
pub struct ClientIdentity<'a> {
    pub cert_pem: &'a [u8],
    pub key_pem: &'a [u8],
}

// Never print the private key.
impl std::fmt::Debug for ClientIdentity<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientIdentity")
            .field("cert_pem", &self.cert_pem.len())
            .field("key_pem", &"<redacted>")
            .finish()
    }
}

/// A TLS stream over a TCP socket, ready to hand to [`Client::connect`](crate::Client::connect).
pub type TlsStream = StreamOwned<ClientConnection, TcpStream>;

/// Build a client configuration.
///
/// * `ca_pem`: PEM certificate(s) to trust (a private CA, or a self-signed broker certificate).
///   `None` trusts the Mozilla web roots (`webpki-roots`).
/// * `identity`: a client certificate for brokers that require mutual TLS.
///
/// # Errors
/// `InvalidData` if a PEM block does not parse, holds no certificate, or the key does not match.
pub fn client_config(
    ca_pem: Option<&[u8]>,
    identity: Option<ClientIdentity<'_>>,
) -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    match ca_pem {
        Some(pem) => {
            let certs = parse_certs(pem)?;
            let (added, _) = roots.add_parsable_certificates(certs);
            if added == 0 {
                return Err(invalid("no usable CA certificate in the PEM"));
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    let builder = ClientConfig::builder().with_root_certificates(roots);
    let config = match identity {
        None => builder.with_no_client_auth(),
        Some(id) => {
            let key = PrivateKeyDer::from_pem_slice(id.key_pem)
                .map_err(|e| invalid(format!("client key: {e}")))?;
            builder
                .with_client_auth_cert(parse_certs(id.cert_pem)?, key)
                .map_err(|e| invalid(format!("client certificate: {e}")))?
        }
    };
    Ok(Arc::new(config))
}

/// Connect a TCP socket to `host:port` and wrap it in TLS, verifying the broker's certificate for
/// `host` (a DNS name or an IP address). The handshake runs on the first read or write, so
/// [`Client::connect`](crate::Client::connect) performs it and a rejected certificate shows up
/// there as an I/O error.
///
/// # Errors
/// An I/O error if the TCP connection fails or `host` is not a valid server name.
pub fn connect(host: &str, port: u16, config: Arc<ClientConfig>) -> io::Result<TlsStream> {
    let name = ServerName::try_from(host.to_owned())
        .map_err(|e| invalid(format!("server name {host:?}: {e}")))?;
    let connection = ClientConnection::new(config, name).map_err(io::Error::other)?;
    let socket = TcpStream::connect((host, port))?;
    Ok(StreamOwned::new(connection, socket))
}

impl Transport for TlsStream {
    /// Sets the timeout on the TCP socket. A timeout in the middle of a TLS record is safe: rustls
    /// keeps the partial record and the next read continues it.
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.sock.set_read_timeout(timeout)
    }
}

fn parse_certs(pem: &[u8]) -> io::Result<Vec<CertificateDer<'static>>> {
    CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| invalid(format!("certificate: {e}")))
}

fn invalid(msg: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}
