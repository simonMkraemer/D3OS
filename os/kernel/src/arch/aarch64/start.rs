#![no_std]
#![no_main]

mod bootinfo;
mod framebuffer;
mod multiboot2;
mod serial;

use core::panic::PanicInfo;
use crate::framebuffer::Framebuffer;

unsafe extern "C" {
    static ___KERNEL_DATA_START__: u8;
    static ___KERNEL_DATA_END__: u8;
}

#[derive(Clone, Copy)]
struct Region {
    start: usize,
    end: usize,
}

const PAGE_SIZE: usize = 0x1000;
const MAX_MEMORY_REGIONS: usize = 64;
const EFI_MEMORY_MAP_CAPACITY: usize = 64 * 1024;

#[repr(C)]
struct EfiTableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[repr(C)]
struct EfiSystemTable {
    header: EfiTableHeader,
    firmware_vendor: *const u16,
    firmware_revision: u32,
    _padding: u32,
    console_in_handle: usize,
    con_in: usize,
    console_out_handle: usize,
    con_out: usize,
    standard_error_handle: usize,
    std_err: usize,
    runtime_services: usize,
    boot_services: *const EfiBootServices,
}

#[repr(C)]
struct EfiBootServices {
    header: EfiTableHeader,
    raise_tpl: usize,
    restore_tpl: usize,
    allocate_pages: usize,
    free_pages: usize,
    get_memory_map: unsafe extern "efiapi" fn(
        *mut usize,
        *mut u8,
        *mut usize,
        *mut usize,
        *mut u32,
    ) -> usize,
    _before_exit_boot_services: [usize; 21],
    exit_boot_services: unsafe extern "efiapi" fn(usize, usize) -> usize,
}

#[repr(align(8))]
struct EfiMemoryMap([u8; EFI_MEMORY_MAP_CAPACITY]);

static mut EFI_MEMORY_MAP: EfiMemoryMap = EfiMemoryMap([0; EFI_MEMORY_MAP_CAPACITY]);

struct EarlyFrameAllocator {
    regions: [Region; MAX_MEMORY_REGIONS],
    count: usize,
    current: usize,
}

#[derive(Clone, Copy)]
struct FramebufferInfo {
    address: usize,
    pitch: usize,
    width: usize,
    height: usize,
    bpp: usize,
}

#[unsafe(no_mangle)]
pub extern "C" fn start_aarch64(x0: usize, x1: usize, x2: usize, x3: usize) -> ! {
    serial::write_str("HELLO WORLD start\n");
    serial::write_labelled_hex("x0 = ", x0);
    serial::write_labelled_hex("x1 = ", x1);
    serial::write_labelled_hex("x2 = ", x2);
    serial::write_labelled_hex("x3 = ", x3);

    if x0 != multiboot2::MAGIC {
        serial::write_str("x0 does not match Multiboot2 magic\n");
        loop {
            core::hint::spin_loop();
        }
    }
    serial::write_str("x0 matches Multiboot2 magic\n");

    multiboot2::dump_words(x1, 4);
    multiboot2::dump_tags(x1);

    let Some(parsed) = multiboot2::parse_boot_info(x1) else {
        serial::write_str("failed to parse boot info\n");
        loop {
            core::hint::spin_loop();
        }
    };
    bootinfo::dump(&parsed);

    if parsed.efi_boot_services_not_exited {
        exit_boot_services(&parsed);
    }

    match parse_framebuffer_tag(x1) {
        Some(framebuffer) => {
            serial::write_str("framebuffer tag parsed:\n");
            serial::write_labelled_hex("  addr = ", framebuffer.address);
            serial::write_labelled_hex("  pitch = ", framebuffer.pitch);
            serial::write_labelled_hex("  width = ", framebuffer.width);
            serial::write_labelled_hex("  height = ", framebuffer.height);
            serial::write_labelled_hex("  bpp = ", framebuffer.bpp);
            //paint_green(framebuffer);
            print_framebuffer_status(framebuffer, x1);
        }
        None => serial::write_str("framebuffer tag missing or unsupported\n"),
    }

    dump_memory_map(x1);
    dump_reserved_regions(x1);
    dump_allocator_regions(x1);
    exercise_frame_allocator(x1);

    loop {
        core::hint::spin_loop();
    }
}

fn exit_boot_services(parsed: &bootinfo::BootInfo) {
    let (Some(system_table), Some(image_handle)) =
        (parsed.efi_system_table, parsed.efi_image_handle)
    else {
        serial::write_str("EFI handoff is incomplete\n");
        return;
    };

    serial::write_str("exiting EFI Boot Services\n");
    match unsafe { exit_boot_services_raw(system_table, image_handle) } {
        Ok(()) => serial::write_str("EFI Boot Services exited\n"),
        Err(status) => serial::write_labelled_hex("ExitBootServices failed: ", status),
    }
}

