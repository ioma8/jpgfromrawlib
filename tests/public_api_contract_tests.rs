#[test]
fn crate_root_exports_cover_the_supported_library_surface() {
    use jpgfromraw::{FindJpegType, SUPPORTED_EXTENSIONS, process_file, process_file_bytes};

    let _ = FindJpegType::Largest;
    let _ = FindJpegType::Smallest;
    let _ = process_file;
    let _ = process_file_bytes;
    assert!(SUPPORTED_EXTENSIONS.contains(&"arw"));
    assert!(SUPPORTED_EXTENSIONS.contains(&"gpr"));
}

#[test]
fn parser_internal_helpers_are_not_public_api() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/parser_internal_api_is_hidden.rs");
}
