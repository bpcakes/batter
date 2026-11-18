use runledger_postgres::{PgSessionProfile, RunledgerDatabase};
use std::time::Duration;

/// Example policy: direct authentication unless an explicit serving role is set.
pub async fn connect(url: &str) -> Result<RunledgerDatabase, Box<dyn std::error::Error>> {
    // SQLx 0.9.1 would connect to an ambient or declared `hostaddr` (or a Unix
    // socket in its place) instead of the URL host; refuse both.
    let options = runledger_postgres::connect_options(url)?;
    let login = options.get_username().to_owned();
    let role = std::env::var("RUNLEDGER_DB_ROLE").unwrap_or_else(|_| login.clone());
    let schema = std::env::var("RUNLEDGER_DB_SCHEMA").unwrap_or_else(|_| "public".into());
    let profile = PgSessionProfile::new(
        login,
        role,
        vec![schema],
        Duration::from_secs(30),
        Duration::from_secs(5),
    )?;
    Ok(runledger_postgres::low_level::connect_database(options, profile, 5).await?)
}
