// SPDX-License-Identifier: GPL-3.0-or-later

//! Property tests: no sequence of writes and splits may break the segment map's invariants.

use omnidm_core::{Segment, SegmentId, SegmentMap, SplitPolicy};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    /// Write a fraction (per mille) of what is left in the n-th incomplete segment.
    Write { pick: usize, per_mille: u64 },
    /// Plan a split with this donor speed and apply it, possibly after the donor advanced.
    Split { speed: u64, advance_first: u64 },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<usize>(), 0u64..=1000).prop_map(|(pick, per_mille)| Op::Write { pick, per_mille }),
        (0u64..4_000_000, 0u64..3_000_000).prop_map(|(speed, advance_first)| Op::Split {
            speed,
            advance_first
        }),
    ]
}

/// Independent check of the layout, written without using `SegmentMap`'s own validation.
fn assert_covers_exactly(total: u64, segments: &[Segment]) {
    let mut next = 0;
    for s in segments {
        assert_eq!(s.start, next, "gap or overlap at {next}: {segments:?}");
        assert!(s.end >= s.start, "empty segment: {s:?}");
        assert!(s.written <= s.end - s.start + 1, "written past end: {s:?}");
        next = s.end + 1;
    }
    assert_eq!(next, total, "segments end at {next}, file is {total}");
    let mut ids: Vec<SegmentId> = segments.iter().map(|s| s.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), segments.len(), "duplicate segment ids");
}

fn total_written(map: &SegmentMap) -> u64 {
    map.segments().iter().map(|s| s.written).sum()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn writes_and_splits_keep_the_map_consistent(
        total in 1u64..50_000_000,
        min_segment in 1u64..2_000_000,
        align in prop_oneof![Just(1u64), Just(4096), Just(65_536)],
        ops in prop::collection::vec(op(), 1..60),
    ) {
        let policy = SplitPolicy { min_segment, align };
        let mut map = SegmentMap::new(total);
        let mut written = 0;

        for op in ops {
            match op {
                Op::Write { pick, per_mille } => {
                    let open: Vec<Segment> =
                        map.segments().iter().copied().filter(|s| !s.is_complete()).collect();
                    if open.is_empty() {
                        continue;
                    }
                    let s = open[pick % open.len()];
                    let bytes = s.remaining() * per_mille / 1000;
                    map.record_written(s.id, bytes).unwrap();
                    written += bytes;
                }
                Op::Split { speed, advance_first } => {
                    let Some(plan) = map.plan_split(policy, |_| speed) else { continue };
                    let donor = *map.get(plan.donor).unwrap();
                    prop_assert!(plan.new_start > donor.pos() && plan.new_start <= donor.end);
                    prop_assert_eq!(plan.new_start % align, 0);
                    // The donor keeps downloading between planning and applying.
                    let advance = advance_first.min(donor.remaining());
                    map.record_written(donor.id, advance).unwrap();
                    written += advance;
                    let passed = map.get(donor.id).unwrap().pos() >= plan.new_start;
                    let before = map.clone();
                    match map.apply_split(plan) {
                        Ok(new_id) => {
                            prop_assert!(!passed);
                            let new = map.get(new_id).unwrap();
                            prop_assert_eq!(new.written, 0);
                            prop_assert!(new.end - new.start + 1 >= 1);
                        }
                        Err(_) => {
                            prop_assert!(passed);
                            prop_assert_eq!(&map, &before);
                        }
                    }
                }
            }
            assert_covers_exactly(total, map.segments());
            prop_assert_eq!(total_written(&map), written);
        }

        // Finishing every segment always completes the file.
        for s in map.segments().to_vec() {
            map.record_written(s.id, s.remaining()).unwrap();
        }
        prop_assert!(map.is_complete());
        prop_assert_eq!(total_written(&map), total);
    }
}
