//! A generic decoder and encoder for the kiwi binary schema format (an open, MIT-licensed message
//! format with a public specification). A binary schema lists definitions (enums, structs and messages, each
//! with typed fields); a message is then decoded with it into a [`Kv`] tree keyed by field name,
//! and encoded back from one ([`encode`]).
//!
//! Everything here reads untrusted bytes: reads are bounds-checked, nesting is capped, array
//! counts can't exceed the bytes left, and the total number of decoded values is capped.

use std::fmt;

/// Deepest struct/message nesting decoded.
const MAX_DEPTH: usize = 64;
/// Most values decoded from one message (every scalar, array element and field counts): at least
/// [`MIN_VALUES`], and [`VALUES_PER_PACKED_BYTE`] per byte of the file's compressed canvas up to
/// [`MAX_VALUES`], so a large real file decodes while a small one can't expand into a huge tree,
/// however far its chunks inflate. 32-bit targets (wasm32) have a 4 GB address space in all, so
/// their ceiling is lower: past it the file is refused cleanly instead of running out of memory.
const MIN_VALUES: usize = 20_000_000;
/// Value budget per compressed canvas byte. The real files measured in
/// `large_files_get_a_larger_value_budget` hold up to 0.75 values per compressed byte without their
/// single scalar fields; counting those too adds 15–43% on the generated files in
/// `value_budget_tests` and up to 3.2 times on a rectangle carrying only geometry and paint, so a
/// dense file may reach about 2.4 per byte. Four leaves two-thirds more room than that. The
/// trade-off: from 32 MiB of canvas (8 MiB on 32-bit targets) a file gets all of [`MAX_VALUES`],
/// and above the floor it may decode four values for every byte it carries; [`MIN_VALUES`] and
/// [`MAX_VALUES`] still bound every file.
const VALUES_PER_PACKED_BYTE: usize = 4;
#[cfg(not(target_pointer_width = "32"))]
const MAX_VALUES: usize = 128 << 20;
#[cfg(target_pointer_width = "32")]
const MAX_VALUES: usize = 32 << 20;
/// Most definitions in a schema, and fields in one definition.
const MAX_DEFS: usize = 10_000;
const MAX_FIELDS: usize = 10_000;

#[derive(Debug, PartialEq, Eq)]
pub struct KiwiError(pub String);

impl fmt::Display for KiwiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, KiwiError> {
    Err(KiwiError(msg.into()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Enum,
    Struct,
    Message,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Bool,
    Byte,
    Int,
    Uint,
    Float,
    Str,
    Int64,
    Uint64,
    /// Index of a definition in the schema.
    Def(usize),
}

/// Built-in types in their schema order (encoded as `!index`).
const BUILTINS: [Type; 8] = [Type::Bool, Type::Byte, Type::Int, Type::Uint, Type::Float, Type::Str, Type::Int64, Type::Uint64];

#[derive(Clone, Debug)]
pub struct Field {
    pub name: String,
    /// `None` for enum values.
    pub ty: Option<Type>,
    pub array: bool,
    /// The field id (messages) or the value (enums).
    pub value: u32,
}

#[derive(Clone, Debug)]
pub struct Def {
    pub name: String,
    pub kind: Kind,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, Default)]
pub struct Schema {
    pub defs: Vec<Def>,
}

/// A decoded value. Field and enum names borrow from the schema.
#[derive(Clone, Debug, PartialEq)]
pub enum Kv<'s> {
    Bool(bool),
    Int(i64),
    Uint(u64),
    Float(f32),
    Str(String),
    /// A `byte[]` field.
    Bytes(Vec<u8>),
    Enum(&'s str),
    Array(Vec<Kv<'s>>),
    /// A struct or message: its fields in the order read.
    Object(Vec<(&'s str, Kv<'s>)>),
}

impl<'s> Kv<'s> {
    /// Field `name` of an object.
    pub fn get(&self, name: &str) -> Option<&Kv<'s>> {
        match self {
            Kv::Object(f) => f.iter().find(|(n, _)| *n == name).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn f64(&self) -> Option<f64> {
        let v = match self {
            Kv::Float(f) => f64::from(*f),
            Kv::Int(i) => *i as f64,
            Kv::Uint(u) => *u as f64,
            Kv::Bool(b) => f64::from(u8::from(*b)),
            _ => return None,
        };
        v.is_finite().then_some(v)
    }

    pub fn bool(&self) -> Option<bool> {
        match self {
            Kv::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            Kv::Str(s) => Some(s),
            Kv::Enum(s) => Some(s),
            _ => None,
        }
    }

    pub fn list(&self) -> &[Kv<'s>] {
        match self {
            Kv::Array(a) => a,
            _ => &[],
        }
    }

    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Kv::Bytes(b) => Some(b),
            _ => None,
        }
    }
}

/// A bounds-checked reader over kiwi bytes.
struct Reader<'b> {
    data: &'b [u8],
    pos: usize,
}

