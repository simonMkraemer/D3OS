use core::alloc::{Allocator, AllocError, Layout};
use core::ptr::{self, NonNull};
use log::info;
use uefi::boot::exit;
use x86_64::instructions::port::Port;
use linked_list_allocator::LockedHeap;
use x86_64::structures::paging::frame::PhysFrameRange;
use crate::memory::nvmem::Locked;
use crate::memory::nvmem::align_up;
use crate::memory::{PAGE_SIZE, physical};


pub struct NvramAllocator {
    heap: LockedHeap,
}

impl NvramAllocator {
    pub const fn new() -> Self {
        Self {
            heap: LockedHeap::empty(),
        }
    }

    pub fn init(&self, frames: &PhysFrameRange) {
        let mut heap = self.heap.lock();
        unsafe {
            heap.init(frames.start.start_address().as_u64() as *mut u8, (frames.end - frames.start) as usize * PAGE_SIZE);
        }
    }
}

unsafe impl Allocator for NvramAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        info!("Allocating memory with layout: {:?}", layout);
        if layout.size() == 0 {
            info!("Allocating zero size memory");
            return Ok(NonNull::slice_from_raw_parts(layout.dangling(), 0));
        }

        match self.heap.lock().allocate_first_fit(layout) {
            Ok(ptr) => {
                info!("Allocated memory at: {:?}, size: {}", ptr, layout.size());
                Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
            },
            Err(_) => Err(AllocError),
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        info!("Deallocating memory at: {:?}, size: {}", ptr, layout.size());
        if layout.size() != 0 {
            let mut heap = self.heap.lock();
            heap.deallocate(ptr, layout);
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