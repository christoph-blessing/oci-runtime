use super::validation::AbsolutePath;
use super::validation::ExistingDir;
use semver::Version;

#[derive(Debug)]
pub struct Config {
    pub oci_version: Version,
    pub hostname: Option<String>,
    pub root: RootConfig,
}

#[derive(Debug)]
pub struct RootConfig {
    pub path: ExistingDir,
    pub readonly: bool,
}

pub struct MountConfig {
    pub destination: AbsolutePath,
}
