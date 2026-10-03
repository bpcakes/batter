//! Additive, object-only grant fragments published by libraries.
//!
//! A library that owns PostgreSQL relations knows which objects, columns and
//! privileges its own statements require. It does not know, and must not
//! decide, the application's database policy. [`GrantFragment`] carries only
//! object declarations so a library can publish the first without touching the
//! second.

use super::{
    ColumnGrantGroup, DeclarationPurpose, ExactRoleManifest, Identifier, MAX_MANIFEST_INPUTS,
    ManifestError, ObjectPrivilege, PolicyError, PublicDelivery, QualifiedName, RelationGrantGroup,
    RoutineGrantSpec, SchemaGrantSpec, checked_add, collect_bounded,
};

/// One library's additive object/privilege requirements for an exact role.
///
/// A fragment carries schema, relation, column and routine declarations and
/// nothing else. It cannot carry or replace the current-database declaration,
/// the discovery scope or its defaults, role-attribute ceilings, or the
/// current-database ownership guard: those stay application-owned on
/// [`ExactRoleManifest`]. A fragment also performs no compilation of its own; it
/// is added to a manifest and reaches the one existing compiler and renderer.
///
/// Every builder consumes the fragment, so a rejected declaration yields no
/// partially extended value. [`ExactRoleManifest::with_fragment`] follows the
/// same shape, so a failed append cannot leave an executable partial grant set.
///
/// ```
/// use batter_sqlx::verification::{
///     DeclarationPurpose, GrantFragment, ObjectPrivilege, QualifiedName, RelationGrantGroup,
/// };
///
/// let ledger = QualifiedName::new("service", "ledger")?;
/// let fragment = GrantFragment::new().with_relations(RelationGrantGroup::new(
///     [ledger],
///     [ObjectPrivilege::Select],
///     DeclarationPurpose::RequiredAndProvisioned,
/// )?)?;
/// assert!(!fragment.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GrantFragment {
    schemas: Vec<SchemaGrantSpec>,
    relations: Vec<RelationGrantGroup>,
    columns: Vec<ColumnGrantGroup>,
    routines: Vec<RoutineGrantSpec>,
}

impl GrantFragment {
    /// Begin an empty fragment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare one schema the fragment's objects live in.
    ///
    /// The declaration is the caller's: a library asks for the schema USAGE its
    /// statements need, while PUBLIC delivery and schema ownership remain
    /// explicit application choices carried by the supplied specification.
    ///
    /// # Errors
    /// Returns [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`]
    /// when the fragment would exceed its declaration bound.
    pub fn with_schema(mut self, spec: SchemaGrantSpec) -> Result<Self, ManifestError> {
        self.ensure_capacity(spec.input_size())?;
        self.schemas.push(spec);
        Ok(self)
    }

    /// Declare one grouped relation requirement.
    ///
    /// # Errors
    /// Returns [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`]
    /// when the fragment would exceed its declaration bound.
    pub fn with_relations(mut self, group: RelationGrantGroup) -> Result<Self, ManifestError> {
        self.ensure_capacity(group.input_size())?;
        self.relations.push(group);
        Ok(self)
    }

    /// Declare one grouped column requirement on an already declared relation.
    ///
    /// A fragment is self-contained: declare the parent relation first so its
    /// ownership, row-type and PUBLIC behavior are never implicit, and so an
    /// application never has to attach a parent declaration for native columns.
    ///
    /// # Errors
    /// Returns [`ManifestError::MissingRelationDeclaration`] when this fragment
    /// does not already declare the parent relation, or
    /// [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`] when
    /// the fragment would exceed its declaration bound.
    pub fn with_columns(mut self, group: ColumnGrantGroup) -> Result<Self, ManifestError> {
        if !self.declares_relation(&group.relation) {
            return Err(ManifestError::MissingRelationDeclaration);
        }
        self.ensure_capacity(group.input_size())?;
        self.columns.push(group);
        Ok(self)
    }

    /// Declare one exact routine overload the fragment's statements invoke.
    ///
    /// # Errors
    /// Returns [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`]
    /// when the fragment would exceed its declaration bound.
    pub fn with_routine(mut self, spec: RoutineGrantSpec) -> Result<Self, ManifestError> {
        self.ensure_capacity(spec.input_size())?;
        self.routines.push(spec);
        Ok(self)
    }

    /// Return whether the fragment declares nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.schemas.is_empty()
            && self.relations.is_empty()
            && self.columns.is_empty()
            && self.routines.is_empty()
    }

    fn declares_relation(&self, relation: &QualifiedName) -> bool {
        self.relations
            .iter()
            .any(|group| group.relations.contains(relation))
    }

    fn ensure_capacity(&self, additional: usize) -> Result<(), ManifestError> {
        self.input_size()?
            .checked_add(additional)
            .filter(|size| *size <= MAX_MANIFEST_INPUTS)
            .ok_or(ManifestError::Policy(PolicyError::AuthorityCapacity))
            .map(|_| ())
    }

    pub(super) fn input_size(&self) -> Result<usize, ManifestError> {
        self.schemas
            .iter()
            .map(SchemaGrantSpec::input_size)
            .chain(self.relations.iter().map(RelationGrantGroup::input_size))
            .chain(self.columns.iter().map(ColumnGrantGroup::input_size))
            .chain(self.routines.iter().map(RoutineGrantSpec::input_size))
            .try_fold(0, checked_add)
    }
}

