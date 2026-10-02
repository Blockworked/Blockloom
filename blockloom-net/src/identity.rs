use std::fmt;
use std::str::FromStr;

use boring::pkey::{PKey, Private};
use boring::x509::X509;
use ring::digest;

use crate::NetError;

/// SHA-256 of a certificate's DER bytes: what an invite carries and a client pins.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn of_der(der: &[u8]) -> Self {
        let d = digest::digest(&digest::SHA256, der);
        let mut out = [0u8; 32];
        out.copy_from_slice(d.as_ref());
        Fingerprint(out)
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

impl FromStr for Fingerprint {
    type Err = NetError;

    fn from_str(s: &str) -> Result<Self, NetError> {
        let s: String = s.chars().filter(|c| *c != ':').collect();
        if s.len() != 64 || !s.is_ascii() {
            return Err(NetError::Tls("fingerprint must be 64 hex digits".into()));
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| NetError::Tls("fingerprint must be hex".into()))?;
        }
        Ok(Fingerprint(out))
    }
}

/// A server's self-signed certificate and key, held in memory only.
pub struct Identity {
    cert: X509,
    key: PKey<Private>,
    fingerprint: Fingerprint,
}

impl Identity {
    pub fn generate(names: &[&str]) -> Result<Self, NetError> {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        let made =
            rcgen::generate_simple_self_signed(names).map_err(|e| NetError::Tls(e.to_string()))?;
        Self::from_pem(
            made.cert.pem().as_bytes(),
            made.signing_key.serialize_pem().as_bytes(),
        )
    }

    pub fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> Result<Self, NetError> {
        let cert = X509::from_pem(cert_pem)?;
        let key = PKey::private_key_from_pem(key_pem)?;
        let fingerprint = Fingerprint::of_der(&cert.to_der()?);
        Ok(Identity {
            cert,
            key,
            fingerprint,
        })
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn cert(&self) -> &X509 {
        &self.cert
    }

    pub fn key(&self) -> &PKey<Private> {
        &self.key
    }
}
