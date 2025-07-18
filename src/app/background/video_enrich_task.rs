// SPDX-FileCopyrightText: © 2024 David Bliss
//
// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::*;
use fotema_core::video::metadata;
use rayon::prelude::*;
use relm4::Worker;
use relm4::prelude::*;
use relm4::shared_state::Reducer;

use tracing::{error, info};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::components::progress_monitor::{
    MediaType, ProgressMonitor, ProgressMonitorInput, TaskName,
};

#[derive(Debug)]
pub enum VideoEnrichTaskInput {
    Start,
}

#[derive(Debug)]
pub enum VideoEnrichTaskOutput {
    // Thumbnail generation has started.
    Started,

    // Thumbnail generation has completed
    Completed(usize),
}

pub struct VideoEnrichTask {
    // Stop flag
    stop: Arc<AtomicBool>,
    repo: fotema_core::video::Repository,
    progress_monitor: Arc<Reducer<ProgressMonitor>>,
}

impl VideoEnrichTask {
    fn enrich(
        stop: Arc<AtomicBool>,
        mut repo: fotema_core::video::Repository,
        progress_monitor: Arc<Reducer<ProgressMonitor>>,
        sender: &ComponentSender<VideoEnrichTask>,
    ) -> Result<()> {
        let start = std::time::Instant::now();

        let unprocessed = repo.find_need_metadata_update()?;

        let count = unprocessed.len();
        info!("Found {} videos as candidates for enriching", count);

        // Short-circuit before sending progress messages to stop
        // banner from appearing and disappearing.
        if count == 0 {
            let _ = sender.output(VideoEnrichTaskOutput::Completed(count));
            return Ok(());
        }

        let _ = sender.output(VideoEnrichTaskOutput::Started);

        progress_monitor.emit(ProgressMonitorInput::Start(
            TaskName::Enrich(MediaType::Video),
            count,
        ));

        let metadatas = unprocessed
            .par_iter()
            .take_any_while(|_| !stop.load(Ordering::Relaxed))
            .flat_map(|vid| {
                let result = metadata::from_path(&vid.sandbox_path());
                progress_monitor.emit(ProgressMonitorInput::Advance);
                result.map(|m| (vid.video_id, m))
            })
            .collect();

        repo.add_metadata(metadatas)?;

        progress_monitor.emit(ProgressMonitorInput::Complete);

        info!(
            "Enriched {} videos in {} seconds.",
            count,
            start.elapsed().as_secs()
        );

        if let Err(e) = sender.output(VideoEnrichTaskOutput::Completed(count)) {
            error!("Failed sending VideoEnrichTaskOutput::Completed: {:?}", e);
        }

        Ok(())
    }
}

impl Worker for VideoEnrichTask {
    type Init = (
        Arc<AtomicBool>,
        fotema_core::video::Repository,
        Arc<Reducer<ProgressMonitor>>,
    );
    type Input = VideoEnrichTaskInput;
    type Output = VideoEnrichTaskOutput;

    fn init((stop, repo, progress_monitor): Self::Init, _sender: ComponentSender<Self>) -> Self {
        VideoEnrichTask {
            stop,
            repo,
            progress_monitor,
        }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            VideoEnrichTaskInput::Start => {
                info!("Enriching videos...");
                let stop = self.stop.clone();
                let repo = self.repo.clone();
                let progress_monitor = self.progress_monitor.clone();

                // Avoid runtime panic from calling block_on
                rayon::spawn(move || {
                    if let Err(e) = VideoEnrichTask::enrich(stop, repo, progress_monitor, &sender) {
                        error!("Failed to enrich videos: {}", e);
                    }
                });
            }
        };
    }
}
