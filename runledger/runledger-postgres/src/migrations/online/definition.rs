//! Exact canonical renderings, independent of the session's quoting setting.

use super::OnlineIndex;

/// Compare immutable catalog text with declared PostgreSQL 18 renderings.
/// Quoting belongs to the expected definition, never to a normalization pass
/// over live SQL. No session setting or caller cleanup protocol is required.
pub(super) fn definition_matches(index: &OnlineIndex, definition: &str, schema: &str) -> bool {
    for (quote, expected, type_name) in [
        ("", index.expected_definition, "job_status"),
        ("\"", index.expected_quoted_definition, "\"job_status\""),
    ] {
        // PostgreSQL decides whether ordinary schemas need quoting. Keep both
        // exact spellings, including embedded quotes/USING, without a keyword
        // table or parsing the catalog's header at a delimiter.
        for schema_prefix in [
            format!("\"{}\".", schema.replace('"', "\"\"")),
            format!("{schema}."),
        ] {
            let header = format!(
                "CREATE INDEX {quote}{}{quote} ON {schema_prefix}{quote}{}{quote} USING ",
                index.name, index.table
            );
            if let Some(body) = definition.strip_prefix(&header) {
                let cast = format!("::{type_name}");
                if body == expected
                    || body == expected.replace(&cast, &format!("::{schema_prefix}{type_name}"))
                {
                    return true;
                }
            }
        }
    }
    false
}
