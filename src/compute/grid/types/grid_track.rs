//! Contains GridTrack used to represent a single grid track (row/column) during layout
use crate::{
    prelude::TaffyZero,
    style::{LengthPercentage, MaxTrackSizingFunction, MinTrackSizingFunction},
    util::sys::f32_min,
    CompactLength,
};

/// Whether a GridTrack represents an actual track or a gutter.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(in super::super) enum GridTrackKind {
    /// Track is an actual track
    Track,
    /// Track is a gutter (aka grid line) (aka gap)
    Gutter, // { name: Option<u16> },
}

/// Internal sizing information for a single grid track (row/column)
/// Gutters between tracks are sized similarly to actual tracks, so they
/// are also represented by this struct
#[derive(Debug, Clone)]
pub(in super::super) struct GridTrack {
    #[allow(dead_code)] // Used in tests + may be useful in future
    /// Whether the track is a full track, a gutter, or a placeholder that has not yet been initialised
    pub kind: GridTrackKind,

    /// Whether the track is a collapsed track/gutter. Collapsed tracks are effectively treated as if
    /// they don't exist for the purposes of grid sizing. Gutters between collapsed tracks are also collapsed.
    pub is_collapsed: bool,

    /// The minimum track sizing function of the track
    pub min_track_sizing_function: MinTrackSizingFunction,

    /// The maximum track sizing function of the track
    pub max_track_sizing_function: MaxTrackSizingFunction,

    /// The distance of the start of the track from the start of the grid container
    pub offset: f32,

    /// The size (width/height as applicable) of the track
    pub base_size: f32,

    /// A temporary scratch value when sizing tracks
    /// Note: can be infinity
    pub growth_limit: f32,

    /// A temporary scratch value when sizing tracks. Is used as an additional amount to add to the
    /// estimate for the available space in the opposite axis when content sizing items
    pub content_alignment_adjustment: f32,

    /// A temporary scratch value when "distributing space" to avoid clobbering planned increase variable
    pub item_incurred_increase: f32,
    /// A temporary scratch value when "distributing space" to avoid clobbering the main variable
    pub base_size_planned_increase: f32,
    /// A temporary scratch value when "distributing space" to avoid clobbering the main variable
    pub growth_limit_planned_increase: f32,
    /// A temporary scratch value when "distributing space"
    /// See: https://www.w3.org/TR/css3-grid-layout/#infinitely-growable
    pub infinitely_growable: bool,
}

/// Two facts about *all* tracks of one axis that `GridItem::minimum_contribution` needs for every item of the axis.
///
/// The tracks' sizing functions do not change while the intrinsic track sizes are being resolved (the only writer is
/// `GridTrack::collapse`, which runs in `initialize_grid_tracks` before sizing starts), so the facts are computed once
/// per axis instead of once per item. Scanning every track for every item is quadratic in the number of rows of a table.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct AxisTrackFlags {
    /// Whether any track of the axis has a literal `auto` min track sizing function
    pub any_auto_min: bool,
    /// Whether any track of the axis has a flexible (`fr`) max track sizing function
    pub any_flexible_max: bool,
}

impl AxisTrackFlags {
    /// Scan the tracks of an axis once. Both checks are the ones `GridItem::minimum_contribution` used to run per item.
    pub fn of(axis_tracks: &[GridTrack]) -> Self {
        // TODO: should this be 'behaves as auto' rather than just literal auto?
        let any_auto_min = axis_tracks.iter().any(|track| track.min_track_sizing_function.is_auto());
        let any_flexible_max = axis_tracks.iter().any(|track| track.max_track_sizing_function.is_fr());
        AxisTrackFlags { any_auto_min, any_flexible_max }
    }
}

impl GridTrack {
    /// GridTrack constructor with all configuration parameters for the other constructors exposed
    fn new_with_kind(
        kind: GridTrackKind,
        min_track_sizing_function: MinTrackSizingFunction,
        max_track_sizing_function: MaxTrackSizingFunction,
    ) -> GridTrack {
        GridTrack {
            kind,
            is_collapsed: false,
            min_track_sizing_function,
            max_track_sizing_function,
            offset: 0.0,
            base_size: 0.0,
            growth_limit: 0.0,
            content_alignment_adjustment: 0.0,
            item_incurred_increase: 0.0,
            base_size_planned_increase: 0.0,
            growth_limit_planned_increase: 0.0,
            infinitely_growable: false,
        }
    }

