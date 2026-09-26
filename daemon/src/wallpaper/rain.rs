//! Rain falling over the wallpaper.
//!
//! Drawing thousands of moving streaks on the CPU would mean re-uploading the whole screen every
//! frame. Instead, when rain starts we render a few *layers* of rain streaks once. Far layers have
//! many faint, thin, short and slow streaks; near layers have few bright, long and fast ones,
//! which gives the rain depth.
//!
//! Each layer's texture repeats seamlessly with a period of one *tile*, and holds 2x2 tiles. The
//! screen is covered by a fixed grid of subsurfaces, one per tile, whose positions and sizes never
//! change. To make the rain fall, every frame we only slide their viewport source rectangles
//! across the texture. After setup, a frame is just a handful of tiny protocol messages: no pixels
//! are ever uploaded again, and the compositor does all the blending on the GPU.
//!
//! Note: we deliberately never move or resize the subsurfaces after creating them. Changing
//! positions, sizes and source rectangles together every frame showed glitches in practice (the
//! compositor would briefly combine a new source rectangle with an old size, stretching a sliver
//! of rain across the screen).
//!
//! To save memory, textures are rendered at full horizontal resolution but compressed vertically,
//! and the compositor stretches them back. Rain streaks are mostly vertical and motion blurred,
//! so this is practically invisible.

use common::{
    ipc::Rain,
    log::{debug, error},
    mmap::Mmap,
};
use rustix::time::Timespec;
use waybackend::{
    Waybackend,
    objman::ObjectManager,
    types::{ObjectId, WlFixed},
};

use crate::{
    WaylandObject,
    wayland::{
        wl_buffer, wl_compositor, wl_region, wl_shm, wl_shm_pool, wl_subcompositor, wl_subsurface,
        wl_surface, wp_viewport, wp_viewporter,
    },
};

/// Wayland globals needed to build the overlay
#[derive(Clone, Copy)]
pub struct Globals {
    pub compositor: ObjectId,
    pub subcompositor: Option<ObjectId>,
    pub shm: ObjectId,
    pub viewporter: ObjectId,
}

struct LayerSpec {
    /// streaks per logical megapixel
    density: f32,
    /// logical pixels
    length: (f32, f32),
    /// logical pixels
    width: f32,
    /// width of the antialiased edge, in physical pixels
    softness: f32,
    alpha: (f32, f32),
    /// logical pixels per second
    speed: f32,
    /// how many tiles cover the screen, horizontally and vertically. The rain repeats once per
    /// tile, so different layers use different grids to hide the repetition
    grid: (i32, i32),
    /// how much the texture is compressed vertically
    stretch: i32,
}

/// From far to near
const LAYERS: [LayerSpec; 3] = [
    LayerSpec {
        density: 900.0,
        length: (14.0, 30.0),
        width: 0.9,
        softness: 1.0,
        alpha: (0.10, 0.22),
        speed: 650.0,
        grid: (3, 3),
        stretch: 2,
    },
    LayerSpec {
        density: 260.0,
        length: (30.0, 62.0),
        width: 1.2,
        softness: 1.0,
        alpha: (0.16, 0.32),
        speed: 1050.0,
        grid: (2, 2),
        stretch: 2,
    },
    LayerSpec {
        density: 55.0,
        length: (60.0, 125.0),
        width: 1.8,
        softness: 1.6,
        alpha: (0.22, 0.42),
        speed: 1600.0,
        grid: (2, 1),
        stretch: 2,
    },
];

/// Color of the streaks, slightly blue
const STREAK_COLOR: [f32; 3] = [0.83, 0.88, 0.96];
/// Color of the dimming overlay, a dark blueish gray, like an overcast sky
const DIM_COLOR: [f32; 3] = [0.05, 0.06, 0.09];

/// Texels of periodic padding around each texture, so bilinear filtering at the edges of a
/// source rectangle samples the right neighbors
const MARGIN: i32 = 2;

/// Longest time step we allow, see [super::camera]
const MAX_STEP: f64 = 0.1;

/// A small xorshift generator. The rain does not need anything better, and this keeps the
/// daemon free of extra dependencies.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, (min, max): (f32, f32)) -> f32 {
        min + (max - min) * self.next_f32()
    }
}

