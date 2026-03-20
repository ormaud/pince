//! Guest agent — PID 1 inside Firecracker microVMs.
//!
//! Responsibilities:
//!   1. Minimal init duties (mount /proc, /sys, mount workspace drive).
//!   2. Listen on vsock port 52000 for tool execution requests from the host.
//!   3. Dispatch tool calls and return results over vsock.
//!   4. On receiving a shutdown request, flush and exit cleanly.
//!
//! Wire format: 4-byte LE length prefix + JSON payload (see protocol.rs).
//!
//! # vsock
//!
//! Uses raw Linux `AF_VSOCK` sockets via libc since there is no standard-library
//! or tokio support for vsock.  The guest agent is intentionally synchronous to
//! minimize complexity in a PID-1 context.

#![allow(clippy::needless_return)]

mod tools;

// Re-use the protocol types from the library.
// When cross-compiled into the rootfs, this binary is built from the same
// crate, so `crate::protocol` is available.
use pince_sandbox::protocol::{ShutdownRequest, ToolRequest, ToolResponse};

use std::io::{Read, Write};
use std::os::unix::io::RawFd;

const VSOCK_PORT: u32 = 52000;
const WORKSPACE_DEV: &str = "/dev/vdb";
const WORKSPACE_MNT: &str = "/workspace";

fn main() {
    init_system();

    let listen_fd = vsock_listen(VSOCK_PORT).unwrap_or_else(|e| {
        eprintln!("guest-agent: vsock_listen failed: {e}");
        std::process::exit(1);
    });

    eprintln!("guest-agent: listening on vsock port {VSOCK_PORT}");

    loop {
        let conn_fd = vsock_accept(listen_fd).unwrap_or_else(|e| {
            eprintln!("guest-agent: accept failed: {e}");
            std::process::exit(1);
        });

        if let Err(e) = handle_connection(conn_fd) {
            eprintln!("guest-agent: connection error: {e}");
        }
    }
}

// ── Init duties ───────────────────────────────────────────────────────────────

fn init_system() {
    // Mount proc and sys — ignore errors (may already be mounted by kernel).
    let _ = mount("proc", "/proc", "proc");
    let _ = mount("sysfs", "/sys", "sysfs");

    // Mount the workspace block device.
    if let Err(e) = mount(WORKSPACE_DEV, WORKSPACE_MNT, "ext4") {
        eprintln!("guest-agent: warning: could not mount workspace: {e}");
    }
}

fn mount(source: &str, target: &str, fstype: &str) -> Result<(), std::io::Error> {
    // Create mount point if it doesn't exist.
    let _ = std::fs::create_dir_all(target);

    let source_c = std::ffi::CString::new(source).unwrap();
    let target_c = std::ffi::CString::new(target).unwrap();
    let fstype_c = std::ffi::CString::new(fstype).unwrap();

    let ret = unsafe {
        libc::mount(
            source_c.as_ptr(),
            target_c.as_ptr(),
            fstype_c.as_ptr(),
            0,
            std::ptr::null(),
        )
    };

    if ret == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

// ── vsock ─────────────────────────────────────────────────────────────────────

fn vsock_listen(port: u32) -> Result<RawFd, std::io::Error> {
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let addr = libc::sockaddr_vm {
            svm_family: libc::AF_VSOCK as libc::sa_family_t,
            svm_reserved1: 0,
            svm_port: port,
            svm_cid: libc::VMADDR_CID_ANY,
            svm_zero: [0u8; 4],
        };

        let bind_ret = libc::bind(
            fd,
            &addr as *const libc::sockaddr_vm as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t,
        );
        if bind_ret < 0 {
            libc::close(fd);
            return Err(std::io::Error::last_os_error());
        }

        let listen_ret = libc::listen(fd, 4);
        if listen_ret < 0 {
            libc::close(fd);
            return Err(std::io::Error::last_os_error());
        }

        Ok(fd)
    }
}

fn vsock_accept(listen_fd: RawFd) -> Result<RawFd, std::io::Error> {
    let conn_fd = unsafe { libc::accept(listen_fd, std::ptr::null_mut(), std::ptr::null_mut()) };
    if conn_fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(conn_fd)
    }
}

// ── Connection handler ────────────────────────────────────────────────────────

fn handle_connection(fd: RawFd) -> Result<(), String> {
    let mut stream = FdStream(fd);

    loop {
        // Read length-prefixed frame.
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Host closed the connection.
                return Ok(());
            }
            Err(e) => return Err(format!("read length: {e}")),
        }

        let len = u32::from_le_bytes(len_buf) as usize;
        if len > 64 * 1024 * 1024 {
            return Err(format!("payload too large: {len}"));
        }

        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).map_err(|e| format!("read payload: {e}"))?;

        // Check if this is a shutdown request.
        if let Ok(shutdown) = serde_json::from_slice::<ShutdownRequest>(&buf) {
            if shutdown.shutdown {
                // Send an acknowledgement and exit.
                let ack = ToolResponse::success(serde_json::Value::Null);
                let _ = send_frame(&mut stream, &ack);
                eprintln!("guest-agent: shutdown requested, exiting");
                std::process::exit(0);
            }
        }

        // Decode as tool request.
        let response = match serde_json::from_slice::<ToolRequest>(&buf) {
            Ok(req) => tools::dispatch(&req),
            Err(e) => ToolResponse::failure(format!("invalid request: {e}")),
        };

        send_frame(&mut stream, &response).map_err(|e| format!("send response: {e}"))?;
    }
}

fn send_frame<T: serde::Serialize>(
    stream: &mut FdStream,
    payload: &T,
) -> Result<(), std::io::Error> {
    let json = serde_json::to_vec(payload).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
    })?;
    let len = json.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&json)?;
    stream.flush()?;
    Ok(())
}

// ── FdStream ─────────────────────────────────────────────────────────────────

/// A thin `Read + Write` wrapper around a raw file descriptor.
struct FdStream(RawFd);

impl Read for FdStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = unsafe {
            libc::read(self.0, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
        };
        if n < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }
}

impl Write for FdStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = unsafe {
            libc::write(self.0, buf.as_ptr() as *const libc::c_void, buf.len())
        };
        if n < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for FdStream {
    fn drop(&mut self) {
        unsafe { libc::close(self.0) };
    }
}
