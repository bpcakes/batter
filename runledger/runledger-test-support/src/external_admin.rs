//! Checks on an external `RUNLEDGER_TEST_ADMIN_DATABASE_URL` before any
//! database is provisioned through it.

use std::net::IpAddr;

use url::Url;

/// Why derived database URLs would reach a different endpoint than the
/// external admin URL, if they would.
///
/// Derived database URLs keep the authority and drop the query, so a query
/// parameter that selects the transport, port or address reaches the admin
/// pool but not the leases. SQLx 0.9.0 honoured these too, so the split
/// predates SQLx 0.9.1. `host` (a socket directory or another host) and `port`
/// are refused; a `hostaddr` is accepted only when it repeats the authority's
/// own IP literal, where dropping it changes nothing. An ambient `PGHOSTADDR`
/// is refused separately in every fixture mode. Query keys are read decoded,
/// as SQLx reads them. An unparseable URL is left to SQLx's own error.
pub(crate) fn dropped_endpoint_parameter(admin_url: &str) -> Option<&'static str> {
    let url = Url::parse(admin_url).ok()?;
    let mut pins = Vec::new();
    for (key, value) in url.query_pairs() {
        match &*key {
            "host" | "port" => {
                return Some(
                    "move host and port from the RUNLEDGER_TEST_ADMIN_DATABASE_URL query into its \
                     authority: derived database URLs carry no query",
                );
            }
            "hostaddr" => pins.push(value.into_owned()),
            _ => {}
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
            "postgres://u:p@db.invalid:5432/postgres?sslmode=disable",
            "postgres://u:p@%2Ftmp/postgres",
            "postgres://u:p@127.0.0.1:5432/postgres?hostaddr=127.0.0.1",
            "postgres://u:p@[::1]:5432/postgres?hostaddr=::1",
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
