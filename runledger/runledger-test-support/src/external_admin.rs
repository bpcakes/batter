//! Checks on an external `RUNLEDGER_TEST_ADMIN_DATABASE_URL` before any
//! database is provisioned through it.

use std::net::IpAddr;

use url::Url;

/// Why derived database URLs would reach a different endpoint than the
/// external admin URL, if they would.
///
/// Derived database URLs keep the authority and drop the query, so any query
/// parameter reaches the admin pool but not the leases: `host` or `port`
/// would move them to another endpoint, and `sslmode`, `sslrootcert` and the
/// other TLS settings would leave them on SQLx's default `prefer`. Every key
/// is therefore refused, except a `hostaddr` that repeats the authority's own
/// IP literal, where dropping it changes nothing. Ambient `PG*` variables are
/// refused separately in every fixture mode, so an external fixture has no
/// way to require TLS for its leases. Query keys are read decoded, as SQLx
/// reads them. An unparseable URL is left to SQLx's own error.
pub(crate) fn dropped_endpoint_parameter(admin_url: &str) -> Option<&'static str> {
    let url = Url::parse(admin_url).ok()?;
    // Lease URLs replace the database path segment; without exactly one, the
    // last-"/" split would land inside "://" or keep extra segments.
    let database = url.path().strip_prefix('/').unwrap_or("");
    if database.is_empty() || database.contains('/') {
        return Some(
            "give RUNLEDGER_TEST_ADMIN_DATABASE_URL exactly one database path segment: lease URLs \
             replace it",
        );
    }
    // Native test proxies dial (host, port) over TCP themselves: a Unix socket
    // directory or a bracketed IPv6 literal would not reach them.
    let host = url.host_str().unwrap_or("");
    if host.is_empty() || host.starts_with('[') || host.contains('%') {
        return Some(
            "give RUNLEDGER_TEST_ADMIN_DATABASE_URL a DNS name or IPv4 host: native test proxies \
             connect over TCP to that host",
        );
    }
    let mut pins = Vec::new();
    for (key, value) in url.query_pairs() {
        if key == "hostaddr" {
            pins.push(value.into_owned());
        } else {
            return Some(
                "remove the query from RUNLEDGER_TEST_ADMIN_DATABASE_URL (host and port belong in \
                 its authority): derived database URLs carry no query, so their TLS and endpoint \
                 settings would differ from the admin connection's",
            );
        }
    }
    if pins.is_empty() {
        return None;
    }
    // `postgres` is not a special URL scheme, so an IPv4 host arrives as text.
    let authority = url
        .host_str()
        .map(|host| host.trim_start_matches('[').trim_end_matches(']'))
        .and_then(|host| host.parse::<IpAddr>().ok());
    let repeats_authority = authority.is_some()
        && pins
            .iter()
            .all(|pin| pin.parse::<IpAddr>().ok() == authority);
    (!repeats_authority).then_some(
        "remove hostaddr from RUNLEDGER_TEST_ADMIN_DATABASE_URL or put that address in its \
         host: derived database URLs carry no query, so they would reach the URL host",
    )
}

/// Panic before provisioning when [`dropped_endpoint_parameter`] finds a
/// split. The message never quotes the URL, which can carry credentials.
pub(crate) fn reject_dropped_endpoint_parameters(admin_url: &str) {
    if let Some(reason) = dropped_endpoint_parameter(admin_url) {
        panic!("{reason}");
    }
}

#[cfg(test)]
mod tests {
    use super::dropped_endpoint_parameter;

    #[test]
    fn endpoint_parameters_derived_urls_would_drop_are_refused() {
        for url in [
            "postgres://u:p@db.invalid:5432/postgres",
            "postgres://u:p@127.0.0.1:5432/postgres?hostaddr=127.0.0.1",
            "not a url",
        ] {
            assert_eq!(dropped_endpoint_parameter(url), None, "{url}");
        }
        for url in [
            // A socket directory in the query: provisioning uses the socket,
            // derived URLs TCP to the authority host.
            "postgres://u:p@localhost/postgres?host=/tmp",
            "postgres://u:p@localhost/postgres?host=192.0.2.1",
            "postgres://u:p@localhost/postgres?port=6543",
            // TLS settings the leases would lose.
            "postgres://u:p@db.invalid/postgres?sslmode=verify-full&sslrootcert=/ca/root.pem",
            "postgres://u:p@db.invalid/postgres?sslmode=disable",
            "postgres://u:p@db.invalid/postgres?ss%6Cmode=require",
            // No database path, or more than one segment: the lease URL would
            // lose its authority or keep stray segments.
            "postgres://alice@db.example",
            "postgres://alice@db.example/",
            "postgres://alice@db.example/one/two",
            // Transports the native proxies cannot dial.
            "postgres://u:p@%2Ftmp/postgres",
            "postgres://u:p@[::1]:5432/postgres?hostaddr=::1",
            // A pin naming another address than the authority host.
            "postgres://u:p@db.invalid:5432/postgres?hostaddr=192.0.2.1",
            "postgres://u:p@127.0.0.1:5432/postgres?hostaddr=192.0.2.1",
            // Keys are decoded as SQLx decodes them.
            "postgres://u:p@db.invalid:5432/postgres?host%61ddr=192.0.2.1",
            "postgres://u:p@localhost/postgres?%68ost=/tmp",
            "postgres://u:p@127.0.0.1/postgres?hostaddr=127.0.0.1&hostaddr=192.0.2.1",
        ] {
            // Refused, without quoting the URL's address.
            assert!(
                dropped_endpoint_parameter(url)
                    .is_some_and(|reason| !reason.contains("192.0.2.1") && !reason.contains("/tmp")),
                "{url}"
            );
        }
    }
}
