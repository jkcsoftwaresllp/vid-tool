use indicatif::{ProgressBar, ProgressStyle};
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

    // Verify input files exist
    let input_video = "assets/videos/TEEN_PATTI/HostA/playerA_1.mp4";
    let card1_path = "card1.jpg";
    let card2_path = "card2.jpg";

    for path in &[input_video, card1_path, card2_path] {
        if !std::path::Path::new(path).exists() {
            pb.finish_with_message("Error: Missing input file");
            return Err(format!("Input file not found: {}", path).into());
        }
    }

    let mut processor = VideoProcessor::new(input_video)?;

    let game_data = GameData {
        card_assets: vec![card1_path.to_string().into(), card2_path.to_string().into()],
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
