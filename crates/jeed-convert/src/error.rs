//! Error types for parsing and configuration failures.


/// Parsing error types for numeric conversion.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, PartialOrd, Ord)]
pub enum ParseErr {
    /// Input string was empty.
    Empty,
    /// Invalid digit found in input.
    InvalidDigit,
    /// Integer overflow occurred.
    Overflow,
    /// Negative integer overflow occurred.
    NegOverflow,
    /// Unsigned integer not allowed for this operation.
    UnsignedNotAllowed,
    /// Invalid decimal point index.
    InvalidPointIndex,
    /// Invalid decimal point location.
    InvalidPointLocation,
    /// Input length is invalid.
    InvalidLength,
    /// Division by zero attempted.
    DivideByZero,
    /// A fractional digit the target scale cannot keep was **not** zero.
    ///
    /// Only the `*_exact` readers raise this. The plain readers truncate, which
    /// is right for a fixed-width field whose width the standard fixes, and
    /// wrong for venue text whose precision the venue chooses: `"0.000015"`
    /// read at two decimals is `0`, and a zero price is a plausible-looking
    /// value. See `jeed_convert::DynamicExtractor::to_i64_exact`.
    Precision,
}

impl std::fmt::Display for ParseErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseErr::Empty => write!(f, "cannot parse integer from empty string"),
            ParseErr::InvalidDigit => write!(f, "invalid digit found in string"),
            ParseErr::Overflow => write!(f, "integer overflow"),
            ParseErr::NegOverflow => write!(f, "negative integer overflow"),
            ParseErr::UnsignedNotAllowed => write!(f, "unsigned integer not allowed"),
            ParseErr::InvalidPointIndex => write!(f, "invalid point index"),
            ParseErr::InvalidPointLocation => write!(f, "invalid point location"),
            ParseErr::DivideByZero => write!(f, "divide by zero"),
            ParseErr::InvalidLength => write!(f, "invalid length"),
            ParseErr::Precision => write!(f, "value has more decimals than the scale keeps"),
        }
    }
}

impl std::error::Error for ParseErr {}

impl ParseErr {
    /// Returns the error name as a string slice.
    pub fn as_str(&self) -> &str {
        match self {
            ParseErr::Empty => "Empty",
            ParseErr::InvalidDigit => "InvalidDigit",
            ParseErr::Overflow => "Overflow",
            ParseErr::NegOverflow => "NegOverflow",
            ParseErr::UnsignedNotAllowed => "UnsignedNotAllowed",
            ParseErr::InvalidPointIndex => "InvalidPointIndex",
            ParseErr::InvalidPointLocation => "InvalidPointLocation",
            ParseErr::DivideByZero => "DivideByZero",
            ParseErr::InvalidLength => "InvalidLength",
            ParseErr::Precision => "Precision",
        }
    }
}

impl From<ParseErr> for String {
    fn from(error: ParseErr) -> Self {
        error.to_string()
    }
}

/// Configuration error for extractor setup.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, PartialOrd, Ord)]
pub enum ConfigErr {
    /// Normalizer value must not be zero.
    ZeroNormalizer,
    /// Invalid size configuration.
    InvalidSize,
}

impl std::fmt::Display for ConfigErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigErr::ZeroNormalizer => write!(f, "normalizer must not be zero"),
            ConfigErr::InvalidSize => write!(f, "invalid size"),
        }
    }
}

impl std::error::Error for ConfigErr {}
