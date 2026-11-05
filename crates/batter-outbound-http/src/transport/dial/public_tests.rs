use super::*;
use crate::Origin;
use std::cell::RefCell;

#[tokio::test]
async fn mixed_dns_answers_only_dial_allowed_candidates_in_order() {
    let origin = Origin::public_https("https://provider.example").unwrap();
    let addresses = [
        "127.0.0.1:443",
        "[::1]:443",
        "1.1.1.1:443",
        "10.0.0.1:443",
        "8.8.8.8:443",
    ];
    let attempts = RefCell::new(Vec::new());
    let result = connect(
        &origin,
        addresses.map(|v| v.parse().unwrap()).to_vec(),
        Instant::now() + Duration::from_secs(1),
        |address| {
            attempts.borrow_mut().push(address);
            std::future::ready(if address.ip().to_string() == "8.8.8.8" {
                Ok(address)
            } else {
                Err(io::ErrorKind::ConnectionRefused.into())
            })
        },
    )
    .await
    .unwrap();
    assert_eq!(result, "8.8.8.8:443".parse().unwrap());
    assert_eq!(*attempts.borrow(), ["1.1.1.1:443".parse().unwrap(), result]);
}

#[tokio::test]
async fn all_private_answers_never_invoke_the_dialer() {
    let origin = Origin::public_https("https://provider.example").unwrap();
    let result = connect::<(), _, _>(
        &origin,
        vec![
            "127.0.0.1:443".parse().unwrap(),
            "[::1]:443".parse().unwrap(),
        ],
        Instant::now() + Duration::from_secs(1),
        |_| async {
            panic!("private destination reached the dialer");
        },
    )
    .await;
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AddrNotAvailable);
}
