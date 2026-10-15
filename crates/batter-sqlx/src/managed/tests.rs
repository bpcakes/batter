use super::source::{Token, scan};
use super::*;
use crate::verification::RoutineType;

fn signature(schema: &str, name: &str, arguments: &[&str]) -> RoutineSignature {
    RoutineSignature::new(
        schema,
        name,
        arguments
            .iter()
            .map(|argument| RoutineType::new("pg_catalog", *argument).unwrap()),
    )
    .unwrap()
}

fn total() -> RoutineSignature {
    signature("app", "total", &["int8"])
}

fn table() -> QualifiedName {
    QualifiedName::new("app", "records").unwrap()
}

fn trigger_name() -> Identifier {
    Identifier::new("records_touch").unwrap()
}

fn ledger() -> QualifiedName {
    QualifiedName::new("app", "managed_objects").unwrap()
}

const TOTAL: &str = "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint
    LANGUAGE sql IMMUTABLE RETURN amount * 2;";
const TOUCH: &str = "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.records
    FOR EACH ROW EXECUTE FUNCTION app.touch_updated_at();";

fn function(source: &'static str) -> Result<ManagedObject, ManagedObjectError> {
    ManagedObject::function(total(), source)
}

fn trigger(source: &'static str) -> Result<ManagedObject, ManagedObjectError> {
    ManagedObject::trigger(table(), trigger_name(), source)
}

#[test]
fn accepted_routine_sources_cover_quoting_comments_and_bodies() {
    for source in [
        TOTAL,
        "create or replace function app.total(bigint) returns bigint language sql return 1",
        "-- leading comment; with a semicolon\n/* nested /* block */ comment */\n\
         CREATE OR REPLACE FUNCTION \"app\".\"total\"(bigint) RETURNS bigint LANGUAGE sql RETURN 1;\n-- trailing\n",
        "CREATE OR REPLACE FUNCTION App.Total(bigint) RETURNS bigint LANGUAGE plpgsql AS $body$
         BEGIN RETURN $1; /* not a comment terminator ; */ END; $body$;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE plpgsql AS $$
         DECLARE note text := $inner$ ; $inner$; BEGIN RETURN 1; END $$;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql AS 'SELECT ''a;b''::text::bigint';",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql AS E'SELECT \\'x;\\'::text; ' ;",
        "CREATE OR REPLACE FUNCTION app.total(amount bigint DEFAULT (1)) RETURNS bigint LANGUAGE sql
         BEGIN ATOMIC SELECT amount; SELECT CASE WHEN amount > 0 THEN amount ELSE 0 END; END;",
    ] {
        assert!(function(source).is_ok(), "{source}");
    }
    assert!(
        ManagedObject::procedure(
            signature("app", "settle", &[]),
            "CREATE OR REPLACE PROCEDURE app.settle() LANGUAGE sql BEGIN ATOMIC SELECT 1; END",
        )
        .is_ok()
    );
}

#[test]
fn accepted_trigger_sources_cover_every_timing_and_event_form() {
    for source in [
        TOUCH,
        "create or replace trigger records_touch after insert or update of note, \"Amount\" or delete
         on app.records referencing new table as added for each statement execute function app.audit()",
        "CREATE OR REPLACE TRIGGER \"records_touch\" INSTEAD OF TRUNCATE ON \"app\".\"records\"
         EXECUTE FUNCTION app.audit()",
    ] {
        assert!(trigger(source).is_ok(), "{source}");
    }
}

#[test]
fn headers_must_be_create_or_replace_of_the_declared_kind() {
    for source in [
        "",
        "-- only a comment",
        "CREATE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE PROCEDURE app.total(bigint) LANGUAGE sql BEGIN ATOMIC SELECT 1; END;",
        "\"CREATE\" OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "SELECT 1;",
    ] {
        assert_eq!(
            function(source),
            Err(ManagedObjectError::SourceHeader),
            "{source}"
        );
    }
    for source in [
        "CREATE OR REPLACE CONSTRAINT TRIGGER records_touch AFTER UPDATE ON app.records
         FOR EACH ROW EXECUTE FUNCTION app.audit();",
        "CREATE TRIGGER records_touch BEFORE UPDATE ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE MERGE ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE OF ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch INSTEAD UPDATE ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE app.records EXECUTE FUNCTION app.audit();",
    ] {
        assert_eq!(
            trigger(source),
            Err(ManagedObjectError::SourceHeader),
            "{source}"
        );
    }
}

