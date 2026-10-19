// AArch64 exception state, the D3OS vector table, and the first exception handler.
//
// The vector table itself lives in boot.S (exception_vector_table). Every entry
// saves the interrupted registers into a TrapFrame on the current stack and calls
// handle_exception with the entry index (0..16).

unsafe extern "C" {
    static exception_vector_table: u8;
}

// Exception Class (ESR_EL1 bits [31:26]) of a BRK instruction executed in AArch64 state.
const EC_BRK: usize = 0x3c;
// ELR_EL1 points at the BRK instruction itself, so returning must skip it.
const BRK_INSTRUCTION_SIZE: usize = 4;

const EXCEPTION_TYPES: [&str; 4] = ["Synchronous", "IRQ", "FIQ", "SError"];
const EXCEPTION_SOURCES: [&str; 4] = [
    "current EL with SP_EL0",
    "current EL with SP_ELx",
    "lower EL in AArch64",
    "lower EL in AArch32",
];

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

/// Mask Debug, SError, IRQ and FIQ. Synchronous exceptions (BRK, SVC, aborts) cannot be masked.
pub fn mask_all() {
    unsafe { core::arch::asm!("msr daifset, #0xf", options(nostack, preserves_flags)) };
}

/// Point VBAR_EL1 for D3OS vector table. Must only be called after ExitBootServices,
/// because UEFI still relies on its own vectors.
///
/// I use ISB here to ensure that the new vector table is used immediately,
/// because the CPU may have already prefetched instructions from the old table.
pub fn install_vector_table() {
    let table = core::ptr::addr_of!(exception_vector_table) as usize;
    unsafe { core::arch::asm!("msr vbar_el1, {}", "isb", in(reg) table, options(nostack, preserves_flags)) };
    println!("VBAR_EL1: {:#x}", table);
}

/// Raise a synchronous BRK exception to test the vector table and the return path.
pub fn trigger_breakpoint() {
    // clobber_abi: the handler is Rust code and may change caller-saved registers.
    unsafe { core::arch::asm!("brk #0", clobber_abi("C")) };
}


/// Currently for debugging only :D
/// Adresses are checkt in manual i added the links
#[unsafe(no_mangle)]
extern "C" fn handle_exception(frame: &mut TrapFrame, entry: usize) {
    let esr = read_esr();
    let exception_class = (esr >> 26) & 0x3f;

    println!("exception: {} from {}", EXCEPTION_TYPES[entry % 4], EXCEPTION_SOURCES[entry / 4]);
    println!("  ESR_EL1:  {:#018x} (EC {:#x})", esr, exception_class);
    println!("  ELR_EL1:  {:#018x}", frame.elr);
    println!("  SPSR_EL1: {:#018x}", frame.spsr);
    println!("  FAR_EL1:  {:#018x}", read_far());


    //TODO: remove this when we have a proper exception handler
    let is_synchronous = entry % 4 == 0;
    if is_synchronous && exception_class == EC_BRK {
        frame.elr += BRK_INSTRUCTION_SIZE;
        return;
    }

    println!("unhandled exception, halting");
    crate::halt();
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
