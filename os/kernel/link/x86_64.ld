/*
 * Future x86_64-specific linker script.
 *
 * The current build still uses ../link.ld.
 * This file exists as a staging point for later architecture split work.
 */

ENTRY(start)

PHDRS {
    text PT_LOAD;
    data PT_LOAD;
    bss PT_LOAD;
}

SECTIONS {
    . = 25M;

    ___KERNEL_DATA_START__ = .;

    .boot :
    {
        *(.multiboot2_header)
    } :text

    .text ALIGN (4K) :
    {
        *(.text*)
    } :text

   .bss ALIGN (4K) :
    {
      ___BSS_START__ = .;
      *(".bss*")
      ___BSS_END__ = .;
    } :bss

    .data ALIGN (4K) :
    {
        *(.data*)
    } :data
    ___KERNEL_DATA_END__ = .;
}
