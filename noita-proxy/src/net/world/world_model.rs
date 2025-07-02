use std::num::NonZeroU16;
use std::sync::Arc;

use bitcode::{Decode, Encode};
use chunk::{Chunk, CompactPixel, Pixel, PixelFlags};
use encoding::{NoitaWorldUpdate, PixelRun, PixelRunner};
use rustc_hash::{FxHashMap, FxHashSet};
use tracing::info;

use xxhash_rust::xxh64::Xxh64;

pub(crate) mod chunk;
pub mod encoding;

pub(crate) const CHUNK_SIZE: usize = 128;

#[derive(Debug, Encode, Decode, Clone, Copy, Hash, PartialEq, Eq)]
pub(crate) struct ChunkCoord(pub i32, pub i32);

#[derive(Default)]
pub(crate) struct WorldModel {
    chunks: FxHashMap<ChunkCoord, Chunk>,
    /// Tracks chunks which we written to.
    /// This includes any write, not just those that actually changed at least one pixel.
    updated_chunks: FxHashSet<ChunkCoord>,
}

/// Contains full info abount a chunk, RLE encoded.
/// Kinda close to ChunkDelta, but doesn't assume we know anything about the chunk.
#[derive(Debug, Encode, Decode, Clone)]
pub(crate) struct ChunkData {
    pub runs: Vec<PixelRun<CompactPixel>>,
}

/// Contains a diff, only pixels that were updated, for a given chunk.
#[derive(Debug, Encode, Decode, Clone)]
pub(crate) struct ChunkDelta {
    pub chunk_coord: ChunkCoord,
    runs: Arc<Vec<PixelRun<Option<CompactPixel>>>>,
}

impl ChunkData {
    pub(crate) fn make_random() -> Self {
        let mut runner = PixelRunner::new();
        for i in 0..CHUNK_SIZE * CHUNK_SIZE {
            runner.put_pixel(
                Pixel {
                    flags: PixelFlags::Normal,
                    material: (i as u16) % 512,
                }
                .to_compact(),
            )
        }
        let runs = runner.build();
        ChunkData { runs }
    }

    #[cfg(test)]
    pub(crate) fn new(mat: u16) -> Self {
        let mut runner = PixelRunner::new();
        for _ in 0..CHUNK_SIZE * CHUNK_SIZE {
            runner.put_pixel(
                Pixel {
                    flags: PixelFlags::Normal,
                    material: mat,
                }
                .to_compact(),
            )
        }
        let runs = runner.build();
        ChunkData { runs }
    }

    pub(crate) fn apply_to_chunk(&self, chunk: &mut Chunk) {
        let mut offset = 0;
        for run in &self.runs {
            let pixel = run.data;
            for _ in 0..run.length {
                chunk.set_compact_pixel(offset, pixel);
                offset += 1;
            }
        }
    }
    pub(crate) fn apply_delta(&mut self, delta: ChunkData) {
        let nil = CompactPixel(NonZeroU16::new(4095).unwrap());
        let mut chunk = Chunk::default();
        self.apply_to_chunk(&mut chunk);
        let mut offset = 0;
        for run in delta.runs.iter() {
            if run.data != nil {
                for _ in 0..run.length {
                    chunk.set_compact_pixel(offset, run.data);
                    offset += 1;
                }
            } else {
                offset += run.length as usize
            }
        }
        *self = chunk.to_chunk_data()
    }
}

impl WorldModel {
    fn get_chunk_coords(x: i32, y: i32) -> (ChunkCoord, usize) {
        let chunk_x = x.div_euclid(CHUNK_SIZE as i32);
        let chunk_y = y.div_euclid(CHUNK_SIZE as i32);
        let x = x.rem_euclid(CHUNK_SIZE as i32) as usize;
        let y = y.rem_euclid(CHUNK_SIZE as i32) as usize;
        let offset = x + y * CHUNK_SIZE;
        (ChunkCoord(chunk_x, chunk_y), offset)
    }

    /*fn set_pixel(&mut self, x: i32, y: i32, pixel: Pixel) {
        let (chunk_coord, offset) = Self::get_chunk_coords(x, y);
        let chunk = self.chunks.entry(chunk_coord).or_default();
        let current = chunk.pixel(offset);
        if current != pixel {
            chunk.set_pixel(offset, pixel);
        }
        self.updated_chunks.insert(chunk_coord);
    }*/

    fn get_pixel(&self, x: i32, y: i32) -> Pixel {
        let (chunk_coord, offset) = Self::get_chunk_coords(x, y);
        self.chunks
            .get(&chunk_coord)
            .map(|chunk| chunk.pixel(offset))
            .unwrap_or_default()
    }