impl ExactRoleManifest {
    /// Add one library's additive object declarations to this manifest.
    ///
    /// The manifest is consumed, so a rejected fragment yields no manifest to
    /// compile and no partially extended grant set. Repeated and interleaved
    /// fragments are normalized by the existing compiler: declarations that
    /// agree merge, and declarations that disagree about one exact semantic
    /// option are rejected before any policy or grant plan exists.
    ///
    /// Adding a fragment never changes the discovery scope, discovery defaults,
    /// role ceilings, the current-database declaration, or the current-database
    /// ownership guard.
    ///
    /// # Errors
    /// Returns [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`]
    /// when the combined declarations exceed the manifest input bound.
    pub fn with_fragment(mut self, fragment: GrantFragment) -> Result<Self, ManifestError> {
        self.ensure_input_capacity(fragment.input_size()?)?;
        self.schemas.extend(fragment.schemas);
        self.relations.extend(fragment.relations);
        self.columns.extend(fragment.columns);
        self.routines.extend(fragment.routines);
        Ok(self)
    }
}

/// The application's explicit policy for one library fragment's own objects.
///
/// A library knows which relations, columns and privileges its statements need.
/// It cannot know whether the application's provisioned roles reach those
/// privileges through PUBLIC, whether PostgreSQL's default PUBLIC USAGE on
/// composite row types is retained, or who owns the schema and its relations.
/// This value carries those choices so a published fragment stays compatible
/// with the surrounding manifest instead of overriding it.
///
/// Defaults are the narrow ones: schema USAGE only, no ownership, no PUBLIC
/// delivery and no row-type PUBLIC USAGE. Grant options are never declared and
/// every declaration is required and provisioned, so a fragment cannot smuggle
/// in an allowance the application did not ask for.
///
/// ```
/// use batter_sqlx::verification::{FragmentObjectPolicy, Identifier, ObjectPrivilege};
///
/// let policy = FragmentObjectPolicy::new(Identifier::new("Jobs")?)
///     .allow_row_type_public_usage(true)
///     .with_public_delivered_privileges([ObjectPrivilege::Select])?;
/// assert_eq!(policy.schema().as_str(), "Jobs");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FragmentObjectPolicy {
    schema: Identifier,
    schema_privileges: Vec<ObjectPrivilege>,
    schema_owner: bool,
    relation_owner: bool,
    public_delivered: Vec<ObjectPrivilege>,
    row_type_public_usage: bool,
}

impl FragmentObjectPolicy {
    /// Begin a policy for objects in one exact schema.
    ///
    /// The schema declaration always provisions `USAGE`, because no fragment
    /// statement can reach a relation without it.
    #[must_use]
    pub fn new(schema: Identifier) -> Self {
        Self {
            schema,
            schema_privileges: vec![ObjectPrivilege::Usage],
            schema_owner: false,
            relation_owner: false,
            public_delivered: Vec::new(),
            row_type_public_usage: false,
        }
    }

    /// Declare further schema privileges the application provisions here.
    ///
    /// # Errors
    /// Returns [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`]
    /// when the declaration bound would be exceeded.
    pub fn with_schema_privileges(
        mut self,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
    ) -> Result<Self, ManifestError> {
        self.schema_privileges.extend(collect_bounded(privileges)?);
        self.schema_privileges = collect_bounded(self.schema_privileges)?;
        Ok(self)
    }

    /// Permit a role reachable from the login to own this schema.
    #[must_use]
    pub const fn allow_schema_owner(mut self, allow: bool) -> Self {
        self.schema_owner = allow;
        self
    }

    /// Permit a role reachable from the login to own the fragment's relations.
    #[must_use]
    pub const fn allow_relation_owner(mut self, allow: bool) -> Self {
        self.relation_owner = allow;
        self
    }

    /// Permit PUBLIC to deliver exactly these privileges on the fragment's
    /// relations and columns.
    ///
    /// # Errors
    /// Returns [`ManifestError::Policy`] with [`PolicyError::AuthorityCapacity`]
    /// when the declaration bound would be exceeded.
    pub fn with_public_delivered_privileges(
        mut self,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
    ) -> Result<Self, ManifestError> {
        self.public_delivered.extend(collect_bounded(privileges)?);
        self.public_delivered = collect_bounded(self.public_delivered)?;
        Ok(self)
    }

    /// Retain PostgreSQL's default PUBLIC `USAGE` on the relations' row types.
    #[must_use]
    pub const fn allow_row_type_public_usage(mut self, allow: bool) -> Self {
        self.row_type_public_usage = allow;
        self
    }

    /// Return the exact schema these objects live in.
    #[must_use]
    pub const fn schema(&self) -> &Identifier {
        &self.schema
    }

