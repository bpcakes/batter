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

use sqlx::{Postgres, postgres::PgArguments, query::Query};

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
        statements!(@munch [$($done)* {
            [$(#[$attribute])*] $name (::core::$reader!($path)) (Some($path))
        }] $($rest)*);
    };
    (@munch [$($done:tt)*]
        $(#[$attribute:meta])* $name:ident = $sql:literal; $($rest:tt)*
    ) => {
        statements!(@munch [$($done)* { [$(#[$attribute])*] $name ($sql) (None) }] $($rest)*);
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
    (@emit $({ [$($attribute:tt)*] $name:ident ($($text:tt)+) ($($file:tt)+) })+) => {
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
                    $(Self::$name => $($text)+,)+
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
                    $(Self::$name => $($file)+,)+
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

impl Statement {
    /// The statement as a query ready for binding.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place SQL text reaches sqlx"
    )]
    pub(crate) fn query<'q>(self) -> Query<'q, Postgres, PgArguments> {
        sqlx::query(self.sql())
    }
}
