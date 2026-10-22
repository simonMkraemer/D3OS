// rustc-based build used. boot.S is included via global_asm! in start.rs.
const LINKER_SCRIPT: &str = "../../../link/aarch64.ld";

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");

    println!("cargo:rustc-link-arg-bins=-T{manifest_dir}/{LINKER_SCRIPT}");
    println!("cargo:rustc-link-arg-bins=-zmax-page-size=0x1000");

    println!("cargo:rerun-if-changed={LINKER_SCRIPT}");
    println!("cargo:rerun-if-changed=boot.S");
}
