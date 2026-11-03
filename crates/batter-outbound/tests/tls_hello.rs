//! The TLS ClientHello the fixed profile sends.
//!
//! The test build compiles reqwest's HTTP/2 support (see this package's
//! dev-dependencies), as an unrelated dependency could through feature
//! unification. The profile must still offer only HTTP/1.1 in ALPN, and TLS
//! 1.3 or 1.2.

#[allow(dead_code)]
#[path = "support/peer.rs"]
mod peer;

use batter_outbound::{ConnectFailure, Exchange, http::Method};
use peer::{Peer, Step, after, limit, millis};
use std::time::Duration;

const ALPN: u16 = 0x0010;
const SUPPORTED_VERSIONS: u16 = 0x002b;

/// The extensions of one ClientHello record, as (type, data) pairs.
fn extensions(record: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut reader = Reader(record);
    assert_eq!(reader.u8(), 0x16, "not a TLS handshake record");
    reader.skip(2);
    let record_length = usize::from(reader.u16());
    assert!(
        reader.0.len() >= record_length,
        "the ClientHello arrived in more than one read"
    );
    assert_eq!(reader.u8(), 0x01, "not a ClientHello");
    reader.skip(3 + 2 + 32);
    let session = usize::from(reader.u8());
    reader.skip(session);
    let suites = usize::from(reader.u16());
    reader.skip(suites);
    let compression = usize::from(reader.u8());
    reader.skip(compression);
    let extensions_length = usize::from(reader.u16());
    let mut list = Reader(reader.take(extensions_length));
    let mut found = Vec::new();
    while !list.0.is_empty() {
        let kind = list.u16();
        let length = usize::from(list.u16());
        found.push((kind, list.take(length).to_vec()));
    }
    found
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> &'a [u8] {
        let (taken, rest) = self.0.split_at(length);
        self.0 = rest;
        taken
    }

    fn skip(&mut self, length: usize) {
        self.take(length);
    }

    fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    fn u16(&mut self) -> u16 {
        u16::from_be_bytes([self.u8(), self.u8()])
    }
}

fn alpn_protocols(data: &[u8]) -> Vec<String> {
    let mut reader = Reader(data);
    let list_length = usize::from(reader.u16());
    let mut list = Reader(reader.take(list_length));
    let mut protocols = Vec::new();
    while !list.0.is_empty() {
        let length = usize::from(list.u8());
        protocols.push(String::from_utf8_lossy(list.take(length)).into_owned());
    }
    protocols
}

fn tls_versions(data: &[u8]) -> Vec<u16> {
    let mut reader = Reader(data);
    let list_length = usize::from(reader.u8());
    let mut list = Reader(reader.take(list_length));
    let mut versions = Vec::new();
    while !list.0.is_empty() {
        versions.push(list.u16());
    }
    versions
}

#[tokio::test]
async fn client_hello_offers_only_http1_and_modern_tls() {
    let peer = Peer::start([Step::Sniff, Step::Hold]).await;
    let prepared = peer
        .tls_client(millis(300))
        .request(Method::GET, ["v1"])
        .prepare()
        .expect("a valid request");
    let outcome = prepared
        .send(after(Duration::from_secs(3)), limit(1024))
        .await;
    assert!(
        matches!(&outcome, Exchange::NotDispatched(failure) if failure.failure() == ConnectFailure::TimedOut),
        "{outcome:?}"
    );
    peer.wait_sniffed(1).await;
    assert_eq!(peer.connections(), 1);
    let sniffed = peer.sniffed();
    let extensions = extensions(&sniffed[0]);
    let alpn = extensions
        .iter()
        .find(|(kind, _)| *kind == ALPN)
        .map(|(_, data)| alpn_protocols(data));
    assert_eq!(alpn, Some(vec!["http/1.1".to_owned()]));
    let versions = extensions
        .iter()
        .find(|(kind, _)| *kind == SUPPORTED_VERSIONS)
        .map(|(_, data)| tls_versions(data))
        .expect("a supported_versions extension");
    assert!(!versions.is_empty());
    // TLS 1.3 (0x0304) and TLS 1.2 (0x0303) only, apart from GREASE values.
    // rustls supports no older version, so this pins the TLS backend and its
    // version list rather than detecting a weaker minimum.
    for version in versions {
        let grease = version & 0x0f0f == 0x0a0a;
        assert!(
            grease || version == 0x0304 || version == 0x0303,
            "{version:#06x}"
        );
    }
    // The client passed ring explicitly: building it and sending a ClientHello
    // installed no process-default provider. The handshake never completes.
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_none(),
        "the client installed a process-default crypto provider"
    );
}
