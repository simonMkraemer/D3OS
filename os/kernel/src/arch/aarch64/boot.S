// AArch64 bootstrap code and the Multiboot2-compatible header read by Tow-Boot.
//
// Tow-Boot branches to aarch64_entry with x0 = boot magic and x1 = boot-info
// pointer. This stub deliberately preserves those registers for start_aarch64.

// Multiboot2-compatible header constants.
.set MULTIBOOT2_HEADER_MAGIC, 0xe85250d6
.set MULTIBOOT2_HEADER_ARCHITECTURE, 0
.set MULTIBOOT2_TAG_ENTRY_ADDRESS, 3
.set MULTIBOOT2_TAG_FRAMEBUFFER, 5
.set MULTIBOOT2_TAG_EFI_BOOT_SERVICES, 7
.set MULTIBOOT2_TAG_END, 0
.set FRAMEBUFFER_WIDTH, 800
.set FRAMEBUFFER_HEIGHT, 600
.set FRAMEBUFFER_BPP, 32

// First Rust function. The AArch64 calling convention passes its first arguments
// in x0, x1, x2, and x3, so the boot handoff values already have the right form.
.extern start_aarch64

// Executable entry point selected by the AArch64 linker script.
.section .text.boot, "ax"
.global aarch64_entry
.type aarch64_entry, %function
aarch64_entry:
    // Establish a private 16 KiB stack before entering Rust. At entry sp is
    // unspecified by the D3OS boot ABI; the stack grows down from boot_stack_end.
    adrp x9, boot_stack_end
    add x9, x9, :lo12:boot_stack_end
    mov sp, x9

    // Do not change x0 or x1: they carry the boot magic and boot-information
    // pointer supplied by Tow-Boot. `b` is used because Rust never returns.
    // Exception vectors, interrupt masks, and MMU state are not configured here;
    // establish them before relying on them when adding exception/MMU support.
    b start_aarch64

// Exception vector table installed in VBAR_EL1 by exception.rs after
// ExitBootServices. 4 sources x 4 types, each entry 0x80 bytes, base aligned
// to 2 KiB. An entry has only 32 instructions, so it saves x0/x1, stores its
// index in x0, and continues in exception_common.
.set TRAP_FRAME_SIZE, 272   // x0-x30, ELR, SPSR, padding to 16 bytes
.set TRAP_FRAME_X30, 240
.set TRAP_FRAME_ELR, 248

.macro VECTOR_ENTRY index
    .balign 0x80
    sub sp, sp, #TRAP_FRAME_SIZE
    stp x0, x1, [sp, #0]
    mov x0, #\index
    b exception_common
.endm

.section .text.exception_vectors, "ax"
.balign 0x800
.global exception_vector_table
exception_vector_table:
    // current EL with SP_EL0
    VECTOR_ENTRY 0
    VECTOR_ENTRY 1
    VECTOR_ENTRY 2
    VECTOR_ENTRY 3
    // current EL with SP_ELx
    VECTOR_ENTRY 4
    VECTOR_ENTRY 5
    VECTOR_ENTRY 6
    VECTOR_ENTRY 7
    // lower EL in AArch64
    VECTOR_ENTRY 8
    VECTOR_ENTRY 9
    VECTOR_ENTRY 10
    VECTOR_ENTRY 11
    // lower EL in AArch32
    VECTOR_ENTRY 12
    VECTOR_ENTRY 13
    VECTOR_ENTRY 14
    VECTOR_ENTRY 15

// From: https://jcomes.org/aarch64-os-interrupt-handling-i    
// Save the remaining registers into the TrapFrame, call
// handle_exception(frame, index), restore everything and return with eret.
// The layout must match TrapFrame in exception.rs.
// TODO: save FP/SIMD registers before handling asynchronous exceptions (IRQ).
exception_common:
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #TRAP_FRAME_X30]
    mrs x2, elr_el1
    mrs x3, spsr_el1
    stp x2, x3, [sp, #TRAP_FRAME_ELR]

    mov x1, x0              // second argument: vector entry index
    mov x0, sp              // first argument: &mut TrapFrame
    bl handle_exception

    // The handler may change ELR (e.g. skip a BRK), so restore from the frame.
    ldp x2, x3, [sp, #TRAP_FRAME_ELR]
    msr elr_el1, x2
    msr spsr_el1, x3
    ldr x30, [sp, #TRAP_FRAME_X30]
    ldp x28, x29, [sp, #224]
    ldp x26, x27, [sp, #208]
    ldp x24, x25, [sp, #192]
    ldp x22, x23, [sp, #176]
    ldp x20, x21, [sp, #160]
    ldp x18, x19, [sp, #144]
    ldp x16, x17, [sp, #128]
    ldp x14, x15, [sp, #112]
    ldp x12, x13, [sp, #96]
    ldp x10, x11, [sp, #80]
    ldp x8, x9, [sp, #64]
    ldp x6, x7, [sp, #48]
    ldp x4, x5, [sp, #32]
    ldp x2, x3, [sp, #16]
    ldp x0, x1, [sp, #0]
    add sp, sp, #TRAP_FRAME_SIZE
    eret

// Bootstrap stack. %nobits reserves space in the loaded image without storing
// 16 KiB of zero bytes in the kernel file.
.section .bss.boot_stack, "aw", %nobits
.balign 16
boot_stack:
    .skip 16384
boot_stack_end:

// This section is data, not instructions executed by the CPU. Tow-Boot scans it
// before transferring control to aarch64_entry. The linker keeps it near the
// beginning of the kernel image.
.section .boot_header, "a"
multiboot2_header:
    // Required fixed header: magic, architecture, total length, and checksum.
    // The checksum makes the four 32-bit values add up to zero modulo 2^32.
    .balign 8
    .long MULTIBOOT2_HEADER_MAGIC
    .long MULTIBOOT2_HEADER_ARCHITECTURE
    .long multiboot2_header_end - multiboot2_header
    .long -(MULTIBOOT2_HEADER_MAGIC + MULTIBOOT2_HEADER_ARCHITECTURE + (multiboot2_header_end - multiboot2_header))

    // Generic entry-address tag: tell Tow-Boot where to branch after loading.
    .balign 8
    .short MULTIBOOT2_TAG_ENTRY_ADDRESS
    .short 0
    .long 12
    .long aarch64_entry

    // Keep UEFI Boot Services active. start_aarch64 performs ExitBootServices.
    .balign 8
    .short MULTIBOOT2_TAG_EFI_BOOT_SERVICES
    .short 0
    .long 8

    // Ask Tow-Boot to select this GOP framebuffer mode before handoff.
    // Required final tag marking the end of the loader header.
    .balign 8
    .short MULTIBOOT2_TAG_FRAMEBUFFER
    .short 0
    .long 20
    .long FRAMEBUFFER_WIDTH
    .long FRAMEBUFFER_HEIGHT
    .long FRAMEBUFFER_BPP

    .balign 8
    .short MULTIBOOT2_TAG_END
    .short 0
    .long 8

multiboot2_header_end:
