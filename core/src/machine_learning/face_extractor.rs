// SPDX-FileCopyrightText: © 2024 David Bliss
//
// SPDX-License-Identifier: GPL-3.0-or-later

use crate::people::FaceDetectionCandidate;
use crate::thumbnailify::{ThumbnailSize, Thumbnailer};

use anyhow::*;

use super::nms::Nms;
use image::ImageReader;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::result::Result::Ok;

use rust_faces::{
    BlazeFaceParams, Face as DetectedFace, FaceDetection, FaceDetectorBuilder, ToArray3,
};

use gdk4::prelude::TextureExt;
use image::DynamicImage;
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone)]
pub struct Face {
    /// Path to thumbnail generated from face bounds.
    /// Normalized to be square and expanded to capture the whole head.
    pub thumbnail_path: PathBuf,

    /// Image cropped from bounds returned by face detection algorithm
    pub bounds_path: PathBuf,

    /// Bounds of detected face.
    pub bounds: Rect,

    /// Confidence (0.0 to 1.0) that the detected face is actually a face.
    pub confidence: f32,

    /// Facial landmarks.
    /// I _think_ this is right eye, left eye, nose, right mouth corner, left mouth corner.
    /// Note that left/right are from the subject's perspective, not the observer.
    landmarks: Option<Vec<(f32, f32)>>,

    /// Name of model that detected this face.
    pub model_name: String,
}

impl Face {
    fn landmark(&self, index: usize) -> Option<(f32, f32)> {
        self.landmarks
            .as_ref()
            .filter(|x| x.len() == 5)
            .map(|x| (x[index].0, x[index].1))
    }

    pub fn right_eye(&self) -> Option<(f32, f32)> {
        self.landmark(0)
    }

    pub fn left_eye(&self) -> Option<(f32, f32)> {
        self.landmark(1)
    }

    pub fn nose(&self) -> Option<(f32, f32)> {
        self.landmark(2)
    }

    pub fn right_mouth_corner(&self) -> Option<(f32, f32)> {
        self.landmark(3)
    }

    pub fn left_mouth_corner(&self) -> Option<(f32, f32)> {
        self.landmark(4)
    }
}

pub struct FaceExtractor {
    faces_base_path: PathBuf,
    thumbnail_base_path: PathBuf,

    thumbnailer: Thumbnailer,

    detectors: Vec<(Box<dyn rust_faces::FaceDetector>, String)>,
}

impl FaceExtractor {
    pub fn build(base_path: &Path, thumbnailer: Thumbnailer) -> Result<FaceExtractor> {
        let faces_base_path = PathBuf::from(base_path).join("faces");
        let _ = std::fs::create_dir_all(&faces_base_path)?;

        let thumbnail_base_path = PathBuf::from(base_path)
            .join("face_thumbnails")
            .join("small");
        let _ = std::fs::create_dir_all(&thumbnail_base_path)?;

        let mut detectors: Vec<(Box<dyn rust_faces::FaceDetector>, String)> = vec![];

        // Tweaking the target size seems to affect which faces are detected.
        // Testing against my library, it looks like smaller numbers match bigger faces,
        // bigger numbers smaller faces.
        //
        // 1280. Default. Misses larger faces.
        // 960. Three quarters. Matches a mix of some larger, some smaller.
        // 640. Half default. Misses a mix of some larger, some smaller.
        // 320. Quarter default. Matches only very big faces.

        /*
                let bz_params_huge = BlazeFaceParams {
                    score_threshold: 0.95,
                    target_size: 160,
                    ..BlazeFaceParams::default()
                };

                let blaze_face_huge =
                    FaceDetectorBuilder::new(FaceDetection::BlazeFace640(bz_params_huge.clone()))
                        .download()
                        .build()?;

                //detectors.push((blaze_face_huge, "blaze_face_640_huge".into()));

                let blaze_face_huge = FaceDetectorBuilder::new(FaceDetection::BlazeFace320(bz_params_huge))
                    .download()
                    .build()?;

                //detectors.push((blaze_face_huge, "blaze_face_320_huge".into()));

                let bz_params_big = BlazeFaceParams {
                    score_threshold: 0.95,
                    target_size: 640,
                    ..BlazeFaceParams::default()
                };

                let blaze_face_big =
                    FaceDetectorBuilder::new(FaceDetection::BlazeFace640(bz_params_big.clone()))
                        .download()
                        .build()?;

                //detectors.push((blaze_face_big, "blaze_face_640_big".into()));

                let blaze_face_big = FaceDetectorBuilder::new(FaceDetection::BlazeFace320(bz_params_big))
                    .download()
                    .build()?;

                //detectors.push((blaze_face_big, "blaze_face_320_big".into()));
        */
        let bz_params_default = BlazeFaceParams::default();

        let blaze_face_default =
            FaceDetectorBuilder::new(FaceDetection::BlazeFace640(bz_params_default.clone()))
                .download()
                .build()?;

        detectors.push((blaze_face_default, "blaze_face_640_default".into()));

        /*
        let blaze_face_default =
            FaceDetectorBuilder::new(FaceDetection::BlazeFace320(bz_params_small))
                .download()
                .build()?;

        detectors.push((blaze_face_default, "blaze_face_320_default".into()));
        */

        let mtcnn_params = rust_faces::MtCnnParams::default();

        let mtcnn = FaceDetectorBuilder::new(FaceDetection::MtCnn(mtcnn_params))
            .download()
            .build()?;

        detectors.push((mtcnn, "mtcnn".into()));

        Ok(FaceExtractor {
            faces_base_path,
            thumbnail_base_path,
            thumbnailer,
            detectors,
        })
    }

