// Bump frame allocator that hands out 4 KiB frames from the CONVENTIONAL regions of the
// UEFI memory map, front to back, and never takes them back
// Placeholder until the shared D3OS frame allocator (llfree, see memory/frames_lf.rs)

use crate::{MAX_MEMORY_REGIONS, PAGE_SIZE, Region};

pub struct BumpFrameAllocator {
    regions: [Region; MAX_MEMORY_REGIONS],
    region_count: usize,
    // Index of the region `next` points into.
    current_region: usize,
    // Physical address of the next free frame.
    next: usize,
}

impl BumpFrameAllocator {
    /// `regions` must only contain free memory (CONVENTIONAL minus everything the kernel
    /// still uses), as built by build_aarch64_boot_info.
    pub fn new(regions: [Region; MAX_MEMORY_REGIONS], region_count: usize) -> Self {
        // UEFI describes memory in 4 KiB pages, so every region starts on a frame boundary.
        for region in &regions[..region_count] {
            assert_eq!(region.start % PAGE_SIZE, 0, "usable region {:#x} is not frame aligned", region.start);
        }

        Self { regions, region_count, current_region: 0, next: regions[0].start }
    }

    /// Returns the physical address of `frame_count` contiguous, frame-aligned frames.
    /// The memory is not zeroed.
    pub fn alloc(&mut self, frame_count: usize) -> usize {
        let size = frame_count * PAGE_SIZE;

        while self.current_region < self.region_count {
            let region = self.regions[self.current_region];
            if self.next + size <= region.end {
                let frame = self.next;
                self.next += size;
                return frame;
            }

            // Not enough room left in this region: skip the rest and move to the next one.
            self.current_region += 1;
            if self.current_region < self.region_count {
                self.next = self.regions[self.current_region].start;
            }
        }

        panic!("out of physical frames (requested {} frames)", frame_count);
    }
}
