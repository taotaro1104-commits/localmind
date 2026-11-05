//! 重い処理の直列化・優先度・端末条件ゲート。
//!
//! 規則: 重いモデルを載せるジョブは同時に1つだけ。録音中は録音以外の重いジョブを開始しない。
//! 同期的な状態機械として表現し、非同期ランタイム統合は呼び出し側で行う。
//!
//! - `next` が `None` のとき、理由は [`Scheduler::blocker`] で分かる。
//! - 実行中に端末条件が悪化したら [`Scheduler::running_should_stop`] が停止すべきジョブを返す。
//!   実際の中断は呼び出し側の責務（再処理は途中から再開できる前提）。
//! - 録音ジョブ同士の同時実行は禁止しない（複数入力源の録音を許容する）。
//! - 優先度の高い重いジョブが続くと低優先度は待たされる（エイジングは行わない。仕様）。

pub mod gate;
pub mod priority;

use std::collections::BTreeMap;

pub use gate::{favorable, DeviceState, FixedDeviceState, Thermal};
pub use priority::{Job, Priority};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SchedulerError {
    #[error("ジョブID {0} は既に登録されています")]
    DuplicateId(u64),
    #[error("ジョブID {0} は実行中ではありません")]
    NotRunning(u64),
    #[error("ジョブID {0} は待機中ではありません")]
    NotQueued(u64),
}

/// 待機中のジョブが開始できない理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// 別の重いジョブ（id）が実行中。
    HeavyJobRunning(u64),
    /// 録音ジョブ（id）の実行中のため、録音以外の重いジョブは開始しない。
    RecordingInProgress(u64),
    /// 充電中かつ低温の条件を満たしていない（再処理ジョブ）。
    UnfavorableConditions,
}

