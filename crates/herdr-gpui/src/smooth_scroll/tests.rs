#![allow(clippy::unwrap_used)]
use super::*;
use herdr_client::protocol::{CellData, PaneSurfacePane, PaneSurfaceScrollMetrics};

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

/// The pane scrolled back `n` rows from a bottom row reading 1000.
fn back(n: u32) -> Arc<PaneSurfaceFrame> {
    surface(1000 - n, u64::from(n))
}

/// The slide's offset and the shifts of the frames behind it.
fn offset(scroll: &mut SmoothScroll, now: Instant) -> Option<(f32, Vec<i32>)> {
    scroll
        .slide(now)
        .map(|slide| (slide.offset, slide.behind.iter().map(|(_, s)| *s).collect()))
}

#[test]
fn the_pane_is_drawn_where_the_os_put_it_and_rests_there() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    let (a, b) = (back(10), back(11));
    // Entering row 11 asks the daemon for it at once.
    assert_eq!(scroll.wheel(&a, "pane", 0.25, start), Some(1));
    // Asked one row ahead, the daemon shows row 11; row 10 fills behind it.
    scroll.observe(&a, &b, start);
    assert_eq!(offset(&mut scroll, start), Some((-1., vec![1])));
    assert_eq!(
        offset(&mut scroll, start + 24 * MS),
        Some((-0.875, vec![1]))
    );
    assert_eq!(offset(&mut scroll, start + SPREAD), Some((-0.75, vec![1])));
    // The gesture stopped a quarter into row 11: it rests there.
    let later = start + 100 * SPREAD;
    assert_eq!(offset(&mut scroll, later), Some((-0.75, vec![1])));
    // Hit testing targets the cells where they are drawn.
    assert_eq!(scroll.offset(later), Some((b.panes[0].inner_rect, -0.75)));
    // A resting pane is not redrawn, and resumes from where it is drawn.
    assert!(!scroll.moving(later));
    assert_eq!(scroll.wheel(&b, "pane", 0.125, later), Some(0));
    assert!(scroll.moving(later));
    assert_eq!(offset(&mut scroll, later + SPREAD), Some((-0.625, vec![1])));
    // Landing on a whole row ends the motion.
    assert_eq!(scroll.wheel(&b, "pane", 0.625, later + SPREAD), Some(0));
    assert_eq!(offset(&mut scroll, later + 2 * SPREAD), None);
}

#[test]
fn rows_no_surface_showed_are_never_drawn() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    // The OS is 2.5 rows on, the daemon one: the drawing waits at row 11.
    assert_eq!(scroll.wheel(&back(10), "pane", 2.5, start), Some(3));
    scroll.observe(&back(10), &back(11), start);
    assert_eq!(offset(&mut scroll, start + SPREAD), Some((0., vec![])));
    // A changing status row still lets the picture be followed, so a step
    // back, resuming from the row drawn, draws from the frames behind.
    let mut status = (*back(12)).clone();
    status.frame.cells[usize::from(ROWS - 1) * 4].symbol = "x".into();
    scroll.observe(&back(11), &status, start);
    assert_eq!(scroll.wheel(&status, "pane", -1., start + SPREAD), Some(-2));
    assert_eq!(
        offset(&mut scroll, start + 2 * SPREAD),
        Some((-1., vec![1]))
    );
    // Rows that mostly changed cannot be filled from: it restarts there.
    let mut changed = (*back(13)).clone();
    for cell in &mut changed.frame.cells[8..] {
        cell.symbol = "x".into();
    }
    scroll.observe(&Arc::new(status), &changed, start + SPREAD);
    assert_eq!(offset(&mut scroll, start + 2 * SPREAD), None);
    // Back the other way, nothing has shown the rows below.
    assert_eq!(
        scroll.wheel(&changed, "pane", -3., start + 2 * SPREAD),
        Some(-3)
    );
    assert_eq!(offset(&mut scroll, start + 3 * SPREAD), Some((0., vec![])));
}

#[test]
fn motion_past_the_bottom_is_not_owed_back() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    // Flicking down at the bottom moves nothing, then up moves at once.
    assert_eq!(scroll.wheel(&back(0), "pane", -50., start), Some(0));
    assert_eq!(scroll.wheel(&back(0), "pane", 0.5, start), Some(1));
    scroll.observe(&back(0), &back(1), start);
    assert_eq!(offset(&mut scroll, start + SPREAD), Some((-0.5, vec![1])));
}

