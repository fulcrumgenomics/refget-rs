//! Digest computation for GA4GH refget: SHA-512/24 with base64url encoding
//! and RFC-8785 JSON Canonicalization Scheme (JCS).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha512};

/// Compute the GA4GH sha512t24u digest of the given data.
///
/// This truncates the SHA-512 hash to 24 bytes and encodes it as base64url
/// without padding, producing a 32-character string.
pub fn sha512t24u(data: &[u8]) -> String {
    let hash = Sha512::digest(data);
    let truncated = &hash[..24];
    URL_SAFE_NO_PAD.encode(truncated)
}

/// Canonicalize a JSON value according to RFC 8785 (JCS).
///
/// This produces a deterministic byte representation suitable for hashing.
/// Key ordering is lexicographic by Unicode code point, numbers use the
/// shortest representation, and no whitespace is added.
pub fn jcs_canonicalize(value: &serde_json::Value) -> Vec<u8> {
    let mut buf = Vec::new();
    write_canonical(value, &mut buf);
    buf
}

/// Canonicalize then compute sha512t24u of a JSON value.
pub fn digest_json(value: &serde_json::Value) -> String {
    let canonical = jcs_canonicalize(value);
    sha512t24u(&canonical)
}

fn write_canonical(value: &serde_json::Value, buf: &mut Vec<u8>) {
    match value {
        serde_json::Value::Null => buf.extend_from_slice(b"null"),
        serde_json::Value::Bool(b) => {
            if *b {
                buf.extend_from_slice(b"true");
            } else {
                buf.extend_from_slice(b"false");
            }
        }
        serde_json::Value::Number(n) => {
            // RFC 8785: use the shortest representation.
            // serde_json's Display for Number already produces the right format
            // for integers. For floats, we need to ensure no trailing zeros.
            let s = n.to_string();
            buf.extend_from_slice(s.as_bytes());
        }
        serde_json::Value::String(s) => {
            write_canonical_string(s, buf);
        }
        serde_json::Value::Array(arr) => {
            buf.push(b'[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    buf.push(b',');
                }
                write_canonical(item, buf);
            }
            buf.push(b']');
        }
        serde_json::Value::Object(obj) => {
            // RFC 8785: keys sorted by UTF-16 code units. For ASCII-only keys
            // (common in refget), this is equivalent to lexicographic byte order.
            // For full correctness, we sort by UTF-16 encoding.
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort_by(|a, b| cmp_utf16(a, b));

            buf.push(b'{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    buf.push(b',');
                }
                write_canonical_string(key, buf);
                buf.push(b':');
                write_canonical(&obj[*key], buf);
            }
            buf.push(b'}');
        }
    }
}

/// Compare two strings by their UTF-16 code unit sequences, as required by RFC 8785.
fn cmp_utf16(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Write a JSON-escaped string to the buffer.
fn write_canonical_string(s: &str, buf: &mut Vec<u8>) {
    buf.push(b'"');
    for ch in s.chars() {
        match ch {
            '"' => buf.extend_from_slice(b"\\\""),
            '\\' => buf.extend_from_slice(b"\\\\"),
            '\x08' => buf.extend_from_slice(b"\\b"),
            '\x0C' => buf.extend_from_slice(b"\\f"),
            '\n' => buf.extend_from_slice(b"\\n"),
            '\r' => buf.extend_from_slice(b"\\r"),
            '\t' => buf.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                // Control characters must use \u00XX
                let code = c as u32;
                buf.extend_from_slice(format!("\\u{code:04x}").as_bytes());
            }
            c => {
                let mut utf8_buf = [0u8; 4];
                buf.extend_from_slice(c.encode_utf8(&mut utf8_buf).as_bytes());
            }
        }
    }
    buf.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha512t24u_empty() {
        // SHA-512 of empty string, truncated to 24 bytes, base64url-encoded
        let result = sha512t24u(b"");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_sha512t24u_spec_vector() {
        // GA4GH canonical test vector: ACGT → aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2
        let result = sha512t24u(b"ACGT");
        assert_eq!(result, "aKF498dAxcJAqme6QYQ7EZ07-fiw8Kw2");
    }

    #[test]
    fn test_sha512t24u_known_value() {
        let digest = sha512t24u(b"ACGT");
        assert_eq!(digest.len(), 32);
        // Verify it's valid base64url
        assert!(digest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn test_jcs_canonicalize_object_key_order() {
        let json: serde_json::Value = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        let canonical = jcs_canonicalize(&json);
        assert_eq!(String::from_utf8(canonical).unwrap(), r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn test_jcs_canonicalize_nested() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"z":{"b":2,"a":1},"a":"hello"}"#).unwrap();
        let canonical = jcs_canonicalize(&json);
        assert_eq!(String::from_utf8(canonical).unwrap(), r#"{"a":"hello","z":{"a":1,"b":2}}"#);
    }

    #[test]
    fn test_jcs_canonicalize_array() {
        let json: serde_json::Value = serde_json::from_str(r#"[3,1,2]"#).unwrap();
        let canonical = jcs_canonicalize(&json);
        assert_eq!(String::from_utf8(canonical).unwrap(), "[3,1,2]");
    }

    #[test]
    fn test_jcs_string_escaping() {
        let json = serde_json::Value::String("hello\nworld".to_string());
        let canonical = jcs_canonicalize(&json);
        assert_eq!(String::from_utf8(canonical).unwrap(), r#""hello\nworld""#);
    }

    #[test]
    fn test_digest_json() {
        let json: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
        let digest = digest_json(&json);
        assert_eq!(digest.len(), 32);
        // Should be deterministic
        assert_eq!(digest, digest_json(&json));
    }

    #[test]
    fn test_jcs_primitives() {
        assert_eq!(String::from_utf8(jcs_canonicalize(&serde_json::Value::Null)).unwrap(), "null");
        assert_eq!(
            String::from_utf8(jcs_canonicalize(&serde_json::Value::Bool(true))).unwrap(),
            "true"
        );
        assert_eq!(
            String::from_utf8(jcs_canonicalize(&serde_json::Value::Bool(false))).unwrap(),
            "false"
        );
    }

    #[test]
    fn test_sha512t24u_large_input() {
        // 1 MB of repeated bytes should still produce exactly 32 chars
        let data = vec![0xABu8; 1_000_000];
        let result = sha512t24u(&data);
        assert_eq!(result.len(), 32);
        assert!(result.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        // Determinism
        assert_eq!(result, sha512t24u(&data));
    }

    #[test]
    fn test_jcs_empty_object_and_array() {
        let obj: serde_json::Value = serde_json::from_str("{}").unwrap();
        assert_eq!(String::from_utf8(jcs_canonicalize(&obj)).unwrap(), "{}");

        let arr: serde_json::Value = serde_json::from_str("[]").unwrap();
        assert_eq!(String::from_utf8(jcs_canonicalize(&arr)).unwrap(), "[]");
    }

    #[test]
    fn test_jcs_numbers() {
        let zero: serde_json::Value = serde_json::from_str("0").unwrap();
        assert_eq!(String::from_utf8(jcs_canonicalize(&zero)).unwrap(), "0");

        let neg: serde_json::Value = serde_json::from_str("-1").unwrap();
        assert_eq!(String::from_utf8(jcs_canonicalize(&neg)).unwrap(), "-1");

        let frac: serde_json::Value = serde_json::from_str("1.5").unwrap();
        assert_eq!(String::from_utf8(jcs_canonicalize(&frac)).unwrap(), "1.5");

        let big: serde_json::Value = serde_json::from_str("9007199254740992").unwrap();
        assert_eq!(String::from_utf8(jcs_canonicalize(&big)).unwrap(), "9007199254740992");
    }

    #[test]
    fn test_jcs_string_all_escape_sequences() {
        // Build a string containing: quote, backslash, \b, \f, \n, \r, \t, and control char U+0001
        let input = "\"\\\x08\x0C\n\r\t\x01";
        let val = serde_json::Value::String(input.to_string());
        let canonical = String::from_utf8(jcs_canonicalize(&val)).unwrap();
        assert_eq!(canonical, r#""\"\\\b\f\n\r\t\u0001""#);
    }

    #[test]
    fn test_jcs_unicode_emoji() {
        let val = serde_json::Value::String("\u{1F600}".to_string()); // grinning face emoji
        let canonical = jcs_canonicalize(&val);
        // Emoji should be passed through as raw UTF-8, not escaped
        let s = String::from_utf8(canonical).unwrap();
        assert_eq!(s, "\"\u{1F600}\"");
    }

    #[test]
    fn test_jcs_deeply_nested() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"a":{"b":{"c":{"d":"deep"}}}}"#).unwrap();
        let canonical = String::from_utf8(jcs_canonicalize(&json)).unwrap();
        assert_eq!(canonical, r#"{"a":{"b":{"c":{"d":"deep"}}}}"#);
    }

    #[test]
    fn test_cmp_utf16_via_key_ordering() {
        // U+00E9 (é, Latin Small Letter E with Acute) has UTF-16 code unit 0x00E9.
        // U+0101 (ā, Latin Small Letter A with Macron) has UTF-16 code unit 0x0101.
        // By UTF-16 ordering, é (0x00E9) < ā (0x0101), even though by Unicode code point
        // 'a' < 'é' < 'ā' already holds. Use a case where UTF-16 differs from naive byte order:
        // U+FB33 (Hebrew Letter Dalet with Dagesh) encodes as a single UTF-16 unit 0xFB33,
        // while U+1F600 (emoji) encodes as a surrogate pair starting with 0xD83D.
        // 0xD83D < 0xFB33 in UTF-16, so emoji sorts before FB33.
        let json: serde_json::Value =
            serde_json::from_str(r#"{"\uFB33":1,"\uD83D\uDE00":2}"#).unwrap();
        let canonical = String::from_utf8(jcs_canonicalize(&json)).unwrap();
        // Emoji (surrogate pair 0xD83D,0xDE00) sorts before U+FB33 (0xFB33) in UTF-16 order
        assert!(canonical.starts_with("{\"\u{1F600}\":2"));
    }

    #[test]
    fn test_digest_json_different_objects() {
        let a: serde_json::Value = serde_json::from_str(r#"{"key":"value1"}"#).unwrap();
        let b: serde_json::Value = serde_json::from_str(r#"{"key":"value2"}"#).unwrap();
        let digest_a = digest_json(&a);
        let digest_b = digest_json(&b);
        assert_ne!(digest_a, digest_b);
        assert_eq!(digest_a.len(), 32);
        assert_eq!(digest_b.len(), 32);
    }
}
