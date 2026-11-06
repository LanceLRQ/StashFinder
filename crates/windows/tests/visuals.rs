use stashfinder_windows::visuals;

#[test]
fn windows_type_icons_work_without_a_source_file() {
    for (name, folder) in [
        ("sample.txt", false),
        ("unknown.sf-test-unknown", false),
        ("folder", true),
    ] {
        let image = visuals::type_icon(name, folder, 32).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(image));
        let mut reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().width, 32);
        assert_eq!(reader.info().height, 32);
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut pixels).unwrap();
        assert!(
            pixels[..frame.buffer_size()]
                .chunks_exact(4)
                .any(|p| p[3] > 0)
        );
    }
    assert!(visuals::type_icon("sample.txt", false, 0).is_err());
}

#[test]
fn windows_thumbnail_cache_can_generate_a_temporary_image_and_reuse_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("thumbnail.png");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(file, 64, 48);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[40, 140, 180, 255].repeat(64 * 48))
        .unwrap();
    writer.finish().unwrap();
    let _ = visuals::thumbnail(&path, 256, true).unwrap();
    let image = visuals::thumbnail(&path, 256, false).unwrap().unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(image));
    let mut reader = decoder.read_info().unwrap();
    assert!(reader.info().width > 0 && reader.info().height > 0);
    let center =
        ((reader.info().height / 2 * reader.info().width + reader.info().width / 2) * 4) as usize;
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    assert_eq!(&pixels[center..center + 4], &[40, 140, 180, 255]);
    assert!(visuals::thumbnail(&path, 256, true).unwrap().is_some());
    let icon = visuals::file_icon(&path, 32).unwrap();
    assert!(
        png::Decoder::new(std::io::Cursor::new(icon))
            .read_info()
            .is_ok()
    );
    let unsupported = temp.path().join("not-an-image.sf-no-thumbnail-8d73c060");
    std::fs::write(&unsupported, b"plain text").unwrap();
    assert!(
        visuals::thumbnail(&unsupported, 256, false)
            .unwrap()
            .is_none()
    );
}
