//! Recording inspection and lossless-in-time joining for imported meetings.
//! Filesystem dates are hints only: copying a file does not date the recording.
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey, Tag},
    probe::Hint,
};

pub const EXTENSIONS: &[&str] = &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "mp4", "mov", "mkv", "webm"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSource {
    pub path: String,
    pub name: String,
    pub duration_ms: u64,
    pub recorded_start: Option<String>,
    pub recorded_end: Option<String>,
    pub timestamp_source: Option<String>,
    pub file_created_at: Option<String>,
    pub file_modified_at: Option<String>,
    #[serde(default)]
    pub offset_ms: u64,
}

fn system_date(time: std::io::Result<std::time::SystemTime>) -> Option<String> {
    time.ok().map(|t| DateTime::<Utc>::from(t).to_rfc3339())
}

fn parse_date(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim_matches(char::from(0)).trim();
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Some(date.with_timezone(&Utc));
    }
    // A year or a date without a time is not precise enough to order recordings.
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(date) = NaiveDateTime::parse_from_str(value, format) {
            return Local
                .from_local_datetime(&date)
                .single()
                .map(|d| d.with_timezone(&Utc));
        }
    }
    None
}

fn tag_date(tags: &[Tag]) -> Option<DateTime<Utc>> {
    tags.iter().find_map(|tag| {
        let key = tag.key.to_ascii_lowercase();
        if tag.std_key == Some(StandardTagKey::Date)
            || matches!(
                key.as_str(),
                "creation_time" | "recording_time" | "recorded_date" | "tdrc"
            )
        {
            parse_date(&tag.value.to_string())
        } else {
            None
        }
    })
}

/// Only reads bounded headers, seeking over media payloads (never loading a video).
/// BWF dates describe capture; MP4 creation dates describe the container and
/// are explicitly labelled as such in the import UI.
fn container_date(path: &Path) -> Option<(DateTime<Utc>, String)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut header = [0u8; 12];
    file.read_exact(&mut header).ok()?;
    if &header[..4] == b"RIFF" && &header[8..] == b"WAVE" {
        let mut offset = 12u64;
        while offset.checked_add(8)? <= len {
            file.seek(SeekFrom::Start(offset)).ok()?;
            let mut chunk = [0u8; 8];
            file.read_exact(&mut chunk).ok()?;
            let size = u32::from_le_bytes(chunk[4..8].try_into().ok()?) as u64;
            let end = offset.checked_add(8)?.checked_add(size)?;
            if end > len {
                return None;
            }
            if &chunk[..4] == b"bext" && size >= 338 {
                let mut bext = [0u8; 338];
                file.read_exact(&mut bext).ok()?;
                let date = std::str::from_utf8(&bext[320..330]).ok()?;
                let time = std::str::from_utf8(&bext[330..338]).ok()?;
                return parse_date(&format!("{date} {time}"))
                    .map(|d| (d, "Broadcast WAV recording time (local timezone)".into()));
            }
            offset = end.checked_add(size % 2)?;
        }
        return None;
    }
    fn atoms(file: &mut std::fs::File, start: u64, end: u64, depth: u8) -> Option<DateTime<Utc>> {
        if depth > 1 {
            return None;
        }
        let mut pos = start;
        while pos.checked_add(8)? <= end {
            file.seek(SeekFrom::Start(pos)).ok()?;
            let mut h = [0u8; 8];
            file.read_exact(&mut h).ok()?;
            let short = u32::from_be_bytes(h[..4].try_into().ok()?);
            let (size, header_len) = match short {
                0 => (end - pos, 8),
                1 => {
                    let mut long = [0u8; 8];
                    file.read_exact(&mut long).ok()?;
                    (u64::from_be_bytes(long), 16)
                }
                n => (n as u64, 8),
            };
            let next = pos.checked_add(size)?;
            if size < header_len || next > end {
                return None;
            }
            if &h[4..] == b"moov" {
                if let Some(date) = atoms(file, pos + header_len, next, depth + 1) {
                    return Some(date);
                }
            } else if &h[4..] == b"mvhd" && size >= header_len + 12 {
                let mut data = [0u8; 12];
                file.read_exact(&mut data).ok()?;
                let seconds = match data[0] {
                    0 => u32::from_be_bytes(data[4..8].try_into().ok()?) as i64,
                    1 => i64::try_from(u64::from_be_bytes(data[4..12].try_into().ok()?)).ok()?,
                    _ => return None,
                };
                // Zero means unspecified. QuickTime's epoch is 1904-01-01.
                if seconds == 0 {
                    return None;
                }
                let date = Utc
                    .timestamp_opt(seconds.checked_sub(2_082_844_800)?, 0)
                    .single()?;
                if date.timestamp() > 0 && date <= Utc::now() + chrono::Duration::days(1) {
                    return Some(date);
                }
            }
            pos = next;
        }
        None
    }
    atoms(&mut file, 0, len, 0).map(|d| {
        (
            d,
            "MP4/MOV container creation time (may be export time)".into(),
        )
    })
}

