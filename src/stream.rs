use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

use tokio::time::Duration;

use serde::{Deserialize, Serialize};

// Import GameData from vid module
use crate::vid::VideoProcessor;

// Add OpenCV imports
use opencv::{
    core::{Mat, Vector},
    imgcodecs,
    prelude::*,
};

#[derive(Deserialize)]
struct ProcessRequest {
    phase: String, // "non_dealing_stream", "dealing_stage", "switch_to_dealing", "return_to_non_dealing"
    game: String,
    host: String,
    roundId: String,
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

#[derive(Serialize, Debug)]
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

struct WebSocketBroadcaster {
    connections: Arc<Mutex<HashMap<String, Vec<tokio::sync::mpsc::UnboundedSender<Message>>>>>,
}

impl WebSocketBroadcaster {
    fn new() -> Self {
        WebSocketBroadcaster {
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn broadcast(
        &self,
        round_id: &str,
        frame_response: &ProcessResponse,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response_str = serde_json::to_string(frame_response)?;
        let mut conns = self.connections.lock().unwrap();

        if let Some(round_connections) = conns.get_mut(round_id) {
            println!("Broadcasting to {} connections", round_connections.len());
            round_connections.retain(|tx| {
                match tx.send(Message::Text(response_str.clone().into())) {
                    Ok(_) => {
                        println!("Successfully sent frame");
                        true
                    }
                    Err(e) => {
                        println!("Failed to send frame: {:?}", e);
                        false
                    }
                }
            });
        } else {
            println!("No connections found for round_id: {}", round_id);
        }
        Ok(())
    }
}

pub fn start_streaming(socket_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let _ = std::fs::remove_file(socket_path);
    let unix_listener = UnixListener::bind(socket_path)?;
    println!("Video processor listening on {}", socket_path);

    let broadcaster = Arc::new(WebSocketBroadcaster::new());
    let game_streams = Arc::new(Mutex::new(HashMap::new()));

    // Start WebSocket server
    let ws_broadcaster = broadcaster.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ws_addr = "127.0.0.1:8000";
            let ws_listener = TcpListener::bind(ws_addr).await.unwrap();
            println!("WebSocket server listening on {}", ws_addr);

            while let Ok((stream, _)) = ws_listener.accept().await {
                let broadcaster = ws_broadcaster.clone();
                tokio::spawn(async move { handle_ws_connection(stream, broadcaster).await });
            }
        });
    });

    // Handle Unix socket connections
    for stream in unix_listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let broadcaster = broadcaster.clone();
                let game_streams = game_streams.clone();
                thread::spawn(move || {
                    let _ = handle_connection(&mut stream, broadcaster, game_streams);
                });
            }
            Err(err) => eprintln!("Error accepting connection: {}", err),
        }
    }

    Ok(())
}

