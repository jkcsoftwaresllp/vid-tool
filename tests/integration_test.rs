use indicatif::{ProgressBar, ProgressStyle};
use serde_json::json;
use std::error::Error;
use std::path::PathBuf;
use vid_tool::{vid::GameData, vid::VideoProcessor};

#[test]
fn test_main() -> Result<(), Box<dyn Error>> {
    // Set up progress bar
    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {percent}% ({eta})",
            )?
            .progress_chars("#>-"),
    );

    // Test data based on the provided game stream
    let game_state = json!({
        "gameType": "TEEN_PATTI",
        "roundId": "TP1_1740827233326",
        "status": "dealing",
        "cards": {
            "jokerCard": null,
            "blindCard": "CQ",
            "playerA": ["C8", "C9", "C10"],
            "playerB": ["S4", "H4", "SK"],
            "playerC": []
        },
        "winner": "playerA",
        "startTime": 124
    });

    // Set up paths
    let input_video = "assets/videos/TEEN_PATTI/HostA/playerA_1.mp4";
    let output_video = "test_output.mp4";

    // Initialize VideoProcessor
    let mut processor = VideoProcessor::new(input_video, output_video)?;

    // Create GameData with card assets
    let mut card_assets = Vec::new();

    // Add player A cards
    for card in game_state["cards"]["playerA"].as_array().unwrap() {
        card_assets.push(
            processor
                .get_card_asset_path(card.as_str().unwrap())
                .to_string_lossy()
                .to_string(),
        );
    }

    // Add player B cards
    for card in game_state["cards"]["playerB"].as_array().unwrap() {
        card_assets.push(
            processor
                .get_card_asset_path(card.as_str().unwrap())
                .to_string_lossy()
                .to_string(),
        );
    }

    let game_data = GameData { card_assets };

    println!("Starting video processing...");

    // Process video with progress callback
    processor.process_game_video(&game_data, |progress| {
        pb.set_position(progress as u64);
        Ok(())
    })?;

    pb.finish_with_message("Video processing completed!");

    // Verify output video was created
    assert!(PathBuf::from(output_video).exists());

    Ok(())
}
