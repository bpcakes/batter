//! Checks on an external `RUNLEDGER_TEST_ADMIN_DATABASE_URL` before any
//! database is provisioned through it.

use std::net::IpAddr;

use url::Url;

/// Why an external admin URL's `hostaddr` would split provisioning from the
/// database URLs derived from it, if it would.
///
/// SQLx connects the admin pool to a URL `hostaddr`, but derived database URLs
/// carry no query, so they reach the authority host instead (or, in SQLx
/// 0.9.1, an ambient `PGHOSTADDR`). A pin is harmless only when it repeats
/// the authority's own IP literal and no ambient address overrides the derived
/// URLs. Query keys are read decoded, as SQLx reads them. An unparseable URL
/// is left to SQLx's own connection error.
pub(crate) fn dropped_host_address_pin(
    admin_url: &str,
    ambient_present: bool,
) -> Option<&'static str> {
    let url = Url::parse(admin_url).ok()?;
    let pins: Vec<String> = url
        .query_pairs()
        .filter(|(key, _)| key == "hostaddr")
        .map(|(_, value)| value.into_owned())
        .collect();
    if pins.is_empty() {
        return None;
    }
    if ambient_present {
        return Some(
            "unset PGHOSTADDR or remove hostaddr from RUNLEDGER_TEST_ADMIN_DATABASE_URL: derived \
             database URLs would follow PGHOSTADDR while provisioning follows the pin",
        );
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

/// Panic before provisioning when [`dropped_host_address_pin`] finds a split.
/// The message never quotes the URL, which can carry credentials.
pub(crate) fn reject_dropped_host_address_pin(admin_url: &str, ambient_present: bool) {
    if let Some(reason) = dropped_host_address_pin(admin_url, ambient_present) {
        panic!("{reason}");
    }
}

#[cfg(test)]
mod tests {
    use super::dropped_host_address_pin;

    #[test]
    fn pins_derived_urls_would_drop_are_refused() {
        let accepted = [
            ("postgres://u:p@db.invalid:5432/postgres", false),
            ("postgres://u:p@db.invalid:5432/postgres", true),
            (
                "postgres://u:p@127.0.0.1:5432/postgres?hostaddr=127.0.0.1",
                false,
            ),
            ("postgres://u:p@[::1]:5432/postgres?hostaddr=::1", false),
            ("not a url", true),
        ];
        for (url, ambient) in accepted {
            assert_eq!(
                dropped_host_address_pin(url, ambient),
                None,
                "{url} {ambient}"
            );
        }
        let refused = [
            // The 0.9.1 split: an ambient address overrides only derived URLs.
            (
                "postgres://u:p@127.0.0.1:5432/postgres?hostaddr=127.0.0.1",
                true,
            ),
            // Already split under 0.9.0: derived URLs reach the host name.
            (
                "postgres://u:p@db.invalid:5432/postgres?hostaddr=192.0.2.1",
                false,
            ),
            (
                "postgres://u:p@127.0.0.1:5432/postgres?hostaddr=192.0.2.1",
                false,
            ),
            // Keys are decoded as SQLx decodes them.
            (
                "postgres://u:p@db.invalid:5432/postgres?host%61ddr=192.0.2.1",
                false,
            ),
            (
                "postgres://u:p@127.0.0.1/postgres?hostaddr=127.0.0.1&hostaddr=192.0.2.1",
                false,
            ),
        ];
        for (url, ambient) in refused {
            let reason = dropped_host_address_pin(url, ambient);
            // Refused, without quoting the URL's address.
            assert!(
                reason.is_some_and(|reason| !reason.contains("192.0.2.1")),
                "{url} {ambient}"
            );
        }
    }
}
