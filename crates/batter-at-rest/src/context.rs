use std::{fmt, ops::Range};

use zeroize::Zeroizing;

use crate::{Error, MAX_BINDING_COMPONENT_BYTES, MAX_CONTEXT_BYTES, MAX_CONTEXT_LABEL_BYTES};

const CONTEXT_CODEC_VERSION: u8 = 1;
const ROW_BINDING: u8 = 1;
const TENANT_TOKEN_BINDING: u8 = 2;

/// Canonical application ownership and payload-schema context authenticated by an envelope.
///
/// Contexts can only be created through a concrete binding factory. The raw
/// encoding is intentionally not public, so callers cannot replace a required
/// owner/resource/tenant tuple with an arbitrary byte string.
pub struct Context {
    encoded: Zeroizing<Vec<u8>>,
    namespace: Range<usize>,
    purpose: Range<usize>,
}

impl Context {
    /// Creates a context for a persisted row or logical aggregate.
    ///
    /// `owner` and `resource` must be the application's canonical, stable bytes
    /// for the real owner and resource. `payload_schema` is independent of the
    /// envelope format version.
    pub fn for_row(
        namespace: &str,
        purpose: &str,
        owner: &[u8],
        resource: &[u8],
        payload_schema: &str,
    ) -> Result<Self, Error> {
        validate_label(namespace)?;
        validate_label(purpose)?;
        validate_binding(owner)?;
        validate_binding(resource)?;
        validate_label(payload_schema)?;

        let mut encoded = Zeroizing::new(Vec::with_capacity(MAX_CONTEXT_BYTES));
        encoded.extend_from_slice(&[CONTEXT_CODEC_VERSION, ROW_BINDING]);
        let namespace = push_component(&mut encoded, namespace.as_bytes());
        let purpose = push_component(&mut encoded, purpose.as_bytes());
        push_component(&mut encoded, owner);
        push_component(&mut encoded, resource);
        push_component(&mut encoded, payload_schema.as_bytes());
        validate_total(&encoded)?;

        Ok(Self {
            encoded,
            namespace,
            purpose,
        })
    }

    /// Creates a context for a tenant-scoped bearer token or similar capability.
    ///
    /// This binding intentionally has no row/resource component: the
    /// authoritative tenant and payload schema are the complete application
    /// identity for this class of value.
    pub fn for_tenant_token(
        namespace: &str,
        purpose: &str,
        tenant: &[u8],
        payload_schema: &str,
    ) -> Result<Self, Error> {
        validate_label(namespace)?;
        validate_label(purpose)?;
        validate_binding(tenant)?;
        validate_label(payload_schema)?;

        let mut encoded = Zeroizing::new(Vec::with_capacity(MAX_CONTEXT_BYTES));
        encoded.extend_from_slice(&[CONTEXT_CODEC_VERSION, TENANT_TOKEN_BINDING]);
        let namespace = push_component(&mut encoded, namespace.as_bytes());
        let purpose = push_component(&mut encoded, purpose.as_bytes());
        push_component(&mut encoded, tenant);
        push_component(&mut encoded, payload_schema.as_bytes());
        validate_total(&encoded)?;

        Ok(Self {
            encoded,
            namespace,
            purpose,
        })
    }

    pub(crate) fn encoded(&self) -> &[u8] {
        &self.encoded
    }

    pub(crate) fn namespace(&self) -> &[u8] {
        &self.encoded[self.namespace.clone()]
    }

    pub(crate) fn purpose(&self) -> &[u8] {
        &self.encoded[self.purpose.clone()]
    }
}

impl fmt::Debug for Context {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Context(<redacted>)")
    }
}

fn validate_label(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > MAX_CONTEXT_LABEL_BYTES
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._:-".contains(&byte)
        })
    {
        return Err(Error::InvalidContext);
    }
    Ok(())
}

fn validate_binding(value: &[u8]) -> Result<(), Error> {
    if value.is_empty() || value.len() > MAX_BINDING_COMPONENT_BYTES {
        return Err(Error::InvalidContext);
    }
    Ok(())
}

fn push_component(target: &mut Vec<u8>, value: &[u8]) -> Range<usize> {
    let length = u16::try_from(value.len()).expect("validated context component fits u16");
    target.extend_from_slice(&length.to_be_bytes());
    let start = target.len();
    target.extend_from_slice(value);
    start..target.len()
}

fn validate_total(encoded: &[u8]) -> Result<(), Error> {
    if encoded.len() > MAX_CONTEXT_BYTES {
        return Err(Error::InvalidContext);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