unsafe fn exit_boot_services_raw(system_table: usize, image_handle: usize) -> Result<(), usize> {
    let system_table = unsafe { &*(system_table as *const EfiSystemTable) };
    let boot_services = unsafe { system_table.boot_services.as_ref() }.ok_or(usize::MAX)?;
    let mut memory_map_size = EFI_MEMORY_MAP_CAPACITY;
    let mut memory_map_key = 0usize;
    let mut descriptor_size = 0usize;
    let mut descriptor_version = 0u32;
    let memory_map = core::ptr::addr_of_mut!(EFI_MEMORY_MAP.0).cast::<u8>();

    let status = unsafe {
        (boot_services.get_memory_map)(
            &mut memory_map_size,
            memory_map,
            &mut memory_map_key,
            &mut descriptor_size,
            &mut descriptor_version,
        )
    };
    if status != 0 {
        return Err(status);
    }

    let status = unsafe { (boot_services.exit_boot_services)(image_handle, memory_map_key) };
    if status != 0 {
        return Err(status);
    }

    Ok(())
}




fn read_u32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn read_u64(addr: usize) -> u64 {
    unsafe { core::ptr::read_volatile(addr as *const u64) }
}

fn parse_framebuffer_tag(bootinfo: usize) -> Option<FramebufferInfo> {
    if bootinfo == 0 || bootinfo & 7 != 0 {
        return None;
    }

    let total_size = read_u32(bootinfo) as usize;
    let mut offset = 8usize;

    while offset + 8 <= total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        if tag_size < 8 || offset + tag_size > total_size {
            return None;
        }

        if tag_type == 8 && tag_size >= 30 {
            let framebuffer_type = unsafe { core::ptr::read_volatile((tag_addr + 29) as *const u8) };
            if framebuffer_type != 1 {
                return None;
            }

            return Some(FramebufferInfo {
                address: read_u64(tag_addr + 8) as usize,
                pitch: read_u32(tag_addr + 16) as usize,
                width: read_u32(tag_addr + 20) as usize,
                height: read_u32(tag_addr + 24) as usize,
                bpp: unsafe { core::ptr::read_volatile((tag_addr + 28) as *const u8) as usize },
            });
        }

        if tag_type == 0 {
            break;
        }

        offset = (offset + tag_size + 7) & !7;
    }

    None
}

fn parse_first_module_region(bootinfo: usize) -> Option<Region> {
    if bootinfo == 0 || bootinfo & 7 != 0 {
        return None;
    }

    let total_size = read_u32(bootinfo) as usize;
    let mut offset = 8usize;

    while offset + 8 <= total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        if tag_size < 8 || offset + tag_size > total_size {
            return None;
        }

        if tag_type == 3 && tag_size >= 16 {
            return Some(Region {
                start: read_u32(tag_addr + 8) as usize,
                end: read_u32(tag_addr + 12) as usize,
            });
        }

        if tag_type == 0 {
            break;
        }

        offset = (offset + tag_size + 7) & !7;
    }

    None
}

fn dump_memory_map(bootinfo: usize) {
    if bootinfo == 0 || bootinfo & 7 != 0 {
        serial::write_str("memory map unavailable\n");
        return;
    }

    let total_size = read_u32(bootinfo) as usize;
    let mut offset = 8usize;

    while offset + 8 <= total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        if tag_size < 8 || offset + tag_size > total_size {
            serial::write_str("invalid memory map tag\n");
            return;
        }

        if tag_type == 6 && tag_size >= 16 {
            let entry_size = read_u32(tag_addr + 8) as usize;
            if entry_size < 24 {
                serial::write_str("memory map entry size too small\n");
                return;
            }

            serial::write_str("memory map entries:\n");
            let mut entry_addr = tag_addr + 16;
            let tag_end = tag_addr + tag_size;
            while entry_addr + entry_size <= tag_end {
                let start = read_u64(entry_addr) as usize;
                let length = read_u64(entry_addr + 8) as usize;
                let end = start.saturating_add(length);
                let area_type = read_u32(entry_addr + 16);

                serial::write_str("  [");
                serial::write_hex_usize(start);
                serial::write_str(", ");
                serial::write_hex_usize(end);
                serial::write_str(") type=");
                serial::write_dec_usize(area_type as usize);
                serial::write_str(" (");
                serial::write_str(memory_area_type_name(area_type));
                serial::write_str(")");
                if area_type == 1 {
                    serial::write_str(" usable");
                }
                serial::write_str("\n");

                entry_addr += entry_size;
            }
            return;
        }

        if tag_type == 0 {
            break;
        }

        offset = (offset + tag_size + 7) & !7;
    }

    serial::write_str("memory map tag missing\n");
}

