use super::*;
use std::path::PathBuf;
use std::sync::Arc;

const NOW: u64 = 1_800_000_000_000;
const DAY: u64 = 24 * 60 * 60 * 1000;
const RATE: u32 = 16_000;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lm-ffi-rec-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
    fn store(&self) -> Arc<RecordingStore> {
        RecordingStore::open(self.0.to_string_lossy().into_owned()).unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pcm(seconds: usize) -> Vec<i16> {
    (0..RATE as usize * seconds)
        .map(|i| (i % 200) as i16)
        .collect()
}

#[test]
fn recording_without_consent_is_refused_and_creates_nothing() {
    let d = Dir::new("consent");
    let store = d.store();
    let err = store.begin("r1".into(), false, RATE, NOW).err().unwrap();
    assert!(matches!(err, FfiError::Ingest { .. }), "{err:?}");
    assert!(store.recordings().unwrap().is_empty());
    assert!(!d.0.join("r1").exists());
}

#[test]
fn audio_is_saved_in_30_second_chunks_and_the_tail_on_finish() {
    let d = Dir::new("chunks");
    let store = d.store();
    let rec = store.begin("r1".into(), true, RATE, NOW).unwrap();
    assert_eq!(
        rec.push(pcm(20)).unwrap(),
        0,
        "30 秒に満たない間は保存しない"
    );
    assert_eq!(rec.push(pcm(20)).unwrap(), 1, "40 秒分で 1 チャンク");
    assert_eq!(rec.finish().unwrap(), 2, "残りの 10 秒分も保存する");
    assert_eq!(rec.finish().unwrap(), 2, "2 回目は何もしない");
    let list = store.recordings().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (
            list[0].id.as_str(),
            list[0].chunks,
            list[0].consent_confirmed
        ),
        ("r1", 2, true)
    );
    assert_eq!(list[0].started_at_ms, NOW);
    assert!(list[0].readable && list[0].bytes > 0);
}

#[test]
fn pushing_after_finish_is_an_error() {
    let d = Dir::new("after");
    let rec = d.store().begin("r1".into(), true, RATE, NOW).unwrap();
    rec.finish().unwrap();
    assert!(matches!(rec.push(pcm(1)), Err(FfiError::Ingest { .. })));
}

#[test]
fn a_recording_id_cannot_be_reused_or_be_a_path() {
    let d = Dir::new("ids");
    let store = d.store();
    store.begin("r1".into(), true, RATE, NOW).unwrap();
    assert!(
        store.begin("r1".into(), true, RATE, NOW + 1).is_err(),
        "開始時刻を上書きできない"
    );
    for bad in ["../x", "a/b", "A", "con", ""] {
        assert!(store.begin(bad.into(), true, RATE, NOW).is_err(), "{bad:?}");
    }
    for rate in [0, 192_001, u32::MAX] {
        assert!(
            store.begin("r2".into(), true, rate, NOW).is_err(),
            "不正なサンプルレート {rate}"
        );
    }
}

#[test]
fn expired_recordings_are_purged_and_recent_ones_stay() {
    let d = Dir::new("purge");
    let store = d.store();
    for (id, started) in [("old", NOW - 40 * DAY), ("new", NOW - DAY)] {
        let rec = store.begin(id.into(), true, RATE, started).unwrap();
        rec.push(pcm(31)).unwrap();
        rec.finish().unwrap();
    }
    let r = store.purge_expired(NOW, 30).unwrap();
    assert_eq!(r.deleted, ["old"]);
    assert_eq!(
        store
            .recordings()
            .unwrap()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["new"]
    );
    assert!(store.purge_expired(NOW, 0).is_err(), "保持日数 0 は拒否");
    assert!(store.purge_expired(5, 30).is_err(), "不正な現在時刻");
}

#[test]
fn deleting_and_revoking_consent_work_and_a_revoked_recording_cannot_be_written() {
    let d = Dir::new("revoke");
    let store = d.store();
    let rec = store.begin("r1".into(), true, RATE, NOW).unwrap();
    rec.push(pcm(31)).unwrap();
    store.revoke_consent("r1".into()).unwrap();
    assert!(rec.push(pcm(30)).is_err(), "同意の撤回後は保存できない");
    assert!(rec.push(pcm(1)).is_err(), "失敗したあとは終了扱い");
    assert_eq!(
        rec.finish().unwrap(),
        1,
        "失敗後は端数を保存しない（抜けのある並びを作らない）"
    );
    assert!(!store.recordings().unwrap()[0].consent_confirmed);
    assert!(store.delete_recording("r1".into()).unwrap());
    assert!(!store.delete_recording("r1".into()).unwrap());
    assert!(store.recordings().unwrap().is_empty());
}

