use zeus_core::Config;

#[test]
fn x_twitter_accepts_official_consumer_credential_names() {
    let toml = r#"
model = "anthropic/claude-sonnet-4"

[channels.x_twitter]
bearer_token = "bearer"
consumer_key = "consumer"
consumer_key_secret = "consumer-secret"
access_token = "access"
access_token_secret = "access-secret"
client_id = "client-id"
client_secret = "client-secret"
oauth2_access_token = "oauth2-access"
oauth2_refresh_token = "oauth2-refresh"
oauth2_expires_at = 1735689600
"#;

    let cfg: Config = toml::from_str(toml).expect("official x_twitter TOML parses");
    let x = cfg
        .channels
        .as_ref()
        .and_then(|c| c.x_twitter.as_ref())
        .expect("x_twitter section");

    assert_eq!(x.bearer_token, "bearer");
    assert_eq!(x.consumer_key, "consumer");
    assert_eq!(x.consumer_key_secret, "consumer-secret");
    assert_eq!(x.access_token, "access");
    assert_eq!(x.access_token_secret, "access-secret");
    assert_eq!(x.client_id, "client-id");
    assert_eq!(x.client_secret, "client-secret");
    assert_eq!(x.oauth2_access_token, "oauth2-access");
    assert_eq!(x.oauth2_refresh_token, "oauth2-refresh");
    assert_eq!(x.oauth2_expires_at, 1_735_689_600);
}

#[test]
fn x_twitter_still_accepts_legacy_api_credential_names() {
    let toml = r#"
model = "anthropic/claude-sonnet-4"

[channels.x_twitter]
api_key = "legacy-consumer"
api_secret = "legacy-consumer-secret"
access_token = "access"
access_token_secret = "access-secret"
"#;

    let cfg: Config = toml::from_str(toml).expect("legacy x_twitter TOML parses");
    let x = cfg
        .channels
        .as_ref()
        .and_then(|c| c.x_twitter.as_ref())
        .expect("x_twitter section");

    assert_eq!(x.consumer_key, "legacy-consumer");
    assert_eq!(x.consumer_key_secret, "legacy-consumer-secret");
    assert!(x.oauth2_access_token.is_empty());
    assert!(x.oauth2_refresh_token.is_empty());
    assert_eq!(x.oauth2_expires_at, 0);
}
