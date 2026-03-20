//! Firecracker microVM process management.
//!
//! Builds the Firecracker config JSON and manages the process lifecycle.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Child;

use crate::config::SandboxConfig;
use crate::error::SandboxError;

// ── Firecracker config JSON ───────────────────────────────────────────────────

#[derive(Serialize)]
struct FcConfig {
    #[serde(rename = "boot-source")]
    boot_source: BootSource,
    drives: Vec<Drive>,
    #[serde(rename = "machine-config")]
    machine_config: MachineConfig,
    vsock: Vsock,
}

#[derive(Serialize)]
struct BootSource {
    kernel_image_path: PathBuf,
    boot_args: String,
}

#[derive(Serialize)]
struct Drive {
    drive_id: String,
    path_on_host: PathBuf,
    is_root_device: bool,
    is_read_only: bool,
}

#[derive(Serialize)]
struct MachineConfig {
    vcpu_count: u32,
    mem_size_mib: u32,
}

#[derive(Serialize)]
struct Vsock {
    vsock_id: String,
    guest_cid: u32,
    uds_path: PathBuf,
}

// ── Public types ──────────────────────────────────────────────────────────────

/// A running Firecracker microVM process.
pub struct FirecrackerInstance {
    /// The Firecracker child process.
    pub process: Child,
    /// Host-side Unix socket path for vsock proxy (without port suffix).
    pub uds_path: PathBuf,
    /// Path to the generated config JSON file.
    pub config_path: PathBuf,
}

impl FirecrackerInstance {
    /// Spawn a Firecracker microVM.
    ///
    /// - `run_dir`: directory for runtime files (config JSON, vsock socket).
    /// - `workspace_image`: path to the per-agent ext4 workspace image.
    pub async fn spawn(
        config: &SandboxConfig,
        agent_id: &str,
        workspace_image: &Path,
        run_dir: &Path,
    ) -> Result<Self, SandboxError> {
        let config_path = run_dir.join(format!("{agent_id}.json"));
        let uds_path = run_dir.join(format!("{agent_id}.vsock"));

        let fc_config = FcConfig {
            boot_source: BootSource {
                kernel_image_path: config.kernel_image.clone(),
                // console=ttyS0 for serial output; reboot=k to reboot instead of shutdown
                boot_args: "console=ttyS0 reboot=k panic=1 pci=off".to_string(),
            },
            drives: vec![
                Drive {
                    drive_id: "rootfs".to_string(),
                    path_on_host: config.rootfs_image.clone(),
                    is_root_device: true,
                    is_read_only: true,
                },
                Drive {
                    drive_id: "workspace".to_string(),
                    path_on_host: workspace_image.to_path_buf(),
                    is_root_device: false,
                    is_read_only: false,
                },
            ],
            machine_config: MachineConfig {
                vcpu_count: config.default_vcpu_count,
                mem_size_mib: config.default_memory_mb,
            },
            vsock: Vsock {
                vsock_id: "1".to_string(),
                // Guest CID 3 is the standard guest CID in Firecracker.
                guest_cid: 3,
                uds_path: uds_path.clone(),
            },
        };

        let config_json = serde_json::to_string_pretty(&fc_config).map_err(|e| {
            SandboxError::SpawnFailed(agent_id.to_string(), format!("serialize config: {e}"))
        })?;

        tokio::fs::write(&config_path, &config_json).await.map_err(|e| {
            SandboxError::SpawnFailed(agent_id.to_string(), format!("write config: {e}"))
        })?;

        let process = tokio::process::Command::new(&config.firecracker_binary)
            .arg("--no-api")
            .arg("--config-file")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                SandboxError::SpawnFailed(
                    agent_id.to_string(),
                    format!("spawn firecracker: {e}"),
                )
            })?;

        tracing::debug!(agent_id, ?uds_path, "firecracker process spawned");

        Ok(FirecrackerInstance { process, uds_path, config_path })
    }

    /// Gracefully terminate the process, with a timeout before force-kill.
    pub async fn shutdown(mut self, timeout: Duration) {
        // Try graceful termination first (SIGTERM).
        let _ = self.process.start_kill();

        if tokio::time::timeout(timeout, self.process.wait()).await.is_err() {
            tracing::warn!("firecracker did not exit within timeout, force-killing");
            let _ = self.process.kill().await;
        }

        // Clean up the config file; ignore errors.
        let _ = tokio::fs::remove_file(&self.config_path).await;
    }
}

// ── Workspace image creation ──────────────────────────────────────────────────

/// Create a sparse ext4 workspace image at `path` with the given size in MiB.
///
/// Requires `dd` and `mkfs.ext4` (from e2fsprogs) to be available on the host.
pub async fn create_workspace_image(path: &Path, size_mb: u32) -> Result<(), SandboxError> {
    // Create a sparse file using `dd` with seek (no actual data written).
    let status = tokio::process::Command::new("dd")
        .args(["if=/dev/zero", "bs=1M", "count=0"])
        .arg(format!("seek={size_mb}"))
        .arg(format!("of={}", path.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| {
            SandboxError::SpawnFailed(
                "workspace".to_string(),
                format!("dd not available: {e}"),
            )
        })?;

    if !status.success() {
        return Err(SandboxError::SpawnFailed(
            "workspace".to_string(),
            "dd failed to create workspace image".to_string(),
        ));
    }

    // Format as ext4.
    let status = tokio::process::Command::new("mkfs.ext4")
        .arg("-q")
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|e| {
            SandboxError::SpawnFailed(
                "workspace".to_string(),
                format!("mkfs.ext4 not available: {e}"),
            )
        })?;

    if !status.success() {
        return Err(SandboxError::SpawnFailed(
            "workspace".to_string(),
            "mkfs.ext4 failed to format workspace image".to_string(),
        ));
    }

    tracing::debug!(?path, size_mb, "workspace ext4 image created");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fc_config_serializes_correctly() {
        let fc_config = FcConfig {
            boot_source: BootSource {
                kernel_image_path: PathBuf::from("/boot/vmlinux"),
                boot_args: "console=ttyS0".to_string(),
            },
            drives: vec![
                Drive {
                    drive_id: "rootfs".to_string(),
                    path_on_host: PathBuf::from("/rootfs.ext4"),
                    is_root_device: true,
                    is_read_only: true,
                },
                Drive {
                    drive_id: "workspace".to_string(),
                    path_on_host: PathBuf::from("/workspace.ext4"),
                    is_root_device: false,
                    is_read_only: false,
                },
            ],
            machine_config: MachineConfig { vcpu_count: 1, mem_size_mib: 256 },
            vsock: Vsock {
                vsock_id: "1".to_string(),
                guest_cid: 3,
                uds_path: PathBuf::from("/tmp/agent.vsock"),
            },
        };

        let json = serde_json::to_value(&fc_config).unwrap();
        assert_eq!(json["boot-source"]["kernel_image_path"], "/boot/vmlinux");
        assert_eq!(json["machine-config"]["vcpu_count"], 1);
        assert_eq!(json["vsock"]["guest_cid"], 3);
        assert_eq!(json["drives"][0]["drive_id"], "rootfs");
        assert_eq!(json["drives"][1]["drive_id"], "workspace");
        assert!(json["drives"][0]["is_read_only"].as_bool().unwrap());
        assert!(!json["drives"][1]["is_read_only"].as_bool().unwrap());
    }
}
