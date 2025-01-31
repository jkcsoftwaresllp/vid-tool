use criterion::{criterion_group, criterion_main, Criterion};
use std::time::{Duration, Instant};
use sysinfo::{CpuExt, System, SystemExt};
use vid_tool::vid::{GameData, VideoProcessor};

pub fn benchmark_video_processing(c: &mut Criterion) {
    let mut group = c.benchmark_group("video_processing");

    // Test data setup
    let input_video = "assets/teen_patti_1.mp4";
    let output_video = "bench_output.mp4";
    let card_assets = vec!["card1.jpg".to_string()];

    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));

    // Single video processing benchmark
    group.bench_function("single_video_processing", |b| {
        b.iter(|| {
            let mut sys = System::new_all();
            let start_time = Instant::now();
            let initial_memory = sys.used_memory();

            let mut processor = VideoProcessor::new(input_video, output_video).unwrap();
            let game_data = GameData {
                card_assets: card_assets.clone(),
            };
            let result = processor.process_video(&game_data);

            let processing_time = start_time.elapsed();
            sys.refresh_all();
            let memory_used = sys.used_memory() - initial_memory;
            let cpu_usage = sys.global_cpu_info().cpu_usage();

            println!("Performance Metrics:");
            println!("Latency: {}ms", processing_time.as_millis());
            println!("Memory Usage: {}MB", memory_used / 1024 / 1024);
            println!("CPU Usage: {}%", cpu_usage);

            result
        })
    });

    // Sequential throughput test (instead of parallel)
    group.bench_function("throughput_test", |b| {
        b.iter(|| {
            let start_time = Instant::now();
            let test_count = 3; // Process 3 videos sequentially
            let mut completed_videos = 0;

            for i in 0..test_count {
                let output = format!("bench_output_{}.mp4", i);
                let mut processor = VideoProcessor::new(input_video, &output).unwrap();
                let game_data = GameData {
                    card_assets: card_assets.clone(),
                };
                processor.process_video(&game_data).unwrap();
                completed_videos += 1;
            }

            let duration = start_time.elapsed();
            let throughput = completed_videos as f64 / duration.as_secs_f64();
            println!("Throughput: {} videos/second", throughput);
        })
    });

    // Error rate test
    group.bench_function("error_rate_test", |b| {
        b.iter(|| {
            let total_attempts = 10; // Reduced for testing
            let mut errors = 0;

            for i in 0..total_attempts {
                let result = std::panic::catch_unwind(|| {
                    let mut processor =
                        VideoProcessor::new(input_video, &format!("bench_error_{}.mp4", i))
                            .unwrap();
                    let game_data = GameData {
                        card_assets: card_assets.clone(),
                    };
                    processor.process_video(&game_data)
                });

                if result.is_err() {
                    errors += 1;
                }
            }

            let error_rate = (errors as f64 / total_attempts as f64) * 100.0;
            println!("Error Rate: {}%", error_rate);
        })
    });

    // Sequential scalability test
    group.bench_function("scalability_test", |b| {
        b.iter(|| {
            let workloads = vec![1, 2, 4]; // Reduced workload sizes

            for load in workloads {
                let start_time = Instant::now();

                // Process videos sequentially
                for i in 0..load {
                    let mut processor = VideoProcessor::new(
                        input_video,
                        &format!("bench_scale_{}_{}.mp4", load, i),
                    )
                    .unwrap();
                    let game_data = GameData {
                        card_assets: card_assets.clone(),
                    };
                    processor.process_video(&game_data).unwrap();
                }

                let duration = start_time.elapsed();
                println!("Load {}: {}ms", load, duration.as_millis());
            }
        })
    });

    group.finish();
}

criterion_group!(benches, benchmark_video_processing);
criterion_main!(benches);
