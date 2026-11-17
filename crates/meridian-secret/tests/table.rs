//! The secret table: a run's secrets by number, joined, encoded and compared
//! without leaving protected memory, and described rather than held when
//! they have not been fetched.

use meridian_secret::{Piece, SecretTable, SecretValue, TableError, Transform};
use meridian_value::SecretRef;

fn secret(text: &str) -> SecretValue {
    SecretValue::from_reader(text.as_bytes(), 4096).unwrap()
}

fn is(table: &SecretTable, at: SecretRef, want: &str) -> bool {
    table
        .with(at, |value| value.expose(|bytes| bytes == want.as_bytes()))
        .unwrap()
}

#[test]
fn a_reference_shows_nothing_of_the_secret() {
    let table = SecretTable::new();
    let at = table.insert(secret("hunter2-and-more"));
    assert_eq!(format!("{at:?}"), "(sensitive)");
    assert_eq!(table.describe(at), "(sensitive)");
    let shown = format!("{table:?}");
    assert_eq!(shown, "SecretTable(1 secrets)");
    assert!(is(&table, at, "hunter2-and-more"));
}

#[test]
fn text_and_secrets_are_joined_into_a_new_secret() {
    let table = SecretTable::new();
    let user = table.insert(secret("deploy"));
    let password = table.insert(secret("p@ss/w0rd"));
    let joined = table
        .join(&[
            Piece::Text("postgres://"),
            Piece::Secret(user),
            Piece::Text(":"),
            Piece::Secret(password),
            Piece::Text("@db/app"),
        ])
        .unwrap();
    assert!(is(&table, joined, "postgres://deploy:p@ss/w0rd@db/app"));
    assert_eq!(table.describe(joined), "(sensitive)");
    // The parts are untouched.
    assert!(is(&table, user, "deploy"));
    // Nothing at all joins to an empty secret.
    let empty = table.join(&[]).unwrap();
    assert!(table.is_empty(empty).unwrap());
}

#[test]
fn each_transform_gives_a_new_secret() {
    let table = SecretTable::new();
    let at = table.insert(secret("p@ss/w0rd +?"));
    let url = table.transform(at, Transform::UrlEncode).unwrap();
    assert!(is(&table, url, "p%40ss%2Fw0rd%20%2B%3F"));

    let encoded = table.transform(at, Transform::Base64Encode).unwrap();
    assert!(is(&table, encoded, "cEBzcy93MHJkICs/"));
    let decoded = table.transform(encoded, Transform::Base64Decode).unwrap();
    assert!(table.same(decoded, at).unwrap());

    // Every length of padding.
    for (plain, want) in [
        ("", ""),
        ("a", "YQ=="),
        ("ab", "YWI="),
        ("abc", "YWJj"),
        ("abcd", "YWJjZA=="),
    ] {
        let at = table.insert(secret(plain));
        let encoded = table.transform(at, Transform::Base64Encode).unwrap();
        assert!(is(&table, encoded, want), "{plain}");
        let back = table.transform(encoded, Transform::Base64Decode).unwrap();
        assert!(is(&table, back, plain), "{plain}");
    }

    let padded = table.insert(secret("  \t tok_123 \r\n"));
    let trimmed = table.transform(padded, Transform::Trim).unwrap();
    assert!(is(&table, trimmed, "tok_123"));
    let blank = table.insert(secret(" \n "));
    let trimmed = table.transform(blank, Transform::Trim).unwrap();
    assert!(table.is_empty(trimmed).unwrap());
}

#[test]
fn a_secret_that_is_not_base64_is_refused_without_quoting_it() {
    let table = SecretTable::new();
    let at = table.insert(secret("not base64 at all!"));
    let err = table.transform(at, Transform::Base64Decode).unwrap_err();
    assert_eq!(
        err,
        TableError::Invalid("base64Decode(...): the secret is not base64".to_owned())
    );
    assert!(!err.to_string().contains("not base64 at all"));
}

