#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| bicraft_fuzz::powerpivot_workbook(data));
