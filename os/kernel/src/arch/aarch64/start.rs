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
const MEBIBYTE: usize = 1024 * 1024;
const MAX_MEMORY_REGIONS: usize = 64;
const EFI_MEMORY_MAP_CAPACITY: usize = 64 * 1024;
const EFI_MEMORY_DESCRIPTOR_MIN_SIZE: usize = 40;
const EFI_CONVENTIONAL_MEMORY: u32 = 7;
const EFI_INVALID_PARAMETER: usize = (1usize << (usize::BITS - 1)) | 2;
const EXIT_BOOT_SERVICES_RETRIES: usize = 3;

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

#[derive(Clone, Copy)]
struct EfiMemoryMapInfo {
    address: usize,
    len: usize,
    descriptor_size: usize,
    descriptor_version: u32,
}

#[derive(Clone, Copy)]
struct MemorySummary {
    total_ram: usize,
    free_ram: usize,
}

impl EfiMemoryMapInfo {
    fn is_valid(self) -> bool {
        self.address != 0
            && self.descriptor_size >= EFI_MEMORY_DESCRIPTOR_MIN_SIZE
            && self.len % self.descriptor_size == 0
    }
}

#[derive(Clone, Copy)]
struct FramebufferInfo {
    address: usize,
    pitch: usize,
    width: usize,
    height: usize,
    bpp: usize,
}


// Tow-Boot lädt Kernel und Initrd                                                                                                                                                                                 LSP
// -> Tow-Boot erstellt erste BootInfo inklusive älterer MemoryMap                                                                                                                                             LSPs are disabled
// -> Tow-Boot springt nach D3OS
// -> D3OS liest UEFI-Systemtabelle und Image-Handle aus BootInfo
// -> D3OS ruft UEFI GetMemoryMap auf
// -> D3OS erhält aktuelle Map + Map-Key
// -> D3OS ruft ExitBootServices auf
// -> D3OS nutzt die frische Map für eigene freie/reservierte Regionen
// -> EarlyFrameAllocator gibt daraus physische 4-KiB-Frames aus

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

    let Some(parsed) = multiboot2::parse_boot_info(x1) else {
        serial::write_str("failed to parse boot info\n");
        halt();
    };
    bootinfo::dump(&parsed);

    if !parsed.has_required_uefi_handoff() {
        serial::write_str("required UEFI handoff data is missing\n");
        halt();
    }

    multiboot2::dump_words(x1, 4);
    multiboot2::dump_tags(x1);
    let efi_memory_map = exit_boot_services(&parsed);
    if !efi_memory_map.is_valid() {
        serial::write_str("fresh EFI memory map is invalid\n");
        halt();
    }
    dump_efi_memory_map(efi_memory_map);
    let memory_summary = memory_summary(x1, efi_memory_map);
    dump_memory_summary(memory_summary);

    match parse_framebuffer_tag(x1) {
        Some(framebuffer) => {
            serial::write_str("framebuffer tag parsed:\n");
            serial::write_labelled_hex("  addr = ", framebuffer.address);
            serial::write_labelled_hex("  pitch = ", framebuffer.pitch);
            serial::write_labelled_hex("  width = ", framebuffer.width);
            serial::write_labelled_hex("  height = ", framebuffer.height);
            serial::write_labelled_hex("  bpp = ", framebuffer.bpp);
            //paint_green(framebuffer);
            print_framebuffer_status(framebuffer, x1, efi_memory_map, memory_summary);
        }
        None => serial::write_str("framebuffer tag missing or unsupported\n"),
    }

    dump_reserved_regions(x1, efi_memory_map);
    dump_allocator_regions(x1, efi_memory_map);
    //exercise_frame_allocator(x1, efi_memory_map);

    loop {
        core::hint::spin_loop();
    }
}

fn exit_boot_services(parsed: &bootinfo::BootInfo) -> EfiMemoryMapInfo {
    let system_table = parsed.efi_system_table.expect("validated UEFI system table");
    let image_handle = parsed.efi_image_handle.expect("validated UEFI image handle");
    serial::write_str("exiting EFI Boot Services\n");
    match unsafe { exit_boot_services_raw(system_table, image_handle) } {
        Ok(memory_map) => {
            serial::write_str("EFI Boot Services exited\n");
            memory_map
        }
        Err(status) => {
            serial::write_labelled_hex("ExitBootServices failed: ", status);
            halt();
        }
    }
}

unsafe fn exit_boot_services_raw(
    system_table: usize, image_handle: usize,
) -> Result<EfiMemoryMapInfo, usize> {
    let system_table = unsafe { &*(system_table as *const EfiSystemTable) };
    let boot_services = unsafe { system_table.boot_services.as_ref() }.ok_or(usize::MAX)?;
    let memory_map = core::ptr::addr_of_mut!(EFI_MEMORY_MAP.0).cast::<u8>();

    for _ in 0..EXIT_BOOT_SERVICES_RETRIES {
        let mut memory_map_size = EFI_MEMORY_MAP_CAPACITY;
        let mut memory_map_key = 0usize;
        let mut descriptor_size = 0usize;
        let mut descriptor_version = 0u32;

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
        if status == 0 {
            return Ok(EfiMemoryMapInfo {
                address: memory_map as usize,
                len: memory_map_size,
                descriptor_size,
                descriptor_version,
            });
        }
        if status != EFI_INVALID_PARAMETER {
            return Err(status);
        }
    }

    Err(EFI_INVALID_PARAMETER)
}