    /// Identify faces in a photo and return a vector of paths of extracted face images.
    pub async fn extract_faces(&self, candidate: &FaceDetectionCandidate) -> Result<Vec<Face>> {
        info!("Detecting faces in {:?}", candidate.host_path);

        let thumbnail_hash = candidate.thumbnail_hash();

        let image_path = self
            .thumbnailer
            .get_thumbnail_hash_output(&thumbnail_hash, ThumbnailSize::XLarge);

        let original_image = Self::open_image(&image_path).await?;

        let image = original_image.clone().into_rgb8().into_array3();

        let mut faces: Vec<(DetectedFace, String)> = vec![];

        for (detector, name) in &self.detectors {
            let result = detector.detect(image.view().into_dyn());
            if let Ok(detected_faces) = result {
                for f in detected_faces {
                    faces.push((f, name.into()));
                }
            } else {
                error!("Failed extracting faces with {name}: {:?}", result);
            }
        }

        // Use "non-maxima suppression" to remove duplicate matches.
        let nms = Nms::default();
        let faces = nms.suppress_non_maxima(faces);

        debug!(
            "Picture {} has {} faces.",
            candidate.picture_id,
            faces.len()
        );

        let faces = faces
            .into_iter()
            .enumerate()
            .map(|(index, (f, model_name))| {
                // Extract face and save to thumbnail.
                // The bounding box is pretty tight, so make it a bit bigger.
                // Also, make the box a square.

                let longest: f32 = if f.rect.width < f.rect.height {
                    f.rect.width
                } else {
                    f.rect.height
                };

                let mut longest = longest * 1.6;
                let mut half_longest = longest / 2.0;

                let (centre_x, centre_y) = Self::centre(&f);

                // Normalize thumbnail to be a square.
                if (original_image.width() as f32) < centre_x + half_longest {
                    half_longest = original_image.width() as f32 - centre_x;
                    longest = half_longest * 2.0;
                }
                if (original_image.height() as f32) < centre_y + half_longest {
                    half_longest = original_image.height() as f32 - centre_y;
                    longest = half_longest * 2.0;
                }

                if centre_x < half_longest {
                    half_longest = centre_x;
                    longest = half_longest * 2.0;
                }

                if centre_y < half_longest {
                    half_longest = centre_y;
                    longest = half_longest * 2.0;
                }

                // Don't panic when x or y would be < zero
                let mut x = centre_x - half_longest;
                if x < 0.0 {
                    x = 0.0;
                }
                let mut y = centre_y - half_longest;
                if y < 0.0 {
                    y = 0.0;
                }

                // FIXME use fast_image_resize instead of image-rs
                let thumbnail =
                    original_image.crop_imm(x as u32, y as u32, longest as u32, longest as u32);

                // 64x64 matches size in thumbnail list in picture view
                let thumbnail = thumbnail.thumbnail(64, 64);
                let thumbnail_path = self
                    .thumbnail_base_path
                    .join(format!("{}_{}.png", &thumbnail_hash, index));
                let _ = thumbnail.save(&thumbnail_path);

                let bounds = Rect {
                    x: f.rect.x,
                    y: f.rect.y,
                    width: f.rect.width,
                    height: f.rect.height,
                };

                let bounds_img = original_image.crop_imm(
                    bounds.x as u32,
                    bounds.y as u32,
                    bounds.width as u32,
                    bounds.height as u32,
                );

                let bounds_path = self
                    .faces_base_path
                    .join(format!("{}_{}.png", &thumbnail_hash, index));
                let _ = bounds_img.save(&bounds_path);

                Face {
                    thumbnail_path,
                    bounds_path,
                    bounds,
                    confidence: f.confidence,
                    landmarks: f.landmarks,
                    model_name,
                }
            })
            .collect();

        Ok(faces)
    }

    /// Computes the centre of a face.
    fn centre(f: &DetectedFace) -> (f32, f32) {
        if let Some(ref landmarks) = f.landmarks {
            // If we have landmarks, then the first two are the right and left eyes.
            // Use the midpoint between the eyes as the centre of the thumbnail.
            let x = (landmarks[0].0 + landmarks[1].0) / 2.0;
            let y = (landmarks[0].1 + landmarks[1].1) / 2.0;
            (x, y)
        } else {
            let x = f.rect.x + (f.rect.width / 2.0);
            let y = f.rect.y + (f.rect.height / 2.0);
            (x, y)
        }
    }

    async fn open_image(source_path: &Path) -> Result<DynamicImage> {
        let file = gio::File::for_path(source_path);

        let loader = glycin::Loader::new(file);
        let image = loader.load().await?;
        let frame = image.next_frame().await?;
        let bytes = frame.texture().save_to_png_bytes();
        let image =
            ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png).decode()?;

        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_faces() {
        let dir = env!("CARGO_MANIFEST_DIR");
        // let image_path = Path::new(dir).join("resources/test/Sandow.jpg");
        let image_path = Path::new(
            "/var/home/david/Pictures/Ente/Recents/0400B8FC-B0FB-413A-BDDA-428499E5905C.JPG",
        );
        let base_face_path = Path::new(".");
        /*
        let extractor = FaceExtractor::build(&base_face_path).unwrap();
        extractor
            .extract_faces(&PictureId::new(0), &image_path)
            .await
            .unwrap();
            */
    }
}
