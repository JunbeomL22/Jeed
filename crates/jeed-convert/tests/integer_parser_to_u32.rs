use jeed_convert::ParseErr;
use jeed_convert::integer_parser::Biscuit;

const U32_LENGTH_BOUND: usize = 10;

#[test]
fn test_back_and_forth() {
    for i in (0_u32..1_000_000).step_by(1000) {
        let x = i.to_string();
        let x_byte: &[u8] = x.as_bytes();
        let val = u32::parse_decimal(x_byte);
        assert_eq!(val, Ok(i), "Failed for {} bytes", i);
    }
}

#[test]
fn test_to_u32_decimal() {
    for i in 1..U32_LENGTH_BOUND {
        let x_vec: Vec<u8> = vec![b'0'; i];
        let x: &[u8] = &x_vec[..];
        let val = u32::parse_decimal(x);
        assert_eq!(val, Ok(0), "Failed for {} bytes", i);
    }

    for i in 1..U32_LENGTH_BOUND {
        let x_vec: Vec<u8> = vec![b'1'; i];
        let x: &[u8] = &x_vec[..];
        let val = u32::parse_decimal(x).unwrap();
        assert_eq!(
            val,
            std::str::from_utf8(x).unwrap().parse::<u32>().unwrap(),
            "Failed for {} bytes",
            i
        );
    }

    for i in 1..(U32_LENGTH_BOUND - 1) {
        let x_vec: Vec<u8> = vec![b'9'; i];
        let x: &[u8] = &x_vec[..];
        let val = u32::parse_decimal(x).unwrap();
        assert_eq!(
            val,
            std::str::from_utf8(x).unwrap().parse::<u32>().unwrap(),
            "Failed for {} bytes",
            i
        );
    }
}

#[test]
fn test_u32_max() {
    let max_string = u32::MAX.to_string();
    let max_byte: &[u8] = max_string.as_bytes();
    let val = u32::parse_decimal(max_byte);
    assert_eq!(val, Ok(u32::MAX));

    let byte_test_p1 = b"4294967296"; // u32::MAX + 1
    let val_p1 = u32::parse_decimal(byte_test_p1);
    assert_eq!(val_p1, Err(ParseErr::Overflow));
}

#[test]
fn test_u32_leading_zeros() {
    let byte_leading_zeros = b"01234567890";
    let x_leading_zeros: &[u8] = &byte_leading_zeros[..];
    let val_leading_zeros = u32::parse_decimal(x_leading_zeros);
    assert_eq!(val_leading_zeros, Ok(1234567890));
}

