//! `jeed::cpu` — masks, pinning, and the SMT layout.
//!
//! The pinning tests run on whatever box runs them, so they pin to the
//! processor the thread is already on rather than to a number the box may not
//! have.

use jeed::cpu::{CpuError, MAX_LOGICAL, Topology, current_processor, mask_of, pin_current_thread};

#[test]
fn a_mask_is_one_bit_per_core() {
    assert_eq!(mask_of(&[]), Some(0));
    assert_eq!(mask_of(&[2]), Some(0b100));
    assert_eq!(mask_of(&[4, 5, 6, 7]), Some(0xF0));
    assert_eq!(mask_of(&[0, 63]), Some(1 | 1 << 63));
    assert_eq!(mask_of(&[64]), None, "beyond a u64");
    assert_eq!(mask_of(&[1, 64]), None);
}

#[test]
fn an_empty_mask_is_refused_before_any_syscall() {
    assert_eq!(pin_current_thread(0), Err(CpuError::EmptyMask));
}

#[test]
fn pinning_moves_the_thread_and_keeps_it_there() {
    // On its own thread so the test runner's thread is left alone.
    std::thread::spawn(|| {
        let here = current_processor();
        assert!(here < MAX_LOGICAL);
        pin_current_thread(1 << here).unwrap();
        for _ in 0..1000 {
            assert_eq!(current_processor(), here);
            std::hint::spin_loop();
        }
        std::thread::yield_now();
        assert_eq!(current_processor(), here, "still there after a yield");
    })
    .join()
    .unwrap();
}

#[test]
fn a_processor_the_box_does_not_have_is_an_os_error() {
    let topo = Topology::detect().unwrap();
    if topo.logical() >= MAX_LOGICAL {
        eprintln!("skipped: {} logical processors fill the mask", topo.logical());
        return;
    }
    // Only the top bit: a mask entirely outside the box.
    let err = std::thread::spawn(|| pin_current_thread(1 << (MAX_LOGICAL - 1))).join().unwrap().unwrap_err();
    assert!(matches!(err, CpuError::Os { .. }), "{err}");
}

#[test]
fn the_detected_topology_is_consistent() {
    let topo = Topology::detect().unwrap();
    let n = topo.logical();
    assert!(n >= 1);
    assert!(n <= MAX_LOGICAL);
    assert_eq!(topo.siblings_of(n), None);
    for lp in 0..n {
        let mask = topo.siblings_of(lp).unwrap();
        assert_ne!(mask & (1 << lp), 0, "processor {lp} is its own sibling");
        // Symmetric: everything in my mask has my mask.
        for other in 0..n {
            if mask & (1 << other) != 0 {
                assert_eq!(topo.siblings_of(other), Some(mask), "{lp} and {other} disagree");
            }
        }
    }
}

#[test]
fn siblings_outside_names_only_the_other_half_of_each_core() {
    // (0,1) (2,3) (4,5) SMT pairs.
    let topo = Topology::from_siblings(vec![0b11, 0b11, 0b1100, 0b1100, 0b11_0000, 0b11_0000]);
    assert_eq!(topo.logical(), 6);
    assert_eq!(topo.siblings_outside(0b100), 0b1000, "core 2's sibling is 3");
    assert_eq!(topo.siblings_outside(0b1100), 0, "both halves already taken");
    assert_eq!(topo.siblings_outside(0b1 | 0b1_0000), 0b10 | 0b10_0000);
    assert_eq!(topo.siblings_outside(0), 0);
    // A bit beyond the topology is ignored, not a panic.
    assert_eq!(topo.siblings_outside(1 << 40), 0);
}
