//! Server-side TLS setup for [`crate::broker::MqttBroker`], built on
//! `rustls`/`tokio-rustls` — pure Rust, no OpenSSL/system-TLS dependency.
//! Only reached when [`crate::config::MqttBrokerConfig::tls`] is `Some`;
//! the plain-TCP and WebSocket listeners are completely unaffected.

use std::sync::Arc;

use tokio_rustls::rustls;
use tokio_rustls::rustls::pki_types::pem::{Error as PemError, PemObject};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

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

/// What a verified client certificate says about its holder (see [`crate::CertClientId`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PeerIdentity {
    /// The subject common name.
    pub common_name: Option<String>,
    /// The DNS subject alternative names.
    pub dns_names: Vec<String>,
}

/// Where a connection came from, as far as the broker can tell: plain TCP or WebSocket, or TLS with (mutual TLS) or
/// without a client certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Peer {
    /// Raw TCP or WebSocket: no certificate.
    Plain,
    /// TLS; the identity of the client certificate when it presented one.
    Tls(Option<PeerIdentity>),
}

/// Reads the identity out of a DER client certificate (already verified by the TLS handshake against the client CA).
pub(crate) fn peer_identity(der: &[u8]) -> Option<PeerIdentity> {
    use x509_parser::extensions::GeneralName;
    use x509_parser::prelude::{FromDer, X509Certificate};
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    let common_name = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .map(str::to_owned);
    let mut dns_names = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let GeneralName::DNSName(d) = name {
                dns_names.push((*d).to_owned());
            }
        }
    }
    Some(PeerIdentity {
        common_name,
        dns_names,
    })
}

fn parse_certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, String> {
    CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn parse_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, String> {
    PrivateKeyDer::from_pem_slice(pem).map_err(|e| match e {
        PemError::NoItemsFound => "no private key found in PEM".to_string(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn self_signed() -> (Vec<u8>, Vec<u8>) {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        (cert.pem().into_bytes(), key.serialize_pem().into_bytes())
    }

    fn config(cert: &[u8], key: &[u8], ca: Option<&[u8]>) -> BrokerTlsConfig {
        BrokerTlsConfig {
            port: 0,
            cert_pem: cert.to_vec(),
            key_pem: key.to_vec(),
            client_ca_pem: ca.map(<[u8]>::to_vec),
            bind_client_id: crate::CertClientId::None,
        }
    }

    fn err(config: &BrokerTlsConfig) -> String {
        build_acceptor(config)
            .err()
            .expect("configuration must be rejected")
    }

    #[test]
    fn a_valid_certificate_and_key_build_an_acceptor() {
        let (cert, key) = self_signed();
        assert!(build_acceptor(&config(&cert, &key, None)).is_ok());
    }

    #[test]
    fn mutual_tls_accepts_a_ca_bundle() {
        let (cert, key) = self_signed();
        let (ca, _) = self_signed();
        assert!(build_acceptor(&config(&cert, &key, Some(&ca))).is_ok());
    }

    #[test]
    fn a_missing_private_key_is_reported_clearly() {
        let (cert, _) = self_signed();
        assert!(err(&config(&cert, b"", None)).contains("no private key"));
    }

    #[test]
    fn a_certificate_that_does_not_match_the_key_is_rejected() {
        let (cert, _) = self_signed();
        let (_, other_key) = self_signed();
        assert!(err(&config(&cert, &other_key, None)).contains("certificate/key"));
    }

    #[test]
    fn the_common_name_and_dns_names_are_read_from_a_certificate() {
        let key = rcgen::KeyPair::generate().expect("key");
        let mut params = rcgen::CertificateParams::new(vec![
            "rig-a1.farm.local".to_string(),
            "rig-a1".to_string(),
        ])
        .expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "rig-a1");
        let cert = params.self_signed(&key).expect("cert");
        let id = peer_identity(cert.der().as_ref()).expect("an identity");
        assert_eq!(id.common_name.as_deref(), Some("rig-a1"));
        assert_eq!(id.dns_names, ["rig-a1.farm.local", "rig-a1"]);
        assert_eq!(peer_identity(b"not a certificate"), None);
    }

    #[test]
    fn garbage_in_the_client_ca_is_rejected_not_ignored() {
        let (cert, key) = self_signed();
        let message = err(&config(
            &cert,
            &key,
            Some(b"-----BEGIN CERTIFICATE-----\nnot base64!!\n-----END CERTIFICATE-----\n"),
        ));
        assert!(message.contains("client CA"), "{message}");
    }

    #[test]
    fn non_pem_private_key_reports_the_parser_error() {
        let err =
            parse_private_key(b"-----BEGIN PRIVATE KEY-----\n!!!\n-----END PRIVATE KEY-----\n")
                .unwrap_err();
        assert!(!err.is_empty());
        assert!(
            parse_private_key(b"")
                .unwrap_err()
                .contains("no private key")
        );
    }
}
