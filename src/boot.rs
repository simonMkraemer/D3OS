#![feature(allocator_api)]
#![feature(alloc_layout_extra)]
#![feature(const_mut_refs)]
#![feature(naked_functions)]
#![feature(asm_const)]
#![feature(exact_size_is_empty)]
#![feature(panic_info_message)]
#![feature(fmt_internals)]
#![feature(abi_x86_interrupt)]
#![allow(internal_features)]
#![no_main]
#![no_std]

extern crate tinyrlibc;
extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::ToString;
use core::arch::asm;
use core::ffi::c_void;
use core::mem::size_of;
use core::ops::Deref;
use core::panic::PanicInfo;
use core::ptr;
use core::fmt::Arguments;
use chrono::DateTime;
use log::{error, info, Log, Record};
use log::Level::Error;
use multiboot2::{BootInformation, BootInformationHeader, MemoryAreaType, Tag};
use uefi_raw::table::boot::MemoryType;
use x86_64::instructions::interrupts;
use uefi::prelude::*;
use uefi::table::boot::PAGE_SIZE;
use uefi::table::Runtime;
use x86_64::instructions::segmentation::{CS, DS, ES, FS, GS, Segment, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::PrivilegeLevel::Ring0;
use x86_64::registers::control::{Cr0, Cr0Flags, Cr3, Cr4, Cr4Flags};
use x86_64::registers::segmentation::SegmentSelector;
use x86_64::structures::gdt::Descriptor;
use x86_64::structures::paging::PageTableFlags;
use crate::kernel::interrupt::interrupt_dispatcher;
use crate::kernel::syscall::syscall_dispatcher;
use crate::kernel::thread::thread::Thread;

// insert other modules
#[macro_use]
mod device;
mod kernel;
mod library;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if kernel::terminal_initialized() {
        println!("Panic: {}", info);
    } else {
        let record = Record::builder()
            .level(Error)
            .file(Some("panic"))
            .args(*info.message().unwrap_or(&Arguments::new_const(&["A panic occurred!"])))
            .build();

        let logger = kernel::logger().lock();
        unsafe { kernel::logger().force_unlock() }; // log() also calls kernel::logger().lock()
        logger.log(&record);
    }

    loop {}
}

pub mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

extern "C" {
    static ___BSS_START__: u64;
    static ___BSS_END__: u64;
}

