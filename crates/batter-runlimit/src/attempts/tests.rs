use super::*;

#[test]
fn output_debug_never_formats_application_payloads() {
    struct Secret;
    let accepted: Authentication<_, Secret> = Authentication::Accepted(Secret);
    let rejected: Authentication<Secret, _> = Authentication::Rejected(Secret);
    assert_eq!(format!("{accepted:?}"), "Authentication::Accepted");
    assert_eq!(format!("{rejected:?}"), "Authentication::Rejected");
}
