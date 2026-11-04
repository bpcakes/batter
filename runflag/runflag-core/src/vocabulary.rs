//! The process-kind and environment-class vocabularies an application declares.
//!
//! Values are minted only by [`CatalogBuilder`](crate::CatalogBuilder) and carry
//! their catalog's process-local identity, so an undeclared kind or class has
//! no representation and a value from another catalog is recognised wherever
//! it is presented.

use std::{
    fmt,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// The process-local identity of one built catalog.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CatalogId(NonZeroU64);

static NEXT_CATALOG: AtomicU64 = AtomicU64::new(1);

impl CatalogId {
    /// Mints an identity no other catalog in this process has.
    pub(crate) fn mint() -> Self {
        let raw = NEXT_CATALOG.fetch_add(1, Ordering::Relaxed);
        // The counter starts at one and a process cannot build 2^64 catalogs.
        Self(NonZeroU64::new(raw).unwrap_or(NonZeroU64::MIN))
    }
}

macro_rules! vocabulary_type {
    ($name:ident, $what:literal, $description:literal) => {
        #[doc = $description]
        ///
        /// Minted only by the catalog builder, with a validated unique name of
        /// the form `[a-z][a-z0-9_]{0,63}`, and carrying the catalog it belongs
        /// to. Two values are equal when they are the same declaration of the
        /// same catalog. There is no public constructor, so a value that no
        /// catalog declared cannot be written:
        ///
        /// ```compile_fail,E0599
        #[doc = concat!("let kind = runflag_core::", stringify!($name), "::new(\"serving\");")]
        /// ```
        #[derive(Clone)]
        pub struct $name {
            catalog: CatalogId,
            index: usize,
            name: Arc<str>,
        }

        impl $name {
            pub(crate) fn mint(catalog: CatalogId, index: usize, name: Arc<str>) -> Self {
                Self {
                    catalog,
                    index,
                    name,
                }
            }

            #[doc = concat!("The declared ", $what, " name.")]
            pub fn name(&self) -> &str {
                &self.name
            }

            pub(crate) fn catalog(&self) -> CatalogId {
                self.catalog
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.catalog == other.catalog && self.index == other.index
            }
        }

        impl Eq for $name {}

        impl std::hash::Hash for $name {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.catalog.hash(state);
                self.index.hash(state);
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}({:?})", stringify!($name), &*self.name)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.name)
            }
        }
    };
}

vocabulary_type!(
    ProcessKind,
    "process kind",
    "A kind of process the application runs, such as its HTTP serving or worker kind."
);
vocabulary_type!(
    EnvironmentClass,
    "environment class",
    "A class of environment the application deploys to, such as development or production."
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_identities_are_distinct() {
        assert_ne!(CatalogId::mint(), CatalogId::mint());
    }

    #[test]
    fn vocabulary_values_compare_by_declaration() {
        let catalog = CatalogId::mint();
        let web = ProcessKind::mint(catalog, 0, Arc::from("web"));
        let same = ProcessKind::mint(catalog, 0, Arc::from("web"));
        let other_catalog = ProcessKind::mint(CatalogId::mint(), 0, Arc::from("web"));
        assert_eq!(web, same);
        assert_ne!(web, other_catalog);
        assert_eq!(web.name(), "web");
        assert_eq!(format!("{web:?}"), "ProcessKind(\"web\")");
        assert_eq!(web.to_string(), "web");
    }
}
