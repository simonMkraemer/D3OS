// Stage 1 translation for TTBR0_EL1 (4 KiB granule)
//
// It reads table entries through their physical address, so it only works while the tables are identity mapped

const OUTPUT_ADDRESS_MASK: usize = 0x0000_ffff_ffff_f000;
const DESCRIPTOR_VALID: usize = 1 << 0;
// Levels 0-2: 1 = points to the next table, 0 = block. Level 3: must be 1 (page).
const DESCRIPTOR_TABLE_OR_PAGE: usize = 1 << 1;
const ENTRIES_PER_TABLE: usize = 512;
const PAGE_SHIFT: usize = 12;
const BITS_PER_LEVEL: usize = 9;
const LAST_LEVEL: usize = 3;
const ATTR_INDEX_SHIFT: usize = 2;
const ACCESS_FLAG: usize = 1 << 10;

/// One translation table: 512 entries of 8 bytes, exactly one 4 KiB frame
#[repr(C, align(4096))]
pub struct PageTable {
    entries: [PageTableEntry; ENTRIES_PER_TABLE],
}

impl PageTable {
    /// View the table stored in the frame at `address`.
    /// the caller must ensure that `address` is a valid physical address of a frame
    pub unsafe fn at(address: usize) -> &'static PageTable {
        unsafe { &*(address as *const PageTable) }
    }

    pub fn entry(&self, index: usize) -> PageTableEntry {
        self.entries[index]
    }
}

/// what an entry means depends on its level: bit 1 is "table" on levels 0-2 but "page" on level 3
pub enum EntryKind {
    Invalid,
    Table(usize),
    Block(usize),
    Page(usize),
}

/// 64-bit descriptor - all bit knowledge lives here, so `translate` only asks questions
#[derive(Clone, Copy)]
//Transparent to be exactly usize nothing more or less
#[repr(transparent)]
pub struct PageTableEntry(usize);

impl PageTableEntry {
    pub fn raw(&self) -> usize {
        self.0
    }

    pub fn kind(&self, level: usize) -> EntryKind {
        // Bits 47:12 of a descriptor or TTBR: physical address of the next table, block or page.
        let address = self.0 & OUTPUT_ADDRESS_MASK;

        let is_table_or_page = self.0 & DESCRIPTOR_TABLE_OR_PAGE != 0;
        match (self.0 & DESCRIPTOR_VALID != 0, level == LAST_LEVEL, is_table_or_page) {
            //valid, level 3, and bit 1 = 0
            (false, _, _) => EntryKind::Invalid,
            (true, false, true) => EntryKind::Table(address), //go to next table/level
            (true, false, false) => EntryKind::Block(address),//block, so we re done
            (true, true, true) => EntryKind::Page(address),//page, we re done
            // treat as invalid based on arm doc
            (true, true, false) => EntryKind::Invalid,// reserved, so treat as invalid
        }
    }

    /// index into MAIR_EL1 (bits 4:2): which memory type (Device, Normal, ...) applies
    pub fn attr_index(&self) -> usize {
        (self.0 >> ATTR_INDEX_SHIFT) & 0b111
    }

    /// Access flag (bit 10): must be 1, otherwise the first access raises an Access Flag Fault
    pub fn access_flag(&self) -> bool {
        self.0 & ACCESS_FLAG != 0
    }
}

/// Read TCR_EL1 (Translation Control Register) to see how the firmware set up translation
/// T0SZ (bits 5:0), EPD0 (7), TG0 (15:14), T1SZ (21:16), EPD1 (23), IPS (34:32)
/// https://support.arm.com/documentation/ddi0601/2025-03/AArch64-Registers/TCR-EL1--Translation-Control-Register--EL1-
pub fn read_tcr() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, TCR_EL1", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}


/// physical address of the top-level table used for the lower address range
pub fn read_ttbr0() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, TTBR0_EL1", out(reg) value, options(nomem, nostack, preserves_flags)) };
    // Bits 63:48 hold the ASID, bit 0 CnP ... we only what the table
    value & OUTPUT_ADDRESS_MASK
}

