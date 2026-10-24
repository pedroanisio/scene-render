//! The PNG frames of a `render`: converted on the output stage to 16-bit display sRGB with their alpha, and written
//! on a writer thread so the render thread neither encodes the files nor waits for one frame's readback before
//! rendering the next.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, sync_channel, Receiver, SyncSender, TryRecvError};
use std::thread::JoinHandle;
use std::time::Instant;

use sr_gpu::output::FrameBytes;

/// A frame on its way to a PNG file: what the report says of it once it is written.
pub struct Frame {
    /// The file.
    pub path: PathBuf,
    /// The file's name under the output directory: the fingerprint sidecar's key.
    pub name: String,
    /// The frame's size.
    pub size: [u32; 2],
    /// Composition time.
    pub time: f64,
    /// Draws the frame took.
    pub draws: usize,
    /// The fingerprint `--changed-only` records once the file is there, when it keeps one.
    pub print: Option<String>,
}

/// A frame the writer finished: whether writing it failed, and the seconds that took.
pub struct Written {
    pub frame: Frame,
    pub error: Option<String>,
    pub seconds: f64,
}

struct Job {
    frame: Frame,
    bytes: FrameBytes,
}

/// Writes frames as PNG files on its own thread, at most two queued ahead of it. After a failure it writes no more,
/// so no frame after the failed one appears; the frames before it are complete.
pub struct Writer {
    jobs: Option<SyncSender<Job>>,
    done: Receiver<Written>,
    thread: Option<JoinHandle<()>>,
}

impl Writer {
    /// A writer of PNG files at `bit_depth` bits a channel (8 or 16).
    pub fn start(bit_depth: u8) -> Writer {
        let (jobs, rx) = sync_channel::<Job>(2);
        let (tx, done) = channel::<Written>();
        let thread = std::thread::Builder::new()
            .name("png writer".into())
            .spawn(move || {
                for Job { frame, bytes } in rx {
                    let start = Instant::now();
                    let error = write(&frame.path, frame.size, &bytes, bit_depth).err();
                    drop(bytes);
                    let failed = error.is_some();
                    if tx.send(Written { frame, error, seconds: start.elapsed().as_secs_f64() }).is_err() || failed {
                        break;
                    }
                }
            })
            .expect("a writer thread");
        Writer { jobs: Some(jobs), done, thread: Some(thread) }
    }

    /// Queues `frame`'s `bytes` (16-bit RGBA, little-endian, straight alpha); waits while two frames are queued. A
    /// frame queued after a failure is not written: the failure is in [`ready`](Self::ready) or
    /// [`finish`](Self::finish).
    pub fn send(&self, frame: Frame, bytes: FrameBytes) {
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Job { frame, bytes });
        }
    }

    /// The frames written since the last call, without waiting.
    pub fn ready(&self) -> Vec<Written> {
        let mut v = Vec::new();
        loop {
            match self.done.try_recv() {
                Ok(w) => v.push(w),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return v,
            }
        }
    }

    /// Waits for every queued frame: the frames written since [`ready`](Self::ready).
    pub fn finish(mut self) -> Vec<Written> {
        drop(self.jobs.take());
        let v = self.done.iter().collect();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        v
    }
}

impl Drop for Writer {
    /// Every queued frame is written out before the writer goes, so a run that stops early leaves no torn file.
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Writes `bytes`, a `size` frame of 16-bit RGBA, little-endian, as a PNG at `bit_depth`: the 16-bit frame as it
/// is, or each sample rounded to 8 bits.
fn write(path: &Path, [w, h]: [u32; 2], bytes: &[u8], bit_depth: u8) -> Result<(), String> {
    let samples = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b));
    let res = if bit_depth == 16 {
        let data: Vec<u16> = samples.collect();
        image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(w, h, data).expect("sized").save(path)
    } else {
        let data: Vec<u8> = samples.map(|v| ((v as u32 + 128) / 257) as u8).collect();
        image::RgbaImage::from_raw(w, h, data).expect("sized").save(path)
    };
    res.map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-frames-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A 2x1 frame: a half-transparent mid grey and an opaque blue, 16-bit little-endian.
    fn frame() -> Vec<u8> {
        [[32896u16, 32896, 32896, 32768], [0x3333, 0x6666, 0xCCCC, 65535]]
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| v.to_le_bytes()))
            .collect()
    }

    fn a_frame(path: PathBuf) -> Frame {
        Frame {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            size: [2, 1],
            time: 0.0,
            draws: 1,
            print: None,
        }
    }

    #[test]
    fn writes_the_frame_at_sixteen_bits_and_rounded_to_eight() {
        let d = dir("depths");
        write(&d.join("16.png"), [2, 1], &frame(), 16).unwrap();
        write(&d.join("8.png"), [2, 1], &frame(), 8).unwrap();
        let sixteen = image::open(d.join("16.png")).unwrap().into_rgba16();
        assert_eq!(sixteen.get_pixel(0, 0).0, [32896, 32896, 32896, 32768]);
        assert_eq!(sixteen.get_pixel(1, 0).0, [0x3333, 0x6666, 0xCCCC, 65535]);
        let eight = image::open(d.join("8.png")).unwrap().into_rgba8();
        assert_eq!(eight.get_pixel(0, 0).0, [128, 128, 128, 128]);
        assert_eq!(eight.get_pixel(1, 0).0, [0x33, 0x66, 0xCC, 255]);
    }

    #[test]
    fn frames_come_back_in_order_and_none_is_written_after_a_failure() {
        let d = dir("failure");
        std::fs::create_dir_all(d.join("f1.png")).unwrap();
        let writer = Writer::start(8);
        for k in 0..4 {
            writer.send(a_frame(d.join(format!("f{k}.png"))), FrameBytes::from(frame()));
        }
        let done = writer.finish();
        let names: Vec<_> = done.iter().map(|w| w.frame.name.as_str()).collect();
        assert_eq!(names, ["f0.png", "f1.png"], "the failure ends the writing");
        assert!(done[0].error.is_none() && done[0].seconds > 0.0);
        let e = done[1].error.as_deref().unwrap();
        assert!(e.starts_with("cannot write ") && e.contains("f1.png"), "{e}");
        assert!(d.join("f0.png").is_file());
        assert!(!d.join("f2.png").exists() && !d.join("f3.png").exists());
    }

    #[test]
    fn a_dropped_writer_finishes_what_it_was_sent() {
        let d = dir("drop");
        let writer = Writer::start(16);
        writer.send(a_frame(d.join("f0.png")), FrameBytes::from(frame()));
        writer.send(a_frame(d.join("f1.png")), FrameBytes::from(frame()));
        drop(writer);
        assert!(d.join("f0.png").is_file() && d.join("f1.png").is_file());
    }
}
