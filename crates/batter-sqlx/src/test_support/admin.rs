//! The harness admin URL must select the same server SQLx derives from it.

use std::fmt;

use super::FixtureError;

/// Refuse a harness admin URL that could provision on a server other than the
/// one the derived lease and template URLs reach.
///
/// The harness parses the admin URL with tokio-postgres, whose query parser
/// does not stop at `#`, and then removes the fragment from every URL it
/// derives; SQLx ends the query at `#`. A parameter after a literal `#` could
/// therefore steer provisioning but never reach the fixture's pools. The URL
/// must also pass [`crate::connect_options`], which refuses an ambient or
/// declared `hostaddr`. Run before every harness operation, so a refusal
/// creates no database and calls no initializer.
pub(super) fn admin_endpoint(admin_url: &str) -> Result<(), FixtureError> {
    let refusal = if admin_url.contains('#') {
        Some(sqlx::Error::Configuration(Box::new(AdminUrlFragment)))
    } else {
        crate::connect_options(admin_url).err()
    };
    refusal.map_or(Ok(()), |source| {
        Err(FixtureError::Connect {
            source,
            cleanup: None,
        })
    })
}

/// A harness admin URL contains `#`, which the harness and SQLx read
/// differently. Encode a literal `#` as `%23`. Formatting names no URL part.
#[derive(Debug)]
struct AdminUrlFragment;

impl fmt::Display for AdminUrlFragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the fixture admin URL contains '#'; encode it as %23")
    }
}

impl std::error::Error for AdminUrlFragment {}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(url: &str) -> Option<String> {
        match admin_endpoint(url) {
            Ok(()) => None,
            Err(FixtureError::Connect {
                source: sqlx::Error::Configuration(source),
                cleanup: None,
            }) => Some(source.to_string()),
            Err(_) => panic!("refusal must be a cleanup-free Connect configuration error"),
        }
    }

    #[test]
    fn fragments_and_declared_addresses_are_refused_before_harness_work() {
        let fragment = refusal(
            "postgres://u:secret-marker@db.invalid/postgres?application_name=a#b&hostaddr=192.0.2.1",
        );
        assert!(
            fragment.as_deref().is_some_and(|text| text.contains("%23")),
            "fragment refused"
        );
        assert!(
            !fragment.unwrap().contains("secret-marker"),
            "refusal names no URL part"
        );
        let declared = refusal("postgres://u@db.invalid/postgres?hostaddr=192.0.2.1");
        assert!(declared.is_some(), "declared hostaddr refused");
        assert_eq!(
            refusal("postgres://u@127.0.0.1/postgres?application_name=a%23b"),
            None,
            "%23 is data"
        );
    }
}
