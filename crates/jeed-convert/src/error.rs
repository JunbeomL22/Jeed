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
