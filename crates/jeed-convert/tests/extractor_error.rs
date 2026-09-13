// Allow clone_on_copy since we test Clone trait explicitly
#![allow(clippy::clone_on_copy)]

use jeed_convert::extractor::{ConfigErr, ParseErr};

#[test]
fn test_parse_err_display_empty() {
    let err = ParseErr::Empty;
    assert_eq!(format!("{}", err), "cannot parse integer from empty string");
}

#[test]
fn test_parse_err_display_invalid_digit() {
    let err = ParseErr::InvalidDigit;
    assert_eq!(format!("{}", err), "invalid digit found in string");
}

#[test]
fn test_parse_err_display_overflow() {
    let err = ParseErr::Overflow;
    assert_eq!(format!("{}", err), "integer overflow");
}

#[test]
fn test_parse_err_display_neg_overflow() {
    let err = ParseErr::NegOverflow;
    assert_eq!(format!("{}", err), "negative integer overflow");
}

#[test]
fn test_parse_err_display_unsigned_not_allowed() {
    let err = ParseErr::UnsignedNotAllowed;
    assert_eq!(format!("{}", err), "unsigned integer not allowed");
}

#[test]
fn test_parse_err_display_invalid_point_index() {
    let err = ParseErr::InvalidPointIndex;
    assert_eq!(format!("{}", err), "invalid point index");
}

#[test]
fn test_parse_err_display_invalid_point_location() {
    let err = ParseErr::InvalidPointLocation;
    assert_eq!(format!("{}", err), "invalid point location");
}

#[test]
fn test_parse_err_display_divide_by_zero() {
    let err = ParseErr::DivideByZero;
    assert_eq!(format!("{}", err), "divide by zero");
}

#[test]
fn test_parse_err_display_invalid_length() {
    let err = ParseErr::InvalidLength;
    assert_eq!(format!("{}", err), "invalid length");
}

#[test]
fn test_config_err_display_zero_normalizer() {
    let err = ConfigErr::ZeroNormalizer;
    assert_eq!(format!("{}", err), "normalizer must not be zero");
}

#[test]
fn test_config_err_display_invalid_size() {
    let err = ConfigErr::InvalidSize;
    assert_eq!(format!("{}", err), "invalid size");
}

#[test]
fn test_parse_err_equality() {
    assert_eq!(ParseErr::Empty, ParseErr::Empty);
    assert_eq!(ParseErr::InvalidDigit, ParseErr::InvalidDigit);
    assert_ne!(ParseErr::Empty, ParseErr::InvalidDigit);
}

#[test]
fn test_config_err_equality() {
    assert_eq!(ConfigErr::ZeroNormalizer, ConfigErr::ZeroNormalizer);
    assert_eq!(ConfigErr::InvalidSize, ConfigErr::InvalidSize);
    assert_ne!(ConfigErr::ZeroNormalizer, ConfigErr::InvalidSize);
}

#[test]
fn test_parse_err_ordering() {
    // ParseErr implements Ord, so we can compare
    assert!(ParseErr::Empty < ParseErr::InvalidDigit);
}

#[test]
fn test_parse_err_clone() {
    let err = ParseErr::Overflow;
    let cloned = err.clone();
    assert_eq!(err, cloned);
}

#[test]
fn test_parse_err_copy() {
    let err = ParseErr::InvalidDigit;
    let copied: ParseErr = err; // Copy, not move
    assert_eq!(err, copied);
}

#[test]
fn test_parse_err_debug() {
    let err = ParseErr::InvalidDigit;
    let debug_str = format!("{:?}", err);
    assert_eq!(debug_str, "InvalidDigit");
}

#[test]
fn test_config_err_debug() {
    let err = ConfigErr::InvalidSize;
    let debug_str = format!("{:?}", err);
    assert_eq!(debug_str, "InvalidSize");
}

#[test]
fn test_parse_err_is_error() {
    // Verify ParseErr implements std::error::Error
    fn assert_is_error<E: std::error::Error>() {}
    assert_is_error::<ParseErr>();
}

#[test]
fn test_config_err_is_error() {
    // Verify ConfigErr implements std::error::Error
    fn assert_is_error<E: std::error::Error>() {}
    assert_is_error::<ConfigErr>();
}

