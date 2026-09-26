//! TLS: the local CA, leaf certificates made on demand, and the SNI resolver.
//!
//! See ADR 01, change 4. Invariants: I5 (leaf only for routed `.localhost`
//! names), I6 (`ca.key` is 0600 and never leaves the folder), I7 (an existing
//! CA is never replaced except by an explicit reset).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use time::OffsetDateTime;

use crate::paths::Paths;
use crate::routes::{HELP_HOST, host_key};

pub const CA_VALIDITY_DAYS: i64 = 3650;
pub const LEAF_VALIDITY_DAYS: i64 = 90;
/// A cached leaf is made again after this age, long before it expires.
const LEAF_REFRESH_AFTER: Duration = Duration::from_secs(60 * 24 * 3600);

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("certificate: {0}")]
    Cert(#[from] rcgen::Error),
    #[error("rustls: {0}")]
    Rustls(#[from] rustls::Error),
}

/// A loaded local CA.
pub struct LocalCa {
    issuer: Issuer<'static, KeyPair>,
    cert_der: CertificateDer<'static>,
    common_name: String,
}

impl std::fmt::Debug for LocalCa {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalCa").field("common_name", &self.common_name).finish_non_exhaustive()
    }
}

/// Result of loading the CA folder.
#[derive(Debug)]
pub enum CaLoad {
    Ready(Box<LocalCa>),
    /// The folder exists but cannot be used. HTTPS stays off (I7).
    Broken(String),
}

impl LocalCa {
    pub fn common_name(&self) -> &str {
        &self.common_name
    }