impl<'b> Reader<'b> {
    fn left(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn byte(&mut self) -> Result<u8, KiwiError> {
        let b = *self.data.get(self.pos).ok_or_else(|| KiwiError("unexpected end of data".into()))?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'b [u8], KiwiError> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.data.len()).ok_or_else(|| KiwiError("unexpected end of data".into()))?;
        let s = self.data.get(self.pos..end).ok_or_else(|| KiwiError("unexpected end of data".into()))?;
        self.pos = end;
        Ok(s)
    }

    fn var_uint(&mut self) -> Result<u32, KiwiError> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            value |= u32::from(b & 127).checked_shl(shift).unwrap_or(0);
            shift += 7;
            if b & 128 == 0 || shift >= 35 {
                return Ok(value);
            }
        }
    }

    fn var_int(&mut self) -> Result<i32, KiwiError> {
        let v = self.var_uint()?;
        Ok(if v & 1 != 0 { !((v >> 1) as i32) } else { (v >> 1) as i32 })
    }

    fn var_uint64(&mut self) -> Result<u64, KiwiError> {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let b = self.byte()?;
            if shift == 56 {
                // The ninth byte carries a full eight bits.
                return Ok(value | (u64::from(b) << 56));
            }
            value |= u64::from(b & 127) << shift;
            shift += 7;
            if b & 128 == 0 {
                return Ok(value);
            }
        }
    }

    fn var_int64(&mut self) -> Result<i64, KiwiError> {
        let v = self.var_uint64()?;
        Ok(if v & 1 != 0 { !((v >> 1) as i64) } else { (v >> 1) as i64 })
    }

    fn var_float(&mut self) -> Result<f32, KiwiError> {
        if self.data.get(self.pos) == Some(&0) {
            self.pos += 1;
            return Ok(0.0);
        }
        let b = self.take(4)?;
        let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        // The exponent is stored first so zero fits one byte; move it back into place.
        Ok(f32::from_bits(bits.rotate_left(23)))
    }

    fn string(&mut self) -> Result<String, KiwiError> {
        let rest = self.data.get(self.pos..).unwrap_or_default();
        let len = rest.iter().position(|b| *b == 0).ok_or_else(|| KiwiError("unterminated string".into()))?;
        let s = String::from_utf8_lossy(self.take(len)?).into_owned();
        self.pos += 1;
        Ok(s)
    }
}

