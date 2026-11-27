//! Every SQL statement the limiters send at run time, and the only way to
//! send one.
//!
//! A [`Statement`]'s text is private to this module and its submodules, so
//! the rest of the crate can execute only the statements defined here, through
//! [`Statement::query`]. Each one calls catalog functions and casts to
//! non-keyword types by their `pg_catalog` names, and keeps to the dialect
//! `crate::catalog_guard` reads; that guard checks every entry of [`ALL`] and
//! that no other module builds SQL. Published migrations are immutable and
//! live with their migrators instead.

use sqlx::{Postgres, postgres::PgArguments, query::Query};

/// One operational statement.
#[derive(Clone, Copy)]
pub(crate) struct Statement(&'static str);

/// Declares a family's statements and its [`ALL`]-style registry together, so
/// every statement is registered under its own name and nothing else in this
/// module constructs a [`Statement`].
macro_rules! statements {
    ($($(#[$attribute:meta])* $name:ident = $sql:expr;)+) => {
        $($(#[$attribute])* pub(crate) const $name: Statement = Statement($sql);)+

        /// Every statement of this family, by name.
        #[cfg(test)]
        pub(crate) const ALL: &[(&str, Statement)] = &[$((stringify!($name), $name)),+];
    };
}

pub(crate) mod attempts;
pub(crate) mod fixed_window;
pub(crate) mod gcra;

impl Statement {
    /// The statement as a query ready for binding.
    pub(crate) fn query<'q>(self) -> Query<'q, Postgres, PgArguments> {
        sqlx::query(self.0)
    }

    /// The statement's text, for the guard and for tests.
    #[cfg(test)]
    pub(crate) const fn sql(self) -> &'static str {
        self.0
    }
}

statements! {
    SET_LOCAL_TIMEOUTS_SQL = r"
SELECT
    pg_catalog.set_config('statement_timeout', $1, true),
    pg_catalog.set_config('lock_timeout', $2, true)
";
}

/// Every statement, by its path in this module: the root's and each family's,
/// whose names come from their own declarations.
#[cfg(test)]
pub(crate) fn all() -> Vec<(String, Statement)> {
    let families: [(&str, &[(&str, Statement)]); 4] = [
        ("", ALL),
        ("attempts::", attempts::ALL),
        ("fixed_window::", fixed_window::ALL),
        ("gcra::", gcra::ALL),
    ];
    families
        .into_iter()
        .flat_map(|(family, statements)| {
            statements
                .iter()
                .map(move |(name, statement)| (format!("{family}{name}"), *statement))
        })
        .collect()
}
