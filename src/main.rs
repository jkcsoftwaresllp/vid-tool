#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_imports)]

use opencv::{
    core::{self, Point, Rect, Size, Vector},
    imgcodecs, imgproc,
    prelude::*,
    types, videoio,
};
use std::collections::HashMap;
use std::error::Error;

struct CardPlacement {
    position: core::Rect,
    card_asset_path: String,
    contour: Vector<Point>, // Add this field
}

struct VideoProcessor {
    source: videoio::VideoCapture,
    card_assets: HashMap<String, Mat>,
    output: videoio::VideoWriter,
}

impl VideoProcessor {
    fn new(input_path: &str, output_path: &str) -> Result<Self, Box<dyn Error>> {
        let source = videoio::VideoCapture::from_file(input_path, videoio::CAP_ANY)?;

        // Get video properties
        let fps = source.get(videoio::CAP_PROP_FPS)?;
        let width = source.get(videoio::CAP_PROP_FRAME_WIDTH)? as i32;
        let height = source.get(videoio::CAP_PROP_FRAME_HEIGHT)? as i32;

        // Create VideoWriter
        let fourcc = videoio::VideoWriter::fourcc('M', 'J', 'P', 'G')?;
        let output = videoio::VideoWriter::new(
            output_path,
            fourcc,
            fps,
            core::Size::new(width, height),
            true,
        )?;

        Ok(VideoProcessor {
            source,
            card_assets: HashMap::new(),
            output,
        })
    }

    fn process_frame(
        &mut self,
        frame: &mut Mat,
        placements: &[CardPlacement],
    ) -> Result<(), Box<dyn Error>> {
        for placement in placements {
            // Draw the actual contour in blue
            let mut contours = Vector::<Vector<Point>>::new();
            contours.push(placement.contour.clone());
            imgproc::draw_contours(
                frame,
                &contours,
                0,
                core::Scalar::new(255.0, 0.0, 0.0, 255.0), // Blue
                2,
                imgproc::LINE_8,
                &Mat::default(),
                0,
                Point::new(0, 0),
            )?;

            // Draw bounding rectangle in red
            // imgproc::rectangle(
            //     frame,
            //     placement.position,
            //     core::Scalar::new(0.0, 0.0, 255.0, 255.0), // Red
            //     2,
            //     imgproc::LINE_8,
            //     0,
            // )?;

            // Draw corner points in green
            let corners = [
                Point::new(placement.position.x, placement.position.y),
                Point::new(
                    placement.position.x + placement.position.width,
                    placement.position.y,
                ),
                Point::new(
                    placement.position.x,
                    placement.position.y + placement.position.height,
                ),
                Point::new(
                    placement.position.x + placement.position.width,
                    placement.position.y + placement.position.height,
                ),
            ];

            // for corner in &corners {
            //     imgproc::circle(
            //         frame,
            //         *corner,
            //         5,
            //         core::Scalar::new(0.0, 255.0, 0.0, 255.0), // Green
            //         -1,
            //         imgproc::LINE_8,
            //         0,
            //     )?;
            // }
        }
        Ok(())
    }

    fn process_video(&mut self, game_data: &GameData) -> Result<(), Box<dyn Error>> {
        let mut frame = Mat::default();

        while self.source.read(&mut frame)? {
            // Detect card placeholders
            let placements = self.detect_placeholders(&frame, game_data)?;

            // Process frame with overlays
            self.process_frame(&mut frame, &placements)?;

            // Write processed frame
            self.output.write(&frame)?;
        }
        Ok(())
    }

