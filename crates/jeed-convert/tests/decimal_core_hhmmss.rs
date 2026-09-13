use jeed_convert::ParseErr;
use jeed_convert::decimal_core::hhmmss::parse_hhmmss_to_seconds;

#[test]
fn test_parse_hhmmss_basic() {
    // 10:30:10 = 10*3600 + 30*60 + 10 = 36000 + 1800 + 10 = 37810
    let input = b"103010";
    let result = parse_hhmmss_to_seconds(input);

    let expected = 10 * 3600 + 30 * 60 + 10;
    assert_eq!(result, Ok(expected));
}

#[test]
fn test_parse_hhmmss_midnight() {
    // 00:00:00 = 0
    let input = b"000000";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(0));
}

#[test]
fn test_parse_hhmmss_noon() {
    // 12:00:00 = 12*3600 = 43200
    let input = b"120000";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(43200));
}

#[test]
fn test_parse_hhmmss_end_of_day() {
    // 23:59:59 = 23*3600 + 59*60 + 59 = 82800 + 3540 + 59 = 86399
    let input = b"235959";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(86399));
}

#[test]
fn test_parse_hhmmss_one_second() {
    // 00:00:01 = 1
    let input = b"000001";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(1));
}

#[test]
fn test_parse_hhmmss_one_minute() {
    // 00:01:00 = 60
    let input = b"000100";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(60));
}

#[test]
fn test_parse_hhmmss_one_hour() {
    // 01:00:00 = 3600
    let input = b"010000";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(3600));
}

#[test]
fn test_parse_hhmmss_invalid_digit() {
    // Contains non-digit character 'a'
    let input = b"10a010";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Err(ParseErr::InvalidDigit));
}

#[test]
fn test_parse_hhmmss_invalid_special_char() {
    // Contains colon (7 bytes - invalid length)
    let input = b"10:3010";
    let result = parse_hhmmss_to_seconds(input);
    assert_eq!(result, Err(ParseErr::InvalidLength));

    // Contains colon with 6 bytes
    let input = b"10:010";
    let result = parse_hhmmss_to_seconds(input);
    assert_eq!(result, Err(ParseErr::InvalidDigit));
}

#[test]
fn test_parse_hhmmss_various_times() {
    let test_cases = [
        (b"093045", 9 * 3600 + 30 * 60 + 45),  // 09:30:45
        (b"153022", 15 * 3600 + 30 * 60 + 22), // 15:30:22
        (b"200000", 20 * 3600),                // 20:00:00
        (b"000059", 59),                       // 00:00:59
        (b"005900", 59 * 60),                  // 00:59:00
    ];

    for (input, expected) in test_cases {
        let result = parse_hhmmss_to_seconds(input);
        assert_eq!(
            result,
            Ok(expected),
            "Failed for input: {:?}",
            std::str::from_utf8(input)
        );
    }
}

#[test]
fn test_parse_hhmmss_market_open() {
    // Common market open time 09:00:00 = 32400
    let input = b"090000";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(32400));
}

#[test]
fn test_parse_hhmmss_market_close() {
    // Common market close time 15:30:00 = 55800
    let input = b"153000";
    let result = parse_hhmmss_to_seconds(input);

    assert_eq!(result, Ok(55800));
}