impl Schema {
    /// Decode a binary schema.
    pub fn decode(data: &[u8]) -> Result<Schema, KiwiError> {
        let mut r = Reader { data, pos: 0 };
        let count = r.var_uint()? as usize;
        if count > MAX_DEFS || count > r.left() {
            return err("schema: too many definitions");
        }
        let mut defs = Vec::with_capacity(count);
        let mut raw_types = Vec::new();
        for _ in 0..count {
            let name = r.string()?;
            let kind = match r.byte()? {
                0 => Kind::Enum,
                1 => Kind::Struct,
                2 => Kind::Message,
                k => return err(format!("schema: unknown definition kind {k}")),
            };
            let n = r.var_uint()? as usize;
            if n > MAX_FIELDS || n > r.left() {
                return err("schema: too many fields");
            }
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let fname = r.string()?;
                let ty = r.var_int()?;
                let array = r.byte()? & 1 != 0;
                let value = r.var_uint()?;
                raw_types.push(ty);
                fields.push(Field { name: fname, ty: None, array, value });
            }
            defs.push(Def { name, kind, fields });
        }
        let total = defs.len();
        let mut types = raw_types.into_iter();
        for d in &mut defs {
            for f in &mut d.fields {
                let raw = types.next().unwrap_or(0);
                if d.kind == Kind::Enum {
                    continue;
                }
                f.ty = Some(if raw < 0 {
                    *BUILTINS.get((!raw) as usize).ok_or_else(|| KiwiError(format!("schema: unknown type {raw}")))?
                } else {
                    let i = raw as usize;
                    if i >= total {
                        return err(format!("schema: field {} names missing definition {i}", f.name));
                    }
                    Type::Def(i)
                });
            }
        }
        Ok(Schema { defs })
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.defs.iter().position(|d| d.name == name)
    }

    /// Decode a message of definition `root`, within [`MIN_VALUES`] values.
    pub fn decode_message<'s>(&'s self, root: usize, data: &[u8]) -> Result<Kv<'s>, KiwiError> {
        self.decode_message_within(root, data, MIN_VALUES)
    }

    /// [`Self::decode_message`] within `budget` values (see [`value_budget`]).
    pub fn decode_message_within<'s>(&'s self, root: usize, data: &[u8], budget: usize) -> Result<Kv<'s>, KiwiError> {
        let mut d = Decoder { schema: self, r: Reader { data, pos: 0 }, budget };
        d.def(root, 0)
    }
}

/// Most values decoded from a file whose compressed canvas is `packed` bytes.
pub(crate) fn value_budget(packed: usize) -> usize {
    packed.saturating_mul(VALUES_PER_PACKED_BYTE).clamp(MIN_VALUES, MAX_VALUES)
}

struct Decoder<'s, 'b> {
    schema: &'s Schema,
    r: Reader<'b>,
    budget: usize,
}

