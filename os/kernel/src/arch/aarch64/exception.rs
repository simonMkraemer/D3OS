// AArch64 exception state, the D3OS vector table, and the first exception handler.
//
// The vector table itself lives in boot.S (exception_vector_table). Every entry
// saves the interrupted registers into a TrapFrame on the current stack and calls
// handle_exception with the entry index (0..16).

unsafe extern "C" {
    static exception_vector_table: u8;
}

// Exception Classes (ESR_EL1 bits [31:26]) for handler
const EC_UNKNOWN: usize = 0x00;
const EC_SVC: usize = 0x15;
const EC_INSTRUCTION_ABORT_LOWER_EL: usize = 0x20;
const EC_INSTRUCTION_ABORT_SAME_EL: usize = 0x21;
const EC_DATA_ABORT_LOWER_EL: usize = 0x24;
const EC_DATA_ABORT_SAME_EL: usize = 0x25;
const EC_BRK: usize = 0x3c;
// ELR_EL1 points at the BRK instruction itself, so returning must skip it.
const BRK_INSTRUCTION_SIZE: usize = 4;

const EXCEPTION_TYPES: [&str; 4] = ["Synchronous", "IRQ", "FIQ", "SError"];
const EXCEPTION_SOURCES: [&str; 4] = ["current EL with SP_EL0", "current EL with SP_ELx", "lower EL in AArch64", "lower EL in AArch32"];

/// Registers saved by exception_common in boot.S. The layout must match the
/// TRAP_FRAME_* offsets there.
#[repr(C)]
pub struct TrapFrame {
    pub x: [usize; 31],
    pub elr: usize,
    pub spsr: usize,
    _padding: usize,
}

/// Current Exception Level (CurrentEL bits [3:2]).
// https://support.arm.com/documentation/ddi0601/2025-03/AArch64-Registers/CurrentEL--Current-Exception-Level
pub fn read_current_el() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, CurrentEL", out(reg) value, options(nomem, nostack, preserves_flags)) };
    (value >> 2) & 0b11
}

/// Read SCTLR_EL1 (System Control Register) to check MMU and caches
/// https://support.arm.com/documentation/ddi0601/2025-03/AArch64-Registers/SCTLR-EL1--System-Control-Register--EL1-
/// We caree about M (bit 0) for MMU, C (bit 2) for data cache, and I (bit 12) for instruction cache
pub fn read_sctlr() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, SCTLR_EL1", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Interrupt masks D, A, I, F (DAIF bits [9:6]) as a 4-bit value: bit 3 = D ... bit 0 = F.
// https://support.arm.com/documentation/ddi0601/2025-03/AArch64-Registers/DAIF--Interrupt-Mask-Bits
pub fn read_daif() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, DAIF", out(reg) value, options(nomem, nostack, preserves_flags)) };
    (value >> 6) & 0b1111
}

/// Stack pointer selection: 0 = SP_EL0, 1 = SP_ELx.
pub fn read_spsel() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, SPSel", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value & 0b1
}

fn read_esr() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, esr_el1", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