pub fn inspect(path: &Path) -> Result<RecordingSource, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "Unsupported recording: {}. Use WAV, MP3, M4A, AAC, FLAC, OGG, OPUS, MP4, MOV, MKV or WebM.",
            path.display()
        ));
    }
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("Not a file: {}", path.display()));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hint = Hint::new();
    hint.with_extension(&ext);
    let mut probe = symphonia::default::get_probe()
        .format(
            &hint,
            MediaSourceStream::new(Box::new(file), Default::default()),
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let track = probe
        .format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or("No audio track in recording")?;
    let params = &track.codec_params;
    let duration = params
        .n_frames
        .zip(params.time_base)
        .map(|(frames, base)| {
            let time = base.calc_time(frames);
            time.seconds * 1000 + (time.frac * 1000.0) as u64
        })
        .filter(|ms| *ms > 0);
    let tag_start = probe
        .metadata
        .get()
        .and_then(|m| m.current().and_then(|r| tag_date(r.tags())))
        .or_else(|| {
            probe
                .format
                .metadata()
                .current()
                .and_then(|r| tag_date(r.tags()))
        });
    let start = tag_start
        .map(|d| {
            (
                d,
                "Embedded recording/date tag (timezone-less dates use local timezone)".into(),
            )
        })
        .or_else(|| container_date(path));
    // Some formats have no duration header. Decode in bounded packets to count it.
    let duration_ms = if let Some(ms) = duration {
        ms
    } else {
        let (frames, rate) =
            crate::audio_decode::decode_audio_mono_stream_strict(path, |_, _, _, _| Ok(()))?;
        frames * 1000 / rate.max(1) as u64
    };
    if duration_ms == 0 {
        return Err(format!("Empty audio recording: {}", path.display()));
    }
    let recorded_end = start
        .as_ref()
        .and_then(|(date, _)| {
            date.checked_add_signed(chrono::Duration::milliseconds(
                i64::try_from(duration_ms).ok()?,
            ))
        })
        .map(|d| d.to_rfc3339());
    Ok(RecordingSource {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        duration_ms,
        recorded_start: start.as_ref().map(|(d, _)| d.to_rfc3339()),
        recorded_end,
        timestamp_source: start.map(|(_, source)| source),
        file_created_at: system_date(metadata.created()),
        file_modified_at: system_date(metadata.modified()),
        offset_ms: 0,
    })
}

/// Keeps all audio, in the user-selected order, without inserting wall-clock gaps
/// or trimming overlaps. Sources retain wall-clock dates plus playback offsets.
pub fn join(paths: &[String], dest: &Path) -> Result<Vec<RecordingSource>, String> {
    join_cancellable(paths, dest, || Ok(()))
}

