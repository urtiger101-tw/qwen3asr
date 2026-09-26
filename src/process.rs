//! Cancellable native processes with bounded diagnostic capture.
use anyhow::{Context, Result, bail};
use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

pub fn terminate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
}

fn read_bounded(
    mut reader: impl Read,
    limit: usize,
    tail: bool,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer)? {
            0 => break,
            n => {
                if tail {
                    bytes.extend_from_slice(&buffer[..n]);
                    if bytes.len() > limit {
                        bytes.drain(..bytes.len() - limit);
                        truncated = true;
                    }
                } else {
                    let keep = n.min(limit.saturating_sub(bytes.len()));
                    bytes.extend_from_slice(&buffer[..keep]);
                    truncated |= keep != n;
                }
            }
        }
    }
    Ok((bytes, truncated))
}

pub fn capture(command: &mut Command, cancel: &Arc<AtomicBool>, limit: usize) -> Result<Captured> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start native process")?;
    let stdout = child.stdout.take().context("Missing subprocess stdout")?;
    let stderr = child.stderr.take().context("Missing subprocess stderr")?;
    let out_reader = thread::spawn(move || read_bounded(stdout, limit, false));
    let err_reader = thread::spawn(move || read_bounded(stderr, 65536, true));
    let status = loop {
        if cancel.load(Ordering::Relaxed) || crate::config::CANCELLED.load(Ordering::Relaxed) {
            terminate(&mut child);
            let _ = out_reader.join();
            let _ = err_reader.join();
            bail!("Recognition cancelled");
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => {
                terminate(&mut child);
                let _ = out_reader.join();
                let _ = err_reader.join();
                return Err(error.into());
            }
        }
    };
    let (stdout, truncated) = out_reader
        .join()
        .map_err(|_| anyhow::anyhow!("Native stdout reader failed"))??;
    let (stderr, _) = err_reader
        .join()
        .map_err(|_| anyhow::anyhow!("Native stderr reader failed"))??;
    Ok(Captured {
        status,
        stdout,
        stderr,
        stdout_truncated: truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_retains_prefix_and_diagnostic_tail_without_unbounded_growth() {
        let input = vec![b'x'; 10000];
        let (bytes, truncated) = read_bounded(input.as_slice(), 17, false).unwrap();
        assert_eq!(bytes.len(), 17);
        assert!(truncated);
        let (bytes, truncated) = read_bounded(b"abcdef".as_slice(), 3, true).unwrap();
        assert_eq!(bytes, b"def");
        assert!(truncated);
    }
}
