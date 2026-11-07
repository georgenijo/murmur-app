//! Local, correctly rounded JSON numbers without workspace-wide serde features.
use super::{Error, Result};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{Number, Value};

/// Let serde_json validate JSON grammar and integer ranges, but use Rust's
/// correctly rounded decimal parser for floats. Substitute every numeric token
/// with its index before parsing the tree, then restore the original numbers.
/// Strings are copied verbatim, so no user key or string can alias an index.
pub(super) fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    // Preserve the original typed decoder's rejection of duplicate struct
    // fields and invalid types; parsing through Value alone would hide them.
    serde_json::from_slice::<T>(bytes).map_err(|_| Error::Invalid)?;
    let mut rewritten = Vec::with_capacity(bytes.len());
    let mut numbers = Vec::new();
    let mut position = 0;
    while position < bytes.len() {
        let start = position;
        match bytes[position] {
            b'"' => {
                position += 1;
                while position < bytes.len() {
                    match bytes[position] {
                        b'\\' => position += 2,
                        b'"' => {
                            position += 1;
                            break;
                        }
                        _ => position += 1,
                    }
                }
                rewritten.extend_from_slice(bytes.get(start..position).ok_or(Error::Invalid)?);
            }
            b'-' | b'0'..=b'9' => {
                position += 1;
                while bytes
                    .get(position)
                    .is_some_and(|b| matches!(b, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'))
                {
                    position += 1;
                }
                let token = &bytes[start..position];
                // Validate the original number syntax even though the tree will
                // see an integer index. Preserve signed zero and integer types.
                let mut number: Number =
                    serde_json::from_slice(token).map_err(|_| Error::Invalid)?;
                if number.is_f64() {
                    let float = std::str::from_utf8(token)
                        .map_err(|_| Error::Invalid)?
                        .parse::<f64>()
                        .map_err(|_| Error::Invalid)?;
                    number = Number::from_f64(float).ok_or(Error::Invalid)?;
                }
                rewritten.extend_from_slice(numbers.len().to_string().as_bytes());
                numbers.push(number);
            }
            byte => {
                rewritten.push(byte);
                position += 1;
            }
        }
    }
    let mut value: Value = serde_json::from_slice(&rewritten).map_err(|_| Error::Invalid)?;
    fn restore(value: &mut Value, numbers: &[Number]) -> Result<()> {
        match value {
            Value::Number(index) => {
                *index = numbers
                    .get(index.as_u64().ok_or(Error::Invalid)? as usize)
                    .ok_or(Error::Invalid)?
                    .clone();
            }
            Value::Array(values) => {
                for value in values {
                    restore(value, numbers)?;
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    restore(value, numbers)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    restore(&mut value, &numbers)?;
    serde_json::from_value(value).map_err(|_| Error::Invalid)
}

/// Return a root member's original JSON bytes, excluding surrounding whitespace.
/// The caller validates the whole envelope first. IgnoredAny locates a value's
/// end without materializing or reserializing any numbers in that value.
pub(super) fn member_bytes<'a>(bytes: &'a [u8], member: &str) -> Option<&'a [u8]> {
    fn whitespace(bytes: &[u8], position: &mut usize) {
        while bytes.get(*position).is_some_and(u8::is_ascii_whitespace) {
            *position += 1;
        }
    }
    let mut position = 0;
    whitespace(bytes, &mut position);
    if bytes.get(position) != Some(&b'{') {
        return None;
    }
    position += 1;
    let mut found = None;
    let mut keys_seen = std::collections::HashSet::new();
    loop {
        whitespace(bytes, &mut position);
        if bytes.get(position) == Some(&b'}') {
            return found;
        }
        let mut keys =
            serde_json::Deserializer::from_slice(&bytes[position..]).into_iter::<String>();
        let key = keys.next()?.ok()?;
        if !keys_seen.insert(key.clone()) {
            return None;
        }
        position += keys.byte_offset();
        whitespace(bytes, &mut position);
        if bytes.get(position) != Some(&b':') {
            return None;
        }
        position += 1;
        whitespace(bytes, &mut position);
        let start = position;
        let mut values =
            serde_json::Deserializer::from_slice(&bytes[position..]).into_iter::<IgnoredAny>();
        values.next()?.ok()?;
        position += values.byte_offset();
        if key == member {
            found = Some(&bytes[start..position]);
        }
        whitespace(bytes, &mut position);
        if bytes.get(position) == Some(&b'}') {
            return found;
        }
        if bytes.get(position) != Some(&b',') {
            return None;
        }
        position += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_roundtrip_exact_bits_without_touching_strings_or_integers() {
        let mut values = vec![
            -0.0,
            0.0,
            f64::MAX,
            f64::MIN,
            f64::MIN_POSITIVE,
            f64::from_bits(1),
            1.5287289387243241e-9,
            1.0000226757369615,
        ];
        let mut bits = 0x123456789abcdef0_u64;
        for _ in 0..4096 {
            bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = f64::from_bits(bits);
            if value.is_finite() {
                values.push(value);
            }
        }
        let original = serde_json::json!({
            "nested": [values, {"0": "\\\"123.45e-6\\\\", "unicode": "\\u0000 ☃"}],
            "unsigned": u64::MAX, "signed": i64::MIN, "null": null,
        });
        let bytes = serde_json::to_vec(&original).unwrap();
        let decoded: Value = from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
        let floats: Vec<f64> = serde_json::from_value(decoded["nested"][0].clone()).unwrap();
        for (before, after) in values.iter().zip(floats) {
            assert_eq!(before.to_bits(), after.to_bits());
        }
    }

    #[test]
    fn malformed_original_numbers_and_json_are_rejected() {
        for bytes in [
            "01",
            "-01",
            "1.",
            "1e",
            "1e+",
            "1e400",
            "NaN",
            "[1-2]",
            "[1 2]",
            "{1:2}",
            "[0,]",
            "\"unterminated\\",
            "\"invalid\\x\"",
        ] {
            assert!(from_slice::<Value>(bytes.as_bytes()).is_err(), "{bytes}");
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Schema {
            #[serde(rename = "schema")]
            _schema: u32,
        }
        assert!(from_slice::<Schema>(br#"{"schema":1,"schema":2}"#).is_err());
    }

    #[test]
    fn digest_member_uses_original_bytes_and_rejects_duplicate_envelope_keys() {
        let bytes = br#" {"snapshots": [ 1.0000226757369615, {"snapshots": 2} ], "schema":1} "#;
        assert_eq!(
            member_bytes(bytes, "snapshots").unwrap(),
            br#"[ 1.0000226757369615, {"snapshots": 2} ]"#
        );
        assert!(member_bytes(br#"{"snapshots":[],"snapshots":[1]}"#, "snapshots").is_none());
        assert!(member_bytes(br#"{"snapshots":[],"schema":1,"schema":2}"#, "snapshots").is_none());
    }
}
