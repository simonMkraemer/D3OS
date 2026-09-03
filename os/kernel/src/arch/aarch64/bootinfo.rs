use crate::serial;

#[derive(Copy, Clone)]
pub struct BootModule {
    pub start: usize,
    pub end: usize,
    pub name: StaticStr,
}

#[derive(Copy, Clone)]
pub struct StaticStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl StaticStr {
    pub const fn empty() -> Self {
        Self {
            ptr: core::ptr::null(),
            len: 0,
        }
    }

    pub fn is_empty(self) -> bool {
        self.ptr.is_null() || self.len == 0
    }
}

pub struct BootInfo {
    pub bootloader_name: StaticStr,
    pub first_module: Option<BootModule>,
    pub memory_map_entries: usize,
    pub acpi_rsdp: Option<usize>,
    pub efi_system_table: Option<usize>,
    pub efi_image_handle: Option<usize>,
    pub efi_boot_services_not_exited: bool,
}

impl BootInfo {
    pub const fn empty() -> Self {
        Self {
            bootloader_name: StaticStr::empty(),
            first_module: None,
            memory_map_entries: 0,
            acpi_rsdp: None,
            efi_system_table: None,
            efi_image_handle: None,
            efi_boot_services_not_exited: false,
        }
    }
}

pub fn write_static_str(value: StaticStr) {
    if value.is_empty() {
        serial::write_str("<none>");
        return;
    }

    for index in 0..value.len {
        let byte = unsafe { core::ptr::read_volatile(value.ptr.add(index)) };
        if byte == 0 {
            break;
        }
        if (0x20..=0x7e).contains(&byte) {
            serial::write_byte(byte);
        } else {
            serial::write_byte(b'.');
        }
    }
}

pub fn dump(parsed: &BootInfo) {
    serial::write_str("parsed boot info:\n");
    serial::write_str("  bootloader = ");
    write_static_str(parsed.bootloader_name);
    serial::write_str("\n");

    serial::write_str("  memory_map_entries = ");
    serial::write_dec_usize(parsed.memory_map_entries);
    serial::write_str("\n");

    serial::write_str("  acpi_rsdp = ");
    match parsed.acpi_rsdp {
        Some(addr) => serial::write_hex_usize(addr),
        None => serial::write_str("<none>"),
    }
    serial::write_str("\n");

    serial::write_str("  efi_system_table = ");
    match parsed.efi_system_table {
        Some(addr) => serial::write_hex_usize(addr),
        None => serial::write_str("<none>"),
    }
    serial::write_str("\n");

    serial::write_str("  efi_image_handle = ");
    match parsed.efi_image_handle {
        Some(handle) => serial::write_hex_usize(handle),
        None => serial::write_str("<none>"),
    }
    serial::write_str("\n");

    serial::write_str("  efi_boot_services_not_exited = ");
    if parsed.efi_boot_services_not_exited {
        serial::write_str("true");
    } else {
        serial::write_str("false");
    }
    serial::write_str("\n");

    serial::write_str("  first_module = ");
    match parsed.first_module {
        Some(module) => {
            serial::write_str("[");
            serial::write_hex_usize(module.start);
            serial::write_str(", ");
            serial::write_hex_usize(module.end);
            serial::write_str("] name=\"");
            write_static_str(module.name);
            serial::write_str("\"");
        }
        None => serial::write_str("<none>"),
    }
    serial::write_str("\n");
}