    pub fn apply_noita_update(
        &mut self,
        update: &NoitaWorldUpdate,
        changed: &mut FxHashSet<ChunkCoord>,
    ) {
        fn set_pixel(pixel: Pixel, chunk: &mut Chunk, offset: usize) -> bool {
            let current = chunk.pixel(offset);
            if current != pixel {
                chunk.set_pixel(offset, pixel);
                true
            } else {
                false
            }
        }
        let header = &update.header;
        let runs = &update.runs;
        let mut x = 0;
        let mut y = 0;
        let (mut chunk_coord, _) = Self::get_chunk_coords(header.x, header.y);
        let mut chunk = self.chunks.entry(chunk_coord).or_default();
        for run in runs {
            let flags = if run.data.flags > 0 {
                PixelFlags::Fluid
            } else {
                PixelFlags::Normal
            };
            for _ in 0..run.length {
                let xs = header.x + x;
                let ys = header.y + y;
                let (new_chunk_coord, offset) = Self::get_chunk_coords(xs, ys);
                if chunk_coord != new_chunk_coord {
                    chunk_coord = new_chunk_coord;
                    chunk = self.chunks.entry(chunk_coord).or_default();
                }
                if set_pixel(
                    Pixel {
                        material: run.data.material,
                        flags,
                    },
                    chunk,
                    offset,
                ) {
                    self.updated_chunks.insert(chunk_coord);
                    if changed.contains(&chunk_coord) {
                        changed.remove(&chunk_coord);
                    }
                }
                x += 1;
                if x == i32::from(header.w) + 1 {
                    x = 0;
                    y += 1;
                }
            }
        }
    }

    pub fn get_noita_update(&self, x: i32, y: i32, w: u32, h: u32) -> NoitaWorldUpdate {
        assert!(w <= 256);
        assert!(h <= 256);
        let mut runner = PixelRunner::new();
        for j in 0..(h as i32) {
            for i in 0..(w as i32) {
                runner.put_pixel(self.get_pixel(x + i, y + j).to_raw())
            }
        }
        runner.into_noita_update(x, y, (w - 1) as u8, (h - 1) as u8)
    }

    pub fn get_all_noita_updates(&self) -> Vec<Vec<u8>> {
        let mut updates = Vec::new();
        for chunk_coord in &self.updated_chunks {
            let update = self.get_noita_update(
                chunk_coord.0 * (CHUNK_SIZE as i32),
                chunk_coord.1 * (CHUNK_SIZE as i32),
                CHUNK_SIZE as u32,
                CHUNK_SIZE as u32,
            );
            updates.push(update.save());
        }
        updates
    }

    pub(crate) fn apply_chunk_delta(&mut self, delta: &ChunkDelta) {
        self.updated_chunks.insert(delta.chunk_coord);
        let chunk = self.chunks.entry(delta.chunk_coord).or_default();
        let mut offset = 0;
        for run in delta.runs.iter() {
            if let Some(pixel) = run.data {
                for _ in 0..run.length {
                    chunk.set_compact_pixel(offset, pixel);
                    offset += 1;
                }
            } else {
                offset += run.length as usize
            }
        }
    }

    pub(crate) fn get_chunk_delta(
        &self,
        chunk_coord: ChunkCoord,
        ignore_changed: bool,
    ) -> Option<ChunkDelta> {
        let chunk = self.chunks.get(&chunk_coord)?;
        let mut runner = PixelRunner::new();
        for i in 0..CHUNK_SIZE * CHUNK_SIZE {
            runner.put_pixel((ignore_changed || chunk.changed(i)).then(|| chunk.compact_pixel(i)))
        }
        let runs = runner.build().into();
        Some(ChunkDelta { chunk_coord, runs })
    }

    pub fn updated_chunks(&self) -> &FxHashSet<ChunkCoord> {
        &self.updated_chunks
    }

    pub fn reset_change_tracking(&mut self) {
        for chunk_pos in &self.updated_chunks {
            if let Some(chunk) = self.chunks.get_mut(chunk_pos) {
                chunk.clear_changed();
            }
        }
        self.updated_chunks.clear();
    }

    pub fn reset(&mut self) {
        self.chunks.clear();
        self.updated_chunks.clear();
        info!("World model reset");
    }

    pub(crate) fn apply_chunk_data(&mut self, chunk: ChunkCoord, chunk_data: &ChunkData) {
        self.updated_chunks.insert(chunk);
        let chunk = self.chunks.entry(chunk).or_default();
        chunk_data.apply_to_chunk(chunk);
    }

    pub(crate) fn get_chunk_data(&self, chunk: ChunkCoord) -> Option<ChunkData> {
        let chunk = self.chunks.get(&chunk)?;
        Some(chunk.to_chunk_data())
    }

    pub(crate) fn forget_chunk(&mut self, chunk: ChunkCoord) {
        self.chunks.remove(&chunk);
        self.updated_chunks.remove(&chunk);
    }

    pub fn compute_chunk_checksum(&self, coord: &ChunkCoord) -> u64 {
        if let Some(_) = self.chunks.get(coord) {
            // 1) Genera la actualización de Noita para ese chunk
            let update = self.get_noita_update(
                coord.0 * CHUNK_SIZE as i32,
                coord.1 * CHUNK_SIZE as i32,
                CHUNK_SIZE as u32,
                CHUNK_SIZE as u32,
            );

            // 2) Crea el hasher
            let mut hasher = Xxh64::new(0);

            // 3) Mezcla el header
            let h = &update.header;
            hasher.update(&h.x.to_le_bytes());
            hasher.update(&h.y.to_le_bytes());
            hasher.update(&[h.w]);
            hasher.update(&[h.h]);
            hasher.update(&h.run_count.to_le_bytes());

            // 4) Mezcla cada run: length + material + flags
            for run in &update.runs {
                hasher.update(&run.length.to_le_bytes());
                hasher.update(&run.data.material.to_le_bytes());
                hasher.update(&[run.data.flags]);
            }

            // 5) Devuelve el digest
            hasher.digest()
        } else {
            0
        }
    }
}
