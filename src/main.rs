#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_snake_case)]

mod stream;
mod system;
mod vid;

use std::collections::HashMap;
use std::error::Error;

use std::env;
use vid_tool::vid::VideoProcessor;

fn main() {
    let ws_addr = "0.0.0.0:5500";

    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "preprocess" {
        println!("Starting video pre-processing...");
        if let Err(e) = preprocess_videos() {
            eprintln!("Error during pre-processing: {}", e);
            return;
        }
        println!("Pre-processing completed successfully!");
        return;
    }

    let socket_path = "/tmp/video-processor.sock";
    stream::start_streaming(socket_path, ws_addr).expect("Error running streaming service");
}

fn preprocess_videos() -> Result<(), Box<dyn Error>> {
    let video_dir = "assets/videos";

    // Process each game type directory
    for game_entry in std::fs::read_dir(video_dir)? {
        let game_entry = game_entry?;
        if !game_entry.file_type()?.is_dir() {
            continue;
        }

        let game_path = game_entry.path();

        // Process each host directory
        for host_entry in std::fs::read_dir(&game_path)? {
            let host_entry = host_entry?;
            if !host_entry.file_type()?.is_dir() {
                continue;
            }

            let host_path = host_entry.path();

            // Process each video file
            for video_entry in std::fs::read_dir(&host_path)? {
                let video_entry = video_entry?;
                let path = video_entry.path();

                // Skip if not an mp4 file
                if !path.extension().map_or(false, |ext| ext == "mp4") {
                    continue;
                }

                // Skip files starting with "nd-"
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    if file_name.starts_with("nd-") {
                        continue;
                    }
                }

                let placeholder_path = path.with_extension("json");

                println!("Processing {}", path.display());

                let mut processor =
                    VideoProcessor::new(path.to_str().unwrap(), "dummy_output.mp4")?;

                processor.scan_and_save_placeholders(placeholder_path.to_str().unwrap())?;
            }
        }
    }

    Ok(())
}
