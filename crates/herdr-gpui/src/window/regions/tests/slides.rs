use super::*;
use crate::{
    smooth_scroll::Slide,
    terminal_painter::Span,
    window::regions::{Region, split},
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
fn a_sliding_region_never_replays_its_last_paint() {
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
    let sliding = Region {
        slide: Some(Slide {
            pane_id: "p".into(),
            rect: rect(0, 0, 4, 1),
            offset: 0.,
            behind: vec![],
        }),
        ..at_rest.clone()
    };
    assert!(at_rest.paints_like(&at_rest));
    assert!(!sliding.paints_like(&sliding));
    // Coming to rest repaints once without the offset.
    assert!(!sliding.paints_like(&at_rest));
}
