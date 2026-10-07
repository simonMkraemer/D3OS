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
