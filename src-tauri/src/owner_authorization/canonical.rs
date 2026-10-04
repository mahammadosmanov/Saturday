use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CanonicalError {
    #[error("value is outside the canonical JSON domain")]
    InvalidValue,
    #[error("hex value is invalid")]
    InvalidHex,
    #[error("base64 value is invalid")]
    InvalidBase64,
    #[error("decoded value exceeds the maximum size")]
    Oversized,
}

pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, CanonicalError> {
    let mut output = Vec::new();
    encode_value(value, &mut output)?;
    Ok(output)
}

pub fn decode_lower_hex<const N: usize>(value: &str) -> Result<[u8; N], CanonicalError> {
    if value.len() != N.checked_mul(2).ok_or(CanonicalError::InvalidHex)?
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CanonicalError::InvalidHex);
    }

    let mut decoded = [0_u8; N];
    for (index, byte) in decoded.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = (hex_nibble(value.as_bytes()[offset])? << 4)
            | hex_nibble(value.as_bytes()[offset + 1])?;
    }
    Ok(decoded)
}

pub fn decode_canonical_base64(value: &str, maximum: usize) -> Result<Vec<u8>, CanonicalError> {
    let decoded = STANDARD
        .decode(value)
        .map_err(|_| CanonicalError::InvalidBase64)?;
    if decoded.len() > maximum {
        return Err(CanonicalError::Oversized);
    }
    if STANDARD.encode(&decoded) != value {
        return Err(CanonicalError::InvalidBase64);
    }
    Ok(decoded)
}

fn encode_value(value: &Value, output: &mut Vec<u8>) -> Result<(), CanonicalError> {
    match value {
        Value::Null => Err(CanonicalError::InvalidValue),
        Value::Bool(boolean) => {
            output.extend_from_slice(if *boolean { b"true" } else { b"false" });
            Ok(())
        }
        Value::Number(number) => {
            let integer = number.as_i64().ok_or(CanonicalError::InvalidValue)?;
            output.extend_from_slice(integer.to_string().as_bytes());
            Ok(())
        }
        Value::String(string) => encode_string(string, output),
        Value::Array(values) => {
            output.push(b'[');
            for (index, item) in values.iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                encode_value(item, output)?;
            }
            output.push(b']');
            Ok(())
        }
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            output.push(b'{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(b',');
                }
                encode_string(key, output)?;
                output.push(b':');
                encode_value(&values[key], output)?;
            }
            output.push(b'}');
            Ok(())
        }
    }
}

fn encode_string(value: &str, output: &mut Vec<u8>) -> Result<(), CanonicalError> {
    if !value.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err(CanonicalError::InvalidValue);
    }
    let encoded = serde_json::to_string(value).map_err(|_| CanonicalError::InvalidValue)?;
    output.extend_from_slice(encoded.as_bytes());
    Ok(())
}

fn hex_nibble(value: u8) -> Result<u8, CanonicalError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(CanonicalError::InvalidHex),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owner_authorization::PAYLOAD_SCHEMA;

    const EXPECTED_PAYLOAD: &[u8] = br#"{"binding_sha256":"2222222222222222222222222222222222222222222222222222222222222222","expires_at_unix":1795000300,"generation":"00112233445566778899aabbccddeeff","issued_at_unix":1795000000,"model_transfer":"blocked","nonce_sha256":"1111111111111111111111111111111111111111111111111111111111111111","purpose":"stage1_bottle_transfer","schema":"saturday.local-provider.stage1-owner-authorization-payload.v1","state":"granted"}"#;

    #[test]
    fn fixture_is_exact_ascii_sorted_json() {
        let payload = serde_json::json!({
            "schema": PAYLOAD_SCHEMA,
            "state": "granted",
            "purpose": "stage1_bottle_transfer",
            "generation": "00112233445566778899aabbccddeeff",
            "issued_at_unix": 1_795_000_000_i64,
            "expires_at_unix": 1_795_000_300_i64,
            "nonce_sha256": "11".repeat(32),
            "binding_sha256": "22".repeat(32),
            "model_transfer": "blocked"
        });

        assert_eq!(canonical_json_bytes(&payload).unwrap(), EXPECTED_PAYLOAD);
    }

    #[test]
    fn rejects_values_outside_the_canonical_domain() {
        assert!(canonical_json_bytes(&serde_json::json!({"value": "é"})).is_err());
        assert!(canonical_json_bytes(&serde_json::json!({"value": 1.5})).is_err());
        assert!(canonical_json_bytes(&serde_json::json!({"value": null})).is_err());
        assert!(canonical_json_bytes(&serde_json::json!({"bad\nkey": 1})).is_err());
    }

    #[test]
    fn schema_integer_callers_do_not_accept_booleans() {
        let document = serde_json::json!({"issued_at_unix": true});
        assert!(document["issued_at_unix"].as_i64().is_none());
    }

    #[test]
    fn lowercase_hex_is_exact_length_and_alphabet() {
        assert_eq!(
            decode_lower_hex::<4>("0011aaff").unwrap(),
            [0x00, 0x11, 0xaa, 0xff]
        );
        for value in ["0011AAff", "0011aaf", "0011aaff00", "0011aagf"] {
            assert!(decode_lower_hex::<4>(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn base64_requires_padding_round_trip_and_bound() {
        assert_eq!(decode_canonical_base64("YQ==", 1).unwrap(), b"a");
        for value in ["YQ", "YQ=", "YR==", "YQ===", "YQ==\n"] {
            assert!(
                decode_canonical_base64(value, 8).is_err(),
                "accepted {value:?}"
            );
        }
        assert!(decode_canonical_base64("YWI=", 1).is_err());
    }
}
