pub mod core;
pub mod s3;

use std::path::PathBuf;

use rocket::{
    http::uri::{error::PathError, fmt::Path, Segments},
    request::FromSegments,
};

pub struct DotPathBuf(PathBuf);

impl<'r> FromSegments<'r> for DotPathBuf {
    type Error = PathError;
    fn from_segments(segments: Segments<'r, Path>) -> Result<Self, Self::Error> {
        segments.to_path_buf(true).map(DotPathBuf)
    }
}