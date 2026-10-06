//! Scrollback steps slid at display rate instead of jumping when they land.
//!
//! The daemon presents at most one surface per render interval (16 ms upstream),
//! so a wheel or trackpad gesture reaches the client as one-to-three-row jumps at
//! intervals that do not line up with the display's refresh. Painting each jump
//! as it lands reads as judder, most visibly on high-refresh displays. When a
//! new surface shows that a pane's scrollback moved by whole rows, the pane is
//! painted offset by that shift and slid back to rest over a little more than
//! the observed arrival interval, so motion stays continuous between surfaces.
//! Rows the slide uncovers at the trailing edge come from the earlier surfaces
//! that showed them.
//!
//! A slide starts only when the daemon's scroll metrics *and* the pane's rows
//! agree on the shift: output arriving while scrolled back moves the offset
//! without moving the picture, and anything else that changed the rows snaps.
//!
//! Presentation only: hit testing, input routing, selection, and IME placement
//! keep reading the live surface, which a slide reaches within one interval.

use herdr_client::protocol::{FrameData, PaneSurfaceFrame, PaneSurfacePane, SurfaceRect};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// The daemon's render interval, used until arrivals have been observed.
const DEFAULT_INTERVAL: Duration = Duration::from_millis(16);
const MIN_INTERVAL: Duration = Duration::from_millis(8);
const MAX_INTERVAL: Duration = Duration::from_millis(40);
/// Steps further apart than this start a new motion rather than pace one.
const MAX_PACE: Duration = Duration::from_millis(200);
/// Slides last this many step intervals. Each arrival then lands while the
/// last slide is still moving, so a late surface does not stop the motion and
/// uneven jumps blend into an even pace, at the cost of this much lag.
const LAG: f32 = 1.5;
/// Earlier frames a slide may keep to fill the rows it uncovers.
const MAX_BEHIND: usize = 4;

/// One pane to paint offset this frame.
#[derive(Clone)]
pub(crate) struct Slide {
    pub(crate) pane_id: String,
    /// The pane's inner rectangle, which clips the content as it slides.
    pub(crate) rect: SurfaceRect,
    /// Rows the current frame's pane content is drawn below its grid position.
    pub(crate) offset: f32,
    /// Earlier frames, newest first, each with the rows the content has moved
    /// down since it, all in one direction. Each fills the rows uncovered
    /// beyond those the frames before it can.
    pub(crate) behind: Vec<(Arc<PaneSurfaceFrame>, i32)>,
}

struct Active {
    pane_id: String,
    rect: SurfaceRect,
    from: f32,
    started: Instant,
    duration: Duration,
    behind: Vec<(Arc<PaneSurfaceFrame>, i32)>,
}

impl Active {
    fn offset(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32();
        self.from * (1. - (elapsed / self.duration.as_secs_f32()).min(1.))
    }
}

pub(crate) struct SmoothScroll {
    active: Vec<Active>,
    interval: Duration,
    last_shift: Option<Instant>,
}

impl Default for SmoothScroll {
    fn default() -> Self {
        Self {
            active: Vec::new(),
            interval: DEFAULT_INTERVAL,
            last_shift: None,
        }
    }
}

