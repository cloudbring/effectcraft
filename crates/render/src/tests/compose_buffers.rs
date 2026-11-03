use super::*;
use crate::{NoFootage, RenderOpts};
use effectcraft_color::Label;
use effectcraft_project::{ItemKind, Project, Solid, TrackMatte, build};
use effectcraft_time::FrameRate;

fn setup() -> (Project, effectcraft_project::ItemId) {
    let mut project = Project::default();
    let comp = Comp::new(200, 100, FrameRate::FPS_30, Tick::from_seconds_f64(2.0));
    let comp_id = project.add_item("Comp", Label::Sandstone, None, ItemKind::Comp(comp.clone().into()));
    let item = project.add_item("Solid", Label::Red, None, ItemKind::Solid(Solid { width: 40, height: 30, color: [0.2, 0.4, 0.8], pixel_aspect: 1.0 }));
    for _ in 0..8 {
        let mut layer = build::layer(&mut project, &comp, "Layer", LayerSource::Solid { item }, (40, 30), None);
        layer.preserve_transparency = true;
        project.comp_mut(comp_id).unwrap().layers.push(layer);
    }
    (project, comp_id)
}

fn assert_bits(left: &Image, right: &Image) {
    assert_eq!((left.width, left.height), (right.width, right.height));
    for (index, (left, right)) in left.data.iter().flatten().zip(right.data.iter().flatten()).enumerate() {
        assert_eq!(left.to_bits(), right.to_bits(), "channel {index}");
    }
}

#[test]
fn same_frame_twice_reuses_compositing_buffers() {
    let (project, comp_id) = setup();
    let renderer = Renderer::new(&project, &NoFootage, RenderOpts::default());
    let comp = project.comp(comp_id).unwrap();
    let ctx = renderer.ctx(comp_id, comp, Tick::ZERO);
    let source = Buf { img: Image::filled(40, 30, [0.1, 0.2, 0.4, 0.5]), offset: [0.0; 2], scale: 1.0 };
    let placements: Vec<_> = comp.layers.iter().map(|layer| (layer, renderer.placement(&ctx, layer, &source))).collect();
    let mut canvas = Image::new(200, 100);
    let workers = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    workers.install(|| {
        let draw = |canvas: &mut Image| {
            let mut buffer_allocations = 0;
            canvas.data.fill([0.15, 0.25, 0.35, 0.75]);
            for (layer, placement) in &placements {
                let mut image = None;
                buffer_allocations += allocation_counter::measure(|| image = renderer.scratch.take(canvas.width, canvas.height)).count_total;
                let mut image = image.unwrap();
                let options = WarpOpts { sampling: placement.sampling, seed: placement.seed, ..Default::default() };
                let region = composite_warp_changed(&mut image, &source.img, &placement.matrices[0], &options, None);
                renderer.composite_isolated(&ctx, layer, Isolation { image, matte: None, region }, canvas, 0.75);
            }
            buffer_allocations
        };
        assert!(draw(&mut canvas) > 0);
        let first = canvas.clone();
        let mut buffer_allocations = 0;
        let allocations = allocation_counter::measure(|| buffer_allocations = draw(&mut canvas));
        println!("steady compositing: buffers={buffer_allocations}, including Rayon tasks={allocations:?}");
        assert_eq!(buffer_allocations, 0);
        assert!(allocations.count_total <= 24 && allocations.bytes_total <= 2048, "only small Rayon tasks may allocate: {allocations:?}");
        assert_bits(&first, &canvas);
    });
}

#[test]
fn full_renderer_reuses_source_and_scratch_storage() {
    let (mut project, comp_id) = setup();
    project.comp_mut(comp_id).unwrap().layers.last_mut().unwrap().preserve_transparency = false;
    let renderer = Renderer::new(&project, &NoFootage, RenderOpts::default());
    let workers = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    workers.install(|| {
        let first = renderer.comp_frame(comp_id, Tick::ZERO);
        let mut second = None;
        let allocations = allocation_counter::measure(|| second = Some(renderer.comp_frame(comp_id, Tick::ZERO)));
        let output_bytes = 200 * 100 * size_of::<effectcraft_raster::Px>();
        assert!(allocations.bytes_total < output_bytes as u64 + 32 * 1024, "only the returned image and evaluator metadata may allocate: {allocations:?}");
        assert_bits(&first, &second.unwrap());
    });
}

#[test]
fn pooled_frames_match_fresh_renderers_after_motion_and_resize() {
    use effectcraft_keyframe::{Keyframe, Value};

    let (mut project, comp_id) = setup();
    let comp = project.comp_mut(comp_id).unwrap();
    comp.layers.last_mut().unwrap().preserve_transparency = false;
    let matte = comp.layers[1].id;
    comp.layers[0].track_matte = Some(TrackMatte { layer: matte, kind: MatteKind::LumaInverted });
    for (index, layer) in comp.layers.iter_mut().enumerate() {
        layer.props.prop_mut("transform/position").unwrap().keys =
            vec![Keyframe::new(Tick::ZERO, Value::Vec3([-10.25, 12.75, 0.0])), Keyframe::new(Tick::from_seconds_f64(1.0), Value::Vec3([180.75, 90.25, 0.0]))];
        layer.props.prop_mut("transform/rotation").unwrap().value = Value::Scalar(index as f64 * 11.5);
    }
    let mut renderer = Renderer::new(&project, &NoFootage, RenderOpts::default());
    for (time, scale) in [(0.0, 1.0), (0.5, 0.5), (1.0, 1.0), (0.0, 1.0), (0.5, 0.25)] {
        renderer.opts.scale = scale;
        let time = Tick::from_seconds_f64(time);
        let pooled = renderer.comp_frame(comp_id, time);
        let fresh = Renderer::new(&project, &NoFootage, renderer.opts).comp_frame(comp_id, time);
        assert_bits(&pooled, &fresh);
    }
}

#[test]
fn isolated_roi_matches_full_canvas_for_every_blend_and_matte() {
    let (mut project, comp_id) = setup();
    let matte_id = project.comp(comp_id).unwrap().layers[1].id;
    let source = Buf { img: Image::filled(40, 30, [0.1, 0.2, 0.4, 0.5]), offset: [0.0; 2], scale: 1.0 };
    for mode in BlendMode::ALL {
        for matte in [None, Some(MatteKind::Alpha), Some(MatteKind::AlphaInverted), Some(MatteKind::Luma), Some(MatteKind::LumaInverted)] {
            let layer = &mut project.comp_mut(comp_id).unwrap().layers[0];
            layer.blend_mode = mode;
            layer.track_matte = matte.map(|kind| TrackMatte { layer: matte_id, kind });
            let renderer = Renderer::new(&project, &NoFootage, RenderOpts::default());
            let comp = project.comp(comp_id).unwrap();
            let ctx = renderer.ctx(comp_id, comp, Tick::ZERO);
            let layer = &comp.layers[0];
            let mut full = Image::filled(200, 100, [0.15, 0.25, 0.35, 0.75]);
            let mut bounded = full.clone();
            let mut isolated = Image::new(200, 100);
            renderer.place(&ctx, layer, &source, &mut isolated, BlendMode::Normal, 1.0);
            renderer.composite_iso(&ctx, layer, isolated, &mut full, 0.75);
            renderer.composite_layer(&ctx, layer, &source, &mut bounded, 0.75, None);
            assert_bits(&full, &bounded);
        }
    }
}
