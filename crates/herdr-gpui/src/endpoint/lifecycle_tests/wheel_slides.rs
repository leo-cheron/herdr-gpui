use super::*;
use gpui::{ScrollDelta, ScrollWheelEvent, TouchPhase};
use herdr_client::protocol::{
    PaneSurfaceScrollMetrics, SurfaceGraphicsAssetKey, SurfaceGraphicsFormat,
    SurfaceGraphicsPlacement, SurfaceGraphicsSource, SurfaceGraphicsTarget,
};

/// A quarter row's trackpad delta follows the OS at once while the pane can
/// slide, but accumulates into whole lines while an image is on screen,
/// since images never slide: each delta must not become a line of its own.
#[gpui::test]
fn wheel_deltas_accumulate_into_lines_while_images_hold_the_grid(cx: &mut gpui::TestAppContext) {
    let (fixture, cx) = cx.add_window_view(|window, cx| {
        Fixture(cx.new(|cx| crate::sidebar::layout_tests::fixture_window(window, cx)))
    });
    let view = fixture.update(cx, |fixture, _| fixture.0.clone());
    for images in [false, true] {
        let (endpoint, mut server) = connected_endpoint("ssh:wheel");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                prepare_mouse(view, endpoint, cx);
                let surface = Arc::make_mut(view.live.surface.as_mut().unwrap());
                let pane = &mut surface.panes[0];
                pane.mouse_reporting = false;
                pane.scroll = Some(PaneSurfaceScrollMetrics {
                    offset_from_bottom: 0,
                    max_offset_from_bottom: 1000,
                    viewport_rows: 22,
                });
                if images {
                    surface.graphics.placements = vec![placement()];
                }
                view.presentation.frame(&view.live);
                let quarter = view.config.terminal.line_height() / 4.;
                view.scroll_wheel(
                    &ScrollWheelEvent {
                        position: mouse_position(view, 3.5, 4.5),
                        delta: ScrollDelta::Pixels(point(px(0.), px(quarter))),
                        touch_phase: TouchPhase::Moved,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                view.send(ClientPaneInputEvent::TextCommit("sentinel".into()), cx);
            });
        });
        let ClientMessage::ClientShellPaneInput { events, .. } = server.receive() else {
            panic!("missing input");
        };
        let sentinel = events == [ClientPaneInputEvent::TextCommit("sentinel".into())];
        assert_eq!(sentinel, images, "images={images}: {events:?}");
    }
}

fn placement() -> SurfaceGraphicsPlacement {
    SurfaceGraphicsPlacement {
        asset: SurfaceGraphicsAssetKey {
            source: SurfaceGraphicsSource::Terminal {
                target: SurfaceGraphicsTarget::Pane {
                    pane_id: "w1:p1".into(),
                },
                image_id: 1,
            },
            image_width: 1,
            image_height: 1,
            format: SurfaceGraphicsFormat::Rgba,
            data_len: 4,
            data_fingerprint: 1,
        },
        logical_placement_id: 1,
        x: 0,
        y: 0,
        cols: 1,
        rows: 1,
        source_x: 0,
        source_y: 0,
        source_width: 0,
        source_height: 0,
        x_offset: 0,
        y_offset: 0,
        z: -1,
        scrollback_offset: 0,
    }
}
