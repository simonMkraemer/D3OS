#![no_std]
#![no_main]

mod bootinfo;
mod multiboot2;
mod serial;

use core::panic::PanicInfo;

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

    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial::write_str("HELLO WORLD panic\n");
    loop {
        core::hint::spin_loop();
    }
}
