//! `jeed_crypto::book` — depth and emptiness.

use jeed_crypto::BookShape;
use jeed_wire::header_flags;

#[test]
fn the_deeper_side_sets_the_depth() {
    assert_eq!(BookShape::new(3, 7).depth, 7);
    assert_eq!(BookShape::new(7, 3).depth, 7);
}

#[test]
fn an_empty_side_is_flagged_and_only_that_side() {
    let s = BookShape::new(0, 4);
    assert!(s.flags & header_flags::BID_EMPTY != 0);
    assert!(s.flags & header_flags::ASK_EMPTY == 0);
    assert_eq!(s.depth, 4);
}

#[test]
fn a_book_with_neither_side_is_flagged_on_both() {
    let s = BookShape::new(0, 0);
    assert_eq!(s.depth, 0);
    assert!(s.flags & header_flags::BID_EMPTY != 0);
    assert!(s.flags & header_flags::ASK_EMPTY != 0);
}

#[test]
fn top_of_book_reads_the_quantities() {
    assert_eq!(BookShape::top_of_book(10, 20), BookShape::new(1, 1));
    assert_eq!(BookShape::top_of_book(0, 20), BookShape::new(0, 1));
    assert_eq!(BookShape::top_of_book(0, 0), BookShape::new(0, 0));
}