fn dump_reserved_regions(bootinfo: usize) {
    serial::write_str("reserved regions:\n");

    let kernel = Region {
        start: core::ptr::addr_of!(___KERNEL_DATA_START__) as usize,
        end: core::ptr::addr_of!(___KERNEL_DATA_END__) as usize,
    };
    dump_region("  kernel", kernel);

    let bootinfo_region = Region {
        start: bootinfo,
        end: bootinfo.saturating_add(read_u32(bootinfo) as usize),
    };
    dump_region("  bootinfo", bootinfo_region);

    match parse_first_module_region(bootinfo) {
        Some(initrd) => dump_region("  initrd", initrd),
        None => serial::write_str("  initrd: <missing>\n"),
    }

    match parse_framebuffer_tag(bootinfo) {
        Some(framebuffer) => dump_region(
            "  framebuffer",
            Region {
                start: framebuffer.address,
                end: framebuffer.address.saturating_add(framebuffer.pitch.saturating_mul(framebuffer.height)),
            },
        ),
        None => serial::write_str("  framebuffer: <missing>\n"),
    }
}

fn dump_allocator_regions(bootinfo: usize) {
    let (usable, usable_count) = build_allocator_regions(bootinfo);

    serial::write_str("allocator regions:\n");
    for region in usable.iter().copied().take(usable_count) {
        dump_region("  free", region);
    }
}

fn exercise_frame_allocator(bootinfo: usize) {
    let mut allocator = EarlyFrameAllocator::from_bootinfo(bootinfo);

    serial::write_str("frame allocator test:\n");
    for index in 0..3 {
        serial::write_str("  frame ");
        serial::write_dec_usize(index);
        serial::write_str(" = ");
        match allocator.alloc_frame() {
            Some(frame) => serial::write_hex_usize(frame),
            None => serial::write_str("<none>"),
        }
        serial::write_str("\n");
    }

    serial::write_str("  contiguous 4 pages = ");
    match allocator.alloc_frames(4) {
        Some(start) => serial::write_hex_usize(start),
        None => serial::write_str("<none>"),
    }
    serial::write_str("\n");
}

fn build_allocator_regions(bootinfo: usize) -> ([Region; MAX_MEMORY_REGIONS], usize) {
    let mut usable = [Region { start: 0, end: 0 }; MAX_MEMORY_REGIONS];
    let mut usable_count = collect_usable_regions(bootinfo, &mut usable);

    let mut reserved = [Region { start: 0, end: 0 }; 4];
    let mut reserved_count = 0usize;

    reserved[reserved_count] = Region {
        start: core::ptr::addr_of!(___KERNEL_DATA_START__) as usize,
        end: core::ptr::addr_of!(___KERNEL_DATA_END__) as usize,
    };
    reserved_count += 1;

    reserved[reserved_count] = Region {
        start: bootinfo,
        end: bootinfo.saturating_add(read_u32(bootinfo) as usize),
    };
    reserved_count += 1;

    if let Some(initrd) = parse_first_module_region(bootinfo) {
        reserved[reserved_count] = initrd;
        reserved_count += 1;
    }

    if let Some(framebuffer) = parse_framebuffer_tag(bootinfo) {
        reserved[reserved_count] = Region {
            start: framebuffer.address,
            end: framebuffer.address.saturating_add(framebuffer.pitch.saturating_mul(framebuffer.height)),
        };
        reserved_count += 1;
    }

    for reserved_region in reserved.iter().copied().take(reserved_count) {
        usable_count = subtract_reserved_regions(&mut usable, usable_count, reserved_region);
    }

    (usable, usable_count)
}

fn collect_usable_regions(bootinfo: usize, out: &mut [Region; MAX_MEMORY_REGIONS]) -> usize {
    if bootinfo == 0 || bootinfo & 7 != 0 {
        return 0;
    }

    let total_size = read_u32(bootinfo) as usize;
    let mut offset = 8usize;

    while offset + 8 <= total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        if tag_size < 8 || offset + tag_size > total_size {
            return 0;
        }

        if tag_type == 6 && tag_size >= 16 {
            let entry_size = read_u32(tag_addr + 8) as usize;
            if entry_size < 24 {
                return 0;
            }

            let mut count = 0usize;
            let mut entry_addr = tag_addr + 16;
            let tag_end = tag_addr + tag_size;
            while entry_addr + entry_size <= tag_end && count < out.len() {
                let start = read_u64(entry_addr) as usize;
                let length = read_u64(entry_addr + 8) as usize;
                let area_type = read_u32(entry_addr + 16);

                if area_type == 1 && length != 0 {
                    let region = Region {
                        start: align_up(start, PAGE_SIZE),
                        end: align_down(start.saturating_add(length), PAGE_SIZE),
                    };
                    if region.end > region.start {
                        out[count] = region;
                        count += 1;
                    }
                }

                entry_addr += entry_size;
            }
            return count;
        }

        if tag_type == 0 {
            break;
        }

        offset = (offset + tag_size + 7) & !7;
    }

    0
}