#[test]
fn headers_must_name_the_declared_qualified_identity() {
    for source in [
        "CREATE OR REPLACE FUNCTION total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION other.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION app.totals(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION \"App\".total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION db.app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION app.total AS 'x';",
        "CREATE OR REPLACE FUNCTION app.;",
    ] {
        assert_eq!(
            function(source),
            Err(ManagedObjectError::SourceIdentity),
            "{source}"
        );
    }
    for source in [
        "CREATE OR REPLACE TRIGGER other_touch BEFORE UPDATE ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.accounts EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON db.app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER \"Records_Touch\" BEFORE UPDATE ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER app.records_touch BEFORE UPDATE ON app.records EXECUTE FUNCTION app.audit();",
        "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.records.extra EXECUTE FUNCTION app.audit();",
    ] {
        assert_eq!(
            trigger(source),
            Err(ManagedObjectError::SourceIdentity),
            "{source}"
        );
    }
}

#[test]
fn sources_must_be_exactly_one_terminated_statement() {
    for source in [
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1; DROP TABLE app.records;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE plpgsql AS $$ BEGIN RETURN 1; END",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql AS 'SELECT 1",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql AS E'SELECT 1\\';",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint /* unterminated",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS \"bigint LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS \"\" LANGUAGE sql RETURN 1;",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;\0",
        "CREATE OR REPLACE FUNCTION app.total(bigint) RETURNS bigint LANGUAGE sql
         BEGIN ATOMIC SELECT 1; END; SELECT 2;",
    ] {
        assert_eq!(
            function(source),
            Err(ManagedObjectError::SourceStatement),
            "{source}"
        );
    }
    assert_eq!(
        trigger(
            "CREATE OR REPLACE TRIGGER records_touch BEFORE UPDATE ON app.records
             FOR EACH ROW EXECUTE FUNCTION app.touch_updated_at(); BEGIN; END;"
        ),
        Err(ManagedObjectError::SourceStatement)
    );
}

#[test]
fn oversized_sources_are_rejected_before_scanning() {
    let source = format!("{TOTAL}\n-- {}", "x".repeat(MAX_SOURCE_BYTES));
    assert_eq!(
        ManagedObject::function(total(), source),
        Err(ManagedObjectError::SourceTooLarge)
    );
    let fitting = format!(
        "{TOTAL}\n-- {}",
        "x".repeat(MAX_SOURCE_BYTES - TOTAL.len() - 4)
    );
    assert!(ManagedObject::function(total(), fitting).is_ok());
}

#[test]
fn scanner_distinguishes_parameters_identifiers_and_dollar_quotes() {
    assert_eq!(
        scan("SELECT $1, a$b$, $tag$;$tag$ , e'\\'' ;").unwrap(),
        [
            Token::Word("SELECT"),
            Token::Literal,
            Token::Punct(b','),
            Token::Word("a$b$"),
            Token::Punct(b','),
            Token::Literal,
            Token::Punct(b','),
            Token::Literal,
            Token::Punct(b';'),
        ]
    );
    assert_eq!(
        scan("\"a\"\"b\".c").unwrap(),
        [
            Token::Quoted("a\"b".to_owned()),
            Token::Punct(b'.'),
            Token::Word("c"),
        ]
    );
    // A different tag inside a dollar quote is content, not a terminator.
    assert_eq!(scan("$a$ $b$ ; $a$").unwrap(), [Token::Literal]);
    assert_eq!(scan("$a$ $b$ ;"), Err(ManagedObjectError::SourceStatement));
}

