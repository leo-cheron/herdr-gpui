//! Wheel scrolling drawn exactly where the OS puts it.
//!
//! The OS reports a trackpad or wheel gesture as fractional deltas, already
//! eased through its momentum. The daemon can only show whole rows, at most
//! one surface per render interval (16 ms upstream). So the pane is drawn at
//! the OS's position, each delta slid in over `SPREAD` to cover the daemon's
//! latency, from the rows of the presented surface and of the earlier ones that
//! showed the rows it uncovers. The daemon is asked for each row as the motion
//! enters it, so the rows to draw are normally already on screen; where they
//! are not, the drawing waits at the nearest one it has. When the gesture
//! stops part-way into a row, the pane rests there, and hit testing follows the
//! same offset (`HerdrWindow::grid_position`).
//!
//! Earlier surfaces fill only when the pane's rows show the shift. Output
//! arriving while scrolled back moves the offset without moving the picture;
//! a picture that cannot be followed, or a scroll the wheel did not ask for,
//! ends the motion at the daemon's row.

use herdr_client::protocol::{FrameData, PaneSurfaceFrame, SurfaceRect};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

/// Each delta slides in over this long: the daemon's render interval plus
/// transport, so the row a delta reaches has normally arrived by then.
const SPREAD: Duration = Duration::from_millis(48);
/// Earlier surfaces kept to fill the rows the drawing uncovers.
const MAX_BEHIND: usize = 4;
/// Deltas kept while they slide in; a trackpad sends about one per frame.
const MAX_RECENT: usize = 64;

/// One pane to paint offset this frame.
#[derive(Clone)]
pub(crate) struct Slide {
    pub(crate) pane_id: String,
    /// The pane's inner rectangle, which clips the content as it slides.
    pub(crate) rect: SurfaceRect,
    /// Rows the current frame's pane content is drawn below its grid position.
    pub(crate) offset: f32,
    /// Earlier frames, nearest first, each with the rows the content has moved
    /// down since it, all in one direction. Each fills the rows uncovered
    /// beyond those the frames before it can.
    pub(crate) behind: Vec<(Arc<PaneSurfaceFrame>, i32)>,
}

impl Slide {
    /// Whether `other` draws exactly the same, so a resting pane keeps its
    /// cached paint.
    pub(crate) fn paints_like(&self, other: &Self) -> bool {
        self.pane_id == other.pane_id
            && self.rect == other.rect
            && self.offset == other.offset
            && self.behind.len() == other.behind.len()
            && (self.behind.iter().zip(&other.behind))
                .all(|((a, s), (b, t))| s == t && Arc::ptr_eq(a, b))
    }
}

/// One pane's gesture, in rows from the bottom of its scrollback.
struct Motion {
    pane_id: String,
    rect: SurfaceRect,
    /// The presented surface's offset.
    shown: u64,
    /// Where the OS has put the content.
    target: f64,
    /// The offset the daemon has been asked for: the row the motion is
    /// entering, so it is normally on screen before the drawing reaches it.
    requested: i64,
    /// Deltas still sliding in, oldest first.
    recent: VecDeque<(Instant, f64)>,
    last: Instant,
    /// Earlier verified surfaces with the offset each showed, newest first.
    behind: Vec<(Arc<PaneSurfaceFrame>, u64)>,
}

impl Motion {
    /// This motion's pane's scrollback offset in `surface`.
    fn scrolled(&self, surface: &PaneSurfaceFrame) -> Option<u64> {
        surface
            .panes
            .iter()
            .find(|pane| pane.pane_id == self.pane_id && pane.inner_rect == self.rect)?
            .scroll
            .map(|scroll| scroll.offset_from_bottom)
    }

