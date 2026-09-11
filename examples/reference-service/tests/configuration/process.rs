pub(crate) use crate::process_runtime::{ExpectedExit, launch, run};
use crate::temp_dir;

#[test]
fn captured_process_environment_and_passfile_policy_fail_closed() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    };
    let endpoint = OsString::from("postgres://user@localhost/database?sslmode=disable");
    for name in [
        "PGHOST",
        "PGHOSTADDR",
        "PGPORT",
        "PGUSER",
        "PGPASSWORD",
        "PGDATABASE",
        "PGSSLMODE",
        "PGSSLROOTCERT",
        "PGSSLCERT",
        "PGSSLKEY",
        "PGAPPNAME",
        "PGOPTIONS",
        "PGPASSFILE",
        "PG_UNKNOWN",
    ] {
        for value in [
            OsString::from("secret-marker"),
            OsString::new(),
            OsString::from_vec(b"secret-marker\xff".to_vec()),
        ] {
            run(
                "reject-pg",
                &[("DATABASE_URL", endpoint.clone()), (name, value)],
            )
            .validate_text(
                ExpectedExit::Code(42),
                &["configuration-child:rejected source: unknown key"],
                &[
                    "secret-marker",
                    "panicked",
                    "configuration-child:passfile-ignored",
                ],
            )
            .unwrap();
        }
    }
    run("injected-pg", &[("PGPASSWORD", "secret-marker".into())])
        .validate_text(
            ExpectedExit::Code(42),
            &["configuration-child:rejected environment: native PG settings are unsupported"],
            &["secret-marker"],
        )
        .unwrap();
    for value in [
        "postgres://user:secret-marker@host/db?sslmode=disable&unknown=secret-marker",
        "postgres://user:secret-marker@host/db?sslmode=disable&password=secret-marker",
        "secret-marker",
    ] {
        run("reject-url", &[("DATABASE_URL", value.into())])
            .validate_text(
                ExpectedExit::Code(42),
                &["configuration-child:rejected DATABASE_URL:"],
                &["secret-marker"],
            )
            .unwrap();
    }
    run(
        "reject-nonunicode",
        &[(
            "DATABASE_URL",
            OsString::from_vec(b"secret-marker\xff".to_vec()),
        )],
    )
    .validate_text(
        ExpectedExit::Code(42),
        &["DATABASE_URL: non-Unicode value"],
        &["secret-marker"],
    )
    .unwrap();
    let directory = temp_dir::TempDir::new().unwrap();
    let home = directory.path();
    let passfile = home.join(".pgpass");
    std::fs::write(&passfile, "localhost:5432:database:user:secret-marker\n").unwrap();
    std::fs::set_permissions(&passfile, std::fs::Permissions::from_mode(0o600)).unwrap();
    let result = run(
        "valid-passfile-ignored",
        &[
            ("DATABASE_URL", endpoint),
            ("HOME", home.as_os_str().to_owned()),
        ],
    );
    directory.close().unwrap();
    result
        .validate_text(
            ExpectedExit::Success,
            &["configuration-child:passfile-ignored"],
            &["secret-marker"],
        )
        .unwrap();
}

// Native constructors deliberately consult process state. Always cross this
// subprocess boundary, even when the settings themselves came from injected pairs.
pub(crate) fn native(scenario: &str) {
    run(scenario, &[])
        .validate_text(
            ExpectedExit::Success,
            &["configuration-child:native-complete"],
            &["panicked"],
        )
        .unwrap();
}
