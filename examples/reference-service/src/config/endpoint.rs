use super::ConfigMode;
use batter::settings::{SecretString, SettingsError};
use sqlx::postgres::{PgConnectOptions, PgSslMode};

// Never derive Debug over native options or an unredacted URL aggregate.
#[derive(Clone)]
pub(super) struct Endpoint {
    host: SecretString,
    port: u16,
    username: SecretString,
    password: SecretString,
    database: SecretString,
    ssl: PgSslMode,
    application: SecretString,
}
fn error(reason: &'static str) -> SettingsError {
    SettingsError::new("DATABASE_URL", reason)
}

fn decode(value: &str, query: bool) -> Result<String, SettingsError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1..index + 3]
                    .iter()
                    .all(u8::is_ascii_hexdigit)
            {
                return Err(error("invalid percent encoding"));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    let value = if query {
        value.replace('+', " ")
    } else {
        value.to_owned()
    };
    let decoded = percent_encoding::percent_decode_str(&value)
        .decode_utf8()
        .map_err(|e| error("non-UTF-8 encoding").with_cause(e))?
        .into_owned();
    if decoded.contains('\0') {
        return Err(error("NUL is not permitted"));
    }
    Ok(decoded)
}

impl Endpoint {
    pub fn parse(value: &str, mode: ConfigMode) -> Result<Self, SettingsError> {
        // URL parsers normalize some whitespace; reject it rather than hide mistakes.
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(error("invalid URL syntax"));
        }
        let url = url::Url::parse(value).map_err(|e| error("invalid URL syntax").with_cause(e))?;
        if !matches!(url.scheme(), "postgres" | "postgresql") || url.fragment().is_some() {
            return Err(error("unsupported URL structure"));
        }
        let host = match url.host() {
            Some(url::Host::Ipv6(ip)) => ip.to_string(),
            Some(url::Host::Ipv4(ip)) => ip.to_string(),
            Some(url::Host::Domain(host)) if !host.is_empty() => decode(host, false)?,
            _ => return Err(error("explicit TCP host required")),
        };
        if host.contains(['/', '\\']) {
            return Err(error("explicit TCP host required"));
        }
        let username = decode(url.username(), false)?;
        let database = decode(url.path().strip_prefix('/').unwrap_or(""), false)?;
        if username.is_empty() || database.is_empty() {
            return Err(error("explicit username and database required"));
        }
        let port = url.port().unwrap_or(5432);
        if port == 0 {
            return Err(error("port must be positive"));
        }
        // Url normalizes an explicitly empty password to None. Retain presence
        // so an empty userinfo password still conflicts with a query password.
        let explicit_password = value
            .split_once("://")
            .and_then(|(_, rest)| rest.split(['/', '?', '#']).next())
            .and_then(|authority| authority.rsplit_once('@'))
            .is_some_and(|(userinfo, _)| userinfo.contains(':'));
        let mut password = if explicit_password {
            Some(decode(url.password().unwrap_or(""), false)?)
        } else {
            None
        };
        let mut ssl = None;
        let mut application = None;
        if let Some(query) = url.query() {
            for pair in query.split('&') {
                let (key, value) = pair
                    .split_once('=')
                    .ok_or_else(|| error("invalid query syntax"))?;
                let key = decode(key, true)?;
                let value = decode(value, true)?;
                match key.as_str() {
                    "password" if password.is_none() => password = Some(value),
                    "sslmode" if ssl.is_none() => {
                        ssl = Some(
                            value
                                .parse::<PgSslMode>()
                                .map_err(|e| error("invalid TLS mode").with_cause(e))?,
                        )
                    }
                    "application_name" if application.is_none() => application = Some(value),
                    "password" | "sslmode" | "application_name" => {
                        return Err(error("duplicate or conflicting query setting"));
                    }
                    _ => return Err(error("unsupported query setting")),
                }
            }
        }
        let ssl = ssl.ok_or_else(|| error("explicit sslmode required"))?;
        if mode == ConfigMode::Serve && password.as_ref().is_none_or(String::is_empty) {
            return Err(error("serving requires explicit nonempty password"));
        }
        Ok(Self {
            host: SecretString::new(host),
            port,
            username: SecretString::new(username),
            password: SecretString::new(password.unwrap_or_default()),
            database: SecretString::new(database),
            ssl,
            application: SecretString::new(
                application.unwrap_or_else(|| "reference-service".into()),
            ),
        })
    }
    pub fn connect_options_from_process(&self) -> Result<PgConnectOptions, SettingsError> {
        // Defend even injected loaders from SQLx's ambient native defaults.
        // The application must not mutate process environment during construction.
        if std::env::vars_os().any(|(key, _)| key.as_encoded_bytes().starts_with(b"PG")) {
            return Err(SettingsError::new(
                "environment",
                "native PG settings are unsupported",
            ));
        }
        Ok(PgConnectOptions::new_without_pgpass()
            .host(self.host.expose_secret())
            .port(self.port)
            .username(self.username.expose_secret())
            .password(self.password.expose_secret())
            .database(self.database.expose_secret())
            .ssl_mode(self.ssl)
            .application_name(self.application.expose_secret()))
    }
}