    /// Rows from the bottom drawn at `now`.
    fn drawn(&self, now: Instant) -> f64 {
        let pending: f64 = self
            .recent
            .iter()
            .map(|(at, rows)| {
                let slid = now.saturating_duration_since(*at).as_secs_f64() / SPREAD.as_secs_f64();
                rows * (1. - slid).max(0.)
            })
            .sum();
        let shown = self.shown as f64;
        let wanted = self.target - pending - shown;
        // Only rows some surface showed can be drawn.
        let reach = self
            .behind
            .iter()
            .map(|(_, offset)| *offset as f64 - shown)
            .filter(|rows| rows.signum() == wanted.signum())
            .fold(0., |reach: f64, rows| reach.max(rows.abs()));
        shown + wanted.clamp(-reach, reach)
    }

    /// Rows the pane's content is drawn below its grid position at `now`.
    fn offset(&self, now: Instant) -> f32 {
        (self.drawn(now) - self.shown as f64) as f32
    }
}

#[derive(Default)]
pub(crate) struct SmoothScroll {
    motion: Option<Motion>,
}

impl SmoothScroll {
    /// Records that the OS scrolled `pane_id` by `rows` (up into history is
    /// positive) over `presented`, and returns the lines to scroll the daemon,
    /// or `None` when the pane's scrollback does not follow the wheel: an
    /// application reading it moves no scrollback to draw.
    pub(crate) fn wheel(
        &mut self,
        presented: &PaneSurfaceFrame,
        pane_id: &str,
        rows: f32,
        now: Instant,
    ) -> Option<i16> {
        let Some((pane, scroll)) = presented
            .panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .filter(|pane| !pane.mouse_reporting && !pane.alternate_screen_active)
            .and_then(|pane| Some((pane, pane.scroll?)))
        else {
            self.motion = None;
            return None;
        };
        let rows = f64::from(rows);
        if !rows.is_finite() {
            return Some(0);
        }
        let continued = self
            .motion
            .take()
            .filter(|motion| motion.pane_id == pane_id && motion.rect == pane.inner_rect)
            .map(|mut motion| {
                let drawn = motion.drawn(now);
                // A gesture resuming while its deltas still slide in, or
                // turning back, starts from where the content is drawn.
                let reversed = motion
                    .recent
                    .back()
                    .is_some_and(|(_, last)| last * rows < 0.);
                if reversed || now >= motion.last + SPREAD {
                    motion.target = drawn;
                    motion.recent.clear();
                }
                motion
            });
        let offset = scroll.offset_from_bottom;
        let motion = self.motion.insert(continued.unwrap_or_else(|| Motion {
            pane_id: pane_id.into(),
            rect: pane.inner_rect,
            shown: offset,
            target: offset as f64,
            requested: offset as i64,
            recent: VecDeque::new(),
            last: now,
            behind: Vec::new(),
        }));
        // The scrollback ends both ways; motion past an end draws nothing.
        let max = scroll.max_offset_from_bottom as f64;
        let rows = (motion.target + rows).clamp(0., max) - motion.target;
        if rows == 0. {
            return Some(0);
        }
        motion.target += rows;
        motion.recent.push_back((now, rows));
        while motion.recent.len() > MAX_RECENT
            || motion
                .recent
                .front()
                .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= SPREAD)
        {
            motion.recent.pop_front();
        }
        motion.last = now;
        let entering = match rows > 0. {
            true => motion.target.ceil(),
            false => motion.target.floor(),
        } as i64;
        // Bound each event's work, as the wheel's own accumulator does.
        let lines = (entering - motion.requested).clamp(-128, 128);
        if lines * rows.signum() as i64 <= 0 {
            return Some(0);
        }
        motion.requested += lines;
        Some(lines as i16)
    }

    /// Records that `next` replaced `previous` on screen at `now`.
    pub(crate) fn observe(
        &mut self,
        previous: &Arc<PaneSurfaceFrame>,
        next: &PaneSurfaceFrame,
        now: Instant,
    ) {
        let Some(motion) = &mut self.motion else {
            return;
        };
        let (Some(from), Some(to), true) = (
            motion.scrolled(previous),
            motion.scrolled(next),
            previous.boot_id == next.boot_id,
        ) else {
            self.motion = None;
            return;
        };
        // The picture's own shift: the scroll, nothing when output arrived
        // while scrolled back, or the scroll less output arriving with it.
        // The shifts nearest the scroll are tried first, so usually one is.
        let moved = to as i64 - from as i64;
        let height = i64::from(motion.rect.height);
        let nearest = moved.max(1 - height).min(height - 1);
        let picture = (0..2 * height)
            .flat_map(|distance| [nearest - distance, nearest + distance])
            .skip(1) // `nearest` itself comes twice
            .filter(|shift| shift.abs() < height)
            .filter_map(|shift| i32::try_from(shift).ok())
            .find(|shift| rows_shifted(&previous.frame, &next.frame, motion.rect, *shift));
        // Long after the gesture, a scroll the wheel did not ask for, such as
        // the keyboard's, ends the motion where the daemon put it.
        if picture.is_some_and(|shift| shift != 0) && now >= motion.last + 2 * SPREAD {
            self.motion = None;
            return;
        }
        if let Some(shift) = picture {
            // Rows that arrived without moving the picture move the OS's
            // position and the earlier surfaces with the offset.
            let output = moved - i64::from(shift);
            motion.target += output as f64;
            motion.requested += output;
            for (_, at) in &mut motion.behind {
                *at = at.saturating_add_signed(output);
            }
            if shift != 0 {
                let at = from.saturating_add_signed(output);
                motion.behind.insert(0, (previous.clone(), at));
                motion.behind.truncate(MAX_BEHIND);
            }
        } else {
            // A picture that cannot be followed restarts the motion at it.
            motion.behind.clear();
            motion.recent.clear();
            motion.target = to as f64;
            motion.requested = to as i64;
        }
        motion.shown = to;
    }

    /// Whether the drawing still moves after `now`, so the next frame differs.
    pub(crate) fn moving(&self, now: Instant) -> bool {
        self.motion
            .as_ref()
            .is_some_and(|motion| now < motion.last + SPREAD)
    }

    /// The moving or resting pane's inner rectangle and its content's offset
    /// in rows at `now`, for hit testing what is drawn there.
    pub(crate) fn offset(&self, now: Instant) -> Option<(SurfaceRect, f32)> {
        let motion = self.motion.as_ref()?;
        Some((motion.rect, motion.offset(now)))
    }

    /// The pane drawn offset at `now`; a motion resting on the daemon's row
    /// ends.
    pub(crate) fn slide(&mut self, now: Instant) -> Option<Slide> {
        let motion = self.motion.as_ref()?;
        let offset = motion.offset(now);
        if motion.target == motion.shown as f64 && !self.moving(now) {
            self.motion = None;
            return None;
        }
        let mut behind: Vec<_> = motion
            .behind
            .iter()
            .filter_map(|(frame, at)| {
                let shift = i32::try_from(i128::from(motion.shown) - i128::from(*at)).ok()?;
                (shift.signum() == -offset.signum() as i32).then(|| (frame.clone(), shift))
            })
            .collect();
        behind.sort_by_key(|(_, shift)| shift.unsigned_abs());
        if let Some(needed) = behind
            .iter()
            .position(|(_, shift)| shift.unsigned_abs() as f32 >= offset.abs())
        {
            behind.truncate(needed + 1);
        }
        Some(Slide {
            pane_id: motion.pane_id.clone(),
            rect: motion.rect,
            offset,
            behind,
        })
    }

    pub(crate) fn clear(&mut self) {
        self.motion = None;
    }
}

/// Whether most rows of `rect` in `new` that were already on screen in `old`
/// show the same cells `shift` rows further down. A few may differ: rows of
/// the live screen below the scrollback, such as a status line, keep changing.
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
    let overlap = top.max(top + shift)..bottom.min(bottom + shift);
    let rows = overlap.len();
    let same = overlap
        .filter(|y| {
            let new = new.cells.get(row(*y));
            new.is_some() && new == old.cells.get(row(y - shift))
        })
        .count();
    rows > 0 && same * 4 >= rows * 3
}

#[cfg(test)]
mod tests;
