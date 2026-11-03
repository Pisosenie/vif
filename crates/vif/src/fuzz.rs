//! Entry points for `fuzz/`: the parsers that read what other programs and files send.

use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::graphics::{Filter, Shared};
use crate::marks::Blocks;

/// Program output through the PTY filter, in pieces cut where the first bytes say, the way
/// reads split it.
pub fn pty(data: &[u8]) {
    let Some((&cuts, data)) = data.split_first() else {
        return;
    };
    let (Ok(input), Ok(reply)) = (File::open("/dev/null"), File::create("/dev/null")) else {
        return;
    };
    let shared = Arc::new(Shared::default());
    shared.cell_width.store(10, Ordering::Relaxed);
    shared.cell_height.store(20, Ordering::Relaxed);
    shared.columns.store(80, Ordering::Relaxed);
    let mut filter = Filter::new(input, reply, shared, Arc::new(Blocks::default()));
    let step = usize::from(cuts).max(1);
    let mut sink = [0u8; 4096];
    for piece in data.chunks(step) {
        filter.feed(piece);
        while filter.read(&mut sink).is_ok_and(|n| n > 0) {}
    }
}

/// A GIF, PNG or JPEG drawn in text, as `vif gif` and the greeting do.
pub fn picture(data: &[u8]) {
    let _ = crate::gifplay::decode(data, data.len().is_multiple_of(2), false, (80, 24), 2.0);
}
