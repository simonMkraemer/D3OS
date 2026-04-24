/*
 * Future AArch64-specific linker script.
 *
 * This is not wired into the build yet.
 * It exists as a staging point for later AArch64 startup work.
 */

ENTRY(start_aarch64)

PHDRS {
    text PT_LOAD;
    data PT_LOAD;
    bss PT_LOAD;
}

SECTIONS {
    . = 0x40000000;

    ___KERNEL_DATA_START__ = .;

    .boot :
    {
        KEEP(*(.boot_header*))
        KEEP(*(.multiboot2_header))
    } :text

    .text ALIGN (4K) :
    {
        *(.text*)
    } :text

   .bss ALIGN (4K) :
    {
      ___BSS_START__ = .;
      *(.bss*)
      ___BSS_END__ = .;
    } :bss

    .data ALIGN (4K) :
    {
        *(.data*)
    } :data
    ___KERNEL_DATA_END__ = .;
}
