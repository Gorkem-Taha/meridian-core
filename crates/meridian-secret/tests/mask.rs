//! The output filter: a secret a tool prints is masked in every form a tool
//! is likely to print it.

use base64::Engine;
use meridian_secret::{Masker, SecretValue, MASK};

fn secret(text: &str) -> SecretValue {
    SecretValue::from_reader(text.as_bytes(), 4096).unwrap()
}

fn masker(texts: &[&str]) -> Masker {
    let mut masker = Masker::new();
    for text in texts {
        assert!(
            masker.add(&secret(text)).unwrap(),
            "every text here is long enough to search for"
        );
    }
    masker
}

const TOKEN: &str = "tok_9fXw/kq7Zp2+not real?";

#[test]
fn a_secret_is_masked_wherever_it_appears_in_a_line() {
    let masker = masker(&[TOKEN]);
    assert_eq!(
        masker.mask(&format!("Authorization: Bearer {TOKEN}")),
        format!("Authorization: Bearer {MASK}")
    );
    assert_eq!(
        masker.mask(&format!("{TOKEN} and again {TOKEN}.")),
        format!("{MASK} and again {MASK}.")
    );
    assert_eq!(masker.mask(TOKEN), MASK);
    // Twice with nothing between is one mask, not two.
    assert_eq!(masker.mask(&format!("{TOKEN}{TOKEN}")), MASK);
    assert_eq!(masker.mask("nothing to hide here"), "nothing to hide here");
    assert_eq!(masker.mask(""), "");
    assert!(masker.finds(format!("x{TOKEN}y").as_bytes()));
    assert!(!masker.finds(b"nothing to hide here"));
}

#[test]
fn the_encoded_forms_a_tool_prints_are_masked_too() {
    let masker = masker(&[TOKEN]);
    let engines = [
        base64::engine::general_purpose::STANDARD.encode(TOKEN),
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(TOKEN),
        base64::engine::general_purpose::URL_SAFE.encode(TOKEN),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(TOKEN),
    ];
    for encoded in engines {
        let masked = masker.mask(&format!("data: {encoded}"));
        assert!(!masked.contains(&encoded), "{masked}");
        assert!(masked.contains(MASK), "{masked}");
    }
    let hex: String = TOKEN.bytes().map(|b| format!("{b:02x}")).collect();
    assert_eq!(masker.mask(&format!("sha: {hex}")), format!("sha: {MASK}"));
    let url = "tok_9fXw%2Fkq7Zp2%2Bnot%20real%3F";
    assert_eq!(
        masker.mask(&format!("https://x/?t={url}&a=1")),
        format!("https://x/?t={MASK}&a=1")
    );
}

#[test]
fn each_line_of_a_secret_with_several_is_masked_on_its_own() {
    let key = "-----BEGIN KEY-----\nMIIEvQIBADANBgkqhkiG9w0BAQEF\nAASCBKcwggSjAgEAAoIBAQC7\r\n-----END KEY-----\n";
    let masker = masker(&[key]);
    // A tool that echoes a key prints it a line at a time, with its own
    // prefix on each.
    let echoed = "debug1: MIIEvQIBADANBgkqhkiG9w0BAQEF\ndebug1: AASCBKcwggSjAgEAAoIBAQC7\n";
    assert_eq!(
        masker.mask(echoed),
        format!("debug1: {MASK}\ndebug1: {MASK}\n")
    );
    // And whole.
    assert_eq!(masker.mask(key), MASK);
}

#[test]
fn several_secrets_are_all_masked_and_the_longest_wins() {
    let masker = masker(&["password-one", "password-one-and-more", "zzzz"]);
    assert_eq!(
        masker.mask("a password-one-and-more b password-one c zzzz"),
        format!("a {MASK} b {MASK} c {MASK}")
    );
}

#[test]
fn a_secret_too_short_to_search_for_is_reported_and_not_added() {
    let mut masker = Masker::new();
    for short in ["", "a", "ab", "abc"] {
        assert!(!masker.add(&secret(short)).unwrap(), "{short:?}");
    }
    assert!(masker.is_empty());
    assert_eq!(masker.mask("abc is left alone"), "abc is left alone");
    assert!(masker.add(&secret("abcd")).unwrap());
    assert_eq!(masker.mask("abcd is not"), format!("{MASK} is not"));
}

#[test]
fn output_that_is_not_text_is_masked_as_bytes() {
    let masker = masker(&[TOKEN]);
    let mut output = vec![0xff, 0xfe, 0x00];
    output.extend_from_slice(TOKEN.as_bytes());
    output.extend_from_slice(&[0x00, 0xff]);
    let masked = masker.mask_bytes(&output);
    let mut want = vec![0xff, 0xfe, 0x00];
    want.extend_from_slice(MASK.as_bytes());
    want.extend_from_slice(&[0x00, 0xff]);
    assert_eq!(masked, want);
}

#[test]
fn a_masker_shows_how_many_forms_it_holds_and_none_of_them() {
    let masker = masker(&[TOKEN]);
    let shown = format!("{masker:?}");
    assert!(shown.starts_with("Masker("), "{shown}");
    assert!(!shown.contains("tok_"), "{shown}");
}

#[test]
fn masking_is_the_same_whichever_order_secrets_were_added_in() {
    let a = masker(&["first-secret", "second-secret"]);
    let b = masker(&["second-secret", "first-secret"]);
    let line = "x first-secret y second-secret z first-secretsecond-secret";
    assert_eq!(a.mask(line), b.mask(line));
}
