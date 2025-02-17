use opencv::{
    core::{self, Point, Point2f, Vector},
    imgcodecs, imgproc,
    prelude::*,
    videoio,
};
use std::collections::HashMap;
use std::error::Error;

#[allow(dead_code)]
pub struct CardPlacement {
    position: core::Rect,
    card_asset_path: String,
    contour: Vector<Point>, // Add this field
}

pub struct GameData {
    pub card_assets: Vec<String>,
}

pub struct VideoProcessor {
   pub source: videoio::VideoCapture,
    card_assets: HashMap<String, Mat>,
    output: videoio::VideoWriter,
}

use crate::stream::GameState;

impl VideoProcessor {
    pub fn new(input_path: &str,) -> Result<Self, Box<dyn Error>> {
        let source = videoio::VideoCapture::from_file(input_path, videoio::CAP_ANY)?;

        // Get video properties
        let fps = source.get(videoio::CAP_PROP_FPS)?;
        let width = source.get(videoio::CAP_PROP_FRAME_WIDTH)? as i32;
        let height = source.get(videoio::CAP_PROP_FRAME_HEIGHT)? as i32;

        // Create VideoWriter
        let fourcc = videoio::VideoWriter::fourcc('a', 'v', 'c', '1')?; // TODO: use better codec
        let mut output = videoio::VideoWriter::new(
            "output_production.mp4",
            fourcc,
            fps,
            core::Size::new(width, height),
            true,
        )?;

        // Set higher bitrate
        output.set(videoio::VIDEOWRITER_PROP_QUALITY, 320.0)?;

        // Initialize card assets HashMap
        let card_assets = HashMap::new();

        Ok(VideoProcessor {
            source,
            card_assets,
            output,
        })
    }

    pub fn process_dealing_frame(&mut self, frame: &mut Mat, game_state: &GameState) -> Result<(), Box<dyn Error>> {
        // Create GameData from game state
        let game_data = self.create_game_data_from_state(game_state)?;
        
        // Detect placeholders and get card placements
        let placements = self.detect_placeholders(frame, &game_data)?;
        
        // Process frame with the detected placements
        self.process_frame(frame, &placements)?;
        
        Ok(())
    }

    fn create_game_data_from_state(&self, game_state: &GameState) -> Result<GameData, Box<dyn Error>> {
        let mut card_assets = Vec::new();

        // Process joker card if present
        if let Some(joker) = &game_state.cards.jokerCard {
            card_assets.push(self.get_card_asset_path(joker));
        }

        // Process blind card if present
        if let Some(blind) = &game_state.cards.blindCard {
            card_assets.push(self.get_card_asset_path(blind));
        }

        // Process player cards
        for card in &game_state.cards.playerA {
            card_assets.push(self.get_card_asset_path(card));
        }
        for card in &game_state.cards.playerB {
            card_assets.push(self.get_card_asset_path(card));
        }
        // Process playerC cards if present
        if let Some(player_c_cards) = &game_state.cards.playerC {
            for card in player_c_cards {
                card_assets.push(self.get_card_asset_path(card));
            }
        }

        Ok(GameData { card_assets })
    }

    fn get_card_asset_path(&self, card: &str) -> String {
        // Convert card code to asset path
        // Example: "H2" -> "assets/cards/hearts_2.png"
        let (suit, rank) = card.split_at(1);
        let suit_name = match suit {
            "H" => "hearts",
            "D" => "diamonds",
            "C" => "clubs",
            "S" => "spades",
            _ => "unknown",
        };
        
        format!("assets/cards/{}_{}.png", suit_name, rank)
    }

    pub fn switch_video_source(&mut self, video_path: &str) -> Result<(), Box<dyn Error>> {
        self.source = videoio::VideoCapture::from_file(video_path, videoio::CAP_ANY)?;
        Ok(())
    }

    pub fn reset_frame_count(&mut self) -> Result<(), Box<dyn Error>> {
        self.source.set(videoio::CAP_PROP_POS_FRAMES, 0.0)?;
        Ok(())
    }

    pub fn get_frame_number(&self) -> Result<i32, Box<dyn std::error::Error>> {
        Ok(self.source.get(videoio::CAP_PROP_POS_FRAMES)? as i32)
    }

    pub fn get_total_frames(&self) -> Result<i32, Box<dyn std::error::Error>> {
        Ok(self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32)
    }

