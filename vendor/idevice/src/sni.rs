// Jackson Coxson
// https://gist.github.com/doroved/2c92ddd5e33f257f901c763b728d1b61
//
// Why:
// https://github.com/rustls/rustls/issues/578
// We aren't in control of the certs served.
// Assuming that there's no use for unchecked certs is naive.

use rustls::{
    ClientConfig, DigitallySignedStruct,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime, pem::PemObject},
};
use std::sync::Arc;

use crate::{IdeviceError, pairing_file::PairingFile};

#[derive(Debug)]
pub struct PairedDeviceCertificateVerifier {
    inner: Arc<WebPkiServerVerifier>,
    expected_device_certificate: CertificateDer<'static>,
}

impl PairedDeviceCertificateVerifier {
    pub fn new(
        inner: Arc<WebPkiServerVerifier>,
        expected_device_certificate: CertificateDer<'static>,
    ) -> Self {
        Self {
            inner,
            expected_device_certificate,
        }
    }
}

impl ServerCertVerifier for PairedDeviceCertificateVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if !matches_pairing_certificate(
            end_entity.as_ref(),
            self.expected_device_certificate.as_ref(),
        ) {
            return Err(rustls::Error::General(
                "iOS TLS certificate does not match the trusted pairing record".into(),
            ));
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

pub fn create_client_config(pairing_file: &PairingFile) -> Result<ClientConfig, IdeviceError> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.add(pairing_file.root_certificate.clone())?;
    let private_key = PrivateKeyDer::from_pem_slice(&pairing_file.host_private_key)?;

    let mut config = ClientConfig::builder()
        .with_root_certificates(root_store.clone())
        .with_client_auth_cert(vec![pairing_file.host_certificate.clone()], private_key)
        .unwrap();

    let inner = rustls::client::WebPkiServerVerifier::builder(Arc::new(root_store)).build()?;
    let verifier = Arc::new(PairedDeviceCertificateVerifier::new(
        inner,
        pairing_file.device_certificate.clone(),
    ));
    config.dangerous().set_certificate_verifier(verifier);

    Ok(config)
}

fn matches_pairing_certificate(received: &[u8], expected: &[u8]) -> bool {
    received == expected
}

#[cfg(test)]
mod tests {
    use super::{PairedDeviceCertificateVerifier, matches_pairing_certificate};
    use rustls::{
        client::WebPkiServerVerifier,
        client::danger::ServerCertVerifier,
        pki_types::{CertificateDer, ServerName, UnixTime},
    };
    use std::sync::Arc;

    #[test]
    fn server_certificate_must_match_the_pairing_record() {
        let paired_certificate = b"trusted device certificate";
        assert!(matches_pairing_certificate(
            paired_certificate,
            paired_certificate
        ));
        assert!(!matches_pairing_certificate(
            b"other certificate",
            paired_certificate
        ));
    }

    #[test]
    fn verifier_rejects_a_certificate_not_in_the_pairing_record() {
        let _ = rustls::crypto::CryptoProvider::install_default(rustls_rustcrypto::provider());
        let inner = WebPkiServerVerifier::builder(Arc::new(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        }))
        .build()
        .unwrap();
        let verifier = PairedDeviceCertificateVerifier::new(
            inner,
            CertificateDer::from(b"trusted device certificate".to_vec()),
        );
        let result = verifier.verify_server_cert(
            &CertificateDer::from(b"other certificate".to_vec()),
            &[],
            &ServerName::try_from("Device").unwrap(),
            &[],
            UnixTime::now(),
        );
        assert!(matches!(
            result,
            Err(rustls::Error::General(message))
                if message.contains("does not match the trusted pairing record")
        ));
    }
}
