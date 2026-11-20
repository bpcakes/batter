//! The URL handed to template initializers, rebuilt from the harness's parse
//! and checked by reading it back with both clients.

use sqlx::{
    ConnectOptions,
    postgres::{PgConnectOptions, PgSslMode},
};

use super::{Address, Environment, HarnessEndpoint, HarnessEndpointReason, raw_options, refusal};

impl HarnessEndpoint {
    /// A minimal URL for `database` that tokio-postgres and SQLx both read back
    /// as this endpoint, for callers such as template initializers that take a
    /// URL. Refuses rather than hand out a URL either parser reads differently.
    pub(super) fn url(&self, database: &str) -> Result<String, sqlx::Error> {
        let host = match &self.address {
            Address::Tcp(name) if name.contains(':') => format!("[{name}]"),
            Address::Tcp(name)
                if name
                    .bytes()
                    // URI unreserved characters, which both parsers keep literally.
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)) =>
            {
                name.clone()
            }
            Address::Tcp(_) => return Err(refusal(HarnessEndpointReason::RoundTrip)),
            Address::Unix(path) => strict_encode(
                path.to_str()
                    .ok_or_else(|| refusal(HarnessEndpointReason::RoundTrip))?,
            ),
        };
        let mut url = format!("postgres://{}", strict_encode(&self.user));
        // SQLx reads an empty userinfo password as none and may then consult a
        // passfile, so an empty password travels as an explicit parameter.
        let password = self.password.as_str();
        if !password.is_empty() {
            url.push(':');
            url.push_str(&strict_encode(password));
        }
        url.push_str(&format!(
            "@{host}:{}/{}",
            self.port,
            strict_encode(database)
        ));
        url.push_str("?sslmode=disable");
        if let Some(name) = &self.application_name {
            url.push_str(&format!("&application_name={}", strict_encode(name)));
        }
        if let Some(raw) = self.options.as_deref().and_then(raw_options) {
            url.push_str(&format!("&options={}", strict_encode(&raw)));
        }
        if self.channel_binding_disabled {
            url.push_str("&channel_binding=disable");
        }
        if let Some(timeout) = self.connect_timeout {
            // tokio-postgres parsed whole seconds, so this is exact.
            url.push_str(&format!("&connect_timeout={}", timeout.as_secs()));
        }
        if password.is_empty() {
            url.push_str("&password=");
        }
        self.check_round_trip(&url, database, password)?;
        Ok(url)
    }

    fn check_round_trip(
        &self,
        url: &str,
        database: &str,
        password: &str,
    ) -> Result<(), sqlx::Error> {
        let mismatch = || refusal(HarnessEndpointReason::RoundTrip);
        // tokio-postgres's parse, as the harness and many initializers read it.
        let again = Self::from_admin_url(url, Environment::CLEAN).map_err(|_| mismatch())?;
        if again.admin_database != database || !self.same_server_and_login(&again) {
            return Err(mismatch());
        }
        // SQLx has no password getter. Check the credential in the Url that
        // its parser consumes *before* calling from_url, which opens .pgpass
        // if neither userinfo nor a password query parameter supplies one.
        let parsed = url::Url::parse(url).map_err(|_| mismatch())?;
        let query_password = parsed
            .query_pairs()
            .filter(|(key, _)| key == "password")
            .last();
        let supplied = match query_password {
            Some((_, value)) => password.is_empty() && value.is_empty(),
            None => parsed
                .password()
                .is_some_and(|value| value == strict_encode(password)),
        };
        if !supplied {
            return Err(mismatch());
        }
        let native = PgConnectOptions::from_url(&parsed).map_err(|_| mismatch())?;
        let same_address = match &self.address {
            // SQLx keeps the URL brackets around an IPv6 host.
            Address::Tcp(name) => {
                native.get_socket().is_none()
                    && native
                        .get_host()
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        == name
            }
            Address::Unix(path) => native.get_socket() == Some(path),
        };
        if !same_address
            || native.get_port() != self.port
            || native.get_username() != self.user
            || native.get_database() != Some(database)
            || !matches!(native.get_ssl_mode(), PgSslMode::Disable)
            || native.get_application_name() != self.application_name.as_deref()
            || native.get_options().map(str::to_owned)
                != self.options.as_deref().and_then(raw_options)
        {
            return Err(mismatch());
        }
        Ok(())
    }
}

/// Percent-encode everything but RFC 3986 unreserved bytes.
fn strict_encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