    pub fn cert_der(&self) -> &CertificateDer<'static> {
        &self.cert_der
    }

    /// Load `ca/`, or create it when it does not exist. Never replaces an
    /// existing CA: a damaged one is reported as [`CaLoad::Broken`].
    pub fn load_or_create(paths: &Paths) -> CaLoad {
        remove_leftover_tmp_dirs(&paths.data);
        if !paths.ca_dir().exists() {
            return match Self::create(paths) {
                Ok(ca) => CaLoad::Ready(Box::new(ca)),
                Err(e) => CaLoad::Broken(format!("could not create the CA: {e}")),
            };
        }
        match Self::load(paths) {
            Ok(ca) => CaLoad::Ready(Box::new(ca)),
            Err(reason) => CaLoad::Broken(reason),
        }
    }

    fn load(paths: &Paths) -> Result<Self, String> {
        let key_pem = fs::read_to_string(paths.ca_key())
            .map_err(|e| format!("cannot read {}: {e}", paths.ca_key().display()))?;
        let cert_pem = fs::read_to_string(paths.ca_pem())
            .map_err(|e| format!("cannot read {}: {e}", paths.ca_pem().display()))?;
        let key = KeyPair::from_pem(&key_pem).map_err(|e| format!("ca.key does not parse: {e}"))?;
        let der = pem_to_der(&cert_pem).ok_or("ca.pem holds no certificate")?;
        let common_name = common_name_of(&der).ok_or("ca.pem does not parse")?;
        let issuer = Issuer::from_ca_cert_pem(&cert_pem, key).map_err(|e| format!("ca.pem is not usable: {e}"))?;
        Ok(Self { issuer, cert_der: der, common_name })
    }

    /// Create a new CA into `ca.tmp-<pid>/`, then rename the folder to `ca/`,
    /// so `ca/` either holds both files or does not exist.
    fn create(paths: &Paths) -> Result<Self, TlsError> {
        fs::create_dir_all(&paths.data)?;
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let common_name = format!("LocalRouter CA {}", short_id(&key));

        let mut params = CertificateParams::default();
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, common_name.clone());
        dn.push(DnType::OrganizationName, "LocalRouter");
        params.distinguished_name = dn;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
        let now = OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(CA_VALIDITY_DAYS);
        #[cfg(feature = "name-constraints")]
        {
            params.name_constraints = Some(rcgen::NameConstraints {
                permitted_subtrees: vec![rcgen::GeneralSubtree::DnsName(crate::TLD.into())],
                excluded_subtrees: vec![],
            });
        }
        let cert = params.self_signed(&key)?;

        let tmp = paths.data.join(format!("ca.tmp-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir(&tmp)?;
        write_file(&tmp.join("ca.key"), key.serialize_pem().as_bytes(), 0o600)?;
        write_file(&tmp.join("ca.pem"), cert.pem().as_bytes(), 0o644)?;
        fsync_dir(&tmp)?;
        fs::rename(&tmp, paths.ca_dir())?;
        fsync_dir(&paths.data)?;

        let issuer = Issuer::new(params, key);
        Ok(Self { issuer, cert_der: cert.der().clone(), common_name })
    }

    /// Delete the CA and make a new one. Only for an explicit user action.
    pub fn reset(paths: &Paths) -> Result<Self, TlsError> {
        if paths.ca_dir().exists() {
            fs::remove_dir_all(paths.ca_dir())?;
        }
        Self::create(paths)
    }

    /// A 90-day certificate for exactly one name.
    pub fn issue_leaf(&self, name: &str) -> Result<CertifiedKey, TlsError> {
        let mut params = CertificateParams::new(vec![name.to_string()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        params.distinguished_name = dn;
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.use_authority_key_identifier_extension = true;
        let now = OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::hours(1);
        params.not_after = now + time::Duration::days(LEAF_VALIDITY_DAYS);

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let cert = params.signed_by(&key, &self.issuer)?;
        let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let signing_key = rustls::crypto::ring::sign::any_ecdsa_type(&key_der)?;
        Ok(CertifiedKey::new(vec![cert.der().clone(), self.cert_der.clone()], signing_key))
    }
}

fn common_name_of(der: &CertificateDer<'_>) -> Option<String> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(der).ok()?;
    let cn = cert.subject().iter_common_name().next()?.as_str().ok()?.to_string();
    Some(cn)
}

fn short_id(key: &KeyPair) -> String {
    use rcgen::PublicKeyData;
    let bytes = key.der_bytes();
    let tail = &bytes[bytes.len().saturating_sub(4)..];
    tail.iter().map(|b| format!("{b:02x}")).collect()
}

fn pem_to_der(pem: &str) -> Option<CertificateDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    CertificateDer::from_pem_slice(pem.as_bytes()).ok()
}

/// Create a file with its final mode already set, write it and fsync it.
fn write_file(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

fn fsync_dir(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}

fn remove_leftover_tmp_dirs(data: &Path) {
    let Ok(entries) = fs::read_dir(data) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with("ca.tmp-") {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Decides whether a TLS name may get a certificate (it must have a route).
/// `router.localhost` (the help page) always gets one.
pub type AllowName = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Holds the current CA and the leaf cache; answers SNI lookups.
pub struct CertStore {
    ca: RwLock<Option<Arc<LocalCa>>>,
    leaves: Mutex<HashMap<String, (Arc<CertifiedKey>, Instant)>>,
    allow: AllowName,
}

impl std::fmt::Debug for CertStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertStore").finish_non_exhaustive()
    }
}

impl CertStore {
    pub fn new(ca: Option<LocalCa>, allow: AllowName) -> Self {
        Self { ca: RwLock::new(ca.map(Arc::new)), leaves: Mutex::new(HashMap::new()), allow }
    }

    /// Replace the CA (after a reset). Drops every cached leaf.
    pub fn set_ca(&self, ca: Option<LocalCa>) {
        *self.ca.write().unwrap() = ca.map(Arc::new);
        self.leaves.lock().unwrap().clear();
    }

    pub fn has_ca(&self) -> bool {
        self.ca.read().unwrap().is_some()
    }

    pub fn common_name(&self) -> Option<String> {
        self.ca.read().unwrap().as_ref().map(|ca| ca.common_name.clone())
    }

    /// A certificate for `name`, or `None` when the name must be refused (I5).
    pub fn cert_for(&self, name: &str) -> Option<Arc<CertifiedKey>> {
        if host_key(name)? != HELP_HOST && !(self.allow)(name) {
            return None;
        }
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if let Some((cert, made)) = self.leaves.lock().unwrap().get(&name)
            && made.elapsed() < LEAF_REFRESH_AFTER
        {
            return Some(cert.clone());
        }
        let ca = self.ca.read().unwrap().clone()?;
        match ca.issue_leaf(&name) {
            Ok(cert) => {
                let cert = Arc::new(cert);
                self.leaves.lock().unwrap().insert(name, (cert.clone(), Instant::now()));
                Some(cert)
            }
            Err(e) => {
                tracing::warn!("could not issue a certificate for {name}: {e}");
                None
            }
        }
    }
}

/// rustls adapter: no SNI, no name under `.localhost`, or no route → no
/// certificate, and the handshake fails.
#[derive(Debug)]
pub struct SniResolver(pub Arc<CertStore>);

impl ResolvesServerCert for SniResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.0.cert_for(hello.server_name()?)
    }
}

pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Server config for port 443: SNI resolver and ALPN `h2`, `http/1.1`.
pub fn server_config(store: Arc<CertStore>) -> Result<Arc<rustls::ServerConfig>, TlsError> {
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(SniResolver(store)));
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Client config for `https://` targets. It does NOT verify the dev server's
/// certificate: dev servers use self-signed certificates, and a target is
/// always a loopback address (I9), so no other machine can be in the middle.
pub fn insecure_loopback_client_config() -> Arc<rustls::ClientConfig> {
    let config = rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring supports the default versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();
    Arc::new(config)
}

