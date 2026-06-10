#![no_std]
#![no_main]

mod bootinfo;
mod multiboot2;
mod serial;

use core::panic::PanicInfo;


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

    if x2 == multiboot2::MAGIC {
        serial::write_str("x2 matches Multiboot2 magic\n");
    } else {
        serial::write_str("x2 does not match Multiboot2 magic\n");
    }

    multiboot2::dump_words(x1, 4);
    multiboot2::dump_tags(x1);

    match multiboot2::parse_boot_info(x1) {
        Some(parsed) => bootinfo::dump(&parsed),
        None => serial::write_str("failed to parse boot info\n"),
    }






    match parse_framebuffer_tag(x1) {
        Some(framebuffer) => {
            serial::write_str("framebuffer tag parsed:\n");
            serial::write_labelled_hex("  addr = ", framebuffer.address);
            serial::write_labelled_hex("  pitch = ", framebuffer.pitch);
            serial::write_labelled_hex("  width = ", framebuffer.width);
            serial::write_labelled_hex("  height = ", framebuffer.height);
            serial::write_labelled_hex("  bpp = ", framebuffer.bpp);
            paint_green(framebuffer);
            serial::write_str("painted framebuffer green\n");
        }
        None => serial::write_str("framebuffer tag missing or unsupported\n"),
    }

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

fn paint_green(framebuffer: FramebufferInfo) {
    if framebuffer.bpp != 32 {
        serial::write_str("framebuffer bpp is not 32, skipping paint\n");
        return;
    }

    for y in 0..framebuffer.height {
        let row = framebuffer.address + y * framebuffer.pitch;
        for x in 0..framebuffer.width {
            let pixel = (row + x * 4) as *mut u8;
            unsafe {
                // Current QEMU/AAVMF setup reports BGR32.
                core::ptr::write_volatile(pixel, 0x00);
                core::ptr::write_volatile(pixel.add(1), 0xff);
                core::ptr::write_volatile(pixel.add(2), 0x00);
                core::ptr::write_volatile(pixel.add(3), 0x00);
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial::write_str("HELLO WORLD panic\n");
    loop {
        core::hint::spin_loop();
    }
}
