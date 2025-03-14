#![allow(dead_code)]
#![allow(unused_mut)]
#![allow(unused_variables)]

use opencv::{
    core::{self, Point, Point2f, Rect, Scalar, Size, Vector},
    imgcodecs, imgproc,
    prelude::*,
    videoio,
};
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[allow(dead_code)]
pub struct CardPlacement {
    pub position: Rect,
    pub card_asset_path: String,
    pub contour: Vector<Point>, // Keep track of contour
}

pub struct GameData {
    pub card_assets: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PlaceholderAppearance {
    pub frame_number: i32,
    pub placeholder_count: usize,
    pub new_placeholder_index: Option<usize>, // The index of the new placeholder that appeared
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PlaceholderData {
    video_path: String,
    frame_count: i32,
    pub placeholders: Vec<FramePlaceholders>,
    pub placeholder_appearances: Vec<PlaceholderAppearance>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FramePlaceholders {
    pub frame_number: i32,
    pub positions: Vec<PlaceholderPosition>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PlaceholderPosition {
    pub contour_points: Vec<(i32, i32)>, // Store points as tuples
    pub rect: (i32, i32, i32, i32),      // x, y, width, height
    pub placeholder_id: usize,           // Consistent ID for tracking
    pub first_seen_frame: i32,           // When this placeholder first appeared
}

pub fn create_placements_from_stored(
    positions: &[PlaceholderPosition],
    game_data: &GameData,
) -> Result<Vec<CardPlacement>, Box<dyn Error>> {
    let mut placements = Vec::new();
    let player_cards = &game_data.card_assets;

    // Sort by placeholder_id for consistent assignment
    let mut sorted_positions = positions.to_vec();
    sorted_positions.sort_by_key(|p| p.placeholder_id);

    for (i, position) in sorted_positions.iter().enumerate() {
        if i >= player_cards.len() {
            break;
        }
        let rect = Rect::new(
            position.rect.0,
            position.rect.1,
            position.rect.2,
            position.rect.3,
        );

        let mut contour = Vector::new();
        for &(x, y) in &position.contour_points {
            contour.push(Point::new(x, y));
        }

        placements.push(CardPlacement {
            position: rect,
            card_asset_path: player_cards[i].clone(),
            contour,
        });
    }
    Ok(placements)
}

/// A helper structure to hold the processed overlay for a placement.
/// 'warped' is the processed card asset image and 'mask' is the mask to composite it.
struct RegionResult {
    warped: Mat,
    mask: Mat,
}

pub struct VideoProcessor {
    pub source: videoio::VideoCapture,
    card_assets: HashMap<String, Mat>,
    pub output: videoio::VideoWriter,
    previous_frame: Option<Mat>,
    // Persistent overlays for each placement (keyed by an index corresponding to the
    // order in the placements array)
    persistent_overlays: HashMap<usize, RegionResult>,
}

impl VideoProcessor {
    pub fn new(input_path: &str, output_path: &str) -> Result<Self, Box<dyn Error>> {
        let source = videoio::VideoCapture::from_file(input_path, videoio::CAP_ANY)?;

        // Get video properties
        let fps = source.get(videoio::CAP_PROP_FPS)?;
        let width = source.get(videoio::CAP_PROP_FRAME_WIDTH)? as i32;
        let height = source.get(videoio::CAP_PROP_FRAME_HEIGHT)? as i32;

        // Create VideoWriter using H.264 codec
        let fourcc = videoio::VideoWriter::fourcc('a', 'v', 'c', '1')?;
        let mut output =
            videoio::VideoWriter::new(output_path, fourcc, fps, Size::new(width, height), true)?;
        // Set higher bitrate quality (adjust as needed)
        output.set(videoio::VIDEOWRITER_PROP_QUALITY, 320.0)?;

        Ok(VideoProcessor {
            source,
            card_assets: HashMap::new(),
            output,
            previous_frame: None,
            persistent_overlays: HashMap::new(),
        })
    }

    // scan_and_save_placeholders, load_placeholders, get_card_asset_path, reset_frame_count
    // (remain unchanged; see your original code below)
    pub fn scan_and_save_placeholders(&mut self, output_path: &str) -> Result<(), Box<dyn Error>> {
        let mut placeholder_data = PlaceholderData {
            video_path: self.source.get_backend_name()?.to_string(),
            frame_count: self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32,
            placeholders: Vec::new(),
            placeholder_appearances: Vec::new(),
        };

        let mut frame = Mat::default();
        let mut frame_number = 0;
        let mut tracked_placeholders: HashMap<usize, (i32, i32)> = HashMap::new();
        let mut next_id = 0;

        while self.source.read(&mut frame)? {
            let mut frame_positions: Vec<PlaceholderPosition> = Vec::new();
            let mut frame_new_ids: Vec<usize> = Vec::new();

            let mut mask = Mat::default();
            let threshold = 30.0;
            let lower_green = Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
            let upper_green = Scalar::new(threshold, 255.0, threshold, 0.0);
            core::in_range(&frame, &lower_green, &upper_green, &mut mask)?;
            let mut contours = Vector::<Vector<Point>>::new();
            imgproc::find_contours(
                &mask,
                &mut contours,
                imgproc::RETR_EXTERNAL,
                imgproc::CHAIN_APPROX_SIMPLE,
                Point::new(0, 0),
            )?;
            for contour in contours.iter() {
                let area = imgproc::contour_area(&contour, false)?;
                if area < 100.0 {
                    continue;
                }
                let rect = imgproc::bounding_rect(&contour)?;
                let contour_points: Vec<(i32, i32)> = contour.iter().map(|p| (p.x, p.y)).collect();

                let centroid_x = rect.x + rect.width / 2;
                let centroid_y = rect.y + rect.height / 2;

                let mut matched_id = None;
                const DISTANCE_THRESHOLD: f64 = 50.0;
                for (&id, &(prev_x, prev_y)) in &tracked_placeholders {
                    let dx = (centroid_x - prev_x) as f64;
                    let dy = (centroid_y - prev_y) as f64;
                    if (dx * dx + dy * dy).sqrt() < DISTANCE_THRESHOLD {
                        matched_id = Some(id);
                        break;
                    }
                }

                let placeholder_id = if let Some(id) = matched_id {
                    tracked_placeholders.insert(id, (centroid_x, centroid_y));
                    id
                } else {
                    let new_id = next_id;
                    next_id += 1;
                    tracked_placeholders.insert(new_id, (centroid_x, centroid_y));
                    placeholder_data
                        .placeholder_appearances
                        .push(PlaceholderAppearance {
                            frame_number,
                            placeholder_count: tracked_placeholders.len(),
                            new_placeholder_index: Some(new_id),
                        });
                    frame_new_ids.push(new_id);
                    new_id
                };

                frame_positions.push(PlaceholderPosition {
                    contour_points,
                    rect: (rect.x, rect.y, rect.width, rect.height),
                    placeholder_id,
                    first_seen_frame: frame_number,
                });
            }

            if !frame_positions.is_empty() {
                frame_positions.sort_by_key(|p| p.placeholder_id);
                placeholder_data.placeholders.push(FramePlaceholders {
                    frame_number,
                    positions: frame_positions,
                });
            }
            frame_number += 1;
        }
        let file = std::fs::File::create(output_path)?;
        serde_json::to_writer(file, &placeholder_data)?;
        self.reset_frame_count()?;
        Ok(())
    }

    pub fn load_placeholders(&self, path: &str) -> Result<PlaceholderData, Box<dyn Error>> {
        let file = std::fs::File::open(path)?;
        let data: PlaceholderData = serde_json::from_reader(file)?;
        Ok(data)
    }

    pub fn get_card_asset_path(&self, card: &str) -> PathBuf {
        let (suit, rank) = card.split_at(1);
        let suit_name = match suit {
            "H" => "hearts",
            "D" => "diamond",
            "C" => "clubs",
            "S" => "spades",
            _ => "unknown",
        };
        PathBuf::from(format!(
            "assets/cards/{}_{}.jpg",
            suit_name,
            rank.to_lowercase()
        ))
    }

    pub fn reset_frame_count(&mut self) -> Result<(), Box<dyn Error>> {
        self.source.set(videoio::CAP_PROP_POS_FRAMES, 0.0)?;
        Ok(())
    }

    /// Helper function (as a method) that processes one card placement and returns a RegionResult.
    fn process_placement(
        &mut self,
        placement: &CardPlacement,
        frame_size: Size,
    ) -> Result<RegionResult, Box<dyn Error>> {
        // Get card asset – load and cache it in self.card_assets.
        let card_asset = if let Some(asset) = self.card_assets.get(&placement.card_asset_path) {
            asset.clone()
        } else {
            let asset = imgcodecs::imread(&placement.card_asset_path, imgcodecs::IMREAD_UNCHANGED)?;
            self.card_assets
                .insert(placement.card_asset_path.clone(), asset.clone());
            asset
        };

        // Get minimum area rectangle for the placement contour.
        let rot_rect = imgproc::min_area_rect(&placement.contour)?;
        let mut vertices = [Point2f::default(); 4];
        rot_rect.points(&mut vertices)?;

        // Sort vertices (by angle relative to center)
        let center = vertices.iter().fold(Point2f::new(0.0, 0.0), |acc, &p| {
            Point2f::new(acc.x + p.x / 4.0, acc.y + p.y / 4.0)
        });
        vertices.sort_by(|a, b| {
            let a_angle = (a.y - center.y).atan2(a.x - center.x);
            let b_angle = (b.y - center.y).atan2(b.x - center.x);
            b_angle
                .partial_cmp(&a_angle)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let width = rot_rect.size.width as i32;
        let height = rot_rect.size.height as i32;
        let source_points = [
            Point2f::new(0.0, 0.0),
            Point2f::new(width as f32, 0.0),
            Point2f::new(width as f32, height as f32),
            Point2f::new(0.0, height as f32),
        ];
        let src_points = Mat::from_slice(&source_points)?;
        let dst_points = Mat::from_slice(&vertices)?;

        let mut resized_asset = Mat::default();
        imgproc::resize(
            &card_asset,
            &mut resized_asset,
            Size::new(width, height),
            0.0,
            0.0,
            imgproc::INTER_CUBIC,
        )?;

        let transform_matrix =
            imgproc::get_perspective_transform(&src_points, &dst_points, core::DECOMP_LU)?;
        let mut warped = Mat::default();
        imgproc::warp_perspective(
            &resized_asset,
            &mut warped,
            &transform_matrix,
            frame_size,
            imgproc::INTER_CUBIC,
            core::BORDER_CONSTANT,
            Scalar::default(),
        )?;

        // Create mask from the contour.
        let mut mask = Mat::zeros(frame_size.height, frame_size.width, core::CV_8UC1)?.to_mat()?;
        let mut contours = Vector::<Vector<Point>>::new();
        contours.push(placement.contour.clone());
        imgproc::draw_contours(
            &mut mask,
            &contours,
            0,
            Scalar::new(255.0, 0.0, 0.0, 0.0),
            -1,
            imgproc::LINE_8,
            &Mat::default(),
            0,
            Point::new(0, 0),
        )?;
        Ok(RegionResult { warped, mask })
    }

    /// The new process_frame uses frame differencing only to update the overlay.
    /// It then composites the (persistent) overlay for each placement on every frame.
    pub fn process_frame(
        &mut self,
        frame: &mut Mat,
        placements: &[CardPlacement],
    ) -> Result<(), Box<dyn Error>> {
        if placements.is_empty() {
            return Ok(());
        }

        let current_size = frame.size()?;
        // If no previous frame, process every placement (initial frame)
        if self.previous_frame.is_none() {
            for (idx, placement) in placements.iter().enumerate() {
                let region = self.process_placement(placement, current_size)?;
                self.persistent_overlays.insert(idx, region);
            }
            // Save a copy for diffing in later frames.
            let mut frame_copy = Mat::default();
            frame.copy_to(&mut frame_copy)?;
            self.previous_frame = Some(frame_copy);
        } else {
            // For each placement, check if there is significant change in its ROI.
            for (idx, placement) in placements.iter().enumerate() {
                let expanded_rect = expand_rect(&placement.position, 10, current_size);
                let roi_current = Mat::roi(frame, expanded_rect)?;
                let roi_previous = Mat::roi(self.previous_frame.as_ref().unwrap(), expanded_rect)?;
                let mut diff = Mat::default();
                core::absdiff(&roi_current, &roi_previous, &mut diff)?;
                let mean = core::mean(&diff, &Mat::default())?;
                // If diff mean is above threshold in any channel, reprocess this placement.
                if mean[0] > 2.0 || mean[1] > 2.0 || mean[2] > 2.0 {
                    let region = self.process_placement(placement, current_size)?;
                    self.persistent_overlays.insert(idx, region);
                }
            }
            // Update previous frame after processing.
            let mut frame_copy = Mat::default();
            frame.copy_to(&mut frame_copy)?;
            self.previous_frame = Some(frame_copy);
        }
        // Composite all persistent overlays onto the current frame.
        for (idx, _placement) in placements.iter().enumerate() {
            if let Some(region) = self.persistent_overlays.get(&idx) {
                let _ = region.warped.copy_to_masked(frame, &region.mask);
            }
        }
        Ok(())
    }

    // The remainder of your methods remains unchanged (process_frame_for_stream, process_video, etc.)
    pub fn process_frame_for_stream(
        &mut self,
        frame: &mut Mat,
        game_data: &GameData,
    ) -> Result<(), Box<dyn Error>> {
        let frame_height = frame.rows();
        let frame_width = frame.cols();
        let roi_y = frame_height / 3;
        let roi = Rect::new(0, roi_y, frame_width, frame_height / 3);
        let roi_frame = Mat::roi(frame, roi)?;

        let mut mask = Mat::default();
        let threshold = 30.0;
        let lower_green = Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
        let upper_green = Scalar::new(threshold, 255.0, threshold, 0.0);
        core::in_range(&roi_frame, &lower_green, &upper_green, &mut mask)?;

        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &mask,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, roi_y),
        )?;

        let mut placements = Vec::new();
        let player_cards = &game_data.card_assets;
        let mut valid_contours: Vec<(Vector<Point>, Rect)> = contours
            .iter()
            .filter_map(|contour| {
                let area = imgproc::contour_area(&contour, false).ok()?;
                if area < 100.0 {
                    return None;
                }
                let rect = imgproc::bounding_rect(&contour).ok()?;
                Some((contour.clone(), rect))
            })
            .collect();
        valid_contours.sort_by(|a, b| a.1.x.cmp(&b.1.x));
        let start = if game_data.card_assets.len() > 2 {
            2
        } else {
            0
        };
        for (i, (contour, rect)) in valid_contours.iter().enumerate() {
            if i < player_cards.len() - start {
                placements.push(CardPlacement {
                    position: *rect,
                    card_asset_path: player_cards[i + start].clone(),
                    contour: contour.clone(),
                });
            }
        }
        if !placements.is_empty() {
            self.process_frame(frame, &placements)?;
        }
        Ok(())
    }

    pub fn process_video(&mut self, game_data: &GameData) -> Result<(), Box<dyn Error>> {
        let mut frame = Mat::default();
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        let mut frame_count = 0;
        while self.source.read(&mut frame)? {
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
        Ok(())
    }

    pub fn detect_placeholders(
        &self,
        frame: &Mat,
        game_data: &GameData,
    ) -> Result<Vec<CardPlacement>, Box<dyn Error>> {
        let mut mask = Mat::default();
        let threshold = 30.0;
        let lower_green = Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
        let upper_green = Scalar::new(threshold, 255.0, threshold, 0.0);
        core::in_range(&frame, &lower_green, &upper_green, &mut mask)?;
        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &mask,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;
        let mut placements = Vec::new();
        let mut valid_contours: Vec<(Vector<Point>, Rect)> = contours
            .iter()
            .filter_map(|contour| {
                let area = imgproc::contour_area(&contour, false).ok()?;
                if area < 100.0 {
                    return None;
                }
                let rect = imgproc::bounding_rect(&contour).ok()?;
                Some((contour.clone(), rect))
            })
            .collect();
        valid_contours.sort_by(|a, b| a.1.x.cmp(&b.1.x));
        let player_cards = if game_data.card_assets.len() > 2 {
            &game_data.card_assets[2..]
        } else {
            &game_data.card_assets
        };
        for (i, (contour, rect)) in valid_contours.iter().enumerate() {
            if i < player_cards.len() {
                placements.push(CardPlacement {
                    position: *rect,
                    card_asset_path: player_cards[i].clone(),
                    contour: contour.clone(),
                });
            }
        }
        Ok(placements)
    }

    pub fn get_frame_number(&self) -> Result<i32, Box<dyn Error>> {
        Ok(self.source.get(videoio::CAP_PROP_POS_FRAMES)? as i32)
    }

    pub fn get_total_frames(&self) -> Result<i32, Box<dyn Error>> {
        Ok(self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32)
    }
}

/// Helper to expand a rectangle with padding without exceeding the image bounds.
fn expand_rect(rect: &Rect, padding: i32, image_size: Size) -> Rect {
    let new_x = (rect.x - padding).max(0);
    let new_y = (rect.y - padding).max(0);
    let new_width = (rect.width + 2 * padding).min(image_size.width - new_x);
    let new_height = (rect.height + 2 * padding).min(image_size.height - new_y);
    Rect::new(new_x, new_y, new_width, new_height)
}

#[allow(dead_code)]
fn _preprocess_videos() -> Result<(), Box<dyn Error>> {
    let video_dir = "assets/videos";
    for entry in std::fs::read_dir(video_dir)? {
        let entry = entry?;
        if entry.path().extension() == Some(std::ffi::OsStr::new("mp4")) {
            let video_path = entry.path();
            let placeholder_path = video_path.with_extension("json");
            println!("Processing {}", video_path.display());
            let mut processor =
                VideoProcessor::new(video_path.to_str().unwrap(), "dummy_output.mp4")?;
            processor.scan_and_save_placeholders(placeholder_path.to_str().unwrap())?;
        }
    }
    Ok(())
}
