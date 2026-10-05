//! TLS transport for [`super::MqttClient`], built on `rustls`/`tokio-rustls`
//! — pure Rust, no OpenSSL/system-TLS dependency. Plain TCP stays the
//! default and is completely unaffected; this module is only reached when
//! [`ConnectOptions::tls`] is `Some`.

use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::pem::{Error as PemError, PemObject};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

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
    CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MqttError::Protocol(format!("invalid PEM certificate: {e}")))
}

fn parse_private_key(pem: &[u8]) -> MqttResult<rustls::pki_types::PrivateKeyDer<'static>> {
    PrivateKeyDer::from_pem_slice(pem).map_err(|e| match e {
        PemError::NoItemsFound => MqttError::Protocol("no private key found in PEM".into()),
        other => MqttError::Protocol(format!("invalid PEM private key: {other}")),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> TlsOptions {
        TlsOptions::new()
    }

    #[test]
    fn default_options_trust_the_bundled_roots() {
        assert!(build_client_config(&opts()).is_ok());
    }

    #[test]
    fn insecure_mode_builds_without_any_roots() {
        let mut o = opts();
        o.insecure_skip_certificate_verification = true;
        assert!(build_client_config(&o).is_ok());
    }

    #[test]
    fn a_client_cert_without_its_key_is_rejected() {
        let mut o = opts();
        o.client_cert_pem = Some(b"cert".to_vec());
        let err = build_client_config(&o).unwrap_err().to_string();
        assert!(err.contains("both be set"), "{err}");
        let mut o = opts();
        o.client_key_pem = Some(b"key".to_vec());
        assert!(build_client_config(&o).is_err());
    }

    #[test]
    fn an_unparseable_ca_bundle_is_an_error_not_silently_ignored() {
        let mut o = opts();
        o.ca_cert_pem = Some(
            b"-----BEGIN CERTIFICATE-----\nnot base64!!\n-----END CERTIFICATE-----\n".to_vec(),
        );
        assert!(build_client_config(&o).is_err());
    }

    #[test]
    fn a_pem_without_a_private_key_is_reported() {
        let mut o = opts();
        o.client_cert_pem = Some(b"".to_vec());
        o.client_key_pem = Some(b"".to_vec());
        let err = build_client_config(&o).unwrap_err().to_string();
        assert!(err.contains("no private key"), "{err}");
    }
}
