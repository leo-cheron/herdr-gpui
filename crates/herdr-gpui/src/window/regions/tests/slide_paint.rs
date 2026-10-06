use super::*;
use crate::{
    smooth_scroll::Slide,
    terminal_painter::TerminalPainter,
    window::regions::{Region, RegionView},
};
use gpui::{AppContext, Empty, IntoElement, ParentElement, Point, Styled, px, size};
use std::{cell::RefCell, rc::Rc};

/// A sliding pane paints its border at rest, its rows from the current frame
/// offset inside the border, and only the uncovered band from the frames
/// behind: each cell once, with one cursor.
#[gpui::test]
fn a_sliding_region_fills_only_its_uncovered_edge_from_earlier_frames(cx: &mut TestAppContext) {
    let grid = |letter: &str| {
        let row = letter.repeat(4);
        let mut frame = frame(&[row.as_str(); 6]);
        frame.cursor = Some(CursorState {
            x: 1,
            y: 2,
            visible: true,
            shape: 0,
        });
        frame
    };
    let surface = |letter: &str| {
        let mut pane = pane("p", rect(0, 0, 4, 6));
        pane.inner_rect = rect(0, 1, 4, 5);
        Arc::new(PaneSurfaceFrame {
            boot_id: "boot".into(),
            projection_revision: 1,
            surface_revision: 1,
            frame: grid(letter),
            panes: vec![pane],
            splits: vec![],
            popup: None,
            graphics: Default::default(),
        })
    };
    let (previous, current) = (surface("a"), surface("b"));
    let (_, cx) = cx.add_window_view(|_, _| Empty);
    // Bands of 2 then 1 rows from two earlier frames; then one reversed.
    for (behind, band) in [(vec![2, 3], 3), (vec![-1], 1)] {
        let region = Rc::new(Region {
            owner: Owner::Pane("p".into()),
            surface: current.clone(),
            area: partition(&current.frame, &current.panes).remove(0).1,
            highlights: vec![],
            look: look(),
            slide: Some(Slide {
                pane_id: "p".into(),
                rect: current.panes[0].inner_rect,
                offset: -behind[behind.len() - 1] as f32 / 2.,
                behind: behind
                    .iter()
                    .map(|shift| (previous.clone(), *shift))
                    .collect(),
            }),
        });
        let painter = Rc::new(RefCell::new(TerminalPainter::default()));
        let before = cx.update(|_, cx| *cx.default_global::<crate::performance::Counts>());
        cx.draw(Point::default(), size(px(800.), px(600.)), |_, cx| {
            let layers = Layer::ALL.map(|layer| {
                let (painter, region) = (painter.clone(), region.clone());
                cx.new(|_| RegionView {
                    painter,
                    region,
                    layer,
                })
            });
            gpui::div()
                .size_full()
                .children(layers.map(|view| view.into_any_element()))
        });
        let after = cx.update(|_, cx| *cx.default_global::<crate::performance::Counts>());
        // The row above the pane's content, its five rows, and the band.
        assert_eq!(after.glyphs - before.glyphs, (1 + 5 + band) * 4);
        assert_eq!(after.decorations - before.decorations, 1, "one cursor");
    }
}
