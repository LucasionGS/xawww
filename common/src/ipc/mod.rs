use ::alloc::boxed::Box;
use ::alloc::string::String;

use alloc::string::ToString;
use alloc::vec::Vec;
use rustix::fs::{Mode, OFlags};
use rustix::io;

use transmit::RawMsg;

mod error;
mod socket;
mod transmit;
mod types;

use crate::cache::{self, read_cache_file};
use crate::cache::{cache_dir, get_previous_image_cache};
use crate::log;
use crate::mmap::Mmap;
pub use error::*;
pub use socket::*;
pub use types::*;

pub struct ImageRequestBuilder {
    memory: Mmap,
    len: usize,
    img_count: u8,
    img_count_index: usize,
}

impl ImageRequestBuilder {
    #[inline]
    pub fn new(transition: Transition) -> io::Result<Self> {
        let memory = Mmap::create(1 << (20 + 3))?; // start with 8 MB
        let len = 0;
        let mut builder = Self {
            memory,
            len,
            img_count: 0,
            img_count_index: 0,
        };
        transition.serialize(&mut builder);
        builder.img_count_index = builder.len;
        builder.len += 1;
        assert_eq!(builder.len, 52);
        Ok(builder)
    }

    fn push_byte(&mut self, byte: u8) {
        if self.len >= self.memory.len() {
            self.grow();
        }
        self.memory.slice_mut()[self.len] = byte;
        self.len += 1;
    }

    pub(crate) fn extend(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() >= self.memory.len() {
            self.memory.remap(self.memory.len() + bytes.len() * 2);
        }
        self.memory.slice_mut()[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    fn grow(&mut self) {
        self.memory.remap((self.memory.len() * 3) / 2);
    }

    #[inline]
    pub fn push(
        &mut self,
        img: ImgSend,
        namespace: &str,
        use_cache: bool,
        resize: &str,
        crop_gravity: Option<&str>,
        filter: &str,
        outputs: &[String],
        animation: Option<Animation>,
    ) {
        self.img_count += 1;

        let ImgSend {
            path,
            img,
            dim: dims,
            format,
        } = &img;
        self.serialize_bytes(path.as_bytes());
        self.serialize_bytes(img);
        self.extend(&dims.0.to_ne_bytes());
        self.extend(&dims.1.to_ne_bytes());
        self.push_byte(*format as u8);

        self.push_byte(outputs.len() as u8);
        for output in outputs {
            self.serialize_bytes(output.as_bytes());
        }

        let animation_start = self.len + 1;
        if let Some(animation) = animation.as_ref() {
            self.push_byte(1);
            animation.serialize(self);
        } else {
            self.push_byte(0);
        }

        if use_cache {
            // cache the request
            for output in outputs {
                if let Err(e) =
                    super::cache::CacheEntry::new(namespace, resize, crop_gravity, filter, path)
                        .store(output)
                {
                    log::error!("failed to store cache: {e}");
                }
            }
        }

        if animation.is_some() && path != "-" {
            let animation = &self.memory.slice()[animation_start..];
            let mut buf = crate::path::PathBuf::new();
            buf.append_str(path.as_str());
            let path = buf.as_path();
            if let Err(e) = cache::store_animation_frames(animation, path, *dims, resize, *format) {
                log::error!("failed storing cache for {}: {e}", path.display());
            }
        }
    }

    /// This function is called only when no output is specified (using `--outputs ...`)
    #[inline]
    pub fn update_disconnected_caches(
        &self,
        img_path: String,
        namespace: &str,
        outputs: &[Vec<String>],
    ) {
        let outputs: Vec<&String> = outputs.iter().flatten().collect();

        if let Ok(cache_dir) = cache_dir()
            && let Ok(fd) = rustix::fs::open(cache_dir, OFlags::DIRECTORY, Mode::empty())
            && let Ok(dir) = rustix::fs::Dir::read_from(fd)
        {
            // Update outputs that are not physically connected, but have an entry in the
            // cache.
            // This fixes the case where a screen not connected when the wallpaper was changed
            // would still have the old wallpaper when reconnected
            let cached_outputs_not_in_outputs: Vec<String> = dir
                .filter_map(|file| {
                    if let Ok(f) = file
                        && let Ok(filename) = f.file_name().to_str()
                        && filename != ".."
                        && filename != "."
                        && !outputs.contains(&&filename.to_string())
                    {
                        Some(filename.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            // Update the cache's img_path
            for output in cached_outputs_not_in_outputs {
                if let Ok(cache_data) = read_cache_file(&output)
                    && let Ok(Some(prev_image_cache)) =
                        get_previous_image_cache(&output, namespace, &cache_data)
                    && let Err(e) = super::cache::CacheEntry::new(
                        namespace,
                        prev_image_cache.resize,
                        prev_image_cache.crop_gravity,
                        prev_image_cache.filter,
                        &img_path,
                    )
                    .store(&output)
                {
                    log::error!("failed to update cache's img_path: {e}");
                }
            }
        }
    }

    #[inline]
    pub fn build(mut self) -> Mmap {
        self.memory.slice_mut()[self.img_count_index] = self.img_count;
        self.memory
    }

    fn serialize_bytes(&mut self, bytes: &[u8]) {
        self.extend(&(bytes.len() as u32).to_ne_bytes());
        self.extend(bytes);
    }
}

pub enum RequestSend {
    Ping,
    Query,
    Clear(Mmap),
    Img(Mmap),
    Toggle,
    Pause,
    Unpause,
    Kill,
}

pub enum RequestRecv {
    Ping,
    Query,
    Clear(ClearReq),
    Img(ImageReq),
    Toggle,
    Pause,
    Unpause,
    Kill,
}

impl RequestSend {
    pub fn send(self, stream: &IpcSocket) -> Result<(), IpcError> {
        stream.send(self.into())
    }
}

impl RequestRecv {
    #[must_use]
    #[inline]
    pub fn receive(msg: RawMsg) -> Self {
        msg.into()
    }
}

pub enum Answer {
    Ok,
    Ping(bool),
    Info(Box<[BgInfo]>),
}

impl Answer {
    pub fn send(self, stream: &IpcSocket) -> Result<(), IpcError> {
        stream.send(self.try_into()?)
    }

    #[must_use]
    #[inline]
    pub fn receive(msg: RawMsg) -> Self {
        msg.into()
    }
}
