use super::*;
use crate::{NoFootage, RenderOpts, Renderer};

fn benchmark_project(size: (u32, u32)) -> (Project, ItemId) {
    let mut project = Project::default();
    project.settings.bit_depth = effectcraft_project::BitDepth::Bpc32;
    let comp = Comp::new(1920, 1080, FrameRate::FPS_30, Tick::from_seconds_f64(3.0));
    let comp_id = project.add_item("Buffers", Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    for index in 0..8 {
        let mut layer = solid(&mut project, &comp, [0.1 * index as f32, 0.3, 0.7], size.0, size.1);
        layer.preserve_transparency = index != 7;
        layer.props.prop_mut("transform/position").unwrap().keys =
            vec![Keyframe::new(Tick::ZERO, Value::Vec3([960.0, 540.0, 0.0])), Keyframe::new(Tick::from_seconds_f64(2.0), Value::Vec3([1020.0, 570.0, 0.0]))];
        project.comp_mut(comp_id).unwrap().layers.push(layer);
    }
    (project, comp_id)
}

#[test]
#[ignore = "1080p allocation/timing benchmark; run in release with --nocapture"]
fn compose_buffers_1080p() {
    let workers = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    for size in [(1920, 1080), (480, 270)] {
        let (project, comp_id) = benchmark_project(size);
        let renderer = Renderer::new(&project, &NoFootage, RenderOpts::default());
        workers.install(|| {
            std::hint::black_box(renderer.comp_frame(comp_id, Tick::ZERO));
            let start = std::time::Instant::now();
            let allocations = allocation_counter::measure(|| {
                for frame in 0..60 {
                    std::hint::black_box(renderer.comp_frame(comp_id, FrameRate::FPS_30.tick_of(frame)));
                }
            });
            println!(
                "compose_buffers_1080p layer={}x{} frames=60 ms/frame={:.3} allocations/frame={:.1} bytes/frame={:.0}",
                size.0,
                size.1,
                start.elapsed().as_secs_f64() * 1000.0 / 60.0,
                allocations.count_total as f64 / 60.0,
                allocations.bytes_total as f64 / 60.0,
            );
        });
    }
}