/// One subsurface. Its position and size are fixed; only its source rectangle ever changes.
struct Quad {
    surface: ObjectId,
    subsurface: ObjectId,
    viewport: ObjectId,
    /// logical pixels, relative to the wallpaper
    position: (i32, i32),
    size: (i32, i32),
    /// last source rectangle we sent, in 1/256ths of a texel
    source: Option<[i32; 4]>,
}

impl Quad {
    #[allow(clippy::too_many_arguments)]
    fn new(
        backend: &mut Waybackend,
        objman: &mut ObjectManager<WaylandObject>,
        globals: &Globals,
        subcompositor: ObjectId,
        parent: ObjectId,
        empty_region: ObjectId,
        buffer: ObjectId,
        position: (i32, i32),
        size: (i32, i32),
    ) -> Self {
        let surface = objman.create(WaylandObject::Surface);
        wl_compositor::req::create_surface(backend, globals.compositor, surface).unwrap();
        // wallpapers must never take input
        wl_surface::req::set_input_region(backend, surface, Some(empty_region)).unwrap();

        let subsurface = objman.create(WaylandObject::Subsurface);
        wl_subcompositor::req::get_subsurface(backend, subcompositor, subsurface, surface, parent)
            .unwrap();
        wl_subsurface::req::set_position(backend, subsurface, position.0, position.1).unwrap();

        let viewport = objman.create(WaylandObject::Viewport);
        wp_viewporter::req::get_viewport(backend, globals.viewporter, viewport, surface).unwrap();
        wp_viewport::req::set_destination(backend, viewport, size.0, size.1).unwrap();

        wl_surface::req::attach(backend, surface, Some(buffer), 0, 0).unwrap();
        Self {
            surface,
            subsurface,
            viewport,
            position,
            size,
            source: None,
        }
    }

    /// Sends the new source rectangle, if it changed. Like every subsurface in synchronized
    /// mode, it only takes effect when the parent surface is committed.
    fn set_source(&mut self, backend: &mut Waybackend, source: [i32; 4]) {
        if self.source == Some(source) {
            return;
        }
        let fixed = |v: i32| WlFixed::from(f64::from(v) / 256.0);
        let [x, y, w, h] = source;
        wp_viewport::req::set_source(
            backend,
            self.viewport,
            fixed(x),
            fixed(y),
            fixed(w),
            fixed(h),
        )
        .unwrap();
        wl_surface::req::damage(backend, self.surface, 0, 0, self.size.0, self.size.1).unwrap();
        wl_surface::req::commit(backend, self.surface).unwrap();
        self.source = Some(source);
    }

    fn destroy(self, backend: &mut Waybackend) {
        wp_viewport::req::destroy(backend, self.viewport).unwrap();
        wl_subsurface::req::destroy(backend, self.subsurface).unwrap();
        wl_surface::req::destroy(backend, self.surface).unwrap();
    }
}

/// Where a layer's tile geometry lives in its texture
#[derive(Clone, Copy)]
struct Tiling {
    /// size of one tile, in logical pixels
    period: (i32, i32),
    /// how many texels one logical pixel covers, horizontally and vertically
    texel_scale: (f64, f64),
}

impl Tiling {
    /// Source rectangle, in 1/256ths of a texel, for a quad at `position` with `size`, when the
    /// rain has moved by `offset` logical pixels
    fn source(&self, position: (i32, i32), size: (i32, i32), offset: (f64, f64)) -> [i32; 4] {
        let (period_x, period_y) = (f64::from(self.period.0), f64::from(self.period.1));
        let texel = |v: f64, scale: f64| (v * scale * 256.0).round() as i32;
        // the screen pixel at `x` shows the rain from `x - offset`
        let content_x = (f64::from(position.0) - offset.0).rem_euclid(period_x);
        let content_y = (f64::from(position.1) - offset.1).rem_euclid(period_y);
        [
            MARGIN * 256 + texel(content_x, self.texel_scale.0),
            MARGIN * 256 + texel(content_y, self.texel_scale.1),
            texel(f64::from(size.0), self.texel_scale.0),
            texel(f64::from(size.1), self.texel_scale.1),
        ]
    }
}

struct Layer {
    buffer: ObjectId,
    tiling: Tiling,
    /// logical pixels per second, along the direction of the rain
    speed: f64,
    /// how far the rain has moved, in logical pixels, wrapped to one period
    offset: (f64, f64),
    quads: Vec<Quad>,
}

