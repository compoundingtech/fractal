//! Strict JSON boundary for TaskManifest v1, before any lossy map conversion.

use std::{borrow::Cow, fmt};

use serde::{Deserialize, Deserializer, de::{self, MapAccess, SeqAccess, Visitor}};
use serde_json::{Map, Value};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn parse_json(input: &[u8]) -> Result<Value, serde_json::Error> {
    let input = integer_negative_zero(input);
    let mut deserializer = serde_json::Deserializer::from_slice(&input);
    let value = StrictValue::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value.0)
}

// serde_json classifies the integer token -0 as floating point. Replace only that
// complete token, outside strings, with whitespace + zero. Keeping its byte width
// preserves error positions; fractions/exponents still reach the rejecting visitor.
// Allocate only when such a token occurs, not for ordinary manifests.
fn integer_negative_zero(input: &[u8]) -> Cow<'_, [u8]> {
    let mut output = Cow::Borrowed(input);
    let mut in_string = false;
    let mut escaped = false;
    for (index, &byte) in input.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'-'
            && input.get(index + 1) == Some(&b'0')
            && (index == 0 || matches!(input[index - 1], b'[' | b':' | b',' | b' ' | b'\n' | b'\r' | b'\t'))
            && input.get(index + 2).is_none_or(|byte| matches!(byte, b',' | b']' | b'}' | b' ' | b'\n' | b'\r' | b'\t'))
        {
            output.to_mut()[index] = b' ';
        }
    }
    output
}