impl<'s> Decoder<'s, '_> {
    fn spend(&mut self, n: usize) -> Result<(), KiwiError> {
        self.budget = self.budget.checked_sub(n).ok_or_else(|| KiwiError("message: too many values".into()))?;
        Ok(())
    }

    fn def(&mut self, i: usize, depth: usize) -> Result<Kv<'s>, KiwiError> {
        if depth > MAX_DEPTH {
            return err("message: nested too deeply");
        }
        self.spend(1)?;
        let schema = self.schema;
        let d = schema.defs.get(i).ok_or_else(|| KiwiError("message: missing definition".into()))?;
        match d.kind {
            Kind::Enum => {
                let v = self.r.var_uint()?;
                Ok(d.fields.iter().find(|f| f.value == v).map_or(Kv::Uint(u64::from(v)), |f| Kv::Enum(f.name.as_str())))
            }
            Kind::Struct => {
                let mut out = Vec::with_capacity(d.fields.len());
                for f in &d.fields {
                    out.push((f.name.as_str(), self.field(f, depth)?));
                }
                Ok(Kv::Object(out))
            }
            Kind::Message => {
                let mut out = Vec::new();
                loop {
                    let id = self.r.var_uint()?;
                    if id == 0 {
                        // Grown by doubling: trimmed, a large file's tree takes a good part less.
                        out.shrink_to_fit();
                        return Ok(Kv::Object(out));
                    }
                    let f = d.fields.iter().find(|f| f.value == id).ok_or_else(|| KiwiError(format!("message: {} has no field {id}", d.name)))?;
                    out.push((f.name.as_str(), self.field(f, depth)?));
                }
            }
        }
    }

    fn field(&mut self, f: &Field, depth: usize) -> Result<Kv<'s>, KiwiError> {
        let ty = f.ty.ok_or_else(|| KiwiError("message: untyped field".into()))?;
        if !f.array {
            // A nested definition spends its own value in `def`.
            if !matches!(ty, Type::Def(_)) {
                self.spend(1)?;
            }
            return self.value(ty, depth);
        }
        let n = self.r.var_uint()? as usize;
        // Every element takes at least one byte.
        if n > self.r.left() {
            return err("message: array longer than the data");
        }
        if ty == Type::Byte {
            self.spend(1)?;
            return Ok(Kv::Bytes(self.r.take(n)?.to_vec()));
        }
        self.spend(n)?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.value(ty, depth)?);
        }
        Ok(Kv::Array(out))
    }

    fn value(&mut self, ty: Type, depth: usize) -> Result<Kv<'s>, KiwiError> {
        Ok(match ty {
            Type::Bool => Kv::Bool(self.r.byte()? != 0),
            Type::Byte => Kv::Uint(u64::from(self.r.byte()?)),
            Type::Int => Kv::Int(i64::from(self.r.var_int()?)),
            Type::Uint => Kv::Uint(u64::from(self.r.var_uint()?)),
            Type::Float => Kv::Float(self.r.var_float()?),
            Type::Str => Kv::Str(self.r.string()?),
            Type::Int64 => Kv::Int(self.r.var_int64()?),
            Type::Uint64 => Kv::Uint(self.r.var_uint64()?),
            Type::Def(i) => self.def(i, depth + 1)?,
        })
    }
}

/// An encoder for the same format: the `.fig` export writes its schema and message with it (and
/// test fixtures are built with it). Values are checked against the schema; it never panics.
pub mod encode {
    use super::{BUILTINS, Field, Kind, KiwiError, Kv, MAX_DEPTH, Schema, Type, err};

    #[derive(Default)]
    pub struct Writer(pub Vec<u8>);

    impl Writer {
        pub fn byte(&mut self, b: u8) {
            self.0.push(b);
        }

        pub fn var_uint(&mut self, mut v: u32) {
            loop {
                let b = (v & 127) as u8;
                v >>= 7;
                if v == 0 {
                    self.0.push(b);
                    return;
                }
                self.0.push(b | 128);
            }
        }

        pub fn var_int(&mut self, v: i32) {
            self.var_uint(((v << 1) ^ (v >> 31)) as u32);
        }

        pub fn var_uint64(&mut self, mut v: u64) {
            for _ in 0..8 {
                let b = (v & 127) as u8;
                v >>= 7;
                if v == 0 {
                    self.0.push(b);
                    return;
                }
                self.0.push(b | 128);
            }
            self.0.push(v as u8);
        }

        pub fn var_int64(&mut self, v: i64) {
            self.var_uint64(((v << 1) ^ (v >> 63)) as u64);
        }

        pub fn var_float(&mut self, f: f32) {
            let bits = f.to_bits().rotate_right(23);
            if bits & 255 == 0 {
                self.0.push(0);
            } else {
                self.0.extend_from_slice(&bits.to_le_bytes());
            }
        }

        /// A NUL-terminated string (NULs inside are dropped: they would end it early).
        pub fn string(&mut self, s: &str) {
            self.0.extend(s.bytes().filter(|b| *b != 0));
            self.0.push(0);
        }
    }

    pub fn schema(s: &Schema) -> Vec<u8> {
        let mut w = Writer::default();
        w.var_uint(s.defs.len() as u32);
        for d in &s.defs {
            w.string(&d.name);
            w.byte(match d.kind {
                Kind::Enum => 0,
                Kind::Struct => 1,
                Kind::Message => 2,
            });
            w.var_uint(d.fields.len() as u32);
            for f in &d.fields {
                w.string(&f.name);
                w.var_int(match f.ty {
                    None => 0,
                    Some(Type::Def(i)) => i32::try_from(i).unwrap_or(i32::MAX),
                    Some(t) => !(BUILTINS.iter().position(|b| *b == t).unwrap_or(0) as i32),
                });
                w.byte(u8::from(f.array));
                w.var_uint(f.value);
            }
        }
        w.0
    }

