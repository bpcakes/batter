//! One endpoint policy shared by the live runner preflight and fixture acquisition.
use batter::settings::{SecretString, SettingsError, SettingsSource};
use batter_example_reference_service::config::{ConfigMode, RootSettings};
use sqlx::postgres::{PgConnectOptions, PgSslMode};

pub fn from_process() -> Result<(SecretString, PgConnectOptions), SettingsError> {
    from_named_process("POSTGRES_TEST_ADMIN_URL")
}

pub fn from_named_process(
    name: &'static str,
) -> Result<(SecretString, PgConnectOptions), SettingsError> {
    let url = std::env::var(name).map_err(|error| {
        SettingsError::new(name, "explicit Unicode endpoint required").with_cause(error)
    })?;
    from_url_in_process(&url)
        .map_err(|error| SettingsError::new(name, error.reason()).with_cause(error))
}

pub fn from_url_in_process(url: &str) -> Result<(SecretString, PgConnectOptions), SettingsError> {
    // Reuse the application validator; no second URL grammar or PG policy.
    let settings = RootSettings::from_sources(
        ConfigMode::Setup,
        None,
        SettingsSource::default(),
        SettingsSource::from_pairs([("DATABASE_URL".into(), url.into())])?,
    )?;
    let options = settings.connect_options_from_process()?;
    if options.get_host() == "::1" {
        return Err(SettingsError::new(
            "POSTGRES_TEST_ADMIN_URL",
            "IPv6 literals are unsupported by the live fixture URL consumer",
        ));
    }
    if !matches!(options.get_host(), "localhost" | "127.0.0.1")
        || !matches!(options.get_ssl_mode(), PgSslMode::Disable)
    {
        return Err(SettingsError::new(
            "POSTGRES_TEST_ADMIN_URL",
            "local disposable endpoint with sslmode=disable required",
        ));
    }
    // Serialize selected values for both native URL consumers. SQLx retains
    // encoded TCP hosts and trims leading database slashes before decoding.
    let mut handoff = url::Url::parse(url).map_err(|error| {
        SettingsError::new("POSTGRES_TEST_ADMIN_URL", "invalid URL syntax").with_cause(error)
    })?;
    handoff
        .set_host(Some(options.get_host()))
        .map_err(|error| {
            SettingsError::new("POSTGRES_TEST_ADMIN_URL", "invalid TCP host").with_cause(error)
        })?;
    let database = options.get_database().ok_or_else(|| {
        SettingsError::new("POSTGRES_TEST_ADMIN_URL", "explicit database required")
    })?;
    handoff.set_path(&format!(
        "/{}",
        percent_encoding::utf8_percent_encode(database, percent_encoding::NON_ALPHANUMERIC)
    ));
    let mut pairs: Vec<_> = handoff
        .query_pairs()
        .map(|(key, value)| {
            // SQLx accepts case-insensitive modes; tokio-postgres does not.
            let value = if key == "sslmode" { "disable" } else { &value };
            (key.into_owned(), value.to_owned())
        })
        .collect();
    // Root Setup selects an explicit empty password. Retain that choice so
    // SQLx's URL parser cannot fall back to an ambient passfile for fixture pools.
    if handoff.password().is_none() && !pairs.iter().any(|(key, _)| key == "password") {
        pairs.push(("password".into(), String::new()));
    }
    // The root decodes query '+' as a space; the harness's tokio-postgres
    // parser only percent-decodes. Encode those spaces before handing it the
    // validated URL. Userinfo '+' stays literal; query values are re-encoded.
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
        .replace('+', "%20");
    handoff.set_query(Some(&query));
    Ok((SecretString::new(String::from(handoff)), options))
}
