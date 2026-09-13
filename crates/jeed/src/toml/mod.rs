//! A TOML reader for two files.
//!
//! ```text
//! [table]            [[array.of.tables]]        key = "string"
//! key = 123          key = true                 key = [1, 2, 3]
//! key = { a = 1, b = "two" }                    "quoted key" = 'literal'
//! ```
//!
//! ## What this is not
//!
//! Not a TOML implementation. It reads the subset `conf/krx.toml` and
//! `conf/krx_trcodes.toml` are written in — tables, arrays of tables, inline
//! tables, dotted keys, basic and literal strings, integers, floats, booleans,
//! arrays with comments and newlines inside them — and refuses the rest
//! (multi-line strings, dates and times) by name rather than by misreading it.
//!
//! The reason to write it rather than take `toml` is the dependency, not the
//! grammar: `toml` brings `serde` and its derive machinery, a dozen packages
//! for a file that is parsed once at start-up. The workspace has one external
//! dependency and it is `rustls` (`documents/todo.md` §2).
//!
//! ## Shape of the result
//!
//! A [`Table`] is an ordered list of `(key, Value)` pairs. Order is kept
//! because the conf's `[[feed]]` order is the order feeds are started in and
//! reported in, and because error messages that name "the second feed" should
//! mean the second one in the file.

use core::fmt;

/// A TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A basic or literal string.
    String(String),
    /// A decimal, hex, octal or binary integer.
    Integer(i64),
    /// A float. Accepted for completeness; nothing in `conf/` uses one.
    Float(f64),
    /// `true` or `false`.
    Boolean(bool),
    /// `[…]`, or the accumulation of `[[…]]` headers.
    Array(Vec<Value>),
    /// `[…]` header, `{…}` inline, or the implicit parent of a dotted key.
    Table(Table),
}

impl Value {
    /// The string, if this is one.
    #[inline]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// The integer, if this is one.
    #[inline]
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// The float, if this is one. An integer is **not** promoted: a conf that
    /// says `1` where `1.0` was meant should say so.
    #[inline]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(x) => Some(*x),
            _ => None,
        }
    }

    /// The boolean, if this is one.
    #[inline]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// The array, if this is one.
    #[inline]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The table, if this is one.
    #[inline]
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Self::Table(t) => Some(t),
            _ => None,
        }
    }

    /// What kind of value this is, for error messages.
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Integer(_) => "integer",
            Self::Float(_) => "float",
            Self::Boolean(_) => "boolean",
            Self::Array(_) => "array",
            Self::Table(_) => "table",
        }
    }
}

/// An ordered map from key to [`Value`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Table {
    entries: Vec<(String, Value)>,
    /// `true` once a `[header]` or an inline `{…}` has named this table
    /// explicitly. A table that exists only because a dotted key or a deeper
    /// header passed through it may still be defined by a later header; one
    /// defined twice is an error.
    defined: bool,
}

impl Table {
    /// An empty table.
    pub const fn new() -> Self {
        Self { entries: Vec::new(), defined: false }
    }

    /// The value under `key`, if any.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `true` if `key` is present.
    #[inline]
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The entries, in file order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// The keys, in file order.
    #[inline]
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    /// Number of entries.
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if there are no entries.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Inserts a new key. The caller has checked it is absent.
    fn push(&mut self, key: String, value: Value) -> &mut Value {
        self.entries.push((key, value));
        &mut self.entries.last_mut().expect("just pushed").1
    }
}

