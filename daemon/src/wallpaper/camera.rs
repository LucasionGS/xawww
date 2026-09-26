//! Slow "Ken Burns" pan and zoom, implemented purely through the `wp_viewport` source rectangle.
//!
//! The canvas is rendered `zoom` times larger than the output. At zoom level 1 the whole canvas
//! is shown (downscaled by the compositor), and at the maximum zoom level we show an
//! output-sized region at a 1:1 pixel ratio, so the image is never upscaled and stays sharp.
//!
//! All motion is made of sinusoids with different periods, so it never stops, never has sudden
//! changes in direction, and takes a long time to repeat itself.

use common::ipc::PanZoom;
use rustix::time::Timespec;

use core::f64::consts::TAU;

/// Pan periods relative to the zoom period. Irrational-ish ratios so the path wanders.
const PAN_X_PERIOD: f64 = 0.77;
const PAN_Y_PERIOD: f64 = 1.29;

/// Longest time step we allow. Anything bigger means we were paused or hidden (the compositor
/// stops sending frame callbacks to hidden surfaces), so we just continue from where we were.
const MAX_STEP: f64 = 0.1;

pub struct Camera {
    pan_zoom: PanZoom,
    /// seconds of motion elapsed
    t: f64,
    last: Option<Timespec>,
}

impl Camera {
    pub fn new(pan_zoom: PanZoom) -> Self {
        Self {
            pan_zoom,
            t: 0.0,
            last: None,
        }
    }

    pub fn pan_zoom(&self) -> &PanZoom {
        &self.pan_zoom
    }

    /// Keeps the current position, but changes the parameters of the motion
    pub fn set_pan_zoom(&mut self, pan_zoom: PanZoom) {
        self.pan_zoom = pan_zoom;
    }

    pub fn advance(&mut self, now: Timespec) {
        if let Some(last) = self.last {
            let dt = (now.tv_sec - last.tv_sec) as f64 + (now.tv_nsec - last.tv_nsec) as f64 / 1e9;
            self.t += dt.clamp(0.0, MAX_STEP);
        }
        self.last = Some(now);
    }

    /// Returns the source rectangle `(x, y, width, height)` inside a canvas of the given
    /// dimensions, in units of 1/256 of a pixel (the precision of `wl_fixed`).
    ///
    /// The rectangle is guaranteed to lie within the canvas, since the compositor will kill us
    /// with a protocol error otherwise.
    pub fn source_rect(&self, canvas_width: i32, canvas_height: i32) -> [i32; 4] {
        let period = f64::from(self.pan_zoom.duration.max(1.0));
        let max_zoom = f64::from(self.pan_zoom.zoom.max(1.0));
        let (w, h) = (f64::from(canvas_width), f64::from(canvas_height));

        let phase = TAU * self.t / period;
        let zoom = 1.0 + (max_zoom - 1.0) * 0.5 * (1.0 - phase.cos());
        let (src_w, src_h) = (w / zoom, h / zoom);

        // how far from the center we can go without leaving the canvas
        let (room_x, room_y) = ((w - src_w) * 0.5, (h - src_h) * 0.5);
        let center_x = w * 0.5 + room_x * (phase / PAN_X_PERIOD).sin();
        let center_y = h * 0.5 + room_y * (phase / PAN_Y_PERIOD + 1.0).sin();

        let (max_w, max_h) = (canvas_width * 256, canvas_height * 256);
        let src_w = ((src_w * 256.0) as i32).clamp(256, max_w);
        let src_h = ((src_h * 256.0) as i32).clamp(256, max_h);
        let x = (((center_x * 256.0) as i32) - src_w / 2).clamp(0, max_w - src_w);
        let y = (((center_y * 256.0) as i32) - src_h / 2).clamp(0, max_h - src_h);
        [x, y, src_w, src_h]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(zoom: f32) -> Camera {
        Camera::new(PanZoom {
            zoom,
            duration: 60.0,
            fps: 30,
        })
    }

    #[test]
    fn starts_showing_the_whole_canvas() {
        let cam = camera(1.2);
        assert_eq!(cam.source_rect(2304, 1296), [0, 0, 2304 * 256, 1296 * 256]);
    }

    #[test]
    fn never_leaves_the_canvas_nor_upscales() {
        let (w, h) = (2304, 1296);
        let mut cam = camera(1.2);
        for _ in 0..200_000 {
            cam.t += 0.013;
            let [x, y, sw, sh] = cam.source_rect(w, h);
            assert!(x >= 0 && y >= 0, "{x} {y}");
            assert!(x + sw <= w * 256 && y + sh <= h * 256);
            // at most zoomed in to the output's size: 1920x1080
            assert!(sw >= 1919 * 256 && sh >= 1079 * 256, "{sw} {sh}");
        }
    }

    #[test]
    fn large_time_steps_are_clamped() {
        let mut cam = camera(1.2);
        cam.advance(Timespec {
            tv_sec: 10,
            tv_nsec: 0,
        });
        cam.advance(Timespec {
            tv_sec: 1000,
            tv_nsec: 0,
        });
        assert_eq!(cam.t, MAX_STEP);
    }
}
