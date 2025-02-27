use opencv::{
    core::{self, Point, Point2f, Vector},
    imgcodecs, imgproc,
    prelude::*,
    videoio,
};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

#[allow(dead_code)]
pub struct CardPlacement {
    position: core::Rect,
    card_asset_path: PathBuf, // Use PathBuf instead of String
    contour: Vector<Point>,
}

pub struct GameData {
    pub card_assets: Vec<PathBuf>, // Use PathBuf instead of String
}

pub struct VideoProcessor {
    pub source: videoio::VideoCapture,
    card_assets: HashMap<PathBuf, Mat>,
    output: videoio::VideoWriter,
    // Cache for transformation matrices
    transform_cache: HashMap<(i32, i32), Mat>,
}

use crate::stream::GameState;

impl VideoProcessor {
    pub fn new(input_path: &str) -> Result<Self, Box<dyn Error>> {
        let source = videoio::VideoCapture::from_file(input_path, videoio::CAP_ANY)?;

        // Get video properties
        let fps = source.get(videoio::CAP_PROP_FPS)?;
        let width = source.get(videoio::CAP_PROP_FRAME_WIDTH)? as i32;
        let height = source.get(videoio::CAP_PROP_FRAME_HEIGHT)? as i32;

        // Create VideoWriter with better codec settings
        let fourcc = videoio::VideoWriter::fourcc('H', '2', '6', '4')?; // H264 is generally better supported
        let mut output = videoio::VideoWriter::new(
            "output_production.mp4",
            fourcc,
            fps,
            core::Size::new(width, height),
            true,
        )?;

        // Set higher bitrate
        output.set(videoio::VIDEOWRITER_PROP_QUALITY, 320.0)?;

        Ok(VideoProcessor {
            source,
            card_assets: HashMap::new(),
            output,
            transform_cache: HashMap::new(),
        })
    }

    pub fn process_dealing_frame(
        &mut self,
        frame: &mut Mat,
        game_state: &GameState,
    ) -> Result<(), Box<dyn Error>> {
        // Create GameData from game state
        let game_data = self.create_game_data_from_state(game_state)?;

        // Pre-load all card assets at once to avoid loading during frame processing
        self.preload_card_assets(&game_data)?;

        // Detect placeholders and get card placements
        let placements = self.detect_placeholders(frame, &game_data)?;

        // Process frame with the detected placements
        self.process_frame(frame, &placements)?;

        Ok(())
    }

    fn preload_card_assets(&mut self, game_data: &GameData) -> Result<(), Box<dyn Error>> {
        for path in &game_data.card_assets {
            if !self.card_assets.contains_key(path) {
                let asset = imgcodecs::imread(
                    path.to_str().ok_or("Invalid path")?,
                    imgcodecs::IMREAD_UNCHANGED,
                )?;
                self.card_assets.insert(path.clone(), asset);
            }
        }
        Ok(())
    }