/// Where in the file a [`Error`] was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// One-based line number.
    pub line: usize,

    /// What went wrong.
    pub kind: ErrorKind,
}

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// Something else was expected here.
    Expected(&'static str),

    /// A string or a header ran to the end of the file.
    Unterminated(&'static str),

    /// An escape sequence in a basic string was not one TOML defines.
    BadEscape,

    /// A number could not be read as one.
    BadNumber(String),

    /// A key was assigned twice, or a header opened the same table twice.
    Duplicate(String),

    /// A key was used as a table, or as an array of tables, and is neither.
    NotATable(String),

    /// Valid TOML this reader does not implement.
    Unsupported(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: ", self.line)?;
        match &self.kind {
            ErrorKind::Expected(what) => write!(f, "expected {what}"),
            ErrorKind::Unterminated(what) => write!(f, "unterminated {what}"),
            ErrorKind::BadEscape => write!(f, "unknown escape sequence"),
            ErrorKind::BadNumber(s) => write!(f, "not a number: `{s}`"),
            ErrorKind::Duplicate(k) => write!(f, "`{k}` is defined twice"),
            ErrorKind::NotATable(k) => write!(f, "`{k}` is not a table"),
            ErrorKind::Unsupported(what) => write!(f, "{what} is not supported by this reader"),
        }
    }
}

impl std::error::Error for Error {}

/// Reads a whole document.
pub fn parse(text: &str) -> Result<Table, Error> {
    let mut p = Parser { src: text.as_bytes(), pos: 0, line: 1 };
    let mut root = Table::new();
    // The header currently in effect, as a key path from the root. Empty is
    // the root itself. Re-resolved on every assignment: the file is small and
    // read once, and holding a `&mut` into the tree across the loop is not
    // worth the borrow gymnastics.
    let mut current: Vec<String> = Vec::new();

    loop {
        p.skip_blank();
        let Some(b) = p.peek() else { break };

        if b == b'[' {
            let line = p.line;
            p.bump();
            let is_array = p.peek() == Some(b'[');
            if is_array {
                p.bump();
            }
            p.skip_ws();
            let path = p.key_path()?;
            p.skip_ws();
            p.expect(b']', "`]` after table name")?;
            if is_array {
                p.expect(b']', "`]]` after array-of-tables name")?;
            }
            p.expect_eol()?;

            if is_array {
                open_array_table(&mut root, &path, line)?;
            } else {
                open_table(&mut root, &path, line)?;
            }
            current = path;
            continue;
        }

        let line = p.line;
        let path = p.key_path()?;
        p.skip_ws();
        p.expect(b'=', "`=` after key")?;
        p.skip_ws();
        let value = p.value()?;
        p.expect_eol()?;

        let table = table_at(&mut root, &current, line)?;
        assign(table, &path, value, line)?;
    }

    Ok(root)
}

/// Walks `path` from `root`, creating implicit tables on the way and stepping
/// into the **last** element of any array of tables it crosses — which is what
/// a header or key under `[[feed]]` refers to.
fn table_at<'t>(root: &'t mut Table, path: &[String], line: usize) -> Result<&'t mut Table, Error> {
    let mut t = root;
    for key in path {
        if !t.contains(key) {
            t.push(key.clone(), Value::Table(Table::new()));
        }
        t = match t.get_mut(key).expect("present") {
            Value::Table(inner) => inner,
            Value::Array(items) => match items.last_mut() {
                Some(Value::Table(inner)) => inner,
                _ => return Err(Error { line, kind: ErrorKind::NotATable(key.clone()) }),
            },
            _ => return Err(Error { line, kind: ErrorKind::NotATable(key.clone()) }),
        };
    }
    Ok(t)
}

/// `[a.b.c]`.
fn open_table(root: &mut Table, path: &[String], line: usize) -> Result<(), Error> {
    let (last, prefix) = path.split_last().expect("a header names at least one key");
    let parent = table_at(root, prefix, line)?;
    match parent.get_mut(last) {
        None => {
            let mut t = Table::new();
            t.defined = true;
            parent.push(last.clone(), Value::Table(t));
            Ok(())
        }
        Some(Value::Table(t)) if !t.defined => {
            t.defined = true;
            Ok(())
        }
        Some(Value::Table(_)) => Err(Error { line, kind: ErrorKind::Duplicate(path.join(".")) }),
        Some(_) => Err(Error { line, kind: ErrorKind::NotATable(path.join(".")) }),
    }
}

/// `[[a.b.c]]`.
fn open_array_table(root: &mut Table, path: &[String], line: usize) -> Result<(), Error> {
    let (last, prefix) = path.split_last().expect("a header names at least one key");
    let parent = table_at(root, prefix, line)?;
    let mut t = Table::new();
    t.defined = true;
    match parent.get_mut(last) {
        None => {
            parent.push(last.clone(), Value::Array(vec![Value::Table(t)]));
            Ok(())
        }
        Some(Value::Array(items)) if items.iter().all(|v| matches!(v, Value::Table(_))) => {
            items.push(Value::Table(t));
            Ok(())
        }
        Some(_) => Err(Error { line, kind: ErrorKind::NotATable(path.join(".")) }),
    }
}

/// `a.b.c = value` inside `table`.
fn assign(table: &mut Table, path: &[String], value: Value, line: usize) -> Result<(), Error> {
    let (last, prefix) = path.split_last().expect("a key path has at least one key");
    let parent = table_at(table, prefix, line)?;
    if parent.contains(last) {
        return Err(Error { line, kind: ErrorKind::Duplicate(path.join(".")) });
    }
    parent.push(last.clone(), value);
    Ok(())
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    line: usize,
}