impl SmoothScroll {
    /// Records that `next` replaced `previous` on screen at `now`.
    pub(crate) fn observe(
        &mut self,
        previous: &Arc<PaneSurfaceFrame>,
        next: &PaneSurfaceFrame,
        now: Instant,
    ) {
        if previous.boot_id != next.boot_id {
            self.active.clear();
            return;
        }
        let shifts: Vec<_> = next
            .panes
            .iter()
            .filter_map(|pane| {
                let old = previous
                    .panes
                    .iter()
                    .find(|old| old.pane_id == pane.pane_id)?;
                let shift = scroll_shift(old, pane)?;
                rows_shifted(&previous.frame, &next.frame, pane.inner_rect, shift)
                    .then_some((pane, shift))
            })
            .collect();
        // Fast steps arrive at the daemon's cadence, paced by its smoothed
        // interval so a burst cannot rush a slide. As a trackpad's momentum
        // dies, steps spread further apart than that; each then lasts as long
        // as the gap before it, so the motion slows down instead of stuttering.
        let gap = self
            .last_shift
            .map(|last| now.saturating_duration_since(last));
        if !shifts.is_empty() {
            if let Some(gap) = gap.filter(|gap| *gap <= MAX_INTERVAL) {
                self.interval = ((self.interval * 3 + gap) / 4).clamp(MIN_INTERVAL, MAX_INTERVAL);
            }
            self.last_shift = Some(now);
        }
        let duration = gap
            .filter(|gap| *gap <= MAX_PACE)
            .map_or(self.interval, |gap| gap.max(self.interval))
            .mul_f32(LAG);
        let mut prior = std::mem::take(&mut self.active);
        let mut active = Vec::with_capacity(shifts.len());
        for (pane, shift) in shifts {
            let slide = prior
                .iter()
                .position(|slide| slide.pane_id == pane.pane_id && slide.rect == pane.inner_rect)
                .map(|index| prior.swap_remove(index));
            let mut behind = vec![(previous.clone(), shift)];
            let mut from = -shift as f32;
            if let Some(slide) = slide {
                // The new frame continues from wherever the old one was drawn.
                from = slide.offset(now) - shift as f32;
                if slide
                    .behind
                    .first()
                    .is_some_and(|(_, s)| s.signum() == shift.signum())
                {
                    behind.extend(
                        (slide.behind.into_iter())
                            .take(MAX_BEHIND - 1)
                            .map(|(frame, s)| (frame, s + shift)),
                    );
                    if let Some(needed) = behind
                        .iter()
                        .position(|(_, s)| s.unsigned_abs() as f32 >= from.abs())
                    {
                        behind.truncate(needed + 1);
                    }
                }
            }
            // Never uncover more rows than the frames behind can fill.
            let reach = -behind.last().map_or(0, |(_, s)| *s) as f32;
            active.push(Active {
                pane_id: pane.pane_id.clone(),
                rect: pane.inner_rect,
                from: from.clamp(reach.min(0.), reach.max(0.)),
                started: now,
                duration,
                behind,
            });
        }
        // A pane that did not change keeps sliding over the same rows.
        active.extend(prior.into_iter().filter(|slide| {
            next.panes
                .iter()
                .any(|pane| pane.pane_id == slide.pane_id && pane.inner_rect == slide.rect)
                && rows_shifted(&previous.frame, &next.frame, slide.rect, 0)
        }));
        self.active = active;
    }

    /// The panes still sliding at `now`; finished slides are forgotten.
    pub(crate) fn slides(&mut self, now: Instant) -> Vec<Slide> {
        self.active
            .retain(|slide| now.saturating_duration_since(slide.started) < slide.duration);
        self.active
            .iter()
            .map(|slide| Slide {
                pane_id: slide.pane_id.clone(),
                rect: slide.rect,
                offset: slide.offset(now),
                behind: slide.behind.clone(),
            })
            .collect()
    }

    pub(crate) fn clear(&mut self) {
        self.active.clear();
        self.last_shift = None;
    }
}

/// Rows the daemon reports this pane's scrollback moved, when the pane kept
/// its place and some of its rows remain on screen.
fn scroll_shift(old: &PaneSurfacePane, new: &PaneSurfacePane) -> Option<i32> {
    if old.inner_rect != new.inner_rect {
        return None;
    }
    let delta = i64::try_from(new.scroll?.offset_from_bottom).ok()?
        - i64::try_from(old.scroll?.offset_from_bottom).ok()?;
    (delta != 0 && delta.unsigned_abs() < u64::from(new.inner_rect.height))
        .then(|| i32::try_from(delta).ok())
        .flatten()
}

/// Whether every row of `rect` in `new` that was already on screen in `old`
/// shows the same cells `shift` rows further down.
fn rows_shifted(old: &FrameData, new: &FrameData, rect: SurfaceRect, shift: i32) -> bool {
    if old.width != new.width || old.height != new.height {
        return false;
    }
    let width = usize::from(new.width);
    let (left, right) = (
        usize::from(rect.x),
        usize::from(rect.x) + usize::from(rect.width),
    );
    let (top, bottom) = (
        i32::from(rect.y),
        i32::from(rect.y) + i32::from(rect.height),
    );
    if right > width || bottom > i32::from(new.height) {
        return false;
    }
    // Rows between `top` and `bottom`, so never negative.
    let row = |y: i32| {
        let start = y as usize * width;
        start + left..start + right
    };
    (top.max(top + shift)..bottom.min(bottom + shift)).all(|y| {
        let new = new.cells.get(row(y));
        new.is_some() && new == old.cells.get(row(y - shift))
    })
}

#[cfg(test)]
mod tests;
