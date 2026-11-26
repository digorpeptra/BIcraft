//! TLS for TDS 7.x ([MS-TDS] 2.2.6.5, 3.2.5.2): after `PRELOGIN`, the TLS handshake travels in
//! the payload of `PRELOGIN` packets; once it is done, every TDS packet is sent inside TLS
//! records on the bare TCP stream. BIcraft always asks for encryption (`ENCRYPT_ON`) and never
//! falls back to an unencrypted connection.
//!
//! The server certificate is checked against the bundled Mozilla roots (`webpki-roots`) and the
//! extra roots of [`Settings::extra_roots`](crate::Settings), for the host name of the server;
//! [`Settings::trust_server_certificate`](crate::Settings) skips that check (a self-signed
//! certificate) while still checking the handshake's signatures.

use std::io::Read;
use std::net::TcpStream;
use std::sync::Arc;

use bicraft_data::connect::{ConnectError, ConnectErrorKind};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::WebPkiSupportedAlgorithms;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::packet::{self, PRELOGIN, REPLY};
use crate::wire::{io_error, protocol};

fn tls_error(e: &rustls::Error) -> ConnectError {
    ConnectError::new(
        ConnectErrorKind::Network,
        format!(
            "SQL Server: the TLS connection failed ({e}); a server with a self-signed certificate \
             needs its CA (BICRAFT_SQLSERVER_CA_FILE) or BICRAFT_SQLSERVER_TRUST_SERVER_CERTIFICATE=1"
        ),
    )
}

/// Accepts any certificate, still checking the handshake signatures with the provider's
/// algorithms.
#[derive(Debug)]
struct TrustAny(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for TrustAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}

/// The client configuration of `settings`.
pub fn config(extra_roots: &[Vec<u8>], trust_any: bool) -> Result<Arc<ClientConfig>, ConnectError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|e| tls_error(&e))?;
    let config = if trust_any {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustAny(provider.signature_verification_algorithms)))
            .with_no_client_auth()
    } else {
        let mut roots = RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        for der in extra_roots {
            roots.add(CertificateDer::from(der.clone())).map_err(|e| {
                ConnectError::new(
                    ConnectErrorKind::Unsupported,
                    format!("SQL Server CA certificate: {e}"),
                )
            })?;
        }
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Ok(Arc::new(config))
}

/// The certificates of a PEM text (`-----BEGIN CERTIFICATE-----` blocks).
pub fn pem_certificates(text: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    use rustls::pki_types::pem::PemObject;
    let certs: Vec<Vec<u8>> = CertificateDer::pem_slice_iter(text)
        .map(|c| c.map(|c| c.as_ref().to_vec()))
        .collect::<Result<_, _>>()
        .map_err(|e| format!("not a PEM certificate file ({e:?})"))?;
    if certs.is_empty() {
        return Err("no certificate in the file".into());
    }
    Ok(certs)
}

/// The TLS handshake inside `PRELOGIN` packets; then the stream is TLS.
pub fn handshake(
    tcp: &mut TcpStream,
    config: Arc<ClientConfig>,
    host: &str,
) -> Result<ClientConnection, ConnectError> {
    let name = ServerName::try_from(host.to_string()).map_err(|_| {
        ConnectError::new(
            ConnectErrorKind::Unsupported,
            format!("`{host}` is not a host name"),
        )
    })?;
    let mut conn = ClientConnection::new(config, name).map_err(|e| tls_error(&e))?;
    for _ in 0..64 {
        if conn.wants_write() {
            let mut out = Vec::new();
            while conn.wants_write() {
                conn.write_tls(&mut out).map_err(|e| io_error(&e))?;
            }
            packet::write_message(tcp, PRELOGIN, &out, 4096).map_err(|e| io_error(&e))?;
            continue;
        }
        if !conn.is_handshaking() {
            return Ok(conn);
        }
        let mut msg = Vec::new();
        packet::Message::new(tcp, &[PRELOGIN, REPLY], 1 << 20)
            .read_to_end(&mut msg)
            .map_err(|e| io_error(&e))?;
        if msg.is_empty() {
            return Err(protocol("the server sent an empty TLS handshake message"));
        }
        let mut rd = msg.as_slice();
        while !rd.is_empty() {
            conn.read_tls(&mut rd).map_err(|e| io_error(&e))?;
            conn.process_new_packets().map_err(|e| tls_error(&e))?;
        }
    }
    Err(protocol("the TLS handshake did not finish"))
}