#[no_mangle]
pub extern fn start() {
    interrupts::disable();

    // Get multiboot values from eax and ebx
    let multiboot2_magic: u32;
    let multiboot2_address: u32;

    unsafe {
        asm!(
        "mov ecx, ebx", // ebx cannot be used with 'out', because rbx is reserved for internal LLVM usage
        out("eax") multiboot2_magic,
        out("ecx") multiboot2_address
        );
    }

    // Clear bss section before any static structures are accessed
    clear_bss();

    // Initialize logger
    if kernel::logger().lock().init().is_err() {
        panic!("Failed to initialize logger!")
    }

    // Log messages and panics are now working, but cannot use format string until the heap is initialized later on
    info!("Welcome to hhuTOSr early boot environment!");

    // Get multiboot information
    if multiboot2_magic != multiboot2::MAGIC {
        panic!("Invalid Multiboot2 magic number!");
    }

    let multiboot;
    unsafe { multiboot = BootInformation::load(multiboot2_address as *const BootInformationHeader).unwrap_or_else(|_| panic!("Failed to get Multiboot2 information!")); };

    let heap_start: usize;
    let heap_end: usize;

    if let Some(_) = multiboot.efi_bs_not_exited_tag() {
        // EFI boot services have not been exited and we obtain access to the memory map and EFI runtime services by exiting them manually
        info!("EFI boot services have not been exited");
        let image_tag = multiboot.efi_ih64_tag().unwrap_or_else(|| panic!("EFI image handle not available!"));
        let sdt_tag = multiboot.efi_sdt64_tag().unwrap_or_else(|| panic!("EFI system table not available!"));
        let image_handle;
        let system_table;

        unsafe {
            image_handle = Handle::from_ptr(image_tag.image_handle() as *mut c_void).unwrap_or_else(|| panic!("Failed to create EFI image handle struct from pointer!"));
            system_table = SystemTable::<Boot>::from_ptr(sdt_tag.sdt_address() as *mut c_void).unwrap_or_else(|| panic!("Failed to create EFI system table struct from pointer!"));
            system_table.boot_services().set_image_handle(image_handle);
        }

        info!("Exiting EFI boot services to obtain runtime system table and memory map");
        let (runtime_table, memory_map) = system_table.exit_boot_services(MemoryType::LOADER_DATA);

        info!("Searching memory map for largest usable area");
        let mut heap_area = memory_map.entries().next().unwrap_or_else(|| panic!("EFI memory map is empty!"));
        for area in memory_map.entries() {
            if area.ty == MemoryType::CONVENTIONAL && area.page_count > heap_area.page_count {
                heap_area = area;
            }
        }

        heap_start = heap_area.phys_start as usize;
        heap_end = heap_area.phys_start as usize + heap_area.page_count as usize * PAGE_SIZE - 1;

        kernel::init_efi_system_table(runtime_table);
    } else if let Some(memory_map) = multiboot.memory_map_tag() {
        // EFI services have been exited, but the bootloader has provided us with a Multiboot2 memory map
        info!("EFI boot services have been exited");
        info!("Bootloader provides Multiboot2 memory map");
        let mut heap_area = memory_map.memory_areas().get(0).unwrap_or_else(|| panic!("Multiboot2 memory map is empty!"));

        info!("Searching memory map for largest usable area");
        for area in memory_map.memory_areas() {
            if area.typ() == MemoryAreaType::Available && area.size() > heap_area.size() {
                heap_area = area;
            }
        }

        heap_start = heap_area.start_address() as usize;
        heap_end = heap_area.end_address() as usize;
    } else if let Some(memory_map) = multiboot.efi_memory_map_tag() {
        // EFI services have been exited, but the bootloader has provided us with the EFI memory map
        info!("EFI boot services have been exited");
        info!("Bootloader provides EFI memory map");
        let mut heap_area = memory_map.memory_areas().next().unwrap_or_else(|| panic!("EFI memory map is empty!"));

        info!("Searching memory map for largest usable area");
        for area in memory_map.memory_areas() {
            if area.ty.0 == MemoryType::CONVENTIONAL.0 && area.page_count > heap_area.page_count {
                heap_area = area;
            }
        }

        heap_start = heap_area.phys_start as usize;
        heap_end = (heap_area.phys_start + heap_area.page_count * 4096 - 1) as usize;
    } else {
        panic!("No memory information available!");
    }

    // Setup global descriptor table
    // Has to be done after EFI boot services have been exited, since they rely on their own GDT
    info!("Initializing GDT");
    setup_gdt();

    // Enable user access bits in EFI identity mapping (needed for system calls to work)
    info!("Initializing Paging");
    setup_paging();

    // Initialize heap, after which format strings may be used in log messages and panics
    info!("Initializing heap");
    unsafe { kernel::allocator().init(heap_start, heap_end); }
    info!("Heap is initialized (Start: [{} MiB], End: [{} MiB]]", heap_start / 1024 / 1024, heap_end / 1024 / 1024);

    // Initialize serial port and enable serial logging
    kernel::init_serial_port();
    if let Some(serial) = kernel::serial_port() {
        kernel::logger().lock().register(serial);
    }

    // Initialize terminal and enable terminal logging
    let fb_info = multiboot.framebuffer_tag()
        .unwrap_or_else(|| panic!("No framebuffer information provided by bootloader!"))
        .unwrap_or_else(|fb_type| panic!("Unknown framebuffer type [{}]!", fb_type));
    kernel::init_terminal(fb_info.address() as *mut u8, fb_info.pitch(), fb_info.width(), fb_info.height(), fb_info.bpp());
    kernel::logger().lock().register(kernel::terminal());

    info!("Welcome to hhuTOSr!");
    let version = format!("v{} ({} - O{})", built_info::PKG_VERSION, built_info::PROFILE, built_info::OPT_LEVEL);
    let git_ref = built_info::GIT_HEAD_REF.unwrap_or_else(|| "Unknown");
    let git_commit = built_info::GIT_COMMIT_HASH_SHORT.unwrap_or_else(|| "Unknown");
    let build_date = match DateTime::parse_from_rfc2822(built_info::BUILT_TIME_UTC) {
        Ok(date_time) => date_time.format("%Y-%m-%d %H:%M:%S").to_string(),
        Err(_) => "Unknown".to_string()
    };
    let bootloader_name = match multiboot.boot_loader_name_tag() {
        Some(tag) => if tag.name().is_ok() { tag.name().unwrap_or("Unknown") } else { "Unknown" },
        None => "Unknown"
    };

    info!("OS Version: [{}]", version);
    info!("Git Version: [{} - {}]", built_info::GIT_HEAD_REF.unwrap_or_else(|| "Unknown"), git_commit);
    info!("Build Date: [{}]", build_date);
    info!("Compiler: [{}]", built_info::RUSTC_VERSION);
    info!("Bootloader: [{}]", bootloader_name);

    // Initialize ACPI tables
    let rsdp_addr: usize = if let Some(rsdp_tag) = multiboot.rsdp_v2_tag() {
        ptr::from_ref(rsdp_tag) as usize + size_of::<Tag>()
    } else if let Some(rsdp_tag) = multiboot.rsdp_v1_tag() {
        ptr::from_ref(rsdp_tag) as usize + size_of::<Tag>()
    } else {
        panic!("ACPI not available!");
    };

    kernel::init_acpi_tables(rsdp_addr);

    // Initialize interrupts
    info!("Initializing IDT");
    interrupt_dispatcher::setup_idt();
    info!("Initializing system calls");
    syscall_dispatcher::init();
    kernel::init_apic();

    // Initialize timer
    {
        info!("Initializing timer");
        let mut timer = kernel::timer().write();
        timer.interrupt_rate(1);
        timer.plugin();
    }

    // Enable interrupts
    info!("Enabling interrupts");
    interrupts::enable();

    // Initialize EFI runtime service (if available and not done already during memory initialization)
    if kernel::efi_system_table().is_none() {
        if let Some(sdt_tag) = multiboot.efi_sdt64_tag() {
            info!("Initializing EFI runtime services");
            let system_table;
            unsafe { system_table = SystemTable::<Runtime>::from_ptr(sdt_tag.sdt_address() as *mut c_void); };

            if system_table.is_some() {
                kernel::init_efi_system_table(system_table.unwrap());
            } else {
                error!("Failed to create EFI system table struct from pointer!");
            }
        }
    }

    // Initialize keyboard
    info!("Initializing PS/2 devices");
    kernel::init_keyboard();
    kernel::ps2_devices().keyboard().plugin();

    // Enable serial port interrupts
    if let Some(serial) = kernel::serial_port() {
        serial.plugin();
    }

    let scheduler = kernel::scheduler();
    scheduler.ready(Thread::new_kernel_thread(Box::new(|| {
        let terminal = kernel::terminal();
        terminal.write_str("> ");

        loop {
            match terminal.read_byte() {
                -1 => panic!("Terminal input stream closed!"),
                0x0a => terminal.write_str("> "),
                _ => {}
            }
        }
    })));

    // Disable terminal logging
    kernel::logger().lock().remove(kernel::terminal());
    kernel::terminal().clear();

    println!(include_str!("banner.txt"),
             version,
             git_ref.rsplit("/").next().unwrap_or(git_ref),
             git_commit,
             build_date,
             built_info::RUSTC_VERSION.split_once("(").unwrap_or((built_info::RUSTC_VERSION, "")).0.trim(),
             bootloader_name);

    info!("Starting scheduler");
    scheduler.start();
}

