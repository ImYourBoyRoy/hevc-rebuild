use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

const MAX_STREAMS: usize = 1024;
const STDERR_LINES: usize = 160;
const STDERR_LINE_BYTES: usize = 4096;

#[derive(Serialize, PartialEq, Eq)]
pub(crate) struct PacketFingerprint {
    pub(crate) extradata: Option<String>,
    pub(crate) packet_sha256: String,
    pub(crate) packet_count: u64,
    pub(crate) packet_bytes: u64,
}

struct FingerprintAccumulator {
    extradata: Option<String>,
    digest: Sha256,
    packet_count: u64,
    packet_bytes: u64,
    seen: bool,
}

impl FingerprintAccumulator {
    fn new() -> Self {
        Self {
            extradata: None,
            digest: Sha256::new(),
            packet_count: 0,
            packet_bytes: 0,
            seen: false,
        }
    }

    fn finish(&self) -> PacketFingerprint {
        PacketFingerprint {
            extradata: self.extradata.clone(),
            packet_sha256: encode_hex(&self.digest.clone().finalize()),
            packet_count: self.packet_count,
            packet_bytes: self.packet_bytes,
        }
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_nibble(pair[0])?;
        let low = hex_nibble(pair[1])?;
        decoded[index] = (high << 4) | low;
    }
    Some(decoded)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn consume_framehash_line(
    line: &str,
    streams: &mut [FingerprintAccumulator],
) -> Result<(), String> {
    if let Some(metadata) = line.strip_prefix("#media_type ") {
        if let Some((index, _)) = metadata.split_once(':') {
            if let Ok(index) = index.trim().parse::<usize>() {
                if let Some(stream) = streams.get_mut(index) {
                    stream.seen = true;
                }
            }
        }
        return Ok(());
    }

    if let Some(metadata) = line.strip_prefix("#extradata ") {
        let fields: Vec<_> = metadata.split(',').map(str::trim).collect();
        if fields.len() >= 3 {
            if let Ok(index) = fields[0].parse::<usize>() {
                if let Some(stream) = streams.get_mut(index) {
                    if decode_sha256(fields[2]).is_some() {
                        stream.extradata = Some(fields[2].to_ascii_lowercase());
                    }
                }
            }
        }
        return Ok(());
    }

    let fields: Vec<_> = line.trim().split(',').map(str::trim).collect();
    if fields.len() < 6 {
        return Ok(());
    }
    let (Ok(index), Ok(packet_size)) = (fields[0].parse::<usize>(), fields[4].parse::<u64>())
    else {
        return Ok(());
    };
    let Some(packet_digest) = decode_sha256(fields[5]) else {
        return Ok(());
    };
    let Some(stream) = streams.get_mut(index) else {
        return Ok(());
    };
    stream.packet_count = stream
        .packet_count
        .checked_add(1)
        .ok_or_else(|| "FFmpeg packet count exceeds the supported range.".to_string())?;
    stream.packet_bytes = stream
        .packet_bytes
        .checked_add(packet_size)
        .ok_or_else(|| "FFmpeg packet byte count exceeds the supported range.".to_string())?;
    stream.digest.update(packet_size.to_be_bytes());
    stream.digest.update(packet_digest);
    Ok(())
}

fn append_stderr_line(tail: &mut VecDeque<String>, line: &mut Vec<u8>) {
    if tail.len() == STDERR_LINES {
        tail.pop_front();
    }
    tail.push_back(String::from_utf8_lossy(line).into_owned());
    line.clear();
}

fn drain_stderr(mut input: impl Read) -> String {
    let mut tail = VecDeque::with_capacity(STDERR_LINES);
    let mut line = Vec::with_capacity(STDERR_LINE_BYTES);
    let mut buffer = [0u8; 4096];
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for byte in &buffer[..read] {
            if *byte == b'\n' {
                append_stderr_line(&mut tail, &mut line);
            } else {
                line.push(*byte);
                if line.len() > STDERR_LINE_BYTES * 2 {
                    line.drain(..line.len() - STDERR_LINE_BYTES);
                }
            }
        }
    }
    if !line.is_empty() {
        append_stderr_line(&mut tail, &mut line);
    }
    tail.into_iter().collect::<Vec<_>>().join("\n")
}

fn ffmpeg_args(path: &Path, specifiers: &[String], format: &str) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-loglevel"),
        OsString::from("error"),
        OsString::from("-nostdin"),
        OsString::from("-i"),
        path.as_os_str().to_owned(),
    ];
    for specifier in specifiers {
        args.push(OsString::from("-map"));
        args.push(OsString::from(format!("0:{specifier}")));
    }
    args.extend([
        OsString::from("-c"),
        OsString::from("copy"),
        OsString::from("-f"),
        OsString::from(format),
        OsString::from("-hash"),
        OsString::from("sha256"),
        OsString::from("-"),
    ]);
    args
}

