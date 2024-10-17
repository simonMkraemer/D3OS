use core::alloc::{Allocator, AllocError, Layout};
use core::ptr::{self, NonNull};
use log::info;
use uefi::boot::exit;
use x86_64::instructions::port::Port;
use crate::memory::nvmem::Locked;
use crate::memory::nvmem::align_up;

pub struct NvramAllocator {
    heap_start: usize,
    heap_end: usize,
    next: usize,
    allocations: usize,
}

impl NvramAllocator {
    pub const fn new() -> Self {
        Self {
            heap_start: 0,
            heap_end: 0,
            next: 0,
            allocations: 0,
        }
    }

    pub fn init(&mut self, heap_start: usize, heap_size: usize) {
        self.heap_start = heap_start;
        self.heap_end = heap_start.saturating_add(heap_size);
        self.next = heap_start;
    }
}

unsafe impl Allocator for Locked<NvramAllocator> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let mut nvalloc = self.lock(); // get a mutable reference

        let alloc_start = align_up(nvalloc.next, layout.align());
        let alloc_end = match alloc_start.checked_add(layout.size()) {
            Some(end) => end,
            None => return Err(AllocError),
        };

        if alloc_end > nvalloc.heap_end {
            Err(AllocError) // out of memory
        } else {
            nvalloc.next = alloc_end;
            nvalloc.allocations += 1;
            Ok(NonNull::slice_from_raw_parts(NonNull::new(alloc_start as *mut u8).unwrap(), layout.size()))
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        let mut nvalloc = self.lock(); // get a mutable reference

        nvalloc.allocations -= 1;
        if nvalloc.allocations == 0 {
            nvalloc.next = nvalloc.heap_start;
        }
    }
}

//testing atomic transactions with qemu exit

pub(crate) fn qemu_exit(exit_code: u32) -> ! {
    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
    loop {}
}