    /// Encode `v` as definition `def` (objects may leave message fields out; struct fields are
    /// required, in order). Fails on what doesn't fit the schema: unknown fields or enum names,
    /// missing struct fields, wrong types, numbers out of range, non-finite floats.
    pub fn message(s: &Schema, def: usize, v: &Kv) -> Result<Vec<u8>, KiwiError> {
        let mut w = Writer::default();
        encode_def(s, def, v, &mut w, 0)?;
        Ok(w.0)
    }

    fn encode_def(s: &Schema, i: usize, v: &Kv, w: &mut Writer, depth: usize) -> Result<(), KiwiError> {
        if depth > MAX_DEPTH {
            return err("encode: nested too deeply");
        }
        let d = s.defs.get(i).ok_or_else(|| KiwiError("encode: missing definition".into()))?;
        match d.kind {
            Kind::Enum => {
                let value = match v {
                    Kv::Uint(u) => u32::try_from(*u).ok(),
                    _ => v.str().and_then(|name| d.fields.iter().find(|f| f.name == name)).map(|f| f.value),
                };
                w.var_uint(value.ok_or_else(|| KiwiError(format!("encode: {v:?} is not a value of {}", d.name)))?);
            }
            Kind::Struct => {
                for f in &d.fields {
                    let fv = v.get(&f.name).ok_or_else(|| KiwiError(format!("encode: {} needs field {}", d.name, f.name)))?;
                    encode_field(s, f, fv, w, depth)?;
                }
            }
            Kind::Message => {
                let Kv::Object(fields) = v else { return err(format!("encode: {} needs an object", d.name)) };
                for (name, fv) in fields {
                    let f = d.fields.iter().find(|f| f.name == *name).ok_or_else(|| KiwiError(format!("encode: {} has no field {name}", d.name)))?;
                    w.var_uint(f.value);
                    encode_field(s, f, fv, w, depth)?;
                }
                w.var_uint(0);
            }
        }
        Ok(())
    }

    fn encode_field(s: &Schema, f: &Field, v: &Kv, w: &mut Writer, depth: usize) -> Result<(), KiwiError> {
        let ty = f.ty.ok_or_else(|| KiwiError("encode: untyped field".into()))?;
        if !f.array {
            return encode_value(s, ty, v, w, depth);
        }
        let len = |n: usize| u32::try_from(n).map_err(|_| KiwiError(format!("encode: {} is too long", f.name)));
        if let (Type::Byte, Kv::Bytes(b)) = (ty, v) {
            w.var_uint(len(b.len())?);
            w.0.extend_from_slice(b);
            return Ok(());
        }
        let Kv::Array(items) = v else { return err(format!("encode: {} needs a list", f.name)) };
        w.var_uint(len(items.len())?);
        for it in items {
            encode_value(s, ty, it, w, depth)?;
        }
        Ok(())
    }

    /// An integer value (whole finite floats count).
    fn int(v: &Kv) -> Option<i128> {
        match v {
            Kv::Int(i) => Some(i128::from(*i)),
            Kv::Uint(u) => Some(i128::from(*u)),
            Kv::Bool(b) => Some(i128::from(*b)),
            Kv::Float(f) if f.is_finite() && f.fract() == 0.0 => Some(*f as i128),
            _ => None,
        }
    }