impl Layer {
    /// Slides every quad's source rectangle so the texture appears shifted by `offset`
    fn update(&mut self, backend: &mut Waybackend) {
        for quad in &mut self.quads {
            let source = self.tiling.source(quad.position, quad.size, self.offset);
            quad.set_source(backend, source);
        }
    }
}

pub struct RainOverlay {
    rain: Rain,
    /// logical dimensions
    screen: (i32, i32),
    /// physical dimensions
    physical: (i32, i32),
    /// direction the rain falls in, as a unit vector
    direction: (f64, f64),
    layers: Vec<Layer>,
    dim: Option<(ObjectId, Quad)>,
    t: f64,
    last: Option<Timespec>,
}

impl RainOverlay {
    /// `screen` is the output's logical size, and `physical` its size in pixels
    pub fn new(
        backend: &mut Waybackend,
        objman: &mut ObjectManager<WaylandObject>,
        globals: &Globals,
        parent: ObjectId,
        rain: Rain,
        screen: (i32, i32),
        physical: (i32, i32),
    ) -> Option<Self> {
        let Some(subcompositor) = globals.subcompositor else {
            error!("the compositor does not support subsurfaces, so we cannot draw rain");
            return None;
        };

        let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
        let mut rng = Rng::new(
            now.tv_nsec as u64 ^ ((now.tv_sec as u64) << 32) ^ u64::from(parent.get().get()),
        );

        let angle = f64::from(rain.angle.clamp(-60.0, 60.0)).to_radians();
        let direction = (angle.sin(), angle.cos());

        // render every texture into a single shared memory pool
        let textures: Vec<Texture> = LAYERS
            .iter()
            .map(|spec| render_layer(spec, &rain, direction, screen, physical, &mut rng))
            .collect();
        let dim = (rain.dim > 0.0).then(|| {
            let a = rain.dim.clamp(0.0, 1.0);
            let [r, g, b] = DIM_COLOR.map(|c| (c * a * 255.0).round() as u8);
            [b, g, r, (a * 255.0).round() as u8]
        });

        let len = textures.iter().map(|t| t.pixels.len()).sum::<usize>() + 4;
        let mut mmap = match Mmap::create(len) {
            Ok(mmap) => mmap,
            Err(e) => {
                error!("failed to create memory for the rain: {e}");
                return None;
            }
        };
        let pool = objman.create(WaylandObject::ShmPool);
        wl_shm::req::create_pool(backend, globals.shm, pool, &mmap.fd(), len as i32).unwrap();

        let mut offset = 0;
        let mut buffers = Vec::with_capacity(textures.len());
        for Texture {
            pixels, buffer_dim, ..
        } in &textures
        {
            mmap.slice_mut()[offset..offset + pixels.len()].copy_from_slice(pixels);
            let buffer = objman.create(WaylandObject::Buffer);
            wl_shm_pool::req::create_buffer(
                backend,
                pool,
                buffer,
                offset as i32,
                buffer_dim.0,
                buffer_dim.1,
                buffer_dim.0 * 4,
                wl_shm::Format::argb8888,
            )
            .unwrap();
            buffers.push(buffer);
            offset += pixels.len();
        }
        let dim_buffer = dim.map(|pixel| {
            mmap.slice_mut()[offset..offset + 4].copy_from_slice(&pixel);
            let buffer = objman.create(WaylandObject::Buffer);
            wl_shm_pool::req::create_buffer(
                backend,
                pool,
                buffer,
                offset as i32,
                1,
                1,
                4,
                wl_shm::Format::argb8888,
            )
            .unwrap();
            buffer
        });
        wl_shm_pool::req::destroy(backend, pool).unwrap();

        let empty_region = objman.create(WaylandObject::Region);
        wl_compositor::req::create_region(backend, globals.compositor, empty_region).unwrap();

        // new subsurfaces are stacked on top, so we create them from the bottom up
        let dim = dim_buffer.map(|buffer| {
            let mut quad = Quad::new(
                backend,
                objman,
                globals,
                subcompositor,
                parent,
                empty_region,
                buffer,
                (0, 0),
                screen,
            );
            quad.set_source(backend, [0, 0, 256, 256]);
            (buffer, quad)
        });

        let layers = LAYERS
            .iter()
            .zip(textures)
            .zip(buffers)
            .map(|((spec, texture), buffer)| {
                let period = texture.period;
                // fixed grid of quads covering the screen, clipped at its right and bottom edges
                let mut quads = Vec::new();
                for y in (0..screen.1).step_by(period.1 as usize) {
                    for x in (0..screen.0).step_by(period.0 as usize) {
                        let size = (period.0.min(screen.0 - x), period.1.min(screen.1 - y));
                        quads.push(Quad::new(
                            backend,
                            objman,
                            globals,
                            subcompositor,
                            parent,
                            empty_region,
                            buffer,
                            (x, y),
                            size,
                        ));
                    }
                }
                let mut layer = Layer {
                    buffer,
                    tiling: texture.tiling(),
                    speed: f64::from(spec.speed * rain.speed.max(0.05)),
                    offset: (
                        f64::from(rng.next_f32()) * f64::from(period.0),
                        f64::from(rng.next_f32()) * f64::from(period.1),
                    ),
                    quads,
                };
                layer.update(backend);
                layer
            })
            .collect();
        wl_region::req::destroy(backend, empty_region).unwrap();

        // Send everything now, so we can drop our side of the shared memory: the compositor
        // keeps its own mapping, and we never touch these pixels again
        if let Err(e) = backend.flush() {
            error!("failed to send the rain to the compositor: {e}");
        }
        drop(mmap);

        debug!(
            "created rain overlay: {}x{} ({}x{} physical), {}Kb of textures",
            screen.0,
            screen.1,
            physical.0,
            physical.1,
            len >> 10
        );
        Some(Self {
            rain,
            screen,
            physical,
            direction,
            layers,
            dim,
            t: 0.0,
            last: None,
        })
    }

