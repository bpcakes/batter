//! One-release compatibility for boolean policy setters.

use super::{
    AuthorityAllowance, ColumnGrantGroup, DatabaseGrantSpec, ExactRoleManifest,
    FragmentObjectPolicy, RelationGrantGroup, RoutineGrantSpec, SchemaGrantSpec,
};

impl DatabaseGrantSpec {
    /// Compatibility wrapper; use [`Self::grant_option`].
    #[deprecated(since = "0.0.2", note = "use grant_option with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_grant_option(self, allow: bool) -> Self {
        self.grant_option(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl DatabaseGrantSpec {
    /// Compatibility wrapper; use [`Self::ownership`].
    #[deprecated(since = "0.0.2", note = "use ownership with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_owner(self, allow: bool) -> Self {
        self.ownership(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl SchemaGrantSpec {
    /// Compatibility wrapper; use [`Self::grant_option`].
    #[deprecated(since = "0.0.2", note = "use grant_option with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_grant_option(self, allow: bool) -> Self {
        self.grant_option(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl SchemaGrantSpec {
    /// Compatibility wrapper; use [`Self::ownership`].
    #[deprecated(since = "0.0.2", note = "use ownership with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_owner(self, allow: bool) -> Self {
        self.ownership(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl RelationGrantGroup {
    /// Compatibility wrapper; use [`Self::grant_option`].
    #[deprecated(since = "0.0.2", note = "use grant_option with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_grant_option(self, allow: bool) -> Self {
        self.grant_option(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl RelationGrantGroup {
    /// Compatibility wrapper; use [`Self::ownership`].
    #[deprecated(since = "0.0.2", note = "use ownership with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_owner(self, allow: bool) -> Self {
        self.ownership(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl RelationGrantGroup {
    /// Compatibility wrapper; use [`Self::row_type_public_usage`].
    #[deprecated(
        since = "0.0.2",
        note = "use row_type_public_usage with AuthorityAllowance"
    )]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_row_type_public_usage(self, allow: bool) -> Self {
        self.row_type_public_usage(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl ColumnGrantGroup {
    /// Compatibility wrapper; use [`Self::grant_option`].
    #[deprecated(since = "0.0.2", note = "use grant_option with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_grant_option(self, allow: bool) -> Self {
        self.grant_option(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl RoutineGrantSpec {
    /// Compatibility wrapper; use [`Self::grant_option`].
    #[deprecated(since = "0.0.2", note = "use grant_option with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_grant_option(self, allow: bool) -> Self {
        self.grant_option(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl RoutineGrantSpec {
    /// Compatibility wrapper; use [`Self::ownership`].
    #[deprecated(since = "0.0.2", note = "use ownership with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_owner(self, allow: bool) -> Self {
        self.ownership(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl RoutineGrantSpec {
    /// Compatibility wrapper; use [`Self::security_definer`].
    #[deprecated(since = "0.0.2", note = "use security_definer with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_security_definer(self, allow: bool) -> Self {
        self.security_definer(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl ExactRoleManifest {
    /// Compatibility wrapper; use [`Self::database_ownership`].
    #[deprecated(
        since = "0.0.2",
        note = "use database_ownership with AuthorityAllowance"
    )]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn deny_current_database_ownership(&mut self, deny: bool) {
        self.database_ownership(if deny {
            AuthorityAllowance::Deny
        } else {
            AuthorityAllowance::Allow
        })
    }
}

impl FragmentObjectPolicy {
    /// Compatibility wrapper; use [`Self::schema_ownership`].
    #[must_use]
    #[deprecated(since = "0.0.2", note = "use schema_ownership with AuthorityAllowance")]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_schema_owner(self, allow: bool) -> Self {
        self.schema_ownership(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl FragmentObjectPolicy {
    /// Compatibility wrapper; use [`Self::relation_ownership`].
    #[must_use]
    #[deprecated(
        since = "0.0.2",
        note = "use relation_ownership with AuthorityAllowance"
    )]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_relation_owner(self, allow: bool) -> Self {
        self.relation_ownership(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}

impl FragmentObjectPolicy {
    /// Compatibility wrapper; use [`Self::row_type_public_usage`].
    #[must_use]
    #[deprecated(
        since = "0.0.2",
        note = "use row_type_public_usage with AuthorityAllowance"
    )]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public policy setter"
    )]
    pub const fn allow_row_type_public_usage(self, allow: bool) -> Self {
        self.row_type_public_usage(if allow {
            AuthorityAllowance::Allow
        } else {
            AuthorityAllowance::Deny
        })
    }
}
