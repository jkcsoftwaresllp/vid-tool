#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_snake_case)]

mod vid;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;

// Add OpenCV imports
use opencv::{
    core::{Mat, Vector},
    imgcodecs,
    prelude::*,
    videoio,
};

// Import GameData from vid module
use crate::vid::{GameData, VideoProcessor};

#[derive(Deserialize)]
struct ProcessRequest {
    req_type: String, // "game_state_video"
    gameState: GameState,
    output_path: String,
}

#[derive(Deserialize)]
struct GameState {
    gameType: String,
    gameId: String,
    status: String,
    cards: Cards,
    winner: Option<String>,
    startTime: i64,
}

#[derive(Deserialize)]
struct Cards {
    jokerCard: Option<String>,
    blindCard: Option<String>,
    playerA: Vec<String>,
    playerB: Vec<String>,
    playerC: Vec<String>,
}

#[derive(Serialize)]
#[serde(tag = "status")]
enum ProcessResponse {
    #[serde(rename = "progress")]
    Progress { progress: f32 },

    #[serde(rename = "completed")]
    Completed { output_path: String },

    #[serde(rename = "error")]
    Error { message: String },

    #[serde(rename = "received")]
    Received { message: String },

    #[serde(rename = "frame")]
    Frame {
        frame_number: i32,
        frame_data: String, // Base64 encoded frame
        total_frames: i32,
    },
}

fn main() -> Result<(), Box<dyn Error>> {
    let socket_path = "/tmp/video-processor.sock";
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)?;
    println!("Video processor listening on {}", socket_path);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                // This gives us the actual UnixStream
                let mut input = String::new();
                {
                    let mut reader = BufReader::new(&stream);
                    reader.read_line(&mut input)?;
                }

                match serde_json::from_str::<ProcessRequest>(&input) {
                    Ok(req) => {
                        let received = ProcessResponse::Received {
                            message: "Starting video processing".to_string(),
                        };
                        let response = serde_json::to_string(&received)?;
                        stream.write_all(response.as_bytes())?; // No ? before write_all needed here
                        stream.write_all(b"\n")?;

                        let input_video = match req.gameState.gameType.as_str() {
                            "AndarBaharGame" => "assets/teen_patti_1.mp4",
                            // "ANDAR_BAHAR" => "assets/andar_bahar_template.mp4",
                            // "LUCKY7B" => "assets/lucky7_template.mp4",
                            // "DRAGON_TIGER" => "assets/dragon_tiger_template.mp4",
                            // "TEEN_PATTI" => "assets/teen_patti_template.mp4",
                            _ => return Err("Unsupported game type".into()),
                        };

                        let mut processor = VideoProcessor::new(input_video, &req.output_path)?;

                        let game_data = GameData {
                            card_assets: vec!["card1.jpg".to_string()],
                        };

                        let mut frame = Mat::default();
                        let total_frames =
                            processor.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
                        let mut frame_count = 0;

                        while processor.source.read(&mut frame)? {
                            let placements = processor.detect_placeholders(&frame, &game_data)?;
                            processor.process_frame(&mut frame, &placements)?;

                            let mut buffer = Vector::new();
                            imgcodecs::imencode(".jpg", &frame, &mut buffer, &Vector::new())?;
                            let frame_data = base64::encode(&buffer);

                            let frame_response = ProcessResponse::Frame {
                                frame_number: frame_count,
                                frame_data,
                                total_frames,
                            };
                            let response = serde_json::to_string(&frame_response)?;
                            stream.write_all(response.as_bytes())?;
                            stream.write_all(b"\n")?;

                            frame_count += 1;
                        }

                        let completed = ProcessResponse::Completed {
                            output_path: req.output_path,
                        };
                        let response = serde_json::to_string(&completed)?;
                        stream.write_all(response.as_bytes())?;
                        stream.write_all(b"\n")?;
                    }
                    Err(e) => {
                        let error = ProcessResponse::Error {
                            message: format!("Invalid request: {}", e),
                        };
                        let response = serde_json::to_string(&error)?;
                        stream.write_all(response.as_bytes())?;
                        stream.write_all(b"\n")?;
                    }
                }
            }
            Err(err) => {
                eprintln!("Error accepting connection: {}", err);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indicatif::{ProgressBar, ProgressStyle};
    use std::path::PathBuf;
    use std::time::Duration;

    #[test]
    fn test_main() -> Result<(), Box<dyn Error>> {
        // Set up progress bar
        let pb = ProgressBar::new(100);
        pb.set_style(ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {percent}% ({eta})")?
                .progress_chars("#>-"));

        // Verify input files exist
        let input_video = "assets/HostA_PlayerB.mp4";
        let card1_path = "card1.jpg";
        let card2_path = "card2.jpg";

        for path in &[input_video, card1_path, card2_path] {
            if !std::path::Path::new(path).exists() {
                pb.finish_with_message("Error: Missing input file");
                return Err(format!("Input file not found: {}", path).into());
            }
        }

        let mut processor = VideoProcessor::new(input_video, "output.mp4")?;

        let game_data = GameData {
            card_assets: vec![card1_path.to_string(), card2_path.to_string()],
        };

        println!("Starting video processing...");

        // Process video with progress callback
        processor.process_game_video(&game_data, |progress| {
            pb.set_position(progress as u64);
            Ok(())
        })?;

        pb.finish_with_message("Video processing completed!");

        assert!(PathBuf::from("output.mp4").exists());
        Ok(())
    }
}