// RFC 7493 §2.1 excludes noncharacters as well as unpaired surrogates. The JSON
// decoder rejects surrogates and invalid UTF-8; check scalar noncharacters here.
fn check_string<E: de::Error>(value: &str) -> Result<(), E> {
    if value.chars().any(|character| {
        let code = u32::from(character);
        (0xfdd0..=0xfdef).contains(&code) || code & 0xffff >= 0xfffe
    }) {
        return Err(E::custom("Unicode noncharacters are not I-JSON"));
    }
    Ok(())
}

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("non-null I-JSON with only safe integer numbers")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue(Value::Bool(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        if value.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(E::custom("integer exceeds the safe integer range"));
        }
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        if value > MAX_SAFE_INTEGER {
            return Err(E::custom("integer exceeds the safe integer range"));
        }
        Ok(StrictValue(Value::Number(value.into())))
    }

    fn visit_f64<E: de::Error>(self, _value: f64) -> Result<Self::Value, E> {
        Err(E::custom("fractional and exponent number tokens are forbidden"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Err(E::custom("null is forbidden; omit optional properties instead"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        check_string(value)?;
        Ok(StrictValue(Value::String(value.to_owned())))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        check_string(&value)?;
        Ok(StrictValue(Value::String(value)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictValue>()? {
            values.push(value.0);
        }
        Ok(StrictValue(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            check_string(&key)?;
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate object key"));
            }
            let value = map.next_value::<StrictValue>()?;
            values.insert(key, value.0);
        }
        Ok(StrictValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_json;
    use serde_json::json;

    #[test]
    fn rejects_duplicate_keys_at_every_depth_and_after_unescaping() {
        for input in [
            r#"{"a":1,"a":2}"#,
            r#"{"outer":{"a":1,"a":2}}"#,
            r#"[{"a":1,"a":2}]"#,
            r#"{"a":1,"\u0061":2}"#,
            r#"{"é":1,"\u00e9":2}"#,
            r#"{"𝄞":1,"\ud834\udd1e":2}"#,
        ] {
            let error = parse_json(input.as_bytes()).unwrap_err();
            assert!(error.to_string().contains("duplicate object key"), "{input}: {error}");
        }
        // Duplicate detection happens before reading the duplicate's value.
        let error = parse_json(br#"{"a":1,"a":null}"#).unwrap_err();
        assert!(error.to_string().contains("duplicate object key"));
        assert!(parse_json(br#"{"a":1,"nested":{"a":2}}"#).is_ok());
    }

    #[test]
    fn rejects_null_anywhere() {
        for input in ["null", "[null]", r#"{"optional":null}"#, r#"{"a":[{"b":null}]}"#] {
            assert!(parse_json(input.as_bytes()).is_err(), "{input}");
        }
    }

    #[test]
    fn accepts_only_safe_integer_tokens() {
        for input in ["0", "-1", "1", "9007199254740991", "-9007199254740991"] {
            assert!(parse_json(input.as_bytes()).is_ok(), "{input}");
        }
        for input in [
            "9007199254740992", "-9007199254740992", "9223372036854775807",
            "-9223372036854775808", "18446744073709551615", "18446744073709551616",
            "-18446744073709551616", "99999999999999999999999999999999999999999",
            "0.0", "1.0", "-1.0", "1e0", "1E+0", "1e-0", "0e0", "-0.0",
            "-0e0", "-0E+0", "1e999", "1e-999", "NaN", "Infinity", "-Infinity",
        ] {
            assert!(parse_json(input.as_bytes()).is_err(), "{input}");
            let nested = format!("{{\"a\":[{input}]}}");
            assert!(parse_json(nested.as_bytes()).is_err(), "{nested}");
        }
    }

    #[test]
    fn canonicalizes_integer_negative_zero_without_rewriting_strings() {
        assert_eq!(parse_json(b"-0").unwrap(), json!(0));
        assert_eq!(parse_json(b" \t-0\r\n").unwrap(), json!(0));
        let value = parse_json(br#"{"-0":"-0","escaped":"\"-0","numbers":[-0,0,-1],"nested":{"n":-0}}"#).unwrap();
        assert_eq!(value, json!({"-0":"-0", "escaped":"\"-0", "numbers":[0,0,-1], "nested":{"n":0}}));
        assert_eq!(serde_json::to_string(&value["numbers"][0]).unwrap(), "0");
        for input in ["-00", "--0", "- 0", "[-0-0]", "[-0.0]", "[-0e0]", "-0x", "-0 0"] {
            assert!(parse_json(input.as_bytes()).is_err(), "{input}");
        }
    }

    #[test]
    fn preserves_unicode_strings_and_distinct_normalization_forms() {
        let input = r#"{"é":"é","é":"é","𝄞":"\ud834\udd1e","controls":"\u0000\b\f\n\r\t"}"#;
        assert_eq!(parse_json(input.as_bytes()).unwrap(), json!({
            "é":"é", "é":"é", "𝄞":"𝄞", "controls":"\0\u{8}\u{c}\n\r\t",
        }));
    }

    #[test]
    fn rejects_invalid_unicode_in_values_and_keys() {
        for input in [
            br#""\ud800""#.as_slice(), br#""\udfff""#, br#""\ud834x""#,
            br#""\udd1e\ud834""#, br#"{"\ud800":1}"#,
            b"\"\xff\"", b"{\"\xff\":1}", b"\"\xc0\xaf\"", b"\"\xed\xa0\x80\"",
            b"\"\xf4\x90\x80\x80\"", b"\"\xe2\x82\"", b"\"raw\nnewline\"",
        ] {
            assert!(parse_json(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn rejects_ijson_noncharacters_in_values_and_keys() {
        let codes = (0xfdd0..=0xfdef).chain((0..=16).flat_map(|plane| {
            [plane * 0x10000 + 0xfffe, plane * 0x10000 + 0xffff]
        }));
        for code in codes {
            let character = char::from_u32(code).unwrap();
            for value in [json!(character.to_string()), json!({character.to_string(): 1})] {
                let input = serde_json::to_vec(&value).unwrap();
                assert!(parse_json(&input).is_err(), "U+{code:04X}");
            }
        }
        assert!(parse_json(br#""\ufdd0""#).is_err());
        assert!(parse_json(br#""\udbff\udfff""#).is_err());
    }

    #[test]
    fn requires_complete_valid_json_input() {
        for input in [
            "", " ", "{}{}", "{} trailing", "true false", "[1,]", "{\"a\":1,}",
            "[+0]", "[01]", "/* comment */{}", "\u{feff}{}",
        ] {
            assert!(parse_json(input.as_bytes()).is_err(), "{input}");
        }
        assert_eq!(parse_json(b" \n{\"a\":[true,false,{},[]]}\t ").unwrap(), json!({"a":[true,false,{},[]]}));
    }
}
