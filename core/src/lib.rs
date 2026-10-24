#[cxx::bridge(namespace = "hypr2d")]
mod ffi {
    extern "Rust" {
        fn spike_round_trip(value: u32) -> u32;
    }
}

// Every exported operation must contain its panic before returning through cxx.
fn spike_round_trip(value: u32) -> u32 {
    contain_panic(|| value.checked_add(1).unwrap_or(0))
}

fn contain_panic(operation: impl FnOnce() -> u32 + std::panic::UnwindSafe) -> u32 {
    match std::panic::catch_unwind(operation) {
        Ok(value) => value,
        Err(payload) => {
            // Even an unusual panic payload with a panicking Drop stays in Rust.
            std::mem::forget(payload);
            let _ = std::io::Write::write_all(
                &mut std::io::stderr(),
                b"hypr2d: contained Rust panic; operation ignored\n",
            );
            0
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn panic_is_contained() {
        assert_eq!(super::contain_panic(|| panic!("test panic")), 0);
    }

    #[test]
    fn round_trip_and_overflow_are_safe() {
        assert_eq!(super::spike_round_trip(41), 42);
        assert_eq!(super::spike_round_trip(u32::MAX), 0);
    }
}
