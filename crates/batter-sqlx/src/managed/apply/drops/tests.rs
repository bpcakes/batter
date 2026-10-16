use super::{LockSet, coalesce, first_uncovered};
use crate::verification::QualifiedName;
use std::collections::{BTreeMap, HashMap};

/// `parted` and its partition `parted_east` held exclusively, `records` only
/// in `SHARE ROW EXCLUSIVE` mode.
fn locks() -> LockSet {
    LockSet(HashMap::from([
        ("s.parted".to_owned(), true),
        ("s.parted_east".to_owned(), true),
        ("s.records".to_owned(), false),
    ]))
}

/// The trees of `parted` (with its partitions), `records`, a table outside
/// the set, and a view, which has no members.
fn trees() -> Vec<Vec<String>> {
    vec![
        vec!["s.parted".to_owned(), "s.parted_east".to_owned()],
        vec!["s.records".to_owned()],
        vec!["s.unlocked".to_owned()],
        Vec::new(),
    ]
}

#[test]
fn coverage_follows_each_entry_mode_over_shared_tables() {
    let (locks, trees) = (locks(), trees());
    // Two entries share `records`; only a drop there needs a stronger lock.
    assert_eq!(
        first_uncovered(&locks, &trees, &[("create on records", 1, false)]),
        None
    );
    assert_eq!(
        first_uncovered(
            &locks,
            &trees,
            &[
                ("create on records", 1, false),
                ("drop on records", 1, true)
            ]
        ),
        Some("drop on records")
    );
    assert_eq!(
        first_uncovered(&locks, &trees, &[("drop on parted", 0, true)]),
        None
    );
}

#[test]
fn coverage_reports_the_first_uncovered_entry() {
    let (locks, trees) = (locks(), trees());
    assert_eq!(
        first_uncovered(
            &locks,
            &trees,
            &[
                ("covered", 0, true),
                ("outside the set", 2, false),
                ("stronger lock", 1, true),
            ]
        ),
        Some("outside the set")
    );
}

#[test]
fn coverage_needs_every_member_of_a_partition_tree() {
    let locks = locks();
    // `parted_west` joined the tree after the set was taken.
    let trees = vec![vec![
        "s.parted".to_owned(),
        "s.parted_east".to_owned(),
        "s.parted_west".to_owned(),
    ]];
    assert_eq!(
        first_uncovered(&locks, &trees, &[("create on parted", 0, false)]),
        Some("create on parted")
    );
}

#[test]
fn entries_without_lockable_members_are_covered() {
    let (locks, trees) = (locks(), trees());
    assert_eq!(
        first_uncovered(&locks, &trees, &[("trigger on a view", 3, true)]),
        None
    );
}

#[test]
fn lock_requests_coalesce_by_table_keeping_the_strongest_mode() {
    let parted = QualifiedName::new("s", "parted").unwrap();
    let records = QualifiedName::new("s", "records").unwrap();
    let elsewhere = QualifiedName::new("t", "records").unwrap();
    // Several triggers share `parted`; one of them needs a drop.
    let requests = coalesce([
        (&parted, false),
        (&records, false),
        (&parted, true),
        (&parted, false),
        (&elsewhere, true),
    ]);
    assert_eq!(
        requests,
        BTreeMap::from([
            (("s", "parted"), true),
            (("s", "records"), false),
            (("t", "records"), true),
        ])
    );
}