    fn detect_placeholders(
        &self,
        frame: &Mat,
        game_data: &GameData,
    ) -> Result<Vec<CardPlacement>, Box<dyn Error>> {
        // Convert to HSV
        let mut hsv = Mat::default();
        imgproc::cvt_color(&frame, &mut hsv, imgproc::COLOR_BGR2HSV, 0)?;

        // Create mask for white color in HSV
        let mut mask = Mat::default();
        let lower_white = core::Scalar::new(0.0, 0.0, 200.0, 0.0); // Low saturation, high value
        let upper_white = core::Scalar::new(180.0, 30.0, 255.0, 0.0);
        core::in_range(&hsv, &lower_white, &upper_white, &mut mask)?;

        // Apply morphological operations to clean up the mask
        let kernel = Mat::ones(5, 5, core::CV_8U)?;

        // Close small gaps
        let mut closed = Mat::default();
        imgproc::morphology_ex(
            &mask,
            &mut closed,
            imgproc::MORPH_CLOSE,
            &kernel,
            Point::new(-1, -1),
            2,
            core::BORDER_CONSTANT,
            core::Scalar::default(),
        )?;

        // Remove noise
        let mut opened = Mat::default();
        imgproc::morphology_ex(
            &closed,
            &mut opened,
            imgproc::MORPH_OPEN,
            &kernel,
            Point::new(-1, -1),
            1,
            core::BORDER_CONSTANT,
            core::Scalar::default(),
        )?;

        // Find contours
        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &opened,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;

        let mut placements = Vec::new();

        for (i, contour) in contours.iter().enumerate() {
            let area = imgproc::contour_area(&contour, false)?;

            // Filter by area
            if area < 3000.0 || area > 10000.0 {
                continue;
            }

            // Approximate the contour
            let epsilon = 0.04 * imgproc::arc_length(&contour, true)?;
            let mut approx = Vector::<Point>::new();
            imgproc::approx_poly_dp(&contour, &mut approx, epsilon, true)?;

            // Check if it's roughly rectangular (4 corners, with some tolerance)
            if approx.len() >= 4 && approx.len() <= 6 {
                let rect = imgproc::bounding_rect(&contour)?;
                let aspect_ratio = rect.width as f64 / rect.height as f64;

                // Check aspect ratio
                if aspect_ratio > 1.2 && aspect_ratio < 1.8 {
                    // Save debug visualization for first detection
                    static mut FIRST_DETECTION: bool = true;
                    unsafe {
                        if FIRST_DETECTION {
                            imgcodecs::imwrite("debug_hsv.png", &hsv, &Vector::new())?;
                            imgcodecs::imwrite("debug_mask.png", &mask, &Vector::new())?;
                            imgcodecs::imwrite("debug_closed.png", &closed, &Vector::new())?;
                            imgcodecs::imwrite("debug_opened.png", &opened, &Vector::new())?;

                            // Draw contour on original frame for debug
                            let mut debug_frame = frame.clone();
                            imgproc::draw_contours(
                                &mut debug_frame,
                                &contours,
                                i as i32,
                                core::Scalar::new(0.0, 0.0, 255.0, 255.0),
                                2,
                                imgproc::LINE_8,
                                &Mat::default(),
                                0,
                                Point::new(0, 0),
                            )?;
                            imgcodecs::imwrite("debug_contour.png", &debug_frame, &Vector::new())?;

                            FIRST_DETECTION = false;
                        }
                    }

                    // Only add the placement once
                    placements.push(CardPlacement {
                        position: rect,
                        card_asset_path: game_data.card_assets[0].clone(),
                        contour: contour.clone(),
                    });
                }
            }
        }

        Ok(placements)
    }
}
// Example game data structure
struct GameData {
    card_assets: Vec<String>,
}

impl GameData {
    fn get_card_asset(&self, index: usize) -> Option<&String> {
        self.card_assets.get(index)
    }
}

