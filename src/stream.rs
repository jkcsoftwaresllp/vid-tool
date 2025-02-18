use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

// Import GameData from vid module
use crate::vid::VideoProcessor;

// Add OpenCV imports
use opencv::{
    core::{Mat, Vector},
    imgcodecs,
    prelude::*
};

#[derive(Deserialize)]
struct ProcessRequest {
    phase: String, // "non_dealing_stream", "dealing_stage", "switch_to_dealing", "return_to_non_dealing"
    game: String,
    host: String,
    game_state: Option<GameState>,
}

#[derive(Deserialize, Clone, Debug)]
#[allow(non_snake_case, dead_code)]
pub struct GameState {
    gameType: String,
    roundId: String,
    pub cards: Cards,
    winner: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[allow(non_snake_case, dead_code)]
pub struct Cards {
    pub jokerCard: Option<String>,
    pub blindCard: Option<String>,
    pub playerA: Vec<String>,
    pub playerB: Vec<String>,
    pub playerC: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(tag = "status")]
#[allow(dead_code)]
enum ProcessResponse {
    #[serde(rename = "progress")]
    Progress { progress: f32 },

    #[serde(rename = "completed")]
    Completed { message: String },

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

#[derive(Clone, PartialEq, Debug)]
enum StreamingStage {
    NonDealing,
    Dealing,
}

// Removed the processor field from GameStream.
#[derive(Debug)]
struct GameStream {
    stage: StreamingStage,
    game_state: Option<GameState>,
}

pub fn start_streaming(socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    println!("Video processor listening on {}", socket_path);

    // Track active game streams
    let game_streams: Arc<Mutex<HashMap<String, GameStream>>> =
        Arc::new(Mutex::new(HashMap::new()));

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let game_streams = Arc::clone(&game_streams);
                thread::spawn(move || {
                    // println!("New connection established");
                    let _ = handle_connection(&mut stream, game_streams);
                });
            }
            Err(err) => eprintln!("Error accepting connection: {}", err),
        }
    }

    Ok(())
}

fn handle_connection(
    stream: &mut UnixStream,
    game_streams: Arc<Mutex<HashMap<String, GameStream>>>,
) -> Result<(), Box<dyn std::error::Error>> {

    // println!("checkpoint #1");
    
    let mut reader = BufReader::new(stream.try_clone()?);
    // println!("checkpoint #2");
    let mut input = String::new();
    reader.read_line(&mut input)?;
    // println!("checkpoint #3");

    let request: ProcessRequest = serde_json::from_str(&input).expect("Error parsing request");

    // println!("Preparing to print the request");
    // println!("Got request: { }!", request.req_type);

    match request.phase.as_str() {
        "non_dealing" => handle_non_dealing_stream(stream, &request.game, game_streams),
        "dealing" => {
            let game_state = request.game_state.ok_or("Dealing stage requires game state")?;
            // println!("Received game state: {:#?}", game_state);
            handle_dealing_stage(
            stream,
            &request.host,
            &request.game,
            game_state,
            game_streams,
        )},
        _ => {
            let error = ProcessResponse::Error {
                message: "Invalid request type".to_string(),
            };
            send_response(stream, &error)?;
            Err("Invalid request type".into())
        }
    }
}

fn handle_non_dealing_stream(
    stream: &mut UnixStream,
    game_type: &str,
    game_streams: Arc<Mutex<HashMap<String, GameStream>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let input_video = "assets/videos/re4.mp4".to_string();
    let mut processor = VideoProcessor::new(&input_video)?;

    {
        let mut gs = game_streams.lock().unwrap();
        gs.insert(
            game_type.to_string(),
            GameStream {
                stage: StreamingStage::NonDealing,
                game_state: None,
            },
        );
    }

    let mut frame = Mat::default();
    loop {
        // Check stage before attempting to read next frame
        {
            let gs = game_streams.lock().unwrap();
            if let Some(current_stream) = gs.get(game_type) {
                if current_stream.stage == StreamingStage::Dealing {
                    return Ok(());  // Exit immediately if dealing stage detected
                }
            }
        }

        // Only read and send frame if still in non-dealing stage
        if !processor.source.read(&mut frame)? {
            break;
        }
        send_frame(stream, &frame, &processor)?;
    }

    Ok(())
}

fn handle_dealing_stage(
    stream: &mut UnixStream,
    host: &String,
    game_type: &str,
    game_state: GameState,
    game_streams: Arc<Mutex<HashMap<String, GameStream>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    {
        let mut gs = game_streams.lock().unwrap();
        if let Some(game_stream) = gs.get_mut(game_type) {
            game_stream.stage = StreamingStage::Dealing;
            game_stream.game_state = Some(game_state.clone());
        }
    }

    // Process dealing specific frames with a new processor
    let dealing_video = get_dealing_video(
        game_type,
        host,
        game_state.winner.as_deref()
    )?;
    let mut processor = VideoProcessor::new(&dealing_video)?;

    let mut frame = Mat::default();
    while processor.source.read(&mut frame)? {
        // Apply game state specific modifications
        processor.process_dealing_frame(&mut frame, &game_state)?;
        println!("Processing frame: {}", processor.get_frame_number()?);
        send_frame(stream, &frame, &processor)?;
    }

    {
        // After processing, return to non-dealing stage
        let mut gs = game_streams.lock().unwrap();
        if let Some(game_stream) = gs.get_mut(game_type) {
            game_stream.stage = StreamingStage::NonDealing;
            game_stream.game_state = None;
        }
    }

    Ok(())
}

fn send_frame(
    stream: &mut UnixStream,
    frame: &Mat,
    processor: &VideoProcessor,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = Vector::new();
    imgcodecs::imencode(".jpg", &frame, &mut buffer, &Vector::new())?;
    let frame_data = base64::encode(&buffer);

    let frame_response = ProcessResponse::Frame {
        frame_number: processor.get_frame_number()?,
        frame_data,
        total_frames: processor.get_total_frames()?,
    };

    // if let ProcessResponse::Frame { frame_number, .. } = &frame_response {
    //     println!("Sending frame: {}", frame_number);
    // }

    send_response(stream, &frame_response)?;
    Ok(())
}

fn send_response(
    stream: &mut UnixStream,
    response: &ProcessResponse,
) -> Result<(), Box<dyn std::error::Error>> {
    let response_str = serde_json::to_string(response)?;
    stream.write_all(response_str.as_bytes())?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn get_dealing_video(game_type: &str, host: &String, winner: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    use rand::Rng;
    let _random_num = rand::thread_rng().gen_range(1..=9);
    
    match game_type {
        "ANDAR_BAHAR_TWO" | "TEEN_PATTI" => {
            let vpath = format!("assets/videos/{}/{}/{}_{}.mp4",
            game_type,
            host,
            winner.unwrap_or("default"),
            "1"
        );

        println!("Dealing video path: {}", vpath);

            Ok(vpath)
        },
        _ => Err("Unsupported game type".into()),
    }
}