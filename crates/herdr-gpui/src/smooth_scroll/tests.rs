#![allow(clippy::unwrap_used)]
use super::*;
use herdr_client::protocol::{CellData, PaneSurfaceScrollMetrics};

const ROWS: u16 = 6;
const MS: Duration = Duration::from_millis(1);

/// A one-pane surface whose row `y` reads `top + y`, scrolled back `offset`.
fn surface(top: u32, offset: u64) -> Arc<PaneSurfaceFrame> {
    let cells = (0..ROWS)
        .flat_map(|y| {
            format!("{:>4}", top + u32::from(y))
                .chars()
                .map(|c| CellData {
                    symbol: c.to_string(),
                    fg: 0,
                    bg: 0,
                    modifier: 0,
                    skip: false,
                    hyperlink: None,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let rect = SurfaceRect {
        x: 0,
        y: 0,
        width: 4,
        height: ROWS,
    };
    Arc::new(PaneSurfaceFrame {
        boot_id: "boot".into(),
        projection_revision: 1,
        surface_revision: 1,
        frame: FrameData {
            cells,
            width: 4,
            height: ROWS,
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        },
        panes: vec![PaneSurfacePane {
            pane_id: "pane".into(),
            content_revision: 1,
            rect,
            inner_rect: rect,
            scrollbar_rect: None,
            scroll: Some(PaneSurfaceScrollMetrics {
                offset_from_bottom: offset,
                max_offset_from_bottom: 1000,
                viewport_rows: u64::from(ROWS),
            }),
            focused: true,
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            alternate_screen_active: false,
            pixel_width: 0,
            pixel_height: 0,
        }],
        splits: vec![],
        popup: None,
        graphics: Default::default(),
    })
}

fn offset(scroll: &mut SmoothScroll, now: Instant) -> Option<(f32, Vec<i32>)> {
    let slides = scroll.slides(now);
    assert!(slides.len() <= 1);
    slides
        .first()
        .map(|slide| (slide.offset, slide.behind.iter().map(|(_, s)| *s).collect()))
}

#[test]
fn a_verified_shift_slides_from_the_old_position_to_rest_over_the_interval() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    // Scrolling back two rows moves the content down: row 0 now reads 98.
    let (old, new) = (surface(100, 10), surface(98, 12));
    scroll.observe(&old, &new, start);
    assert_eq!(offset(&mut scroll, start), Some((-2., vec![2])));
    assert_eq!(offset(&mut scroll, start + 12 * MS), Some((-1., vec![2])));
    assert_eq!(offset(&mut scroll, start + 24 * MS), None);

    // Toward the bottom the content moves up and slides down into place.
    scroll.observe(&new, &old, start + 30 * MS);
    assert_eq!(offset(&mut scroll, start + 30 * MS), Some((2., vec![-2])));
}

#[test]
fn metrics_or_rows_alone_never_start_a_slide() {
    let mut scroll = SmoothScroll::default();
    let now = Instant::now();
    // Output while scrolled back: the offset grows, the picture stays.
    scroll.observe(&surface(100, 10), &surface(100, 13), now);
    // The rows changed without the offset: new output at the bottom.
    scroll.observe(&surface(100, 0), &surface(98, 0), now);
    // A jump that leaves no row on screen cannot be checked or filled.
    scroll.observe(&surface(100, 10), &surface(94, 16), now);
    // Another boot is another picture.
    let mut rebooted = (*surface(98, 12)).clone();
    rebooted.boot_id = "other".into();
    scroll.observe(&surface(100, 10), &rebooted, now);
    assert!(scroll.slides(now).is_empty());
}

#[test]
fn arrivals_mid_slide_continue_from_the_drawn_position_filled_by_earlier_frames() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    let (a, b, c) = (surface(100, 10), surface(98, 12), surface(97, 13));
    scroll.observe(&a, &b, start);
    // Half way through, one more row: 1 row left over plus the new one.
    // `b` fills one uncovered row and `a`, three rows back, the other.
    scroll.observe(&b, &c, start + 12 * MS);
    assert_eq!(
        offset(&mut scroll, start + 12 * MS),
        Some((-2., vec![1, 3]))
    );
    // Nearly at rest, only `b` is needed behind the next step.
    let d = surface(96, 14);
    scroll.observe(&c, &d, start + 34 * MS);
    let (drawn, behind) = offset(&mut scroll, start + 34 * MS).unwrap();
    assert!(drawn > -2. && drawn < -1., "{drawn}");
    assert_eq!(behind, vec![1, 2]);
    // Reversing mid-slide carries the remainder without a jump, and
    // frames from the other direction cannot fill its edge.
    let mut scroll = SmoothScroll::default();
    scroll.observe(&a, &b, start);
    scroll.observe(&b, &surface(99, 11), start + 12 * MS);
    assert_eq!(offset(&mut scroll, start + 12 * MS), Some((0., vec![-1])));
}

#[test]
fn an_unrelated_update_keeps_the_slide_but_changed_rows_snap_it() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    let (old, new) = (surface(100, 10), surface(98, 12));
    scroll.observe(&old, &new, start);
    let mut redrawn = (*new).clone();
    redrawn.surface_revision += 1;
    let redrawn = Arc::new(redrawn);
    scroll.observe(&new, &redrawn, start + 4 * MS);
    let slides = scroll.slides(start + 12 * MS);
    assert_eq!(slides[0].offset, -1.);
    assert!(Arc::ptr_eq(&slides[0].behind[0].0, &old));

    let mut changed = (*redrawn).clone();
    changed.frame.cells[0].symbol = "x".into();
    scroll.observe(&redrawn, &changed, start + 13 * MS);
    assert!(scroll.slides(start + 13 * MS).is_empty());
}

#[test]
fn the_slide_duration_follows_observed_arrivals_within_bounds() {
    let mut scroll = SmoothScroll::default();
    let mut now = Instant::now();
    let mut top = 1000;
    for _ in 0..40 {
        scroll.observe(&surface(top, 0), &surface(top - 1, 1), now);
        now += 30 * MS;
        top -= 1;
    }
    assert!(scroll.interval > 28 * MS && scroll.interval <= 30 * MS);
    for _ in 0..40 {
        scroll.observe(&surface(top, 0), &surface(top - 1, 1), now);
        now += 2 * MS;
        top -= 1;
    }
    assert_eq!(scroll.interval, MIN_INTERVAL);
    // A pause starts a new gesture without stretching the next slide.
    now += 500 * MS;
    scroll.observe(&surface(top, 0), &surface(top - 1, 1), now);
    assert_eq!(scroll.interval, MIN_INTERVAL);
}

#[test]
fn decelerating_steps_keep_moving_until_the_last_one_lands() {
    // One row per step, the gaps growing as a trackpad's momentum dies.
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    let (mut at, mut gap) = (start, 17. * 1e-3);
    let mut steps = vec![];
    for row in 1..=10 {
        steps.push((at, row));
        at += Duration::from_secs_f64(gap);
        gap *= 1.3;
    }
    let frame = |row: u32| surface(1000 - row, u64::from(row));
    let mut shown = frame(0);
    let mut drawn = -1.;
    let mut next = steps.iter().peekable();
    let mut now = start;
    let last = steps[steps.len() - 1].0;
    while now <= last {
        while let Some((_, row)) = next.next_if(|(at, _)| *at <= now) {
            let landed = frame(*row);
            scroll.observe(&shown, &landed, now);
            shown = landed;
        }
        let row = shown.panes[0].scroll.unwrap().offset_from_bottom as f32;
        let position = row + offset(&mut scroll, now).map_or(0., |(offset, _)| offset);
        assert!(position > drawn, "at rest {:?} in", now - start);
        drawn = position;
        now += MS;
    }
    // The last step still finishes its slide rather than snapping.
    assert!(drawn < 10.);
}