#[test]
fn relative_or_escaping_roots_are_refused_and_a_missing_root_is_empty() {
    assert!(RecordingStore::open("recordings".into()).is_err());
    assert!(RecordingStore::open("/tmp/../x".into()).is_err());
    let d = Dir::new("missing");
    assert!(d.store().recordings().unwrap().is_empty());
    assert_eq!(d.store().clean_stale_tmp().unwrap(), 0);
}

#[test]
fn concurrent_pushes_from_several_threads_lose_nothing() {
    let d = Dir::new("threads");
    let rec = d.store().begin("r1".into(), true, RATE, NOW).unwrap();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let rec = rec.clone();
            std::thread::spawn(move || rec.push(pcm(15)).unwrap())
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(rec.finish().unwrap(), 2, "合計 60 秒 = 30 秒 × 2");
}

fn sample_minutes() -> lm_ingest::Minutes {
    use lm_ingest::{ActionItem, Minutes, MinutesItem};
    Minutes {
        summary: vec![MinutesItem {
            text: "予算を議論した".into(),
            citations: vec![12_000],
        }],
        action_items: vec![ActionItem {
            text: "見積書を送る".into(),
            assignee: Some("田中".into()),
            due: Some("2026-11-12".into()),
            citations: vec![62_000],
            done: false,
        }],
        ..Minutes::default()
    }
}

#[test]
fn saved_minutes_are_exported_in_each_format() {
    let d = Dir::new("export");
    let store = d.store();
    let rec = store.begin("r1".into(), true, RATE, NOW).unwrap();
    rec.finish().unwrap();
    // 議事録は、取り込み側（lm-ingest）が保存する。
    lm_ingest::FsChunkStore::new(&d.0)
        .save_minutes("r1", &sample_minutes())
        .unwrap();
    let md = store
        .export_minutes("r1".into(), ExportFormat::Markdown, "定例".into(), 540, NOW)
        .unwrap();
    assert_eq!(
        (md.file_name.as_str(), md.mime_type.as_str()),
        ("minutes-r1.md", "text/markdown; charset=utf-8")
    );
    assert!(
        md.text.starts_with("# 定例\n") && md.text.contains("見積書を送る"),
        "{}",
        md.text
    );
    let csv = store
        .export_minutes("r1".into(), ExportFormat::Csv, String::new(), 0, NOW)
        .unwrap();
    assert!(csv.text.starts_with('\u{FEFF}') && csv.file_name == "minutes-r1.csv");
    let ics = store
        .export_minutes("r1".into(), ExportFormat::Ics, "t".into(), 540, NOW)
        .unwrap();
    assert!(
        ics.text.contains("DUE;VALUE=DATE:20261112")
            && ics.mime_type == "text/calendar; charset=utf-8"
    );
}

#[test]
fn exporting_without_minutes_or_with_bad_input_is_an_error() {
    let d = Dir::new("export-bad");
    let store = d.store();
    store
        .begin("r1".into(), true, RATE, NOW)
        .unwrap()
        .finish()
        .unwrap();
    let err = store
        .export_minutes("r1".into(), ExportFormat::Markdown, "t".into(), 540, NOW)
        .unwrap_err();
    assert!(matches!(err, FfiError::Ingest { .. }), "{err:?}");
    assert!(store
        .export_minutes("../x".into(), ExportFormat::Csv, "t".into(), 0, NOW)
        .is_err());
    lm_ingest::FsChunkStore::new(&d.0)
        .save_minutes("r1", &sample_minutes())
        .unwrap();
    assert!(
        store
            .export_minutes("r1".into(), ExportFormat::Csv, "t".into(), 99_999, NOW)
            .is_err(),
        "不正な UTC の差"
    );
    assert!(
        store
            .export_minutes("r1".into(), ExportFormat::Csv, "t".into(), 0, 5)
            .is_err(),
        "不正な時刻"
    );
    lm_ingest::FsChunkStore::new(&d.0)
        .save_minutes("r1", &lm_ingest::Minutes::default())
        .unwrap();
    assert!(
        store
            .export_minutes("r1".into(), ExportFormat::Ics, "t".into(), 0, NOW)
            .is_err(),
        "ToDo が無い iCalendar"
    );
}
