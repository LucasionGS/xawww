use common::log::error;
use smallvec::SmallVec;
use waybackend::{Waybackend, objman::ObjectManager};

use rustix::time::Timespec;

use common::{
    compression::Decompressor,
    ipc::{self, BgImg, Effects, ImgReq, Nanos, PixelFormat},
    mmap::MmappedBytes,
};

use crate::{WaylandObject, wallpaper::WallpaperCell};

mod keyframe;
mod transitions;
use transitions::Effect;

pub struct Animator {
    pub wallpapers: SmallVec<[WallpaperCell; 2]>,
    now: Timespec,
    animator: AnimatorKind,
}

enum AnimatorKind {
    Transition(Transition),
    Animation(Animation),
    /// Only moves the viewport and the rain. Never touches the buffers.
    Effects(Nanos),
}

impl Animator {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        backend: &mut Waybackend,
        objman: &mut ObjectManager<WaylandObject>,
        globals: &crate::wallpaper::Globals,
        pixel_format: PixelFormat,
        mut wallpapers: SmallVec<[WallpaperCell; 2]>,
        transition: &ipc::Transition,
        img_req: ImgReq,
        animation: Option<ipc::Animation>,
        effects: Effects,
    ) -> Option<Self> {
        let ImgReq { img, path, dim, .. } = img_req;
        if wallpapers.is_empty() {
            return None;
        }
        // animated images drive their own frames, so we don't combine them with effects
        let effects = if animation.is_none() {
            effects
        } else {
            Effects::default()
        };

        let real_dim = wallpapers[0].borrow().get_dimensions();
        let expect = match &effects.pan_zoom {
            Some(pan_zoom) => pan_zoom.canvas_dim(real_dim),
            None => real_dim,
        };
        if dim != expect {
            error!(
                "image has wrong dimensions! Expect {}x{}, actual {}x{}",
                expect.0, expect.1, dim.0, dim.1
            );
            return None;
        }

        for w in &mut wallpapers {
            let mut w = w.borrow_mut();
            w.set_img_info(BgImg::Img(path.str().into()));
            w.ensure_canvas_dimensions(backend, objman, pixel_format, dim, true);
            w.set_effects(backend, objman, globals, effects);
        }

        let effect = Some(Box::new(Effect::new(transition, dim)));
        Some(Self {
            wallpapers,
            now: crate::clock::get(),
            animator: AnimatorKind::Transition(Transition {
                effect,
                fps_nanos: Nanos::from_nanos(1_000_000_000 / transition.fps.max(1) as u64),
                img,
                animation,
                effects,
            }),
        })
    }

    pub fn time_to_draw(&self) -> Timespec {
        match &self.animator {
            AnimatorKind::Transition(transition) => transition.time_to_draw(&self.now),
            AnimatorKind::Animation(animation) => animation.time_to_draw(&self.now),
            AnimatorKind::Effects(fps_nanos) => {
                let elapsed = crate::clock::get() - self.now;
                timespec_saturating_sub(fps_nanos.into_timespec(), elapsed)
            }
        }
    }

    /// Whether this animator only moves the viewport, never drawing new buffers
    pub fn is_viewport_only(&self) -> bool {
        matches!(self.animator, AnimatorKind::Effects(_))
    }

    pub fn updt_time(&mut self) {
        self.now = crate::clock::get();
    }

    pub fn frame(
        &mut self,
        backend: &mut Waybackend,
        objman: &mut ObjectManager<WaylandObject>,
        pixel_format: PixelFormat,
    ) -> bool {
        let Self {
            wallpapers,
            animator,
            ..
        } = self;
        match animator {
            AnimatorKind::Transition(transition) => {
                if !transition.frame(backend, objman, wallpapers.as_mut_slice(), pixel_format) {
                    return false;
                }
                // Note: it needs to have more than a single frame, otherwise there is no point in
                // animating it
                if let Some(animation) = transition.animation.take()
                    && animation.animation.len() > 1
                {
                    *animator = AnimatorKind::Animation(Animation {
                        animation,
                        decompressor: Decompressor::new(),
                        i: 0,
                    });
                    return false;
                }
                if !transition.effects.is_empty() {
                    *animator = AnimatorKind::Effects(Nanos::from_nanos(
                        1_000_000_000 / transition.effects.fps() as u64,
                    ));
                    return false;
                }
                true
            }
            AnimatorKind::Animation(animation) => {
                animation.frame(backend, objman, wallpapers, pixel_format);
                false
            }
            AnimatorKind::Effects(_) => false,
        }
    }
}

