//! Canonical base-62 position keys for local insertions into arrangement registers.
const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
fn integer_length(key: &str) -> usize {
    let head = key.as_bytes()[0];
    if head >= b'a' {
        usize::from(head - b'a') + 2
    } else {
        usize::from(b'Z' - head) + 2
    }
}
fn step_integer(integer: &str, forward: bool) -> Option<String> {
    let mut bytes = integer.as_bytes().to_vec();
    for index in (1..bytes.len()).rev() {
        let digit = DIGITS.iter().position(|d| *d == bytes[index])?;
        if forward && digit < 61 {
            bytes[index] = DIGITS[digit + 1];
            return (!(bytes[0] == b'A' && bytes[1..].iter().all(|d| *d == b'0')))
                .then(|| String::from_utf8(bytes).ok())
                .flatten();
        }
        if !forward && digit > 0 {
            bytes[index] = DIGITS[digit - 1];
            return (!(bytes[0] == b'A' && bytes[1..].iter().all(|d| *d == b'0')))
                .then(|| String::from_utf8(bytes).ok())
                .flatten();
        }
        bytes[index] = if forward { b'0' } else { b'z' };
    }
    let head = bytes[0];
    if forward && head == b'Z' {
        return Some("a0".into());
    }
    if !forward && head == b'a' {
        return Some("Zz".into());
    }
    if (forward && head == b'z') || (!forward && head == b'A') {
        return None;
    }
    let next = if forward { head + 1 } else { head - 1 };
    let mut key = String::from(char::from(next));
    key.extend(std::iter::repeat_n(
        if forward { '0' } else { 'z' },
        integer_length(&key) - 1,
    ));
    Some(key)
}
fn midpoint(a: &str, b: Option<&str>) -> String {
    let mut index = 0;
    let mut prefix = String::new();
    while let Some(upper) = b.and_then(|b| b.as_bytes().get(index)) {
        if a.as_bytes().get(index).copied().unwrap_or(b'0') != *upper {
            break;
        }
        prefix.push(char::from(*upper));
        index += 1;
    }
    let digit = |byte: &u8| DIGITS.iter().position(|d| d == byte).unwrap_or(0);
    let low = a.as_bytes().get(index).map(digit).unwrap_or(0);
    let high = b
        .and_then(|b| b.as_bytes().get(index))
        .map(digit)
        .unwrap_or(62);
    if high - low > 1 {
        prefix.push(char::from(DIGITS[(low + high) / 2]));
    } else if let Some(b) = b
        && b.len() > index + 1
    {
        prefix.push(char::from(b.as_bytes()[index]));
    } else {
        prefix.push(char::from(DIGITS[low]));
        prefix.push_str(&midpoint(a.get(index + 1..).unwrap_or(""), None));
    }
    prefix
}
pub(super) fn between(a: Option<&str>, b: Option<&str>) -> Result<String, String> {
    if a.zip(b).is_some_and(|(a, b)| a >= b) {
        return Err("These rows share a position key; reorder their group first.".into());
    }
    let key = match a {
        None => match b {
            None => "a0".into(),
            Some(b) => {
                let integer = &b[..integer_length(b)];
                if integer < b {
                    integer.into()
                } else {
                    step_integer(integer, false).ok_or("This ordering gap is full.")?
                }
            }
        },
        Some(a) => {
            let integer = &a[..integer_length(a)];
            let fraction = &a[integer.len()..];
            match b {
                None => step_integer(integer, true)
                    .unwrap_or_else(|| format!("{integer}{}", midpoint(fraction, None))),
                Some(b) if integer == &b[..integer_length(b)] => {
                    format!("{integer}{}", midpoint(fraction, Some(&b[integer.len()..])))
                }
                Some(b) => step_integer(integer, true)
                    .filter(|key| key.as_str() < b)
                    .unwrap_or_else(|| format!("{integer}{}", midpoint(fraction, None))),
            }
        }
    };
    if key.len() > 128 {
        return Err("This ordering gap is full; reorder the group first.".into());
    }
    Ok(key)
}
/// Colliding concurrent position registers require an atomic rekey of that partition.
pub(super) fn insertion(
    keys: &[&str],
    index: usize,
) -> Result<(String, Vec<(usize, String)>), String> {
    match between(
        index.checked_sub(1).map(|i| keys[i]),
        keys.get(index).copied(),
    ) {
        Ok(key) => Ok((key, Vec::new())),
        Err(_) if keys.len() < 1024 => {
            let key = |n: usize| {
                format!(
                    "c{}{}{}",
                    char::from(DIGITS[n / (62 * 62)]),
                    char::from(DIGITS[(n / 62) % 62]),
                    char::from(DIGITS[n % 62])
                )
            };
            Ok((
                key(index),
                keys.iter()
                    .enumerate()
                    .map(|(at, _)| (at, key(if at < index { at } else { at + 1 })))
                    .collect(),
            ))
        }
        Err(_) => Err("This collision needs more register edits than one action allows.".into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positions_stay_canonical_across_integer_boundaries_and_tight_gaps() {
        for (a, b) in [
            (None, None),
            (None, Some("a0")),
            (Some("Zz"), Some("a0")),
            (Some("azz"), Some("b00")),
            (Some("Yzz"), Some("Z0")),
            (Some("a0"), Some("a0V")),
        ] {
            let key = between(a, b).unwrap();
            assert!(a.is_none_or(|a| a < key.as_str()) && b.is_none_or(|b| key.as_str() < b));
            assert!(st3_schema::arrangements::valid_key(&key));
        }
        let mut high = "a1".to_owned();
        for _ in 0..600 {
            let key = between(Some("a0"), Some(&high)).unwrap();
            assert!("a0" < key.as_str() && key < high && st3_schema::arrangements::valid_key(&key));
            high = key;
        }
    }
}
