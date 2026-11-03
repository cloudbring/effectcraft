use std::sync::{Mutex, PoisonError};

use crate::Image;
use effectcraft_raster::Px;
use rayon::prelude::*;

const RETAINED_BYTES: usize = 192 * 1024 * 1024;
const MAX_PIXELS: usize = 1 << 26;
const SHELVES: usize = 16;

pub(crate) fn zip_region(image: &mut Image, other: &Image, region: [u32; 4], apply: impl Fn(&mut Px, &Px) + Sync + Send) {
    let [x0, y0, x1, y1] = region;
    let x1 = x1.min(image.width).min(other.width) as usize;
    let y1 = y1.min(image.height).min(other.height) as usize;
    let x0 = (x0 as usize).min(x1);
    let y0 = (y0 as usize).min(y1);
    image.data.par_chunks_mut((image.width as usize).max(1)).zip(other.data.par_chunks((other.width as usize).max(1))).skip(y0).take(y1 - y0).for_each(
        |(target, source)| {
            let Some(target) = target.get_mut(x0..x1) else { return };
            let Some(source) = source.get(x0..x1) else { return };
            for (target, source) in target.iter_mut().zip(source) {
                apply(target, source);
            }
        },
    );
}

#[derive(Default)]
pub(crate) struct BufferPool {
    inner: Mutex<Shelf>,
}

#[derive(Default)]
struct Shelf {
    buffers: [Option<Vec<Px>>; SHELVES],
    bytes: usize,
}

impl BufferPool {
    fn storage(&self, width: u32, height: u32) -> Option<Image> {
        let pixels = (width as usize).checked_mul(height as usize).filter(|pixels| *pixels <= MAX_PIXELS)?;
        let mut shelf = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let data = shelf.buffers.iter_mut().find(|slot| slot.as_ref().is_some_and(|data| data.len() == pixels)).and_then(Option::take);
        if let Some(data) = data {
            shelf.bytes = shelf.bytes.saturating_sub(data.capacity().saturating_mul(size_of::<Px>()));
            return Some(Image { width, height, data });
        }
        drop(shelf);
        let mut data = Vec::new();
        data.try_reserve_exact(pixels).ok()?;
        data.resize(pixels, [0.0; 4]);
        Some(Image { width, height, data })
    }

    pub(crate) fn take(&self, width: u32, height: u32) -> Option<Image> {
        self.filled(width, height, [0.0; 4])
    }

    pub(crate) fn filled(&self, width: u32, height: u32, pixel: Px) -> Option<Image> {
        let mut image = self.storage(width, height)?;
        image.data.fill(pixel);
        Some(image)
    }

    pub(crate) fn copy(&self, source: &Image) -> Option<Image> {
        let mut image = self.storage(source.width, source.height)?;
        if image.data.len() != source.data.len() {
            return None;
        }
        image.data.copy_from_slice(&source.data);
        Some(image)
    }

    pub(crate) fn recycle(&self, image: Image) {
        self.recycle_with_limit(image, RETAINED_BYTES);
    }

    fn recycle_with_limit(&self, image: Image, limit: usize) {
        let bytes = image.data.capacity().saturating_mul(size_of::<Px>());
        if bytes == 0 || bytes > limit {
            return;
        }
        let mut shelf = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        for index in 0..SHELVES {
            if shelf.bytes.saturating_add(bytes) <= limit && shelf.buffers.iter().any(Option::is_none) {
                break;
            }
            if let Some(data) = shelf.buffers.get_mut(index).and_then(Option::take) {
                shelf.bytes = shelf.bytes.saturating_sub(data.capacity().saturating_mul(size_of::<Px>()));
            }
        }
        if let Some(slot) = shelf.buffers.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(image.data);
            shelf.bytes = shelf.bytes.saturating_add(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_evicts_old_storage_and_rejects_oversized_buffers() {
        let pool = BufferPool::default();
        for _ in 0..10 {
            pool.recycle_with_limit(Image::new(8, 8), 2048);
        }
        pool.recycle_with_limit(Image::new(16, 16), 2048);
        let shelf = pool.inner.lock().unwrap();
        assert_eq!(shelf.bytes, 2048);
        assert_eq!(shelf.buffers.iter().flatten().count(), 2);
    }

    #[test]
    fn recycled_storage_is_fully_initialized_and_size_keyed() {
        let pool = BufferPool::default();
        let mut image = pool.take(16, 8).unwrap();
        let pointer = image.data.as_ptr();
        image.data.fill([f32::NAN, -0.0, f32::INFINITY, 1.0]);
        pool.recycle(image);
        let image = pool.take(8, 16).unwrap();
        assert_eq!(pointer, image.data.as_ptr());
        assert!(image.data.iter().flatten().all(|value| value.to_bits() == 0));
        pool.recycle(image);
        let source = Image::filled(16, 8, [-0.0, 0.125, 0.25, 0.5]);
        let copy = pool.copy(&source).unwrap();
        assert_eq!(pointer, copy.data.as_ptr());
        assert!(copy.data.iter().flatten().zip(source.data.iter().flatten()).all(|(left, right)| left.to_bits() == right.to_bits()));
        assert!(pool.take(u32::MAX, u32::MAX).is_none());
    }

    #[test]
    fn simultaneous_leases_do_not_alias_and_shelves_are_bounded() {
        let pool = BufferPool::default();
        let first = pool.take(8, 8).unwrap();
        let second = pool.take(8, 8).unwrap();
        assert_ne!(first.data.as_ptr(), second.data.as_ptr());
        pool.recycle(first);
        pool.recycle(second);
        for width in 1..100 {
            pool.recycle(Image::new(width, 10));
        }
        let shelf = pool.inner.lock().unwrap();
        assert_eq!(shelf.buffers.iter().flatten().count(), SHELVES);
        assert!(shelf.bytes <= RETAINED_BYTES);
        assert_eq!(shelf.bytes, shelf.buffers.iter().flatten().map(|data| data.capacity() * size_of::<Px>()).sum::<usize>());
    }
}