#[derive(Debug)]
struct AcceptAnyCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        provider().signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use x509_parser::extensions::ParsedExtension;
    use x509_parser::prelude::*;

    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path().to_path_buf());
        (dir, paths)
    }

    fn ready(load: CaLoad) -> LocalCa {
        match load {
            CaLoad::Ready(ca) => *ca,
            CaLoad::Broken(why) => panic!("CA broken: {why}"),
        }
    }

    // T2, I6
    #[test]
    fn new_ca_key_is_private_from_the_start() {
        let (_d, p) = paths();
        let ca = ready(LocalCa::load_or_create(&p));
        let mode = fs::metadata(p.ca_key()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(ca.common_name().starts_with("LocalRouter CA "));
        assert!(fs::read_to_string(p.ca_pem()).unwrap().contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn existing_ca_is_loaded_not_replaced() {
        let (_d, p) = paths();
        let first = ready(LocalCa::load_or_create(&p));
        let pem = fs::read(p.ca_pem()).unwrap();
        let second = ready(LocalCa::load_or_create(&p));
        assert_eq!(first.common_name(), second.common_name());
        assert_eq!(pem, fs::read(p.ca_pem()).unwrap());
    }

    // T2, I7
    #[test]
    fn damaged_ca_is_reported_and_left_alone() {
        let (_d, p) = paths();
        ready(LocalCa::load_or_create(&p));
        fs::remove_file(p.ca_key()).unwrap();
        let pem = fs::read(p.ca_pem()).unwrap();
        match LocalCa::load_or_create(&p) {
            CaLoad::Broken(why) => assert!(why.contains("ca.key"), "{why}"),
            CaLoad::Ready(_) => panic!("a damaged CA must not load"),
        }
        assert!(!p.ca_key().exists(), "no new key may be written");
        assert_eq!(pem, fs::read(p.ca_pem()).unwrap());
    }

    #[test]
    fn leftover_tmp_folder_is_deleted() {
        let (_d, p) = paths();
        fs::create_dir_all(p.data.join("ca.tmp-123")).unwrap();
        ready(LocalCa::load_or_create(&p));
        assert!(!p.data.join("ca.tmp-123").exists());
    }

    #[test]
    fn reset_makes_a_new_ca() {
        let (_d, p) = paths();
        let old = ready(LocalCa::load_or_create(&p));
        let new = LocalCa::reset(&p).unwrap();
        assert_ne!(old.common_name(), new.common_name());
    }

    // T2, I5
    #[test]
    fn leaf_has_one_name_server_auth_and_90_days() {
        let (_d, p) = paths();
        let ca = ready(LocalCa::load_or_create(&p));
        let leaf = ca.issue_leaf("feat.shop.localhost").unwrap();
        assert_eq!(leaf.cert.len(), 2, "leaf plus CA");
        let (_, cert) = X509Certificate::from_der(&leaf.cert[0]).unwrap();

        let mut sans = vec![];
        let mut server_auth = false;
        for ext in cert.extensions() {
            match ext.parsed_extension() {
                ParsedExtension::SubjectAlternativeName(san) => {
                    for name in &san.general_names {
                        if let GeneralName::DNSName(n) = name {
                            sans.push(n.to_string());
                        }
                    }
                }
                ParsedExtension::ExtendedKeyUsage(eku) => server_auth = eku.server_auth,
                _ => {}
            }
        }
        assert_eq!(sans, ["feat.shop.localhost"]);
        assert!(server_auth);
        let days = (cert.validity().not_after.timestamp() - cert.validity().not_before.timestamp()) / 86400;
        assert!((LEAF_VALIDITY_DAYS - 1..=LEAF_VALIDITY_DAYS).contains(&days), "{days} days");
        assert!(days <= 825, "Apple limit");
    }

    // T2, I5
    #[test]
    fn store_refuses_foreign_names_and_names_without_route() {
        let (_d, p) = paths();
        let ca = ready(LocalCa::load_or_create(&p));
        let allow: AllowName = Arc::new(|name: &str| name.ends_with("shop.localhost"));
        let store = CertStore::new(Some(ca), allow);
        assert!(store.cert_for("feat.shop.localhost").is_some());
        assert!(store.cert_for("example.com").is_none());
        assert!(store.cert_for("blog.localhost").is_none());
        let a = store.cert_for("shop.localhost").unwrap();
        let b = store.cert_for("shop.localhost").unwrap();
        assert!(Arc::ptr_eq(&a, &b), "cached");
    }

    #[test]
    fn store_without_ca_gives_nothing() {
        let store = CertStore::new(None, Arc::new(|_: &str| true));
        assert!(store.cert_for("shop.localhost").is_none());
    }
}
