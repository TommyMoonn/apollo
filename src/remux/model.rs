use std::path::PathBuf;

use crate::media::container::MediaContainer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub container: MediaContainer,
}
