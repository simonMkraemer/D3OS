# D3OS AArch64 Boot ABI v0.1

This document defines the boot handoff from Tow-Boot to the D3OS AArch64 kernel. It is intentionally independent of the x86 Multiboot machine-state ABI: it retains a Multiboot2-style tag-based information block but uses the AArch64 procedure-call register convention.

## Scope

ABI v0.1 defines the entry registers, the required boot-information contents, and UEFI Boot Services ownership. It does not define D3OS MMU setup, exception vectors, interrupts, drivers, or user mode.

## Entry Point

Tow-Boot reads the generic entry address from the kernel header and branches to it. D3OS currently exposes `aarch64_entry` in `os/kernel/src/arch/aarch64/boot.S`.

The current generic entry-address field is 32-bit. Therefore the kernel entry address must remain below 4 GiB. D3OS currently links the kernel at `0x40000000`.

## Register Contract

At kernel entry:

| Register | Value |
| --- | --- |
| `x0` | Multiboot-style boot magic `0x36d76289` |
| `x1` | 8-byte-aligned pointer to the boot-information block |
| `x2`-`x30` | Undefined |
| `sp` | Undefined; the kernel establishes its own stack before entering Rust |

SIMD/FP registers, thread-pointer registers, exception-level state, MMU state, caches, exception vectors, and interrupt masks are not part of this ABI. The kernel must not rely on a particular value unless it establishes that state itself.

## Boot-Information Block

`x1` points to an 8-byte-aligned Multiboot2-style information block. It begins with a 32-bit total size, a reserved 32-bit field that must be zero, and a sequence of 8-byte-aligned tags ending in an end tag.

The kernel must validate the total size, every tag size, alignment, and the end tag before using tag contents.

### Required Tags

The following tags are required by D3OS ABI v0.1:

| Tag | Purpose |
| --- | --- |
| End tag | Terminates the boot-information block. |
| Module tag | Provides the initrd range. D3OS identifies the initrd through module argument `initrd`. |
| EFI 64-bit system-table tag | Provides the UEFI system-table pointer while Boot Services are active. |
| EFI 64-bit image-handle tag | Provides Tow-Boot's UEFI image handle for `ExitBootServices`. |
| EFI Boot Services-not-exited tag | States that D3OS must perform the final UEFI transition. |

The Multiboot-style memory-map tag is optional compatibility data. It is not used for frame allocation; D3OS uses the fresh UEFI memory map acquired immediately before `ExitBootServices`.

### Optional Tags

Bootloader name, command line, framebuffer, SMBIOS, and ACPI RSDP tags are optional. D3OS ABI v0.1 supports ACPI information when firmware supplies it. Device Tree Blob support is intentionally not part of v0.1.

## UEFI Ownership

Boot Services remain active when Tow-Boot branches to D3OS. D3OS owns the final `ExitBootServices` call.

D3OS must:

1. obtain a fresh UEFI memory map in preallocated kernel memory;
2. retain its size, descriptor size, version, and map key;
3. avoid UEFI allocations or other map-changing calls between the final `GetMemoryMap` and `ExitBootServices` calls;
4. retry the sequence if `ExitBootServices` rejects the map key;
5. stop on a permanent failure;
6. make no Boot Services call after a successful exit.

The fresh UEFI memory map becomes the authoritative source for physical-memory ownership. The kernel must reserve the kernel image, boot-information block, initrd, framebuffer when present, and any firmware data it keeps using before allocating frames.

## Data Lifetime

The boot-information block, initrd, kernel image, and optional firmware tables remain valid only while their backing ranges are reserved and mapped by D3OS. Before replacing the UEFI address-space setup, D3OS must either retain the required ranges in its own mappings or copy the required data into kernel-owned memory.

## Compatibility

Tow-Boot and D3OS must update this document together when changing the register contract, mandatory tags, UEFI ownership, or data lifetime. Additive optional tags may be introduced without changing the ABI version.
