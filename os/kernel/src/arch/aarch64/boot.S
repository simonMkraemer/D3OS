.set MULTIBOOT2_HEADER_MAGIC, 0xe85250d6
.set MULTIBOOT2_HEADER_ARCHITECTURE, 0
.set MULTIBOOT2_TAG_ENTRY_ADDRESS, 3
.set MULTIBOOT2_TAG_EFI_BOOT_SERVICES, 7
.set MULTIBOOT2_TAG_END, 0

.extern start_aarch64

.section .text.boot, "ax"
.global aarch64_entry
.type aarch64_entry, %function
aarch64_entry:
    adrp x9, boot_stack_end
    add x9, x9, :lo12:boot_stack_end
    mov sp, x9
    b start_aarch64

.section .bss.boot_stack, "aw", %nobits
.balign 16
boot_stack:
    .skip 16384
boot_stack_end:

.section .boot_header, "a"
multiboot2_header:
    .balign 8
    .long MULTIBOOT2_HEADER_MAGIC
    .long MULTIBOOT2_HEADER_ARCHITECTURE
    .long multiboot2_header_end - multiboot2_header
    .long -(MULTIBOOT2_HEADER_MAGIC + MULTIBOOT2_HEADER_ARCHITECTURE + (multiboot2_header_end - multiboot2_header))

    .balign 8
    .short MULTIBOOT2_TAG_ENTRY_ADDRESS
    .short 0
    .long 12
    .long aarch64_entry

    .balign 8
    .short MULTIBOOT2_TAG_EFI_BOOT_SERVICES
    .short 0
    .long 8

    .balign 8
    .short MULTIBOOT2_TAG_END
    .short 0
    .long 8

multiboot2_header_end:
