// Generated; do not edit. Inputs: root name AgentProvenanceRuntimeV1, agent-provenance-runtime/v1 JSON Schema without $comment, ../schema-registry/rust/Cargo.toml, ../schema-registry/rust/src/lib.rs, ../schema-registry/rust/src/numbers.rs, ../schema-registry/rust/src/runtime.rs, ../schema-registry/rust/src/bin/schema-registry-rust.rs.
// Regenerate: pnpm run generate; sha256=f016a53e373e51ef708be078b74a97bfdd7b61747e85eeb70e49dcf4d01b7ae6
pub mod __schema_registry_runtime {
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error { Invalid(String) }
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self { Self::Invalid(message) => f.write_str(message) }
    }
}
impl std::error::Error for Error {}
/// Preserve omitted-vs-present semantics, including nullable values when the schema permits them.
///
/// # Errors
/// Propagates the inner schema type's deserialization error.
pub fn deserialize_optional<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where D: serde::Deserializer<'de>, T: serde::Deserialize<'de> {
    T::deserialize(deserializer).map(Some)
}
#[must_use]
pub fn utf16_len(value: &str) -> usize { value.encode_utf16().count() }
#[must_use]
pub fn is_js_trimmed(value: &str) -> bool {
    fn whitespace(c: char) -> bool {
        matches!(c, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
    }
    value.chars().next().is_none_or(|c| !whitespace(c))
        && value.chars().next_back().is_none_or(|c| !whitespace(c))
}

// Exact numeric semantics shared by schema admission and emitted runtime validation.
// serde_json's ordinary Number representation has at most 20 integer digits or
// 17 significant float digits. Keep numeric validation exact and allocation-free.
#[derive(Clone, Copy)]
struct SchemaDecimal {
    negative: bool,
    coefficient: u128,
    scale: i32,
}
impl SchemaDecimal {
    fn parse(value: &str) -> Self {
        let negative = value.starts_with('-');
        let value = value.strip_prefix('-').unwrap_or(value);
        let (mantissa, exponent) = value
            .split_once(['e', 'E'])
            .map_or((value, 0), |(mantissa, exponent)| {
                (mantissa, exponent.parse::<i32>().expect("JSON exponent"))
            });
        let fraction = mantissa.split_once('.').map_or(0, |(_, fraction)| {
            i32::try_from(fraction.len()).expect("JSON significand length")
        });
        let coefficient = mantissa
            .bytes()
            .filter(|c| *c != b'.')
            .fold(0_u128, |value, digit| value * 10 + u128::from(digit - b'0'));
        Self {
            negative: negative && coefficient != 0,
            coefficient,
            scale: exponent - fraction,
        }
    }
    fn number(value: &serde_json::Number) -> Self {
        struct Buffer {
            bytes: [u8; 64],
            length: usize,
        }
        impl std::fmt::Write for Buffer {
            fn write_str(&mut self, value: &str) -> std::fmt::Result {
                let end = self.length + value.len();
                self.bytes
                    .get_mut(self.length..end)
                    .ok_or(std::fmt::Error)?
                    .copy_from_slice(value.as_bytes());
                self.length = end;
                Ok(())
            }
        }
        if let Some(integer) = value.as_i64() {
            return Self {
                negative: integer < 0,
                coefficient: u128::from(integer.unsigned_abs()),
                scale: 0,
            };
        }
        if let Some(integer) = value.as_u64() {
            return Self {
                negative: false,
                coefficient: u128::from(integer),
                scale: 0,
            };
        }
        let mut buffer = Buffer {
            bytes: [0; 64],
            length: 0,
        };
        std::fmt::write(&mut buffer, format_args!("{value}"))
            .expect("finite JSON Number fits 64 bytes");
        Self::parse(
            std::str::from_utf8(&buffer.bytes[..buffer.length]).expect("JSON Number is ASCII"),
        )
    }
    fn compare(self, other: Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.coefficient == 0 || other.coefficient == 0 {
            return match (self.coefficient == 0, other.coefficient == 0) {
                (true, true) => Ordering::Equal,
                (true, false) => {
                    if other.negative {
                        Ordering::Greater
                    } else {
                        Ordering::Less
                    }
                }
                (false, true) => {
                    if self.negative {
                        Ordering::Less
                    } else {
                        Ordering::Greater
                    }
                }
                (false, false) => unreachable!(),
            };
        }
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let left_digits = self.coefficient.ilog10() + 1;
        let right_digits = other.coefficient.ilog10() + 1;
        let magnitude = (i64::from(left_digits) + i64::from(self.scale))
            .cmp(&(i64::from(right_digits) + i64::from(other.scale)));
        let ordering = if magnitude == Ordering::Equal {
            let digits = left_digits.max(right_digits);
            (self.coefficient * 10_u128.pow(digits - left_digits))
                .cmp(&(other.coefficient * 10_u128.pow(digits - right_digits)))
        } else {
            magnitude
        };
        if self.negative {
            ordering.reverse()
        } else {
            ordering
        }
    }
}
/// Compare a Number with a canonical, finite JSON Schema numeric bound.
#[must_use]
pub fn compare_number(value: &serde_json::Number, bound: &str) -> std::cmp::Ordering {
    SchemaDecimal::number(value).compare(SchemaDecimal::parse(bound))
}
#[must_use]
pub fn numbers_equal(left: &serde_json::Number, right: &serde_json::Number) -> bool {
    SchemaDecimal::number(left).compare(SchemaDecimal::number(right)) == std::cmp::Ordering::Equal
}
#[must_use]
pub fn is_integer(value: &serde_json::Number) -> bool {
    let decimal = SchemaDecimal::number(value);
    if decimal.coefficient == 0 || decimal.scale >= 0 {
        return true;
    }
    let mut coefficient = decimal.coefficient;
    let mut zeros = 0;
    while coefficient.is_multiple_of(10) {
        coefficient /= 10;
        zeros += 1;
    }
    zeros >= -decimal.scale
}
/// Check an exact decimal multiple against a positive canonical JSON Schema divisor.
#[must_use]
pub fn is_multiple_of(value: &serde_json::Number, divisor: &str) -> bool {
    let numerator_value = SchemaDecimal::number(value);
    let denominator_value = SchemaDecimal::parse(divisor);
    let (mut numerator, numerator_scale) = (numerator_value.coefficient, numerator_value.scale);
    let (mut denominator, denominator_scale) =
        (denominator_value.coefficient, denominator_value.scale);
    if numerator == 0 {
        return true;
    }
    let (mut a, mut b) = (numerator, denominator);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    numerator /= a;
    denominator /= a;
    let scale = numerator_scale - denominator_scale;
    if scale >= 0 {
        for prime in [2, 5] {
            let mut powers = 0;
            while denominator % prime == 0 {
                denominator /= prime;
                powers += 1;
            }
            if powers > scale {
                return false;
            }
        }
        denominator == 1
    } else {
        if denominator != 1 {
            return false;
        }
        let mut zeros = 0;
        while numerator % 10 == 0 {
            numerator /= 10;
            zeros += 1;
        }
        zeros >= -scale
    }
}

}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1Type(String);
impl AgentProvenanceRuntimeV1Type { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if !["agent-provenance-runtime/v1"].contains(&value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1Type: enum".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1Type { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1Type { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhoAgent(String);
impl AgentProvenanceRuntimeV1WhoAgent { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^(?:agent\\/[A-Za-z0-9][A-Za-z0-9._@-]*(?:\\/(?:[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.\\.[A-Za-z0-9._@-]*|\\.))*|(?:[a-z0-9]+(?:-[a-z0-9]+)*\\.)?(?:direct\\.[a-z0-9]+(?:-[a-z0-9]+)*\\.)?(?:[a-z0-9](?:[a-z0-9-]{0,6}[a-z0-9])?|x-(?:2[de]|3[0-9]|4[1-9a-f]|5[0-9af]|6[1-9a-f]|7[0-9a]){1,255}))$").expect("schema pattern"));
static PATTERN_3: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^(?:agent\\/[A-Za-z0-9._@/-]{1,512}|[^/]+)$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoAgent: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoAgent: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoAgent: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoAgent: pattern".into())); }
if !PATTERN_3.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoAgent: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhoAgent { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhoAgent { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhoPrincipal(String);
impl AgentProvenanceRuntimeV1WhoPrincipal { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^person\\/[A-Za-z0-9][A-Za-z0-9._@-]*(?:\\/(?:[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.\\.[A-Za-z0-9._@-]*|\\.))*$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPrincipal: minLength".into())); }
if value.len() > 519 { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPrincipal: x-maxUtf8ByteLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPrincipal: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPrincipal: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPrincipal: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhoPrincipal { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhoPrincipal { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhoActor(String);
impl AgentProvenanceRuntimeV1WhoActor { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoActor: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoActor: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoActor: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhoActor { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhoActor { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhoPersona(String);
impl AgentProvenanceRuntimeV1WhoPersona { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPersona: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPersona: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoPersona: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhoPersona { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhoPersona { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhoSupervisor(String);
impl AgentProvenanceRuntimeV1WhoSupervisor { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^(?:agent\\/[A-Za-z0-9][A-Za-z0-9._@-]*(?:\\/(?:[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.\\.[A-Za-z0-9._@-]*|\\.))*|(?:[a-z0-9]+(?:-[a-z0-9]+)*\\.)?(?:direct\\.[a-z0-9]+(?:-[a-z0-9]+)*\\.)?(?:[a-z0-9](?:[a-z0-9-]{0,6}[a-z0-9])?|x-(?:2[de]|3[0-9]|4[1-9a-f]|5[0-9af]|6[1-9a-f]|7[0-9a]){1,255}))$").expect("schema pattern"));
static PATTERN_3: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^(?:agent\\/[A-Za-z0-9._@/-]{1,512}|[^/]+)$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoSupervisor: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoSupervisor: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoSupervisor: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoSupervisor: pattern".into())); }
if !PATTERN_3.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhoSupervisor: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhoSupervisor { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhoSupervisor { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1Who {
pub r#agent: AgentProvenanceRuntimeV1WhoAgent,
pub r#principal: AgentProvenanceRuntimeV1WhoPrincipal,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#actor: Option<AgentProvenanceRuntimeV1WhoActor>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#persona: Option<AgentProvenanceRuntimeV1WhoPersona>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#supervisor: Option<AgentProvenanceRuntimeV1WhoSupervisor>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant0Tag(String);
impl AgentProvenanceRuntimeV1WhyVariant0Tag { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if !["bound"].contains(&value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0Tag: enum".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant0Tag { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant0Tag { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant0MissionName(String);
impl AgentProvenanceRuntimeV1WhyVariant0MissionName { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionName: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionName: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionName: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant0MissionName { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant0MissionName { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant0MissionRun(String);
impl AgentProvenanceRuntimeV1WhyVariant0MissionRun { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
// LIVE-MIGRATION BRIDGE arn:lmig:dotfiles:2026-10-13-agent-trace-mission-run-shape — DELETE at contraction — https://app.notion.com/p/Agent-Trace-mission-run-shape-3f8e3d41f4a381f6906cefeadeb7526e
let migration_active = true;
if migration_active { return regex::Regex::new("^(?:[0-9a-f]{32}|mission-run\\/[A-Za-z0-9][A-Za-z0-9._@-]*(?:\\/(?:[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.\\.[A-Za-z0-9._@-]*|\\.))*)$").expect("schema pattern"); }
// LIVE-MIGRATION END arn:lmig:dotfiles:2026-10-13-agent-trace-mission-run-shape
regex::Regex::new("^mission-run\\/[A-Za-z0-9][A-Za-z0-9._@-]*(?:\\/(?:[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.[A-Za-z0-9_@-][A-Za-z0-9._@-]*|\\.\\.\\.[A-Za-z0-9._@-]*|\\.))*$").expect("schema pattern")
});
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionRun: minLength".into())); }
if value.len() > 524 { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionRun: x-maxUtf8ByteLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionRun: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionRun: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0MissionRun: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant0MissionRun { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant0MissionRun { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1WhyVariant0Mission {
pub r#name: AgentProvenanceRuntimeV1WhyVariant0MissionName,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#run: Option<AgentProvenanceRuntimeV1WhyVariant0MissionRun>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant0StepName(String);
impl AgentProvenanceRuntimeV1WhyVariant0StepName { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepName: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepName: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepName: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant0StepName { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant0StepName { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant0StepRun(String);
impl AgentProvenanceRuntimeV1WhyVariant0StepRun { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^[A-Za-z0-9][A-Za-z0-9._@-]{0,511}$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepRun: minLength".into())); }
if value.len() > 512 { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepRun: x-maxUtf8ByteLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepRun: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepRun: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant0StepRun: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant0StepRun { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant0StepRun { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1WhyVariant0Step {
pub r#name: AgentProvenanceRuntimeV1WhyVariant0StepName,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#run: Option<AgentProvenanceRuntimeV1WhyVariant0StepRun>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1WhyVariant0 {
pub r#_tag: AgentProvenanceRuntimeV1WhyVariant0Tag,
pub r#mission: AgentProvenanceRuntimeV1WhyVariant0Mission,
pub r#step: AgentProvenanceRuntimeV1WhyVariant0Step,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant1Tag(String);
impl AgentProvenanceRuntimeV1WhyVariant1Tag { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if !["idle"].contains(&value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant1Tag: enum".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant1Tag { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant1Tag { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1WhyVariant1 {
pub r#_tag: AgentProvenanceRuntimeV1WhyVariant1Tag,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhyVariant2Tag(String);
impl AgentProvenanceRuntimeV1WhyVariant2Tag { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if !["ambiguous"].contains(&value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhyVariant2Tag: enum".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhyVariant2Tag { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhyVariant2Tag { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1WhyVariant2 {
pub r#_tag: AgentProvenanceRuntimeV1WhyVariant2Tag,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum AgentProvenanceRuntimeV1Why {
Variant0(AgentProvenanceRuntimeV1WhyVariant0),
Variant1(AgentProvenanceRuntimeV1WhyVariant1),
Variant2(AgentProvenanceRuntimeV1WhyVariant2),
}
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1Why { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
if let Ok(branch) = <AgentProvenanceRuntimeV1WhyVariant0 as serde::Deserialize>::deserialize(&value) { return Ok(Self::Variant0(branch)); }
if let Ok(branch) = <AgentProvenanceRuntimeV1WhyVariant1 as serde::Deserialize>::deserialize(&value) { return Ok(Self::Variant1(branch)); }
if let Ok(branch) = <AgentProvenanceRuntimeV1WhyVariant2 as serde::Deserialize>::deserialize(&value) { return Ok(Self::Variant2(branch)); }
Err(serde::de::Error::custom("AgentProvenanceRuntimeV1Why: no union branch matched")) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowHarnessTool(String);
impl AgentProvenanceRuntimeV1HowHarnessTool { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowHarnessTool: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowHarnessTool: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowHarnessTool: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowHarnessTool { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowHarnessTool { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowHarnessVersion(String);
impl AgentProvenanceRuntimeV1HowHarnessVersion { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowHarnessVersion: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowHarnessVersion: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowHarnessVersion: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowHarnessVersion { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowHarnessVersion { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1HowHarness {
pub r#tool: AgentProvenanceRuntimeV1HowHarnessTool,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#version: Option<AgentProvenanceRuntimeV1HowHarnessVersion>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowModelRoute(String);
impl AgentProvenanceRuntimeV1HowModelRoute { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^[a-z0-9][a-z0-9._-]*\\/[^\0- \u{7f}-\u{a0}\u{ad}\u{600}-\u{605}\u{61c}\u{6dd}\u{70f}\u{890}-\u{891}\u{8e2}\u{1680}\u{180e}\u{2000}-\u{200f}\\u2028-\u{202f}\u{205f}-\u{2064}\u{2066}-\u{206f}\u{3000}\u{feff}\u{fff9}-\u{fffb}\u{110bd}\u{110cd}\u{13430}-\u{1343f}\u{1bca0}-\u{1bca3}\u{1d173}-\u{1d17a}\u{e0001}\u{e0020}-\u{e007f}]+$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelRoute: minLength".into())); }
if self::__schema_registry_runtime::utf16_len(value) > 64 { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelRoute: maxLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelRoute: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelRoute: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelRoute: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowModelRoute { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowModelRoute { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowModelEffort(String);
impl AgentProvenanceRuntimeV1HowModelEffort { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelEffort: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelEffort: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowModelEffort: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowModelEffort { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowModelEffort { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1HowModel {
pub r#route: AgentProvenanceRuntimeV1HowModelRoute,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#effort: Option<AgentProvenanceRuntimeV1HowModelEffort>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowToolingProfile(String);
impl AgentProvenanceRuntimeV1HowToolingProfile { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowToolingProfile: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowToolingProfile: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowToolingProfile: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowToolingProfile { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowToolingProfile { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowRuntimeProfile(String);
impl AgentProvenanceRuntimeV1HowRuntimeProfile { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowRuntimeProfile: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowRuntimeProfile: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowRuntimeProfile: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowRuntimeProfile { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowRuntimeProfile { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowSkillsManifest(String);
impl AgentProvenanceRuntimeV1HowSkillsManifest { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowSkillsManifest: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowSkillsManifest: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowSkillsManifest: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowSkillsManifest { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowSkillsManifest { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowInvocationId(String);
impl AgentProvenanceRuntimeV1HowInvocationId { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowInvocationId: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowInvocationId: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowInvocationId: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowInvocationId { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowInvocationId { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowIssuanceVariant0Tag(String);
impl AgentProvenanceRuntimeV1HowIssuanceVariant0Tag { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if !["st-issued"].contains(&value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant0Tag: enum".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowIssuanceVariant0Tag { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowIssuanceVariant0Tag { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun(String);
impl AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_0: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^step-run\\/[A-Za-z0-9][A-Za-z0-9._@-]{0,511}\\/[^/\t-\\r \u{85}\u{a0}\u{1680}\u{2000}-\u{200a}\\u2028-\\u2029\u{202f}\u{205f}\u{3000}]+(?:\\/[^/\t-\\r \u{85}\u{a0}\u{1680}\u{2000}-\u{200a}\\u2028-\\u2029\u{202f}\u{205f}\u{3000}]+)*$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun: minLength".into())); }
if value.split('/').skip(2).any(|segment| segment.len() > 160) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun: x-maxUtf8SegmentByteLength".into())); }
if !PATTERN_0.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1HowIssuanceVariant0 {
pub r#_tag: AgentProvenanceRuntimeV1HowIssuanceVariant0Tag,
pub r#step_run: AgentProvenanceRuntimeV1HowIssuanceVariant0StepRun,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowIssuanceVariant1Tag(String);
impl AgentProvenanceRuntimeV1HowIssuanceVariant1Tag { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if !["not-st-issued"].contains(&value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant1Tag: enum".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowIssuanceVariant1Tag { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowIssuanceVariant1Tag { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher(String);
impl AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1HowIssuanceVariant1 {
pub r#_tag: AgentProvenanceRuntimeV1HowIssuanceVariant1Tag,
pub r#launcher: AgentProvenanceRuntimeV1HowIssuanceVariant1Launcher,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum AgentProvenanceRuntimeV1HowIssuance {
Variant0(AgentProvenanceRuntimeV1HowIssuanceVariant0),
Variant1(AgentProvenanceRuntimeV1HowIssuanceVariant1),
}
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1HowIssuance { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
if let Ok(branch) = <AgentProvenanceRuntimeV1HowIssuanceVariant0 as serde::Deserialize>::deserialize(&value) { return Ok(Self::Variant0(branch)); }
if let Ok(branch) = <AgentProvenanceRuntimeV1HowIssuanceVariant1 as serde::Deserialize>::deserialize(&value) { return Ok(Self::Variant1(branch)); }
Err(serde::de::Error::custom("AgentProvenanceRuntimeV1HowIssuance: no union branch matched")) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1How {
pub r#harness: AgentProvenanceRuntimeV1HowHarness,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#model: Option<AgentProvenanceRuntimeV1HowModel>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#tooling_profile: Option<AgentProvenanceRuntimeV1HowToolingProfile>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#runtime_profile: Option<AgentProvenanceRuntimeV1HowRuntimeProfile>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#skills_manifest: Option<AgentProvenanceRuntimeV1HowSkillsManifest>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#invocation_id: Option<AgentProvenanceRuntimeV1HowInvocationId>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#issuance: Option<AgentProvenanceRuntimeV1HowIssuance>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhereSession(String);
impl AgentProvenanceRuntimeV1WhereSession { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
static PATTERN_2: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new("^(?:[a-z0-9]+(?:-[a-z0-9]+)*\\.)?(?:[a-z0-9](?:[a-z0-9-]{0,6}[a-z0-9])?|x-(?:2[de]|3[0-9]|4[1-9a-f]|5[0-9af]|6[1-9a-f]|7[0-9a]){1,255})$").expect("schema pattern"));
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereSession: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereSession: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereSession: pattern".into())); }
if !PATTERN_2.is_match(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereSession: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhereSession { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhereSession { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhereHost(String);
impl AgentProvenanceRuntimeV1WhereHost { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereHost: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereHost: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereHost: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhereHost { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhereHost { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct AgentProvenanceRuntimeV1WhereWorktree(String);
impl AgentProvenanceRuntimeV1WhereWorktree { #[must_use] pub fn as_str(&self) -> &str { &self.0 }
pub(crate) fn validate(value: &str) -> Result<(), self::__schema_registry_runtime::Error> {
if value.is_empty() { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereWorktree: minLength".into())); }
if !self::__schema_registry_runtime::is_js_trimmed(value) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereWorktree: pattern".into())); }
if value.is_empty() || value.contains(['\r', '\n']) { return Err(self::__schema_registry_runtime::Error::Invalid("AgentProvenanceRuntimeV1WhereWorktree: pattern".into())); }
Ok(()) } }
impl TryFrom<String> for AgentProvenanceRuntimeV1WhereWorktree { type Error = self::__schema_registry_runtime::Error; fn try_from(value: String) -> Result<Self, Self::Error> { Self::validate(&value)?; Ok(Self(value)) } }
impl<'de> serde::Deserialize<'de> for AgentProvenanceRuntimeV1WhereWorktree { fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> { let value = <String as serde::Deserialize>::deserialize(d)?; Self::try_from(value).map_err(serde::de::Error::custom) } }
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1Where {
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#session: Option<AgentProvenanceRuntimeV1WhereSession>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#host: Option<AgentProvenanceRuntimeV1WhereHost>,
#[serde(default, deserialize_with = "self::__schema_registry_runtime::deserialize_optional", skip_serializing_if = "Option::is_none")]
pub r#worktree: Option<AgentProvenanceRuntimeV1WhereWorktree>,
}
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProvenanceRuntimeV1 {
pub r#type: AgentProvenanceRuntimeV1Type,
pub r#who: AgentProvenanceRuntimeV1Who,
pub r#why: AgentProvenanceRuntimeV1Why,
pub r#how: AgentProvenanceRuntimeV1How,
pub r#where: AgentProvenanceRuntimeV1Where,
}
