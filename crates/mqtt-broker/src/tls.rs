//! Server-side TLS setup for [`crate::broker::MqttBroker`], built on
//! `rustls`/`tokio-rustls` — pure Rust, no OpenSSL/system-TLS dependency.
//! Only reached when [`crate::config::MqttBrokerConfig::tls`] is `Some`;
//! the plain-TCP and WebSocket listeners are completely unaffected.

use std::io::BufReader;
use std::sync::Arc;

use tokio_rustls::rustls;

use crate::config::BrokerTlsConfig;

/// Build a [`tokio_rustls::TlsAcceptor`] from a [`BrokerTlsConfig`]: parses
/// the server certificate chain + private key, and — if
/// [`BrokerTlsConfig::client_ca_pem`] is set — configures mutual TLS by
/// requiring and verifying client certificates against that CA.
pub(crate) fn build_acceptor(tls: &BrokerTlsConfig) -> Result<tokio_rustls::TlsAcceptor, String> {
    let certs =
        parse_certs(&tls.cert_pem).map_err(|e| format!("invalid server certificate: {e}"))?;
    let key =
        parse_private_key(&tls.key_pem).map_err(|e| format!("invalid server private key: {e}"))?;

    let builder = rustls::ServerConfig::builder();
    let config = match &tls.client_ca_pem {
        Some(ca_pem) => {
            let mut roots = rustls::RootCertStore::empty();
            for cert in
                parse_certs(ca_pem).map_err(|e| format!("invalid client CA certificate: {e}"))?
            {
                roots
                    .add(cert)
                    .map_err(|e| format!("invalid client CA certificate: {e}"))?;
            }
            let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(roots))
                .build()
                .map_err(|e| format!("failed to build client certificate verifier: {e}"))?;
            builder
                .with_client_cert_verifier(verifier)
                .with_single_cert(certs, key)
                .map_err(|e| format!("invalid server certificate/key: {e}"))?
        }
        None => builder
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("invalid server certificate/key: {e}"))?,
    };

    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

fn parse_certs(pem: &[u8]) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, String> {
    let mut reader = BufReader::new(pem);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn parse_private_key(pem: &[u8]) -> Result<rustls::pki_types::PrivateKeyDer<'static>, String> {
    let mut reader = BufReader::new(pem);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no private key found in PEM".to_string())
}
