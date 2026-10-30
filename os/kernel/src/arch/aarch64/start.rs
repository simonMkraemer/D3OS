#![no_std]
#![no_main]

// serial first
#[macro_use]
mod serial;
mod exception;
mod framebuffer;

use crate::framebuffer::Framebuffer;
use crate::framebuffer::FramebufferInfo;
use core::ffi::c_void;
use core::panic::PanicInfo;
use core::ptr;
use multiboot2::{BootInformation, BootInformationHeader, ModuleTag, TagHeader};
use uefi::data_types::Handle;
use uefi::mem::memory_map::{MemoryMap, MemoryMapOwned};
use uefi_raw::table::boot::MemoryType;
use uefi_raw::table::system::SystemTable;

// Entry point, multiboot2 header and exception vectors
core::arch::global_asm!(include_str!("boot.S"), options(raw));

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

#[derive(Clone, Copy)]
struct MemorySummary {
    total_ram: usize,
    free_ram: usize,
}

struct Aarch64BootInfo {
    efi_memory_map: MemoryMapOwned,
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

impl Aarch64BootInfo {
    fn retained_region_count(&self) -> usize {
        let mandatory = [self.kernel_region, self.bootinfo_region, memory_map_region(&self.efi_memory_map)];
        mandatory.iter().filter(|region| region.end > region.start).count() + self.initrd_region.is_some() as usize + self.framebuffer_region.is_some() as usize
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn start_aarch64(x0: usize, x1: usize, _x2: usize, _x3: usize) -> ! {
    if x0 != multiboot2::MAGIC as usize {
        println!("invalid AArch64 boot magic");
        halt();
    }

    // UEFI state at handoff that towboot leaves IRQs unmasked for the UEFI timer
    println!("DAIF at entry: {:#06b}", exception::read_daif());

    // load once, then read each tag where it is needed
    let multiboot = unsafe { BootInformation::load(x1 as *const BootInformationHeader) }.expect("Failed to get Multiboot2 information");
    // towboot always hands over with active boot services on AArch64
    if multiboot.efi_bs_not_exited_tag().is_none() {
        panic!("EFI boot services have already been exited");
    }


    let efi_memory_map = exit_boot_services(&multiboot);

    //defined CPU state now that UEFI no longer owns
    println!("DAIF after ExitBootServices: {:#06b}", exception::read_daif());
    //TODO: austuschen wenn ich GIC hab... irgendwann mal..
    exception::mask_all();
    println!("DAIF after masking: {:#06b}", exception::read_daif());
    println!("CurrentEL: EL{}", exception::read_current_el());
    println!("SPSel: {}", exception::read_spsel());

    //für memory, brauche ich gleich
    //exception::print_sctlr();

    exception::install_vector_table();

    let boot_info = build_aarch64_boot_info(&multiboot, efi_memory_map);
    report_boot_summary(&boot_info);

    match boot_info.framebuffer {
        Some(framebuffer) => {
            print_framebuffer_status(framebuffer, &boot_info);
        }
        None => println!("framebuffer tag missing"),
    }

    // Synchronous exception test: BRK must reach handle_exception and return here.
    exception::trigger_breakpoint();
    println!("returned from BRK exception");

    // small svc test
    // Fange ich gerade noch nicht ab !
    // unsafe { core::arch::asm!("svc #0") };

    println!("SCTLR_EL1: {:#018x}", exception::read_sctlr());

    loop {
        core::hint::spin_loop();
    }
}

/// copy of x86 boot.rs -> hand the pointers from Multiboot2 to the uefi crate, exit afterward
fn exit_boot_services(multiboot: &BootInformation<'_>) -> MemoryMapOwned {
    let image_tag = multiboot.efi_ih64_tag().expect("EFI image handle not available!");
    let sdt_tag = multiboot.efi_sdt64_tag().expect("EFI system table not available!");

    let memory_map = unsafe {
        let image_handle = Handle::from_ptr(image_tag.image_handle() as *mut c_void).expect("Failed to create EFI image handle struct from pointer!");
        uefi::table::set_system_table(sdt_tag.sdt_address() as *const SystemTable);
        uefi::boot::set_image_handle(image_handle);
        // Allocates the map buffer as LOADER_DATA, retries once and resets the machine if exiting fails
        uefi::boot::exit_boot_services(None)
    };
    println!("EFI Boot Services exited");
    memory_map
}

/// The buffer the uefi crate allocated for the memory map; it must not be handed out as free memory.
fn memory_map_region(memory_map: &MemoryMapOwned) -> Region {
    let buffer = memory_map.buffer();
    Region {
        start: buffer.as_ptr() as usize,
        end: buffer.as_ptr() as usize + buffer.len(),
    }
}

fn halt() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// The module named "initrd", searched like in x86 boot.rs.
fn find_initrd<'a>(multiboot: &'a BootInformation<'_>) -> Option<&'a ModuleTag> {
    multiboot.module_tags().find(|module| module.cmdline().is_ok_and(|name| name == "initrd"))
}

/// The RSDP is stored inside the tag, directly after its header (like x86 boot.rs).
fn acpi_rsdp_address(multiboot: &BootInformation<'_>) -> Option<usize> {
    let v2 = multiboot.rsdp_v2_tag().map(|tag| ptr::from_ref(tag) as usize);
    let v1 = multiboot.rsdp_v1_tag().map(|tag| ptr::from_ref(tag) as usize);
    v2.or(v1).map(|tag_address| tag_address + size_of::<TagHeader>())
}

fn build_aarch64_boot_info(multiboot: &BootInformation<'_>, efi_memory_map: MemoryMapOwned) -> Aarch64BootInfo {
    let kernel_region = Region {
        start: core::ptr::addr_of!(___KERNEL_DATA_START__) as usize,
        end: core::ptr::addr_of!(___KERNEL_DATA_END__) as usize,
    };
    let bootinfo_region = Region {
        start: multiboot.start_address(),
        end: multiboot.end_address(),
    };
    let initrd = find_initrd(multiboot).expect("Initrd not found!");
    let initrd_region = Some(Region {
        start: initrd.start_address() as usize,
        end: initrd.end_address() as usize,
    });
    // TODO: optional for now so kernel can still boot with UART only... fix later
    let framebuffer = multiboot.framebuffer_tag().and_then(Result::ok).map(|tag| FramebufferInfo {
        address: tag.address() as usize,
        pitch: tag.pitch() as usize,
        width: tag.width() as usize,
        height: tag.height() as usize,
        bpp: tag.bpp() as usize,
    });
    let framebuffer_region = framebuffer.map(|framebuffer| Region {
        start: framebuffer.address,
        end: framebuffer.address.saturating_add(framebuffer.pitch.saturating_mul(framebuffer.height)),
    });
    let mut reserved = [Region { start: 0, end: 0 }; 5];
    let mut reserved_count = 0;
    for region in [
        Some(kernel_region),
        Some(bootinfo_region),
        Some(memory_map_region(&efi_memory_map)),
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
    let mut usable_count = collect_usable_efi_regions(&efi_memory_map, &mut usable);
    for reserved_region in reserved.iter().copied().take(reserved_count) {
        usable_count = subtract_reserved_regions(&mut usable, usable_count, reserved_region);
    }

    let free_ram = usable
        .iter()
        .copied()
        .take(usable_count)
        .fold(0usize, |total, region| total.saturating_add(region.end - region.start));

    // RAM = every type from LOADER_CODE (1) to ACPI_NON_VOLATILE (10); excludes RESERVED and MMIO
    let ram_types = MemoryType::LOADER_CODE.0..=MemoryType::ACPI_NON_VOLATILE.0;
    let total_ram = efi_memory_map
        .entries()
        .filter(|descriptor| ram_types.contains(&descriptor.ty.0))
        .fold(0usize, |total, descriptor| total.saturating_add((descriptor.page_count as usize).saturating_mul(PAGE_SIZE)));

    Aarch64BootInfo {
        efi_memory_map,
        kernel_region,
        bootinfo_region,
        initrd_region,
        framebuffer_region,
        framebuffer,
        acpi_rsdp: acpi_rsdp_address(multiboot),
        usable_regions: usable,
        usable_region_count: usable_count,
        memory_summary: MemorySummary { total_ram, free_ram },
    }
}

fn collect_usable_efi_regions(efi_memory_map: &MemoryMapOwned, out: &mut [Region; MAX_MEMORY_REGIONS]) -> usize {
    let mut count = 0usize;
    for descriptor in efi_memory_map.entries().filter(|descriptor| descriptor.ty == MemoryType::CONVENTIONAL) {
        if count == out.len() {
            break;
        }

        let start = descriptor.phys_start as usize;
        let Some(length) = (descriptor.page_count as usize).checked_mul(PAGE_SIZE) else {
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

fn report_boot_summary(boot_info: &Aarch64BootInfo) {
    let acpi = if boot_info.acpi_rsdp.is_some() { ", ACPI" } else { "" };
    println!(
        "AArch64 boot: {} free frames, {} retained regions, {} EFI descriptors (v{}){}",
        boot_info.memory_summary.free_ram / PAGE_SIZE,
        boot_info.retained_region_count(),
        boot_info.efi_memory_map.len(),
        boot_info.efi_memory_map.meta().desc_version,
        acpi,
    );
}

fn print_framebuffer_status(framebuffer: FramebufferInfo, boot_info: &Aarch64BootInfo) {
    let Some(console) = Framebuffer::new(framebuffer.address, framebuffer.pitch, framebuffer.width, framebuffer.height, framebuffer.bpp) else {
        println!("framebuffer console unavailable");
        return;
    };

    console.clear(0x00, 0x00, 0x00);
    console.write_line(1, "ARM BOOT OK");
    console.write_line(2, "FRAMEBUFFER OK");
    console.write_line(3, "RAM TOTAL MIB");
    console.write_dec_usize(8, console.line_y(4), boot_info.memory_summary.total_ram / MEBIBYTE);
    console.write_line(5, "RAM FREE MIB");
    console.write_dec_usize(8, console.line_y(6), boot_info.memory_summary.free_ram / MEBIBYTE);

    // console.write_line(7, "ALLOC REGIONS");
    // let shown = core::cmp::min(boot_info.usable_region_count, 3);
    // for (index, region) in boot_info.usable_regions.iter().take(shown).enumerate() {
    //     // TODO: gerade noch nicht wichtig!
    //     // let y = console.line_y(7 + index);
    //     // console.write_hex_usize(8, y, region.start);
    //     // console.write_text(8 + 19 * 6, y, "-");
    //     // console.write_hex_usize(8 + 21 * 6, y, region.end);
    // }

    //uefi usable regioons
    const FIRST_LINE: usize = 8;
    console.write_line(FIRST_LINE, "NR  TYP  START               SEITEN");

    for (index, descriptor) in boot_info.efi_memory_map.entries().enumerate() {
        let y = console.line_y(FIRST_LINE + 1 + index);
        console.write_dec_usize(8, y, index);
        let typ = descriptor.ty.0;
        console.write_dec_usize(8 + 4 * 6, y, typ as usize);
        let start = descriptor.phys_start as usize;
        console.write_hex_usize(8 + 9 * 6, y, start);
        let pages = descriptor.page_count as usize;
        console.write_dec_usize(8 + 29 * 6, y, pages);

    }
}

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    println!("AArch64 kernel panic: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