struct Transition {
    fps_nanos: Nanos,
    effect: Option<Box<Effect>>,
    img: MmappedBytes,
    animation: Option<ipc::Animation>,
    effects: Effects,
}

impl Transition {
    fn time_to_draw(&self, start: &Timespec) -> Timespec {
        let now = crate::clock::get();
        let elapsed = now - *start;
        timespec_saturating_sub(self.fps_nanos.into_timespec(), elapsed)
    }

    fn frame(
        &mut self,
        backend: &mut Waybackend,
        objman: &mut ObjectManager<WaylandObject>,
        wallpapers: &mut [WallpaperCell],
        pixel_format: PixelFormat,
    ) -> bool {
        let Self { effect, img, .. } = self;
        match effect.as_mut() {
            Some(e) => {
                let over = e.execute(backend, objman, pixel_format, wallpapers, img.bytes());
                if over {
                    *effect = None;
                }
                false
            }
            None => true,
        }
    }
}

struct Animation {
    animation: ipc::Animation,
    decompressor: Decompressor,
    i: usize,
}

impl Animation {
    fn time_to_draw(&self, start: &Timespec) -> Timespec {
        let now = crate::clock::get();
        let elapsed = now - *start;
        timespec_saturating_sub(
            self.animation.animation[self.i % self.animation.animation.len()]
                .1
                .into_timespec(),
            elapsed,
        )
    }

    fn frame(
        &mut self,
        backend: &mut Waybackend,
        objman: &mut ObjectManager<WaylandObject>,
        wallpapers: &mut SmallVec<[WallpaperCell; 2]>,
        pixel_format: PixelFormat,
    ) {
        let Self {
            animation,
            decompressor,
            i,
            ..
        } = self;

        let frame = &animation.animation[*i % animation.animation.len()].0;

        if *i < animation.animation.len() {
            wallpapers.retain(|w| {
                let mut borrow = w.borrow_mut();
                let result = borrow.canvas_change(backend, objman, pixel_format, |canvas| {
                    decompressor.decompress(frame, canvas, pixel_format)
                });
                match result {
                    Ok(()) => true,
                    Err(e) => {
                        error!("failed to unpack frame: {e}");
                        false
                    }
                }
            });
        } else {
            // if we already went through one loop, we can use the unsafe version, because
            // everything was already validated
            for w in wallpapers {
                let mut borrow = w.borrow_mut();
                // SAFETY: we have already validated every frame and removed the ones that have
                // errors in the previous loops. The only ones left should be those that can be
                // decompressed correctly
                borrow.canvas_change(backend, objman, pixel_format, |canvas| unsafe {
                    decompressor.decompress_unchecked(frame, canvas, pixel_format);
                });
            }
        }

        *i += 1;
    }
}

/// inspired by the std Duration implementation
fn timespec_saturating_sub(a: Timespec, b: Timespec) -> Timespec {
    let mut res = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    if a.tv_sec >= b.tv_sec {
        let mut secs = a.tv_sec - b.tv_sec;
        let nanos = if a.tv_nsec >= b.tv_nsec {
            a.tv_nsec - b.tv_nsec
        } else if secs > 0 {
            secs -= 1;
            a.tv_nsec + 1_000_000_000 - b.tv_nsec
        } else {
            return res;
        };
        res.tv_sec = secs;
        res.tv_nsec = nanos;
    }

    res
}