#[test]
fn temporary_namespaces_are_rejected_everywhere() {
    assert_eq!(
        ManagedObject::function(
            signature("pg_temp", "total", &["int8"]),
            "CREATE OR REPLACE FUNCTION pg_temp.total(bigint) RETURNS bigint LANGUAGE sql RETURN 1;",
        ),
        Err(ManagedObjectError::TemporaryNamespace)
    );
    let temporary_argument = RoutineSignature::new(
        "app",
        "total",
        [RoutineType::new("pg_temp_3", "amount").unwrap()],
    )
    .unwrap();
    assert_eq!(
        ManagedObject::function(temporary_argument.clone(), TOTAL),
        Err(ManagedObjectError::TemporaryNamespace)
    );
    assert_eq!(
        ManagedObject::trigger(
            QualifiedName::new("pg_toast_temp_1", "records").unwrap(),
            trigger_name(),
            TOUCH,
        ),
        Err(ManagedObjectError::TemporaryNamespace)
    );
    assert_eq!(
        ManagedObjectManifest::new(
            QualifiedName::new("pg_temp", "managed_objects").unwrap(),
            PublicExecute::Revoke,
            [function(TOTAL).unwrap()],
            [],
        ),
        Err(ManagedObjectError::TemporaryNamespace)
    );
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [],
            [Retirement::function(temporary_argument)],
        ),
        Err(ManagedObjectError::TemporaryNamespace)
    );
}

#[test]
fn manifests_reject_empty_reserved_duplicate_and_overlapping_sets() {
    assert_eq!(
        ManagedObjectManifest::new(ledger(), PublicExecute::Revoke, [], []),
        Err(ManagedObjectError::EmptyManifest)
    );
    assert_eq!(
        ManagedObjectManifest::new(
            QualifiedName::new("app", "_sqlx_migrations").unwrap(),
            PublicExecute::Revoke,
            [function(TOTAL).unwrap()],
            [],
        ),
        Err(ManagedObjectError::ReservedLedger)
    );
    let identity = total().as_str().to_owned();
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [function(TOTAL).unwrap(), function(TOTAL).unwrap()],
            [],
        ),
        Err(ManagedObjectError::DuplicateObject {
            identity: identity.clone()
        })
    );
    let procedure = ManagedObject::procedure(
        total(),
        "CREATE OR REPLACE PROCEDURE app.total(bigint) LANGUAGE sql BEGIN ATOMIC SELECT 1; END;",
    )
    .unwrap();
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [function(TOTAL).unwrap(), procedure],
            [],
        ),
        Err(ManagedObjectError::DuplicateObject {
            identity: identity.clone()
        })
    );
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [],
            [Retirement::function(total()), Retirement::function(total())],
        ),
        Err(ManagedObjectError::DuplicateRetirement {
            identity: identity.clone()
        })
    );
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [function(TOTAL).unwrap()],
            [Retirement::procedure(total())],
        ),
        Err(ManagedObjectError::RetiredObjectDeclared { identity })
    );
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [trigger(TOUCH).unwrap()],
            [Retirement::trigger(table(), trigger_name())],
        ),
        Err(ManagedObjectError::RetiredObjectDeclared {
            identity: "\"records_touch\" ON \"app\".\"records\"".to_owned()
        })
    );
}

#[test]
fn manifests_stop_consuming_at_the_capacity_boundary() {
    let mut produced = 0usize;
    let retirements = std::iter::from_fn(|| {
        produced += 1;
        assert!(produced <= MAX_MANAGED_OBJECTS + 1);
        Some(Retirement::function(signature(
            "app",
            &format!("retired_{produced}"),
            &[],
        )))
    });
    assert_eq!(
        ManagedObjectManifest::new(ledger(), PublicExecute::Revoke, [], retirements),
        Err(ManagedObjectError::Capacity)
    );
    assert_eq!(produced, MAX_MANAGED_OBJECTS + 1);

    let mut produced = 0usize;
    let retirements = std::iter::from_fn(|| {
        produced += 1;
        assert!(produced <= MAX_MANAGED_OBJECTS);
        Some(Retirement::function(signature(
            "app",
            &format!("retired_{produced}"),
            &[],
        )))
    });
    assert_eq!(
        ManagedObjectManifest::new(
            ledger(),
            PublicExecute::Revoke,
            [function(TOTAL).unwrap()],
            retirements,
        ),
        Err(ManagedObjectError::Capacity)
    );
}

