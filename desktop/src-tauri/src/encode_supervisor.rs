//! Own the FFmpeg child process for media runs launched through the Rust app.
//!
//! The compatibility engine still parses FFmpeg progress and builds reports.
//! This helper keeps the progress stream stable while ensuring that cancelling
//! or closing the Python adapter also stops its FFmpeg child.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

const MAX_PROGRESS_LINE_BYTES: usize = 64 * 1024;
const PROGRESS_QUEUE_CAPACITY: usize = 16;
const STDERR_TAIL_BYTES: usize = 16 * 1024;
const CHILD_SIGNAL_MARKER: &str = "__HEVC_NATIVE_CHILD_SIGNAL__=";

enum ProgressMessage {
    Line(Vec<u8>),
    Error(String),
}

pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    let Some(separator) = args.first() else {
        eprintln!("The internal FFmpeg supervisor requires -- followed by an FFmpeg command.");
        return 2;
    };
    if separator != "--" || args.len() < 2 {
        eprintln!("The internal FFmpeg supervisor requires -- followed by an FFmpeg command.");
        return 2;
    }

    supervise(&args[1..])
}

fn supervise(args: &[OsString]) -> i32 {
    let Some(program) = args.first() else {
        eprintln!("The internal FFmpeg supervisor received an empty command.");
        return 2;
    };

    let parent_pid = std::process::id();
    let mut command = Command::new(program);
    command
        .args(&args[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // If this helper is force-closed, Linux also closes the FFmpeg process.
        // The parent check handles the race where the adapter exits just before
        // the child completes its spawn setup.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::getppid() != parent_pid as libc::pid_t {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "the FFmpeg supervisor exited during process startup",
                    ));
                }
                Ok(())
            });
        }
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("Could not start FFmpeg: {error}");
            return 127;
        }
    };

    let Some(stdout) = child.stdout.take() else {
        eprintln!("Could not read FFmpeg progress output.");
        let _ = terminate_child(&mut child);
        return 2;
    };
    let Some(stderr) = child.stderr.take() else {
        eprintln!("Could not read FFmpeg diagnostics.");
        let _ = terminate_child(&mut child);
        return 2;
    };

    let (progress_sender, progress_receiver) = mpsc::sync_channel(PROGRESS_QUEUE_CAPACITY);
    let progress_thread = thread::spawn(move || forward_progress(stdout, progress_sender));
    let stderr_thread =
        thread::spawn(move || drain_and_forward_stderr(stderr, io::stderr().lock()));
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation_flag = Arc::clone(&cancelled);
    // This reader waits for cancellation/EOF. The short-lived helper exits after
    // the FFmpeg child, which closes stdin and releases the detached reader.
    let _control_thread = thread::spawn(move || read_cancel_requests(cancellation_flag));

    let mut output = io::stdout().lock();
    let mut child_status: Option<ExitStatus> = None;
    let mut progress_closed = false;
    let mut cancel_sent = false;
    let mut forwarding_error = None;

    while child_status.is_none() || !progress_closed {
        if cancelled.load(Ordering::Acquire) && !cancel_sent {
            cancel_sent = true;
            if let Err(error) = terminate_child(&mut child) {
                if child.try_wait().ok().flatten().is_none() {
                    forwarding_error =
                        Some(format!("Could not stop FFmpeg after cancellation: {error}"));
                }
            }
        }

        if child_status.is_none() {
            match child.try_wait() {
                Ok(status) => child_status = status,
                Err(error) => {
                    forwarding_error = Some(format!("Could not wait for FFmpeg: {error}"));
                    let _ = terminate_child(&mut child);
                    child_status = child.wait().ok();
                }
            }
        }

        if progress_closed {
            if child_status.is_none() {
                thread::sleep(Duration::from_millis(50));
            }
            continue;
        }

        match progress_receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(ProgressMessage::Line(line)) => {
                if let Err(error) = output.write_all(&line).and_then(|_| output.flush()) {
                    forwarding_error = Some(format!("Could not forward FFmpeg progress: {error}"));
                    cancelled.store(true, Ordering::Release);
                }
            }
            Ok(ProgressMessage::Error(error)) => {
                forwarding_error = Some(error);
                cancelled.store(true, Ordering::Release);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => progress_closed = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }

    let Some(status) = child_status.or_else(|| child.wait().ok()) else {
        eprintln!("Could not obtain FFmpeg's exit status.");
        return 2;
    };
    let _ = progress_thread.join();
    let _stderr_tail = stderr_thread.join().unwrap_or_else(|_| String::new());
    if let Some(error) = forwarding_error {
        eprintln!("{error}");
        return 2;
    }
    if cancelled.load(Ordering::Acquire) {
        // The caller requested a safe stop (or disconnected); it is already
        // handling cancellation and must not interpret this as an encode error.
        return status.code().unwrap_or(130);
    }

    #[cfg(unix)]
    if let Some(signal) = unix_signal(&status) {
        eprintln!("{CHILD_SIGNAL_MARKER}{signal}");
        return 255;
    }

    status.code().unwrap_or(255)
}

