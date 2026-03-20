//! Sandbox configuration.

use std::path::PathBuf;

use serde::Deserialize;

/// Configuration for the sandbox runtime.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Path to the Firecracker binary.
    pub firecracker_binary: PathBuf,
    /// Path to the kernel image (vmlinux).
    pub kernel_image: PathBuf,
    /// Path to the read-only root filesystem image.
    pub rootfs_image: PathBuf,
    /// Base directory for per-agent workspace directories.
    pub workspace_base: PathBuf,
    /// Default memory per microVM in MiB.
    pub default_memory_mb: u32,
    /// Default vCPU count per microVM.
    pub default_vcpu_count: u32,
    /// Vsock port the guest agent listens on.
    pub vsock_port: u32,
    /// Size of the per-agent workspace ext4 image in MiB.
    pub workspace_size_mb: u32,
    /// Seconds to wait for guest boot before timing out.
    pub boot_timeout_secs: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let data_dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                PathBuf::from(home).join(".local/share")
            });

        Self {
            firecracker_binary: PathBuf::from("/usr/bin/firecracker"),
            kernel_image: PathBuf::from("/var/lib/pince/vmlinux"),
            rootfs_image: PathBuf::from("/var/lib/pince/rootfs.ext4"),
            workspace_base: data_dir.join("pince/workspaces"),
            default_memory_mb: 256,
            default_vcpu_count: 1,
            vsock_port: 52000,
            workspace_size_mb: 1024,
            boot_timeout_secs: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = SandboxConfig::default();
        assert_eq!(cfg.default_memory_mb, 256);
        assert_eq!(cfg.default_vcpu_count, 1);
        assert_eq!(cfg.vsock_port, 52000);
        assert!(cfg.boot_timeout_secs > 0);
    }

    #[test]
    fn deserialize_partial_toml() {
        let toml_str = r#"
default_memory_mb = 512
default_vcpu_count = 2
"#;
        let cfg: SandboxConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.default_memory_mb, 512);
        assert_eq!(cfg.default_vcpu_count, 2);
        // Non-specified fields get defaults
        assert_eq!(cfg.vsock_port, 52000);
    }

    #[test]
    fn deserialize_full_config() {
        let toml_str = r#"
firecracker_binary = "/opt/bin/firecracker"
kernel_image = "/boot/vmlinux"
rootfs_image = "/var/lib/pince/rootfs.ext4"
workspace_base = "/tmp/workspaces"
default_memory_mb = 128
default_vcpu_count = 1
vsock_port = 12345
workspace_size_mb = 512
boot_timeout_secs = 60
"#;
        let cfg: SandboxConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.firecracker_binary, PathBuf::from("/opt/bin/firecracker"));
        assert_eq!(cfg.vsock_port, 12345);
        assert_eq!(cfg.workspace_size_mb, 512);
        assert_eq!(cfg.boot_timeout_secs, 60);
    }
}
