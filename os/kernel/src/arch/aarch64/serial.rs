const PL011_UART0_BASE: usize = 0x0900_0000;
const UARTDR: *mut u32 = PL011_UART0_BASE as *mut u32;
const UARTFR: *mut u32 = (PL011_UART0_BASE + 0x18) as *mut u32;
const UARTFR_TXFF: u32 = 1 << 5;

pub fn write_byte(byte: u8) {
    while unsafe { core::ptr::read_volatile(UARTFR) } & UARTFR_TXFF != 0 {
        core::hint::spin_loop();
    }
    unsafe { core::ptr::write_volatile(UARTDR, u32::from(byte)) };
}

pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

pub fn write_hex_usize(value: usize) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let nybbles = core::mem::size_of::<usize>() * 2;

    write_str("0x");
    for shift in (0..nybbles).rev() {
        let digit = (value >> (shift * 4)) & 0xf;
        write_byte(HEX[digit] as u8);
    }
}

pub fn write_labelled_hex(label: &str, value: usize) {
    write_str(label);
    write_hex_usize(value);
    write_str("\n");
}

pub fn write_dec_usize(mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0;

    if value == 0 {
        write_byte(b'0');
        return;
    }

    while value != 0 {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    while len != 0 {
        len -= 1;
        write_byte(digits[len]);
    }
}

pub fn write_bytes(bytes: &[u8]) {
    for &byte in bytes {
        write_byte(byte);
    }
}

pub fn write_cstr(ptr: *const u8, len: usize) {
    for index in 0..len {
        let byte = unsafe { core::ptr::read_volatile(ptr.add(index)) };
        if byte == 0 {
            break;
        }
        if (0x20..=0x7e).contains(&byte) {
            write_byte(byte);
        } else {
            write_byte(b'.');
        }
    }
}