fn forward_progress(input: impl Read, sender: mpsc::SyncSender<ProgressMessage>) {
    let mut reader = BufReader::new(input);
    let mut line = Vec::with_capacity(256);
    loop {
        line.clear();
        match read_bounded_line(&mut reader, &mut line) {
            Ok(false) => break,
            Ok(true) => {
                if sender.send(ProgressMessage::Line(line.clone())).is_err() {
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                let _ = sender.send(ProgressMessage::Error(format!(
                    "FFmpeg progress line exceeded {MAX_PROGRESS_LINE_BYTES} bytes."
                )));
                break;
            }
            Err(error) => {
                let _ = sender.send(ProgressMessage::Error(format!(
                    "Could not read FFmpeg progress output: {error}"
                )));
                break;
            }
        }
    }
}

fn read_bounded_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<bool> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(!line.is_empty());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if count > MAX_PROGRESS_LINE_BYTES.saturating_sub(line.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "progress line exceeds configured limit",
            ));
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(true);
        }
    }
}

fn drain_and_forward_stderr(mut input: impl Read, mut output: impl Write) -> String {
    let mut tail = VecDeque::with_capacity(STDERR_TAIL_BYTES);
    let mut buffer = [0_u8; 4096];
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        if output.write_all(&buffer[..read]).is_err() {
            break;
        }
        let _ = output.flush();
        tail.extend(&buffer[..read]);
        while tail.len() > STDERR_TAIL_BYTES {
            tail.pop_front();
        }
    }
    String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>())
        .trim()
        .to_string()
}

fn read_cancel_requests(cancelled: Arc<AtomicBool>) {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        match line {
            Ok(line) if line.trim() == "cancel" => {
                cancelled.store(true, Ordering::Release);
                return;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    // EOF means the Python compatibility process closed unexpectedly. Stop the
    // child instead of leaving a detached encode running without its UI/report.
    cancelled.store(true, Ordering::Release);
}

fn terminate_child(child: &mut Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        if let Ok(pid) = libc::pid_t::try_from(child.id()) {
            // The encoder owns this group; terminate pipe-holding descendants too.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    child.wait().map(|_| ())
}

#[cfg(unix)]
fn unix_signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_are_bounded() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let bytes = vec![b'x'; MAX_PROGRESS_LINE_BYTES + 1];
        forward_progress(&bytes[..], sender);
        assert!(matches!(
            receiver.recv().unwrap(),
            ProgressMessage::Error(_)
        ));
    }

    #[test]
    fn stderr_tail_is_bounded_and_contains_recent_bytes() {
        let bytes = vec![b'a'; STDERR_TAIL_BYTES + 8];
        let tail = drain_and_forward_stderr(&bytes[..], Vec::new());
        assert_eq!(tail.len(), STDERR_TAIL_BYTES);
        assert!(tail.bytes().all(|byte| byte == b'a'));
    }
}