fn dump_efi_memory_map(memory_map: EfiMemoryMapInfo) {
    serial::write_str("fresh EFI memory map:\n");
    serial::write_labelled_hex("  address = ", memory_map.address);
    serial::write_labelled_hex("  size = ", memory_map.len);
    serial::write_labelled_hex("  descriptor size = ", memory_map.descriptor_size);
    serial::write_labelled_hex("  descriptor version = ", memory_map.descriptor_version as usize);
    serial::write_labelled_hex("  descriptor count = ", memory_map.len / memory_map.descriptor_size);
}

fn dump_memory_summary(summary: MemorySummary) {
    serial::write_str("memory summary:\n");
    serial::write_str("  total firmware RAM = ");
    serial::write_dec_usize(summary.total_ram / MEBIBYTE);
    serial::write_str(" MiB\n");
    serial::write_str("  allocator free RAM = ");
    serial::write_dec_usize(summary.free_ram / MEBIBYTE);
    serial::write_str(" MiB\n");
    serial::write_str("  not allocator free = ");
    serial::write_dec_usize(summary.total_ram.saturating_sub(summary.free_ram) / MEBIBYTE);
    serial::write_str(" MiB\n");
    serial::write_str("  allocator free frames = ");
    serial::write_dec_usize(summary.free_ram / PAGE_SIZE);
    serial::write_str("\n");
}

fn halt() -> ! {
    loop {
        core::hint::spin_loop();
    }
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

fn dump_reserved_regions(bootinfo: usize, efi_memory_map: EfiMemoryMapInfo) {
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

    dump_region(
        "  fresh EFI memory map",
        Region {
            start: efi_memory_map.address,
            end: efi_memory_map.address.saturating_add(efi_memory_map.len),
        },
    );

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

fn dump_allocator_regions(bootinfo: usize, efi_memory_map: EfiMemoryMapInfo) {
    let (usable, usable_count) = build_allocator_regions(bootinfo, efi_memory_map);

    serial::write_str("allocator regions from fresh EFI memory map:\n");
    for region in usable.iter().copied().take(usable_count) {
        dump_region("  free", region);
    }
}

fn build_allocator_regions(
    bootinfo: usize, efi_memory_map: EfiMemoryMapInfo,
) -> ([Region; MAX_MEMORY_REGIONS], usize) {
    let mut usable = [Region { start: 0, end: 0 }; MAX_MEMORY_REGIONS];
    let mut usable_count = collect_usable_efi_regions(efi_memory_map, &mut usable);

    let mut reserved = [Region { start: 0, end: 0 }; 5];
    let mut reserved_count = 0usize;

    reserved[reserved_count] = Region {
        start: core::ptr::addr_of!(___KERNEL_DATA_START__) as usize,
        end: core::ptr::addr_of!(___KERNEL_DATA_END__) as usize,
    };
    reserved_count += 1;

    reserved[reserved_count] = Region {
        start: efi_memory_map.address,
        end: efi_memory_map.address.saturating_add(efi_memory_map.len),
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

fn memory_summary(bootinfo: usize, efi_memory_map: EfiMemoryMapInfo) -> MemorySummary {
    let (regions, count) = build_allocator_regions(bootinfo, efi_memory_map);
    let free_ram = regions
        .iter()
        .copied()
        .take(count)
        .fold(0usize, |total, region| total.saturating_add(region.end - region.start));

    let mut total_ram = 0usize;
    let descriptor_count = efi_memory_map.len / efi_memory_map.descriptor_size;
    for index in 0..descriptor_count {
        let descriptor = efi_memory_map.address + index * efi_memory_map.descriptor_size;
        let memory_type = read_u32(descriptor);
        if !(1..=10).contains(&memory_type) {
            continue;
        }

        let page_count = read_u64(descriptor + 24) as usize;
        if let Some(bytes) = page_count.checked_mul(PAGE_SIZE) {
            total_ram = total_ram.saturating_add(bytes);
        }
    }

    MemorySummary { total_ram, free_ram }
}

fn collect_usable_efi_regions(
    efi_memory_map: EfiMemoryMapInfo, out: &mut [Region; MAX_MEMORY_REGIONS],
) -> usize {
    if !efi_memory_map.is_valid() {
        return 0;
    }

    let mut count = 0usize;
    let descriptor_count = efi_memory_map.len / efi_memory_map.descriptor_size;
    for index in 0..descriptor_count {
        if count == out.len() {
            break;
        }

        let descriptor = efi_memory_map.address + index * efi_memory_map.descriptor_size;
        let memory_type = read_u32(descriptor);
        if memory_type != EFI_CONVENTIONAL_MEMORY {
            continue;
        }

        let start = read_u64(descriptor + 8) as usize;
        let page_count = read_u64(descriptor + 24) as usize;
        let Some(length) = page_count.checked_mul(PAGE_SIZE) else {
            continue;
        };

        let region = Region {
            start: align_up(start, PAGE_SIZE),
            end: align_down(start.saturating_add(length), PAGE_SIZE),
        };
        if region.end > region.start {
            out[count] = region;
            count += 1;
        }
    }

    count
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

fn print_framebuffer_status(
    framebuffer: FramebufferInfo, bootinfo: usize, efi_memory_map: EfiMemoryMapInfo,
    memory_summary: MemorySummary,
) {
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
    console.write_line(2, "RAM TOTAL MIB");
    console.write_dec_usize(8, console.line_y(3), memory_summary.total_ram / MEBIBYTE);
    console.write_line(4, "RAM FREE MIB");
    console.write_dec_usize(8, console.line_y(5), memory_summary.free_ram / MEBIBYTE);

    let (regions, count) = build_allocator_regions(bootinfo, efi_memory_map);
    console.write_line(6, "ALLOC REGIONS");
    let shown = core::cmp::min(count, 3);
    for (index, region) in regions.iter().take(shown).enumerate() {
        let y = console.line_y(7 + index);
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