    /// Create new GridTrack representing an actual track (not a gutter)
    pub fn new(
        min_track_sizing_function: MinTrackSizingFunction,
        max_track_sizing_function: MaxTrackSizingFunction,
    ) -> GridTrack {
        Self::new_with_kind(GridTrackKind::Track, min_track_sizing_function, max_track_sizing_function)
    }

    /// Create a new GridTrack representing a gutter
    pub fn gutter(size: LengthPercentage) -> GridTrack {
        Self::new_with_kind(
            GridTrackKind::Gutter,
            MinTrackSizingFunction::from(size),
            MaxTrackSizingFunction::from(size),
        )
    }

    /// Mark a GridTrack as collapsed. Also sets both of the track's sizing functions
    /// to fixed zero-sized sizing functions.
    pub fn collapse(&mut self) {
        self.is_collapsed = true;
        self.min_track_sizing_function = MinTrackSizingFunction::ZERO;
        self.max_track_sizing_function = MaxTrackSizingFunction::ZERO;
    }

    #[inline(always)]
    /// Returns true if the track is flexible (has a Flex MaxTrackSizingFunction), else false.
    pub fn is_flexible(&self) -> bool {
        self.max_track_sizing_function.is_fr()
    }

    #[inline(always)]
    /// Returns true if the track is flexible (has a Flex MaxTrackSizingFunction), else false.
    pub fn uses_percentage(&self) -> bool {
        self.min_track_sizing_function.uses_percentage() || self.max_track_sizing_function.uses_percentage()
    }

    #[inline(always)]
    /// Returns true if the track has an intrinsic min and or max sizing function
    pub fn has_intrinsic_sizing_function(&self) -> bool {
        self.min_track_sizing_function.is_intrinsic() || self.max_track_sizing_function.is_intrinsic()
    }

    #[inline]
    /// Returns true if the track is flexible (has a Flex MaxTrackSizingFunction), else false.
    pub fn fit_content_limit(&self, axis_available_grid_space: Option<f32>) -> f32 {
        match self.max_track_sizing_function.0.tag() {
            CompactLength::FIT_CONTENT_PX_TAG => self.max_track_sizing_function.0.value(),
            CompactLength::FIT_CONTENT_PERCENT_TAG => match axis_available_grid_space {
                Some(space) => space * self.max_track_sizing_function.0.value(),
                None => f32::INFINITY,
            },
            _ => f32::INFINITY,
        }
    }

    #[inline]
    /// Returns true if the track is flexible (has a Flex MaxTrackSizingFunction), else false.
    pub fn fit_content_limited_growth_limit(&self, axis_available_grid_space: Option<f32>) -> f32 {
        f32_min(self.growth_limit, self.fit_content_limit(axis_available_grid_space))
    }

