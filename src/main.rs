#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(non_snake_case)]

mod stream;
mod system;
mod vid;

use std::collections::HashMap;
use std::error::Error;

use image::ImageFormat;
use std::fs;
use std::path::Path;

fn convert_png_to_jpg() -> Result<(), Box<dyn Error>> {
    // Create output directory if it doesn't exist
    fs::create_dir_all("assets/cards_jpg")?;

    // Read all files from png directory
    let entries = fs::read_dir("assets/cards_png")?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // Skip if not a PNG file
        if path.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }

        // Get the filename without extension
        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or("Invalid filename")?;

        // Create output path
        let output_path = format!("assets/cards_jpg/{}.jpg", file_stem);

        println!("Converting: {} -> {}", path.display(), output_path);

        // Open the image
        let img = image::open(&path)?;

        // Save as JPG
        img.save_with_format(&output_path, ImageFormat::Jpeg)?;
    }

    println!("Conversion completed successfully!");
    Ok(())
}
fn main() {
    // if let Err(e) = convert_png_to_jpg() {
    //     eprintln!("Error during conversion: {}", e);
    // }

    let socket_path = "/tmp/video-processor.sock";

    stream::start_streaming(socket_path).expect("Error running streaming service");
    // TODO: Handle error appropriately
}
