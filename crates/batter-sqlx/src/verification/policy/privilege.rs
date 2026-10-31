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
}
