#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_snake_case)]

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
mod vid;
use std::collections::HashMap;
use std::error::Error;
use vid::VideoProcessor;

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
}

fn main() -> Result<(), Box<dyn Error>> {
    let socket_path = "/tmp/video-processor.sock";
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)?;
    println!("Video processor listening on {}", socket_path);

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                // Read the entire input into a string first
                let mut input = String::new();
                {
                    let mut reader = BufReader::new(&stream);
                    reader.read_line(&mut input)?;
                }

                // Now process the input
                match serde_json::from_str::<ProcessRequest>(&input) {
                    Ok(_req) => {
                        let received = ProcessResponse::Received {
                            message: "Request received successfully".to_string(),
                        };
                        let response = serde_json::to_string(&received)?;
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