/// スケジューラ。`next` で次に実行すべきジョブを取り出し、`complete` で完了を通知する。
#[derive(Debug, Default)]
pub struct Scheduler {
    /// (優先度, 投入順) → ジョブ。BTreeMap で優先度順・FIFO を保つ。
    queue: BTreeMap<(Priority, u64), Job>,
    running: Vec<Job>,
    seq: u64,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn submit(&mut self, job: Job) -> Result<(), SchedulerError> {
        let exists = self.running.iter().any(|j| j.id == job.id)
            || self.queue.values().any(|j| j.id == job.id);
        if exists {
            return Err(SchedulerError::DuplicateId(job.id));
        }
        self.seq += 1;
        self.queue.insert((job.priority, self.seq), job);
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    pub fn running(&self) -> &[Job] {
        &self.running
    }

    /// 実行中の重いジョブの id。
    fn heavy_running(&self) -> Option<u64> {
        self.running.iter().find(|j| j.heavy).map(|j| j.id)
    }

    /// 実行中の録音ジョブの id。
    fn recording_running(&self) -> Option<u64> {
        self.running
            .iter()
            .find(|j| j.priority == Priority::Recording)
            .map(|j| j.id)
    }

    /// `job` を今開始できない理由。開始できるなら None。
    fn block_reason(&self, job: &Job, state: &dyn DeviceState) -> Option<BlockReason> {
        if let Some(by) = self.heavy_running().filter(|_| job.heavy) {
            Some(BlockReason::HeavyJobRunning(by))
        } else if let Some(by) = self
            .recording_running()
            .filter(|_| job.heavy && job.priority != Priority::Recording)
        {
            Some(BlockReason::RecordingInProgress(by))
        } else if job.priority.requires_favorable_conditions() && !favorable(state) {
            Some(BlockReason::UnfavorableConditions)
        } else {
            None
        }
    }

    /// 開始可能な最優先ジョブを取り出して実行中にする。なければ None。
    pub fn next(&mut self, state: &dyn DeviceState) -> Option<Job> {
        let key = self
            .queue
            .iter()
            .find_map(|(k, job)| self.block_reason(job, state).is_none().then_some(*k))?;
        let job = self.queue.remove(&key)?;
        self.running.push(job.clone());
        Some(job)
    }

    /// 待機中で最優先のジョブが開始できない理由（`next` が None のときの診断用）。
    /// 待機中のジョブが無い、または先頭が開始可能なら None。
    pub fn blocker(&self, state: &dyn DeviceState) -> Option<(u64, BlockReason)> {
        let job = self.queue.values().next()?;
        self.block_reason(job, state).map(|r| (job.id, r))
    }

    /// 端末条件が悪化したため停止（保留）すべき実行中ジョブの id。
    pub fn running_should_stop(&self, state: &dyn DeviceState) -> Vec<u64> {
        if favorable(state) {
            return Vec::new();
        }
        self.running
            .iter()
            .filter(|j| j.priority.requires_favorable_conditions())
            .map(|j| j.id)
            .collect()
    }

    /// 待機中のジョブを取り消す。実行中のジョブは取り消せない（`complete` で終了させる）。
    pub fn cancel(&mut self, id: u64) -> Result<Job, SchedulerError> {
        let key = self
            .queue
            .iter()
            .find_map(|(k, j)| (j.id == id).then_some(*k))
            .ok_or(SchedulerError::NotQueued(id))?;
        self.queue.remove(&key).ok_or(SchedulerError::NotQueued(id))
    }

    pub fn complete(&mut self, id: u64) -> Result<Job, SchedulerError> {
        let pos = self
            .running
            .iter()
            .position(|j| j.id == id)
            .ok_or(SchedulerError::NotRunning(id))?;
        Ok(self.running.remove(pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cool_charging() -> FixedDeviceState {
        FixedDeviceState {
            charging: true,
            thermal: Thermal::Normal,
        }
    }

    fn on_battery() -> FixedDeviceState {
        FixedDeviceState {
            charging: false,
            thermal: Thermal::Normal,
        }
    }

    fn job(id: u64, p: Priority, heavy: bool) -> Job {
        Job::new(id, format!("job{id}"), p, heavy)
    }

    #[test]
    fn runs_in_priority_order() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Reprocessing, false)).unwrap();
        s.submit(job(2, Priority::Indexing, false)).unwrap();
        s.submit(job(3, Priority::Recording, false)).unwrap();
        s.submit(job(4, Priority::Chat, false)).unwrap();
        let st = cool_charging();
        let order: Vec<u64> = (0..4).map(|_| s.next(&st).unwrap().id).collect();
        assert_eq!(order, vec![3, 4, 2, 1]);
    }

    #[test]
    fn fifo_within_same_priority() {
        let mut s = Scheduler::new();
        s.submit(job(10, Priority::Chat, false)).unwrap();
        s.submit(job(11, Priority::Chat, false)).unwrap();
        let st = cool_charging();
        assert_eq!(s.next(&st).unwrap().id, 10);
        assert_eq!(s.next(&st).unwrap().id, 11);
    }

    #[test]
    fn only_one_heavy_job_at_a_time() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Chat, true)).unwrap();
        s.submit(job(2, Priority::Indexing, true)).unwrap();
        let st = cool_charging();
        assert_eq!(s.next(&st).unwrap().id, 1);
        assert!(s.next(&st).is_none());
        s.complete(1).unwrap();
        assert_eq!(s.next(&st).unwrap().id, 2);
    }

    #[test]
    fn light_job_can_run_beside_heavy() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Chat, true)).unwrap();
        s.submit(job(2, Priority::Indexing, false)).unwrap();
        let st = cool_charging();
        s.next(&st).unwrap();
        assert_eq!(s.next(&st).unwrap().id, 2);
    }

    #[test]
    fn reprocessing_waits_for_charging_and_cool() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Reprocessing, true)).unwrap();
        assert!(s.next(&on_battery()).is_none());
        let hot = FixedDeviceState {
            charging: true,
            thermal: Thermal::Hot,
        };
        assert!(s.next(&hot).is_none());
        assert_eq!(s.next(&cool_charging()).unwrap().id, 1);
    }

    #[test]
    fn blocked_job_does_not_starve_runnable_lower_priority() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Reprocessing, true)).unwrap();
        s.submit(job(2, Priority::Indexing, false)).unwrap();
        assert_eq!(s.next(&on_battery()).unwrap().id, 2);
    }

    #[test]
    fn heavy_non_recording_job_blocked_while_recording() {
        let mut s = Scheduler::new();
        // 録音は重くない（録音・VAD のみ）が、録音中は他の重い処理を始めない
        s.submit(job(1, Priority::Recording, false)).unwrap();
        s.submit(job(2, Priority::Chat, true)).unwrap();
        let st = cool_charging();
        assert_eq!(s.next(&st).unwrap().id, 1);
        assert!(s.next(&st).is_none());
        s.complete(1).unwrap();
        assert_eq!(s.next(&st).unwrap().id, 2);
    }

    #[test]
    fn duplicate_id_rejected_even_when_running() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Chat, false)).unwrap();
        assert_eq!(
            s.submit(job(1, Priority::Chat, false)),
            Err(SchedulerError::DuplicateId(1))
        );
        s.next(&cool_charging()).unwrap();
        assert_eq!(
            s.submit(job(1, Priority::Chat, false)),
            Err(SchedulerError::DuplicateId(1))
        );
    }

    #[test]
    fn complete_unknown_job_errors() {
        let mut s = Scheduler::new();
        assert_eq!(s.complete(9), Err(SchedulerError::NotRunning(9)));
    }

    fn warm_charging() -> FixedDeviceState {
        FixedDeviceState {
            charging: true,
            thermal: Thermal::Warm,
        }
    }

    #[test]
    fn heavy_recording_waits_for_a_running_heavy_job() {
        let mut s = Scheduler::new();
        let st = cool_charging();
        s.submit(job(1, Priority::Indexing, true)).unwrap();
        assert_eq!(s.next(&st).unwrap().id, 1);
        // 重い索引化の実行中に重い録音を投入しても、終わるまで開始できない。
        s.submit(job(4, Priority::Recording, true)).unwrap();
        assert!(s.next(&st).is_none());
        assert_eq!(s.blocker(&st), Some((4, BlockReason::HeavyJobRunning(1))));
        s.complete(1).unwrap();
        assert_eq!(s.next(&st).unwrap().id, 4);
    }

    #[test]
    fn light_non_recording_job_can_start_during_recording() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Recording, false)).unwrap();
        s.submit(job(2, Priority::Indexing, false)).unwrap();
        let st = cool_charging();
        s.next(&st).unwrap();
        assert_eq!(s.next(&st).unwrap().id, 2);
    }

    #[test]
    fn warm_device_still_runs_unconditional_jobs() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Indexing, true)).unwrap();
        s.submit(job(2, Priority::Reprocessing, false)).unwrap();
        let st = warm_charging();
        assert_eq!(s.next(&st).unwrap().id, 1);
        assert!(s.next(&st).is_none(), "再処理は Warm では開始しない");
    }

    #[test]
    fn blocker_explains_why_next_returns_none() {
        let mut s = Scheduler::new();
        assert_eq!(s.blocker(&cool_charging()), None);

        s.submit(job(1, Priority::Reprocessing, false)).unwrap();
        assert_eq!(
            s.blocker(&on_battery()),
            Some((1, BlockReason::UnfavorableConditions))
        );
        assert_eq!(s.blocker(&cool_charging()), None);

        s.next(&cool_charging()).unwrap();
        s.submit(job(2, Priority::Recording, false)).unwrap();
        s.next(&cool_charging()).unwrap();
        s.submit(job(3, Priority::Chat, true)).unwrap();
        assert_eq!(
            s.blocker(&cool_charging()),
            Some((3, BlockReason::RecordingInProgress(2)))
        );
    }

    #[test]
    fn running_should_stop_flags_reprocessing_when_conditions_worsen() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Reprocessing, true)).unwrap();
        s.submit(job(2, Priority::Indexing, false)).unwrap();
        let ok = cool_charging();
        s.next(&ok).unwrap();
        s.next(&ok).unwrap();
        assert!(s.running_should_stop(&ok).is_empty());
        assert_eq!(s.running_should_stop(&on_battery()), vec![1]);
        let hot = FixedDeviceState {
            charging: true,
            thermal: Thermal::Hot,
        };
        assert_eq!(s.running_should_stop(&hot), vec![1]);
    }

    #[test]
    fn cancel_removes_only_queued_jobs_and_frees_the_id() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Chat, false)).unwrap();
        s.submit(job(2, Priority::Chat, false)).unwrap();
        let st = cool_charging();
        s.next(&st).unwrap(); // 1 を実行中にする
        assert_eq!(s.cancel(1), Err(SchedulerError::NotQueued(1)));
        assert_eq!(s.cancel(2).unwrap().id, 2);
        assert_eq!(s.cancel(2), Err(SchedulerError::NotQueued(2)));
        assert_eq!(s.pending(), 0);
        s.submit(job(2, Priority::Chat, false)).unwrap(); // 取り消した id は再投入できる
    }

    #[test]
    fn two_recordings_may_run_together() {
        let mut s = Scheduler::new();
        s.submit(job(1, Priority::Recording, false)).unwrap();
        s.submit(job(2, Priority::Recording, false)).unwrap();
        let st = cool_charging();
        assert_eq!(s.next(&st).unwrap().id, 1);
        assert_eq!(s.next(&st).unwrap().id, 2);
    }
}