async fn handle_ws_connection(
    stream: TcpStream,
    broadcaster: Arc<WebSocketBroadcaster>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("New WebSocket connection established");
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let (ws_sender, mut ws_receiver) = ws_stream.split();

    // Create a channel for this specific connection
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    // Handle incoming messages in a separate task
    let broadcaster_clone = broadcaster.clone();
    let handle_messages = tokio::spawn(async move {
        println!("Started message handling task");
        while let Some(msg) = ws_receiver.next().await {
            let msg = msg?;
            if let Message::Text(text) = msg {
                println!("Received message: {}", text);
                if let Ok(join_msg) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(round_id) = join_msg.get("joinVideoStream") {
                        let round_id = round_id.as_str().unwrap_or_default();
                        println!("Client joined video stream for round: {}", round_id);
                        let mut conns = broadcaster_clone.connections.lock().unwrap();
                        conns
                            .entry(round_id.to_string())
                            .or_insert_with(Vec::new)
                            .push(tx.clone());
                    }
                }
            }
        }
        println!("Message handling task completed");
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Forward messages from the channel to the WebSocket
    let forward_messages = tokio::spawn(async move {
        println!("Started message forwarding task");
        let mut ws_sender = ws_sender;
        while let Some(msg) = rx.recv().await {
            ws_sender.send(msg).await?;
        }
        println!("Message forwarding task completed");
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    });

    // Wait for either task to complete
    tokio::select! {
        res = handle_messages => {
            println!("Handle messages task finished: {:?}", res);
        },
        res = forward_messages => {
            println!("Forward messages task finished: {:?}", res);
        },
    }

    println!("WebSocket connection handled");
    Ok(())
}

fn handle_connection(
    stream: &mut UnixStream,
    broadcaster: Arc<WebSocketBroadcaster>,
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
        "non_dealing" => handle_non_dealing_stream(
            stream,
            &request.game,
            broadcaster.clone(),
            game_streams,
            &request.roundId,
        ),
        "dealing" => {
            let game_state = request
                .game_state
                .ok_or("Dealing stage requires game state")?;
            println!("Received game state: {:#?}", game_state);
            handle_dealing_stage(
                stream,
                &request.host,
                &request.game,
                game_state,
                broadcaster.clone(),
                game_streams,
            )
        }
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
    broadcaster: Arc<WebSocketBroadcaster>,
    game_streams: Arc<Mutex<HashMap<String, GameStream>>>,
    round_id: &str, // Add round_id parameter
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
    let mut frame_count = 0;
    loop {
        // Check stage before attempting to read next frame
        {
            let gs = game_streams.lock().unwrap();
            if let Some(current_stream) = gs.get(game_type) {
                if current_stream.stage == StreamingStage::Dealing {
                    println!("Switching to dealing stage");
                    return Ok(());
                }
            }
        }

        // Only read and send frame if still in non-dealing stage
        if !processor.source.read(&mut frame)? {
            println!("Reached end of video, resetting");
            processor.reset_frame_count()?;
            continue;
        }

        frame_count += 1;
        println!("Processing frame #{}", frame_count);

        send_frame(stream, &frame, &processor, &broadcaster, round_id)?;

        // Add sleep to control frame rate
        std::thread::sleep(Duration::from_millis(33)); // ~30 fps
    }
}

fn handle_dealing_stage(
    stream: &mut UnixStream,
    host: &String,
    game_type: &str,
    game_state: GameState,
    broadcaster: Arc<WebSocketBroadcaster>,
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
    let dealing_video = get_dealing_video(game_type, host, game_state.winner.as_deref())?;
    let mut processor = VideoProcessor::new(&dealing_video)?;

    let mut frame = Mat::default();
    while processor.source.read(&mut frame)? {
        // Apply game state specific modifications
        processor.process_dealing_frame(&mut frame, &game_state)?;
        println!("Processing frame: {}", processor.get_frame_number()?);
        send_frame(
            stream,
            &frame,
            &processor,
            &broadcaster,
            &game_state.roundId,
        )?;
        std::thread::sleep(Duration::from_millis(33));
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
    broadcaster: &WebSocketBroadcaster,
    round_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = Vector::new();
    imgcodecs::imencode(".jpg", &frame, &mut buffer, &Vector::new())?;
    let frame_data = base64::encode(&buffer);

    let frame_response = ProcessResponse::Frame {
        frame_number: processor.get_frame_number()?,
        frame_data,
        total_frames: processor.get_total_frames()?,
    };

    println!("Sending frames... to round: {}", round_id);

    // Broadcast to WebSocket clients
    broadcaster.broadcast(round_id, &frame_response)?;

    // Also send response back to Unix domain socket
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

fn get_dealing_video(
    game_type: &str,
    host: &String,
    winner: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    use rand::Rng;
    let _random_num = rand::thread_rng().gen_range(1..=9);

    match game_type {
        "ANDAR_BAHAR_TWO" | "TEEN_PATTI" => {
            let vpath = format!(
                "assets/videos/{}/{}/{}_{}.mp4",
                game_type,
                host,
                winner.unwrap_or("default"),
                "1"
            );

            println!("Dealing video path: {}", vpath);

            Ok(vpath)
        }
        _ => Err("Unsupported game type".into()),
    }
}