    fn create_game_data_from_state(
        &self,
        game_state: &GameState,
    ) -> Result<GameData, Box<dyn Error>> {
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

    fn get_card_asset_path(&self, card: &str) -> PathBuf {
        // Convert card code to asset path
        // Example: "H2" -> "assets/cards/hearts_2.png"
        let (suit, rank) = card.split_at(1);
        let suit_name = match suit {
            "H" => "hearts",
            "D" => "diamond",
            "C" => "clubs",
            "S" => "spades",
            _ => "unknown",
        };

        PathBuf::from(format!(
            "assets/cards/{}_{}.png",
            suit_name,
            rank.to_lowercase()
        ))
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
        // Pre-allocate buffers to avoid reallocations
        let mut mask =
            Mat::zeros(frame.size()?.height, frame.size()?.width, core::CV_8UC1)?.to_mat()?;
        let mut warped = Mat::default();
        let frame_size = frame.size()?;

        for placement in placements {
            // Get card asset from cache - using references to avoid cloning
            let card_asset = match self.card_assets.get(&placement.card_asset_path) {
                Some(asset) => asset,
                None => {
                    // Load only if not in cache
                    let path_str = placement.card_asset_path.to_str().ok_or("Invalid path")?;
                    let asset = imgcodecs::imread(path_str, imgcodecs::IMREAD_UNCHANGED)?;
                    self.card_assets
                        .insert(placement.card_asset_path.clone(), asset);
                    self.card_assets.get(&placement.card_asset_path).unwrap()
                }
            };

            // Get rotated rectangle
            let rot_rect = imgproc::min_area_rect(&placement.contour)?;
            let mut vertices = [core::Point2f::default(); 4];
            rot_rect.points(&mut vertices)?;

            // Sort vertices (this part can be optimized using direct comparisons)
            self.sort_vertices(&mut vertices);

            // Calculate width and height
            let width = rot_rect.size.width as i32;
            let height = rot_rect.size.height as i32;

            // Use cached transformation matrix if available
            let transform_matrix = self.get_transform_matrix(width, height, &vertices)?;

            // Resize card asset only once per unique size
            let mut resized_asset = Mat::default();
            imgproc::resize(
                card_asset,
                &mut resized_asset,
                core::Size::new(width, height),
                0.0,
                0.0,
                imgproc::INTER_LINEAR, // INTER_LINEAR is faster than INTER_CUBIC with minimal visual difference
            )?;

            // Apply perspective transform
            imgproc::warp_perspective(
                &resized_asset,
                &mut warped,
                &transform_matrix,
                frame_size,
                imgproc::INTER_LINEAR,
                core::BORDER_CONSTANT,
                core::Scalar::default(),
            )?;

            // Create mask from contour (reuse the same mask buffer)
            mask.set_to(&core::Scalar::new(0.0, 0.0, 0.0, 0.0), &Mat::default())?;
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
            warped.copy_to_masked(frame, &mask)?;
        }
        Ok(())
    }

    // Helper method to sort vertices more efficiently
    fn sort_vertices(&self, vertices: &mut [Point2f; 4]) {
        // Calculate center
        let center = vertices.iter().fold(Point2f::new(0.0, 0.0), |acc, &p| {
            Point2f::new(acc.x + p.x / 4.0, acc.y + p.y / 4.0)
        });

        // Sort vertices based on their angle from center
        // This implementation avoids repeated atan2 calculations
        vertices.sort_by(|&a, &b| {
            let a_dx = a.x - center.x;
            let a_dy = a.y - center.y;
            let b_dx = b.x - center.x;
            let b_dy = b.y - center.y;

            // Compare quadrants first (faster than atan2)
            let a_quad = Self::get_quadrant(a_dx, a_dy);
            let b_quad = Self::get_quadrant(b_dx, b_dy);

            if a_quad != b_quad {
                return b_quad.cmp(&a_quad);
            }

            // If in same quadrant, compare slopes
            let cross = b_dx * a_dy - a_dx * b_dy;
            if cross < 0.0 {
                std::cmp::Ordering::Less
            } else if cross > 0.0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    }

    // Helper to determine point quadrant (faster than atan2)
    fn get_quadrant(dx: f32, dy: f32) -> i32 {
        if dx >= 0.0 && dy >= 0.0 {
            0
        } else if dx < 0.0 && dy >= 0.0 {
            1
        } else if dx < 0.0 && dy < 0.0 {
            2
        } else {
            3
        }
    }

    // Get or create transformation matrix
    fn get_transform_matrix(
        &mut self,
        width: i32,
        height: i32,
        vertices: &[Point2f; 4],
    ) -> Result<Mat, Box<dyn Error>> {
        let cache_key = (width, height);

        if !self.transform_cache.contains_key(&cache_key) {
            // Create source points for a properly oriented rectangle
            let source_points = [
                Point2f::new(0.0, 0.0),
                Point2f::new(width as f32, 0.0),
                Point2f::new(width as f32, height as f32),
                Point2f::new(0.0, height as f32),
            ];

            let src_points = Mat::from_slice(&source_points)?;
            let dst_points = Mat::from_slice(vertices)?;

            let matrix =
                imgproc::get_perspective_transform(&src_points, &dst_points, core::DECOMP_LU)?;
            self.transform_cache.insert(cache_key, matrix);
        }

        Ok(self.transform_cache.get(&cache_key).unwrap().clone())
    }

    pub fn process_video(&mut self, game_data: &GameData) -> Result<(), Box<dyn Error>> {
        // Pre-load all assets
        self.preload_card_assets(game_data)?;

        let mut frame = Mat::default();
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        let mut frame_count = 0;

        // Use batch processing if possible - read multiple frames at once
        let batch_size = 5;
        let mut frames = Vec::with_capacity(batch_size);

        while frame_count < total_frames {
            // Read a batch of frames
            frames.clear();
            for _ in 0..batch_size {
                if !self.source.read(&mut frame)? {
                    break;
                }
                frames.push(frame.clone());
            }

            if frames.is_empty() {
                break;
            }

            // Process frames
            for mut frame in frames.iter_mut() {
                let placements = self.detect_placeholders(&frame, game_data)?;
                self.process_frame(&mut frame, &placements)?;
                self.output.write(&frame)?;

                frame_count += 1;
                if frame_count % 10 == 0 {
                    println!(
                        "Processing frame {}/{} ({:.1}%)",
                        frame_count,
                        total_frames,
                        (frame_count as f32 / total_frames as f32) * 100.0
                    );
                }
            }
        }
        Ok(())
    }

    pub fn detect_placeholders(
        &self,
        frame: &Mat,
        game_data: &GameData,
    ) -> Result<Vec<CardPlacement>, Box<dyn Error>> {
        // Create mask for pure green color in BGR - reuse as static buffer
        static mut MASK: Option<Mat> = None;
        let mask = unsafe {
            if MASK.is_none() {
                MASK = Some(Mat::default());
            }
            MASK.as_mut().unwrap()
        };

        let threshold = 30.0; // Tolerance for color detection

        let lower_green = core::Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
        let upper_green = core::Scalar::new(threshold, 255.0, threshold, 0.0);

        core::in_range(&frame, &lower_green, &upper_green, mask)?;

        // Find contours
        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            mask,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;

        // Pre-allocate with expected capacity
        let mut placements = Vec::with_capacity(contours.len());

        // Use area threshold to filter contours
        const MIN_AREA: f64 = 100.0;

        for (i, contour) in contours.iter().enumerate() {
            let area = imgproc::contour_area(&contour, false)?;

            if area < MIN_AREA {
                continue;
            }

            let rect = imgproc::bounding_rect(&contour)?;

            // Use modulo to cycle through available card assets
            let asset_index = i % game_data.card_assets.len();

            placements.push(CardPlacement {
                position: rect,
                card_asset_path: game_data.card_assets[asset_index].clone(),
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
        // Pre-load assets
        self.preload_card_assets(game_data)?;

        let mut frame = Mat::default();
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        let mut frame_count = 0;

        // Only call progress callback every N frames for efficiency
        const PROGRESS_UPDATE_INTERVAL: i32 = 10;

        while self.source.read(&mut frame)? {
            let placements = self.detect_placeholders(&frame, game_data)?;
            self.process_frame(&mut frame, &placements)?;
            self.output.write(&frame)?;

            frame_count += 1;
            if frame_count % PROGRESS_UPDATE_INTERVAL == 0 {
                let progress = (frame_count as f32 / total_frames as f32) * 100.0;
                progress_cb(progress)?;
            }
        }
        Ok(())
    }
}
