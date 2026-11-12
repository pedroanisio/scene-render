//! Rendering parts of a video output on other machines.
//!
//! A video output that renders in chunks (see `Parallel`) can hand some of them to engines running elsewhere. This
//! module is the delivery's side of that: what a chunk is ([`Part`]), what a worker is asked for ([`Segment`], an
//! option of the delivery that renders one chunk and nothing else), and the workers themselves as the delivery sees
//! them ([`RemoteWorkers`]). How a worker is reached (the transport, the files it needs, who may ask) is the
//! caller's: `scene-render encode --workers` and `scene-render serve`.
//!
//! The coordinator keeps the chunks in one queue. Its own workers and the remote ones take the next chunk as they
//! finish the last, so a faster machine renders more of them; a chunk a remote worker fails returns to the queue for
//! the others, and that worker takes no more. Every chunk is encoded from a keyframe with the output's own
//! settings and the chunks are joined without encoding again, as the local ones are.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use sr_model::model as m;

/// The frames of one chunk: `first..last` of the `frames` frames of the output's range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Part {
    /// Index of the chunk's first frame in the output.
    pub first: usize,
    /// Index after its last frame.
    pub last: usize,
    /// Frames of the whole output: a worker that counts another number has another document or other options.
    pub frames: usize,
}

/// Renders one chunk of the output and nothing else: no audio, no sidecars, no destinations. The file written is
/// the chunk as a parallel delivery encodes it, for the coordinator to join.
#[derive(Debug, Clone)]
pub struct Segment {
    /// The chunk.
    pub part: Part,
    /// Set to stop the render at its next frame.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// What a worker says of a chunk it rendered.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PartDone {
    /// The encoder it used.
    pub encoder: String,
    /// Content it could not draw as authored (`Report::unsupported`).
    pub unsupported: Vec<String>,
}

/// One remote worker's share of an output, for the report.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct RemoteShare {
    /// The worker, as it was named to the coordinator.
    pub worker: String,
    /// Chunks it rendered.
    pub chunks: u32,
    /// Frames in them.
    pub frames: u64,
    /// Seconds from asking for its first chunk to receiving its last.
    pub seconds: f64,
    /// Why it stopped taking chunks, when it failed one (the chunk was rendered by another worker).
    pub failed: Option<String>,
}

/// Engines on other machines that render chunks of an output.
pub trait RemoteWorkers: std::fmt::Debug + Send + Sync {
    /// A name for each worker; its index is how [`RemoteWorkers::render`] names it.
    fn names(&self) -> Vec<String>;

    /// Has worker `worker` render `part` of `output` into the file `to`. `frame` is called as each frame is known
    /// to be rendered; `cancelled` is polled, and a render it stops returns an error. Called from a thread of the
    /// worker's own, one chunk at a time.
    fn render(
        &self,
        worker: usize,
        output: &m::Output,
        part: Part,
        to: &Path,
        cancelled: &AtomicBool,
        frame: &dyn Fn(),
    ) -> Result<PartDone, String>;
}

/// How many chunks an output of `frames` frames at `fps` is split into for `workers` workers, local and remote:
/// three each, so that a faster worker takes more and a failed chunk costs little, of at least two seconds.
pub fn chunks_for(workers: usize, frames: u64, fps: f64) -> usize {
    let shortest = (2.0 * fps).round().max(1.0) as u64;
    let most = (frames / shortest).max(1);
    ((workers as u64 * 3).min(most).max((workers as u64).min(frames))).max(1) as usize
}

#[cfg(test)]
mod tests {
    use super::chunks_for;

    #[test]
    fn chunks_are_three_a_worker_of_at_least_two_seconds() {
        assert_eq!(chunks_for(2, 24 * 60, 24.0), 6);
        assert_eq!(chunks_for(4, 24 * 8, 24.0), 4); // 8 s: four chunks of 2 s
        assert_eq!(chunks_for(2, 24 * 3, 24.0), 2); // never fewer than the workers, while there are frames
        assert_eq!(chunks_for(3, 2, 24.0), 2);
        assert_eq!(chunks_for(1, 1, 24.0), 1);
    }
}
