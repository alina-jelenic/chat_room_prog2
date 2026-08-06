use chat_room_prog2::controller::auth::{Claims, create_jwt, validate_jwt_secret, verify_jwt};

use jsonwebtoken::{EncodingKey, Header, encode};
const TEST_SECRET: &str = "test-secret-that-is-longer-than-32-characters";

#[test]
fn jwt_secret_and_signature_are_validated() {
    assert!(validate_jwt_secret("prekratko").is_err());
    assert!(create_jwt(1, "alina", "prekratko").is_err());

    let token = create_jwt(1, "alina", TEST_SECRET).unwrap();
    let claims = verify_jwt(&token, TEST_SECRET).unwrap();
    assert_eq!(claims.sub, 1);
    assert_eq!(claims.username, "alina");
    assert!(verify_jwt(&token, "a-different-secret-that-is-also-long-enough").is_none());
    assert!(verify_jwt("to-ni-jwt", TEST_SECRET).is_none());
}

#[test]
fn expired_jwt_is_rejected() {
    let expired_claims = Claims {
        sub: 1,
        username: "alina".to_string(),
        exp: 1,
    };
    let token = encode(
        &Header::default(),
        &expired_claims,
        &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
    )
    .unwrap();

    assert!(verify_jwt(&token, TEST_SECRET).is_none());
}
