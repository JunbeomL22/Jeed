use jeed_convert::ParseErr;
use jeed_convert::integer_parser::Biscuit;

const U16_LENGTH_BOUND: usize = 5;

#[test]
fn test_back_and_forth() {
    for i in 0_u16..u16::MAX {
        let x = i.to_string();
        let x_byte: &[u8] = x.as_bytes();
        let val = u16::parse_decimal(x_byte);
        assert_eq!(val, Ok(i), "Failed for {} bytes", i);
    }
}

#[test]
fn test_to_u16_decimal() {
    for i in 1..U16_LENGTH_BOUND {
        let x_vec: Vec<u8> = vec![b'0'; i];
        let x: &[u8] = &x_vec[..];
        let val = u16::parse_decimal(x);
        assert_eq!(val, Ok(0), "Failed for {} bytes", i);
    }

    for i in 1..U16_LENGTH_BOUND {
        let x_vec: Vec<u8> = vec![b'1'; i];
        let x: &[u8] = &x_vec[..];
        let val = u16::parse_decimal(x).unwrap();
        assert_eq!(
            val,
            std::str::from_utf8(x).unwrap().parse::<u16>().unwrap(),
            "Failed for {} bytes",
            i
        );
    }
}

#[test]
fn test_u16_max() {
    let max_string = u16::MAX.to_string();
    let max_byte: &[u8] = max_string.as_bytes();
    let val = u16::parse_decimal(max_byte);
    assert_eq!(val, Ok(u16::MAX));

    let byte_test_p1 = b"65536"; // u16::MAX + 1
    let val_p1 = u16::parse_decimal(byte_test_p1);
    assert_eq!(val_p1, Err(ParseErr::Overflow));
}

#[test]
fn test_u16_leading_zeros() {
    let byte_leading_zeros = b"012345";
    let x_leading_zeros: &[u8] = &byte_leading_zeros[..];
    let val_leading_zeros = u16::parse_decimal(x_leading_zeros);
    assert_eq!(val_leading_zeros, Ok(12345));
}

