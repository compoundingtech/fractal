//! Inactive host-envelope declaration. Reading this file never changes admission or cgroups.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fmt,
    num::{NonZeroU16, NonZeroU64},
    path::Path,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "coverage", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Coverage<T> {
    Enforced { value: T },
    NotEnforced { reason: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform {
    LinuxSystemd,
    DarwinAdvisory,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AverageWindow {
    Avg10,
    Avg60,
    Avg300,
}

/// Original SIZE spelling, validated by the same parser used by runtime lanes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct Size(String);
impl TryFrom<String> for Size {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        super::lane::parse_memory(&value)?;
        Ok(Self(value))
    }
}
impl From<Size> for String {
    fn from(value: Size) -> Self {
        value.0
    }
}
impl Size {
    pub fn bytes(&self) -> u64 {
        // Construction and deserialization both pass through the shared SIZE parser.
        super::lane::parse_memory(&self.0).expect("validated SIZE")
    }
}

// Keep field names, requiredness and unknown-field policy shared with the JSON Schema export.
trait Schema {
    fn schema() -> Value;
}
macro_rules! object {
    ($name:ident { $($field:ident : $ty:ty => $key:literal),* $(,)? }) => {
        #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $(#[serde(rename = $key)] pub $field: $ty),* }
        impl Schema for $name {
            fn schema() -> Value {
                json!({"type":"object", "additionalProperties":false,
                    "required":[$($key),*], "properties":{$($key:<$ty>::schema()),*}})
            }
        }
    };
}
object!(PsiThreshold {
    window: AverageWindow => "window",
    percent: u8 => "percent",
    target_slice: String => "target_slice",
});
object!(SlicePsi {
    memory: Coverage<PsiThreshold> => "memory",
    cpu: Coverage<PsiThreshold> => "cpu",
});
object!(CpuMax {
    quota_micros: NonZeroU64 => "quota_micros",
    period_micros: NonZeroU64 => "period_micros",
});
object!(Cpu {
    max: Coverage<CpuMax> => "max",
    weight: Coverage<NonZeroU16> => "weight",
});
object!(Io { weight: NonZeroU16 => "weight" });
object!(Memory {
    max: Coverage<Size> => "max",
    high: Coverage<Size> => "high",
});
object!(Swap { max: Coverage<Size> => "max" });
object!(Tasks { max: Coverage<NonZeroU64> => "max" });

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Slice {
    pub name: String,
    pub memory: Memory,
    pub cpu: Cpu,
    pub io: Coverage<Io>,
    pub swap: Swap,
    pub tasks: Tasks,
}
object!(HostEnvelope {
    schema_version: u64 => "schema_version",
    host: String => "host",
    platform: Platform => "platform",
    mem_available_floor: Coverage<Size> => "mem_available_floor",
    slice_psi: SlicePsi => "slice_psi",
    slices: Vec<Slice> => "slices",
});

#[derive(Debug)]
pub enum Error {
    UnsupportedVersion(u64),
    HostMismatch { node: String, host: String },
    Invalid(String),
    Json(serde_json::Error),
    Io(std::io::Error),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(
                f,
                "host envelope schema version {version} is unsupported; refusing declaration"
            ),
            Self::HostMismatch { node, host } => write!(
                f,
                "host envelope names host {host:?}, but configured node is {node:?}; refusing declaration"
            ),
            Self::Invalid(reason) => write!(f, "invalid host envelope: {reason}"),
            Self::Json(error) => write!(f, "invalid host envelope JSON: {error}"),
            Self::Io(error) => write!(f, "read host envelope: {error}"),
        }
    }
}
impl std::error::Error for Error {}

