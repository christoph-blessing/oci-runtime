use crate::AbsolutePath;
use crate::ExistingDir;
use semver::Version;

#[derive(Debug)]
pub struct Config {
    pub oci_version: Version,
    pub root: RootConfig,
}

#[derive(Debug)]
pub struct RootConfig {
    pub path: ExistingDir,
}

pub struct MountConfig {
    pub destination: AbsolutePath,
}
