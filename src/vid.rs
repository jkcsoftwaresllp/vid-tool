#![allow(dead_code)]
#![allow(unused_mut)]
#![allow(unused_variables)]

use opencv::{
    core::{self, Point, Point2f, Vector},
    imgcodecs, imgproc,
    prelude::*,
    videoio,
};
use std::error::Error;
use std::{collections::HashMap, path::PathBuf};

#[allow(dead_code)]
pub struct CardPlacement {
    pub position: core::Rect,
    pub card_asset_path: String,
    contour: Vector<Point>, // Add this field
}

pub struct GameData {
    pub card_assets: Vec<String>,
}

use serde::{Deserialize, Serialize};

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
    contour_points: Vec<(i32, i32)>, // Store points as tuples
    rect: (i32, i32, i32, i32),      // x, y, width, height
    pub placeholder_id: usize,       // Add this field for consistent tracking
    first_seen_frame: i32,           // Frame when this placeholder first appeared
}

pub struct VideoProcessor {
    pub source: videoio::VideoCapture,
    card_assets: HashMap<String, Mat>,
    pub output: videoio::VideoWriter,
    previous_frame: Option<Mat>,
    //thread_pool: rayon::ThreadPool,
}

// Modify create_placements_from_stored to use placeholder_id for consistent ordering
pub fn create_placements_from_stored(
    positions: &[PlaceholderPosition],
    game_data: &GameData,
) -> Result<Vec<CardPlacement>, Box<dyn Error>> {
    let mut placements = Vec::new();
    let player_cards = &game_data.card_assets;

    // Sort by placeholder_id (order of first appearance)
    // This is critical for a consistent assignment of cards to placeholders
    let mut sorted_positions = positions.to_vec();
    sorted_positions.sort_by_key(|p| p.placeholder_id);

    for (i, position) in sorted_positions.iter().enumerate() {
        if i >= player_cards.len() {
            break;
        }

        let rect = core::Rect::new(
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

impl VideoProcessor {
    pub fn new(input_path: &str, output_path: &str) -> Result<Self, Box<dyn Error>> {
        let source = videoio::VideoCapture::from_file(input_path, videoio::CAP_ANY)?;

        // Get video properties
        let fps = source.get(videoio::CAP_PROP_FPS)?;
        let width = source.get(videoio::CAP_PROP_FRAME_WIDTH)? as i32;
        let height = source.get(videoio::CAP_PROP_FRAME_HEIGHT)? as i32;

        // Create VideoWriter
        let fourcc = videoio::VideoWriter::fourcc('a', 'v', 'c', '1')?; // Using H.264 codec
        let mut output = videoio::VideoWriter::new(
            output_path,
            fourcc,
            fps,
            core::Size::new(width, height),
            true,
        )?;

        // Set higher bitrate
        output.set(videoio::VIDEOWRITER_PROP_QUALITY, 320.0)?;

        // Create a thread pool with number of cores available
        // let thread_pool = rayon::ThreadPoolBuilder::new()
        //     .num_threads(num_cpus::get())
        //     .build()
        //     .unwrap();

        // Initialize card assets HashMap
        let card_assets = HashMap::new();

        Ok(VideoProcessor {
            source,
            card_assets,
            output,
            previous_frame: None,
            //thread_pool,
        })
    }

    // Enhance scan_and_save_placeholders to track placeholders with persistent IDs
    pub fn scan_and_save_placeholders(&mut self, output_path: &str) -> Result<(), Box<dyn Error>> {
        let mut placeholder_data = PlaceholderData {
            video_path: self.source.get_backend_name()?.to_string(),
            frame_count: self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32,
            placeholders: Vec::new(),
            placeholder_appearances: Vec::new(),
        };

        let mut frame = Mat::default();
        let mut frame_number = 0;

        // Tracking structure for placeholders
        // Key: placeholder_id, Value: (last_seen_centroid_x, last_seen_centroid_y)
        // Tracking for persistent placeholders
        let mut tracked_placeholders: HashMap<usize, (i32, i32)> = HashMap::new();
        let mut next_id = 0; // next available placeholder_id

        while self.source.read(&mut frame)? {
            // Do not reinitialize placeholder_data here!
            // Instead, for each frame, build a list of detected placeholders.
            let mut frame_positions: Vec<PlaceholderPosition> = Vec::new();
            let mut frame_new_ids: Vec<usize> = Vec::new();

            // Detect green placeholders
            let mut mask = Mat::default();
            let threshold = 30.0;

            let lower_green = core::Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
            let upper_green = core::Scalar::new(threshold, 255.0, threshold, 0.0);

            core::in_range(&frame, &lower_green, &upper_green, &mut mask)?;

            let mut contours = Vector::<Vector<Point>>::new();
            imgproc::find_contours(
                &mask,
                &mut contours,
                imgproc::RETR_EXTERNAL,
                imgproc::CHAIN_APPROX_SIMPLE,
                Point::new(0, 0),
            )?;

            // Store new placeholder detections for this frame
            let mut frame_positions = Vec::new();
            let mut frame_new_ids = Vec::new();

            // Process each detected contour
            for contour in contours.iter() {
                let area = imgproc::contour_area(&contour, false)?;
                if area < 100.0 {
                    continue;
                }
                let rect = imgproc::bounding_rect(&contour)?;
                let contour_points: Vec<(i32, i32)> = contour.iter().map(|p| (p.x, p.y)).collect();

                // Determine centroid
                let centroid_x = rect.x + rect.width / 2;
                let centroid_y = rect.y + rect.height / 2;

                // Try matching with tracked placeholders by distance
                let mut matched_id = None;
                const DISTANCE_THRESHOLD: f64 = 50.0; // adjust if needed

                for (&id, &(prev_x, prev_y)) in &tracked_placeholders {
                    let dx = (centroid_x - prev_x) as f64;
                    let dy = (centroid_y - prev_y) as f64;
                    if (dx * dx + dy * dy).sqrt() < DISTANCE_THRESHOLD {
                        matched_id = Some(id);
                        break;
                    }
                }

                // If no match, assign a new ID
                let placeholder_id = if let Some(id) = matched_id {
                    tracked_placeholders.insert(id, (centroid_x, centroid_y));
                    id
                } else {
                    let new_id = next_id;
                    next_id += 1;
                    tracked_placeholders.insert(new_id, (centroid_x, centroid_y));
                    // Record this new placeholder appearance only once
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

                // Create the PlaceholderPosition with persistent id
                frame_positions.push(PlaceholderPosition {
                    contour_points,
                    rect: (rect.x, rect.y, rect.width, rect.height),
                    placeholder_id,
                    first_seen_frame: frame_number,
                });
            }

            if !frame_positions.is_empty() {
                // Sort positions by placeholder_id
                frame_positions.sort_by_key(|p| p.placeholder_id);

                placeholder_data.placeholders.push(FramePlaceholders {
                    frame_number,
                    positions: frame_positions,
                });
            }
            frame_number += 1;
        }

        // Save the complete placeholder_data to file after processing all frames.
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

    // Improved process_frame method with differencing and parallel processing
    pub fn process_frame(
        &mut self,
        frame: &mut Mat,
        placements: &[CardPlacement],
    ) -> Result<(), Box<dyn Error>> {
        // If empty placements, nothing to do
        if placements.is_empty() {
            return Ok(());
        }

        // Check for significant changes in the regions of interest
        let mut regions_to_process = Vec::new();

        if let Some(prev_frame) = &self.previous_frame {
            for (idx, placement) in placements.iter().enumerate() {
                // Expand the rect slightly to ensure we capture all relevant changes
                let expanded_rect = expand_rect(&placement.position, 10, frame.size()?);

                // Extract ROI from current and previous frame
                let roi_current = Mat::roi(frame, expanded_rect)?;
                let roi_previous = Mat::roi(prev_frame, expanded_rect)?;

                // Calculate difference for this region
                let mut diff = Mat::default();
                core::absdiff(&roi_current, &roi_previous, &mut diff)?;

                // Calculate mean change in the region
                let mean = core::mean(&diff, &Mat::default())?;

                // If change is significant, add to regions to process
                if mean[0] > 2.0 || mean[1] > 2.0 || mean[2] > 2.0 {
                    regions_to_process.push(idx);
                }
            }
        } else {
            // For the first frame, process all placements
            regions_to_process = (0..placements.len()).collect();
        }

        // If no regions need processing, return early
        if regions_to_process.is_empty() {
            return Ok(());
        }

        // Store frame for future differencing
        let mut frame_copy = Mat::default();
        frame.copy_to(&mut frame_copy)?;
        self.previous_frame = Some(frame_copy);

        // Use parallel processing only if we have multiple regions to process
        if regions_to_process.len() > 1 {
            // Instead of using thread_pool.install, we'll use rayon's par_iter directly
            use rayon::prelude::*;

            // Prepare shared data that needs to be accessed across threads
            let frame_width = frame.cols();
            let frame_height = frame.rows();
            let frame_size = core::Size::new(frame_width, frame_height);

            // Create a structure to hold results from parallel processing
            struct RegionResult {
                warped: Mat,
                mask: Mat,
            }

            // Process in parallel, but handle errors within the parallel section
            let mut region_results: Vec<Option<RegionResult>> = regions_to_process
                .par_iter()
                .map(|&idx| {
                    let placement = &placements[idx];

                    // Load card asset (this part can't be parallelized easily with shared access)
                    // So we'll just reload the asset each time
                    let card_asset_result =
                        imgcodecs::imread(&placement.card_asset_path, imgcodecs::IMREAD_UNCHANGED);

                    let card_asset = match card_asset_result {
                        Ok(asset) => asset,
                        Err(_) => return None, // Skip this region if asset loading fails
                    };

                    // Process the placement (similar to existing code)
                    let rot_rect_result = imgproc::min_area_rect(&placement.contour);
                    if rot_rect_result.is_err() {
                        return None;
                    }
                    let rot_rect = rot_rect_result.unwrap();

                    let mut vertices = [core::Point2f::default(); 4];
                    if rot_rect.points(&mut vertices).is_err() {
                        return None;
                    }

                    // Sort vertices (simplified error handling)
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

                    // Calculate width and height
                    let width = rot_rect.size.width as i32;
                    let height = rot_rect.size.height as i32;

                    // Create source points
                    let source_points = [
                        Point2f::new(0.0, 0.0),
                        Point2f::new(width as f32, 0.0),
                        Point2f::new(width as f32, height as f32),
                        Point2f::new(0.0, height as f32),
                    ];

                    // Create transformation matrices
                    let src_points_result = Mat::from_slice(&source_points);
                    let dst_points_result = Mat::from_slice(&vertices);

                    if src_points_result.is_err() || dst_points_result.is_err() {
                        return None;
                    }

                    let src_points = src_points_result.unwrap();
                    let dst_points = dst_points_result.unwrap();

                    // Resize card asset
                    let mut resized_asset = Mat::default();
                    if imgproc::resize(
                        &card_asset,
                        &mut resized_asset,
                        core::Size::new(width, height),
                        0.0,
                        0.0,
                        imgproc::INTER_CUBIC,
                    )
                    .is_err()
                    {
                        return None;
                    }

                    // Get perspective transform and apply it
                    let transform_matrix_result = imgproc::get_perspective_transform(
                        &src_points,
                        &dst_points,
                        core::DECOMP_LU,
                    );

                    if transform_matrix_result.is_err() {
                        return None;
                    }

                    let transform_matrix = transform_matrix_result.unwrap();
                    let mut warped = Mat::default();

                    if imgproc::warp_perspective(
                        &resized_asset,
                        &mut warped,
                        &transform_matrix,
                        frame_size,
                        imgproc::INTER_CUBIC,
                        core::BORDER_CONSTANT,
                        core::Scalar::default(),
                    )
                    .is_err()
                    {
                        return None;
                    }

                    // Create mask from contour
                    let mask_result = Mat::zeros(frame_height, frame_width, core::CV_8UC1);
                    if mask_result.is_err() {
                        return None;
                    }

                    let mut mask = mask_result.unwrap().to_mat().unwrap_or(Mat::default());
                    let mut contours = Vector::<Vector<Point>>::new();
                    contours.push(placement.contour.clone());

                    if imgproc::draw_contours(
                        &mut mask,
                        &contours,
                        0,
                        core::Scalar::new(255.0, 0.0, 0.0, 0.0),
                        -1,
                        imgproc::LINE_8,
                        &Mat::default(),
                        0,
                        Point::new(0, 0),
                    )
                    .is_err()
                    {
                        return None;
                    }

                    Some(RegionResult { warped, mask })
                })
                .collect();

            // Apply all successful results to the frame
            for result in region_results.iter_mut() {
                if let Some(region) = result.take() {
                    // If copy_to_masked fails, just continue with the next region
                    let _ = region.warped.copy_to_masked(frame, &region.mask);
                }
            }
        } else {
            // For single region, process sequentially (existing code)
            for &idx in &regions_to_process {
                let placement = &placements[idx];

                let card_asset = self
                    .card_assets
                    .entry(placement.card_asset_path.clone())
                    .or_insert_with(|| {
                        imgcodecs::imread(&placement.card_asset_path, imgcodecs::IMREAD_UNCHANGED)
                            .expect("Failed to load card asset")
                    });

                // Existing processing code...
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

                // Process as in your existing code...
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
                    card_asset,
                    &mut resized_asset,
                    core::Size::new(width, height),
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
                    frame.size()?,
                    imgproc::INTER_CUBIC,
                    core::BORDER_CONSTANT,
                    core::Scalar::default(),
                )?;

                let mut mask =
                    Mat::zeros(frame.size()?.height, frame.size()?.width, core::CV_8UC1)?
                        .to_mat()?;
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

                warped.copy_to_masked(frame, &mask)?;
            }
        }

        Ok(())
    }

    pub fn process_frame_for_stream(
        &mut self,
        frame: &mut Mat,
        game_data: &GameData,
    ) -> Result<(), Box<dyn Error>> {
        // ROI (Region of Interest) based detection
        let frame_height = frame.rows();
        let frame_width = frame.cols();

        // Assume placeholders are in the middle third of the frame
        let roi_y = frame_height / 3;
        let roi_height = frame_height / 3;
        let roi = core::Rect::new(0, roi_y, frame_width, roi_height);

        let roi_frame = Mat::roi(frame, roi)?;

        // Quick green detection in ROI
        let mut mask = Mat::default();
        let threshold = 30.0;

        let lower_green = core::Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
        let upper_green = core::Scalar::new(threshold, 255.0, threshold, 0.0);

        core::in_range(&roi_frame, &lower_green, &upper_green, &mut mask)?;

        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &mask,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, roi_y), // Adjust points for ROI offset
        )?;

        let mut placements = Vec::new();
        let player_cards = &game_data.card_assets;

        // Process only the largest contours matching the number of expected cards
        let mut valid_contours: Vec<(Vector<Point>, f64)> = contours
            .iter()
            .filter_map(|contour| {
                let area = imgproc::contour_area(&contour, false).ok()?;
                if area < 100.0 {
                    return None;
                }
                Some((contour.clone(), area))
            })
            .collect();

        // Sort by area to get the most prominent placeholders
        valid_contours.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        valid_contours.truncate(player_cards.len());

        // Sort left to right for consistent card placement
        let mut position_contours: Vec<(Vector<Point>, core::Rect)> = valid_contours
            .iter()
            .filter_map(|(contour, _)| {
                let rect = imgproc::bounding_rect(&contour).ok()?;
                Some((contour.clone(), rect))
            })
            .collect();
        position_contours.sort_by(|a, b| a.1.x.cmp(&b.1.x));

        // Create placements
        for (i, (contour, rect)) in position_contours.iter().enumerate() {
            if i < player_cards.len() {
                placements.push(CardPlacement {
                    position: *rect,
                    card_asset_path: player_cards[i].clone(),
                    contour: contour.clone(),
                });
            }
        }

        // Process frame with placements
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
        let mut mask = Mat::default();
        let threshold = 30.0;

        let lower_green = core::Scalar::new(0.0, 255.0 - threshold, 0.0, 0.0);
        let upper_green = core::Scalar::new(threshold, 255.0, threshold, 0.0);

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
        let mut valid_contours: Vec<(Vector<Point>, core::Rect)> = contours
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

        // Sort contours by x-coordinate (left to right)
        valid_contours.sort_by(|a, b| a.1.x.cmp(&b.1.x));

        // Skip blind card and joker card indices (they're at the beginning of card_assets)
        let player_cards = if game_data.card_assets.len() > 2 {
            &game_data.card_assets[2..]
        } else {
            &game_data.card_assets
        };

        // Match contours with player cards
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

    pub fn get_frame_number(&self) -> Result<i32, Box<dyn std::error::Error>> {
        let frame_number = self.source.get(videoio::CAP_PROP_POS_FRAMES)? as i32;
        Ok(frame_number)
    }

    pub fn get_total_frames(&self) -> Result<i32, Box<dyn std::error::Error>> {
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        Ok(total_frames)
    }
}

// Helper function to expand a rectangle while keeping it within image bounds
fn expand_rect(rect: &core::Rect, padding: i32, image_size: core::Size) -> core::Rect {
    let new_x = (rect.x - padding).max(0);
    let new_y = (rect.y - padding).max(0);
    let new_width = (rect.width + 2 * padding).min(image_size.width - new_x);
    let new_height = (rect.height + 2 * padding).min(image_size.height - new_y);

    core::Rect::new(new_x, new_y, new_width, new_height)
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
