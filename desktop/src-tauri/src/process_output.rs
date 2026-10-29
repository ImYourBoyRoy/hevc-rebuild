//! Process output readers stop on the first I/O error and preserve prior lines.
use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command};

/// Noninteractive jobs own a process group so pipe failures cannot strand descendants.
pub(crate) fn spawn_owned(command: &mut Command) -> std::io::Result<Child> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn()
}

/// Kill a process group created by `spawn_owned`. The caller still reaps the child.
pub(crate) fn kill_owned_group(process_id: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        let pid = libc::pid_t::try_from(process_id)
            .map_err(|_| "Engine process id is invalid.".to_string())?;
        let group_result = unsafe { libc::kill(-pid, libc::SIGKILL) };
        if group_result == 0 {
            return Ok(());
        }
        let group_error = std::io::Error::last_os_error();
        if group_error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        let process_result = unsafe { libc::kill(pid, libc::SIGKILL) };
        if process_result == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return Ok(());
        }
        Err(format!("Could not stop the engine process: {group_error}"))
    }
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .status()
            .map_err(|error| format!("Could not stop the engine process: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err("Could not stop the engine process tree.".into())
        }
    }
}

pub(crate) fn read_lines(reader: impl BufRead, mut emit: impl FnMut(String)) -> Result<(), String> {
    for line in reader.lines() {
        emit(line.map_err(|error| format!("Process output read failed: {error}"))?);
    }
    Ok(())
}

pub(crate) fn read_process_lines(
    reader: impl Read,
    process_id: u32,
    mut emit: impl FnMut(String),
) -> Result<(), String> {
    match read_lines(BufReader::new(reader), &mut emit) {
        Ok(()) => Ok(()),
        Err(error) => {
            emit(error.clone());
            // This PID belongs to the child whose pipe we own, and the owner
            // reaps it. No arbitrary process-control surface is exposed.
            #[cfg(unix)]
            unsafe {
                // A broken output contract is fatal. Kill the owned job group,
                // including descendants keeping its other pipe open. The owner
                // retains responsibility for reaping the direct child.
                if let Ok(pid) = libc::pid_t::try_from(process_id) {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[test]
    #[cfg(unix)]
    fn kill_owned_group_stops_spawned_child() {
        use std::process::Stdio;
        let mut child = spawn_owned(
            Command::new("python3")
                .args(["-c", "import time; time.sleep(30)"])
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )
        .unwrap();
        let start = std::time::Instant::now();
        kill_owned_group(child.id()).unwrap();
        assert!(!child.wait().unwrap().success());
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
    }
    #[test]
    #[cfg(unix)]
    fn pipe_error_kills_resistant_child_and_pipe_holding_descendant() {
        use std::process::Stdio;
        let script = "import os,signal,subprocess,sys,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); subprocess.Popen([sys.executable,'-c','import time;time.sleep(30)']); sys.stdout.buffer.write(b'valid\\n\\xff\\n');sys.stdout.flush();time.sleep(30)";
        let mut child = spawn_owned(
            Command::new("python3")
                .args(["-c", script])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .unwrap();
        let start = std::time::Instant::now();
        let mut lines = Vec::new();
        assert!(
            read_process_lines(child.stdout.take().unwrap(), child.id(), |line| lines
                .push(line))
            .is_err()
        );
        assert!(!child.wait().unwrap().success());
        let mut diagnostics = Vec::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut diagnostics)
            .unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(lines[0], "valid");
        assert!(lines[1].contains("read failed"));
    }
    struct RepeatedError<'a>(&'a AtomicUsize);
    impl Read for RepeatedError<'_> {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "fixture broken pipe",
            ))
        }
    }
    #[test]
    fn eof_normal_and_malformed_progress_lines_remain_compatible() {
        let mut lines = Vec::new();
        read_lines(
            Cursor::new(b"progress=end\n#HEVC_REBUILD_PROGRESS\tmalformed\ntail"),
            |line| lines.push(line),
        )
        .unwrap();
        assert_eq!(
            lines,
            ["progress=end", "#HEVC_REBUILD_PROGRESS\tmalformed", "tail"]
        );
        read_lines(Cursor::new(b""), |_| panic!("EOF is not a line")).unwrap();
    }
    #[test]
    fn repeated_error_reads_once_and_invalid_utf8_preserves_previous_lines() {
        let attempts = AtomicUsize::new(0);
        assert!(read_lines(BufReader::new(RepeatedError(&attempts)), |_| {})
            .unwrap_err()
            .contains("broken pipe"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        let mut lines = Vec::new();
        assert!(
            read_lines(Cursor::new(b"valid diagnostic\n\xff\n"), |line| lines
                .push(line))
            .is_err()
        );
        assert_eq!(lines, ["valid diagnostic"]);
    }
}
