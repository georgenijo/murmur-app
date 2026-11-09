//! Strict bounded JSON, duplicate keys rejected before constructing a document.
use crate::section::Problem;
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::fmt;
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Strict, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("number"))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut out = vec![];
                while let Some(v) = a.next_element::<Strict>()? {
                    out.push(v.0)
                }
                Ok(Strict(Value::Array(out)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Strict, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if out.contains_key(&k) {
                        return Err(de::Error::custom("duplicate JSON key"));
                    }
                    out.insert(k, a.next_value::<Strict>()?.0);
                }
                Ok(Strict(Value::Object(out)))
            }
        }
        d.deserialize_any(V)
    }
}
/// Why a document was refused, so file importers can report the real reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Size,
    Syntax,
    Duplicate,
    Depth,
}
pub fn parse(text: &str, max: usize, depth: usize) -> Result<Value, Problem> {
    parse_fault(text, max, depth).map_err(|_| Problem::Invalid)
}
pub fn parse_fault(text: &str, max: usize, depth: usize) -> Result<Value, Fault> {
    if text.len() > max {
        return Err(Fault::Size);
    }
    let mut d = serde_json::Deserializer::from_str(text);
    let value = Strict::deserialize(&mut d)
        .map_err(|e| {
            if e.to_string().contains("duplicate JSON key") {
                Fault::Duplicate
            } else {
                Fault::Syntax
            }
        })?
        .0;
    d.end().map_err(|_| Fault::Syntax)?;
    fn valid(v: &Value, depth: usize) -> bool {
        if depth == 0 {
            return false;
        }
        match v {
            Value::Object(o) => o.values().all(|v| valid(v, depth - 1)),
            Value::Array(a) => a.iter().all(|v| valid(v, depth - 1)),
            _ => true,
        }
    }
    if !valid(&value, depth) {
        return Err(Fault::Depth);
    }
    Ok(value)
}
/// JSONC supports comments and trailing commas only. Strings are copied byte for
/// byte, so URLs, escaped quotes and Unicode cannot become comments or syntax.
pub fn jsonc(text: &str, max: usize) -> Result<Value, Problem> {
    if text.len() > max {
        return Err(Problem::Invalid);
    }
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let mut string = false;
    while i < b.len() {
        if string {
            if b[i] == b'\\' {
                i += 2;
                continue;
            }
            if b[i] == b'"' {
                string = false
            }
            i += 1;
            continue;
        }
        if b[i] == b'"' {
            string = true;
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1
            }
        } else if b[i..].starts_with(b"/*") {
            let start = i;
            i += 2;
            while i + 1 < b.len() && !b[i..].starts_with(b"*/") {
                i += 1
            }
            if i + 1 >= b.len() {
                return Err(Problem::Invalid);
            }
            i += 2;
            for c in &mut out[start..i] {
                if *c != b'\n' {
                    *c = b' ';
                }
            }
        } else {
            i += 1
        }
    }
    if string {
        return Err(Problem::Invalid);
    }
    i = 0;
    string = false;
    while i < out.len() {
        if string {
            if out[i] == b'\\' {
                i += 2;
                continue;
            }
            if out[i] == b'"' {
                string = false
            }
        } else if out[i] == b'"' {
            string = true
        } else if out[i] == b',' {
            let mut j = i + 1;
            while j < out.len() && out[j].is_ascii_whitespace() {
                j += 1
            }
            if j < out.len() && matches!(out[j], b'}' | b']') {
                out[i] = b' ';
            }
        }
        i += 1;
    }
    parse(
        std::str::from_utf8(&out).map_err(|_| Problem::Invalid)?,
        max,
        32,
    )
}