fn validate_specifier(specifier: &str) -> Result<(), String> {
    let Some((kind, ordinal)) = specifier.split_once(':') else {
        return Err(format!("Invalid FFmpeg stream specifier {specifier:?}."));
    };
    if !matches!(kind, "v" | "a" | "s" | "t" | "d")
        || ordinal.is_empty()
        || !ordinal.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("Invalid FFmpeg stream specifier {specifier:?}."));
    }
    Ok(())
}

pub(crate) fn packet_fingerprints(
    ffmpeg: &Path,
    path: &Path,
    specifiers: &[String],
) -> Result<Vec<PacketFingerprint>, String> {
    if specifiers.len() > MAX_STREAMS {
        return Err(format!(
            "Stream verification is limited to {MAX_STREAMS} streams per file."
        ));
    }
    if specifiers.is_empty() {
        return Ok(Vec::new());
    }
    for specifier in specifiers {
        validate_specifier(specifier)?;
    }
    let args = ffmpeg_args(path, specifiers, "framehash");
    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start FFmpeg packet verification: {error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not read FFmpeg verification diagnostics.".to_string())?;
    let stderr_thread = thread::spawn(move || drain_stderr(stderr));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read FFmpeg packet fingerprints.".to_string())?;
    let mut reader = BufReader::new(stdout);
    let mut streams: Vec<_> = specifiers
        .iter()
        .map(|_| FingerprintAccumulator::new())
        .collect();
    let mut line = String::new();
    let parsed = loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break Ok(()),
            Ok(_) => {
                if let Err(error) = consume_framehash_line(&line, &mut streams) {
                    break Err(error);
                }
            }
            Err(error) => {
                break Err(format!(
                    "Could not read FFmpeg packet fingerprints: {error}"
                ))
            }
        }
    };
    drop(reader);
    if parsed.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|error| format!("Could not wait for FFmpeg packet verification: {error}"))?;
    let stderr_tail = stderr_thread
        .join()
        .unwrap_or_else(|_| "FFmpeg diagnostics reader stopped unexpectedly.".to_string());
    parsed?;
    if !status.success() {
        return Err(format!(
            "FFmpeg packet verification exited with status {} for {}.\n{}",
            status.code().unwrap_or(-1),
            path.display(),
            stderr_tail
        ));
    }
    if streams.iter().any(|stream| !stream.seen) {
        return Err(format!(
            "Could not obtain packet fingerprints for all mapped streams in {}.\n{}",
            path.display(),
            stderr_tail
        ));
    }
    Ok(streams.iter().map(FingerprintAccumulator::finish).collect())
}

pub(crate) fn stream_copy_hash(
    ffmpeg: &Path,
    path: &Path,
    specifier: &str,
) -> Result<String, String> {
    validate_specifier(specifier)?;
    let args = ffmpeg_args(path, &[specifier.to_owned()], "hash");
    let mut child = Command::new(ffmpeg)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start FFmpeg stream verification: {error}"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not read FFmpeg verification diagnostics.".to_string())?;
    let stderr_thread = thread::spawn(move || drain_stderr(stderr));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not read FFmpeg stream hash.".to_string())?;
    let mut output = String::new();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let read_result = loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break Ok(()),
            Ok(_) if output.len() + line.len() <= 16384 => output.push_str(&line),
            Ok(_) => break Err("FFmpeg stream hash output exceeded its limit.".to_string()),
            Err(error) => break Err(format!("Could not read FFmpeg stream hash: {error}")),
        }
    };
    drop(reader);
    if read_result.is_err() {
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|error| format!("Could not wait for FFmpeg stream verification: {error}"))?;
    let stderr_tail = stderr_thread
        .join()
        .unwrap_or_else(|_| "FFmpeg diagnostics reader stopped unexpectedly.".to_string());
    read_result?;
    if !status.success() {
        return Err(format!(
            "FFmpeg stream verification exited with status {} for {}.\n{}",
            status.code().unwrap_or(-1),
            path.display(),
            stderr_tail
        ));
    }
    let marker = "SHA256=";
    let digest = output
        .find(marker)
        .and_then(|index| output.get(index + marker.len()..index + marker.len() + 64))
        .filter(|value| decode_sha256(value).is_some())
        .ok_or_else(|| {
            format!("Could not compute compressed stream hash for {path:?} stream {specifier}.")
        })?;
    Ok(digest.to_ascii_lowercase())
}

