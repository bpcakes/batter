//! Branded handles: the only way to name a declaration at evaluation.

use std::{fmt, hash::Hash};

use crate::{
    declaration::SafeState, digest::DeclarationFingerprint, key::FlagKey, value::FlagValue,
};

mod sealed {
    use crate::{
        declaration::Kind, digest::DeclarationFingerprint, key::FlagKey, value::ValueType,
        vocabulary::CatalogId,
    };

    /// What every handle carries: its catalog, its entry, its fingerprint and
    /// the kind and value-type tags of its declaration, so that a view from
    /// another catalog can still describe the handle it refuses.
    ///
    /// Public only inside this private module, so no other crate can name it.
    #[derive(Clone)]
    pub struct Handle {
        catalog: CatalogId,
        index: usize,
        key: FlagKey,
        fingerprint: DeclarationFingerprint,
        kind: Kind,
        value_type: ValueType,
    }

    impl Handle {
        pub(crate) fn new(
            catalog: CatalogId,
            index: usize,
            key: FlagKey,
            fingerprint: DeclarationFingerprint,
            (kind, value_type): (Kind, ValueType),
        ) -> Self {
            Self {
                catalog,
                index,
                key,
                fingerprint,
                kind,
                value_type,
            }
        }

        pub(crate) fn kind(&self) -> Kind {
            self.kind
        }

        pub(crate) fn value_type(&self) -> ValueType {
            self.value_type
        }

        pub(crate) fn catalog(&self) -> CatalogId {
            self.catalog
        }

        pub(crate) fn index(&self) -> usize {
            self.index
        }

        pub(crate) fn key(&self) -> &FlagKey {
            &self.key
        }

        pub(crate) fn fingerprint(&self) -> DeclarationFingerprint {
            self.fingerprint
        }
    }

    impl PartialEq for Handle {
        fn eq(&self, other: &Self) -> bool {
            self.catalog == other.catalog && self.index == other.index
        }
    }

    impl Eq for Handle {}

    impl std::hash::Hash for Handle {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            self.catalog.hash(state);
            self.index.hash(state);
        }
    }

    /// Seals [`Flag`](super::Flag): only this crate's handles implement it.
    pub trait Sealed {
        fn handle(&self) -> &Handle;
    }
}

pub(crate) use sealed::{Handle, Sealed};

/// A release or mode flag handle: the declarations that carry a
/// [`FlagValue`] and can be evaluated and pinned.
///
/// Implemented only by [`ReleaseFlag`] and [`ModeFlag`]; a kill switch is not
/// a `Flag`, because switches act through gates and are never pinned.
///
/// Every evaluation names a handle the builder minted; a key string is not a
/// handle:
///
/// ```compile_fail,E0277
/// use batter_flags_core::{ReleaseFlag, Toggle};
///
/// let _flag: ReleaseFlag<Toggle> = "new_checkout".into();
/// ```
pub trait Flag: Sealed + Clone + fmt::Debug + Send + Sync + 'static {
    /// The declared value type.
    type Value: FlagValue;

    /// The declaration's key.
    fn key(&self) -> &FlagKey;

    /// The fingerprint of the declaration this handle was minted for.
    fn fingerprint(&self) -> DeclarationFingerprint;

    /// The declaration's code default, which a decision serves wherever no
    /// layer supplies a value.
    fn code_default(&self) -> Self::Value;
}

macro_rules! flag_handle {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        ///
        /// Minted only by the catalog builder when the declaration is made.
        /// The handle carries its catalog's process-local identity and its
        /// declaration's fingerprint, so a view from another catalog recognises
        /// it as foreign. Two handles are equal when they name the same
        /// declaration of the same catalog.
        pub struct $name<T: FlagValue> {
            handle: Handle,
            default: T,
        }

        impl<T: FlagValue> $name<T> {
            pub(crate) fn mint(handle: Handle, default: T) -> Self {
                Self { handle, default }
            }
        }

        impl<T: FlagValue> Sealed for $name<T> {
            fn handle(&self) -> &Handle {
                &self.handle
            }
        }

        impl<T: FlagValue> Flag for $name<T> {
            type Value = T;

            fn key(&self) -> &FlagKey {
                self.handle.key()
            }

            fn fingerprint(&self) -> DeclarationFingerprint {
                self.handle.fingerprint()
            }

            fn code_default(&self) -> T {
                self.default
            }
        }

        impl<T: FlagValue> Clone for $name<T> {
            fn clone(&self) -> Self {
                Self {
                    handle: self.handle.clone(),
                    default: self.default,
                }
            }
        }

        impl<T: FlagValue> PartialEq for $name<T> {
            fn eq(&self, other: &Self) -> bool {
                self.handle == other.handle
            }
        }

        impl<T: FlagValue> Eq for $name<T> {}

        impl<T: FlagValue> Hash for $name<T> {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.handle.hash(state);
            }
        }

        impl<T: FlagValue> fmt::Debug for $name<T> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    formatter,
                    "{}({:?})",
                    stringify!($name),
                    self.handle.key().as_str()
                )
            }
        }
    };
}

flag_handle!(
    ReleaseFlag,
    "A temporary release flag with a code default, a mandatory expiry and an optional governing kill switch."
);
flag_handle!(
    ModeFlag,
    "A permanent mode flag, such as a permission gate or an operating mode, with a code default and no expiry."
);

/// A permanent kill switch handle, never per-subject and never pinned.
///
/// Minted only by the catalog builder. A switch is read through gates and
/// `view.switch`, never evaluated to a [`FlagValue`].
#[derive(Clone, Eq, Hash, PartialEq)]
pub struct KillSwitch {
    handle: Handle,
    safe_state: SafeState,
}

impl KillSwitch {
    pub(crate) fn mint(handle: Handle, safe_state: SafeState) -> Self {
        Self { handle, safe_state }
    }

    pub(crate) fn handle(&self) -> &Handle {
        &self.handle
    }

    /// The declaration's key.
    pub fn key(&self) -> &FlagKey {
        self.handle.key()
    }

    /// The fingerprint of the declaration this handle was minted for.
    pub fn fingerprint(&self) -> DeclarationFingerprint {
        self.handle.fingerprint()
    }

    /// The state the declaration serves where no entry exists for it.
    pub fn safe_state(&self) -> SafeState {
        self.safe_state
    }
}

impl fmt::Debug for KillSwitch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "KillSwitch({:?})", self.handle.key().as_str())
    }
}