    fn encode_value(s: &Schema, ty: Type, v: &Kv, w: &mut Writer, depth: usize) -> Result<(), KiwiError> {
        let bad = || KiwiError(format!("encode: {v:?} is not a {ty:?}"));
        let ranged = |lo: i128, hi: i128| int(v).filter(|i| (lo..=hi).contains(i)).ok_or_else(bad);
        match ty {
            Type::Bool => w.byte(u8::from(v.bool().ok_or_else(bad)?)),
            Type::Byte => w.byte(ranged(0, 255)? as u8),
            Type::Int => w.var_int(ranged(i32::MIN.into(), i32::MAX.into())? as i32),
            Type::Uint => w.var_uint(ranged(0, u32::MAX.into())? as u32),
            // Infinity is a value of its own (an axis with no maximum size); NaN is not.
            Type::Float => w.var_float(match v {
                Kv::Float(f) if f.is_infinite() => *f,
                _ => v.f64().map(|x| x as f32).filter(|x| x.is_finite()).ok_or_else(bad)?,
            }),
            Type::Str => w.string(v.str().ok_or_else(bad)?),
            Type::Int64 => w.var_int64(ranged(i64::MIN.into(), i64::MAX.into())? as i64),
            Type::Uint64 => w.var_uint64(ranged(0, u64::MAX.into())? as u64),
            Type::Def(i) => encode_def(s, i, v, w, depth + 1)?,
        }
        Ok(())
    }
}

/// The built-in type names, in [`BUILTINS`] order.
const BUILTIN_NAMES: [&str; 8] = ["bool", "byte", "int", "uint", "float", "string", "int64", "uint64"];

impl Schema {
    /// A schema from a compact text form, one definition per line: `enum Name A B …` (values from
    /// 0), `struct Name type field, type field[] …` or `message Name …` (field ids from 1), the
    /// types being built-in names or other definitions'. `A=3` or `type field=7` gives a value or
    /// a field its own number.
    pub fn parse(text: &str) -> Result<Schema, KiwiError> {
        let lines: Vec<Vec<&str>> = text.lines().map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|l| !l.is_empty()).collect();
        let index = |name: &str| lines.iter().position(|l| l.get(1) == Some(&name));
        let mut defs = Vec::with_capacity(lines.len());
        for l in &lines {
            let (Some(kind), Some(name)) = (l.first(), l.get(1)) else { return err("schema text: a definition needs a kind and a name") };
            let rest = l.get(2..).unwrap_or_default();
            let (kind, fields) = match *kind {
                "enum" => {
                    let mut fields = Vec::new();
                    for (n, i) in rest.iter().zip(0..) {
                        let (name, value) = numbered(n, i)?;
                        fields.push(Field { name, ty: None, array: false, value });
                    }
                    (Kind::Enum, fields)
                }
                "struct" | "message" => {
                    let mut fields = Vec::new();
                    for (f, id) in rest.join(" ").split(',').zip(1..) {
                        let Some((t, n)) = f.trim().split_once(' ') else { return err(format!("schema text: bad field `{f}` in {name}")) };
                        let base = t.trim_end_matches("[]");
                        let ty = match BUILTIN_NAMES.iter().position(|b| *b == base) {
                            Some(i) => BUILTINS.get(i).copied(),
                            None => index(base).map(Type::Def),
                        }
                        .ok_or_else(|| KiwiError(format!("schema text: unknown type {base}")))?;
                        let (name, value) = numbered(n.trim(), id)?;
                        fields.push(Field { name, ty: Some(ty), array: t.ends_with("[]"), value });
                    }
                    (if *kind == "struct" { Kind::Struct } else { Kind::Message }, fields)
                }
                k => return err(format!("schema text: unknown kind {k}")),
            };
            defs.push(Def { name: name.to_string(), kind, fields });
        }
        Ok(Schema { defs })
    }
}

/// `name=number` as its parts, or `name` with `default`.
fn numbered(token: &str, default: u32) -> Result<(String, u32), KiwiError> {
    match token.split_once('=') {
        None => Ok((token.to_string(), default)),
        Some((name, n)) => Ok((name.to_string(), n.parse().map_err(|_| KiwiError(format!("schema text: bad number in `{token}`")))?)),
    }
}