fn read_far() -> usize {
    let value: usize;
    unsafe { core::arch::asm!("mrs {}, far_el1", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// WnR (ISS bit 6) of a Data Abort with true = the failed access was a write and vice versa
fn data_abort_is_write(esr: usize) -> bool {
    (esr >> 6) & 0b1 == 1
}

/// DFSC (ISS bits [5:0]) of a Data Abort why the access failed e.g. 0b0001LL = translation
fn data_abort_fault_status(esr: usize) -> usize {
    esr & 0b111111
}

/// Set up exception handling: mask all interrupts, then install the D3OS vector table.
/// Must only be called after ExitBootServices, because UEFI still relies on its own vectors.
pub fn init() {
    //TODO: austuschen wenn ich GIC hab... irgendwann mal..
    mask_all();
    println!("DAIF after masking: {:#06b}", read_daif());
    println!("CurrentEL: EL{}", read_current_el());
    println!("SPSel: {}", read_spsel());

    install_vector_table();
}

/// Mask Debug, SError, IRQ and FIQ. Synchronous exceptions (BRK, SVC, aborts) cannot be masked.
fn mask_all() {
    unsafe { core::arch::asm!("msr daifset, #0xf", options(nostack, preserves_flags)) };
}

/// Point VBAR_EL1 for D3OS vector table. Must only be called after ExitBootServices,
/// because UEFI still relies on its own vectors.
///
/// I use ISB here to ensure that the new vector table is used immediately,
/// because the CPU may have already prefetched instructions from the old table.
fn install_vector_table() {
    let table = core::ptr::addr_of!(exception_vector_table) as usize;
    unsafe { core::arch::asm!("msr vbar_el1, {}", "isb", in(reg) table, options(nostack, preserves_flags)) };
    println!("VBAR_EL1: {:#x}", table);
}

/// Mapping for "Syndrome"
/// https://support.arm.com/documentation/ddi0601/2025-03/AArch64-Registers/ESR-EL1--Exception-Syndrome-Register--EL1-?lang=en
///
/// Not a complete list: the architecture defines about 40 more (see ESR_EL1 link above ;) ).
fn exception_class_name(exception_class: usize) -> &'static str {
    match exception_class {
        EC_UNKNOWN => "Unknown reason",
        EC_SVC => "SVC",
        EC_INSTRUCTION_ABORT_LOWER_EL => "Instruction Abort (lower EL)",
        EC_INSTRUCTION_ABORT_SAME_EL => "Instruction Abort (same EL)",
        EC_DATA_ABORT_LOWER_EL => "Data Abort (lower EL)",
        EC_DATA_ABORT_SAME_EL => "Data Abort (same EL)",
        EC_BRK => "BRK",
        // Only returns a name because of SRP
        _ => "other",
    }
}

/// Exception handler for all exceptions
#[unsafe(no_mangle)]
extern "C" fn handle_exception(frame: &mut TrapFrame, entry: usize) {
    let esr = read_esr();
    let exception_class = (esr >> 26) & 0x3f;

    println!("exception: {} from {}", EXCEPTION_TYPES[entry % 4], EXCEPTION_SOURCES[entry / 4]);
    println!(
        "  ESR_EL1:  {:#018x} (EC {:#x} : {})",
        esr,
        exception_class,
        exception_class_name(exception_class)
    );
    println!("  ELR_EL1:  {:#018x}", frame.elr);
    println!("  SPSR_EL1: {:#018x}", frame.spsr);
    println!("  FAR_EL1:  {:#018x}", read_far());

    // ESR_EL1 is only written for synchronous exceptions (and SError); for IRQ/FIQ
    // it still holds an old value, so the EC must not be looked at.
    let is_synchronous = entry % 4 == 0;
    if !is_synchronous {
        panic!("unhandled {} exception", EXCEPTION_TYPES[entry % 4]);
    }

    match exception_class {
        EC_BRK => {
            frame.elr += BRK_INSTRUCTION_SIZE;
        }
        EC_DATA_ABORT_LOWER_EL | EC_DATA_ABORT_SAME_EL => {
            println!("Data abort: WnR = {}, DFSC = {:#08b}", data_abort_is_write(esr), data_abort_fault_status(esr));
            panic!("data abort at {:#x}", read_far());
        }
        _ => panic!("unhandled exception class {:#x} ({})", exception_class, exception_class_name(exception_class)),
    }
}


//=======================================================
// Exception trigger helpers for testing
pub fn trigger_breakpoint() {
    // clobber_abi: the handler is Rust code and may change caller-saved registers.
    unsafe { core::arch::asm!("brk #0", clobber_abi("C")) };
}

pub fn trigger_data_abort() {
    let addr = 0xFFFF_0000_0000_0000 as *const u64;
    unsafe {
        core::ptr::read_volatile(addr);
    }
}
