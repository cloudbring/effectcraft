//! The crate's integration tests, one binary (each module is a former `tests/<name>.rs`).

mod gen_golden;
mod particle_golden;
mod sim_golden;
mod timewarp_golden;

fn snapshot(name: &str, image: &effectcraft_raster::Image) {
    if let Some(directory) = std::env::var_os("BLUR_GOLDEN_OUT") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let bytes: Vec<u8> = image.data.iter().flatten().flat_map(|value| value.to_le_bytes()).collect();
        let filename = format!("{}_{}x{}.f32", name.replace('/', "_"), image.width, image.height);
        std::fs::write(directory.join(filename), bytes).unwrap();
    }
}