    #[inline]
    /// Returns the track's flex factor if it is a flex track, else 0.
    pub fn flex_factor(&self) -> f32 {
        if self.max_track_sizing_function.is_fr() {
            self.max_track_sizing_function.0.value()
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod axis_track_flags_tests {
    use super::{AxisTrackFlags, GridTrack};
    use crate::style::{LengthPercentage, MaxTrackSizingFunction, MinTrackSizingFunction};

    /// The two scans exactly as `GridItem::minimum_contribution` ran them for every item (Taffy 0.9.2, `grid_item.rs:497-504`)
    fn reference(axis_tracks: &[GridTrack]) -> (bool, bool) {
        let spans_auto_min_track = axis_tracks.iter().any(|track| track.min_track_sizing_function.is_auto());
        let spans_a_flexible_track = axis_tracks.iter().any(|track| track.max_track_sizing_function.is_fr());
        (spans_auto_min_track, spans_a_flexible_track)
    }

    fn check(axis_tracks: &[GridTrack]) {
        let flags = AxisTrackFlags::of(axis_tracks);
        let (any_auto_min, any_flexible_max) = reference(axis_tracks);
        assert_eq!(flags, AxisTrackFlags { any_auto_min, any_flexible_max });
    }

    fn mins() -> [MinTrackSizingFunction; 5] {
        [
            MinTrackSizingFunction::auto(),
            MinTrackSizingFunction::length(10.0),
            MinTrackSizingFunction::percent(0.5),
            MinTrackSizingFunction::min_content(),
            MinTrackSizingFunction::max_content(),
        ]
    }

    fn maxs() -> [MaxTrackSizingFunction; 7] {
        [
            MaxTrackSizingFunction::auto(),
            MaxTrackSizingFunction::length(10.0),
            MaxTrackSizingFunction::percent(0.5),
            MaxTrackSizingFunction::min_content(),
            MaxTrackSizingFunction::max_content(),
            MaxTrackSizingFunction::fit_content_px(30.0),
            MaxTrackSizingFunction::fr(1.0),
        ]
    }

    #[test]
    fn empty_and_gutters_only() {
        check(&[]);
        check(&[GridTrack::gutter(LengthPercentage::length(0.0))]);
        check(&[GridTrack::gutter(LengthPercentage::length(8.0)), GridTrack::gutter(LengthPercentage::percent(0.1))]);
        assert_eq!(AxisTrackFlags::of(&[]), AxisTrackFlags { any_auto_min: false, any_flexible_max: false });
    }

    #[test]
    fn all_auto() {
        let tracks: Vec<GridTrack> =
            (0..9).map(|_| GridTrack::new(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::auto())).collect();
        check(&tracks);
        assert_eq!(AxisTrackFlags::of(&tracks), AxisTrackFlags { any_auto_min: true, any_flexible_max: false });
    }

    #[test]
    fn fixed_next_to_auto_and_fr() {
        let fixed = || GridTrack::new(MinTrackSizingFunction::length(40.0), MaxTrackSizingFunction::length(40.0));
        let auto = || GridTrack::new(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::auto());
        let fr = || GridTrack::new(MinTrackSizingFunction::length(0.0), MaxTrackSizingFunction::fr(1.0));
        let minmax_auto_fr = || GridTrack::new(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::fr(1.0));
        let fit_content =
            || GridTrack::new(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::fit_content_px(50.0));

        check(&[fixed(), auto()]);
        check(&[fixed(), fixed()]);
        check(&[fixed(), fr()]);
        check(&[fixed(), minmax_auto_fr()]);
        check(&[fit_content()]);
        assert_eq!(
            AxisTrackFlags::of(&[fixed(), fr()]),
            AxisTrackFlags { any_auto_min: false, any_flexible_max: true }
        );
        assert_eq!(
            AxisTrackFlags::of(&[minmax_auto_fr()]),
            AxisTrackFlags { any_auto_min: true, any_flexible_max: true }
        );
        assert_eq!(
            AxisTrackFlags::of(&[fit_content()]),
            AxisTrackFlags { any_auto_min: true, any_flexible_max: false }
        );
    }

    #[test]
    fn collapsed_tracks_have_fixed_zero_functions() {
        let mut collapsed = GridTrack::new(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::fr(1.0));
        collapsed.collapse();
        check(&[collapsed.clone()]);
        assert_eq!(AxisTrackFlags::of(&[collapsed]), AxisTrackFlags { any_auto_min: false, any_flexible_max: false });
    }

    #[test]
    fn auto_only_at_the_last_index() {
        // The shape Blitz produces for a table is the opposite (an `auto` at index 1 and no `fr`); an `auto` at the very
        // end checks that the flag is not just looking at the first few entries.
        let mut tracks: Vec<GridTrack> = (0..1001)
            .map(|i| GridTrack::new(MinTrackSizingFunction::length(i as f32), MaxTrackSizingFunction::length(i as f32)))
            .collect();
        check(&tracks);
        assert!(!AxisTrackFlags::of(&tracks).any_auto_min);
        tracks.push(GridTrack::new(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::length(1.0)));
        check(&tracks);
        assert!(AxisTrackFlags::of(&tracks).any_auto_min);
        tracks.push(GridTrack::new(MinTrackSizingFunction::length(1.0), MaxTrackSizingFunction::fr(2.0)));
        check(&tracks);
        assert!(AxisTrackFlags::of(&tracks).any_flexible_max);
    }

    #[test]
    fn every_pair_of_functions_in_vectors_up_to_three_tracks() {
        let mut all = Vec::new();
        for min in mins() {
            for max in maxs() {
                all.push(GridTrack::new(min, max));
            }
        }
        check(&[]);
        for a in &all {
            check(core::slice::from_ref(a));
            for b in &all {
                check(&[a.clone(), b.clone()]);
                for c in &all {
                    check(&[a.clone(), b.clone(), c.clone()]);
                }
            }
        }
    }
}
