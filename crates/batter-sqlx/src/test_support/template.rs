//! Fixture templates and the typed target their initializers prepare.

use std::{fmt, future::Future, sync::Arc};

use postgres_test_harness::{BoxError, DatabaseTemplate, TemplateFingerprint};
use sqlx::postgres::PgConnectOptions;

use super::endpoint::{Environment, HarnessEndpoint};

/// A template database retained by a fixture suite.
///
/// Returned by [`super::FixtureSuite::template`], [`super::FixtureSuite::derive`]
/// and their [`super::FixtureScope`] counterparts, and accepted by their
/// `from_template` and `derive` methods. It exposes only identity: databases
/// are cloned from it through a suite or scope, which checks the endpoint and
/// owns the lease, and children are derived through them too, so every
/// initializer receives a [`TemplateTarget`].
///
/// Migration: 0.0.1 returned the harness's `DatabaseTemplate`. Replace its
/// `derive(spec, |url| ..)` with `suite.derive(&template, spec, |target| ..)`
/// (or `scope.derive`); `database()` and `prewarm(..)` have no replacement on
/// the fixture path, since their leases would sit outside the suite's
/// capacity and cleanup. Use `suite.from_template` or `scope.from_template`.
///
/// ```no_run
/// use batter_sqlx::test_support::{ConnectionPlan, FixtureSuite, FixtureTemplate, template_spec};
/// use sqlx::postgres::PgPoolOptions;
/// # async fn example(mut suite: FixtureSuite) -> Result<(), Box<dyn std::error::Error>> {
/// let template: FixtureTemplate = suite
///     .template(template_spec(&[], "empty-v1"), |_target| async { Ok(()) })
///     .await?;
/// println!("cloning {}", template.database_name());
/// let plan = ConnectionPlan::new(vec![PgPoolOptions::new().max_connections(1)], 0)?;
/// let fixture = suite.from_template(&template, &plan).await?;
/// fixture.finish(Ok::<(), sqlx::Error>(())).await?;
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct FixtureTemplate {
    pub(super) inner: DatabaseTemplate,
}

impl FixtureTemplate {
    /// The template database's name, for catalog observations.
    pub fn database_name(&self) -> &str {
        self.inner.database_name()
    }

    /// The complete template identity, including every ancestor of a derived
    /// template.
    pub fn fingerprint(&self) -> TemplateFingerprint {
        self.inner.fingerprint()
    }
}

impl fmt::Debug for FixtureTemplate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FixtureTemplate")
            .field("database_name", &self.database_name())
            .finish()
    }
}

/// The database a template initializer prepares, on the server the harness
/// provisioned it on.
///
/// Built from the suite's one parse of the harness admin URL and the name the
/// harness recorded, exactly as fixture pools are, so the initializer never
/// handles URL text that SQLx could read differently from the harness. The
/// process environment is checked again just before the initializer runs.
///
/// Migration: 0.0.1 initializers took the harness URL as a `String`. Replace
/// `|url| async move { PgPoolOptions::new().connect(&url).await? .. }` with
/// `|target| async move { PgPoolOptions::new().connect_with(target.connect_options()).await? .. }`.
///
/// ```no_run
/// use batter_sqlx::test_support::{FixtureSuite, TemplateTarget};
/// use postgres_test_harness::TemplateSpec;
/// use sqlx::postgres::PgPoolOptions;
/// # async fn example(mut suite: FixtureSuite, spec: TemplateSpec) -> Result<(), Box<dyn std::error::Error>> {
/// let template = suite
///     .template(spec, |target: TemplateTarget| async move {
///         let pool = PgPoolOptions::new()
///             .max_connections(1)
///             .connect_with(target.connect_options())
///             .await?;
///         sqlx::query("CREATE TABLE items (id bigint PRIMARY KEY)")
///             .execute(&pool)
///             .await?;
///         pool.close().await;
///         Ok(())
///     })
///     .await?;
/// # drop(template);
/// # Ok(()) }
/// ```
pub struct TemplateTarget {
    endpoint: Arc<HarnessEndpoint>,
    database: String,
}

impl TemplateTarget {
    /// Bind `harness_url`, the URL the harness hands its initializer, to the
    /// suite's endpoint and the database name the harness recorded in it.
    fn new(
        endpoint: Arc<HarnessEndpoint>,
        harness_url: &str,
        environment: Environment,
    ) -> Result<Self, sqlx::Error> {
        environment.validate()?;
        let database = HarnessEndpoint::recorded_database(harness_url)
            .ok_or_else(|| sqlx::Error::Configuration(Box::new(super::LeaseDatabaseMismatch)))?;
        endpoint.bind(harness_url, &database)?;
        Ok(Self { endpoint, database })
    }

    /// The template database being initialized.
    pub fn database_name(&self) -> &str {
        &self.database
    }

    /// SQLx options for the template database, every endpoint field set
    /// explicitly as for fixture pools: host or Unix socket, port, the harness
    /// admin login, plaintext transport, `application_name` and startup
    /// `options`. A caller that changes the host or database afterwards
    /// leaves the template.
    pub fn connect_options(&self) -> PgConnectOptions {
        self.endpoint.options(&self.database)
    }
}

impl fmt::Debug for TemplateTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TemplateTarget")
            .field("database_name", &self.database)
            .finish_non_exhaustive()
    }
}

/// Run `initializer` on the target the harness URL selects. A refusal is the
/// initializer's failure, so the harness aborts the template.
pub(super) async fn initialize<F, Fut>(
    endpoint: Arc<HarnessEndpoint>,
    harness_url: String,
    initializer: F,
) -> Result<(), BoxError>
where
    F: FnOnce(TemplateTarget) -> Fut,
    Fut: Future<Output = Result<(), BoxError>>,
{
    let target = TemplateTarget::new(endpoint, &harness_url, Environment::process())?;
    initializer(target).await
}

#[cfg(test)]
mod tests;