impl Parser<'_> {
    #[inline]
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    #[inline]
    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.pos + ahead).copied()
    }

    #[inline]
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn err(&self, kind: ErrorKind) -> Error {
        Error { line: self.line, kind }
    }

    fn expect(&mut self, b: u8, what: &'static str) -> Result<(), Error> {
        if self.peek() == Some(b) {
            self.bump();
            Ok(())
        } else {
            Err(self.err(ErrorKind::Expected(what)))
        }
    }

    /// Spaces and tabs.
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t')) {
            self.bump();
        }
    }

    /// A `# …` comment up to, not including, the newline.
    fn skip_comment(&mut self) {
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.bump();
            }
        }
    }

    /// Whitespace, comments and newlines — between statements, and inside
    /// arrays.
    fn skip_blank(&mut self) {
        loop {
            self.skip_ws();
            self.skip_comment();
            match self.peek() {
                Some(b'\n') => {
                    self.bump();
                }
                Some(b'\r') if self.peek_at(1) == Some(b'\n') => {
                    self.bump();
                    self.bump();
                }
                _ => return,
            }
        }
    }

    /// The rest of a statement line: whitespace, an optional comment, then a
    /// newline or the end of the file.
    fn expect_eol(&mut self) -> Result<(), Error> {
        self.skip_ws();
        self.skip_comment();
        match self.peek() {
            None => Ok(()),
            Some(b'\n') => {
                self.bump();
                Ok(())
            }
            Some(b'\r') if self.peek_at(1) == Some(b'\n') => {
                self.bump();
                self.bump();
                Ok(())
            }
            Some(_) => Err(self.err(ErrorKind::Expected("end of line"))),
        }
    }

    /// `a.b."c d".e`.
    fn key_path(&mut self) -> Result<Vec<String>, Error> {
        let mut path = vec![self.simple_key()?];
        loop {
            self.skip_ws();
            if self.peek() != Some(b'.') {
                return Ok(path);
            }
            self.bump();
            self.skip_ws();
            path.push(self.simple_key()?);
        }
    }

    /// A bare key, a `"basic"` key or a `'literal'` key.
    fn simple_key(&mut self) -> Result<String, Error> {
        match self.peek() {
            Some(b'"') => self.basic_string(),
            Some(b'\'') => self.literal_string(),
            Some(b) if is_bare(b) => {
                let start = self.pos;
                while self.peek().is_some_and(is_bare) {
                    self.bump();
                }
                Ok(String::from_utf8_lossy(&self.src[start..self.pos]).into_owned())
            }
            _ => Err(self.err(ErrorKind::Expected("a key"))),
        }
    }

    fn value(&mut self) -> Result<Value, Error> {
        match self.peek() {
            Some(b'"') => {
                if self.peek_at(1) == Some(b'"') && self.peek_at(2) == Some(b'"') {
                    return Err(self.err(ErrorKind::Unsupported("a multi-line string")));
                }
                self.basic_string().map(Value::String)
            }
            Some(b'\'') => {
                if self.peek_at(1) == Some(b'\'') && self.peek_at(2) == Some(b'\'') {
                    return Err(self.err(ErrorKind::Unsupported("a multi-line literal string")));
                }
                self.literal_string().map(Value::String)
            }
            Some(b'[') => self.array(),
            Some(b'{') => self.inline_table(),
            Some(b) if b == b'+' || b == b'-' || b.is_ascii_alphanumeric() => self.scalar(),
            _ => Err(self.err(ErrorKind::Expected("a value"))),
        }
    }

    /// `"…"` with escapes. The opening quote is at the cursor.
    fn basic_string(&mut self) -> Result<String, Error> {
        self.bump();
        let mut out = String::new();
        loop {
            match self.bump() {
                None | Some(b'\n') => return Err(self.err(ErrorKind::Unterminated("string"))),
                Some(b'"') => return Ok(out),
                Some(b'\\') => {
                    let c = match self.bump() {
                        Some(b'b') => '\u{8}',
                        Some(b't') => '\t',
                        Some(b'n') => '\n',
                        Some(b'f') => '\u{c}',
                        Some(b'r') => '\r',
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'u') => self.unicode_escape(4)?,
                        Some(b'U') => self.unicode_escape(8)?,
                        _ => return Err(self.err(ErrorKind::BadEscape)),
                    };
                    out.push(c);
                }
                Some(b) => {
                    // Copy a whole UTF-8 sequence through, byte for byte.
                    let start = self.pos - 1;
                    let len = utf8_len(b);
                    for _ in 1..len {
                        if self.bump().is_none() {
                            return Err(self.err(ErrorKind::Unterminated("string")));
                        }
                    }
                    out.push_str(&String::from_utf8_lossy(&self.src[start..self.pos]));
                }
            }
        }
    }

    fn unicode_escape(&mut self, digits: usize) -> Result<char, Error> {
        let mut code = 0u32;
        for _ in 0..digits {
            let d = self.bump().and_then(|b| (b as char).to_digit(16));
            let Some(d) = d else {
                return Err(self.err(ErrorKind::BadEscape));
            };
            code = code << 4 | d;
        }
        char::from_u32(code).ok_or_else(|| self.err(ErrorKind::BadEscape))
    }

    /// `'…'`, no escapes. The opening quote is at the cursor.
    fn literal_string(&mut self) -> Result<String, Error> {
        self.bump();
        let start = self.pos;
        loop {
            match self.peek() {
                None | Some(b'\n') => return Err(self.err(ErrorKind::Unterminated("string"))),
                Some(b'\'') => {
                    let s = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
                    self.bump();
                    return Ok(s);
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    /// `true`, `false`, an integer or a float — anything that is not quoted or
    /// bracketed.
    fn scalar(&mut self) -> Result<Value, Error> {
        let start = self.pos;
        while self.peek().is_some_and(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'_' | b'.' | b':')
        }) {
            self.bump();
        }
        let raw = core::str::from_utf8(&self.src[start..self.pos]).expect("ASCII only");

        match raw {
            "true" => return Ok(Value::Boolean(true)),
            "false" => return Ok(Value::Boolean(false)),
            "inf" | "+inf" | "-inf" | "nan" | "+nan" | "-nan" => {
                return Err(self.err(ErrorKind::Unsupported("a non-finite float")));
            }
            _ => {}
        }

        // A date has a `-` after its first digit, a time a `:`. Neither can
        // be an integer, and reading one as a string would be a lie.
        if raw.contains(':') || raw.get(1..).is_some_and(|rest| rest.contains('-') && !raw.contains('e') && !raw.contains('E')) {
            return Err(self.err(ErrorKind::Unsupported("a date or time")));
        }

        let cleaned: String = raw.chars().filter(|&c| c != '_').collect();
        let bad = || self.err(ErrorKind::BadNumber(raw.to_owned()));

        if let Some(hex) = cleaned.strip_prefix("0x") {
            return i64::from_str_radix(hex, 16).map(Value::Integer).map_err(|_| bad());
        }
        if let Some(oct) = cleaned.strip_prefix("0o") {
            return i64::from_str_radix(oct, 8).map(Value::Integer).map_err(|_| bad());
        }
        if let Some(bin) = cleaned.strip_prefix("0b") {
            return i64::from_str_radix(bin, 2).map(Value::Integer).map_err(|_| bad());
        }
        if cleaned.contains(['.', 'e', 'E']) {
            return cleaned.parse::<f64>().map(Value::Float).map_err(|_| bad());
        }
        cleaned.parse::<i64>().map(Value::Integer).map_err(|_| bad())
    }

    /// `[ v, v, ]` — newlines and comments allowed anywhere inside.
    fn array(&mut self) -> Result<Value, Error> {
        self.bump();
        let mut items = Vec::new();
        loop {
            self.skip_blank();
            match self.peek() {
                None => return Err(self.err(ErrorKind::Unterminated("array"))),
                Some(b']') => {
                    self.bump();
                    return Ok(Value::Array(items));
                }
                _ => {}
            }
            items.push(self.value()?);
            self.skip_blank();
            match self.peek() {
                Some(b',') => {
                    self.bump();
                }
                Some(b']') => {}
                None => return Err(self.err(ErrorKind::Unterminated("array"))),
                Some(_) => return Err(self.err(ErrorKind::Expected("`,` or `]` in array"))),
            }
        }
    }

    /// `{ k = v, k = v }` on one line.
    fn inline_table(&mut self) -> Result<Value, Error> {
        let line = self.line;
        self.bump();
        let mut table = Table::new();
        table.defined = true;
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.bump();
            return Ok(Value::Table(table));
        }
        loop {
            self.skip_ws();
            let path = self.key_path()?;
            self.skip_ws();
            self.expect(b'=', "`=` in inline table")?;
            self.skip_ws();
            let value = self.value()?;
            assign(&mut table, &path, value, line)?;
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(Value::Table(table)),
                _ => return Err(self.err(ErrorKind::Expected("`,` or `}` in inline table"))),
            }
        }
    }
}

#[inline]
const fn is_bare(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Length of the UTF-8 sequence that starts with `first`.
#[inline]
const fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}