impl HostEnvelope {
    /// Version is inspected before decoding version-specific fields: never adopt an unknown schema.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let value: Value = serde_json::from_slice(bytes).map_err(Error::Json)?;
        if let Some(version) = value.get("schema_version").and_then(Value::as_u64)
            && version != 1
        {
            return Err(Error::UnsupportedVersion(version));
        }
        let envelope: Self = serde_json::from_value(value).map_err(Error::Json)?;
        envelope.validate()?;
        Ok(envelope)
    }
    pub fn read(path: &Path) -> Result<Self, Error> {
        Self::parse(&std::fs::read(path).map_err(Error::Io)?)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.schema_version != 1 {
            return Err(Error::UnsupportedVersion(self.schema_version));
        }
        let invalid = |reason: &str| Error::Invalid(reason.to_owned());
        if self.host.trim().is_empty() {
            return Err(invalid("host must not be empty"));
        }
        let check = |enforced: bool, reason: Option<&str>| {
            if self.platform == Platform::DarwinAdvisory && enforced {
                return Err(invalid(
                    "darwin-advisory requires every resource to be not-enforced",
                ));
            }
            if reason.is_some_and(|reason| reason.trim().is_empty()) {
                return Err(invalid("not-enforced requires a nonempty reason"));
            }
            Ok(())
        };
        check(
            self.mem_available_floor.enforced(),
            self.mem_available_floor.reason(),
        )?;
        for psi in [&self.slice_psi.memory, &self.slice_psi.cpu] {
            check(psi.enforced(), psi.reason())?;
            if let Coverage::Enforced { value } = psi {
                if value.percent > 100 {
                    return Err(invalid("PSI percent must be 0..100"));
                }
                if !self
                    .slices
                    .iter()
                    .any(|slice| slice.name == value.target_slice)
                {
                    return Err(invalid("PSI target_slice must reference a declared slice"));
                }
            }
        }
        for (index, slice) in self.slices.iter().enumerate() {
            if !slice.name.ends_with(".slice")
                || slice.name.contains('/')
                || slice.name.trim() != slice.name
            {
                return Err(invalid(
                    "slice name must be a user-manager .slice unit name",
                ));
            }
            if self.slices[..index]
                .iter()
                .any(|other| other.name == slice.name)
            {
                return Err(invalid("duplicate slice name"));
            }
            check(slice.memory.max.enforced(), slice.memory.max.reason())?;
            check(slice.memory.high.enforced(), slice.memory.high.reason())?;
            if let (Coverage::Enforced { value: high }, Coverage::Enforced { value: max }) =
                (&slice.memory.high, &slice.memory.max)
                && high.bytes() > max.bytes()
            {
                return Err(invalid("memory.high must not exceed memory.max"));
            }
            check(slice.cpu.max.enforced(), slice.cpu.max.reason())?;
            check(slice.cpu.weight.enforced(), slice.cpu.weight.reason())?;
            check(slice.swap.max.enforced(), slice.swap.max.reason())?;
            check(slice.tasks.max.enforced(), slice.tasks.max.reason())?;
            if let Coverage::Enforced { value: weight } = &slice.cpu.weight
                && weight.get() > 10_000
            {
                return Err(invalid("CPU weight must be 1..10000"));
            }
            check(slice.io.enforced(), slice.io.reason())?;
            if let Coverage::Enforced { value } = &slice.io
                && value.weight.get() > 10_000
            {
                return Err(invalid("IO weight must be 1..10000"));
            }
        }
        Ok(())
    }
}
impl<T> Coverage<T> {
    pub fn enforced(&self) -> bool {
        matches!(self, Self::Enforced { .. })
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::NotEnforced { reason } => Some(reason),
            Self::Enforced { .. } => None,
        }
    }
}

impl Schema for String {
    fn schema() -> Value {
        json!({"type":"string"})
    }
}
impl Schema for u64 {
    fn schema() -> Value {
        json!({"type":"integer", "minimum":0, "maximum":u64::MAX})
    }
}
impl Schema for u8 {
    fn schema() -> Value {
        json!({"type":"integer", "minimum":0, "maximum":100})
    }
}
impl Schema for NonZeroU64 {
    fn schema() -> Value {
        json!({"type":"integer", "minimum":1, "maximum":u64::MAX})
    }
}
impl Schema for NonZeroU16 {
    fn schema() -> Value {
        json!({"type":"integer", "minimum":1, "maximum":10000})
    }
}
impl Schema for Size {
    fn schema() -> Value {
        json!({"type":"string", "pattern":"^[+]?[0-9]+[KkMmGgTt]?$", "description":"Binary SIZE parsed by lane::parse_memory; must fit u64 bytes."})
    }
}
impl Schema for Platform {
    fn schema() -> Value {
        json!({"enum":["linux-systemd","darwin-advisory"]})
    }
}
impl Schema for AverageWindow {
    fn schema() -> Value {
        json!({"enum":["avg10","avg60","avg300"]})
    }
}
impl<T: Schema> Schema for Vec<T> {
    fn schema() -> Value {
        json!({"type":"array", "items":T::schema()})
    }
}
impl<T: Schema> Schema for Coverage<T> {
    fn schema() -> Value {
        json!({"oneOf":[
            {"type":"object","additionalProperties":false,"required":["coverage","value"],
                "properties":{"coverage":{"const":"enforced"},"value":T::schema()}},
            {"type":"object","additionalProperties":false,"required":["coverage","reason"],
                "properties":{"coverage":{"const":"not-enforced"},"reason":{"type":"string","pattern":"\\S"}}}
        ]})
    }
}
impl Schema for Slice {
    fn schema() -> Value {
        json!({"type":"object","additionalProperties":false,
            "required":["name","memory","cpu","io","swap","tasks"],
            "properties":{"name":{"type":"string","pattern":"^[^/\\s]+\\.slice$"},
                "memory":Memory::schema(),"cpu":Cpu::schema(),"io":Coverage::<Io>::schema(),
                "swap":Swap::schema(),"tasks":Tasks::schema()}})
    }
}
/// Export generated from the same field declarations as the serde structures.
/// Dynamic cross-references and SIZE overflow are additionally checked by `validate`/the SIZE parser.
pub fn json_schema() -> Value {
    let mut schema = HostEnvelope::schema();
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["title"] = json!("Inactive Fractal host envelope v1");
    schema["properties"]["schema_version"] = json!({"const":1});
    schema["properties"]["host"] = json!({"type":"string","pattern":"\\S"});
    let advisory = json!({"properties":{"coverage":{"const":"not-enforced"}}});
    schema["allOf"] = json!([{"if":{"properties":{"platform":{"const":"darwin-advisory"}}},
    "then":{"properties":{
        "mem_available_floor":advisory,
        "slice_psi":{"properties":{"memory":advisory,"cpu":advisory}},
        "slices":{"items":{"properties":{
            "memory":{"properties":{"max":advisory,"high":advisory}},
            "cpu":{"properties":{"max":advisory,"weight":advisory}},"io":advisory,
            "swap":{"properties":{"max":advisory}},"tasks":{"properties":{"max":advisory}}
        }}}
    }}}]);
    schema
}
