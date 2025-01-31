use rayon::prelude::*;

use lazy_static::lazy_static;
use std::sync::{Arc, Mutex};

lazy_static! {
    static ref CARD_ASSET_CACHE: Mutex<HashMap<String, Arc<Mat>>> = Mutex::new(HashMap::new());
}

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
    pub output: videoio::VideoWriter,
}

fn calculate_card_dimensions(card_asset: &Mat, target_width: i32) -> (i32, i32) {
    let asset_aspect = card_asset.cols() as f32 / card_asset.rows() as f32;
    let height = (target_width as f32 / asset_aspect) as i32;
    (target_width, height)
}

impl VideoProcessor {
    pub fn new(input_path: &str, output_path: &str) -> Result<Self, Box<dyn Error>> {
        let mut source = videoio::VideoCapture::from_file(input_path, videoio::CAP_ANY)?;

        source.set(
            videoio::CAP_PROP_HW_ACCELERATION,
            videoio::VIDEO_ACCELERATION_ANY as f64,
        )?;

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

        // Initialize card assets HashMap
        let card_assets = HashMap::new();

        Ok(VideoProcessor {
            source,
            card_assets,
            output,
        })
    }

    fn get_card_asset(&self, path: &str) -> Result<Arc<Mat>, Box<dyn Error>> {
        let mut cache = CARD_ASSET_CACHE.lock().unwrap();
        if let Some(asset) = cache.get(path) {
            Ok(asset.clone())
        } else {
            let asset = Arc::new(imgcodecs::imread(path, imgcodecs::IMREAD_UNCHANGED)?);
            cache.insert(path.to_string(), asset.clone());
            Ok(asset)
        }
    }