#[test]
fn manifests_stop_consuming_at_the_total_source_boundary() {
    let allowed = MAX_MANIFEST_SOURCE_BYTES / MAX_SOURCE_BYTES;
    let mut produced = 0usize;
    let objects = std::iter::from_fn(|| {
        produced += 1;
        assert!(
            produced <= allowed + 1,
            "consumed past the first excess source"
        );
        let name = format!("total_{produced}");
        let mut source = format!(
            "CREATE OR REPLACE FUNCTION app.{name}() RETURNS bigint LANGUAGE sql RETURN 1;\n-- "
        );
        source.push_str(&"x".repeat(MAX_SOURCE_BYTES - source.len()));
        Some(ManagedObject::function(signature("app", &name, &[]), source).unwrap())
    });
    let retirements = std::iter::from_fn(|| -> Option<Retirement> {
        panic!("retirements were consumed after objects exceeded their byte bound")
    });
    assert_eq!(
        ManagedObjectManifest::new(ledger(), PublicExecute::Revoke, objects, retirements),
        Err(ManagedObjectError::Capacity)
    );
    assert_eq!(produced, allowed + 1);
}

fn manifest(
    policy: PublicExecute,
    objects: Vec<ManagedObject>,
    retirements: Vec<Retirement>,
) -> ManagedObjectManifest {
    ManagedObjectManifest::new(ledger(), policy, objects, retirements).unwrap()
}

#[test]
fn digest_is_order_independent_and_covers_policy_sources_and_retirements() {
    let touch = trigger(TOUCH).unwrap();
    let base = manifest(
        PublicExecute::Revoke,
        vec![function(TOTAL).unwrap(), touch.clone()],
        vec![Retirement::function(signature("app", "old", &[]))],
    );
    let reordered = manifest(
        PublicExecute::Revoke,
        vec![touch.clone(), function(TOTAL).unwrap()],
        vec![Retirement::function(signature("app", "old", &[]))],
    );
    assert_eq!(base.digest(), reordered.digest());
    assert_eq!(base.objects(), reordered.objects());
    assert_eq!(base.objects()[0].identity(), total().as_str());
    assert_eq!(base.objects()[1].kind(), ManagedObjectKind::Trigger);

    let relocated = ManagedObjectManifest::new(
        QualifiedName::new("audit", "managed_objects").unwrap(),
        PublicExecute::Revoke,
        base.objects().to_vec(),
        base.retirements().to_vec(),
    )
    .unwrap();
    assert_eq!(base.digest(), relocated.digest());
    assert_ne!(base.lock_key(), relocated.lock_key());

    for different in [
        manifest(
            PublicExecute::Unmanaged,
            base.objects().to_vec(),
            base.retirements().to_vec(),
        ),
        manifest(PublicExecute::Revoke, base.objects().to_vec(), Vec::new()),
        manifest(
            PublicExecute::Revoke,
            vec![
                touch.clone(),
                function(
                    "CREATE OR REPLACE FUNCTION app.total(amount bigint) RETURNS bigint
                     LANGUAGE sql IMMUTABLE RETURN amount * 3;",
                )
                .unwrap(),
            ],
            base.retirements().to_vec(),
        ),
        manifest(
            PublicExecute::Revoke,
            vec![touch],
            vec![
                Retirement::function(signature("app", "old", &[])),
                Retirement::function(total()),
            ],
        ),
    ] {
        assert_ne!(base.digest(), different.digest());
    }
    assert_eq!(base.digest().to_string().len(), 64);
    assert!(
        base.digest()
            .to_string()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
}

#[test]
fn identities_checksums_and_diagnostics_are_canonical() {
    let object = trigger(TOUCH).unwrap();
    assert_eq!(
        object.identity(),
        "\"records_touch\" ON \"app\".\"records\""
    );
    assert_eq!(object.source(), TOUCH);
    assert_eq!(
        object.checksum(),
        &<[u8; 32]>::from(Sha256::digest(TOUCH.as_bytes()))
    );
    let debug = format!("{object:?}");
    assert!(debug.contains("records_touch") && !debug.contains("EXECUTE FUNCTION"));
    let retirement = Retirement::procedure(signature("app", "settle", &["int4"]));
    assert_eq!(retirement.kind(), ManagedObjectKind::Procedure);
    assert_eq!(
        retirement.identity(),
        "\"app\".\"settle\"(\"pg_catalog\".\"int4\")"
    );
    assert_eq!(ManagedObjectKind::Procedure.as_str(), "procedure");
    let error = ManagedObjectError::DuplicateObject {
        identity: "\"app\".\"total\"()".to_owned(),
    };
    assert!(error.to_string().contains("\"app\".\"total\"()"));
}