#[test]
fn output_while_scrolled_back_moves_the_position_not_the_picture() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    assert_eq!(scroll.wheel(&back(10), "pane", 1., start), Some(1));
    scroll.observe(&back(10), &back(11), start);
    // Three rows of output: the offset grows, the picture stays.
    let mut grown = (*back(11)).clone();
    grown.panes[0].scroll.as_mut().unwrap().offset_from_bottom = 14;
    scroll.observe(&back(11), &grown, start);
    assert_eq!(
        offset(&mut scroll, start + SPREAD / 2),
        Some((-0.5, vec![1]))
    );
    // One row scrolled while two of output arrived: the picture moved one.
    let scrolled = surface(988, 17);
    scroll.observe(&Arc::new(grown), &scrolled, start);
    assert_eq!(
        offset(&mut scroll, start + SPREAD / 2),
        Some((-1.5, vec![1, 2]))
    );
}

#[test]
fn an_application_reading_the_wheel_another_boot_or_the_keyboard_ends_it() {
    let mut scroll = SmoothScroll::default();
    let now = Instant::now();
    let mut app = (*back(10)).clone();
    app.panes[0].mouse_reporting = true;
    assert_eq!(scroll.wheel(&app, "pane", 1., now), None);
    assert_eq!(scroll.wheel(&back(10), "other", 1., now), None);
    assert!(scroll.slide(now).is_none());

    assert_eq!(scroll.wheel(&back(10), "pane", 1., now), Some(1));
    let mut rebooted = (*back(11)).clone();
    rebooted.boot_id = "other".into();
    scroll.observe(&back(10), &rebooted, now);
    assert!(scroll.slide(now).is_none());

    // Resting mid-row, a scroll the wheel did not ask for lands on its row.
    assert_eq!(scroll.wheel(&back(10), "pane", 0.5, now), Some(1));
    scroll.observe(&back(10), &back(11), now);
    let later = now + 10 * SPREAD;
    assert_eq!(offset(&mut scroll, later), Some((-0.5, vec![1])));
    scroll.observe(&back(11), &back(12), later);
    assert!(scroll.slide(later).is_none());
}

#[test]
fn a_dying_momentum_comes_to_rest_where_the_os_stops() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    let mut shown = back(0);
    let mut requested = vec![(start, 0)];
    let (mut drawn, mut os) = (0., 0.);
    let mut moving = false;
    for ms in 0..2000 {
        let now = start + ms * MS;
        // A trackpad's momentum: one event a frame, decaying.
        if ms % 8 == 0 && ms < 1200 {
            let rows = 0.15 * (-(ms as f32) / 325.).exp();
            os += rows;
            let lines = scroll.wheel(&shown, "pane", rows, now).unwrap();
            if lines != 0 {
                let row = requested.last().unwrap().1 + u32::try_from(lines).unwrap();
                requested.push((now, row));
            }
        }
        // The daemon presents every 16 ms what was asked 4 ms earlier.
        if ms % 16 == 4 {
            let (_, row) = requested
                .iter()
                .rev()
                .find(|(at, _)| *at + 4 * MS <= now)
                .unwrap();
            let next = back(*row);
            scroll.observe(&shown, &next, now);
            shown = next;
        }
        let row = shown.panes[0].scroll.unwrap().offset_from_bottom as f32;
        let position = row + offset(&mut scroll, now).map_or(0., |(offset, _)| offset);
        moving |= position > 0.;
        // The last delta, at 1192 ms, is fully drawn 48 ms later.
        let stopped = ms >= 1192 + 48;
        assert!(!moving || stopped || position > drawn, "paused {ms} ms in");
        assert!(position - drawn < 0.05, "snapped {ms} ms in");
        drawn = position;
    }
    assert!((drawn - os).abs() < 1e-3, "rested at {drawn}, not {os}");
}

#[test]
fn turning_back_moves_the_content_with_the_first_motion() {
    let mut scroll = SmoothScroll::default();
    let start = Instant::now();
    // Up 0.4 rows: row 11 is asked for and the content follows to 10.4.
    assert_eq!(scroll.wheel(&back(10), "pane", 0.4, start), Some(1));
    scroll.observe(&back(10), &back(11), start);
    assert_eq!(offset(&mut scroll, start + SPREAD), Some((-0.6, vec![1])));
    // Turning back asks for row 10 at once, and the content moves down from
    // where it is drawn rather than first unwinding the 0.6 rows ahead.
    assert_eq!(
        scroll.wheel(&back(11), "pane", -0.1, start + SPREAD),
        Some(-1)
    );
    scroll.observe(&back(11), &back(10), start + SPREAD);
    assert_eq!(
        offset(&mut scroll, start + 2 * SPREAD),
        Some((0.3, vec![-1]))
    );
}
