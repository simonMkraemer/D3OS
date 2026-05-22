use crate::bootinfo::{BootInfo, BootModule, StaticStr};
use crate::serial;

pub const MAGIC: usize = 0x36d7_6289;

fn read_u32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn read_u64(addr: usize) -> u64 {
    unsafe { core::ptr::read_volatile(addr as *const u64) }
}

fn tag_name(tag_type: u32) -> &'static [u8] {
    match tag_type {
        0 => b"end",
        1 => b"cmdline",
        2 => b"boot_loader_name",
        3 => b"module",
        4 => b"basic_meminfo",
        6 => b"memory_map",
        8 => b"framebuffer",
        9 => b"elf_sections",
        11 => b"efi32_sdt",
        12 => b"efi64_sdt",
        13 => b"smbios",
        14 => b"acpi_rsdp_v1",
        15 => b"acpi_rsdp_v2",
        17 => b"efi_memory_map",
        18 => b"efi_bs_not_exited",
        19 => b"efi32_image_handle",
        20 => b"efi64_image_handle",
        21 => b"image_load_base",
        _ => b"unknown",
    }
}

fn parse_static_str(ptr: usize, size: usize) -> StaticStr {
    if size <= 8 {
        return StaticStr::empty();
    }

    let bytes_ptr = (ptr + 8) as *const u8;
    let bytes_len = size - 8;
    let mut len = 0;

    while len < bytes_len {
        let byte = unsafe { core::ptr::read_volatile(bytes_ptr.add(len)) };
        if byte == 0 {
            break;
        }
        len += 1;
    }

    StaticStr {
        ptr: bytes_ptr,
        len,
    }
}

pub fn dump_words(bootinfo: usize, words: usize) {
    if bootinfo == 0 {
        serial::write_str("bootinfo preview unavailable: null pointer\n");
        return;
    }

    serial::write_str("bootinfo preview:\n");
    for index in 0..words {
        let value = unsafe { core::ptr::read_volatile((bootinfo as *const usize).add(index)) };
        serial::write_str("  [");
        serial::write_hex_usize(index);
        serial::write_str("] = ");
        serial::write_hex_usize(value);
        serial::write_str("\n");
    }
}

pub fn dump_tags(bootinfo: usize) {
    if bootinfo == 0 {
        return;
    }

    if bootinfo & 7 != 0 {
        serial::write_str("bootinfo alignment error: expected 8-byte alignment\n");
        return;
    }

    let total_size = read_u32(bootinfo) as usize;
    let reserved = read_u32(bootinfo + 4);

    serial::write_labelled_hex("mb2 total_size = ", total_size);
    serial::write_labelled_hex("mb2 reserved   = ", reserved as usize);

    if reserved != 0 {
        serial::write_str("unexpected non-zero multiboot2 reserved field\n");
    }

    let mut offset = 8usize;
    let end = bootinfo.saturating_add(total_size);

    serial::write_str("mb2 tags:\n");
    while bootinfo.saturating_add(offset.saturating_add(8)) <= end && offset < total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        serial::write_str("  type=");
        serial::write_dec_usize(tag_type as usize);
        serial::write_str(" (");
        serial::write_bytes(tag_name(tag_type));
        serial::write_str(") size=");
        serial::write_dec_usize(tag_size);

        if tag_type == 2 && tag_size >= 9 {
            serial::write_str(" value=\"");
            serial::write_cstr((tag_addr + 8) as *const u8, tag_size - 8);
            serial::write_str("\"");
        }

        serial::write_str("\n");

        if tag_type == 0 {
            break;
        }

        if tag_size < 8 {
            serial::write_str("invalid multiboot2 tag size\n");
            break;
        }

        offset = (offset + tag_size + 7) & !7;
    }
}

pub fn parse_boot_info(bootinfo: usize) -> Option<BootInfo> {
    if bootinfo == 0 || bootinfo & 7 != 0 {
        return None;
    }

    let total_size = read_u32(bootinfo) as usize;
    let reserved = read_u32(bootinfo + 4);
    if total_size < 16 || reserved != 0 {
        return None;
    }

    let mut parsed = BootInfo::empty();
    let mut offset = 8usize;

    while offset + 8 <= total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        if tag_size < 8 || offset + tag_size > total_size {
            return None;
        }

        match tag_type {
            0 => break,
            2 => {
                parsed.bootloader_name = parse_static_str(tag_addr, tag_size);
            }
            3 => {
                if parsed.first_module.is_none() && tag_size >= 16 {
                    parsed.first_module = Some(BootModule {
                        start: read_u32(tag_addr + 8) as usize,
                        end: read_u32(tag_addr + 12) as usize,
                        name: parse_static_str(tag_addr + 8, tag_size - 8),
                    });
                }
            }
            6 => {
                if tag_size >= 16 {
                    let entry_size = read_u32(tag_addr + 8) as usize;
                    if entry_size != 0 {
                        parsed.memory_map_entries = (tag_size - 16) / entry_size;
                    }
                }
            }
            12 => {
                if tag_size >= 16 {
                    parsed.efi_system_table = Some(read_u64(tag_addr + 8) as usize);
                }
            }
            15 => {
                parsed.acpi_rsdp = Some(tag_addr + 8);
            }
            18 => {
                parsed.efi_boot_services_not_exited = true;
            }
            _ => {}
        }

        offset = (offset + tag_size + 7) & !7;
    }

    Some(parsed)
}
