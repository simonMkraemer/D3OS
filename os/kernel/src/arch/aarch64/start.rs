#![no_std]
#![no_main]

mod framebuffer;
mod multiboot2;
mod serial;

use crate::framebuffer::Framebuffer;
use crate::multiboot2::{FramebufferInfo, HandoffInfo};
use core::panic::PanicInfo;

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

//https://uefi.org/specs/UEFI/2.10/04_EFI_System_Table.html
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
    get_memory_map: unsafe extern "efiapi" fn(*mut usize, *mut u8, *mut usize, *mut usize, *mut u32) -> usize,
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

struct Aarch64BootInfo {
    efi_memory_map: EfiMemoryMapInfo,
    kernel_region: Region,
    bootinfo_region: Region,
    initrd_region: Option<Region>,
    framebuffer_region: Option<Region>,
    framebuffer: Option<FramebufferInfo>,
    acpi_rsdp: Option<usize>,
    usable_regions: [Region; MAX_MEMORY_REGIONS],
    usable_region_count: usize,
    memory_summary: MemorySummary,
}

impl EfiMemoryMapInfo {
    fn is_valid(self) -> bool {
        self.address != 0 && self.descriptor_size >= EFI_MEMORY_DESCRIPTOR_MIN_SIZE && self.len % self.descriptor_size == 0
    }

    fn descriptor_count(self) -> usize {
        self.len / self.descriptor_size
    }
}

