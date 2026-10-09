//! Background render queue (EXP-03): jobs run one at a time in priority order on
//! a worker the shell provides; each can be paused, resumed or cancelled, and
//! editing continues meanwhile. The queue itself is a state machine so it is
//! testable without threads.

use crate::job::{Control, ExportJob, Progress};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

pub struct Entry {
    pub id: JobId,
    pub name: String,
    pub priority: i32,
    pub job: ExportJob,
    pub output: String,
    pub state: JobState,
    pub progress: Progress,
    pub control: Control,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct ExportQueue {
    entries: VecDeque<Entry>,
    next_id: u64,
}

impl ExportQueue {
    pub fn submit(
        &mut self,
        name: impl Into<String>,
        job: ExportJob,
        output: impl Into<String>,
        priority: i32,
    ) -> JobId {
        self.next_id += 1;
        let id = JobId(self.next_id);
        self.entries.push_back(Entry {
            id,
            name: name.into(),
            priority,
            job,
            output: output.into(),
            state: JobState::Queued,
            progress: Progress::default(),
            control: Control::default(),
            error: None,
        });
        id
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    pub fn get(&self, id: JobId) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    fn get_mut(&mut self, id: JobId) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn set_priority(&mut self, id: JobId, priority: i32) {
        if let Some(e) = self.get_mut(id) {
            e.priority = priority;
        }
    }

    /// The queued job the worker should run next: highest priority, then oldest.
    /// Marks it running and hands back what the worker needs.
    pub fn take_next(&mut self) -> Option<(JobId, ExportJob, String, Control)> {
        let running = self
            .entries
            .iter()
            .any(|e| matches!(e.state, JobState::Running | JobState::Paused));
        if running {
            return None;
        }
        let best = self
            .entries
            .iter()
            .filter(|e| e.state == JobState::Queued)
            .max_by_key(|e| (e.priority, std::cmp::Reverse(e.id)))?
            .id;
        let e = self.get_mut(best)?;
        e.state = JobState::Running;
        e.control.resume();
        Some((e.id, e.job.clone(), e.output.clone(), e.control.clone()))
    }

    pub fn report_progress(&mut self, id: JobId, progress: Progress) {
        if let Some(e) = self.get_mut(id) {
            e.progress = progress;
        }
    }

    pub fn finish(&mut self, id: JobId, result: Result<Progress, String>) {
        if let Some(e) = self.get_mut(id) {
            match result {
                Ok(p) => {
                    e.progress = p;
                    e.state = if e.control.is_cancelled() {
                        JobState::Cancelled
                    } else {
                        JobState::Done
                    };
                }
                Err(msg) => {
                    e.error = Some(msg);
                    e.state = JobState::Failed;
                }
            }
        }
    }

    pub fn pause(&mut self, id: JobId) {
        if let Some(e) = self.get_mut(id) {
            if e.state == JobState::Running {
                e.state = JobState::Paused;
                e.control.pause();
            }
        }
    }

    pub fn resume(&mut self, id: JobId) {
        if let Some(e) = self.get_mut(id) {
            if e.state == JobState::Paused {
                e.state = JobState::Running;
                e.control.resume();
            }
        }
    }

    /// Cancel a queued job immediately, or ask a running one to stop.
    pub fn cancel(&mut self, id: JobId) {
        if let Some(e) = self.get_mut(id) {
            match e.state {
                JobState::Queued => e.state = JobState::Cancelled,
                JobState::Running | JobState::Paused => e.control.cancel(),
                _ => {}
            }
        }
    }

    /// Drop finished, failed and cancelled entries.
    pub fn clear_finished(&mut self) {
        self.entries.retain(|e| {
            matches!(
                e.state,
                JobState::Queued | JobState::Running | JobState::Paused
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_core::{FrameRate, IdGen, Rational};
    use debut_project::Sequence;
    use std::collections::HashMap;

    fn job() -> ExportJob {
        let seq = Sequence::new(IdGen::new(1).fresh(), "q", FrameRate::FPS_25, 16, 9);
        ExportJob {
            sequence: seq,
            range: (Rational::ZERO, Rational::ONE),
            sample_rate: 48_000,
            mixes: HashMap::new(),
        }
    }

    #[test]
    fn runs_by_priority_then_age_one_at_a_time() {
        let mut q = ExportQueue::default();
        let a = q.submit("a", job(), "a.mp4", 0);
        let b = q.submit("b", job(), "b.mp4", 5);
        let c = q.submit("c", job(), "c.mp4", 5);
        let (first, _, out, _) = q.take_next().unwrap();
        assert_eq!((first, out.as_str()), (b, "b.mp4"));
        assert!(q.take_next().is_none(), "one job at a time");
        q.finish(b, Ok(Progress::default()));
        assert_eq!(q.take_next().unwrap().0, c, "same priority: oldest first");
        q.finish(c, Ok(Progress::default()));
        assert_eq!(q.take_next().unwrap().0, a);
        assert_eq!(q.get(b).unwrap().state, JobState::Done);
    }

    #[test]
    fn pause_resume_cancel_drive_the_control() {
        let mut q = ExportQueue::default();
        let a = q.submit("a", job(), "a.mp4", 0);
        let b = q.submit("b", job(), "b.mp4", 0);
        let (id, _, _, control) = q.take_next().unwrap();
        assert_eq!(id, a);
        q.pause(a);
        assert!(control.is_paused());
        assert_eq!(q.get(a).unwrap().state, JobState::Paused);
        assert!(q.take_next().is_none(), "paused still occupies the worker");
        q.resume(a);
        assert!(!control.is_paused());
        q.cancel(a);
        assert!(control.is_cancelled());
        q.finish(
            a,
            Ok(Progress {
                frames_done: 3,
                frames_total: 25,
            }),
        );
        assert_eq!(q.get(a).unwrap().state, JobState::Cancelled);
        q.cancel(b);
        assert_eq!(q.get(b).unwrap().state, JobState::Cancelled);
        assert!(q.take_next().is_none());
        q.clear_finished();
        assert_eq!(q.entries().count(), 0);
    }
}