    fn process_frames_batch(
        &mut self,
        frames: Vec<Mat>,
        game_data: &GameData,
    ) -> Result<Vec<Mat>, Box<dyn Error>> {
        // Process frames sequentially first, then we'll optimize
        let mut processed_frames = Vec::with_capacity(frames.len());

        // Process each frame in parallel using chunks
        let results: Vec<Result<Mat, Box<dyn Error>>> = frames
            .into_iter()
            .map(|mut frame| {
                let placements = self.detect_placeholders(&frame, game_data)?;
                self.process_frame(&mut frame, &placements)?;
                Ok(frame)
            })
            .collect();

        // Collect results, propagating any errors
        for result in results {
            processed_frames.push(result?);
        }

        Ok(processed_frames)
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

            // Smooth out the contour with more points for better angle detection
            let mut smoothed_contour = Vector::new();
            imgproc::approx_poly_dp(&placement.contour, &mut smoothed_contour, 1.0, true)?;

            // Get rotated rectangle
            let rot_rect = imgproc::min_area_rect(&smoothed_contour)?;

            // Calculate proper dimensions maintaining aspect ratio
            let target_width = rot_rect.size.width as i32;
            let (card_width, card_height) = calculate_card_dimensions(card_asset, target_width);

            // Calculate angle more accurately
            let mut angle = rot_rect.angle as f64;

            // Adjust angle based on rectangle orientation
            if rot_rect.size.width < rot_rect.size.height {
                angle += 90.0;
            }

            // Normalize angle to be between -90 and 90 degrees
            while angle > 90.0 {
                angle -= 180.0;
            }
            while angle < -90.0 {
                angle += 180.0;
            }

            // Create rotation matrix
            let rot_mat = imgproc::get_rotation_matrix_2d(
                core::Point2f::new(
                    card_asset.cols() as f32 / 2.0,
                    card_asset.rows() as f32 / 2.0,
                ),
                angle,
                1.0,
            )?;

            // Apply rotation
            let mut rotated_card = Mat::default();
            imgproc::warp_affine(
                card_asset,
                &mut rotated_card,
                &rot_mat,
                card_asset.size()?,
                imgproc::INTER_LANCZOS4,
                core::BORDER_CONSTANT,
                core::Scalar::default(),
            )?;

            // Resize maintaining aspect ratio
            let mut resized_card = Mat::default();
            imgproc::resize(
                &rotated_card,
                &mut resized_card,
                core::Size::new(card_width, card_height),
                0.0,
                0.0,
                imgproc::INTER_LANCZOS4,
            )?;

            // Get the corners of the rotated rectangle
            let mut vertices = [core::Point2f::default(); 4];
            rot_rect.points(&mut vertices)?;

            // Calculate center point for better placement
            let center = vertices.iter().fold(Point2f::new(0.0, 0.0), |acc, &p| {
                Point2f::new(acc.x + p.x / 4.0, acc.y + p.y / 4.0)
            });

            // Create source points considering the card's aspect ratio
            let source_points = [
                Point2f::new(0.0, 0.0),
                Point2f::new(card_width as f32, 0.0),
                Point2f::new(card_width as f32, card_height as f32),
                Point2f::new(0.0, card_height as f32),
            ];

            // Create destination points centered on the placeholder
            let half_width = card_width as f32 / 2.0;
            let half_height = card_height as f32 / 2.0;
            let dest_points = [
                Point2f::new(center.x - half_width, center.y - half_height),
                Point2f::new(center.x + half_width, center.y - half_height),
                Point2f::new(center.x + half_width, center.y + half_height),
                Point2f::new(center.x - half_width, center.y + half_height),
            ];

            let src_points = Mat::from_slice(&source_points)?;
            let dst_points = Mat::from_slice(&dest_points)?;

            // Apply perspective transform
            let transform_matrix =
                imgproc::get_perspective_transform(&src_points, &dst_points, core::DECOMP_LU)?;
            let mut warped = Mat::default();
            imgproc::warp_perspective(
                &resized_card,
                &mut warped,
                &transform_matrix,
                frame.size()?,
                imgproc::INTER_LANCZOS4,
                core::BORDER_CONSTANT,
                core::Scalar::default(),
            )?;

            // Create mask with smoother edges
            let mut mask =
                Mat::zeros(frame.size()?.height, frame.size()?.width, core::CV_8UC1)?.to_mat()?;
            let mut contours = Vector::<Vector<Point>>::new();
            contours.push(smoothed_contour);

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

            // Apply stronger edge smoothing
            let mut final_mask = Mat::default();
            imgproc::gaussian_blur(
                &mask,
                &mut final_mask,
                core::Size::new(5, 5), // Increased blur size
                0.0,
                0.0,
                core::BORDER_DEFAULT,
            )?;

            // Blend the warped image with the frame
            warped.copy_to_masked(frame, &final_mask)?;
        }
        Ok(())
    }

    // Modify process_video to use batch processing
    pub fn process_video(&mut self, game_data: &GameData) -> Result<(), Box<dyn Error>> {
        let mut frame = Mat::default();
        let total_frames = self.source.get(videoio::CAP_PROP_FRAME_COUNT)? as i32;
        let mut frame_count = 0;
        let batch_size = 10; // Adjust based on your system's capabilities
        let mut frame_batch = Vec::with_capacity(batch_size);

        while self.source.read(&mut frame)? {
            frame_batch.push(frame.clone());

            if frame_batch.len() >= batch_size || frame_count + 1 == total_frames {
                let processed_frames = self.process_frames_batch(frame_batch, game_data)?;

                for processed_frame in processed_frames {
                    self.output.write(&processed_frame)?;
                }

                frame_batch = Vec::with_capacity(batch_size);
            }

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
        // Pre-allocate matrices
        let mut mask = Mat::default();
        let mut temp_mask = Mat::default();

        // Use more efficient color space
        let mut hsv = Mat::default();
        imgproc::cvt_color(frame, &mut hsv, imgproc::COLOR_BGR2HSV, 0)?;

        // Optimize threshold values
        let lower_green = core::Scalar::new(35.0, 50.0, 50.0, 0.0);
        let upper_green = core::Scalar::new(85.0, 255.0, 255.0, 0.0);

        core::in_range(&hsv, &lower_green, &upper_green, &mut mask)?;

        // Use more efficient morphological operations
        let kernel = imgproc::get_structuring_element(
            imgproc::MORPH_RECT,
            core::Size::new(3, 3),
            core::Point::new(-1, -1),
        )?;

        imgproc::morphology_ex(
            &mask,
            &mut temp_mask,
            imgproc::MORPH_OPEN,
            &kernel,
            core::Point::new(-1, -1),
            1,
            core::BORDER_CONSTANT,
            core::Scalar::default(),
        )?;

        // Find contours
        let mut contours = Vector::<Vector<Point>>::new();
        imgproc::find_contours(
            &temp_mask,
            &mut contours,
            imgproc::RETR_EXTERNAL,
            imgproc::CHAIN_APPROX_SIMPLE,
            Point::new(0, 0),
        )?;

        // Create placements vector
        let mut placements = Vec::new();

        for contour in contours.iter() {
            let area = imgproc::contour_area(&contour, false)?;

            // Adjust minimum area threshold as needed
            if area < 100.0 {
                continue;
            }

            let rect = imgproc::bounding_rect(&contour)?;

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