    /// Whether this overlay can keep being used for the given parameters
    pub fn matches(&self, rain: &Rain, screen: (i32, i32), physical: (i32, i32)) -> bool {
        self.rain == *rain && self.screen == screen && self.physical == physical
    }

    pub fn has_buffer(&self, buffer: ObjectId) -> bool {
        self.layers.iter().any(|l| l.buffer == buffer)
            || self.dim.as_ref().is_some_and(|(b, _)| *b == buffer)
    }

    /// Moves the rain. Takes effect on the parent surface's next commit.
    pub fn frame(&mut self, backend: &mut Waybackend, now: Timespec) {
        let dt = match self.last {
            Some(last) => ((now.tv_sec - last.tv_sec) as f64
                + (now.tv_nsec - last.tv_nsec) as f64 / 1e9)
                .clamp(0.0, MAX_STEP),
            None => 0.0,
        };
        self.last = Some(now);
        self.t += dt;

        // gentle gusts, so the rain does not look mechanical
        let t = self.t;
        let gust = 1.0 + 0.08 * (0.9 * t).sin() * (0.31 * t + 1.3).sin();
        for layer in &mut self.layers {
            let distance = layer.speed * gust * dt;
            let period = layer.tiling.period;
            layer.offset = (
                (layer.offset.0 + distance * self.direction.0).rem_euclid(f64::from(period.0)),
                (layer.offset.1 + distance * self.direction.1).rem_euclid(f64::from(period.1)),
            );
            layer.update(backend);
        }
    }

    pub fn destroy(self, backend: &mut Waybackend) {
        for layer in self.layers {
            for quad in layer.quads {
                quad.destroy(backend);
            }
            wl_buffer::req::destroy(backend, layer.buffer).unwrap();
        }
        if let Some((buffer, quad)) = self.dim {
            quad.destroy(backend);
            wl_buffer::req::destroy(backend, buffer).unwrap();
        }
    }
}

/// A rendered layer, in premultiplied ARGB8888
struct Texture {
    pixels: Vec<u8>,
    /// including the margins
    buffer_dim: (i32, i32),
    /// size of one tile, in logical pixels
    period: (i32, i32),
    /// size of one tile, in texels. The texture holds 2x2 tiles, plus the margins
    texel_period: (i32, i32),
}

impl Texture {
    fn tiling(&self) -> Tiling {
        Tiling {
            period: self.period,
            texel_scale: (
                f64::from(self.texel_period.0) / f64::from(self.period.0),
                f64::from(self.texel_period.1) / f64::from(self.period.1),
            ),
        }
    }
}

