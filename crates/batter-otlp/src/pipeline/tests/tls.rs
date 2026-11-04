#[path = "../../../tests/support/client.rs"]
mod fixture;

#[tokio::test]
async fn client_uses_explicit_tls_without_trusting_a_fixture_certificate() {
    fixture::assert_bundled_tls(super::client_builder().build().unwrap()).await;
}

#[tokio::test]
async fn consumer_gzip_feature_does_not_change_response_bytes() {
    fixture::assert_no_decompression(
        super::client_builder().build().unwrap(),
        super::client_builder().gzip(true).build().unwrap(),
    )
    .await;
}
