/*
 * Active AArch64 linker script.
 *
 * os/kernel/Makefile.toml invokes rust-lld with this script via -T when
 * building the AArch64 kernel image.
 */

ENTRY(aarch64_entry)

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