#[test]
fn secrets_are_compared_with_each_other_and_with_text() {
    let table = SecretTable::new();
    let a = table.insert(secret("same-bytes"));
    let b = table.insert(secret("same-bytes"));
    let c = table.insert(secret("other-bytes"));
    assert!(table.same(a, b).unwrap());
    assert!(!table.same(a, c).unwrap());
    assert!(table.same_as_text(a, "same-bytes").unwrap());
    assert!(!table.same_as_text(a, "same-byte").unwrap());
    assert!(!table.same_as_text(a, "").unwrap());
    assert!(!table.is_empty(a).unwrap());
}

#[test]
fn a_secret_not_yet_fetched_is_described_and_never_guessed() {
    let table = SecretTable::new();
    let password = table.declare("db_password");
    assert!(!table.is_held(password));
    assert_eq!(table.describe(password), "(secret db_password)");

    // What is made from it says what it would be.
    let line = table
        .join(&[
            Piece::Text("password="),
            Piece::Secret(password),
            Piece::Text("\n"),
        ])
        .unwrap();
    assert_eq!(table.describe(line), "password=(secret db_password)\n");
    let encoded = table.transform(password, Transform::UrlEncode).unwrap();
    assert_eq!(table.describe(encoded), "urlEncode((secret db_password))");

    // A fetched secret beside it is still not written.
    let user = table.insert(secret("deploy-user"));
    let both = table
        .join(&[
            Piece::Secret(user),
            Piece::Text(":"),
            Piece::Secret(password),
        ])
        .unwrap();
    assert_eq!(table.describe(both), "(sensitive):(secret db_password)");

    // Nothing can be asked of the value.
    let not_fetched = TableError::NotFetched("(secret db_password)".to_owned());
    assert_eq!(table.same_as_text(password, "x").unwrap_err(), not_fetched);
    assert_eq!(table.same(password, user).unwrap_err(), not_fetched);
    assert_eq!(table.is_empty(password).unwrap_err(), not_fetched);
    assert_eq!(
        not_fetched.to_string(),
        "(secret db_password) is not fetched here: a secret is fetched by plan and apply, never \
         by validate or compile"
    );

    // Until it is fetched.
    assert!(table.hold(password, secret("fetched-now")));
    assert!(table.is_held(password));
    assert_eq!(table.describe(password), "(sensitive)");
    assert!(table.same_as_text(password, "fetched-now").unwrap());
    assert!(!table.hold(SecretRef::new(999), secret("nowhere")));
}

#[test]
fn a_wiped_secret_is_gone_and_its_reference_says_so() {
    let table = SecretTable::new();
    let a = table.insert(secret("first-secret"));
    let b = table.insert(secret("second-secret"));
    assert_eq!(table.held(), 2);
    table.wipe(a);
    assert_eq!(table.held(), 1);
    assert_eq!(table.with(a, |_| ()).unwrap_err(), TableError::Gone);
    assert_eq!(
        table.join(&[Piece::Secret(a)]).unwrap_err(),
        TableError::Gone
    );
    assert_eq!(
        table.transform(a, Transform::Trim).unwrap_err(),
        TableError::Gone
    );
    assert_eq!(table.same(a, b).unwrap_err(), TableError::Gone);
    assert!(is(&table, b, "second-secret"));

    table.wipe_all();
    assert_eq!(table.held(), 0);
    assert_eq!(table.with(b, |_| ()).unwrap_err(), TableError::Gone);
    // A number that was never given out.
    assert_eq!(
        table.with(SecretRef::new(42), |_| ()).unwrap_err(),
        TableError::Gone
    );
    assert_eq!(table.describe(SecretRef::new(42)), "(sensitive)");
}

#[test]
fn the_functions_that_take_a_secret_are_the_transforms() {
    for name in Transform::FUNCTIONS {
        assert!(Transform::of_function(name).is_some(), "{name}");
    }
    for name in ["upper", "sha256", "toString", "length", "toJson", "replace"] {
        assert!(Transform::of_function(name).is_none(), "{name}");
    }
}
