#![no_main]

// A datagram from the other end of `vif ssh` over UDP, or output for the screens it keeps.
libfuzzer_sys::fuzz_target!(|data: &[u8]| vif::fuzz::remote(data));
