// PL011 UART output on QEMU virt and the print!/println! macros built on it.
//
// Serial implements core::fmt::Write, so the macros support Rust formatting
// ({}, {:#x}, {:#06b}, ...) without needing a heap. Later a `log` backend can
// write through the same Serial type.

use core::fmt;

const PL011_UART0_BASE: usize = 0x0900_0000;
const UARTDR: *mut u32 = PL011_UART0_BASE as *mut u32;
const UARTFR: *mut u32 = (PL011_UART0_BASE + 0x18) as *mut u32;
const UARTFR_TXFF: u32 = 1 << 5;

pub struct Serial;

impl fmt::Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

/// Used by print!/println!. Not meant to be called directly.
pub fn print_args(args: fmt::Arguments<'_>) {
    use core::fmt::Write;
    // Serial::write_str never fails, so write_fmt can only fail if a Display
    // implementation reports an error. There is nowhere else to report it.
    let _ = Serial.write_fmt(args);
}

macro_rules! print {
    ($($arg:tt)*) => {
        $crate::serial::print_args(format_args!($($arg)*))
    };
}

macro_rules! println {
    () => {
        print!("\n")
    };
    ($($arg:tt)*) => {
        print!("{}\n", format_args!($($arg)*))
    };
}

fn write_byte(byte: u8) {
    while unsafe { core::ptr::read_volatile(UARTFR) } & UARTFR_TXFF != 0 {
        core::hint::spin_loop();
    }
    unsafe { core::ptr::write_volatile(UARTDR, u32::from(byte)) };
}
