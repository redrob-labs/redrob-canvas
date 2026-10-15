#![no_main]

use libfuzzer_sys::fuzz_target;

// The whole native reader (archive, object stream, model) and the preview, on arbitrary bytes.
fuzz_target!(|bytes: &[u8]| {
    let _ = redrob_vector_affinity::container::header(bytes);
    let _ = redrob_vector_affinity::preview(bytes);
    let _ = redrob_vector_affinity::read(bytes, redrob_vector_affinity::Limits { max_entry: 16 << 20, max_total: 64 << 20 });
});