fn analyze_frame(frame_path: &str) -> Result<(), Box<dyn Error>> {
    let frame = imgcodecs::imread(frame_path, imgcodecs::IMREAD_COLOR)?;
    let mut display_frame = frame.clone();

    // Convert to HSV for better white detection
    let mut hsv = Mat::default();
    imgproc::cvt_color(&frame, &mut hsv, imgproc::COLOR_BGR2HSV, 0)?;

    // Create mask for white color
    let mut mask = Mat::default();
    let lower_white = core::Scalar::new(0.0, 0.0, 200.0, 0.0);
    let upper_white = core::Scalar::new(180.0, 30.0, 255.0, 0.0);
    core::in_range(&hsv, &lower_white, &upper_white, &mut mask)?;

    // Find contours
    let mut contours = Vector::<Vector<Point>>::new();
    imgproc::find_contours(
        &mask,
        &mut contours,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )?;

    for (i, contour) in contours.iter().enumerate() {
        let area = imgproc::contour_area(&contour, false)?;
        if area < 1000.0 {
            continue;
        }

        // Draw the actual contour in blue
        imgproc::draw_contours(
            &mut display_frame,
            &contours,
            i as i32,
            core::Scalar::new(255.0, 0.0, 0.0, 255.0), // Blue
            2,
            imgproc::LINE_8,
            &Mat::default(),
            0,
            Point::new(0, 0),
        )?;

        // Get and draw the normal bounding rectangle in green
        let rect = imgproc::bounding_rect(&contour)?;
        imgproc::rectangle(
            &mut display_frame,
            rect,
            core::Scalar::new(0.0, 255.0, 0.0, 255.0), // Green
            2,
            imgproc::LINE_8,
            0,
        )?;

        // Get rotated rectangle
        let rot_rect = imgproc::min_area_rect(&contour)?;

        // Get vertices of rotated rectangle
        let mut vertices = [core::Point2f::default(); 4];
        rot_rect.points(&mut vertices)?;

        // Draw rotated rectangle
        for i in 0..4 {
            let p1 = Point::new(vertices[i].x as i32, vertices[i].y as i32);
            let p2 = Point::new(
                vertices[(i + 1) % 4].x as i32,
                vertices[(i + 1) % 4].y as i32,
            );
            imgproc::line(
                &mut display_frame,
                p1,
                p2,
                core::Scalar::new(0.0, 0.0, 255.0, 255.0), // Red
                2,
                imgproc::LINE_AA,
                0,
            )?;
        }

        // Add measurements for all three
        let aspect_ratio = rect.width as f64 / rect.height as f64;
        let rot_width = rot_rect.size.width;
        let rot_height = rot_rect.size.height;
        let rot_aspect = if rot_width > rot_height {
            rot_width / rot_height
        } else {
            rot_height / rot_width
        };

        let text = format!(
            "Contour {}\nNormal: {}x{} AR:{:.2} A:{}\nRotated: {:.0}x{:.0} AR:{:.2} Ang:{:.1}°",
            i,
            rect.width,
            rect.height,
            aspect_ratio,
            area,
            rot_width,
            rot_height,
            rot_aspect,
            rot_rect.angle
        );

        // Split text into lines and draw each line
        for (line_num, line) in text.split('\n').enumerate() {
            imgproc::put_text(
                &mut display_frame,
                line,
                Point::new(rect.x, rect.y - 10 + (line_num as i32 * 20)),
                imgproc::FONT_HERSHEY_SIMPLEX,
                0.5,
                core::Scalar::new(255.0, 255.0, 255.0, 255.0),
                2,
                imgproc::LINE_AA,
                false,
            )?;
        }
    }

    imgcodecs::imwrite(
        "analyzed_frame_detailed.png",
        &display_frame,
        &Vector::new(),
    )?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut cap = videoio::VideoCapture::from_file("temp1.mp4", videoio::CAP_ANY)?;
    let mut frame = Mat::default();

    // Skip to frame 200
    cap.set(videoio::CAP_PROP_POS_FRAMES, 200.0)?;
    cap.read(&mut frame)?;
    imgcodecs::imwrite("frame_200.png", &frame, &Vector::new())?;

    analyze_frame("frame_200.png")?;
    println!("Analysis complete! Check 'analyzed_frame_detailed.png'");

    // Verify input files exist
    let input_video = "temp1.mp4";
    let card1_path = "card1.png";
    let card2_path = "card2.png";

    if !std::path::Path::new(input_video).exists() {
        return Err(format!("Input video not found: {}", input_video).into());
    }
    if !std::path::Path::new(card1_path).exists() {
        return Err(format!("Card asset not found: {}", card1_path).into());
    }
    if !std::path::Path::new(card2_path).exists() {
        return Err(format!("Card asset not found: {}", card2_path).into());
    }

    let mut processor = VideoProcessor::new(input_video, "output.mp4")?;

    let game_data = GameData {
        card_assets: vec![card1_path.to_string(), card2_path.to_string()],
    };

    println!("Processing video...");
    processor.process_video(&game_data)?;
    println!("Video processing completed!");
    Ok(())
}