/// level the walk starts at: every level resolves 9 address bits above the 12-bit page offset
/// T0SZ 20 (44 bit) -> level 0, T0SZ 32 (32 bit) -> level 1.
fn start_level(t0sz: usize) -> usize {
    let levels = (64 - t0sz - PAGE_SHIFT).div_ceil(BITS_PER_LEVEL);
    LAST_LEVEL + 1 - levels
}

/// Address bits below `level` log2 of the size one entry covers (L1 = 30 -> 1 GiB, L2 = 21 -> 2 MiB, L3 = 12 -> 4 KiB)
fn level_shift(level: usize) -> usize {
    PAGE_SHIFT + BITS_PER_LEVEL * (LAST_LEVEL - level)
}

/// Index of the entry for `virtual_address` in a table of `level`
fn table_index(virtual_address: usize, level: usize) -> usize {
    (virtual_address >> level_shift(level)) % ENTRIES_PER_TABLE
}

/// Part of `virtual_address` below `level`: the offset into a block or page of that level
fn offset_in_level(virtual_address: usize, level: usize) -> usize {
    virtual_address & ((1 << level_shift(level)) - 1)
}

/// root table and start level of the current TTBR0 translation, or None if
/// `virtual_address` lies outside the range TCR_EL1.T0SZ allows
fn root_table(virtual_address: usize) -> Option<(&'static PageTable, usize)> {
    let t0sz = read_tcr() & 0b111111;
    if virtual_address >> (64 - t0sz) != 0 {
        return None;
    }
    Some((unsafe { PageTable::at(read_ttbr0()) }, start_level(t0sz)))
}

/// Translate a virtual address to a physical address with the current tables,
/// or None where the MMU would raise a translation fault. Same shape as x86 `translate`
/// in memory/pages.rs.
pub fn translate(virtual_address: usize) -> Option<usize> {
    let (table, level) = root_table(virtual_address)?;
    translate_in_table(table, virtual_address, level)
}

fn translate_in_table(table: &PageTable, virtual_address: usize, level: usize) -> Option<usize> {
    let entry = table.entry(table_index(virtual_address, level));
    match entry.kind(level) {
        EntryKind::Invalid => None,
        EntryKind::Table(next) => translate_in_table(unsafe { PageTable::at(next) }, virtual_address, level + 1),
        EntryKind::Block(address) | EntryKind::Page(address) => Some(address + offset_in_level(virtual_address, level)),
    }
}



//TODO: demo fn to remove later maybe

/// Demo to check translate
///
/// MMU translate (AT S1E1R = stage 1, EL1, read)

pub fn translate_with_mmu(virtual_address: usize) -> Option<usize> {
    let par: usize;
    unsafe {
        core::arch::asm!(
            "at s1e1r, {va}",
            "isb",
            "mrs {par}, PAR_EL1",
            va = in(reg) virtual_address,
            par = out(reg) par,
            options(nostack, preserves_flags)
        )
    };

    // PAR_EL1.F (bit 0) = 1: the translation failed.
    if par & 0b1 != 0 {
        return None;
    }
    Some((par & OUTPUT_ADDRESS_MASK) | (virtual_address & ((1 << PAGE_SHIFT) - 1)))
}

/// Debug print walk to see if it works :D
pub fn print_walk(virtual_address: usize) {
    let Some((mut table, start_level)) = root_table(virtual_address) else {
        println!("  outside the TTBR0 range -> translation fault level 0");
        return;
    };

    for level in start_level..=LAST_LEVEL {
        let index = table_index(virtual_address, level);
        let entry = table.entry(index);
        println!("  L{} table {:#x} [{:3}] = {:#018x}", level, table as *const PageTable as usize, index, entry.raw());

        let (kind, address) = match entry.kind(level) {
            EntryKind::Invalid => {
                println!("  -> invalid entry -> translation fault level {}", level);
                return;
            }
            EntryKind::Table(next) => {
                table = unsafe { PageTable::at(next) };
                continue;
            }
            EntryKind::Block(address) => ("block", address),
            EntryKind::Page(address) => ("page", address),
        };
        println!(
            "  -> {} KiB {} at {:#x} + offset {:#x} (AttrIndx {}, AF {})",
            (1usize << level_shift(level)) / 1024,
            kind,
            address,
            offset_in_level(virtual_address, level),
            entry.attr_index(),
            entry.access_flag()
        );
        return;
    }
}
