// SPDX-FileCopyrightText: © 2024 David Bliss
//
// SPDX-License-Identifier: GPL-3.0-or-later

pub mod database;
pub mod flatpak_path;
pub mod machine_learning;
pub mod path_encoding;
pub mod people;
pub mod photo;
pub mod scanner;
pub mod thumbnailify;
pub mod time;
pub mod video;
pub mod visual;

pub use flatpak_path::FlatpakPathBuf;
pub use people::model::FaceId;
pub use people::model::PersonId;
pub use photo::model::PictureId;
pub use scanner::ScannedFile;
pub use scanner::Scanner;
pub use time::Year;
pub use time::YearMonth;
pub use video::VideoId;
pub use visual::Visual;
pub use visual::VisualId;
