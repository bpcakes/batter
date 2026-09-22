use super::*;

#[test]
fn row_context_has_one_canonical_encoding() {
    let context = Context::for_row(
        "example-service",
        "private-record",
        b"owner-7",
        b"document-9",
        "record-bytes-v1",
    )
    .unwrap();

    let mut expected = vec![CONTEXT_CODEC_VERSION, ROW_BINDING];
    for component in [
        b"example-service".as_slice(),
        b"private-record",
        b"owner-7",
        b"document-9",
        b"record-bytes-v1",
    ] {
        expected.extend_from_slice(&(component.len() as u16).to_be_bytes());
        expected.extend_from_slice(component);
    }

    assert_eq!(context.encoded(), expected);
    assert_eq!(context.namespace(), b"example-service");
    assert_eq!(context.purpose(), b"private-record");
}

#[test]
fn tenant_token_context_has_a_distinct_canonical_shape() {
    let context = Context::for_tenant_token(
        "example-service",
        "external-token",
        b"account-4",
        "token-v1",
    )
    .unwrap();

    let mut expected = vec![CONTEXT_CODEC_VERSION, TENANT_TOKEN_BINDING];
    for component in [
        b"example-service".as_slice(),
        b"external-token",
        b"account-4",
        b"token-v1",
    ] {
        expected.extend_from_slice(&(component.len() as u16).to_be_bytes());
        expected.extend_from_slice(component);
    }

    assert_eq!(context.encoded(), expected);
    assert_eq!(context.namespace(), b"example-service");
    assert_eq!(context.purpose(), b"external-token");
}

#[test]
fn context_factories_reject_missing_oversized_and_noncanonical_components() {
    let oversized_label = "x".repeat(MAX_CONTEXT_LABEL_BYTES + 1);
    let oversized_binding = vec![0_u8; MAX_BINDING_COMPONENT_BYTES + 1];

    for result in [
        Context::for_row("", "purpose", b"owner", b"resource", "schema"),
        Context::for_row("NameSpace", "purpose", b"owner", b"resource", "schema"),
        Context::for_row("namespace", "has space", b"owner", b"resource", "schema"),
        Context::for_row(&oversized_label, "purpose", b"owner", b"resource", "schema"),
        Context::for_row("namespace", "purpose", b"", b"resource", "schema"),
        Context::for_row("namespace", "purpose", b"owner", b"", "schema"),
        Context::for_row(
            "namespace",
            "purpose",
            &oversized_binding,
            b"resource",
            "schema",
        ),
        Context::for_row("namespace", "purpose", b"owner", b"resource", "SCHEMA"),
    ] {
        assert_eq!(result.unwrap_err(), Error::InvalidContext);
    }

    assert_eq!(
        Context::for_tenant_token("namespace", "purpose", b"", "schema").unwrap_err(),
        Error::InvalidContext
    );

    for result in [
        Context::for_tenant_token("NameSpace", "purpose", b"tenant", "schema"),
        Context::for_tenant_token("namespace", "has space", b"tenant", "schema"),
        Context::for_tenant_token("namespace", "purpose", b"tenant", "SCHEMA"),
        Context::for_tenant_token(&oversized_label, "purpose", b"tenant", "schema"),
        Context::for_tenant_token("namespace", "purpose", &oversized_binding, "schema"),
    ] {
        assert_eq!(result.unwrap_err(), Error::InvalidContext);
    }
}

#[test]
fn debug_redacts_every_context_component() {
    let context = Context::for_row(
        "secret-namespace",
        "secret-purpose",
        b"secret-owner",
        b"secret-resource",
        "secret-schema",
    )
    .unwrap();
    let rendered = format!("{context:?}");

    assert_eq!(rendered, "Context(<redacted>)");
    for value in ["namespace", "purpose", "owner", "resource", "schema"] {
        assert!(!rendered.contains(value));
    }
}
