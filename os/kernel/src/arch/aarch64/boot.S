.section .boot_header, "a"

.set MULTIBOOT2_HEADER_MAGIC, 0xe85250d6
.set MULTIBOOT2_HEADER_ARCHITECTURE, 0
.set MULTIBOOT2_TAG_ENTRY_ADDRESS, 3
.set MULTIBOOT2_TAG_EFI_BOOT_SERVICES, 7
.set MULTIBOOT2_TAG_END, 0

.extern start_aarch64

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
    .long start_aarch64

    .balign 8
    .short MULTIBOOT2_TAG_EFI_BOOT_SERVICES
    .short 0
    .long 8

    .balign 8
    .short MULTIBOOT2_TAG_END
    .short 0
    .long 8

multiboot2_header_end:
