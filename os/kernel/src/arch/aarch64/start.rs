#![no_std]
#![no_main]

use core::panic::PanicInfo;

const PL011_UART0_BASE: usize = 0x0900_0000;
const UARTDR: *mut u32 = PL011_UART0_BASE as *mut u32;
const UARTFR: *mut u32 = (PL011_UART0_BASE + 0x18) as *mut u32;
const UARTFR_TXFF: u32 = 1 << 5;

fn serial_write_byte(byte: u8) {
    while unsafe { core::ptr::read_volatile(UARTFR) } & UARTFR_TXFF != 0 {
        core::hint::spin_loop();
    }
    unsafe { core::ptr::write_volatile(UARTDR, u32::from(byte)) };
}

fn serial_write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            serial_write_byte(b'\r');
        }
        serial_write_byte(byte);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn start_aarch64(_x0: usize, _x1: usize, _x2: usize, _x3: usize) -> ! {
    serial_write_str("HELLO WORLD start\n");
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    serial_write_str("HELLO WORLD panic\n");
    loop {
        core::hint::spin_loop();
    }
}