fn clear_bss() {
    unsafe {
        let bss_start = ptr::from_ref(&___BSS_START__) as *mut u8;
        let bss_end = ptr::from_ref(&___BSS_END__) as *const u8;
        let length = bss_end as usize - bss_start as usize;

        bss_start.write_bytes(0, length);
    }
}

fn setup_gdt() {
    let mut gdt = kernel::gdt().lock();
    let tss = kernel::tss().lock();

    gdt.add_entry(Descriptor::kernel_code_segment());
    gdt.add_entry(Descriptor::kernel_data_segment());
    gdt.add_entry(Descriptor::user_data_segment());
    gdt.add_entry(Descriptor::user_code_segment());

    unsafe {
        // We need to obtain a static reference to the TSS and GDT for the following operations.
        // We know, that they have a static lifetime, since they are declared as static variables in 'kernel/mod.rs'.
        // However, since they are hidden behind a Mutex, the borrow checker does not see them with a static lifetime.
        let gdt_ref = ptr::from_ref(gdt.deref()).as_ref().unwrap();
        let tss_ref = ptr::from_ref(tss.deref()).as_ref().unwrap();
        gdt.add_entry(Descriptor::tss_segment(tss_ref));
        gdt_ref.load();
    }

    unsafe {
        // Load task state segment
        load_tss(SegmentSelector::new(5, Ring0));

        // Set code and stack segment register
        CS::set_reg(SegmentSelector::new(1, Ring0));
        SS::set_reg(SegmentSelector::new(2, Ring0));

        // Other segment registers are not used in long mode (set to 0)
        DS::set_reg(SegmentSelector::new(0, Ring0));
        ES::set_reg(SegmentSelector::new(0, Ring0));
        FS::set_reg(SegmentSelector::new(0, Ring0));
        GS::set_reg(SegmentSelector::new(0, Ring0));
    }
}

fn setup_paging() {
    unsafe {
        Cr0::update(|flags| flags.remove(Cr0Flags::WRITE_PROTECT));

        let page_map_address = Cr3::read().0.start_address();
        let level = if Cr4::read().contains(Cr4Flags::L5_PAGING) { 5 } else { 4 };
        setup_page_map(paging_pointer(page_map_address.as_u64()), level);

        Cr0::update(|flags| flags.insert(Cr0Flags::WRITE_PROTECT));
    }
}

unsafe fn setup_page_map(map: *mut u64, level: usize) {
    for i in 0..512 {
        let entry = map.offset(i).read();
        if entry != 0 {
            let mut flags = PageTableFlags::from_bits_truncate(entry);
            flags.insert(PageTableFlags::USER_ACCESSIBLE);

            map.offset(i).write(entry | flags.bits());

            if level > 1 && !flags.contains(PageTableFlags::HUGE_PAGE) {
                setup_page_map(paging_pointer(entry), level - 1);
            }
        }
    }
}

fn paging_pointer(entry: u64) -> *mut u64 {
    return (entry & 0x000ffffffffff000) as *mut u64;
}