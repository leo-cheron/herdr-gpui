use super::*;
use crate::{
    smooth_scroll::Slide,
    terminal_painter::Span,
    window::regions::{Region, Sliding, split},
};

fn span(row: u16, columns: std::ops::Range<u16>) -> Span {
    Span { row, columns }
}

#[test]
fn split_keeps_every_cell_on_exactly_one_side_of_the_rect() {
    let area = [span(0, 0..6), span(1, 0..6), span(2, 4..6), span(3, 0..6)];
    let (inside, outside) = split(&area, rect(1, 1, 3, 2));
    assert_eq!(inside, [span(1, 1..4)]);
    assert_eq!(
        outside,
        [
            span(0, 0..6),
            span(1, 0..1),
            span(1, 4..6),
            span(2, 4..6),
            span(3, 0..6)
        ]
    );
}

#[test]
fn a_band_paints_only_the_cells_inside_the_pane_on_its_rows() {
    let area: Vec<_> = (0..5).map(|row| span(row, 0..6)).collect();
    let slide = Slide {
        pane_id: "p".into(),
        rect: rect(1, 1, 4, 3),
        offset: 0.,
        behind: vec![],
    };
    let sliding = Sliding::new(slide, &area);
    assert_eq!(sliding.rows(0..1), []);
    assert_eq!(sliding.rows(2..4), [span(2, 1..5), span(3, 1..5)]);
    assert_eq!(sliding.rows(3..9), [span(3, 1..5)]);
}

#[test]
fn a_region_replays_only_a_slide_drawn_at_the_same_offset() {
    let surface = Arc::new(PaneSurfaceFrame {
        boot_id: String::new(),
        projection_revision: 0,
        surface_revision: 1,
        frame: frame(&["aaaa"]),
        panes: vec![pane("p", rect(0, 0, 4, 1))],
        splits: vec![],
        popup: None,
        graphics: Default::default(),
    });
    let at_rest = Region {
        owner: Owner::Pane("p".into()),
        area: partition(&surface.frame, &surface.panes).remove(0).1,
        surface,
        highlights: vec![],
        look: look(),
        slide: None,
    };
    let slide = |offset| Slide {
        pane_id: "p".into(),
        rect: rect(0, 0, 4, 1),
        offset,
        behind: vec![],
    };
    let sliding = Region {
        slide: Some(Sliding::new(slide(-0.5), &at_rest.area)),
        ..at_rest.clone()
    };
    let moved = Region {
        slide: Some(Sliding::new(slide(-0.25), &at_rest.area)),
        ..at_rest.clone()
    };
    assert!(at_rest.paints_like(&at_rest));
    // A pane resting mid-row keeps its paint; a moving one repaints.
    assert!(sliding.paints_like(&sliding));
    assert!(!sliding.paints_like(&moved));
    // Coming to rest repaints once without the offset.
    assert!(!sliding.paints_like(&at_rest));
}
