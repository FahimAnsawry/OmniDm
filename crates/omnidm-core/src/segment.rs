// SPDX-License-Identifier: GPL-3.0-or-later

//! The segment map of one download: which byte ranges exist, how far each has been
//! written, and how the largest remaining range is split for a new connection.

use std::fmt;

/// Stable identifier of a segment. It never changes when other segments are split,
/// so workers and the database can refer to a segment while the map changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SegmentId(pub u32);

/// One byte range of the file. `end` is inclusive; `written` counts bytes from `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub id: SegmentId,
    pub start: u64,
    pub end: u64,
    pub written: u64,
}

impl Segment {
    /// Offset of the next byte to download.
    pub fn pos(&self) -> u64 {
        self.start + self.written
    }

    /// Bytes still to download.
    pub fn remaining(&self) -> u64 {
        (self.end + 1).saturating_sub(self.pos())
    }

    pub fn is_complete(&self) -> bool {
        self.remaining() == 0
    }

    fn len(&self) -> u64 {
        self.end - self.start + 1
    }
}

/// How splits are planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPolicy {
    /// A range is split only if at least twice this many bytes are left in it.
    pub min_segment: u64,
    /// New segments start on a multiple of this many bytes (absolute file offset).
    pub align: u64,
}

impl Default for SplitPolicy {
    fn default() -> Self {
        Self {
            min_segment: 1024 * 1024,
            align: 64 * 1024,
        }
    }
}

/// A split chosen by [`SegmentMap::plan_split`]: the donor keeps `[pos, new_start - 1]`
/// and a new segment takes `[new_start, donor.end]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPlan {
    pub donor: SegmentId,
    pub new_start: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapError {
    /// The segments don't cover `[0, total)` exactly once, in order.
    InvalidLayout,
    /// A segment's `written` exceeds its length.
    WrittenPastEnd(SegmentId),
    UnknownSegment(SegmentId),
    /// The donor already downloaded past the planned split point.
    DonorPassed(SegmentId),
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLayout => write!(f, "segments do not cover the file exactly once"),
            Self::WrittenPastEnd(id) => write!(f, "segment {} written past its end", id.0),
            Self::UnknownSegment(id) => write!(f, "unknown segment {}", id.0),
            Self::DonorPassed(id) => write!(f, "segment {} already passed the split point", id.0),
        }
    }
}

impl std::error::Error for MapError {}

/// The ordered, non-overlapping segments of a file of known size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentMap {
    total: u64,
    segments: Vec<Segment>,
    next_id: u32,
}

impl SegmentMap {
    /// A fresh map: one segment covering the whole file.
    pub fn new(total: u64) -> Self {
        if total == 0 {
            return Self {
                total,
                segments: Vec::new(),
                next_id: 0,
            };
        }
        Self {
            total,
            segments: vec![Segment {
                id: SegmentId(0),
                start: 0,
                end: total - 1,
                written: 0,
            }],
            next_id: 1,
        }
    }