    pub fn process_frame(
        &mut self,
        frame: &mut Mat,
        placements: &[CardPlacement],
    ) -> Result<(), Box<dyn Error>> {
        for placement in placements {
            let card_asset = self
                .card_assets
                .entry(placement.card_asset_path.clone())
                .or_insert_with(|| {
                    imgcodecs::imread(&placement.card_asset_path, imgcodecs::IMREAD_UNCHANGED)
                        .expect("Failed to load card asset")
                });

            // Get rotated rectangle and vertices
            let rot_rect = imgproc::min_area_rect(&placement.contour)?;
            let mut vertices = [core::Point2f::default(); 4];
            rot_rect.points(&mut vertices)?;

            // Sort vertices correctly (top-left, top-right, bottom-right, bottom-left)
            let center = vertices.iter().fold(Point2f::new(0.0, 0.0), |acc, &p| {
                Point2f::new(acc.x + p.x / 4.0, acc.y + p.y / 4.0)
            });

            vertices.sort_by(|a, b| {
                let a_angle = (a.y - center.y).atan2(a.x - center.x);
                let b_angle = (b.y - center.y).atan2(b.x - center.x);
                b_angle.partial_cmp(&a_angle).unwrap()
            });

            // Calculate width and height from the rotated rectangle
            let width = rot_rect.size.width as i32;
            let height = rot_rect.size.height as i32;

            // Create source points for a properly oriented rectangle
            let source_points = [
                Point2f::new(0.0, 0.0),
                Point2f::new(width as f32, 0.0),
                Point2f::new(width as f32, height as f32),
                Point2f::new(0.0, height as f32),
            ];

            // Create transformation matrices
            let src_points = Mat::from_slice(&source_points)?;
            let dst_points = Mat::from_slice(&vertices)?;

            // Resize card asset
            let mut resized_asset = Mat::default();
            imgproc::resize(
                card_asset,
                &mut resized_asset,
                core::Size::new(width, height),
                0.0,
                0.0,
                imgproc::INTER_CUBIC,
            )?;

            // Get perspective transform and apply it
            let transform_matrix =
                imgproc::get_perspective_transform(&src_points, &dst_points, core::DECOMP_LU)?;
            let mut warped = Mat::default();
            imgproc::warp_perspective(
                &resized_asset,
                &mut warped,
                &transform_matrix,
                frame.size()?,
                imgproc::INTER_CUBIC,
                core::BORDER_CONSTANT,
                core::Scalar::default(),
            )?;

            // Create mask from contour
            let mut mask =
                Mat::zeros(frame.size()?.height, frame.size()?.width, core::CV_8UC1)?.to_mat()?;
            let mut contours = Vector::<Vector<Point>>::new();
            contours.push(placement.contour.clone());
            imgproc::draw_contours(
                &mut mask,
                &contours,
                0,
                core::Scalar::new(255.0, 0.0, 0.0, 0.0),
                -1,
                imgproc::LINE_8,
                &Mat::default(),
                0,
                Point::new(0, 0),
            )?;

            // Blend the warped image with the frame using the mask
            // let mut warped_bgr = Mat::default();
            // core::convert_scale_abs(&warped, &mut warped_bgr, 1.0, 0.0)?;
            warped.copy_to_masked(frame, &mask)?;
        }
        Ok(())
    }

    pub fn process_video(&mut self, game_data: &GameData) -> Result<(), Box<dyn Error>> {
        let mut frame = Mat::default();
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        let mut frame_count = 0;

        while self.source.read(&mut frame)? {
            // Process frame
            let placements = self.detect_placeholders(&frame, game_data)?;
            self.process_frame(&mut frame, &placements)?;
            self.output.write(&frame)?;

            // Show progress
            frame_count += 1;
            if frame_count % 10 == 0 {
                // Update every 10 frames
                println!(
                    "Processing frame {}/{} ({:.1}%)",
                    frame_count,
                    total_frames,
                    (frame_count as f32 / total_frames as f32) * 100.0
                );
            }
        }
        Ok(())
    }

    pub fn detect_placeholders(
        &self,
        frame: &Mat,
        game_data: &GameData,
    ) -> Result<Vec<CardPlacement>, Box<dyn Error>> {
        // Create mask for pure green color in BGR
        let mut mask = Mat::default();
        let threshold = 30.0; // Tolerance for color detection

        let lower_green = core::Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
        let upper_green = core::Scalar::new(threshold, 255.0, threshold, 0.0);

        core::in_range(&frame, &lower_green, &upper_green, &mut mask)?;

        // Find contours
        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &mask,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;

        let mut placements = Vec::new();

        for contour in contours.iter() {
            let area = imgproc::contour_area(&contour, false)?;

            if area < 100.0 {
                // Adjust these thresholds as needed
                continue;
            }

            let rect = imgproc::bounding_rect(&contour)?;

            // Centered around 1.51
            placements.push(CardPlacement {
                position: rect,
                card_asset_path: game_data.card_assets[0].clone(),
                contour: contour.clone(),
            });
        }

        Ok(placements)
    }

    pub fn process_game_video<F>(
        &mut self,
        game_data: &GameData,
        mut progress_cb: F,
    ) -> Result<(), Box<dyn Error>>
    where
        F: FnMut(f32) -> Result<(), Box<dyn Error>>,
    {
        let mut frame = Mat::default();
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        let mut frame_count = 0;

        while self.source.read(&mut frame)? {
            let placements = self.detect_placeholders(&frame, game_data)?;
            self.process_frame(&mut frame, &placements)?;
            self.output.write(&frame)?;

            frame_count += 1;
            if frame_count % 10 == 0 {
                let progress = (frame_count as f32 / total_frames as f32) * 100.0;
                progress_cb(progress)?;
            }
        }
        Ok(())
    }
}