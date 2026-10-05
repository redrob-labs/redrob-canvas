//! Read an internally tagged enum's tag without serialising the rest of the value.
//!
//! History labels come from a command's own `#[serde(tag = "type")]`, so a label can never drift from
//! what the command surface accepts and a new command is named the moment it exists. Serialising the
//! whole command to a `serde_json::Value` would also work, but a command can carry a whole imported
//! document or a long stroke; this serializer stops at the first field. serde writes the tag of an
//! internally tagged enum FIRST, through `serialize_struct` (or `serialize_map` for a map-like
//! payload), so stopping there is enough.

use serde::ser::{self, Impossible, Serialize};
use std::fmt;

/// The tag value under `field`, or `None` when the value is not an internally tagged enum.
pub(crate) fn serde_tag<T: Serialize + ?Sized>(value: &T, field: &str) -> Option<String> {
    match value.serialize(TagSerializer { field }) {
        Err(Stop::Found(tag)) => Some(tag),
        _ => None,
    }
}

#[derive(Debug)]
enum Stop {
    Found(String),
    NotTagged,
}

impl fmt::Display for Stop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("serde_tag stop")
    }
}

impl std::error::Error for Stop {}

impl ser::Error for Stop {
    fn custom<M: fmt::Display>(_msg: M) -> Self {
        Stop::NotTagged
    }
}

struct TagSerializer<'a> {
    field: &'a str,
}

/// Captures a `&str`-like value and nothing else.
struct StrCapture;

macro_rules! not_tagged {
    ($($name:ident($($arg:ty),*)),* $(,)?) => {
        $(fn $name(self, $(_: $arg),*) -> Result<Self::Ok, Self::Error> { Err(Stop::NotTagged) })*
    };
}

impl ser::Serializer for StrCapture {
    type Ok = ();
    type Error = Stop;
    type SerializeSeq = Impossible<(), Stop>;
    type SerializeTuple = Impossible<(), Stop>;
    type SerializeTupleStruct = Impossible<(), Stop>;
    type SerializeTupleVariant = Impossible<(), Stop>;
    type SerializeMap = Impossible<(), Stop>;
    type SerializeStruct = Impossible<(), Stop>;
    type SerializeStructVariant = Impossible<(), Stop>;

    fn serialize_str(self, v: &str) -> Result<(), Stop> {
        Err(Stop::Found(v.to_string()))
    }
    not_tagged!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char),
        serialize_bytes(&[u8]),
        serialize_none(),
        serialize_unit(),
        serialize_unit_struct(&'static str),
        serialize_unit_variant(&'static str, u32, &'static str),
    );
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Stop> {
        Err(Stop::NotTagged)
    }
}

/// The first field of a struct or map: the tag when its key matches, otherwise not tagged.
struct FirstField<'a> {
    field: &'a str,
}

impl ser::SerializeStruct for FirstField<'_> {
    type Ok = ();
    type Error = Stop;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Stop> {
        if key == self.field {
            value.serialize(StrCapture)
        } else {
            Err(Stop::NotTagged)
        }
    }
    fn end(self) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
}

impl ser::SerializeMap for FirstField<'_> {
    type Ok = ();
    type Error = Stop;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Stop> {
        match key.serialize(StrCapture) {
            Err(Stop::Found(k)) if k == self.field => Ok(()),
            _ => Err(Stop::NotTagged),
        }
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Stop> {
        value.serialize(StrCapture)
    }
    fn end(self) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
}

impl<'a> ser::Serializer for TagSerializer<'a> {
    type Ok = ();
    type Error = Stop;
    type SerializeSeq = Impossible<(), Stop>;
    type SerializeTuple = Impossible<(), Stop>;
    type SerializeTupleStruct = Impossible<(), Stop>;
    type SerializeTupleVariant = Impossible<(), Stop>;
    type SerializeMap = FirstField<'a>;
    type SerializeStruct = FirstField<'a>;
    type SerializeStructVariant = Impossible<(), Stop>;

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Stop> {
        Ok(FirstField { field: self.field })
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, Stop> {
        Ok(FirstField { field: self.field })
    }
    not_tagged!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_f32(f32),
        serialize_f64(f64),
        serialize_char(char),
        serialize_str(&str),
        serialize_bytes(&[u8]),
        serialize_none(),
        serialize_unit(),
        serialize_unit_struct(&'static str),
        serialize_unit_variant(&'static str, u32, &'static str),
    );
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Stop> {
        Err(Stop::NotTagged)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Stop> {
        Err(Stop::NotTagged)
    }
}

#[cfg(test)]
mod tests {
    use super::serde_tag;
    use serde::Serialize;

    #[derive(Serialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Sample {
        Unit,
        Fields { a: u32, payload: Vec<u8> },
        Wrapped(Inner),
    }

    #[derive(Serialize)]
    struct Inner {
        x: f64,
    }

    #[test]
    fn reads_the_tag_of_every_variant_shape() {
        assert_eq!(serde_tag(&Sample::Unit, "type").as_deref(), Some("unit"));
        let fields = Sample::Fields {
            a: 1,
            payload: vec![0; 4096],
        };
        assert_eq!(serde_tag(&fields, "type").as_deref(), Some("fields"));
        assert_eq!(
            serde_tag(&Sample::Wrapped(Inner { x: 1.0 }), "type").as_deref(),
            Some("wrapped")
        );
    }

    #[test]
    fn is_none_for_untagged_values_and_other_field_names() {
        assert_eq!(serde_tag(&Sample::Unit, "kind"), None);
        assert_eq!(serde_tag(&Inner { x: 1.0 }, "type"), None);
        assert_eq!(serde_tag(&3_u32, "type"), None);
    }
}
