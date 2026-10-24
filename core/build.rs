fn main() {
    cxx_build::bridge("src/lib.rs")
        .std("c++23")
        .compile("hypr2d-bridge");
    println!("cargo:rerun-if-changed=src/lib.rs");
}