/// Renders one layer's repeating texture
fn render_layer(
    spec: &LayerSpec,
    rain: &Rain,
    direction: (f64, f64),
    screen: (i32, i32),
    physical: (i32, i32),
    rng: &mut Rng,
) -> Texture {
    // one tile, in logical pixels
    let period = (
        (screen.0 + spec.grid.0 - 1) / spec.grid.0,
        (screen.1 + spec.grid.1 - 1) / spec.grid.1,
    );
    // We draw in "display space": physical pixels, as they will look after the compositor
    // stretches the texture back. One texel column is one display column, and one texel row
    // covers `stretch_y` display rows.
    let scale = (
        physical.0 as f32 / screen.0 as f32,
        physical.1 as f32 / screen.1 as f32,
    );
    let texel_period = (
        (period.0 as f32 * scale.0).round().max(1.0) as i32,
        (period.1 as f32 * scale.1 / spec.stretch as f32)
            .round()
            .max(1.0) as i32,
    );
    let display_period = (texel_period.0 as f32, period.1 as f32 * scale.1);
    let stretch_y = display_period.1 / texel_period.1 as f32;

    let extent = (2 * texel_period.0, 2 * texel_period.1);
    let buffer_dim = (extent.0 + 2 * MARGIN, extent.1 + 2 * MARGIN);
    let mut pixels = vec![0u8; buffer_dim.0 as usize * buffer_dim.1 as usize * 4];

    let area = period.0 as f32 * period.1 as f32 / 1e6;
    let count = (spec.density * area * rain.intensity.max(0.0)).round() as usize;
    // keep the motion blur consistent with the speed
    let length_mul = scale.1 * rain.speed.clamp(0.25, 4.0);
    let half_width = spec.width * scale.0 * 0.5;

    for _ in 0..count {
        let head = (
            rng.next_f32() * display_period.0,
            rng.next_f32() * display_period.1,
        );
        let length = rng.range(spec.length) * length_mul;
        let alpha = rng.range(spec.alpha);
        // a tiny bit of variation in the angle looks more natural
        let jitter = (rng.next_f32() - 0.5) * 0.05;
        let dir = (
            direction.0 as f32 + jitter * direction.1 as f32,
            direction.1 as f32 - jitter * direction.0 as f32,
        );

        // draw a copy of the streak in every tile it touches, including the margins
        for copy_x in -1..=2 {
            for copy_y in -1..=2 {
                let head = (
                    head.0 + copy_x as f32 * display_period.0,
                    head.1 + copy_y as f32 * display_period.1,
                );
                let tail = (head.0 - dir.0 * length, head.1 - dir.1 * length);
                let streak = Streak {
                    tail,
                    dir,
                    length,
                    half_width,
                    softness: spec.softness,
                    alpha,
                };
                streak.draw(&mut pixels, buffer_dim, extent, stretch_y);
            }
        }
    }

    Texture {
        pixels,
        buffer_dim,
        period,
        texel_period,
    }
}

struct Streak {
    tail: (f32, f32),
    dir: (f32, f32),
    length: f32,
    half_width: f32,
    softness: f32,
    alpha: f32,
}

