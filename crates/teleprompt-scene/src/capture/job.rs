//! What every backend does around running its tool: name a failure, make
//! a scratch directory, file each clip and say so, and cut a reel.

use std::path::{Path, PathBuf};

use super::{reel, CaptureError, Clip, Frame, Progress, Session, SessionShot, WorkDir};

/// One [`super::CaptureBackend::capture`] call under way: what it was
/// asked, and the clips kept so far.
pub struct Job<'a> {
    /// The plugin's name, as a failure reports it.
    pub backend: &'a str,
    pub session: &'a Session,
    pub frame: &'a Frame,
    pub out_dir: &'a Path,
    on_progress: &'a mut dyn FnMut(Progress),
    clips: Vec<Clip>,
}

impl<'a> Job<'a> {
    pub fn new(
        backend: &'a str,
        session: &'a Session,
        frame: &'a Frame,
        out_dir: &'a Path,
        on_progress: &'a mut dyn FnMut(Progress),
    ) -> Self {
        Job {
            backend,
            session,
            frame,
            out_dir,
            on_progress,
            clips: Vec::new(),
        }
    }

    /// `shot` could not be captured, for `why`.
    pub fn failed(&self, shot: &str, why: impl Into<String>) -> CaptureError {
        CaptureError::Failed {
            backend: self.backend.to_string(),
            shot: shot.into(),
            reason: why.into(),
        }
    }

    /// The session could not be captured, for `why`: put down to its first
    /// wanted shot, which is the first an author would look at.
    pub fn failed_all(&self, why: impl Into<String>) -> CaptureError {
        let shots = &self.session.shots;
        let first = shots.iter().find(|s| s.wanted).or(shots.first());
        self.failed(first.map_or("?", |s| s.id.as_str()), why)
    }

    /// The session cannot be captured on this machine: its shots become
    /// slates, with `why`.
    pub fn unavailable(&self, why: impl Into<String>) -> CaptureError {
        CaptureError::Unavailable {
            backend: self.backend.to_string(),
            reason: why.into(),
        }
    }

    /// A scratch directory beside the clips, removed when dropped.
    pub fn work_dir(&self) -> Result<WorkDir, CaptureError> {
        let io = |source| CaptureError::Io {
            path: self.out_dir.display().to_string(),
            source,
        };
        std::fs::create_dir_all(self.out_dir).map_err(io)?;
        WorkDir::create(self.out_dir, self.backend).map_err(io)
    }

    /// The wanted shots, in order.
    pub fn wanted(&self) -> impl Iterator<Item = &'a SessionShot> {
        self.session.shots.iter().filter(|s| s.wanted)
    }

    /// Where `shot`'s clip goes: filed under its key.
    pub fn clip_path(&self, shot: &SessionShot) -> PathBuf {
        self.out_dir.join(format!("{}.mp4", shot.key))
    }

    /// `shot`'s clip is written at [`Self::clip_path`]: kept, and said.
    pub fn keep(&mut self, shot: &SessionShot) {
        self.clips.push(Clip {
            key: shot.key,
            path: self.clip_path(shot),
        });
        (self.on_progress)(Progress {
            scene: self.session.scene.clone(),
            shot: shot.id.clone(),
            done: self.clips.len(),
            of: self.session.wanted(),
        });
    }

    /// Each wanted shot cut from `video`, the session recorded end to end
    /// at its scheduled lengths, once it is checked to be long enough.
    pub fn cut_reel(&mut self, ffmpeg: &str, video: &Path) -> Result<(), CaptureError> {
        if let Some(why) = reel::starved(video, self.session) {
            return Err(self.failed_all(why));
        }
        let starts: Vec<u64> = reel::windows(self.session)
            .into_iter()
            .map(|(from, _)| from)
            .collect();
        self.cut_at(ffmpeg, video, &starts)
    }

    /// Each wanted shot cut from `video` at its start in `starts` (one per
    /// shot of the session, in ms), for its scheduled length.
    pub fn cut_at(
        &mut self,
        ffmpeg: &str,
        video: &Path,
        starts: &[u64],
    ) -> Result<(), CaptureError> {
        let session = self.session;
        for (shot, from_ms) in session.shots.iter().zip(starts) {
            if !shot.wanted {
                continue;
            }
            reel::cut(
                ffmpeg,
                video,
                &self.clip_path(shot),
                *from_ms,
                shot.duration_ms,
            )
            .map_err(|why| self.failed(&shot.id, why))?;
            self.keep(shot);
        }
        Ok(())
    }

    /// The clips kept, once the session is done.
    pub fn clips(self) -> Vec<Clip> {
        self.clips
    }
}

/// `path` made absolute, for a tool run in another directory.
pub fn absolute(path: &Path) -> PathBuf {
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
