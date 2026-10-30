use super::{AllowedPrivilege, ObjectPrivilege};

impl AllowedPrivilege {
    /// A privilege without authority to grant it onward.
    ///
    /// Migration: replaces `AllowedPrivilege::new(privilege, false)`.
    ///
    /// ```
    /// use batter_sqlx::verification::{AllowedPrivilege, ObjectPrivilege};
    /// let usage = AllowedPrivilege::privilege(ObjectPrivilege::Usage);
    /// assert!(!usage.grant_option);
    /// ```
    pub const fn privilege(privilege: ObjectPrivilege) -> Self {
        Self {
            privilege,
            grant_option: false,
        }
    }

    /// A privilege with authority to grant it onward.
    ///
    /// Migration: replaces `AllowedPrivilege::new(privilege, true)`.
    ///
    /// ```
    /// use batter_sqlx::verification::{AllowedPrivilege, ObjectPrivilege};
    /// let usage = AllowedPrivilege::grantable(ObjectPrivilege::Usage);
    /// assert!(usage.grant_option);
    /// ```
    pub const fn grantable(privilege: ObjectPrivilege) -> Self {
        Self {
            privilege,
            grant_option: true,
        }
    }

    /// Compatibility wrapper for explicit privilege constructors.
    #[deprecated(
        since = "0.0.2",
        note = "use AllowedPrivilege::privilege or AllowedPrivilege::grantable"
    )]
    #[allow(
        clippy::fn_params_excessive_bools,
        reason = "one-release compatibility for an existing public constructor"
    )]
    pub const fn new(privilege: ObjectPrivilege, grant_option: bool) -> Self {
        if grant_option {
            Self::grantable(privilege)
        } else {
            Self::privilege(privilege)
        }
    }
}
