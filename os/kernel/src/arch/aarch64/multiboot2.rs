pub const MAGIC: usize = 0x36d7_6289;

#[derive(Copy, Clone)]
pub struct StaticStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl StaticStr {
    const fn empty() -> Self {
        Self {
            ptr: core::ptr::null(),
            len: 0,
        }
    }

    fn equals(self, expected: &[u8]) -> bool {
        if self.len != expected.len() || self.ptr.is_null() {
            return false;
        }

        expected
            .iter()
            .enumerate()
            .all(|(index, byte)| unsafe { core::ptr::read_volatile(self.ptr.add(index)) == *byte })
    }
}

#[derive(Copy, Clone)]
pub struct BootModule {
    pub start: usize,
    pub end: usize,
    pub name: StaticStr,
}

#[derive(Copy, Clone)]
pub struct FramebufferInfo {
    pub address: usize,
    pub pitch: usize,
    pub width: usize,
    pub height: usize,
    pub bpp: usize,
}

pub struct HandoffInfo {
    pub bootinfo_len: usize,
    pub initrd: Option<BootModule>,
    pub framebuffer: Option<FramebufferInfo>,
    pub acpi_rsdp: Option<usize>,
    pub efi_system_table: Option<usize>,
    pub efi_image_handle: Option<usize>,
    pub efi_boot_services_not_exited: bool,
}

impl HandoffInfo {
    const fn empty(bootinfo_len: usize) -> Self {
        Self {
            bootinfo_len,
            initrd: None,
            framebuffer: None,
            acpi_rsdp: None,
            efi_system_table: None,
            efi_image_handle: None,
            efi_boot_services_not_exited: false,
        }
    }

    pub fn has_required_uefi_handoff(&self) -> bool {
        self.bootinfo_len != 0
            && self.initrd.is_some()
            && self.efi_system_table.is_some()
            && self.efi_image_handle.is_some()
            && self.efi_boot_services_not_exited
    }
}

fn read_u32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn read_u64(addr: usize) -> u64 {
    unsafe { core::ptr::read_volatile(addr as *const u64) }
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

    StaticStr { ptr: bytes_ptr, len }
}

pub fn parse_handoff(bootinfo: usize) -> Option<HandoffInfo> {
    if bootinfo == 0 || bootinfo & 7 != 0 {
        return None;
    }

    let total_size = read_u32(bootinfo) as usize;
    let reserved = read_u32(bootinfo + 4);
    if total_size < 16 || reserved != 0 {
        return None;
    }

    let mut parsed = HandoffInfo::empty(total_size);
    let mut offset = 8usize;

    while offset + 8 <= total_size {
        let tag_addr = bootinfo + offset;
        let tag_type = read_u32(tag_addr);
        let tag_size = read_u32(tag_addr + 4) as usize;

        let tag_end = offset.checked_add(tag_size)?;
        if tag_size < 8 || tag_end > total_size {
            return None;
        }

        match tag_type {
            0 if tag_size == 8 => {
                return Some(parsed);
            }
            0 => return None,
            3 => {
                if parsed.initrd.is_none() && tag_size >= 16 {
                    let module = BootModule {
                        start: read_u32(tag_addr + 8) as usize,
                        end: read_u32(tag_addr + 12) as usize,
                        name: parse_static_str(tag_addr + 8, tag_size - 8),
                    };
                    if module.name.equals(b"initrd") {
                        parsed.initrd = Some(module);
                    }
                }
            }
            8 if tag_size >= 30 => {
                parsed.framebuffer = Some(FramebufferInfo {
                    address: read_u64(tag_addr + 8) as usize,
                    pitch: read_u32(tag_addr + 16) as usize,
                    width: read_u32(tag_addr + 20) as usize,
                    height: read_u32(tag_addr + 24) as usize,
                    bpp: unsafe { core::ptr::read_volatile((tag_addr + 28) as *const u8) as usize },
                });
            }
            12 => {
                if tag_size >= 16 {
                    parsed.efi_system_table = Some(read_u64(tag_addr + 8) as usize);
                }
            }
            20 => {
                if tag_size >= 16 {
                    parsed.efi_image_handle = Some(read_u64(tag_addr + 8) as usize);
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

        offset = tag_end.checked_add(7)? & !7;
    }

    None
}