    /// Rebuilds a map from stored segments, checking every invariant.
    pub fn from_parts(total: u64, segments: Vec<Segment>) -> Result<Self, MapError> {
        let mut expected_start = 0;
        let mut ids = std::collections::HashSet::new();
        for s in &segments {
            if s.start != expected_start || s.end < s.start || !ids.insert(s.id) {
                return Err(MapError::InvalidLayout);
            }
            expected_start = s.end + 1;
        }
        if expected_start != total {
            return Err(MapError::InvalidLayout);
        }
        if let Some(s) = segments.iter().find(|s| s.written > s.len()) {
            return Err(MapError::WrittenPastEnd(s.id));
        }
        let next_id = segments.iter().map(|s| s.id.0 + 1).max().unwrap_or(0);
        Ok(Self {
            total,
            segments,
            next_id,
        })
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn get(&self, id: SegmentId) -> Option<&Segment> {
        self.segments.iter().find(|s| s.id == id)
    }

    pub fn is_complete(&self) -> bool {
        self.segments.iter().all(Segment::is_complete)
    }

    /// Records `bytes` more written to segment `id`.
    pub fn record_written(&mut self, id: SegmentId, bytes: u64) -> Result<(), MapError> {
        let segment = self
            .segments
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(MapError::UnknownSegment(id))?;
        if bytes > segment.remaining() {
            return Err(MapError::WrittenPastEnd(id));
        }
        segment.written += bytes;
        Ok(())
    }

    /// Picks the segment with the largest remainder, after discounting what its connection
    /// will fetch in the next second (`speed_of` returns bytes per second).
    pub fn plan_split(
        &self,
        policy: SplitPolicy,
        speed_of: impl Fn(SegmentId) -> u64,
    ) -> Option<SplitPlan> {
        let (donor, free) = self
            .segments
            .iter()
            .filter(|s| !s.is_complete())
            .map(|s| (s, s.remaining().saturating_sub(speed_of(s.id))))
            .max_by_key(|&(_, free)| free)?;
        if free < policy.min_segment.saturating_mul(2) {
            return None;
        }
        let new_start = align_up(donor.end + 1 - free / 2, policy.align);
        (new_start <= donor.end).then_some(SplitPlan {
            donor: donor.id,
            new_start,
        })
    }

    /// Applies a planned split and returns the new segment's id.
    pub fn apply_split(&mut self, plan: SplitPlan) -> Result<SegmentId, MapError> {
        let index = self
            .segments
            .iter()
            .position(|s| s.id == plan.donor)
            .ok_or(MapError::UnknownSegment(plan.donor))?;
        let donor = self.segments[index];
        if plan.new_start <= donor.start || plan.new_start > donor.end {
            return Err(MapError::InvalidLayout);
        }
        if plan.new_start <= donor.pos() {
            return Err(MapError::DonorPassed(donor.id));
        }
        let id = SegmentId(self.next_id);
        self.next_id += 1;
        self.segments[index].end = plan.new_start - 1;
        self.segments.insert(
            index + 1,
            Segment {
                id,
                start: plan.new_start,
                end: donor.end,
                written: 0,
            },
        );
        Ok(id)
    }
}

/// Rounds `value` up to a multiple of `align` (no-op for `align` 0 or 1).
fn align_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        return value;
    }
    value.div_ceil(align) * align
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;

    fn seg(id: u32, start: u64, end: u64, written: u64) -> Segment {
        Segment {
            id: SegmentId(id),
            start,
            end,
            written,
        }
    }

    fn ranges(map: &SegmentMap) -> Vec<(u64, u64, u64)> {
        map.segments()
            .iter()
            .map(|s| (s.start, s.end, s.written))
            .collect()
    }

    fn no_speed(_: SegmentId) -> u64 {
        0
    }

    #[test]
    fn new_map_is_one_segment_covering_the_file() {
        let map = SegmentMap::new(1000);
        assert_eq!(ranges(&map), vec![(0, 999, 0)]);
        assert_eq!(map.total(), 1000);
        assert!(!map.is_complete());
    }

    #[test]
    fn empty_file_is_complete_with_no_segments() {
        let map = SegmentMap::new(0);
        assert!(map.segments().is_empty());
        assert!(map.is_complete());
    }

    #[test]
    fn record_written_advances_until_complete() {
        let mut map = SegmentMap::new(1000);
        let id = map.segments()[0].id;
        map.record_written(id, 400).unwrap();
        assert_eq!(map.get(id).unwrap().pos(), 400);
        assert_eq!(map.get(id).unwrap().remaining(), 600);
        map.record_written(id, 600).unwrap();
        assert!(map.get(id).unwrap().is_complete());
        assert!(map.is_complete());
    }

    #[test]
    fn record_written_past_end_is_rejected_and_changes_nothing() {
        let mut map = SegmentMap::new(1000);
        let id = map.segments()[0].id;
        map.record_written(id, 900).unwrap();
        assert_eq!(
            map.record_written(id, 101),
            Err(MapError::WrittenPastEnd(id))
        );
        assert_eq!(map.get(id).unwrap().written, 900);
    }

    #[test]
    fn record_written_to_unknown_segment_is_rejected() {
        let mut map = SegmentMap::new(1000);
        assert_eq!(
            map.record_written(SegmentId(99), 1),
            Err(MapError::UnknownSegment(SegmentId(99)))
        );
    }

    #[test]
    fn from_parts_accepts_a_valid_layout() {
        let map =
            SegmentMap::from_parts(1000, vec![seg(1, 0, 499, 500), seg(7, 500, 999, 20)]).unwrap();
        assert_eq!(ranges(&map), vec![(0, 499, 500), (500, 999, 20)]);
        assert!(!map.is_complete());
    }

    #[test]
    fn from_parts_rejects_gaps_overlaps_and_wrong_totals() {
        let cases = [
            (1000, vec![seg(1, 0, 499, 0), seg(2, 501, 999, 0)]), // gap
            (1000, vec![seg(1, 0, 500, 0), seg(2, 500, 999, 0)]), // overlap
            (1000, vec![seg(1, 0, 499, 0), seg(2, 500, 998, 0)]), // short of total
            (1000, vec![seg(1, 1, 999, 0)]),                      // doesn't start at 0
            (1000, vec![seg(2, 500, 999, 0), seg(1, 0, 499, 0)]), // out of order
            (1000, vec![seg(1, 0, 499, 0), seg(1, 500, 999, 0)]), // duplicate id
            (1000, vec![]),                                       // nothing
        ];
        for (total, parts) in cases {
            assert_eq!(
                SegmentMap::from_parts(total, parts.clone()),
                Err(MapError::InvalidLayout),
                "{parts:?}"
            );
        }
    }

    #[test]
    fn from_parts_rejects_written_past_segment_end() {
        assert_eq!(
            SegmentMap::from_parts(1000, vec![seg(1, 0, 499, 501), seg(2, 500, 999, 0)]),
            Err(MapError::WrittenPastEnd(SegmentId(1)))
        );
    }

    #[test]
    fn new_ids_after_from_parts_do_not_collide() {
        let mut map = SegmentMap::from_parts(10 * MIB, vec![seg(5, 0, 10 * MIB - 1, 0)]).unwrap();
        let plan = map.plan_split(SplitPolicy::default(), no_speed).unwrap();
        let new_id = map.apply_split(plan).unwrap();
        assert_ne!(new_id, SegmentId(5));
    }

    #[test]
    fn split_takes_the_upper_half_of_an_idle_segment() {
        let map = SegmentMap::new(10 * MIB);
        let plan = map.plan_split(SplitPolicy::default(), no_speed).unwrap();
        assert_eq!(plan.donor, map.segments()[0].id);
        assert_eq!(plan.new_start, 5 * MIB);
    }

    #[test]
    fn split_discounts_one_second_of_donor_speed() {
        let map = SegmentMap::new(10 * MIB);
        // 2 MiB/s: 8 MiB effectively free, the new segment takes the last 4 MiB.
        let plan = map.plan_split(SplitPolicy::default(), |_| 2 * MIB).unwrap();
        assert_eq!(plan.new_start, 6 * MIB);
    }

    #[test]
    fn split_counts_from_the_donor_position_not_its_start() {
        let mut map = SegmentMap::new(10 * MIB);
        let id = map.segments()[0].id;
        map.record_written(id, 4 * MIB).unwrap();
        // 6 MiB left: the new segment starts 3 MiB before the end.
        let plan = map.plan_split(SplitPolicy::default(), no_speed).unwrap();
        assert_eq!(plan.new_start, 7 * MIB);
    }

    #[test]
    fn split_start_is_aligned_up() {
        let map = SegmentMap::new(1_000_000);
        let policy = SplitPolicy {
            min_segment: 100_000,
            align: 64 * KIB,
        };
        // Unaligned midpoint 500_000 rounds up to 8 * 65_536.
        let plan = map.plan_split(policy, no_speed).unwrap();
        assert_eq!(plan.new_start, 524_288);
    }

    #[test]
    fn no_split_when_alignment_pushes_past_the_end() {
        let map = SegmentMap::new(130_000);
        let policy = SplitPolicy {
            min_segment: 60_000,
            align: 128 * KIB,
        };
        assert_eq!(map.plan_split(policy, no_speed), None);
    }

    #[test]
    fn split_needs_twice_the_minimum_segment() {
        let policy = SplitPolicy::default(); // 1 MiB minimum
        assert_eq!(
            SegmentMap::new(2 * MIB - 1).plan_split(policy, no_speed),
            None
        );
        let plan = SegmentMap::new(2 * MIB)
            .plan_split(policy, no_speed)
            .unwrap();
        assert_eq!(plan.new_start, MIB);
    }

    #[test]
    fn split_picks_the_largest_effective_remainder() {
        let map = SegmentMap::from_parts(
            8 * MIB,
            vec![seg(1, 0, 4 * MIB - 1, 0), seg(2, 4 * MIB, 8 * MIB - 1, 0)],
        )
        .unwrap();
        // Segment 1 is fast (3 MiB/s), so only 1 MiB of it is effectively free;
        // idle segment 2 has 4 MiB free and must be chosen.
        let speeds = |id: SegmentId| if id == SegmentId(1) { 3 * MIB } else { 0 };
        let plan = map.plan_split(SplitPolicy::default(), speeds).unwrap();
        assert_eq!(plan.donor, SegmentId(2));
        assert_eq!(plan.new_start, 6 * MIB);
    }

    #[test]
    fn complete_segments_are_never_donors() {
        let map = SegmentMap::from_parts(
            8 * MIB,
            vec![
                seg(1, 0, 4 * MIB - 1, 4 * MIB),
                seg(2, 4 * MIB, 8 * MIB - 1, 4 * MIB),
            ],
        )
        .unwrap();
        assert_eq!(map.plan_split(SplitPolicy::default(), no_speed), None);
    }

    #[test]
    fn apply_split_shrinks_the_donor_and_inserts_in_order() {
        let mut map = SegmentMap::new(10 * MIB);
        let donor = map.segments()[0].id;
        let new_id = map
            .apply_split(SplitPlan {
                donor,
                new_start: 5 * MIB,
            })
            .unwrap();
        assert_ne!(new_id, donor);
        assert_eq!(
            ranges(&map),
            vec![(0, 5 * MIB - 1, 0), (5 * MIB, 10 * MIB - 1, 0)]
        );
        assert_eq!(map.segments()[1].id, new_id);
    }

    #[test]
    fn apply_split_is_refused_when_the_donor_already_passed_the_point() {
        let mut map = SegmentMap::new(10 * MIB);
        let donor = map.segments()[0].id;
        map.record_written(donor, 5 * MIB).unwrap();
        let before = map.clone();
        assert_eq!(
            map.apply_split(SplitPlan {
                donor,
                new_start: 5 * MIB,
            }),
            Err(MapError::DonorPassed(donor))
        );
        assert_eq!(map, before);
    }

    #[test]
    fn apply_split_rejects_a_point_outside_the_donor() {
        let mut map = SegmentMap::new(10 * MIB);
        let donor = map.segments()[0].id;
        assert_eq!(
            map.apply_split(SplitPlan {
                donor,
                new_start: 10 * MIB,
            }),
            Err(MapError::InvalidLayout)
        );
    }
}