pub fn join_cancellable(paths: &[String], dest: &Path, check: impl Fn() -> Result<(),String>) -> Result<Vec<RecordingSource>,String> {
    check()?;
    if paths.is_empty() {
        return Err("Choose at least one recording".into());
    }
    let mut sources = paths
        .iter()
        .map(|p| inspect(Path::new(p)))
        .collect::<Result<Vec<_>, _>>()?;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(dest, spec).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut total_frames = 0u64;
        for source in &mut sources {
            check()?;
            let start = total_frames;
            let mut resampler = None;
            let mut write = |samples: &[f32]| -> Result<(), String> {
                check()?;
                // Fail clearly instead of writing a WAV whose length wraps.
                crate::audio::ensure_wav_fits(&spec, total_frames + samples.len() as u64, "The combined recording")?;
                for &sample in samples {
                    if !sample.is_finite() {
                        return Err("Recording contains invalid audio samples".into());
                    }
                    writer.write_sample(sample).map_err(|e| e.to_string())?;
                }
                total_frames += samples.len() as u64;
                Ok(())
            };
            crate::audio_decode::decode_audio_mono_stream_strict(
                Path::new(&source.path),
                |samples, rate, _, _| {
                    if resampler.is_none() {
                        resampler = Some(crate::audio_preprocess::StreamingResampler::with_target(
                            rate, 48_000, 10_240,
                        )?);
                    }
                    write(&resampler.as_mut().unwrap().push(samples)?)
                },
            )
            .map_err(|e| format!("{}: {e}", source.name))?;
            if let Some(mut resampler) = resampler {
                write(&resampler.finish()?)?;
            }
            source.offset_ms = start * 1000 / 48_000;
            source.duration_ms = (total_frames - start) * 1000 / 48_000;
            if let Some(date) = source.recorded_start.as_deref().and_then(parse_date) {
                source.recorded_end = date
                    .checked_add_signed(chrono::Duration::milliseconds(source.duration_ms as i64))
                    .map(|d| d.to_rfc3339());
            }
            if total_frames == start {
                return Err(format!("No audio decoded from {}", source.name));
            }
        }
        writer.finalize().map_err(|e| e.to_string())?;
        Ok(sources)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

pub fn unique_audio_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        "import_{}_{:016x}.wav",
        Utc::now().timestamp_millis(),
        rand::random::<u64>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_join_removes_partial_output_and_preserves_original() {
        let dir=std::env::temp_dir().join(format!("ts-import-stop-{:016x}",rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();let input=dir.join("original.wav");let dest=dir.join("partial.wav");
        let spec=hound::WavSpec{channels:1,sample_rate:48000,bits_per_sample:16,sample_format:hound::SampleFormat::Int};
        let mut writer=hound::WavWriter::create(&input,spec).unwrap();
        for _ in 0..48000 {writer.write_sample(1000i16).unwrap();}writer.finalize().unwrap();
        let before=std::fs::read(&input).unwrap();let checks=std::cell::Cell::new(0);
        let result=join_cancellable(&[input.to_string_lossy().into_owned()],&dest,||{let n=checks.get()+1;checks.set(n);if n>=3{Err(crate::meeting_job::STOPPED.into())}else{Ok(())}});
        assert!(result.unwrap_err().contains(crate::meeting_job::STOPPED));assert!(!dest.exists());assert_eq!(std::fs::read(&input).unwrap(),before);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dates_need_time_and_do_not_use_copy_dates() {
        assert!(parse_date("2026").is_none());
        assert!(parse_date("2026-10-09").is_none());
        assert_eq!(
            parse_date("2026-10-09T12:30:00-04:00")
                .unwrap()
                .to_rfc3339(),
            "2026-10-09T16:30:00+00:00"
        );
    }
    #[test]
    fn joining_more_than_hundred_recordings_preserves_every_source() {
        let dir = std::env::temp_dir().join(format!("ts-import-large-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("input.wav");
        let spec = hound::WavSpec { channels: 1, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::create(&input, spec).unwrap();
        for _ in 0..480 { writer.write_sample(1000i16).unwrap(); }
        writer.finalize().unwrap();
        let paths = vec![input.to_string_lossy().to_string(); 151];
        let dest = dir.join("joined.wav");
        let sources = join(&paths, &dest).unwrap();
        assert_eq!(sources.len(), 151);
        assert_eq!(sources.last().unwrap().offset_ms, 1500);
        assert_eq!(hound::WavReader::open(dest).unwrap().duration(), 151 * 480);
        assert_eq!(hound::WavReader::open(input).unwrap().duration(), 480);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn joining_preserves_order_duration_and_originals() {
        let dir = std::env::temp_dir().join(format!("ts-import-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let paths: Vec<_> = [1000i16, -2000]
            .iter()
            .enumerate()
            .map(|(i, sample)| {
                let path = dir.join(format!("{i}.wav"));
                let mut writer = hound::WavWriter::create(&path, spec).unwrap();
                for _ in 0..1600 {
                    writer.write_sample(*sample).unwrap();
                    writer.write_sample(*sample).unwrap();
                }
                writer.finalize().unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        let dest = dir.join("joined.wav");
        let sources = join(&[paths[1].clone(), paths[0].clone()], &dest).unwrap();
        assert_eq!(sources[0].name, "1.wav");
        assert_eq!(sources[1].offset_ms, 100);
        assert_eq!(sources[1].duration_ms, 100);
        assert!(sources.iter().all(|s| s.recorded_start.is_none()));
        let mut reader = hound::WavReader::open(dest).unwrap();
        assert_eq!(reader.duration(), 9600);
        let samples: Vec<f32> = reader.samples().map(Result::unwrap).collect();
        assert!(samples[2400] < 0.0);
        assert!(samples[7200] > 0.0);
        assert!(paths.iter().all(|p| Path::new(p).is_file()));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_later_source_does_not_leave_partial_join() {
        let dir =
            std::env::temp_dir().join(format!("ts-import-bad-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.wav");
        std::fs::write(&bad, b"broken").unwrap();
        let dest = dir.join("joined.wav");
        assert!(join(&[bad.to_string_lossy().into_owned()], &dest).is_err());
        assert!(!dest.exists());
        assert!(bad.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn broadcast_wav_capture_dates_are_read_without_using_filesystem_dates() {
        let dir =
            std::env::temp_dir().join(format!("ts-import-bwf-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dated.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..16000 {
            writer.write_sample(1000i16).unwrap();
        }
        writer.finalize().unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let mut chunk = b"bext".to_vec();
        chunk.extend_from_slice(&602u32.to_le_bytes());
        let mut data = vec![0u8; 602];
        data[320..330].copy_from_slice(b"2026-10-08");
        data[330..338].copy_from_slice(b"13:25:00");
        chunk.extend(data);
        bytes.splice(12..12, chunk);
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let found = inspect(&path).unwrap();
        assert_eq!(
            found.recorded_start,
            Some(parse_date("2026-10-08 13:25:00").unwrap().to_rfc3339())
        );
        assert_eq!(found.duration_ms, 1000);
        assert!(found.timestamp_source.unwrap().contains("Broadcast WAV"));
        assert_eq!(
            DateTime::parse_from_rfc3339(&found.recorded_end.unwrap()).unwrap()
                - DateTime::parse_from_rfc3339(&found.recorded_start.unwrap()).unwrap(),
            chrono::Duration::seconds(1)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn mp4_creation_time_and_malformed_atom_bounds() {
        let dir =
            std::env::temp_dir().join(format!("ts-import-mp4-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dated.m4a");
        let date = parse_date("2026-10-08T12:00:00Z").unwrap();
        let mut bytes = 28u32.to_be_bytes().to_vec();
        bytes.extend(b"moov");
        bytes.extend(20u32.to_be_bytes());
        bytes.extend(b"mvhd");
        bytes.extend([0u8; 4]);
        bytes.extend(((date.timestamp() + 2_082_844_800) as u32).to_be_bytes());
        bytes.extend([0u8; 4]);
        std::fs::write(&path, &bytes).unwrap();
        let found = container_date(&path).unwrap();
        assert_eq!(found.0, date);
        assert!(found.1.contains("export time"));
        bytes[..4].copy_from_slice(&29u32.to_be_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(container_date(&path).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn truncated_wav_is_not_imported_as_a_complete_recording() {
        let dir =
            std::env::temp_dir().join(format!("ts-import-short-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("short.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..16000 {
            writer.write_sample(1000i16).unwrap();
        }
        writer.finalize().unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(2044).unwrap();
        drop(file);
        let dest = dir.join("joined.wav");
        assert!(join(&[path.to_string_lossy().into_owned()], &dest).is_err());
        assert!(!dest.exists());
        assert!(path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod offered_format_tests {
    use super::*;
    #[test]
    fn every_new_picker_format_can_be_inspected_and_imported_without_changing_originals() {
        let fixtures=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let root=std::env::temp_dir().join(format!("taurscribe-formats-{:016x}",rand::random::<u64>()));std::fs::create_dir_all(&root).unwrap();
        for (source,extension) in [("opus-1ch.ogg","opus"),("audit-opus.webm","webm"),("audit-opus.webm","mkv")] {
            let input=root.join(format!("original.{extension}"));std::fs::copy(fixtures.join(source),&input).unwrap();
            let before=std::fs::read(&input).unwrap();
            assert!(inspect(&input).unwrap().duration_ms>0);
            let output=root.join(format!("joined-{extension}.wav"));
            let result=join_cancellable(&[input.to_string_lossy().into_owned()],&output,||Ok(())).unwrap();
            assert_eq!(result.len(),1);assert!(result[0].duration_ms>0);
            assert!(hound::WavReader::open(output).unwrap().duration()>0);
            assert_eq!(std::fs::read(&input).unwrap(),before);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupt_mp3_import_removes_partial_join_and_keeps_the_original() {
        let source=Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audit-full.mp3");
        let root=std::env::temp_dir().join(format!("taurscribe-corrupt-import-{:016x}",rand::random::<u64>()));std::fs::create_dir_all(&root).unwrap();
        let bytes=std::fs::read(source).unwrap();let truncated=&bytes[..bytes.len()/2];
        let input=root.join("broken.mp3");std::fs::write(&input,truncated).unwrap();
        let output=root.join("joined.wav");
        assert!(join_cancellable(&[input.to_string_lossy().into_owned()],&output,||Ok(())).unwrap_err().contains("MP3 ended early"));
        assert!(!output.exists());assert_eq!(std::fs::read(&input).unwrap(),truncated);
        std::fs::remove_dir_all(root).unwrap();
    }
}