fn parse_cli_args(args: Vec<OsString>) -> Result<(String, PathBuf, PathBuf, Vec<String>), String> {
    let mut args = args.into_iter();
    let action = args
        .next()
        .ok_or_else(|| "Missing internal verification action.".to_string())?
        .to_string_lossy()
        .into_owned();
    if action != "packet-fingerprints" && action != "stream-copy-hash" {
        return Err("Internal action must be packet-fingerprints or stream-copy-hash.".into());
    }
    let mut ffmpeg = None;
    let mut file = None;
    let mut streams = Vec::new();
    while let Some(flag) = args.next() {
        let flag = flag.to_string_lossy().into_owned();
        let value = args
            .next()
            .ok_or_else(|| format!("Missing value for {flag}."))?;
        match flag.as_str() {
            "--ffmpeg" => {
                if ffmpeg.replace(PathBuf::from(value)).is_some() {
                    return Err("--ffmpeg may be supplied only once.".into());
                }
            }
            "--file" => {
                if file.replace(PathBuf::from(value)).is_some() {
                    return Err("--file may be supplied only once.".into());
                }
            }
            "--stream" => streams.push(value.to_string_lossy().into_owned()),
            _ => return Err(format!("Unknown internal verification option {flag:?}.")),
        }
    }
    let ffmpeg = ffmpeg.ok_or_else(|| "Missing --ffmpeg path.".to_string())?;
    let file = file.ok_or_else(|| "Missing --file path.".to_string())?;
    if streams.is_empty() || streams.len() > MAX_STREAMS {
        return Err(format!("Provide 1 to {MAX_STREAMS} --stream values."));
    }
    if action == "stream-copy-hash" && streams.len() != 1 {
        return Err("stream-copy-hash accepts exactly one --stream value.".into());
    }
    Ok((action, ffmpeg, file, streams))
}

pub(crate) fn cli_mode(args: Vec<OsString>) -> i32 {
    let (action, ffmpeg, file, streams) = match parse_cli_args(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("{error}");
            return 2;
        }
    };
    let output = if action == "stream-copy-hash" {
        stream_copy_hash(&ffmpeg, &file, &streams[0])
            .map(|sha256| serde_json::json!({ "sha256": sha256 }))
    } else {
        packet_fingerprints(&ffmpeg, &file, &streams)
            .map(|fingerprints| serde_json::json!({ "fingerprints": fingerprints }))
    };
    match output.and_then(|value| {
        serde_json::to_string(&value)
            .map_err(|error| format!("Could not serialize verification result: {error}"))
    }) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{consume_framehash_line, decode_sha256, encode_hex, FingerprintAccumulator};
    use sha2::{Digest, Sha256};

    #[test]
    fn decodes_sha256_hex_case_insensitively() {
        let digest =
            decode_sha256("ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789")
                .expect("valid 32-byte hash");
        assert_eq!(
            encode_hex(&digest),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert!(decode_sha256("g".repeat(64).as_str()).is_none());
        assert!(decode_sha256("00").is_none());
    }

    #[test]
    fn folds_packet_size_and_packet_digest_into_fixed_stream_digest() {
        let packet_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut streams = vec![FingerprintAccumulator::new()];
        consume_framehash_line("#media_type 0: audio", &mut streams).unwrap();
        consume_framehash_line(&format!("0, 1, 2, 3, 17, {packet_digest}"), &mut streams).unwrap();

        let mut expected = Sha256::new();
        expected.update(17u64.to_be_bytes());
        expected.update(decode_sha256(packet_digest).unwrap());
        assert!(streams[0].seen);
        assert_eq!(streams[0].packet_count, 1);
        assert_eq!(streams[0].packet_bytes, 17);
        assert_eq!(
            encode_hex(&expected.finalize()),
            encode_hex(&streams[0].digest.clone().finalize())
        );
    }

    #[test]
    fn ignores_headers_malformed_packets_and_out_of_range_streams() {
        let mut streams = vec![FingerprintAccumulator::new()];
        consume_framehash_line("#format: frame checksums", &mut streams).unwrap();
        consume_framehash_line("0, 1, 2, 3, 17, not-a-hash", &mut streams).unwrap();
        consume_framehash_line(
            "#extradata 8, 4, 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            &mut streams,
        )
        .unwrap();
        assert_eq!(streams[0].packet_count, 0);
        assert!(!streams[0].seen);
    }

    #[test]
    fn rejects_stream_specifiers_outside_the_engine_contract() {
        assert!(super::validate_specifier("a:0").is_ok());
        assert!(super::validate_specifier("v:12").is_ok());
        for value in ["a:", "a:0?", "0:a:0", "a:0 -c copy", "file:0"] {
            assert!(
                super::validate_specifier(value).is_err(),
                "accepted {value:?}"
            );
        }
    }
}
