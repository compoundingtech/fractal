//! Client-only unknown-key filtering using the fields supplied by Serde itself.
//! The daemon continues to deserialize the original document directly.
use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub(super) fn parse(bytes: &str) -> anyhow::Result<(super::Config, Vec<String>)> {
    let value: toml::Value = toml::from_str(bytes)?;
    let mut unknown = Vec::new();
    let config = super::Config::deserialize(ClientValue { value, path: String::new(), unknown: &mut unknown })?;
    unknown.sort();
    unknown.dedup();
    Ok((config, unknown))
}

struct ClientValue<'a> {
    value: toml::Value,
    path: String,
    unknown: &'a mut Vec<String>,
}

impl<'de> Deserializer<'de> for ClientValue<'_> {
    type Error = toml::de::Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.value {
            toml::Value::Table(table) => visitor.visit_map(ClientMap {
                entries: table.into_iter(), value: None, path: self.path, unknown: self.unknown,
            }),
            toml::Value::Array(array) => visitor.visit_seq(ClientSeq {
                entries: array.into_iter(), index: 0, path: self.path, unknown: self.unknown,
            }),
            value => value.into_deserializer().deserialize_any(visitor),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(mut self, _name: &'static str, fields: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
        if let toml::Value::Table(table) = &mut self.value {
            table.retain(|key, _| {
                if fields.contains(&key) { return true; }
                self.unknown.push(child_path(&self.path, key));
                false
            });
        }
        self.deserialize_any(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(self, name: &'static str, variants: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error> {
        self.value.into_deserializer().deserialize_enum(name, variants, visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map identifier ignored_any
    }
}

fn child_path(parent: &str, key: &str) -> String {
    if parent.is_empty() { key.to_owned() } else { format!("{parent}.{key}") }
}

struct ClientMap<'a> {
    entries: <toml::Table as IntoIterator>::IntoIter,
    value: Option<(String, toml::Value)>,
    path: String,
    unknown: &'a mut Vec<String>,
}
impl<'de> MapAccess<'de> for ClientMap<'_> {
    type Error = toml::de::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error> {
        let Some((key, value)) = self.entries.next() else { return Ok(None); };
        let path = child_path(&self.path, &key);
        self.value = Some((path, value));
        seed.deserialize(key.into_deserializer()).map(Some)
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
        let (path, value) = self.value.take().ok_or_else(|| de::Error::custom("config value without key"))?;
        seed.deserialize(ClientValue { value, path, unknown: self.unknown })
    }
}

struct ClientSeq<'a> {
    entries: std::vec::IntoIter<toml::Value>,
    index: usize,
    path: String,
    unknown: &'a mut Vec<String>,
}
impl<'de> SeqAccess<'de> for ClientSeq<'_> {
    type Error = toml::de::Error;
    fn next_element_seed<S: DeserializeSeed<'de>>(&mut self, seed: S) -> Result<Option<S::Value>, Self::Error> {
        let Some(value) = self.entries.next() else { return Ok(None); };
        let path = format!("{}[{}]", self.path, self.index);
        self.index += 1;
        seed.deserialize(ClientValue { value, path, unknown: self.unknown }).map(Some)
    }
}
