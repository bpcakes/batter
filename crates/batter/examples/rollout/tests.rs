//! The example's declared mode, run whole.
#![allow(clippy::unwrap_used, reason = "a panic fails the test")]

use batter::settings::SettingsSource;

use super::{resolve, run_declared, today};

#[test]
fn the_catalog_resolves_and_today_is_a_civil_date() {
    let settings = SettingsSource::from_pairs([("FLAG_NEW_CHECKOUT".into(), "on".into())]).unwrap();
    let (resolved, _) = resolve(&settings, today().unwrap()).unwrap();
    assert!(resolved.report().to_string().contains("new_checkout=on"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_engaged_switch_stops_maintenance_and_a_released_one_lets_it_run() {
    let engaged =
        SettingsSource::from_pairs([("FLAG_OUTBOUND_HALT".into(), "engaged".into())]).unwrap();
    assert_eq!(run_declared(&engaged).await.unwrap(), 0);
    let released = SettingsSource::from_pairs([]).unwrap();
    assert!(run_declared(&released).await.unwrap() >= 2);
}