impl Streak {
    /// Draws into the texture region `[-MARGIN, extent + MARGIN)`, in texels
    fn draw(&self, pixels: &mut [u8], buffer_dim: (i32, i32), extent: (i32, i32), stretch_y: f32) {
        let head = (
            self.tail.0 + self.dir.0 * self.length,
            self.tail.1 + self.dir.1 * self.length,
        );
        let pad = self.half_width + self.softness;
        let (min_x, max_x) = (self.tail.0.min(head.0) - pad, self.tail.0.max(head.0) + pad);
        let (min_y, max_y) = (self.tail.1.min(head.1) - pad, self.tail.1.max(head.1) + pad);

        let col_start = (min_x.floor() as i32).max(-MARGIN);
        let col_end = (max_x.ceil() as i32).min(extent.0 + MARGIN);
        let row_start = ((min_y / stretch_y).floor() as i32).max(-MARGIN);
        let row_end = ((max_y / stretch_y).ceil() as i32).min(extent.1 + MARGIN);
        if col_start >= col_end || row_start >= row_end {
            return;
        }

        let [cr, cg, cb] = STREAK_COLOR;
        let stride = buffer_dim.0 as usize * 4;
        for row in row_start..row_end {
            let y = (row as f32 + 0.5) * stretch_y - self.tail.1;
            let line = (row + MARGIN) as usize * stride;
            for col in col_start..col_end {
                let x = col as f32 + 0.5 - self.tail.0;
                // project onto the streak
                let along = (x * self.dir.0 + y * self.dir.1).clamp(0.0, self.length);
                let (dx, dy) = (x - self.dir.0 * along, y - self.dir.1 * along);
                let distance = (dx * dx + dy * dy).sqrt();
                let coverage = ((self.half_width - distance) / self.softness + 0.5).clamp(0.0, 1.0);
                if coverage <= 0.0 {
                    continue;
                }
                // bright at the head, fading towards the tail
                let t = along / self.length;
                let a = self.alpha * coverage * t * t.sqrt();

                // premultiplied "over", in BGRA byte order
                let i = line + (col + MARGIN) as usize * 4;
                let px = &mut pixels[i..i + 4];
                let keep = 1.0 - a;
                let blend = |dst: u8, src: f32| (src * 255.0 + f32::from(dst) * keep).round() as u8;
                px[0] = blend(px[0], cb * a);
                px[1] = blend(px[1], cg * a);
                px[2] = blend(px[2], cr * a);
                px[3] = blend(px[3], a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture(screen: (i32, i32), layer: usize) -> Texture {
        let rain = Rain {
            intensity: 3.0,
            speed: 1.0,
            angle: 10.0,
            dim: 0.0,
            fps: 30,
        };
        let angle = 10f64.to_radians();
        render_layer(
            &LAYERS[layer],
            &rain,
            (angle.sin(), angle.cos()),
            screen,
            screen,
            &mut Rng::new(42),
        )
    }

    fn alpha(t: &Texture, col: i32, row: i32) -> u8 {
        t.pixels[((row + MARGIN) * t.buffer_dim.0 + col + MARGIN) as usize * 4 + 3]
    }

    #[test]
    fn texture_repeats_with_its_period() {
        for layer in 0..LAYERS.len() {
            let t = texture((640, 480), layer);
            let (px, py) = t.texel_period;
            assert_eq!(t.buffer_dim, (2 * px + 2 * MARGIN, 2 * py + 2 * MARGIN));
            assert!(
                t.pixels.chunks_exact(4).any(|p| p[3] > 0),
                "no rain was drawn"
            );
            // every texel, margins included, matches the one a period away
            let mut mismatches = 0;
            for row in -MARGIN..py + MARGIN {
                for col in -MARGIN..px + MARGIN {
                    let a = alpha(&t, col, row);
                    for (c, r) in [(col + px, row), (col, row + py), (col + px, row + py)] {
                        if a.abs_diff(alpha(&t, c, r)) > 1 {
                            mismatches += 1;
                        }
                    }
                }
            }
            assert_eq!(mismatches, 0, "layer {layer}");
        }
    }

    #[test]
    fn source_rectangles_stay_inside_the_texture() {
        for layer in 0..LAYERS.len() {
            for screen in [(1920, 1080), (1080, 1920), (2560, 1440), (1366, 768)] {
                let t = texture(screen, layer);
                let tiling = t.tiling();
                let (px, py) = tiling.period;
                for step in 0..2000 {
                    let offset = (
                        (f64::from(step) * 7.31).rem_euclid(f64::from(px)),
                        (f64::from(step) * 53.7).rem_euclid(f64::from(py)),
                    );
                    for (x, y) in [(0, 0), (px, 0), (0, py), (2 * px, 2 * py)] {
                        let [sx, sy, sw, sh] = tiling.source((x, y), (px, py), offset);
                        assert!(sx >= 0 && sy >= 0);
                        assert!(
                            sx + sw <= t.buffer_dim.0 * 256 && sy + sh <= t.buffer_dim.1 * 256,
                            "layer {layer} screen {screen:?} step {step}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn premultiplied() {
        let t = texture((320, 240), 2);
        for px in t.pixels.chunks_exact(4) {
            assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3], "{px:?}");
        }
    }
}