fn subtract_reserved_regions(regions: &mut [Region; MAX_MEMORY_REGIONS], count: usize, reserved: Region) -> usize {
    let reserved = Region {
        start: align_down(reserved.start, PAGE_SIZE),
        end: align_up(reserved.end, PAGE_SIZE),
    };

    if reserved.end <= reserved.start {
        return count;
    }

    let mut next = [Region { start: 0, end: 0 }; MAX_MEMORY_REGIONS];
    let mut next_count = 0usize;

    for region in regions.iter().copied().take(count) {
        if region.end <= reserved.start || region.start >= reserved.end {
            if next_count < next.len() {
                next[next_count] = region;
                next_count += 1;
            }
            continue;
        }

        if region.start < reserved.start && next_count < next.len() {
            let left = Region {
                start: region.start,
                end: reserved.start,
            };
            if left.end > left.start {
                next[next_count] = left;
                next_count += 1;
            }
        }

        if region.end > reserved.end && next_count < next.len() {
            let right = Region {
                start: reserved.end,
                end: region.end,
            };
            if right.end > right.start {
                next[next_count] = right;
                next_count += 1;
            }
        }
    }

    regions[..next_count].copy_from_slice(&next[..next_count]);
    next_count
}

impl EarlyFrameAllocator {
    fn from_bootinfo(bootinfo: usize) -> Self {
        let (regions, count) = build_allocator_regions(bootinfo);
        Self {
            regions,
            count,
            current: 0,
        }
    }

    fn alloc_frame(&mut self) -> Option<usize> {
        while self.current < self.count {
            let region = &mut self.regions[self.current];
            if region.start + PAGE_SIZE <= region.end {
                let frame = region.start;
                region.start += PAGE_SIZE;
                return Some(frame);
            }
            self.current += 1;
        }

        None
    }

    fn alloc_frames(&mut self, frame_count: usize) -> Option<usize> {
        let bytes = frame_count.checked_mul(PAGE_SIZE)?;

        while self.current < self.count {
            let region = &mut self.regions[self.current];
            if region.start + bytes <= region.end {
                let start = region.start;
                region.start += bytes;
                return Some(start);
            }
            self.current += 1;
        }

        None
    }
}

fn align_up(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }

    let mask = align - 1;
    value.saturating_add(mask) & !mask
}

fn align_down(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }

    value & !(align - 1)
}

fn dump_region(label: &str, region: Region) {
    serial::write_str(label);
    serial::write_str(" = [");
    serial::write_hex_usize(region.start);
    serial::write_str(", ");
    serial::write_hex_usize(region.end);
    serial::write_str(")\n");
}

fn memory_area_type_name(area_type: u32) -> &'static str {
    match area_type {
        1 => "available",
        2 => "reserved",
        3 => "acpi",
        4 => "hibernate",
        5 => "defective",
        _ => "unknown",
    }
}

fn print_framebuffer_status(framebuffer: FramebufferInfo, bootinfo: usize) {
    let Some(console) = Framebuffer::new(
        framebuffer.address,
        framebuffer.pitch,
        framebuffer.width,
        framebuffer.height,
        framebuffer.bpp,
    ) else {
        serial::write_str("framebuffer console unavailable\n");
        return;
    };

    console.clear(0x00, 0x40, 0x00);
    console.write_line(0, "ARM BOOT OK");
    console.write_line(1, "FRAMEBUFFER OK");
    console.write_line(2, "BOOTINFO");
    console.write_hex_usize(8, 8 + 3 * 10, bootinfo);

    let (regions, count) = build_allocator_regions(bootinfo);
    console.write_line(4, "ALLOC REGIONS");
    let shown = core::cmp::min(count, 3);
    for (index, region) in regions.iter().take(shown).enumerate() {
        let y = 8 + (5 + index) * 10;
        console.write_hex_usize(8, y, region.start);
        console.write_text(8 + 19 * 6, y, "-");
        console.write_hex_usize(8 + 21 * 6, y, region.end);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial::write_str("HELLO WORLD panic\n");
    loop {
        core::hint::spin_loop();
    }
}
