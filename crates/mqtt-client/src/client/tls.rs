//! TLS transport for [`super::MqttClient`], built on `rustls`/`tokio-rustls`
//! — pure Rust, no OpenSSL/system-TLS dependency. Plain TCP stays the
//! default and is completely unaffected; this module is only reached when
//! [`ConnectOptions::tls`] is `Some`.

use std::io::BufReader;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls;

use crate::error::{MqttError, MqttResult};

use super::types::{ConnectOptions, TlsOptions};

/// Type-erased duplex transport: plain TCP or TLS-wrapped TCP. Lets the
/// rest of the client (the read loop, packet encode/write helpers) stay
/// completely agnostic to which one is in use — mirroring
/// `mqtt-broker`'s `handle_connection<S: AsyncRead + AsyncWrite>`, which
/// already does the same thing for its TCP/WebSocket transports.
pub(super) trait Transport: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Transport for T {}

/// Connects to `options.host:options.port`, then — if `options.tls` is
/// set — completes a TLS handshake on top of that TCP connection. Returns
/// a type-erased transport either way.
pub(super) async fn connect_transport(options: &ConnectOptions) -> MqttResult<Box<dyn Transport>> {
    let addr = format!("{}:{}", options.host, options.port);
    let tcp = TcpStream::connect(&addr).await?;
    tcp.set_nodelay(true).ok();

    let Some(tls) = &options.tls else {
        return Ok(Box::new(tcp));
    };

    let config = build_client_config(tls)?;
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name = rustls::pki_types::ServerName::try_from(options.host.clone())
        .map_err(|e| MqttError::Protocol(format!("invalid TLS server name: {e}")))?;
    let tls_stream = connector
        .connect(server_name, tcp)
        .await
        .map_err(|e| MqttError::Io(format!("TLS handshake failed: {e}")))?;
    Ok(Box::new(tls_stream))
}

fn build_client_config(tls: &TlsOptions) -> MqttResult<rustls::ClientConfig> {
    let verifier_stage = if tls.insecure_skip_certificate_verification {
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert::new()))
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        if let Some(ca_pem) = &tls.ca_cert_pem {
            for cert in parse_certs(ca_pem)? {
                roots
                    .add(cert)
                    .map_err(|e| MqttError::Protocol(format!("invalid CA certificate: {e}")))?;
            }
        }
        rustls::ClientConfig::builder().with_root_certificates(roots)
    };

    let config = match (&tls.client_cert_pem, &tls.client_key_pem) {
        (Some(cert_pem), Some(key_pem)) => {
            let certs = parse_certs(cert_pem)?;
            let key = parse_private_key(key_pem)?;
            verifier_stage
                .with_client_auth_cert(certs, key)
                .map_err(|e| MqttError::Protocol(format!("invalid client certificate/key: {e}")))?
        }
        (None, None) => verifier_stage.with_no_client_auth(),
        _ => {
            return Err(MqttError::Protocol(
                "TlsOptions: client_cert_pem and client_key_pem must both be set for mTLS, \
                 or both left unset"
                    .into(),
            ))
        }
    };

    Ok(config)
}

fn parse_certs(pem: &[u8]) -> MqttResult<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let mut reader = BufReader::new(pem);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MqttError::Protocol(format!("invalid PEM certificate: {e}")))
}

fn parse_private_key(pem: &[u8]) -> MqttResult<rustls::pki_types::PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(pem);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| MqttError::Protocol(format!("invalid PEM private key: {e}")))?
        .ok_or_else(|| MqttError::Protocol("no private key found in PEM".into()))
}

/// A [`rustls::client::danger::ServerCertVerifier`] that accepts any
/// server certificate unconditionally — backs
/// [`TlsOptions::insecure_skip_certificate_verification`]. Deliberately
/// named loudly; this must never be reachable except when a caller
/// explicitly opted into it.
#[derive(Debug)]
struct AcceptAnyServerCert {
    supported_schemes: Vec<rustls::SignatureScheme>,
}

impl AcceptAnyServerCert {
    fn new() -> Self {
        AcceptAnyServerCert {
            supported_schemes: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes(),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported_schemes.clone()
    }
}
