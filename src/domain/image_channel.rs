//! Which Arkime container image to run in docker mode: the latest stable release
//! or the bleeding-edge snapshot build. Both live under the same ghcr repo,
//! differing only by tag.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageChannel {
    /// Latest stable release.
    #[default]
    Stable,
    /// Latest snapshot (built from the development branch).
    Snapshot,
}

impl ImageChannel {
    pub const ALL: [ImageChannel; 2] = [ImageChannel::Stable, ImageChannel::Snapshot];

    const STABLE_IMAGE: &'static str = "ghcr.io/arkime/arkime/arkime:v6-ja4-latest";
    const SNAPSHOT_IMAGE: &'static str = "ghcr.io/arkime/arkime/arkime:snapshot-v6-ja4-latest";

    /// Toggle Stable <-> Snapshot.
    pub fn cycle(self) -> ImageChannel {
        match self {
            ImageChannel::Stable => ImageChannel::Snapshot,
            ImageChannel::Snapshot => ImageChannel::Stable,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ImageChannel::Stable => "Stable — latest released Arkime",
            ImageChannel::Snapshot => "Snapshot — latest development build",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            ImageChannel::Stable => "stable",
            ImageChannel::Snapshot => "snapshot",
        }
    }

    /// The fully-qualified image reference for this channel.
    pub fn image(self) -> &'static str {
        match self {
            ImageChannel::Stable => Self::STABLE_IMAGE,
            ImageChannel::Snapshot => Self::SNAPSHOT_IMAGE,
        }
    }

    /// Recover the channel from an existing compose image reference. A tag
    /// containing "snapshot" is the snapshot channel; anything else is stable.
    pub fn from_image(image: &str) -> ImageChannel {
        if image.contains("snapshot") {
            ImageChannel::Snapshot
        } else {
            ImageChannel::Stable
        }
    }
}
