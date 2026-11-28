//! Every SQL statement the limiters send at run time, and the only way to
//! send one.
//!
//! [`Statement`] is closed: each family module declares its statements in one
//! `statements!` block, which generates the family's `Id` enum and fixes each
//! statement's text to a string literal, and `families!` below generates
//! [`Statement`] over the families. A value can therefore name only a declared
//! statement, and `all()` lists every one for `crate::catalog_guard`.
//!
//! [`Statement::query`] is the one place this crate's Clippy configuration
//! (`clippy.toml`) lets SQL text reach sqlx: every sqlx entry point that
//! accepts SQL text is disallowed elsewhere. Each statement calls catalog
//! functions and casts to non-keyword types by their `pg_catalog` names and
//! keeps to the dialect `crate::catalog_guard` reads. Published migrations
//! are immutable and run only through their migrators.

use sqlx::{
    Encode, Error, Executor, Postgres, Type,
    postgres::{PgArguments, PgQueryResult, PgRow},
};

/// Declares one family's statements: an `Id` variant and a constant per
/// statement. A statement's text is a string literal or the `include_str!`
/// of a `.sql` file named by a literal path; nothing else, such as a
/// published migration's constant or another macro, is accepted. The include
/// is written out in the family's file, where `scripts/ci_plan.py` resolves
/// it, and this macro only forwards it.
macro_rules! statements {
    (@munch [$($done:tt)*]) => {
        statements!(@emit $($done)*);
    };
    (@munch [$($done:tt)*]
        $(#[$attribute:meta])* $name:ident = $reader:ident!($path:literal); $($rest:tt)*
    ) => {
        statements!(@only_include_str $reader);
        statements!(@munch [$($done)* { [$(#[$attribute])*] $name $reader $path }] $($rest)*);
    };
    (@munch [$($done:tt)*]
        $(#[$attribute:meta])* $name:ident = $sql:literal; $($rest:tt)*
    ) => {
        statements!(@munch [$($done)* { [$(#[$attribute])*] $name literal $sql }] $($rest)*);
    };
    (@munch [$($done:tt)*] $($rest:tt)+) => {
        compile_error!(
            "a statement's text is a string literal or include_str! of a literal .sql path"
        );
    };
    (@only_include_str include_str) => {};
    (@only_include_str $other:ident) => {
        compile_error!("a statement may include its text only with include_str!");
    };
    // Text and file come from the one literal each entry carries, so even a
    // direct `@emit` cannot name a constant: an entry it cannot read falls
    // through to the catch-all, whose `@munch` rejects it. An included
    // entry's kind is the family file's own reader, checked again here.
    (@text literal $sql:literal) => { $sql };
    (@text $reader:ident $path:literal) => {{
        statements!(@only_include_str $reader);
        ::core::$reader!($path)
    }};
    (@file literal $sql:literal) => { None };
    (@file $reader:ident $path:literal) => { Some($path) };
    (@emit $({ [$($attribute:tt)*] $name:ident $kind:ident $value:literal })+) => {
        /// The statements this family declares.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[allow(
            non_camel_case_types,
            clippy::upper_case_acronyms,
            reason = "each variant is named like the constant call sites use"
        )]
        pub(crate) enum Id {
            $($name,)+
        }

        impl Id {
            /// Every statement of this family, in declaration order.
            #[cfg(test)]
            pub(crate) const ALL: &[Self] = &[$(Self::$name),+];

            /// The statement's text.
            pub(crate) const fn sql(self) -> &'static str {
                match self {
                    $(Self::$name => statements!(@text $kind $value),)+
                }
            }

            /// The statement's declared name.
            #[cfg(test)]
            pub(crate) const fn name(self) -> &'static str {
                match self {
                    $(Self::$name => stringify!($name),)+
                }
            }

            /// The `.sql` file the statement includes, relative to its
            /// family's module file, if it is not a literal.
            #[cfg(test)]
            pub(crate) const fn file(self) -> Option<&'static str> {
                match self {
                    $(Self::$name => statements!(@file $kind $value),)+
                }
            }
        }

        $(
            $($attribute)*
            pub(crate) const $name: $crate::statements::Statement = Id::$name.statement();
        )+
    };
    ($($input:tt)+) => {
        statements!(@munch [] $($input)+);
    };
}

/// Generates [`Statement`] over every family, so each family is a variant of
/// it and part of `all()`.
macro_rules! families {
    ($($family:ident => $module:ident),+ $(,)?) => {
        $(pub(crate) mod $module;)+

        /// One declared operational statement.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub(crate) enum Statement {
            $($family($module::Id),)+
        }

        $(
            impl $module::Id {
                const fn statement(self) -> Statement {
                    Statement::$family(self)
                }
            }
        )+

        impl Statement {
            /// The statement's text.
            pub(crate) const fn sql(self) -> &'static str {
                match self {
                    $(Self::$family(id) => id.sql(),)+
                }
            }

            /// The `.sql` file the statement includes, relative to its
            /// family's module file, if it is not a literal.
            #[cfg(test)]
            pub(crate) const fn file(self) -> Option<&'static str> {
                match self {
                    $(Self::$family(id) => id.file(),)+
                }
            }

            /// The statement's path in this module.
            #[cfg(test)]
            fn name(self) -> String {
                match self {
                    $(Self::$family(id) => format!(concat!(stringify!($module), "::{}"), id.name()),)+
                }
            }
        }

        /// Every statement, by its path in this module.
        #[cfg(test)]
        pub(crate) fn all() -> Vec<(String, Statement)> {
            let mut all = Vec::new();
            $(all.extend($module::Id::ALL.iter().map(|id| id.statement()));)+
            all.into_iter().map(|statement| (statement.name(), statement)).collect()
        }
    };
}

families! {
    Attempts => attempts,
    FixedWindow => fixed_window,
    Gcra => gcra,
    Session => session,
}

/// A declared statement with its arguments bound: every query this crate
/// runs, and constructed only by [`Statement::query`]. sqlx's query types
/// are disallowed everywhere else, so a helper that took a query built from
/// other SQL would have to name one.
#[must_use = "a bound statement runs only when executed"]
pub(crate) struct Bound<'q>(
    #[expect(
        clippy::disallowed_types,
        reason = "the one place a sqlx query value is held"
    )]
    sqlx::query::Query<'q, Postgres, PgArguments>,
);

impl<'q> Bound<'q> {
    /// Binds the next parameter.
    pub(crate) fn bind<'t, T: Encode<'t, Postgres> + Type<Postgres>>(self, value: T) -> Self {
        Self(self.0.bind(value))
    }

    /// Runs the statement, returning what it affected.
    pub(crate) async fn execute<'e, 'c: 'e, E>(self, executor: E) -> Result<PgQueryResult, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        self.0.execute(executor).await
    }

    /// Runs the statement, returning every row.
    pub(crate) async fn fetch_all<'e, 'c: 'e, E>(self, executor: E) -> Result<Vec<PgRow>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        self.0.fetch_all(executor).await
    }

    /// Runs the statement, returning its one row.
    pub(crate) async fn fetch_one<'e, 'c: 'e, E>(self, executor: E) -> Result<PgRow, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        self.0.fetch_one(executor).await
    }

    /// Runs the statement, returning its row if it has one.
    pub(crate) async fn fetch_optional<'e, 'c: 'e, E>(
        self,
        executor: E,
    ) -> Result<Option<PgRow>, Error>
    where
        'q: 'e,
        E: Executor<'c, Database = Postgres>,
    {
        self.0.fetch_optional(executor).await
    }
}

impl Statement {
    /// The statement ready for binding.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place SQL text reaches sqlx"
    )]
    pub(crate) fn query<'q>(self) -> Bound<'q> {
        Bound(sqlx::query(self.sql()))
    }
}