impl Aarch64BootInfo {
    fn retained_region_count(&self) -> usize {
        let mandatory = [
            self.kernel_region,
            self.bootinfo_region,
            Region {
                start: self.efi_memory_map.address,
                end: self.efi_memory_map.address.saturating_add(self.efi_memory_map.len),
            },
        ];
        mandatory.iter().filter(|region| region.end > region.start).count() + self.initrd_region.is_some() as usize + self.framebuffer_region.is_some() as usize
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn start_aarch64(x0: usize, x1: usize, _x2: usize, _x3: usize) -> ! {
    if x0 != multiboot2::MAGIC {
        serial::write_str("invalid AArch64 boot magic\n");
        halt();
    }

    let Some(handoff) = multiboot2::parse_handoff(x1) else {
        serial::write_str("invalid boot-information block\n");
        halt();
    };

    if !handoff.has_required_uefi_handoff() {
        serial::write_str("required UEFI handoff data is missing\n");
        halt();
    }

    let efi_memory_map = exit_boot_services(&handoff);
    if !efi_memory_map.is_valid() {
        serial::write_str("fresh EFI memory map is invalid\n");
        halt();
    }
    let boot_info = build_aarch64_boot_info(x1, handoff, efi_memory_map);
    report_boot_summary(&boot_info);

    match boot_info.framebuffer {
        Some(framebuffer) => {
            print_framebuffer_status(framebuffer, &boot_info);
        }
        None => serial::write_str("framebuffer tag missing\n"),
    }

    loop {
        core::hint::spin_loop();
    }
}

fn exit_boot_services(handoff: &HandoffInfo) -> EfiMemoryMapInfo {
    let system_table = handoff.efi_system_table.expect("validated UEFI system table");
    let image_handle = handoff.efi_image_handle.expect("validated UEFI image handle");
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

unsafe fn exit_boot_services_raw(system_table: usize, image_handle: usize) -> Result<EfiMemoryMapInfo, usize> {
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

fn build_aarch64_boot_info(bootinfo_address: usize, handoff: HandoffInfo, efi_memory_map: EfiMemoryMapInfo) -> Aarch64BootInfo {
    let kernel_region = Region {
        start: core::ptr::addr_of!(___KERNEL_DATA_START__) as usize,
        end: core::ptr::addr_of!(___KERNEL_DATA_END__) as usize,
    };
    let bootinfo_region = Region {
        start: bootinfo_address,
        end: bootinfo_address.saturating_add(handoff.bootinfo_len),
    };
    let initrd_region = handoff.initrd.map(|module| Region {
        start: module.start,
        end: module.end,
    });
    let framebuffer_region = handoff.framebuffer.map(|framebuffer| Region {
        start: framebuffer.address,
        end: framebuffer.address.saturating_add(framebuffer.pitch.saturating_mul(framebuffer.height)),
    });
    let mut reserved = [Region { start: 0, end: 0 }; 5];
    let mut reserved_count = 0;
    for region in [
        Some(kernel_region),
        Some(bootinfo_region),
        Some(Region {
            start: efi_memory_map.address,
            end: efi_memory_map.address.saturating_add(efi_memory_map.len),
        }),
        initrd_region,
        framebuffer_region,
    ]
    .iter()
    .flatten()
    {
        reserved[reserved_count] = *region;
        reserved_count += 1;
    }

    let mut usable = [Region { start: 0, end: 0 }; MAX_MEMORY_REGIONS];
    let mut usable_count = collect_usable_efi_regions(efi_memory_map, &mut usable);
    for reserved_region in reserved.iter().copied().take(reserved_count) {
        usable_count = subtract_reserved_regions(&mut usable, usable_count, reserved_region);
    }

    let free_ram = usable
        .iter()
        .copied()
        .take(usable_count)
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

    Aarch64BootInfo {
        efi_memory_map,
        kernel_region,
        bootinfo_region,
        initrd_region,
        framebuffer_region,
        framebuffer: handoff.framebuffer,
        acpi_rsdp: handoff.acpi_rsdp,
        usable_regions: usable,
        usable_region_count: usable_count,
        memory_summary: MemorySummary { total_ram, free_ram },
    }
}

fn report_boot_summary(boot_info: &Aarch64BootInfo) {
    serial::write_str("AArch64 boot: ");
    serial::write_dec_usize(boot_info.memory_summary.free_ram / PAGE_SIZE);
    serial::write_str(" free frames, ");
    serial::write_dec_usize(boot_info.retained_region_count());
    serial::write_str(" retained regions, ");
    serial::write_dec_usize(boot_info.efi_memory_map.descriptor_count());
    serial::write_str(" EFI descriptors (v");
    serial::write_dec_usize(boot_info.efi_memory_map.descriptor_version as usize);
    serial::write_str(")");
    if boot_info.acpi_rsdp.is_some() {
        serial::write_str(", ACPI\n");
    } else {
        serial::write_str("\n");
    }
}

fn collect_usable_efi_regions(efi_memory_map: EfiMemoryMapInfo, out: &mut [Region; MAX_MEMORY_REGIONS]) -> usize {
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

fn print_framebuffer_status(framebuffer: FramebufferInfo, boot_info: &Aarch64BootInfo) {
    let Some(console) = Framebuffer::new(framebuffer.address, framebuffer.pitch, framebuffer.width, framebuffer.height, framebuffer.bpp) else {
        serial::write_str("framebuffer console unavailable\n");
        return;
    };

    console.clear(0x00, 0x00, 0x00);
    console.write_line(1, "ARM BOOT OK");
    console.write_line(2, "FRAMEBUFFER OK");
    console.write_line(3, "RAM TOTAL MIB");
    console.write_dec_usize(8, console.line_y(4), boot_info.memory_summary.total_ram / MEBIBYTE);
    console.write_line(5, "RAM FREE MIB");
    console.write_dec_usize(8, console.line_y(6), boot_info.memory_summary.free_ram / MEBIBYTE);

    console.write_line(7, "ALLOC REGIONS");
    let shown = core::cmp::min(boot_info.usable_region_count, 3);
    for (index, region) in boot_info.usable_regions.iter().take(shown).enumerate() {
        // TODO: gerade noch nicht wichtig!
        // let y = console.line_y(7 + index);
        // console.write_hex_usize(8, y, region.start);
        // console.write_text(8 + 19 * 6, y, "-");
        // console.write_hex_usize(8 + 21 * 6, y, region.end);
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial::write_str("AArch64 kernel panic\n");
    loop {
        core::hint::spin_loop();
    }
}