    /// Add this policy's schema declaration to a fragment.
    ///
    /// Schema privileges are split by their individual PUBLIC-delivery choice, so
    /// permitting PUBLIC to deliver `USAGE` never also permits it to deliver a
    /// further schema privilege the application declared.
    ///
    /// # Errors
    /// Returns [`ManifestError`] when the declaration is invalid or the
    /// fragment's declaration bound would be exceeded.
    pub fn declare_schema(&self, fragment: GrantFragment) -> Result<GrantFragment, ManifestError> {
        let (denied, delivered) = self.split(self.schema_privileges.iter().copied())?;
        let mut fragment =
            fragment.with_schema(self.schema_group(denied, PublicDelivery::Deny)?)?;
        if !delivered.is_empty() {
            fragment = fragment
                .with_schema(self.schema_group(delivered, PublicDelivery::AllowDeclared)?)?;
        }
        Ok(fragment)
    }

    /// Declare relation-level privileges for named relations in this schema.
    ///
    /// An empty privilege list still declares the relations, which a fragment
    /// needs before it can declare their columns. Relation names are validated
    /// here and always qualified by this policy's schema, so a fragment cannot
    /// reach an object outside it.
    ///
    /// # Errors
    /// Returns [`ManifestError`] when a name or privilege is invalid for a
    /// relation, or the fragment's declaration bound would be exceeded.
    pub fn declare_relations<'a>(
        &self,
        fragment: GrantFragment,
        relations: impl IntoIterator<Item = &'a str>,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
    ) -> Result<GrantFragment, ManifestError> {
        let names = self.qualified(relations)?;
        let (denied, delivered) = self.split(privileges)?;
        let mut fragment = fragment.with_relations(self.relation_group(
            &names,
            denied,
            PublicDelivery::Deny,
        )?)?;
        if !delivered.is_empty() {
            fragment = fragment.with_relations(self.relation_group(
                &names,
                delivered,
                PublicDelivery::AllowDeclared,
            )?)?;
        }
        Ok(fragment)
    }

    /// Declare column-level privileges on one already declared relation.
    ///
    /// # Errors
    /// Returns [`ManifestError::MissingRelationDeclaration`] when the fragment
    /// does not already declare the relation, or another [`ManifestError`] when
    /// a name or privilege is invalid or the declaration bound would be
    /// exceeded.
    pub fn declare_columns<'a>(
        &self,
        fragment: GrantFragment,
        relation: &str,
        columns: impl IntoIterator<Item = &'a str>,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
    ) -> Result<GrantFragment, ManifestError> {
        let relation = QualifiedName::new(self.schema.as_str(), relation)?;
        let columns = collect_bounded(columns)?;
        let (denied, delivered) = self.split(privileges)?;
        let mut fragment = fragment;
        for (privileges, delivery) in [
            (denied, PublicDelivery::Deny),
            (delivered, PublicDelivery::AllowDeclared),
        ] {
            if privileges.is_empty() {
                continue;
            }
            fragment = fragment.with_columns(
                ColumnGrantGroup::new(
                    relation.clone(),
                    columns
                        .iter()
                        .map(|column| Identifier::new(*column))
                        .collect::<Result<Vec<_>, _>>()?,
                    privileges,
                    DeclarationPurpose::RequiredAndProvisioned,
                )?
                .public_delivery(delivery),
            )?;
        }
        Ok(fragment)
    }

    fn schema_group(
        &self,
        privileges: Vec<ObjectPrivilege>,
        delivery: PublicDelivery,
    ) -> Result<SchemaGrantSpec, ManifestError> {
        Ok(SchemaGrantSpec::new(
            self.schema.clone(),
            privileges,
            DeclarationPurpose::RequiredAndProvisioned,
        )?
        .public_delivery(delivery)
        .allow_owner(self.schema_owner))
    }

    fn relation_group(
        &self,
        relations: &[QualifiedName],
        privileges: Vec<ObjectPrivilege>,
        delivery: PublicDelivery,
    ) -> Result<RelationGrantGroup, ManifestError> {
        Ok(RelationGrantGroup::new(
            relations.iter().cloned(),
            privileges,
            DeclarationPurpose::RequiredAndProvisioned,
        )?
        .public_delivery(delivery)
        .allow_owner(self.relation_owner)
        .allow_row_type_public_usage(self.row_type_public_usage))
    }

    fn qualified<'a>(
        &self,
        relations: impl IntoIterator<Item = &'a str>,
    ) -> Result<Vec<QualifiedName>, ManifestError> {
        collect_bounded(relations)?
            .into_iter()
            .map(|relation| {
                QualifiedName::new(self.schema.as_str(), relation).map_err(ManifestError::from)
            })
            .collect()
    }

    fn split(
        &self,
        privileges: impl IntoIterator<Item = ObjectPrivilege>,
    ) -> Result<(Vec<ObjectPrivilege>, Vec<ObjectPrivilege>), ManifestError> {
        Ok(collect_bounded(privileges)?
            .into_iter()
            .partition(|privilege| !self.public_delivered.contains(privilege)))
    }
}
